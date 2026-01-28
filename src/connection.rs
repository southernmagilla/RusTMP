use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{interval, Duration};

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
    let mut encoder_name: Option<String> = None;
    let mut publishing = false;

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
                                        display::init_terminal();
                                    }
                                    RtmpEvent::Metadata { ref properties } => {
                                        // Extract encoder name from metadata
                                        for (key, value) in properties {
                                            if key == "encoder" {
                                                if let Some(s) = value.as_str() {
                                                    encoder_name = Some(s.to_string());
                                                }
                                            }
                                        }
                                    }
                                    RtmpEvent::VideoData { timestamp, ref data } => {
                                        let byte_count = data.len();
                                        video_analyzer.process(data, timestamp);
                                        let is_keyframe = !data.is_empty()
                                            && ((data[0] >> 4) & 0x0F) == 1;
                                        stats.record_video_frame(byte_count, is_keyframe);
                                    }
                                    RtmpEvent::AudioData { timestamp, ref data } => {
                                        let byte_count = data.len();
                                        audio_analyzer.process(data, timestamp);
                                        // Don't count AAC sequence headers as audio frames for stats
                                        let is_aac_seq_header = data.len() >= 2
                                            && ((data[0] >> 4) & 0x0F) == 10
                                            && data[1] == 0;
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
                    display::render(
                        handler.app_name(),
                        handler.stream_key(),
                        &stats,
                        &video_analyzer,
                        &audio_analyzer,
                        &encoder_name,
                    );
                }
            }
        }
    }

    display::restore_terminal();
}
