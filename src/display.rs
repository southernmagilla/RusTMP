use std::io::{self, Write};

use crate::flv::audio::AudioAnalyzer;
use crate::flv::video::VideoAnalyzer;
use crate::stats::StreamStats;

// ANSI color codes
#[allow(dead_code)]
mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";

    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
}
use colors::*;

/// Initialize terminal for dashboard output.
/// On Windows, enables ANSI virtual terminal processing.
pub fn init_terminal() {
    #[cfg(windows)]
    {
        enable_windows_ansi();
    }
    // Enter alternate screen buffer, clear, hide cursor
    print!("\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l");
    let _ = io::stdout().flush();
}

/// Restore terminal state.
pub fn restore_terminal() {
    // Show cursor, reset attributes, exit alternate screen buffer
    print!("\x1b[?25h\x1b[0m\x1b[?1049l");
    let _ = io::stdout().flush();
}

#[cfg(windows)]
fn enable_windows_ansi() {
    use std::os::windows::io::AsRawHandle;
    unsafe extern "system" {
        fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut std::ffi::c_void, mode: u32) -> i32;
    }
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    unsafe {
        let handle = io::stdout().as_raw_handle();
        let mut mode: u32 = 0;
        if GetConsoleMode(handle as *mut _, &mut mode) != 0 {
            let _ = SetConsoleMode(
                handle as *mut _,
                mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            );
        }
    }
}

pub fn render(
    app_name: &str,
    stream_key: &str,
    stats: &StreamStats,
    video: &VideoAnalyzer,
    audio: &AudioAnalyzer,
    encoder_name: &Option<String>,
) {
    let mut out = String::with_capacity(2048);

    // Move cursor home (no clear needed - we overwrite everything)
    out.push_str("\x1b[H");

    // ASCII art logo - "RusTMP" with mixed case styling
    out.push_str(&format!("{BRIGHT_RED}{BOLD}"));
    out.push_str(r#"
 ____           _____ __  __ ____
|  _ \ _   _ __|_   _|  \/  |  _ \
| |_) | | | / __|| | | |\/| | |_) |
|  _ <| |_| \__ \| | | |  | |  __/
|_| \_\\__,_|___/|_| |_|  |_|_|
"#);
    out.push_str(&format!("{RESET}"));
    out.push_str(&format!("{DIM}Stream Analyzer v0.1.0{RESET}\n\n"));

    // Stream info box
    out.push_str(&format!("{CYAN}┌─────────────────────────────────────────────────┐{RESET}\n"));
    out.push_str(&format!("{CYAN}│{RESET} {BOLD}Stream:{RESET} {BRIGHT_GREEN}{}/{}{RESET}",
        if app_name.is_empty() { "?" } else { app_name },
        if stream_key.is_empty() { "?" } else { stream_key }
    ));
    let stream_len = app_name.len() + stream_key.len() + 1;
    let padding = 40usize.saturating_sub(stream_len);
    for _ in 0..padding { out.push(' '); }
    out.push_str(&format!("{CYAN}│{RESET}\n"));

    if let Some(enc) = encoder_name {
        out.push_str(&format!("{CYAN}│{RESET} {BOLD}Encoder:{RESET} {DIM}{}{RESET}", enc));
        let enc_padding = 39usize.saturating_sub(enc.len());
        for _ in 0..enc_padding { out.push(' '); }
        out.push_str(&format!("{CYAN}│{RESET}\n"));
    }

    out.push_str(&format!("{CYAN}│{RESET} {BOLD}Duration:{RESET} {BRIGHT_YELLOW}{:.1}s{RESET}", stats.duration_secs));
    let dur_str = format!("{:.1}s", stats.duration_secs);
    let dur_padding = 38usize.saturating_sub(dur_str.len());
    for _ in 0..dur_padding { out.push(' '); }
    out.push_str(&format!("{CYAN}│{RESET}\n"));
    out.push_str(&format!("{CYAN}└─────────────────────────────────────────────────┘{RESET}\n\n"));

    // Video section
    out.push_str(&format!("{MAGENTA}{BOLD}▶ VIDEO{RESET}\n"));
    out.push_str(&format!("{DIM}─────────────────────────────{RESET}\n"));

    let codec_str = video.codec.as_ref().map(|c| c.to_string()).unwrap_or_else(|| "waiting...".to_string());
    out.push_str(&format!("  {CYAN}Codec:{RESET}       {BRIGHT_GREEN}{}{RESET}\n", codec_str));

    if let (Some(w), Some(h)) = (video.width, video.height) {
        out.push_str(&format!("  {CYAN}Resolution:{RESET}  {BRIGHT_YELLOW}{}x{}{RESET}\n", w, h));
    }
    if let Some(ref p) = video.profile {
        out.push_str(&format!("  {CYAN}Profile:{RESET}     {} @ Level {}\n", p, video.level.as_deref().unwrap_or("?")));
    }

    let fps = stats.current_fps().unwrap_or(0.0);
    let fps_color = if fps >= 29.0 { BRIGHT_GREEN } else if fps >= 24.0 { YELLOW } else { BRIGHT_RED };
    out.push_str(&format!("  {CYAN}FPS:{RESET}         {}{:.1}{RESET}\n", fps_color, fps));

    let bitrate = stats.current_video_bitrate_kbps().unwrap_or(0.0);
    out.push_str(&format!("  {CYAN}Bitrate:{RESET}     {BRIGHT_CYAN}{:.0} kbps{RESET}\n", bitrate));

    let kf_interval = stats.keyframe_interval_secs.map(|s| format!("{:.1}s", s)).unwrap_or_else(|| "n/a".to_string());
    out.push_str(&format!("  {CYAN}Keyframes:{RESET}   {GREEN}{}{RESET} (interval: {})\n", video.keyframe_count, kf_interval));
    out.push_str(&format!("  {CYAN}P-frames:{RESET}    {}\n", video.inter_frame_count));
    out.push_str(&format!("  {CYAN}B-frames:{RESET}    {}\n", video.b_frame_count));
    out.push_str(&format!("  {DIM}Total: {} frames, {}{RESET}\n", video.total_video_frames, format_bytes(video.total_video_bytes)));
    out.push('\n');

    // Audio section
    out.push_str(&format!("{BLUE}{BOLD}♪ AUDIO{RESET}\n"));
    out.push_str(&format!("{DIM}─────────────────────────────{RESET}\n"));

    let acodec_str = audio.codec.as_ref().map(|c| c.to_string()).unwrap_or_else(|| "waiting...".to_string());
    out.push_str(&format!("  {CYAN}Codec:{RESET}       {BRIGHT_GREEN}{}{RESET}\n", acodec_str));

    if let Some(ref profile) = audio.aac_profile {
        out.push_str(&format!("  {CYAN}Profile:{RESET}     {}\n", profile));
    }

    let sr_str = audio.effective_sample_rate().map(|r| format!("{} Hz", r)).unwrap_or_else(|| "?".to_string());
    out.push_str(&format!("  {CYAN}Sample Rate:{RESET} {BRIGHT_YELLOW}{}{RESET}\n", sr_str));

    let channels = audio.effective_channels().unwrap_or(0);
    let ch_name = match channels {
        1 => "mono",
        2 => "stereo",
        6 => "5.1 surround",
        8 => "7.1 surround",
        _ => "unknown",
    };
    out.push_str(&format!("  {CYAN}Channels:{RESET}    {} ({})\n", channels, ch_name));

    if let Some(ss) = audio.sample_size {
        out.push_str(&format!("  {CYAN}Bit Depth:{RESET}   {}-bit\n", ss));
    }

    let abitrate = stats.current_audio_bitrate_kbps().unwrap_or(0.0);
    out.push_str(&format!("  {CYAN}Bitrate:{RESET}     {BRIGHT_CYAN}{:.0} kbps{RESET}\n", abitrate));
    out.push_str(&format!("  {DIM}Total: {} frames, {}{RESET}\n", audio.total_audio_frames, format_bytes(audio.total_audio_bytes)));

    // Footer
    out.push_str(&format!("\n{DIM}Press Ctrl+C to stop{RESET}"));

    // Clear to end of screen (removes any leftover content)
    out.push_str("\x1b[J");

    print!("{}", out);
    let _ = io::stdout().flush();
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}
