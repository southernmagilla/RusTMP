use std::collections::HashMap;

/// A fully reassembled RTMP message.
#[derive(Debug, Clone)]
pub struct RtmpMessage {
    pub timestamp: u32,
    pub type_id: u8,
    pub stream_id: u32,
    pub payload: Vec<u8>,
}

/// Per-chunk-stream state for reassembly.
#[derive(Debug, Clone)]
struct ChunkStreamState {
    timestamp: u32,
    timestamp_delta: u32,
    message_length: u32,
    type_id: u8,
    stream_id: u32,
    // Accumulation buffer for the current message being reassembled
    buffer: Vec<u8>,
}

impl Default for ChunkStreamState {
    fn default() -> Self {
        Self {
            timestamp: 0,
            timestamp_delta: 0,
            message_length: 0,
            type_id: 0,
            stream_id: 0,
            buffer: Vec::new(),
        }
    }
}

/// Reads RTMP chunks from a byte buffer and reassembles them into messages.
pub struct ChunkReader {
    states: HashMap<u32, ChunkStreamState>,
    max_chunk_size: usize,
    buf: Vec<u8>,
}

impl ChunkReader {
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            max_chunk_size: 128,
            buf: Vec::with_capacity(65536),
        }
    }

    pub fn set_chunk_size(&mut self, size: u32) {
        self.max_chunk_size = size as usize;
    }

    /// Append incoming bytes to the internal buffer.
    pub fn extend(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Try to read complete messages from the buffer.
    /// Returns all complete messages that can be assembled.
    pub fn read_messages(&mut self) -> Vec<RtmpMessage> {
        let mut messages = Vec::new();

        loop {
            match self.try_read_chunk() {
                Some(msg) => {
                    if let Some(m) = msg {
                        messages.push(m);
                    }
                    // Chunk was consumed, possibly produced a message; try again
                }
                None => break, // Not enough data
            }
        }

        messages
    }

    /// Try to read one chunk. Returns:
    /// - Some(Some(msg)) if a chunk was read and completed a message
    /// - Some(None) if a chunk was read but message is still incomplete
    /// - None if there's not enough data to read a chunk
    fn try_read_chunk(&mut self) -> Option<Option<RtmpMessage>> {
        let mut pos = 0;

        if pos >= self.buf.len() {
            return None;
        }

        // ── Basic Header (1-3 bytes) ──
        let first_byte = self.buf[pos];
        pos += 1;

        let fmt = (first_byte >> 6) & 0x03;
        let cs_id_low = first_byte & 0x3F;

        let cs_id = match cs_id_low {
            0 => {
                // 2-byte form: cs_id = byte[1] + 64
                if pos >= self.buf.len() {
                    return None;
                }
                let id = self.buf[pos] as u32 + 64;
                pos += 1;
                id
            }
            1 => {
                // 3-byte form: cs_id = byte[1] + byte[2]*256 + 64
                if pos + 1 >= self.buf.len() {
                    return None;
                }
                let id = self.buf[pos] as u32 + self.buf[pos + 1] as u32 * 256 + 64;
                pos += 2;
                id
            }
            _ => cs_id_low as u32,
        };

        // ── Message Header (0/3/7/11 bytes depending on fmt) ──
        let header_size = match fmt {
            0 => 11,
            1 => 7,
            2 => 3,
            3 => 0,
            _ => unreachable!(),
        };

        if pos + header_size > self.buf.len() {
            return None;
        }

        let state = self.states.entry(cs_id).or_default();

        #[allow(unused_assignments)]
        let mut timestamp_field: u32 = 0;

        match fmt {
            0 => {
                // Full header: timestamp(3) + message_length(3) + type_id(1) + stream_id(4, little-endian)
                timestamp_field = (self.buf[pos] as u32) << 16
                    | (self.buf[pos + 1] as u32) << 8
                    | self.buf[pos + 2] as u32;
                state.message_length = (self.buf[pos + 3] as u32) << 16
                    | (self.buf[pos + 4] as u32) << 8
                    | self.buf[pos + 5] as u32;
                state.type_id = self.buf[pos + 6];
                // Stream ID is little-endian
                state.stream_id = u32::from_le_bytes([
                    self.buf[pos + 7],
                    self.buf[pos + 8],
                    self.buf[pos + 9],
                    self.buf[pos + 10],
                ]);
                pos += 11;
            }
            1 => {
                // timestamp_delta(3) + message_length(3) + type_id(1)
                timestamp_field = (self.buf[pos] as u32) << 16
                    | (self.buf[pos + 1] as u32) << 8
                    | self.buf[pos + 2] as u32;
                state.message_length = (self.buf[pos + 3] as u32) << 16
                    | (self.buf[pos + 4] as u32) << 8
                    | self.buf[pos + 5] as u32;
                state.type_id = self.buf[pos + 6];
                pos += 7;
            }
            2 => {
                // timestamp_delta(3) only
                timestamp_field = (self.buf[pos] as u32) << 16
                    | (self.buf[pos + 1] as u32) << 8
                    | self.buf[pos + 2] as u32;
                pos += 3;
            }
            3 => {
                // No header bytes — reuse everything
                // For fmt 3, we reuse the previous timestamp delta
                timestamp_field = state.timestamp_delta;
            }
            _ => unreachable!(),
        }

        // ── Extended Timestamp ──
        let has_extended = if fmt == 0 {
            timestamp_field == 0xFFFFFF
        } else {
            // For fmt 1/2/3, extended timestamp present if the stored delta was 0xFFFFFF
            timestamp_field == 0xFFFFFF
        };

        if has_extended {
            if pos + 4 > self.buf.len() {
                return None;
            }
            let ext = u32::from_be_bytes([
                self.buf[pos],
                self.buf[pos + 1],
                self.buf[pos + 2],
                self.buf[pos + 3],
            ]);
            pos += 4;
            timestamp_field = ext;
        }

        // Update timestamp
        match fmt {
            0 => {
                state.timestamp = timestamp_field;
                state.timestamp_delta = 0;
            }
            1 | 2 => {
                state.timestamp_delta = timestamp_field;
                state.timestamp = state.timestamp.wrapping_add(timestamp_field);
            }
            3 => {
                state.timestamp = state.timestamp.wrapping_add(state.timestamp_delta);
            }
            _ => {}
        }

        // Start a new message buffer if this is the first chunk of a message
        if state.buffer.is_empty() && state.message_length > 0 {
            state.buffer.reserve(state.message_length as usize);
        }

        // ── Chunk Data ──
        let remaining_in_message = (state.message_length as usize).saturating_sub(state.buffer.len());
        let chunk_data_size = remaining_in_message.min(self.max_chunk_size);

        if pos + chunk_data_size > self.buf.len() {
            return None;
        }

        state
            .buffer
            .extend_from_slice(&self.buf[pos..pos + chunk_data_size]);
        pos += chunk_data_size;

        // Consume the bytes we've processed
        self.buf.drain(..pos);

        // Check if message is complete
        if state.buffer.len() >= state.message_length as usize {
            let msg = RtmpMessage {
                timestamp: state.timestamp,
                type_id: state.type_id,
                stream_id: state.stream_id,
                payload: std::mem::take(&mut state.buffer),
            };
            Some(Some(msg))
        } else {
            Some(None)
        }
    }
}

/// Writes RTMP messages as chunks.
pub struct ChunkWriter {
    chunk_size: usize,
}

impl ChunkWriter {
    pub fn new() -> Self {
        Self { chunk_size: 4096 }
    }

    /// Serialize a message into RTMP chunks.
    /// Always uses fmt=0 (full header) for simplicity and maximum compatibility.
    pub fn write_message(
        &self,
        cs_id: u32,
        timestamp: u32,
        type_id: u8,
        stream_id: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let msg_len = payload.len();
        let mut out = Vec::with_capacity(msg_len + 64);

        let mut offset = 0;
        let mut first_chunk = true;

        while offset < msg_len || first_chunk {
            let chunk_payload_size = (msg_len - offset).min(self.chunk_size);

            if first_chunk {
                // Format 0 basic header + message header
                self.write_basic_header(&mut out, 0, cs_id);
                self.write_fmt0_header(&mut out, timestamp, msg_len as u32, type_id, stream_id);
                first_chunk = false;
            } else {
                // Format 3 (continuation) — just the basic header
                self.write_basic_header(&mut out, 3, cs_id);
                // If extended timestamp was used in fmt 0, include it in fmt 3 too
                if timestamp >= 0xFFFFFF {
                    out.extend_from_slice(&timestamp.to_be_bytes());
                }
            }

            out.extend_from_slice(&payload[offset..offset + chunk_payload_size]);
            offset += chunk_payload_size;

            if msg_len == 0 {
                break;
            }
        }

        out
    }

    fn write_basic_header(&self, out: &mut Vec<u8>, fmt: u8, cs_id: u32) {
        if cs_id >= 2 && cs_id <= 63 {
            out.push((fmt << 6) | cs_id as u8);
        } else if cs_id >= 64 && cs_id <= 319 {
            out.push(fmt << 6); // cs_id_low = 0
            out.push((cs_id - 64) as u8);
        } else {
            out.push((fmt << 6) | 1); // cs_id_low = 1
            let adjusted = cs_id - 64;
            out.push(adjusted as u8);
            out.push((adjusted >> 8) as u8);
        }
    }

    fn write_fmt0_header(
        &self,
        out: &mut Vec<u8>,
        timestamp: u32,
        msg_length: u32,
        type_id: u8,
        stream_id: u32,
    ) {
        // Timestamp (3 bytes) — use 0xFFFFFF if extended
        if timestamp >= 0xFFFFFF {
            out.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
        } else {
            out.push((timestamp >> 16) as u8);
            out.push((timestamp >> 8) as u8);
            out.push(timestamp as u8);
        }

        // Message length (3 bytes, big-endian)
        out.push((msg_length >> 16) as u8);
        out.push((msg_length >> 8) as u8);
        out.push(msg_length as u8);

        // Type ID (1 byte)
        out.push(type_id);

        // Stream ID (4 bytes, little-endian)
        out.extend_from_slice(&stream_id.to_le_bytes());

        // Extended timestamp (4 bytes) if needed
        if timestamp >= 0xFFFFFF {
            out.extend_from_slice(&timestamp.to_be_bytes());
        }
    }
}
