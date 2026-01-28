use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct StreamStats {
    pub stream_start: Option<Instant>,
    pub duration_secs: f64,

    // Rolling window for FPS
    video_frame_times: VecDeque<Instant>,

    // Rolling window for bitrate
    video_byte_window: VecDeque<(Instant, usize)>,
    audio_byte_window: VecDeque<(Instant, usize)>,

    window_duration: Duration,

    // Keyframe interval tracking
    last_keyframe_time: Option<Instant>,
    pub keyframe_interval_secs: Option<f64>,

    // Cumulative
    pub total_video_bytes: u64,
    pub total_audio_bytes: u64,
}

impl StreamStats {
    pub fn new() -> Self {
        Self {
            stream_start: None,
            duration_secs: 0.0,
            video_frame_times: VecDeque::with_capacity(256),
            video_byte_window: VecDeque::with_capacity(256),
            audio_byte_window: VecDeque::with_capacity(256),
            window_duration: Duration::from_secs(2),
            last_keyframe_time: None,
            keyframe_interval_secs: None,
            total_video_bytes: 0,
            total_audio_bytes: 0,
        }
    }

    pub fn record_video_frame(&mut self, byte_count: usize, is_keyframe: bool) {
        let now = Instant::now();
        if self.stream_start.is_none() {
            self.stream_start = Some(now);
        }

        self.video_frame_times.push_back(now);
        self.video_byte_window.push_back((now, byte_count));
        self.total_video_bytes += byte_count as u64;

        // Trim old entries
        let cutoff = now - self.window_duration;
        while self
            .video_frame_times
            .front()
            .map_or(false, |t| *t < cutoff)
        {
            self.video_frame_times.pop_front();
        }
        while self
            .video_byte_window
            .front()
            .map_or(false, |(t, _)| *t < cutoff)
        {
            self.video_byte_window.pop_front();
        }

        if is_keyframe {
            if let Some(last_kf) = self.last_keyframe_time {
                self.keyframe_interval_secs = Some(now.duration_since(last_kf).as_secs_f64());
            }
            self.last_keyframe_time = Some(now);
        }

        self.duration_secs = now.duration_since(self.stream_start.unwrap()).as_secs_f64();
    }

    pub fn record_audio_frame(&mut self, byte_count: usize) {
        let now = Instant::now();
        if self.stream_start.is_none() {
            self.stream_start = Some(now);
        }

        self.audio_byte_window.push_back((now, byte_count));
        self.total_audio_bytes += byte_count as u64;

        let cutoff = now - self.window_duration;
        while self
            .audio_byte_window
            .front()
            .map_or(false, |(t, _)| *t < cutoff)
        {
            self.audio_byte_window.pop_front();
        }

        self.duration_secs = now.duration_since(self.stream_start.unwrap()).as_secs_f64();
    }

    /// Current video FPS over the rolling window.
    pub fn current_fps(&self) -> Option<f64> {
        if self.video_frame_times.len() < 2 {
            return None;
        }
        let first = *self.video_frame_times.front().unwrap();
        let last = *self.video_frame_times.back().unwrap();
        let elapsed = last.duration_since(first).as_secs_f64();
        if elapsed < 0.001 {
            return None;
        }
        Some((self.video_frame_times.len() - 1) as f64 / elapsed)
    }

    /// Video bitrate in kbps over the rolling window.
    pub fn current_video_bitrate_kbps(&self) -> Option<f64> {
        self.rolling_bitrate_kbps(&self.video_byte_window)
    }

    /// Audio bitrate in kbps over the rolling window.
    pub fn current_audio_bitrate_kbps(&self) -> Option<f64> {
        self.rolling_bitrate_kbps(&self.audio_byte_window)
    }

    fn rolling_bitrate_kbps(&self, window: &VecDeque<(Instant, usize)>) -> Option<f64> {
        if window.len() < 2 {
            return None;
        }
        let first_time = window.front().unwrap().0;
        let last_time = window.back().unwrap().0;
        let elapsed = last_time.duration_since(first_time).as_secs_f64();
        if elapsed < 0.001 {
            return None;
        }
        let total_bytes: usize = window.iter().map(|(_, b)| *b).sum();
        Some((total_bytes as f64 * 8.0) / (elapsed * 1000.0))
    }
}
