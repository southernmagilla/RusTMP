use std::io::{self, Write};

use crate::diagnostics::{Diagnostic, Severity, StreamDiagnostics};
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

pub fn init_terminal() {
    #[cfg(windows)]
    {
        enable_windows_ansi();
    }
    print!("\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l");
    let _ = io::stdout().flush();
}

pub fn restore_terminal() {
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
            let _ = SetConsoleMode(handle as *mut _, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
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
    diagnostics: &StreamDiagnostics,
    diagnostic_results: &[Diagnostic],
) {
    let mut out = String::with_capacity(4096);

    // Clear screen and home
    out.push_str("\x1b[2J\x1b[H");

    // ══════════════════════════════════════════════════════════════
    // LOGO - Classic ASCII art in a box
    // ══════════════════════════════════════════════════════════════
    out.push_str(&format!("{BRIGHT_RED}{BOLD}"));
    out.push_str(" ╔════════════════════════════════════════╗\n");
    out.push_str(" ║  ____           _____ __  __ ____      ║\n");
    out.push_str(" ║ |  _ \\ _   _ __|_   _|  \\/  |  _ \\     ║\n");
    out.push_str(" ║ | |_) | | | / __|| | | |\\/| | |_) |    ║\n");
    out.push_str(" ║ |  _ <| |_| \\__ \\| | | |  | |  __/     ║\n");
    out.push_str(" ║ |_| \\_\\\\__,_|___/|_| |_|  |_|_|        ║\n");
    out.push_str(" ╚════════════════════════════════════════╝\n");
    out.push_str(&format!("{RESET}"));
    out.push_str(&format!(" {DIM}Stream Analyzer v0.1.0{RESET}\n\n"));

    // ══════════════════════════════════════════════════════════════
    // STREAM INFO BOX
    // ══════════════════════════════════════════════════════════════
    let stream_path = format!("{}/{}",
        if app_name.is_empty() { "?" } else { app_name },
        if stream_key.is_empty() { "?" } else { stream_key });
    let encoder_str = encoder_name.as_deref().unwrap_or("-");
    let duration_str = format_duration(stats.duration_secs);

    out.push_str(&format!(" {DIM}┌─────────────────────────────────────┐{RESET}\n"));
    out.push_str(&format!(" {DIM}│{RESET} {CYAN}Stream:{RESET}   {BRIGHT_GREEN}{:<25}{RESET} {DIM}│{RESET}\n", stream_path));
    out.push_str(&format!(" {DIM}│{RESET} {CYAN}Encoder:{RESET}  {:<25} {DIM}│{RESET}\n", encoder_str));
    out.push_str(&format!(" {DIM}│{RESET} {CYAN}Duration:{RESET} {BRIGHT_YELLOW}{:<25}{RESET} {DIM}│{RESET}\n", duration_str));
    out.push_str(&format!(" {DIM}└─────────────────────────────────────┘{RESET}\n\n"));

    // ══════════════════════════════════════════════════════════════
    // VIDEO SECTION
    // ══════════════════════════════════════════════════════════════
    out.push_str(&format!(" {MAGENTA}{BOLD}▶ VIDEO{RESET}\n"));
    out.push_str(&format!(" {DIM}─────────────────────────────────────{RESET}\n"));

    let codec = video.codec.as_ref().map(|c| c.to_string()).unwrap_or_else(|| "-".into());
    out.push_str(&format!("   {DIM}Codec:{RESET}       {BRIGHT_GREEN}{}{RESET}\n", codec));

    if let (Some(w), Some(h)) = (video.width, video.height) {
        out.push_str(&format!("   {DIM}Resolution:{RESET}  {BRIGHT_YELLOW}{}x{}{RESET}\n", w, h));
    }

    if let Some(ref p) = video.profile {
        let level_str = video.level.as_deref().unwrap_or("?");
        out.push_str(&format!("   {DIM}Profile:{RESET}     {} @ Level {}\n", p, level_str));
    }

    let fps = stats.current_fps().unwrap_or(0.0);
    let fps_color = if fps >= 29.0 { BRIGHT_GREEN } else if fps >= 24.0 { YELLOW } else { BRIGHT_RED };
    out.push_str(&format!("   {DIM}FPS:{RESET}         {}{:.1}{RESET}\n", fps_color, fps));

    out.push_str(&format!("   {DIM}Bitrate:{RESET}     {BRIGHT_CYAN}{}{RESET}\n",
        format_bitrate(stats.current_video_bitrate_kbps().unwrap_or(0.0))));

    let kf_int = stats.keyframe_interval_secs.map(|s| format!("{:.1}s", s)).unwrap_or_else(|| "-".into());
    out.push_str(&format!("   {DIM}Keyframes:{RESET}   {} {DIM}(interval: {}){RESET}\n", video.keyframe_count, kf_int));
    out.push_str(&format!("   {DIM}P-frames:{RESET}    {}\n", video.inter_frame_count));
    out.push_str(&format!("   {DIM}B-frames:{RESET}    {}\n", video.b_frame_count));

    let total_video_frames = video.keyframe_count + video.inter_frame_count + video.b_frame_count;
    let video_kb = stats.total_video_bytes as f64 / 1024.0;
    out.push_str(&format!(" {DIM}Total: {} frames, {:.1} KB{RESET}\n\n", total_video_frames, video_kb));

    // ══════════════════════════════════════════════════════════════
    // AUDIO SECTION
    // ══════════════════════════════════════════════════════════════
    out.push_str(&format!(" {BLUE}{BOLD}♪ AUDIO{RESET}\n"));
    out.push_str(&format!(" {DIM}─────────────────────────────────────{RESET}\n"));

    let acodec = audio.codec.as_ref().map(|c| c.to_string()).unwrap_or_else(|| "-".into());
    out.push_str(&format!("   {DIM}Codec:{RESET}       {BRIGHT_GREEN}{}{RESET}\n", acodec));

    if let Some(ref p) = audio.aac_profile {
        out.push_str(&format!("   {DIM}Profile:{RESET}     {}\n", p));
    }

    let sr = audio.effective_sample_rate().map(|r| format!("{} Hz", r)).unwrap_or_else(|| "-".into());
    out.push_str(&format!("   {DIM}Sample Rate:{RESET} {BRIGHT_YELLOW}{}{RESET}\n", sr));

    let ch = audio.effective_channels().unwrap_or(0);
    let ch_str = match ch { 1 => "mono", 2 => "stereo", 6 => "5.1", 8 => "7.1", _ => "-" };
    out.push_str(&format!("   {DIM}Channels:{RESET}    {} ({})\n", ch, ch_str));

    // Bit depth from FLV header
    let bit_depth = audio.sample_size.map(|s| format!("{}-bit", s)).unwrap_or_else(|| "-".into());
    out.push_str(&format!("   {DIM}Bit Depth:{RESET}   {}\n", bit_depth));

    out.push_str(&format!("   {DIM}Bitrate:{RESET}     {BRIGHT_CYAN}{}{RESET}\n",
        format_bitrate(stats.current_audio_bitrate_kbps().unwrap_or(0.0))));

    let audio_kb = stats.total_audio_bytes as f64 / 1024.0;
    out.push_str(&format!(" {DIM}Total: {} frames, {:.1} KB{RESET}\n\n", audio.total_audio_frames, audio_kb));

    // ══════════════════════════════════════════════════════════════
    // DIAGNOSTICS SECTION
    // ══════════════════════════════════════════════════════════════
    let errors = diagnostics.error_count();
    let warnings = diagnostics.warning_count();

    if errors > 0 {
        out.push_str(&format!(" {BRIGHT_RED}{BOLD}✖ ERRORS{RESET}\n"));
    } else if warnings > 0 {
        out.push_str(&format!(" {BRIGHT_YELLOW}{BOLD}⚠ WARNINGS{RESET}\n"));
    } else {
        out.push_str(&format!(" {BRIGHT_GREEN}{BOLD}✓ STATUS: OK{RESET}\n"));
    }
    out.push_str(&format!(" {DIM}─────────────────────────────────────{RESET}\n"));

    if diagnostic_results.is_empty() {
        out.push_str(&format!("   {DIM}No issues detected{RESET}\n"));
    } else {
        for diag in diagnostic_results.iter().take(6) {
            let (icon, color) = match diag.severity {
                Severity::Error => ("✖", BRIGHT_RED),
                Severity::Warning => ("!", BRIGHT_YELLOW),
                Severity::Info => ("·", DIM),
            };
            out.push_str(&format!("   {color}{icon}{RESET} [{DIM}{}{RESET}] {}\n", diag.category, diag.message));
        }
        if diagnostic_results.len() > 6 {
            out.push_str(&format!("   {DIM}+{} more...{RESET}\n", diagnostic_results.len() - 6));
        }
    }

    // ══════════════════════════════════════════════════════════════
    // FOOTER - Headers status and help
    // ══════════════════════════════════════════════════════════════
    out.push('\n');
    out.push_str(&format!(" {DIM}Headers:{RESET} "));
    let avc_status = if diagnostics.avc_seq_header_received {
        format!("{GREEN}AVC{RESET}")
    } else {
        format!("{RED}AVC{RESET}")
    };
    let aac_status = if diagnostics.aac_seq_header_received {
        format!("{GREEN}AAC{RESET}")
    } else {
        format!("{RED}AAC{RESET}")
    };
    let meta_status = if diagnostics.metadata_received {
        format!("{GREEN}META{RESET}")
    } else {
        format!("{YELLOW}META{RESET}")
    };
    out.push_str(&avc_status);
    out.push_str(" ");
    out.push_str(&aac_status);
    out.push_str(" ");
    out.push_str(&meta_status);

    out.push_str(&format!("\n\n {DIM}Press Ctrl+C to stop{RESET}\n"));

    print!("{}", out);
    let _ = io::stdout().flush();
}

fn format_bitrate(kbps: f64) -> String {
    if kbps >= 1000.0 {
        format!("{:.1} Mbps", kbps / 1000.0)
    } else if kbps > 0.0 {
        format!("{:.0} kbps", kbps)
    } else {
        "-".into()
    }
}

fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 { format!("{}:{:02}:{:02}", h, m, s) } else { format!("{}:{:02}", m, s) }
}
