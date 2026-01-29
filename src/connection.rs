use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{interval, Duration};

use crate::diagnostics::{ServiceProfile, StreamDiagnostics};
use crate::display;
use crate::flv::audio::AudioAnalyzer;
use crate::flv::video::VideoAnalyzer;
use crate::rtmp::chunk::ChunkReader;
use crate::rtmp::handshake;
use crate::rtmp::message::{MessageHandler, RtmpEvent};
use crate::stats::StreamStats;

pub async fn handle_connection(mut stream: TcpStream, addr: std::net::SocketAddr) {
    // Phase 1: Handshake
    let remaining = match handshake::perform_handshake(&mut stream).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Handshake failed for {}: {}", addr, e);
            return;
        }
    };

    // Phase 2: RTMP session
    let mut chunk_reader = ChunkReader::new();
    let mut handler = MessageHandler::new();
    let mut video_analyzer = VideoAnalyzer::new();
    let mut audio_analyzer = AudioAnalyzer::new();
    let mut stats = StreamStats::new();
    let mut diagnostics = StreamDiagnostics::new();
    let mut encoder_name: Option<String> = None;
    let mut publishing = false;

    // Default to Twitch profile for now (most strict)
    diagnostics.set_profile(ServiceProfile::Twitch);

    // Feed any remaining bytes from handshake
    if !remaining.is_empty() {
        chunk_reader.extend(&remaining);
    }

    let mut buf = vec![0u8; 65536];
    let mut display_interval = interval(Duration::from_secs(1));
    display_interval.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            result = stream.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        break;
                    }
                    Ok(n) => {
                        // Track bytes for window acknowledgement
                        if let Some(ack_data) = handler.track_bytes(n) {
                            let _ = stream.write_all(&ack_data).await;
                        }

                        chunk_reader.extend(&buf[..n]);
                        let messages = chunk_reader.read_messages();

                        for msg in messages {
                            let result = handler.handle(msg);

                            // Apply chunk size change
                            if let Some(new_size) = result.new_chunk_size {
                                chunk_reader.set_chunk_size(new_size);
                            }

                            // Send responses
                            for response in &result.responses {
                                if let Err(e) = stream.write_all(response).await {
                                    eprintln!("Write error: {}", e);
                                    return;
                                }
                            }

                            // Handle events
                            if let Some(event) = result.event {
                                match event {
                                    RtmpEvent::Connected { .. } => {}
                                    RtmpEvent::Publishing { .. } => {
                                        publishing = true;
                                        diagnostics.record_stream_start();
                                        display::init_terminal();
                                    }
                                    RtmpEvent::Metadata { ref properties } => {
                                        let mut has_dims = false;
                                        let mut has_fps = false;
                                        let mut has_bitrate = false;

                                        for (key, value) in properties {
                                            match key.as_str() {
                                                "encoder" => {
                                                    if let Some(s) = value.as_str() {
                                                        encoder_name = Some(s.to_string());
                                                    }
                                                }
                                                "width" | "height" => has_dims = true,
                                                "framerate" | "fps" => has_fps = true,
                                                "videodatarate" | "audiodatarate" => has_bitrate = true,
                                                _ => {}
                                            }
                                        }

                                        diagnostics.record_metadata(has_dims, has_fps, has_bitrate);
                                    }
                                    RtmpEvent::VideoData { timestamp, ref data } => {
                                        let byte_count = data.len();

                                        // Track diagnostics before processing
                                        diagnostics.record_video_timestamp(timestamp);

                                        // Check for AVC sequence header
                                        if data.len() >= 2 {
                                            let codec_id = data[0] & 0x0F;
                                            if codec_id == 7 && data[1] == 0 {
                                                diagnostics.record_avc_seq_header();
                                            }
                                        }

                                        // Process video
                                        video_analyzer.process(data, timestamp);

                                        // Track frame types
                                        let is_keyframe = !data.is_empty() && ((data[0] >> 4) & 0x0F) == 1;
                                        if is_keyframe {
                                            diagnostics.record_keyframe(stats.keyframe_interval_secs);
                                        }

                                        // Check for B-frames (composition time offset != 0)
                                        if data.len() >= 5 && (data[0] & 0x0F) == 7 && data[1] == 1 {
                                            let cto = ((data[2] as i32) << 16)
                                                | ((data[3] as i32) << 8)
                                                | (data[4] as i32);
                                            if cto != 0 {
                                                diagnostics.record_b_frame();
                                            }
                                        }

                                        stats.record_video_frame(byte_count, is_keyframe);
                                    }
                                    RtmpEvent::AudioData { timestamp, ref data } => {
                                        let byte_count = data.len();

                                        // Track diagnostics
                                        diagnostics.record_audio_timestamp(timestamp);

                                        // Check for AAC sequence header
                                        let is_aac_seq_header = data.len() >= 2
                                            && ((data[0] >> 4) & 0x0F) == 10
                                            && data[1] == 0;

                                        if is_aac_seq_header {
                                            diagnostics.record_aac_seq_header();
                                        }

                                        // Process audio
                                        audio_analyzer.process(data, timestamp);

                                        if !is_aac_seq_header {
                                            stats.record_audio_frame(byte_count);
                                        }
                                    }
                                    RtmpEvent::StreamEnded => {
                                        publishing = false;
                                        display::restore_terminal();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
            _ = display_interval.tick() => {
                if publishing {
                    // Run diagnostic checks
                    let results = diagnostics.check_all(
                        video_analyzer.width,
                        video_analyzer.height,
                        video_analyzer.profile.as_deref(),
                        audio_analyzer.effective_sample_rate(),
                        audio_analyzer.effective_channels(),
                        audio_analyzer.aac_profile.as_deref(),
                        stats.keyframe_interval_secs,
                    );

                    display::render(
                        handler.app_name(),
                        handler.stream_key(),
                        &stats,
                        &video_analyzer,
                        &audio_analyzer,
                        &encoder_name,
                        &diagnostics,
                        &results,
                    );
                }
            }
        }
    }

    display::restore_terminal();
}
