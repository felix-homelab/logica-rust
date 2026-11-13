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
