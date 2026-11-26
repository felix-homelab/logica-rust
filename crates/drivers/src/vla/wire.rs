//! The virtual analyser's wire protocol, version 1 (`tools/virtual-analyser/PROTOCOL.md`).
//!
//! Text lines carry the handshake: the analyser greets with a [`Hello`] line, the client answers
//! with a [`Request`] line. While capturing, the analyser sends binary messages: a kind byte, a
//! little-endian `u32` payload length and the payload. Both ends use this module, so it is the one
//! place the format is written down in code.

use serde::{Deserialize, Serialize};

/// Protocol name in the greeting.
pub const PROTOCOL: &str = "logica-vla";
pub const VERSION: u32 = 1;
/// Longest payload either end accepts: a larger length means a broken stream.
pub const MAX_PAYLOAD: usize = 16 << 20;
/// Longest text line accepted.
pub const MAX_LINE: usize = 64 << 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloChannel {
    pub index: u32,
    pub name: String,
}

/// The analyser's greeting, sent as one JSON line as soon as a client connects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    pub version: u32,
    pub model: String,
    pub serial: String,
    pub channels: Vec<HelloChannel>,
    pub sample_rates_hz: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Start {
    /// Enabled physical channels, ascending.
    pub channels: Vec<u32>,
    pub sample_rate_hz: f64,
    /// Samples per channel; the capture ends there unless stopped earlier.
    pub samples: u64,
}

/// What a client sends, one JSON line each: `{"start":{…}}`, then optionally `{"stop":true}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    Start(Start),
    Stop(bool),
}

/// A binary message from the analyser while capturing.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// The capture began; the level of every enabled channel at sample 0, in slot order.
    Begin { initial: Vec<bool> },
    /// Level changes of one enabled channel, ascending, continuing its earlier edges.
    Edges { slot: u16, samples: Vec<u64> },
    /// Every enabled channel is known up to `end` (exclusive).
    Advance { end: u64 },
    /// Something worth telling the person.
    Note { text: String },
    /// The capture is over at `samples`; the analyser closes the connection.
    End { samples: u64 },
    /// The capture was refused or failed; the analyser closes the connection.
    Error { text: String },
}

const BEGIN: u8 = 1;
const EDGES: u8 = 2;
const ADVANCE: u8 = 3;
const NOTE: u8 = 4;
const END: u8 = 5;
const ERROR: u8 = 6;

fn frame(kind: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push(kind);
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
}

impl Message {
    /// Appends the message's bytes to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Message::Begin { initial } => {
                let p: Vec<u8> = initial.iter().map(|&b| u8::from(b)).collect();
                frame(BEGIN, &p, out);
            }
            Message::Edges { slot, samples } => {
                let mut p = Vec::with_capacity(2 + samples.len() * 8);
                p.extend_from_slice(&slot.to_le_bytes());
                for s in samples {
                    p.extend_from_slice(&s.to_le_bytes());
                }
                frame(EDGES, &p, out);
            }
            Message::Advance { end } => frame(ADVANCE, &end.to_le_bytes(), out),
            Message::Note { text } => frame(NOTE, text.as_bytes(), out),
            Message::End { samples } => frame(END, &samples.to_le_bytes(), out),
            Message::Error { text } => frame(ERROR, text.as_bytes(), out),
        }
    }

    /// Reads one message from the front of `buf`: `Ok(None)` while it is incomplete, otherwise the
    /// message and the number of bytes it took.
    pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>, String> {
        if buf.len() < 5 {
            return Ok(None);
        }
        let kind = buf[0];
        let len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(format!(
                "The analyser sent a message of {len} bytes; the most allowed is {MAX_PAYLOAD}."
            ));
        }
        if buf.len() < 5 + len {
            return Ok(None);
        }
        let p = &buf[5..5 + len];
        let u64_of = |p: &[u8]| -> Result<u64, String> {
            let b: [u8; 8] = p.try_into().map_err(|_| "A sample number must be 8 bytes.".to_string())?;
            Ok(u64::from_le_bytes(b))
        };
        let text = |p: &[u8]| String::from_utf8_lossy(p).into_owned();
        let m = match kind {
            BEGIN => Message::Begin { initial: p.iter().map(|&b| b != 0).collect() },
            EDGES => {
                if p.len() < 2 || !(p.len() - 2).is_multiple_of(8) {
                    return Err("An edge message must hold a slot and whole 8-byte samples.".into());
                }
                let slot = u16::from_le_bytes([p[0], p[1]]);
                let samples = p[2..].as_chunks::<8>().0.iter().map(|b| u64::from_le_bytes(*b)).collect();
                Message::Edges { slot, samples }
            }
            ADVANCE => Message::Advance { end: u64_of(p)? },
            NOTE => Message::Note { text: text(p) },
            END => Message::End { samples: u64_of(p)? },
            ERROR => Message::Error { text: text(p) },
            k => return Err(format!("The analyser sent a message of unknown kind {k}.")),
        };
        Ok(Some((m, 5 + len)))
    }
}

/// One JSON line, with its newline.
pub fn line<T: Serialize>(value: &T) -> Vec<u8> {
    let mut v = serde_json::to_vec(value).unwrap_or_default();
    v.push(b'\n');
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_a_round_trip_split_anywhere() {
        let all = vec![
            Message::Begin { initial: vec![true, false, true] },
            Message::Edges { slot: 2, samples: vec![5, 9, u64::MAX] },
            Message::Edges { slot: 0, samples: vec![] },
            Message::Advance { end: 1 << 40 },
            Message::Note { text: "größer".into() },
            Message::End { samples: 77 },
            Message::Error { text: "no".into() },
        ];
        let mut bytes = Vec::new();
        for m in &all {
            m.encode(&mut bytes);
        }
        // fed one byte at a time, as a slow socket would
        let mut buf = Vec::new();
        let mut got = Vec::new();
        for b in bytes {
            buf.push(b);
            while let Some((m, n)) = Message::decode(&buf).unwrap() {
                got.push(m);
                buf.drain(..n);
            }
        }
        assert_eq!(got, all);
        assert!(buf.is_empty());
    }

    #[test]
    fn broken_messages_are_explained() {
        let mut b = vec![EDGES];
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[0, 0, 1]);
        assert!(Message::decode(&b).unwrap_err().contains("whole 8-byte samples"));
        let mut b = vec![99];
        b.extend_from_slice(&0u32.to_le_bytes());
        assert!(Message::decode(&b).unwrap_err().contains("unknown kind 99"));
        let mut b = vec![NOTE];
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Message::decode(&b).unwrap_err().contains("most allowed"));
        let mut b = vec![ADVANCE];
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        assert!(Message::decode(&b).unwrap_err().contains("8 bytes"));
    }

    #[test]
    fn requests_are_one_json_line_each() {
        let s = Request::Start(Start { channels: vec![0, 3], sample_rate_hz: 1e6, samples: 10 });
        assert_eq!(
            String::from_utf8(line(&s)).unwrap(),
            "{\"start\":{\"channels\":[0,3],\"sampleRateHz\":1000000.0,\"samples\":10}}\n"
        );
        let stop: Request = serde_json::from_str("{\"stop\":true}").unwrap();
        assert_eq!(stop, Request::Stop(true));
    }
}
