//! The byte analyser (doc 06): what a run of decoded bytes might mean.

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

/// The lowest 8 bits of `v`.
pub fn low_byte(v: u64) -> u8 {
    v.to_le_bytes()[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modbus_crc_of_a_read_request() {
        // 01 03 00 00 00 02 is followed by C4 0B on the wire
        assert_eq!(modbus_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02]), 0x0bc4);
    }
}
