use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioCodec {
    LinearPcmPlatformEndian,
    Adpcm,
    Mp3,
    LinearPcmLittleEndian,
    Nellymoser16k,
    Nellymoser8k,
    Nellymoser,
    G711ALaw,
    G711MuLaw,
    Aac,
    Speex,
    Mp3_8k,
    DeviceSpecific,
    Unknown(u8),
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioCodec::LinearPcmPlatformEndian => write!(f, "Linear PCM"),
            AudioCodec::Adpcm => write!(f, "ADPCM"),
            AudioCodec::Mp3 => write!(f, "MP3"),
            AudioCodec::LinearPcmLittleEndian => write!(f, "Linear PCM (LE)"),
            AudioCodec::Nellymoser16k => write!(f, "Nellymoser 16kHz"),
            AudioCodec::Nellymoser8k => write!(f, "Nellymoser 8kHz"),
            AudioCodec::Nellymoser => write!(f, "Nellymoser"),
            AudioCodec::G711ALaw => write!(f, "G.711 A-law"),
            AudioCodec::G711MuLaw => write!(f, "G.711 mu-law"),
            AudioCodec::Aac => write!(f, "AAC"),
            AudioCodec::Speex => write!(f, "Speex"),
            AudioCodec::Mp3_8k => write!(f, "MP3 8kHz"),
            AudioCodec::DeviceSpecific => write!(f, "Device Specific"),
            AudioCodec::Unknown(id) => write!(f, "Unknown ({})", id),
        }
    }
}

impl AudioCodec {
    fn from_id(id: u8) -> Self {
        match id {
            0 => AudioCodec::LinearPcmPlatformEndian,
            1 => AudioCodec::Adpcm,
            2 => AudioCodec::Mp3,
            3 => AudioCodec::LinearPcmLittleEndian,
            4 => AudioCodec::Nellymoser16k,
            5 => AudioCodec::Nellymoser8k,
            6 => AudioCodec::Nellymoser,
            7 => AudioCodec::G711ALaw,
            8 => AudioCodec::G711MuLaw,
            10 => AudioCodec::Aac,
            11 => AudioCodec::Speex,
            14 => AudioCodec::Mp3_8k,
            15 => AudioCodec::DeviceSpecific,
            _ => AudioCodec::Unknown(id),
        }
    }
}

pub struct AudioAnalyzer {
    pub codec: Option<AudioCodec>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub sample_size: Option<u8>,

    // AAC-specific
    pub aac_profile: Option<String>,
    pub asc_sample_rate: Option<u32>,
    pub asc_channels: Option<u8>,
    pub asc_received: bool,

    pub total_audio_bytes: u64,
    pub total_audio_frames: u64,
}

impl AudioAnalyzer {
    pub fn new() -> Self {
        Self {
            codec: None,
            sample_rate: None,
            channels: None,
            sample_size: None,
            aac_profile: None,
            asc_sample_rate: None,
            asc_channels: None,
            asc_received: false,
            total_audio_bytes: 0,
            total_audio_frames: 0,
        }
    }

    /// Get the effective sample rate (ASC overrides FLV header for AAC).
    pub fn effective_sample_rate(&self) -> Option<u32> {
        self.asc_sample_rate.or(self.sample_rate)
    }

    /// Get the effective channel count (ASC overrides FLV header for AAC).
    pub fn effective_channels(&self) -> Option<u8> {
        self.asc_channels.or(self.channels)
    }

    pub fn process(&mut self, data: &[u8], _timestamp: u32) {
        if data.is_empty() {
            return;
        }

        self.total_audio_bytes += data.len() as u64;

        let first_byte = data[0];
        let sound_format = (first_byte >> 4) & 0x0F;
        let sound_rate_idx = (first_byte >> 2) & 0x03;
        let sound_size_flag = (first_byte >> 1) & 0x01;
        let sound_type_flag = first_byte & 0x01;

        self.codec = Some(AudioCodec::from_id(sound_format));
        self.sample_size = Some(if sound_size_flag == 0 { 8 } else { 16 });
        self.channels = Some(if sound_type_flag == 0 { 1 } else { 2 });
        self.sample_rate = Some(match sound_rate_idx {
            0 => 5500,
            1 => 11025,
            2 => 22050,
            3 => 44100,
            _ => 0,
        });

        if sound_format == 10 && data.len() >= 2 {
            // AAC
            let aac_packet_type = data[1];
            match aac_packet_type {
                0 => {
                    // AAC Sequence Header (AudioSpecificConfig)
                    if data.len() >= 4 {
                        self.parse_audio_specific_config(&data[2..]);
                    }
                    return; // Don't count sequence header as audio frame
                }
                1 => {
                    // Raw AAC data
                    self.total_audio_frames += 1;
                }
                _ => {}
            }
        } else {
            self.total_audio_frames += 1;
        }
    }

    fn parse_audio_specific_config(&mut self, data: &[u8]) {
        if data.len() < 2 {
            return;
        }

        let byte0 = data[0];
        let byte1 = data[1];

        // audioObjectType: 5 bits from MSB of byte0
        let audio_object_type = (byte0 >> 3) & 0x1F;

        // samplingFrequencyIndex: 4 bits (lower 3 of byte0 + upper 1 of byte1)
        let sample_freq_index = ((byte0 & 0x07) << 1) | ((byte1 >> 7) & 0x01);

        // channelConfiguration: 4 bits from byte1 bits [6:3]
        let channel_config = (byte1 >> 3) & 0x0F;

        self.aac_profile = Some(match audio_object_type {
            1 => "AAC Main".to_string(),
            2 => "AAC-LC".to_string(),
            3 => "AAC SSR".to_string(),
            4 => "AAC LTP".to_string(),
            5 => "HE-AAC (SBR)".to_string(),
            6 => "AAC Scalable".to_string(),
            23 => "ER AAC LD".to_string(),
            29 => "HE-AAC v2 (SBR+PS)".to_string(),
            39 => "ER AAC ELD".to_string(),
            _ => format!("AAC Object Type {}", audio_object_type),
        });

        const SAMPLE_RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
            7350,
        ];

        if (sample_freq_index as usize) < SAMPLE_RATES.len() {
            self.asc_sample_rate = Some(SAMPLE_RATES[sample_freq_index as usize]);
        }

        self.asc_channels = Some(channel_config);
        self.asc_received = true;
    }
}
