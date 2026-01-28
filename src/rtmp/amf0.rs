use std::fmt;

#[derive(Debug, Clone)]
pub enum Amf0Value {
    Number(f64),
    Boolean(bool),
    String(String),
    Object(Vec<(String, Amf0Value)>),
    Null,
    Undefined,
    EcmaArray(Vec<(String, Amf0Value)>),
    StrictArray(Vec<Amf0Value>),
}

impl Amf0Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Amf0Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Amf0Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Amf0Value)]> {
        match self {
            Amf0Value::Object(pairs) | Amf0Value::EcmaArray(pairs) => Some(pairs),
            _ => None,
        }
    }

    pub fn get_property(&self, key: &str) -> Option<&Amf0Value> {
        self.as_object().and_then(|pairs| {
            pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        })
    }
}

impl fmt::Display for Amf0Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Amf0Value::Number(n) => write!(f, "{}", n),
            Amf0Value::Boolean(b) => write!(f, "{}", b),
            Amf0Value::String(s) => write!(f, "\"{}\"", s),
            Amf0Value::Object(pairs) | Amf0Value::EcmaArray(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Amf0Value::Null => write!(f, "null"),
            Amf0Value::Undefined => write!(f, "undefined"),
            Amf0Value::StrictArray(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
        }
    }
}

// ── Decoder ──

pub struct Amf0Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Amf0Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[allow(dead_code)]
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn decode(&mut self) -> Option<Amf0Value> {
        if self.pos >= self.data.len() {
            return None;
        }
        let marker = self.data[self.pos];
        self.pos += 1;

        match marker {
            0x00 => self.read_number(),
            0x01 => self.read_boolean(),
            0x02 => self.read_string(),
            0x03 => self.read_object(),
            0x05 => Some(Amf0Value::Null),
            0x06 => Some(Amf0Value::Undefined),
            0x08 => self.read_ecma_array(),
            0x0A => self.read_strict_array(),
            0x0C => self.read_long_string(),
            _ => {
                // Unknown marker — cannot continue decoding
                None
            }
        }
    }

    pub fn decode_all(&mut self) -> Vec<Amf0Value> {
        let mut values = Vec::new();
        while let Some(val) = self.decode() {
            values.push(val);
        }
        values
    }

    fn read_number(&mut self) -> Option<Amf0Value> {
        if self.pos + 8 > self.data.len() {
            return None;
        }
        let bytes: [u8; 8] = self.data[self.pos..self.pos + 8].try_into().ok()?;
        self.pos += 8;
        Some(Amf0Value::Number(f64::from_be_bytes(bytes)))
    }

    fn read_boolean(&mut self) -> Option<Amf0Value> {
        if self.pos >= self.data.len() {
            return None;
        }
        let val = self.data[self.pos] != 0;
        self.pos += 1;
        Some(Amf0Value::Boolean(val))
    }

    fn read_utf8(&mut self) -> Option<String> {
        if self.pos + 2 > self.data.len() {
            return None;
        }
        let len = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]) as usize;
        self.pos += 2;
        if self.pos + len > self.data.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).into_owned();
        self.pos += len;
        Some(s)
    }

    fn read_string(&mut self) -> Option<Amf0Value> {
        self.read_utf8().map(Amf0Value::String)
    }

    fn read_long_string(&mut self) -> Option<Amf0Value> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        let len = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]) as usize;
        self.pos += 4;
        if self.pos + len > self.data.len() {
            return None;
        }
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).into_owned();
        self.pos += len;
        Some(Amf0Value::String(s))
    }

    fn read_object_properties(&mut self) -> Option<Vec<(String, Amf0Value)>> {
        let mut pairs = Vec::new();
        loop {
            // Check for end marker: 0x00 0x00 0x09
            if self.pos + 3 <= self.data.len()
                && self.data[self.pos] == 0x00
                && self.data[self.pos + 1] == 0x00
                && self.data[self.pos + 2] == 0x09
            {
                self.pos += 3;
                break;
            }
            let key = self.read_utf8()?;
            if key.is_empty() {
                // Some encoders use empty key followed by end marker
                if self.pos < self.data.len() && self.data[self.pos] == 0x09 {
                    self.pos += 1;
                    break;
                }
            }
            let value = self.decode()?;
            pairs.push((key, value));
        }
        Some(pairs)
    }

    fn read_object(&mut self) -> Option<Amf0Value> {
        self.read_object_properties().map(Amf0Value::Object)
    }

    fn read_ecma_array(&mut self) -> Option<Amf0Value> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        // Skip the count — it's often inaccurate
        self.pos += 4;
        self.read_object_properties().map(Amf0Value::EcmaArray)
    }

    fn read_strict_array(&mut self) -> Option<Amf0Value> {
        if self.pos + 4 > self.data.len() {
            return None;
        }
        let count = u32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]) as usize;
        self.pos += 4;
        let mut items = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            match self.decode() {
                Some(v) => items.push(v),
                None => break,
            }
        }
        Some(Amf0Value::StrictArray(items))
    }
}

// ── Encoder ──

pub struct Amf0Encoder {
    buf: Vec<u8>,
}

impl Amf0Encoder {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256),
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn write_number(&mut self, val: f64) -> &mut Self {
        self.buf.push(0x00);
        self.buf.extend_from_slice(&val.to_be_bytes());
        self
    }

    pub fn write_boolean(&mut self, val: bool) -> &mut Self {
        self.buf.push(0x01);
        self.buf.push(if val { 1 } else { 0 });
        self
    }

    pub fn write_string(&mut self, val: &str) -> &mut Self {
        self.buf.push(0x02);
        self.write_utf8(val);
        self
    }

    pub fn write_null(&mut self) -> &mut Self {
        self.buf.push(0x05);
        self
    }

    pub fn write_object(&mut self, pairs: &[(&str, Amf0Value)]) -> &mut Self {
        self.buf.push(0x03);
        for (key, value) in pairs {
            self.write_utf8(key);
            self.write_value(value);
        }
        // Object end marker
        self.buf.extend_from_slice(&[0x00, 0x00, 0x09]);
        self
    }

    fn write_utf8(&mut self, val: &str) {
        let len = val.len().min(u16::MAX as usize) as u16;
        self.buf.extend_from_slice(&len.to_be_bytes());
        self.buf.extend_from_slice(&val.as_bytes()[..len as usize]);
    }

    fn write_value(&mut self, val: &Amf0Value) {
        match val {
            Amf0Value::Number(n) => {
                self.write_number(*n);
            }
            Amf0Value::Boolean(b) => {
                self.write_boolean(*b);
            }
            Amf0Value::String(s) => {
                self.write_string(s);
            }
            Amf0Value::Null | Amf0Value::Undefined => {
                self.write_null();
            }
            Amf0Value::Object(pairs) => {
                let refs: Vec<(&str, Amf0Value)> =
                    pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                self.write_object(&refs);
            }
            Amf0Value::EcmaArray(pairs) => {
                // Encode ECMA arrays as objects for responses
                let refs: Vec<(&str, Amf0Value)> =
                    pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                self.write_object(&refs);
            }
            Amf0Value::StrictArray(items) => {
                self.buf.push(0x0A);
                let count = items.len() as u32;
                self.buf.extend_from_slice(&count.to_be_bytes());
                for item in items {
                    self.write_value(item);
                }
            }
        }
    }
}
