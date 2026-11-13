//! The byte analyser (doc 06): what a run of decoded bytes might mean — text, text hidden by a
//! wrong setting, Modbus RTU, checksums, BCD, fills, 16-bit pairs, an I2C address byte.

use serde::{Deserialize, Serialize};

use crate::format::as_text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingId {
    Text,
    TransformedText,
    Modbus,
    ChecksumSum,
    ChecksumXor,
    Bcd,
    AllFf,
    All00,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Info,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: FindingId,
    pub tone: Tone,
    pub title: String,
    pub body: String,
    /// Monospace excerpt (decoded text, BCD digits).
    pub mono: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRow {
    pub hex: String,
    pub big_endian: u16,
    pub big_endian_signed: i16,
    pub little_endian: u16,
    pub little_endian_signed: i16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct I2cByte {
    pub address: u8,
    pub read: bool,
    pub device: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteAnalysis {
    pub bytes: Vec<u8>,
    pub is_text: bool,
    pub findings: Vec<Finding>,
    /// Pairs read as 16-bit values, when the bytes are not text.
    pub pairs: Option<Vec<PairRow>>,
    /// The first byte read as an I2C address byte, when that makes sense.
    pub i2c_first_byte: Option<I2cByte>,
}

/// Hex bytes typed by a person: spaces, commas or semicolons between them, `0x` prefixes and `h`
/// suffixes allowed.
pub fn parse_hex_bytes(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for raw in input.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let mut t = raw;
        if t.len() > 2 && (t.starts_with("0x") || t.starts_with("0X")) {
            t = &t[2..];
        }
        if t.len() > 1 && (t.ends_with('h') || t.ends_with('H')) {
            t = &t[..t.len() - 1];
        }
        if !(1..=2).contains(&t.len()) || !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "\"{t}\" is not a hex byte. Use two hex digits per byte, such as 4F or 0x4F."
            ));
        }
        out.push(u8::from_str_radix(t, 16).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn is_printable(b: u8) -> bool {
    (0x20..=0x7e).contains(&b) || b == 0x0d || b == 0x0a || b == 0x09
}

pub fn printable_ratio(a: &[u8]) -> f64 {
    if a.is_empty() { 0.0 } else { a.iter().filter(|&&b| is_printable(b)).count() as f64 / a.len() as f64 }
}

fn wordy(a: &[u8]) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    a.iter().filter(|&&b| b.is_ascii_alphanumeric() || b == b' ').count() as f64 / a.len() as f64
}

pub fn is_bcd(a: &[u8]) -> bool {
    a.iter().all(|&b| b >> 4 <= 9 && (b & 15) <= 9)
}

pub fn reverse_bits8(b: u8) -> u8 {
    b.reverse_bits()
}

/// The whole bit stream shifted one bit left (`left`) or right, most significant bit first.
pub fn shift_bits(a: &[u8], left: bool) -> Vec<u8> {
    let mut bits: Vec<u8> = a.iter().flat_map(|&b| (0..8).rev().map(move |k| (b >> k) & 1)).collect();
    if left {
        if !bits.is_empty() {
            bits.remove(0);
            bits.push(0);
        }
    } else {
        bits.insert(0, 0);
        bits.pop();
    }
    bits.chunks(8).map(|c| c.iter().fold(0u8, |v, &b| (v << 1) | b)).collect()
}

fn modbus_function(f: u8) -> Option<&'static str> {
    Some(match f {
        1 => "read coils",
        2 => "read discrete inputs",
        3 => "read holding registers",
        4 => "read input registers",
        5 => "write single coil",
        6 => "write single register",
        15 => "write multiple coils",
        16 => "write multiple registers",
        _ => return None,
    })
}

fn info(id: FindingId, title: impl Into<String>, body: impl Into<String>, mono: Option<String>) -> Finding {
    Finding { id, tone: Tone::Info, title: title.into(), body: body.into(), mono }
}

/// What a short run of bytes might mean.
pub fn analyse_bytes(a: &[u8]) -> ByteAnalysis {
    let bytes = a.to_vec();
    let mut findings = Vec::new();
    let r = printable_ratio(&bytes);
    let is_text = r >= 0.85 && bytes.len() >= 2;
    let bcd = !bytes.is_empty() && is_bcd(&bytes);

    if is_text {
        let t = as_text(&bytes);
        let mut extra = Vec::new();
        if t.ends_with("\\r\\n") {
            extra.push("It ends with CR LF, a normal line ending.");
        }
        if t.starts_with("AT") {
            extra.push("Lines starting with AT are modem-style commands.");
        }
        if t.starts_with("$GP") || t.starts_with("$GN") || t.starts_with("$GL") {
            extra.push("Lines starting with $GP or $GN are GPS (NMEA) sentences.");
        }
        findings.push(info(FindingId::Text, "This is text.", extra.join(" "), Some(t)));
    } else if bytes.len() >= 4 && !bcd {
        let alts: [(&str, Vec<u8>, &str); 4] = [
            (
                "shifted one bit left",
                shift_bits(&bytes, true),
                "On SPI this means the decoder samples on the wrong clock edge: change CPHA. On UART it points to a wrong baud rate.",
            ),
            (
                "shifted one bit right",
                shift_bits(&bytes, false),
                "On SPI this means the decoder samples on the wrong clock edge: change CPHA.",
            ),
            (
                "the bit order is reversed",
                bytes.iter().map(|&b| reverse_bits8(b)).collect(),
                "Switch the decoder between MSB first and LSB first.",
            ),
            (
                "inverted",
                bytes.iter().map(|&b| !b).collect(),
                "The signal is inverted. Switch on the invert option.",
            ),
        ];
        // the best one; ties keep the order above
        let mut best = &alts[0];
        for x in &alts[1..] {
            if printable_ratio(&x.1) > printable_ratio(&best.1) {
                best = x;
            }
        }
        let br = printable_ratio(&best.1);
        if br >= 0.85 && wordy(&best.1) >= 0.75 && br - r >= 0.3 {
            findings.push(info(
                FindingId::TransformedText,
                format!("These bytes become text when {}:", best.0),
                format!("{} The first or last character may be cut off.", best.2),
                Some(as_text(&best.1)),
            ));
        }
    }

    if bytes.len() >= 4 {
        let c = modbus_crc(&bytes[..bytes.len() - 2]);
        if low_byte(u64::from(c)) == bytes[bytes.len() - 2]
            && low_byte(u64::from(c >> 8)) == bytes[bytes.len() - 1]
        {
            let f = bytes[1];
            let name = modbus_function(f & 127);
            let mut body = format!(
                "Device address {}, function {f}{}.",
                bytes[0],
                name.map_or(String::new(), |n| format!(
                    " ({}{n})",
                    if f & 128 != 0 { "error reply to " } else { "" }
                ))
            );
            let be = |i: usize| (u16::from(bytes[i]) << 8) | u16::from(bytes[i + 1]);
            if (f == 3 || f == 4) && bytes.len() == 8 {
                body += &format!(" As a request: read {} register(s) starting at {}.", be(4), be(2));
            }
            if (f == 3 || f == 4) && usize::from(bytes[2]) + 5 == bytes.len() && bytes[2].is_multiple_of(2) {
                let v: Vec<String> =
                    (3..3 + usize::from(bytes[2])).step_by(2).map(|i| be(i).to_string()).collect();
                body += &format!(" As a reply: {} register value(s): {}.", v.len(), v.join(", "));
            }
            findings.push(info(
                FindingId::Modbus,
                "Valid Modbus RTU frame (the last two bytes are a correct CRC-16).",
                body,
                None,
            ));
        }
    }

    if bytes.len() >= 3 && !is_text {
        let (body, last) = (&bytes[..bytes.len() - 1], bytes[bytes.len() - 1]);
        if last != 0 && body.iter().fold(0u8, |s, &x| s.wrapping_add(x)) == last {
            findings.push(info(
                FindingId::ChecksumSum,
                "The last byte equals the sum of all the others (modulo 256).",
                "It is probably a checksum.",
                None,
            ));
        } else if last != 0 && body.iter().fold(0u8, |s, &x| s ^ x) == last {
            findings.push(info(
                FindingId::ChecksumXor,
                "The last byte equals the XOR of all the others.",
                "It is probably a checksum.",
                None,
            ));
        }
    }
    if bytes.len() >= 2 && !is_text && bcd {
        findings.push(info(
            FindingId::Bcd,
            "Every byte is valid BCD.",
            "Read the hex digits as decimal. Clock chips store seconds, minutes, hours, day, date, month, year this way.",
            Some(bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")),
        ));
    }
    if bytes.len() >= 2 && bytes.iter().all(|&b| b == 0xff) {
        findings.push(info(FindingId::AllFf, "All bytes are FF.", "On SPI or I2C that usually means nothing is driving the line: device not selected, not powered, or wrong register.", None));
    }
    if bytes.len() >= 2 && bytes.iter().all(|&b| b == 0) {
        findings.push(info(FindingId::All00, "All bytes are 00.", "The line may be stuck low, or the device has nothing to report. On UART, 00 with frame errors is a break or a disconnected transmitter.", None));
    }
    if findings.is_empty() && !bytes.is_empty() {
        findings.push(Finding {
            id: FindingId::None,
            tone: Tone::Warn,
            title: "No obvious pattern.".into(),
            body: "This is probably binary data: use the tables below and the device datasheet.".into(),
            mono: None,
        });
    }

    let pairs = (bytes.len() >= 2 && !is_text).then(|| {
        bytes[..bytes.len().min(16)]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                let be = (u16::from(p[0]) << 8) | u16::from(p[1]);
                let le = (u16::from(p[1]) << 8) | u16::from(p[0]);
                PairRow {
                    hex: format!("{:02X} {:02X}", p[0], p[1]),
                    big_endian: be,
                    big_endian_signed: i16::from_be_bytes([p[0], p[1]]),
                    little_endian: le,
                    little_endian_signed: i16::from_le_bytes([p[0], p[1]]),
                }
            })
            .collect()
    });
    let i2c_first_byte = (!bytes.is_empty() && (!is_text || bytes.len() <= 2)).then(|| I2cByte {
        address: bytes[0] >> 1,
        read: bytes[0] & 1 == 1,
        device: i2c_device(bytes[0] >> 1).map(String::from),
    });
    ByteAnalysis { bytes, is_text, findings, pairs, i2c_first_byte }
}

/// CRC-16 as used by Modbus RTU (polynomial 0xA001 reflected, initial 0xFFFF); the low byte is
/// sent first.
pub fn modbus_crc(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xa001 } else { crc >> 1 };
        }
    }
    crc
}

/// Common I2C parts by 7-bit address.
pub fn i2c_device(address: u8) -> Option<&'static str> {
    Some(match address {
        0x20..=0x26 => "PCF8574 or MCP23017 I/O expander",
        0x27 => "PCF8574 LCD backpack",
        0x3c | 0x3d => "SSD1306 OLED display",
        0x40 => "INA219, Si7021, HDC1080 or PCA9685",
        0x44 => "SHT3x humidity sensor",
        0x48 => "LM75 or TMP102 temperature sensor, ADS1115, PCF8591",
        0x49..=0x4f => "LM75 or TMP102 temperature sensor, ADS1115",
        0x50..=0x56 => "24Cxx EEPROM",
        0x57 => "24Cxx EEPROM (on DS3231 modules)",
        0x5a => "MLX90614, MPR121",
        0x68 => "DS1307 or DS3231 clock, MPU6050",
        0x69 => "MPU6050 (alternate)",
        0x76 => "BMP280, BME280, MS5611",
        0x77 => "BMP180, BMP280, BME280",
        _ => return None,
    })
}

/// CRC-15 as used by CAN (polynomial 0x4599), over a list of bits.
pub fn can_crc15(bits: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in bits {
        let next = u16::from(b & 1) ^ ((crc >> 14) & 1);
        crc = (crc << 1) & 0x7fff;
        if next == 1 {
            crc ^= 0x4599;
        }
    }
    crc
}

/// The lowest 8 bits of `v`.
pub fn low_byte(v: u64) -> u8 {
    v.to_le_bytes()[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_crc15_matches_the_reference_value() {
        // CRC-15/CAN of the ASCII bits of "123456789" is 0x059E
        let bits: Vec<u8> =
            b"123456789".iter().flat_map(|&b| (0..8).map(move |k| (b >> (7 - k)) & 1)).collect();
        assert_eq!(can_crc15(&bits), 0x059e);
    }

    #[test]
    fn modbus_crc_of_a_read_request() {
        // 01 03 00 00 00 02 is followed by C4 0B on the wire
        assert_eq!(modbus_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02]), 0x0bc4);
    }
}
