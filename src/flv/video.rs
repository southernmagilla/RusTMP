use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoCodec {
    H263,
    Screen,
    VP6,
    VP6Alpha,
    ScreenV2,
    Avc, // H.264
    Unknown(u8),
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideoCodec::H263 => write!(f, "Sorenson H.263"),
            VideoCodec::Screen => write!(f, "Screen Video"),
            VideoCodec::VP6 => write!(f, "VP6"),
            VideoCodec::VP6Alpha => write!(f, "VP6 Alpha"),
            VideoCodec::ScreenV2 => write!(f, "Screen Video V2"),
            VideoCodec::Avc => write!(f, "H.264/AVC"),
            VideoCodec::Unknown(id) => write!(f, "Unknown ({})", id),
        }
    }
}

impl VideoCodec {
    fn from_id(id: u8) -> Self {
        match id {
            2 => VideoCodec::H263,
            3 => VideoCodec::Screen,
            4 => VideoCodec::VP6,
            5 => VideoCodec::VP6Alpha,
            6 => VideoCodec::ScreenV2,
            7 => VideoCodec::Avc,
            _ => VideoCodec::Unknown(id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    Keyframe,
    Inter,
    DisposableInter,
    GeneratedKeyframe,
    VideoInfo,
    Unknown(u8),
}

pub struct VideoAnalyzer {
    pub codec: Option<VideoCodec>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub profile: Option<String>,
    pub level: Option<String>,

    pub avc_config_received: bool,
    nalu_length_size: u8,

    pub keyframe_count: u64,
    pub inter_frame_count: u64,
    pub b_frame_count: u64,
    pub total_video_frames: u64,
    pub total_video_bytes: u64,
}

impl VideoAnalyzer {
    pub fn new() -> Self {
        Self {
            codec: None,
            width: None,
            height: None,
            profile: None,
            level: None,
            avc_config_received: false,
            nalu_length_size: 4,
            keyframe_count: 0,
            inter_frame_count: 0,
            b_frame_count: 0,
            total_video_frames: 0,
            total_video_bytes: 0,
        }
    }

    pub fn process(&mut self, data: &[u8], _timestamp: u32) {
        if data.is_empty() {
            return;
        }

        self.total_video_bytes += data.len() as u64;

        let first_byte = data[0];
        let frame_type_id = (first_byte >> 4) & 0x0F;
        let codec_id = first_byte & 0x0F;

        let codec = VideoCodec::from_id(codec_id);
        self.codec = Some(codec);

        let frame_type = match frame_type_id {
            1 => FrameType::Keyframe,
            2 => FrameType::Inter,
            3 => FrameType::DisposableInter,
            4 => FrameType::GeneratedKeyframe,
            5 => FrameType::VideoInfo,
            _ => FrameType::Unknown(frame_type_id),
        };

        // Don't count info/command frames
        if matches!(frame_type, FrameType::VideoInfo) {
            return;
        }

        if codec == VideoCodec::Avc && data.len() >= 5 {
            let avc_packet_type = data[1];
            let composition_time = ((data[2] as i32) << 16)
                | ((data[3] as i32) << 8)
                | (data[4] as i32);
            // Sign-extend from 24-bit
            let composition_time = if composition_time & 0x800000 != 0 {
                composition_time | !0xFFFFFF_u32 as i32
            } else {
                composition_time
            };

            match avc_packet_type {
                0 => {
                    // AVC Sequence Header
                    if data.len() > 5 {
                        self.parse_avc_sequence_header(&data[5..]);
                    }
                    return; // Don't count sequence headers as frames
                }
                1 => {
                    // AVC NALU — count frames
                    self.total_video_frames += 1;

                    match frame_type {
                        FrameType::Keyframe | FrameType::GeneratedKeyframe => {
                            self.keyframe_count += 1;
                        }
                        FrameType::Inter | FrameType::DisposableInter => {
                            if composition_time != 0 {
                                self.b_frame_count += 1;
                            } else {
                                self.inter_frame_count += 1;
                            }
                        }
                        _ => {}
                    }
                }
                2 => {
                    // End of sequence
                    return;
                }
                _ => {}
            }
        } else {
            // Non-AVC codec — just count frames
            self.total_video_frames += 1;
            match frame_type {
                FrameType::Keyframe | FrameType::GeneratedKeyframe => {
                    self.keyframe_count += 1;
                }
                FrameType::Inter | FrameType::DisposableInter => {
                    self.inter_frame_count += 1;
                }
                _ => {}
            }
        }
    }

    fn parse_avc_sequence_header(&mut self, data: &[u8]) {
        // AVCDecoderConfigurationRecord
        if data.len() < 6 {
            return;
        }

        let _config_version = data[0]; // should be 1
        let profile_idc = data[1];
        let _profile_compat = data[2];
        let level_idc = data[3];
        self.nalu_length_size = (data[4] & 0x03) + 1;
        let num_sps = (data[5] & 0x1F) as usize;

        // Set profile/level from the config record directly
        self.profile = Some(h264_profile_name(profile_idc));
        self.level = Some(format!("{}.{}", level_idc / 10, level_idc % 10));

        let mut offset = 6;
        for _ in 0..num_sps {
            if offset + 2 > data.len() {
                return;
            }
            let sps_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;
            if offset + sps_len > data.len() {
                return;
            }

            let sps_nalu = &data[offset..offset + sps_len];
            self.parse_sps(sps_nalu);
            offset += sps_len;
        }

        self.avc_config_received = true;
    }

    fn parse_sps(&mut self, nalu: &[u8]) {
        if nalu.is_empty() {
            return;
        }

        // Remove emulation prevention bytes (0x00 0x00 0x03 → 0x00 0x00)
        let rbsp = remove_emulation_prevention(nalu);

        // Skip NAL header byte (forbidden_zero_bit + nal_ref_idc + nal_unit_type)
        if rbsp.len() < 2 {
            return;
        }
        let mut reader = BitstreamReader::new(&rbsp[1..]);

        // profile_idc
        let profile_idc = reader.read_bits(8) as u8;
        // constraint_set0..5_flags + reserved_zero_2bits
        let _constraints = reader.read_bits(8);
        // level_idc
        let level_idc = reader.read_bits(8) as u8;
        // seq_parameter_set_id
        let _sps_id = reader.read_exp_golomb();

        // Update profile/level from the actual SPS data
        self.profile = Some(h264_profile_name(profile_idc));
        self.level = Some(format!("{}.{}", level_idc / 10, level_idc % 10));

        // High profile and above have additional fields
        if matches!(
            profile_idc,
            100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
        ) {
            let chroma_format_idc = reader.read_exp_golomb();
            if chroma_format_idc == 3 {
                let _separate_colour_plane = reader.read_bits(1);
            }
            let _bit_depth_luma = reader.read_exp_golomb(); // + 8
            let _bit_depth_chroma = reader.read_exp_golomb(); // + 8
            let _qpprime_y_zero = reader.read_bits(1);
            let scaling_matrix_present = reader.read_bits(1);
            if scaling_matrix_present != 0 {
                let count = if chroma_format_idc != 3 { 8 } else { 12 };
                for i in 0..count {
                    let present = reader.read_bits(1);
                    if present != 0 {
                        let size = if i < 6 { 16 } else { 64 };
                        skip_scaling_list(&mut reader, size);
                    }
                }
            }
        }

        // log2_max_frame_num_minus4
        let _log2_max_frame_num = reader.read_exp_golomb();
        // pic_order_cnt_type
        let poc_type = reader.read_exp_golomb();
        match poc_type {
            0 => {
                let _log2_max_poc_lsb = reader.read_exp_golomb();
            }
            1 => {
                let _delta_pic_order_always_zero = reader.read_bits(1);
                let _offset_for_non_ref = reader.read_signed_exp_golomb();
                let _offset_for_top = reader.read_signed_exp_golomb();
                let num_ref_frames_in_poc = reader.read_exp_golomb();
                for _ in 0..num_ref_frames_in_poc {
                    let _offset = reader.read_signed_exp_golomb();
                }
            }
            _ => {}
        }

        // max_num_ref_frames
        let _max_ref_frames = reader.read_exp_golomb();
        // gaps_in_frame_num_value_allowed_flag
        let _gaps = reader.read_bits(1);

        // pic_width_in_mbs_minus1
        let pic_width_mbs = reader.read_exp_golomb() + 1;
        // pic_height_in_map_units_minus1
        let pic_height_map_units = reader.read_exp_golomb() + 1;
        // frame_mbs_only_flag
        let frame_mbs_only = reader.read_bits(1);

        if frame_mbs_only == 0 {
            // mb_adaptive_frame_field_flag
            let _mb_adaptive = reader.read_bits(1);
        }

        // direct_8x8_inference_flag
        let _direct_8x8 = reader.read_bits(1);

        // frame_cropping_flag
        let cropping = reader.read_bits(1);
        let (crop_left, crop_right, crop_top, crop_bottom) = if cropping != 0 {
            (
                reader.read_exp_golomb(),
                reader.read_exp_golomb(),
                reader.read_exp_golomb(),
                reader.read_exp_golomb(),
            )
        } else {
            (0, 0, 0, 0)
        };

        // Calculate dimensions
        let width = pic_width_mbs * 16;
        let height = pic_height_map_units * 16 * (2 - frame_mbs_only as u64);

        // Apply cropping (crop units depend on chroma format, default 4:2:0 → cropUnitX=2, cropUnitY=2*(2-frame_mbs_only))
        let crop_unit_x: u64 = 2;
        let crop_unit_y: u64 = 2 * (2 - frame_mbs_only as u64);

        let final_width = width - crop_unit_x * (crop_left + crop_right);
        let final_height = height - crop_unit_y * (crop_top + crop_bottom);

        self.width = Some(final_width as u32);
        self.height = Some(final_height as u32);
    }
}

fn h264_profile_name(profile_idc: u8) -> String {
    match profile_idc {
        66 => "Baseline".to_string(),
        77 => "Main".to_string(),
        88 => "Extended".to_string(),
        100 => "High".to_string(),
        110 => "High 10".to_string(),
        122 => "High 4:2:2".to_string(),
        244 => "High 4:4:4 Predictive".to_string(),
        44 => "CAVLC 4:4:4 Intra".to_string(),
        83 => "Scalable Baseline".to_string(),
        86 => "Scalable High".to_string(),
        118 => "Multiview High".to_string(),
        128 => "Stereo High".to_string(),
        138 => "Multiview Depth High".to_string(),
        _ => format!("Profile {}", profile_idc),
    }
}

fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if i + 2 < data.len() && data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x03 {
            rbsp.push(0x00);
            rbsp.push(0x00);
            i += 3; // Skip the emulation prevention byte
        } else {
            rbsp.push(data[i]);
            i += 1;
        }
    }
    rbsp
}

fn skip_scaling_list(reader: &mut BitstreamReader, size: usize) {
    let mut last_scale: i64 = 8;
    let mut next_scale: i64 = 8;
    for _ in 0..size {
        if next_scale != 0 {
            let delta = reader.read_signed_exp_golomb();
            next_scale = (last_scale + delta + 256) % 256;
        }
        last_scale = if next_scale == 0 {
            last_scale
        } else {
            next_scale
        };
    }
}

// ── Bitstream Reader (for H.264 SPS parsing) ──

struct BitstreamReader<'a> {
    data: &'a [u8],
    byte_offset: usize,
    bit_offset: u8, // 0-7, bits consumed in current byte
}

impl<'a> BitstreamReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_offset: 0,
            bit_offset: 0,
        }
    }

    fn read_bits(&mut self, count: u8) -> u64 {
        let mut value: u64 = 0;
        for _ in 0..count {
            if self.byte_offset >= self.data.len() {
                return value;
            }
            let bit = (self.data[self.byte_offset] >> (7 - self.bit_offset)) & 1;
            value = (value << 1) | bit as u64;
            self.bit_offset += 1;
            if self.bit_offset == 8 {
                self.bit_offset = 0;
                self.byte_offset += 1;
            }
        }
        value
    }

    /// Read unsigned Exp-Golomb coded value.
    fn read_exp_golomb(&mut self) -> u64 {
        let mut leading_zeros: u32 = 0;
        while self.read_bits(1) == 0 {
            leading_zeros += 1;
            if leading_zeros > 32 {
                return 0; // Safety limit
            }
        }
        if leading_zeros == 0 {
            return 0;
        }
        let suffix = self.read_bits(leading_zeros as u8);
        (1u64 << leading_zeros) - 1 + suffix
    }

    /// Read signed Exp-Golomb coded value.
    fn read_signed_exp_golomb(&mut self) -> i64 {
        let code = self.read_exp_golomb();
        if code == 0 {
            return 0;
        }
        let value = ((code + 1) / 2) as i64;
        if code % 2 == 0 {
            -value
        } else {
            value
        }
    }
}
