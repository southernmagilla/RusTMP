use std::time::Instant;

/// Severity level for diagnostic warnings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A diagnostic warning or issue detected in the stream
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

impl Diagnostic {
    pub fn info(category: &'static str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Info, category, message: message.into() }
    }

    pub fn warning(category: &'static str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, category, message: message.into() }
    }

    pub fn error(category: &'static str, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, category, message: message.into() }
    }
}

/// Known streaming service profiles for compatibility checking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ServiceProfile {
    Twitch,
    YouTube,
    Generic,
}

impl ServiceProfile {
    pub fn name(&self) -> &'static str {
        match self {
            ServiceProfile::Twitch => "Twitch",
            ServiceProfile::YouTube => "YouTube",
            ServiceProfile::Generic => "Generic",
        }
    }
}

/// Tracks stream health and compatibility issues
pub struct StreamDiagnostics {
    pub profile: ServiceProfile,

    // Sequence headers
    pub avc_seq_header_received: bool,
    pub avc_seq_header_time: Option<Instant>,
    pub aac_seq_header_received: bool,
    pub aac_seq_header_time: Option<Instant>,

    // First keyframe
    pub first_keyframe_time: Option<Instant>,
    pub stream_start_time: Option<Instant>,

    // Timestamp tracking
    pub last_video_ts: Option<u32>,
    pub last_audio_ts: Option<u32>,
    pub video_ts_rollbacks: u32,
    pub audio_ts_rollbacks: u32,
    pub max_video_ts_gap: u32,
    pub max_audio_ts_gap: u32,
    pub max_av_desync_ms: i64,

    // Metadata
    pub metadata_received: bool,
    pub metadata_has_dimensions: bool,
    pub metadata_has_framerate: bool,
    pub metadata_has_bitrate: bool,

    // Frame analysis
    pub has_b_frames: bool,
    pub keyframe_intervals: Vec<f64>,

    // Collected diagnostics
    diagnostics: Vec<Diagnostic>,
    last_check_time: Option<Instant>,
}

impl StreamDiagnostics {
    pub fn new() -> Self {
        Self {
            profile: ServiceProfile::Generic,
            avc_seq_header_received: false,
            avc_seq_header_time: None,
            aac_seq_header_received: false,
            aac_seq_header_time: None,
            first_keyframe_time: None,
            stream_start_time: None,
            last_video_ts: None,
            last_audio_ts: None,
            video_ts_rollbacks: 0,
            audio_ts_rollbacks: 0,
            max_video_ts_gap: 0,
            max_audio_ts_gap: 0,
            max_av_desync_ms: 0,
            metadata_received: false,
            metadata_has_dimensions: false,
            metadata_has_framerate: false,
            metadata_has_bitrate: false,
            has_b_frames: false,
            keyframe_intervals: Vec::new(),
            diagnostics: Vec::new(),
            last_check_time: None,
        }
    }

    pub fn set_profile(&mut self, profile: ServiceProfile) {
        self.profile = profile;
    }

    pub fn record_stream_start(&mut self) {
        if self.stream_start_time.is_none() {
            self.stream_start_time = Some(Instant::now());
        }
    }

    pub fn record_avc_seq_header(&mut self) {
        if !self.avc_seq_header_received {
            self.avc_seq_header_received = true;
            self.avc_seq_header_time = Some(Instant::now());
        }
    }

    pub fn record_aac_seq_header(&mut self) {
        if !self.aac_seq_header_received {
            self.aac_seq_header_received = true;
            self.aac_seq_header_time = Some(Instant::now());
        }
    }

    pub fn record_keyframe(&mut self, interval_secs: Option<f64>) {
        if self.first_keyframe_time.is_none() {
            self.first_keyframe_time = Some(Instant::now());
        }
        if let Some(interval) = interval_secs {
            self.keyframe_intervals.push(interval);
            // Keep last 10 intervals
            if self.keyframe_intervals.len() > 10 {
                self.keyframe_intervals.remove(0);
            }
        }
    }

    pub fn record_video_timestamp(&mut self, ts: u32) {
        if let Some(last) = self.last_video_ts {
            if ts < last && (last - ts) < 0x80000000 {
                // Rollback detected (not a wraparound)
                self.video_ts_rollbacks += 1;
            } else if ts > last {
                let gap = ts - last;
                if gap > self.max_video_ts_gap {
                    self.max_video_ts_gap = gap;
                }
            }
        }
        self.last_video_ts = Some(ts);
        self.update_av_desync();
    }

    pub fn record_audio_timestamp(&mut self, ts: u32) {
        if let Some(last) = self.last_audio_ts {
            if ts < last && (last - ts) < 0x80000000 {
                self.audio_ts_rollbacks += 1;
            } else if ts > last {
                let gap = ts - last;
                if gap > self.max_audio_ts_gap {
                    self.max_audio_ts_gap = gap;
                }
            }
        }
        self.last_audio_ts = Some(ts);
        self.update_av_desync();
    }

    fn update_av_desync(&mut self) {
        if let (Some(v), Some(a)) = (self.last_video_ts, self.last_audio_ts) {
            let desync = (v as i64) - (a as i64);
            if desync.abs() > self.max_av_desync_ms.abs() {
                self.max_av_desync_ms = desync;
            }
        }
    }

    pub fn record_b_frame(&mut self) {
        self.has_b_frames = true;
    }

    pub fn record_metadata(&mut self, has_dimensions: bool, has_framerate: bool, has_bitrate: bool) {
        self.metadata_received = true;
        self.metadata_has_dimensions = has_dimensions;
        self.metadata_has_framerate = has_framerate;
        self.metadata_has_bitrate = has_bitrate;
    }

    /// Run all diagnostic checks and return warnings
    pub fn check_all(
        &mut self,
        video_width: Option<u32>,
        video_height: Option<u32>,
        video_profile: Option<&str>,
        audio_sample_rate: Option<u32>,
        audio_channels: Option<u8>,
        aac_profile: Option<&str>,
        current_keyframe_interval: Option<f64>,
    ) -> Vec<Diagnostic> {
        // Throttle checks to once per second
        let now = Instant::now();
        if let Some(last) = self.last_check_time {
            if now.duration_since(last).as_millis() < 500 {
                return self.diagnostics.clone();
            }
        }
        self.last_check_time = Some(now);

        self.diagnostics.clear();

        // === SEQUENCE HEADERS ===
        if !self.avc_seq_header_received {
            self.diagnostics.push(Diagnostic::error("Video", "No AVC sequence header received"));
        }
        if !self.aac_seq_header_received {
            self.diagnostics.push(Diagnostic::error("Audio", "No AAC sequence header received"));
        }

        // === FIRST KEYFRAME TIMING ===
        if let Some(start) = self.stream_start_time {
            if self.first_keyframe_time.is_none() {
                let elapsed = now.duration_since(start).as_secs_f64();
                if elapsed > 2.0 {
                    self.diagnostics.push(Diagnostic::warning(
                        "Video",
                        format!("No keyframe received after {:.1}s", elapsed)
                    ));
                }
            } else if let Some(kf_time) = self.first_keyframe_time {
                let delay = kf_time.duration_since(start).as_secs_f64();
                if delay > 1.0 {
                    self.diagnostics.push(Diagnostic::warning(
                        "Video",
                        format!("First keyframe took {:.2}s to arrive", delay)
                    ));
                }
            }
        }

        // === KEYFRAME INTERVAL ===
        if let Some(interval) = current_keyframe_interval {
            let max_interval = match self.profile {
                ServiceProfile::Twitch => 2.0,
                ServiceProfile::YouTube => 4.0,
                ServiceProfile::Generic => 4.0,
            };
            if interval > max_interval {
                self.diagnostics.push(Diagnostic::error(
                    "Video",
                    format!("Keyframe interval {:.1}s exceeds {} max ({:.0}s)",
                        interval, self.profile.name(), max_interval)
                ));
            } else if interval > max_interval * 0.9 {
                self.diagnostics.push(Diagnostic::warning(
                    "Video",
                    format!("Keyframe interval {:.1}s near {} limit ({:.0}s)",
                        interval, self.profile.name(), max_interval)
                ));
            }
        }

        // === B-FRAMES ===
        if self.has_b_frames {
            match self.profile {
                ServiceProfile::Twitch => {
                    self.diagnostics.push(Diagnostic::warning(
                        "Video",
                        "B-frames detected (may increase latency on Twitch)"
                    ));
                }
                _ => {
                    self.diagnostics.push(Diagnostic::info(
                        "Video",
                        "B-frames detected"
                    ));
                }
            }
        }

        // === VIDEO PROFILE ===
        if let Some(profile) = video_profile {
            if profile.contains("Baseline") {
                self.diagnostics.push(Diagnostic::info(
                    "Video",
                    "Baseline profile (consider Main/High for better compression)"
                ));
            }
        }

        // === RESOLUTION ===
        if let (Some(w), Some(h)) = (video_width, video_height) {
            // Check for non-standard resolutions
            let is_standard = matches!(
                (w, h),
                (1920, 1080) | (1280, 720) | (854, 480) | (640, 360) |
                (2560, 1440) | (3840, 2160) | (1080, 1920) | (720, 1280)
            );
            if !is_standard && w % 2 != 0 || h % 2 != 0 {
                self.diagnostics.push(Diagnostic::error(
                    "Video",
                    format!("Resolution {}x{} has odd dimensions (must be even)", w, h)
                ));
            }
        }

        // === AUDIO SAMPLE RATE ===
        if let Some(sr) = audio_sample_rate {
            let allowed = match self.profile {
                ServiceProfile::Twitch => matches!(sr, 44100 | 48000),
                ServiceProfile::YouTube => matches!(sr, 44100 | 48000 | 96000),
                ServiceProfile::Generic => matches!(sr, 22050 | 44100 | 48000 | 96000),
            };
            if !allowed {
                self.diagnostics.push(Diagnostic::error(
                    "Audio",
                    format!("{} Hz sample rate not supported by {}", sr, self.profile.name())
                ));
            }
        }

        // === AUDIO CHANNELS ===
        if let Some(ch) = audio_channels {
            if ch == 1 {
                self.diagnostics.push(Diagnostic::warning(
                    "Audio",
                    "Mono audio (stereo recommended for streaming)"
                ));
            } else if ch > 2 {
                match self.profile {
                    ServiceProfile::Twitch => {
                        self.diagnostics.push(Diagnostic::error(
                            "Audio",
                            format!("{} channels not supported by Twitch (max 2)", ch)
                        ));
                    }
                    _ => {}
                }
            }
        }

        // === AAC PROFILE ===
        if let Some(profile) = aac_profile {
            if profile.contains("Main") {
                self.diagnostics.push(Diagnostic::warning(
                    "Audio",
                    "AAC Main profile (AAC-LC recommended for compatibility)"
                ));
            } else if profile.contains("HE-AAC") || profile.contains("SBR") {
                match self.profile {
                    ServiceProfile::Twitch => {
                        self.diagnostics.push(Diagnostic::warning(
                            "Audio",
                            "HE-AAC may have compatibility issues on Twitch"
                        ));
                    }
                    _ => {}
                }
            }
        }

        // === TIMESTAMP ISSUES ===
        if self.video_ts_rollbacks > 0 {
            self.diagnostics.push(Diagnostic::error(
                "Timing",
                format!("{} video timestamp rollback(s) detected", self.video_ts_rollbacks)
            ));
        }
        if self.audio_ts_rollbacks > 0 {
            self.diagnostics.push(Diagnostic::error(
                "Timing",
                format!("{} audio timestamp rollback(s) detected", self.audio_ts_rollbacks)
            ));
        }

        // Large timestamp gaps (> 1 second = 1000ms)
        if self.max_video_ts_gap > 1000 {
            self.diagnostics.push(Diagnostic::warning(
                "Timing",
                format!("Large video timestamp gap detected ({}ms)", self.max_video_ts_gap)
            ));
        }
        if self.max_audio_ts_gap > 1000 {
            self.diagnostics.push(Diagnostic::warning(
                "Timing",
                format!("Large audio timestamp gap detected ({}ms)", self.max_audio_ts_gap)
            ));
        }

        // A/V desync
        if self.max_av_desync_ms.abs() > 500 {
            self.diagnostics.push(Diagnostic::warning(
                "Timing",
                format!("A/V desync detected ({}ms)", self.max_av_desync_ms)
            ));
        }

        // === METADATA ===
        if !self.metadata_received {
            // Only warn after stream has been going for a bit
            if let Some(start) = self.stream_start_time {
                if now.duration_since(start).as_secs() > 2 {
                    self.diagnostics.push(Diagnostic::warning(
                        "Metadata",
                        "No onMetaData received from encoder"
                    ));
                }
            }
        }

        // Sort by severity (errors first)
        self.diagnostics.sort_by(|a, b| b.severity.cmp(&a.severity));

        self.diagnostics.clone()
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error).count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count()
    }
}
