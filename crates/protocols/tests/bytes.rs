//! The byte analyser (Signal Detective's `tests/bytes.test.ts`).

#![allow(clippy::cast_possible_truncation)]

use logica_protocols::bytes::*;

fn ids(hex: &str) -> Vec<FindingId> {
    analyse_bytes(&parse_hex_bytes(hex).unwrap()).findings.iter().map(|f| f.id).collect()
}

#[test]
fn parsing_accepts_spaces_commas_0x_and_h_suffixes() {
    assert_eq!(parse_hex_bytes("48 65,0x6C;6Ch 6f").unwrap(), [0x48, 0x65, 0x6c, 0x6c, 0x6f]);
    assert_eq!(parse_hex_bytes("  ").unwrap(), Vec::<u8>::new());
    assert_eq!(parse_hex_bytes("A").unwrap(), [0x0a]);
    assert_eq!(
        parse_hex_bytes("48 zz 65").unwrap_err(),
        "\"zz\" is not a hex byte. Use two hex digits per byte, such as 4F or 0x4F."
    );
    assert!(parse_hex_bytes("123").is_err());
}

#[test]
fn text_with_cr_lf_at_commands_and_gps() {
    let a = analyse_bytes(b"Hello\r\n");
    assert!(a.is_text);
    assert_eq!((a.findings[0].id, a.findings[0].mono.as_deref()), (FindingId::Text, Some("Hello\\r\\n")));
    assert!(a.findings[0].body.contains("CR LF"));
    assert!(analyse_bytes(b"AT+GMR\r\n").findings[0].body.contains("modem-style"));
    assert!(analyse_bytes(b"$GPGGA,123\r\n").findings[0].body.contains("NMEA"));
    let a = analyse_bytes(b"Hello");
    assert!(a.pairs.is_none() && a.i2c_first_byte.is_none());
}

#[test]
fn text_hidden_by_a_wrong_setting() {
    let reversed: Vec<u8> = b"Hello world".iter().map(|&b| reverse_bits8(b)).collect();
    let a = analyse_bytes(&reversed);
    assert_eq!(
        (a.findings[0].id, a.findings[0].title.as_str()),
        (FindingId::TransformedText, "These bytes become text when the bit order is reversed:")
    );
    let inverted: Vec<u8> = b"Temperature".iter().map(|&b| !b).collect();
    assert!(analyse_bytes(&inverted).findings[0].title.contains("inverted"));
    let shifted = shift_bits(b"Status OK now", false);
    assert!(analyse_bytes(&shifted).findings[0].title.contains("shifted one bit left"));
}

#[test]
fn bcd_clock_bytes_are_not_bit_reversed_text() {
    // DS3231 time registers from mystery_C: 12:34:56, weekday 4, 12 Nov 2026
    assert_eq!(ids("56 34 12 04 12 11 26"), [FindingId::Bcd]);
}

#[test]
fn short_printable_and_random_data_are_not_transformed_text() {
    assert!(!ids("12 48").contains(&FindingId::TransformedText));
    assert_eq!(ids("41 42 43 44"), [FindingId::Text]);
    assert!(!ids("8F 13 E2 07 B1 C9").contains(&FindingId::TransformedText));
}

#[test]
fn modbus_requests_replies_and_errors() {
    let c = modbus_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02]);
    assert_eq!([c & 255, c >> 8], [0xc4, 0x0b]);
    let f =
        |b: &[u8]| analyse_bytes(b).findings.into_iter().find(|x| x.id == FindingId::Modbus).unwrap().body;
    assert_eq!(
        f(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02, 0xc4, 0x0b]),
        "Device address 1, function 3 (read holding registers). As a request: read 2 register(s) starting at 0."
    );
    assert!(
        f(&[0x01, 0x03, 0x04, 0x00, 0xee, 0x01, 0xef, 0xda, 0x1a]).contains("2 register value(s): 238, 495")
    );
    let body = [0x11u8, 0x83, 0x02];
    let c = modbus_crc(&body);
    assert!(
        f(&[0x11, 0x83, 0x02, low_byte(u64::from(c)), low_byte(u64::from(c >> 8))])
            .contains("error reply to read holding registers")
    );
    assert!(!ids("01 03 00 00 00 03 C4 0B").contains(&FindingId::Modbus));
}

#[test]
fn checksums_bcd_and_fills() {
    assert!(ids("10 20 30 60").contains(&FindingId::ChecksumSum));
    assert!(ids("81 F2 07 74").contains(&FindingId::ChecksumXor));
    assert!(!ids("00 00 00").contains(&FindingId::ChecksumSum));
    assert!(ids("FF FF FF").contains(&FindingId::AllFf));
    assert!(ids("00 00").contains(&FindingId::All00));
    assert!(is_bcd(&[0x99, 0x00]) && !is_bcd(&[0x9a]) && !is_bcd(&[0xa0]));
    assert_eq!(ids("8F 13 E2"), [FindingId::None]);
    assert!(analyse_bytes(&[]).findings.is_empty());
}

#[test]
fn pairs_both_ways_and_the_i2c_reading() {
    let a = analyse_bytes(&[0x19, 0x80, 0xff, 0xfe]);
    assert_eq!(
        a.pairs.unwrap(),
        [
            PairRow {
                hex: "19 80".into(),
                big_endian: 0x1980,
                big_endian_signed: 0x1980,
                little_endian: 0x8019,
                little_endian_signed: (0x8019 - 65536) as i16
            },
            PairRow {
                hex: "FF FE".into(),
                big_endian: 0xfffe,
                big_endian_signed: -2,
                little_endian: 0xfeff,
                little_endian_signed: (0xfeff - 65536) as i16
            },
        ]
    );
    assert_eq!(
        analyse_bytes(&[0xd1, 0x00]).i2c_first_byte,
        Some(I2cByte { address: 0x68, read: true, device: Some("DS1307 or DS3231 clock, MPU6050".into()) })
    );
    assert_eq!(
        analyse_bytes(&[0x02, 0x00]).i2c_first_byte,
        Some(I2cByte { address: 0x01, read: false, device: None })
    );
}
