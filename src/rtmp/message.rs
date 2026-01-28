use crate::rtmp::amf0::{Amf0Decoder, Amf0Encoder, Amf0Value};
use crate::rtmp::chunk::{ChunkWriter, RtmpMessage};

/// Result of processing a single RTMP message.
pub struct HandleResult {
    /// Bytes to send back to the client.
    pub responses: Vec<Vec<u8>>,
    /// If a new chunk size was requested by the client.
    pub new_chunk_size: Option<u32>,
    /// Event raised for the connection handler.
    pub event: Option<RtmpEvent>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum RtmpEvent {
    /// Client connected with app name
    Connected { app_name: String },
    /// Client started publishing
    Publishing {
        app_name: String,
        stream_key: String,
    },
    /// Stream metadata received (onMetaData)
    Metadata {
        properties: Vec<(String, Amf0Value)>,
    },
    /// Video data received
    VideoData { timestamp: u32, data: Vec<u8> },
    /// Audio data received
    AudioData { timestamp: u32, data: Vec<u8> },
    /// Client disconnected / stream ended
    StreamEnded,
}

pub struct MessageHandler {
    writer: ChunkWriter,
    app_name: String,
    stream_key: String,
    window_ack_size: u32,
    bytes_received: u64,
    last_ack_sent: u64,
}

impl MessageHandler {
    pub fn new() -> Self {
        Self {
            writer: ChunkWriter::new(),
            app_name: String::new(),
            stream_key: String::new(),
            window_ack_size: 2500000,
            bytes_received: 0,
            last_ack_sent: 0,
        }
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    pub fn stream_key(&self) -> &str {
        &self.stream_key
    }

    /// Track bytes received for window acknowledgement.
    pub fn track_bytes(&mut self, count: usize) -> Option<Vec<u8>> {
        self.bytes_received += count as u64;
        if self.window_ack_size > 0
            && self.bytes_received - self.last_ack_sent >= self.window_ack_size as u64
        {
            self.last_ack_sent = self.bytes_received;
            let ack = self.writer.write_message(
                2,
                0,
                3, // Acknowledgement
                0,
                &(self.bytes_received as u32).to_be_bytes(),
            );
            Some(ack)
        } else {
            None
        }
    }

    pub fn handle(&mut self, msg: RtmpMessage) -> HandleResult {
        match msg.type_id {
            1 => self.handle_set_chunk_size(&msg),
            3 => HandleResult::empty(), // Acknowledgement — ignore
            4 => self.handle_user_control(&msg),
            5 => self.handle_window_ack_size(&msg),
            6 => self.handle_set_peer_bandwidth(&msg),
            8 => HandleResult::event(RtmpEvent::AudioData {
                timestamp: msg.timestamp,
                data: msg.payload,
            }),
            9 => HandleResult::event(RtmpEvent::VideoData {
                timestamp: msg.timestamp,
                data: msg.payload,
            }),
            18 => self.handle_amf0_data(&msg),
            20 => self.handle_amf0_command(&msg),
            _ => HandleResult::empty(), // Unknown type — silently ignore
        }
    }

    fn handle_set_chunk_size(&self, msg: &RtmpMessage) -> HandleResult {
        if msg.payload.len() >= 4 {
            let size = u32::from_be_bytes([
                msg.payload[0],
                msg.payload[1],
                msg.payload[2],
                msg.payload[3],
            ]);
            HandleResult {
                responses: vec![],
                new_chunk_size: Some(size),
                event: None,
            }
        } else {
            HandleResult::empty()
        }
    }

    fn handle_user_control(&self, msg: &RtmpMessage) -> HandleResult {
        if msg.payload.len() >= 6 {
            let event_type =
                u16::from_be_bytes([msg.payload[0], msg.payload[1]]);
            match event_type {
                6 => {
                    // Ping Request — respond with Pong
                    let mut pong_payload = vec![0u8; 6];
                    pong_payload[0] = 0;
                    pong_payload[1] = 7; // Pong event type
                    pong_payload[2..6].copy_from_slice(&msg.payload[2..6]);
                    let response = self.writer.write_message(2, 0, 4, 0, &pong_payload);
                    HandleResult::response(response)
                }
                _ => HandleResult::empty(),
            }
        } else {
            HandleResult::empty()
        }
    }

    fn handle_window_ack_size(&mut self, msg: &RtmpMessage) -> HandleResult {
        if msg.payload.len() >= 4 {
            self.window_ack_size = u32::from_be_bytes([
                msg.payload[0],
                msg.payload[1],
                msg.payload[2],
                msg.payload[3],
            ]);
        }
        HandleResult::empty()
    }

    fn handle_set_peer_bandwidth(&self, _msg: &RtmpMessage) -> HandleResult {
        // Respond with our Window Ack Size
        let payload = self.window_ack_size.to_be_bytes();
        let response = self.writer.write_message(2, 0, 5, 0, &payload);
        HandleResult::response(response)
    }

    fn handle_amf0_data(&self, msg: &RtmpMessage) -> HandleResult {
        let mut decoder = Amf0Decoder::new(&msg.payload);
        let values = decoder.decode_all();

        // Look for onMetaData / @setDataFrame
        for (i, val) in values.iter().enumerate() {
            if let Some(name) = val.as_str() {
                if name == "@setDataFrame" || name == "onMetaData" {
                    // The metadata object is the next value (or the one after "@setDataFrame" + "onMetaData")
                    let meta_idx = if name == "@setDataFrame" { i + 2 } else { i + 1 };
                    if let Some(meta_val) = values.get(meta_idx).or_else(|| values.get(i + 1)) {
                        if let Some(props) = meta_val.as_object() {
                            return HandleResult::event(RtmpEvent::Metadata {
                                properties: props.to_vec(),
                            });
                        }
                    }
                }
            }
        }

        HandleResult::empty()
    }

    fn handle_amf0_command(&mut self, msg: &RtmpMessage) -> HandleResult {
        let mut decoder = Amf0Decoder::new(&msg.payload);
        let values = decoder.decode_all();

        let command_name = values
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let transaction_id = values.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);

        match command_name.as_str() {
            "connect" => self.handle_connect(&values, transaction_id),
            "releaseStream" => self.handle_release_stream(transaction_id),
            "FCPublish" => self.handle_fc_publish(transaction_id),
            "createStream" => self.handle_create_stream(transaction_id),
            "publish" => self.handle_publish(&values, transaction_id, msg.stream_id),
            "FCUnpublish" | "deleteStream" => {
                HandleResult::event(RtmpEvent::StreamEnded)
            }
            "_checkbw" | "_result" | "_error" | "onStatus" => {
                // Responses/internal commands — ignore
                HandleResult::empty()
            }
            _ => {
                // Unknown command — respond with generic _result to prevent encoder stalls
                self.handle_unknown_command(transaction_id)
            }
        }
    }

    fn handle_connect(&mut self, values: &[Amf0Value], txn_id: f64) -> HandleResult {
        // Extract app name from the command object (3rd value, index 2)
        if let Some(obj) = values.get(2) {
            if let Some(app) = obj.get_property("app") {
                if let Some(name) = app.as_str() {
                    self.app_name = name.to_string();
                }
            }
        }

        let mut responses = Vec::new();

        // 1. Window Acknowledgement Size (type 5)
        let win_ack = self
            .writer
            .write_message(2, 0, 5, 0, &self.window_ack_size.to_be_bytes());
        responses.push(win_ack);

        // 2. Set Peer Bandwidth (type 6): size(4) + limit_type(1)
        let mut peer_bw = self.window_ack_size.to_be_bytes().to_vec();
        peer_bw.push(2); // Dynamic limit
        let peer_bw_msg = self.writer.write_message(2, 0, 6, 0, &peer_bw);
        responses.push(peer_bw_msg);

        // 3. Set Chunk Size (type 1) — we use 4096
        let chunk_size: u32 = 4096;
        let chunk_msg =
            self.writer
                .write_message(2, 0, 1, 0, &chunk_size.to_be_bytes());
        responses.push(chunk_msg);

        // 4. Stream Begin (User Control, type 4): event=0 (StreamBegin), stream_id=0
        let stream_begin = vec![0u8, 0, 0, 0, 0, 0];
        let stream_begin_msg = self.writer.write_message(2, 0, 4, 0, &stream_begin);
        responses.push(stream_begin_msg);

        // 5. _result response
        let mut enc = Amf0Encoder::new();
        enc.write_string("_result");
        enc.write_number(txn_id);
        // Properties object
        enc.write_object(&[
            ("fmsVer", Amf0Value::String("FMS/3,5,7,7009".to_string())),
            ("capabilities", Amf0Value::Number(31.0)),
            ("mode", Amf0Value::Number(1.0)),
        ]);
        // Information object
        enc.write_object(&[
            (
                "level",
                Amf0Value::String("status".to_string()),
            ),
            (
                "code",
                Amf0Value::String("NetConnection.Connect.Success".to_string()),
            ),
            (
                "description",
                Amf0Value::String("Connection succeeded.".to_string()),
            ),
            ("objectEncoding", Amf0Value::Number(0.0)),
        ]);

        let result_msg = self.writer.write_message(3, 0, 20, 0, &enc.into_bytes());
        responses.push(result_msg);

        HandleResult {
            responses,
            new_chunk_size: None,
            event: Some(RtmpEvent::Connected {
                app_name: self.app_name.clone(),
            }),
        }
    }

    fn handle_release_stream(&self, txn_id: f64) -> HandleResult {
        let mut enc = Amf0Encoder::new();
        enc.write_string("_result");
        enc.write_number(txn_id);
        enc.write_null();
        let response = self.writer.write_message(3, 0, 20, 0, &enc.into_bytes());
        HandleResult::response(response)
    }

    fn handle_fc_publish(&self, _txn_id: f64) -> HandleResult {
        let mut enc = Amf0Encoder::new();
        enc.write_string("onFCPublish");
        enc.write_number(0.0);
        enc.write_null();
        let response = self.writer.write_message(3, 0, 20, 0, &enc.into_bytes());
        HandleResult::response(response)
    }

    fn handle_create_stream(&self, txn_id: f64) -> HandleResult {
        let mut enc = Amf0Encoder::new();
        enc.write_string("_result");
        enc.write_number(txn_id);
        enc.write_null();
        enc.write_number(1.0); // Stream ID = 1
        let response = self.writer.write_message(3, 0, 20, 0, &enc.into_bytes());
        HandleResult::response(response)
    }

    fn handle_publish(
        &mut self,
        values: &[Amf0Value],
        _txn_id: f64,
        msg_stream_id: u32,
    ) -> HandleResult {
        // publish command: ["publish", txn, null, stream_key, "live"]
        if let Some(key) = values.get(3).and_then(|v| v.as_str()) {
            self.stream_key = key.to_string();
        }

        let mut responses = Vec::new();

        // Stream Begin for stream ID 1
        let mut stream_begin = vec![0u8; 6];
        stream_begin[0] = 0;
        stream_begin[1] = 0; // StreamBegin event
        stream_begin[4] = 0;
        stream_begin[5] = 1; // stream id = 1
        let sb_msg = self.writer.write_message(2, 0, 4, 0, &stream_begin);
        responses.push(sb_msg);

        // onStatus response
        let mut enc = Amf0Encoder::new();
        enc.write_string("onStatus");
        enc.write_number(0.0);
        enc.write_null();
        enc.write_object(&[
            ("level", Amf0Value::String("status".to_string())),
            (
                "code",
                Amf0Value::String("NetStream.Publish.Start".to_string()),
            ),
            (
                "description",
                Amf0Value::String("Publishing started.".to_string()),
            ),
        ]);
        let status_msg =
            self.writer
                .write_message(3, 0, 20, msg_stream_id, &enc.into_bytes());
        responses.push(status_msg);

        HandleResult {
            responses,
            new_chunk_size: None,
            event: Some(RtmpEvent::Publishing {
                app_name: self.app_name.clone(),
                stream_key: self.stream_key.clone(),
            }),
        }
    }

    fn handle_unknown_command(&self, txn_id: f64) -> HandleResult {
        // Respond with _result(null) to prevent encoder from stalling
        if txn_id > 0.0 {
            let mut enc = Amf0Encoder::new();
            enc.write_string("_result");
            enc.write_number(txn_id);
            enc.write_null();
            let response = self.writer.write_message(3, 0, 20, 0, &enc.into_bytes());
            HandleResult::response(response)
        } else {
            HandleResult::empty()
        }
    }
}

impl HandleResult {
    pub fn empty() -> Self {
        Self {
            responses: vec![],
            new_chunk_size: None,
            event: None,
        }
    }

    pub fn response(data: Vec<u8>) -> Self {
        Self {
            responses: vec![data],
            new_chunk_size: None,
            event: None,
        }
    }

    pub fn event(evt: RtmpEvent) -> Self {
        Self {
            responses: vec![],
            new_chunk_size: None,
            event: Some(evt),
        }
    }
}
