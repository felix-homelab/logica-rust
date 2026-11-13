//! Generated example captures, one per protocol that has no real sample file (Signal Detective's
//! examples). Each says what it contains, so a detection can be checked against it.

use logica_core::Level;

use crate::custom::{CustomConfig, built_in_config};
use crate::pulsecode::PulseCodeConfig;
use crate::synth::*;

pub struct Example {
    pub id: &'static str,
    pub title: &'static str,
    /// What the capture contains.
    pub contents: &'static str,
    /// The protocol a detector should find.
    pub protocol_id: &'static str,
    pub build: fn() -> GenCapture,
}

fn pulse_config(id: &str) -> PulseCodeConfig {
    match built_in_config(id) {
        Some(CustomConfig::PulseCode(c)) => c,
        _ => panic!("built-in pulse-code config {id} is missing"),
    }
}

fn nec_bits(addr: u8, cmd: u8) -> Vec<u8> {
    bits_of(&[addr, addr ^ 0xff, cmd, cmd ^ 0xff], false)
}

pub const EXAMPLES: [Example; 10] = [
    Example {
        id: "nec",
        title: "Infrared remote (NEC)",
        contents: "Channel 0: button press with address 0x00, command 0x45, then one repeat code 40 ms later.",
        protocol_id: "ir-nec",
        build: || {
            let rate = 1e6;
            let cfg = pulse_config("ir-nec");
            let mut l = Lane::high(rate);
            pulse_code_send(&mut l, 0.01, &cfg, 0, &nec_bits(0x00, 0x45));
            pulse_code_send(&mut l, 0.01 + 0.108, &cfg, 1, &[]);
            make_capture("Example: NEC infrared remote", rate, 0.2, vec![(0, "IR", l)])
        },
    },
    Example {
        id: "dht22",
        title: "DHT22 humidity sensor",
        contents: "Channel 0: one reading. Humidity 0x028C = 65.2 %RH, temperature 0x00E7 = 23.1 °C, checksum 0x75.",
        protocol_id: "dht",
        build: || {
            let rate = 1e6;
            let mut l = Lane::high(rate);
            let mut bytes = vec![0x02u8, 0x8c, 0x00, 0xe7];
            bytes.push(bytes.iter().fold(0u8, |s, &b| s.wrapping_add(b)));
            pulse_code_send(&mut l, 0.005, &pulse_config("dht"), 0, &bits_of(&bytes, true));
            make_capture("Example: DHT22 sensor", rate, 0.05, vec![(0, "DATA", l)])
        },
    },
    Example {
        id: "ws2812",
        title: "WS2812 LED strip",
        contents: "Channel 0: three LEDs set to red (G 00 R FF B 00), green (FF 00 00) and blue (00 00 FF), sent twice.",
        protocol_id: "ws2812",
        build: || {
            let rate = 20e6;
            let cfg = pulse_config("ws2812");
            let mut l = Lane::low(rate);
            let bits = bits_of(&[0x00, 0xff, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff], true);
            pulse_code_send(&mut l, 0.0001, &cfg, 0, &bits);
            pulse_code_send(&mut l, 0.0005, &cfg, 0, &bits);
            make_capture("Example: WS2812 LEDs", rate, 0.001, vec![(0, "DIN", l)])
        },
    },
    Example {
        id: "onewire",
        title: "1-Wire temperature sensor (DS18B20)",
        contents: "Channel 0: Skip ROM + Convert T, then Skip ROM + Read scratchpad. The first two bytes, 91 01, are 0x0191 = 25.06 °C.",
        protocol_id: "onewire",
        build: || {
            let rate = 2e6;
            let cfg = pulse_config("onewire");
            let mut l = Lane::high(rate);
            pulse_code_send(&mut l, 0.001, &cfg, 0, &bits_of(&[0xcc, 0x44], false));
            pulse_code_send(
                &mut l,
                0.02,
                &cfg,
                0,
                &bits_of(&[0xcc, 0xbe, 0x91, 0x01, 0x4b, 0x46, 0x7f, 0xff, 0x0f, 0x10, 0x2a], false),
            );
            make_capture("Example: 1-Wire DS18B20", rate, 0.05, vec![(0, "DQ", l)])
        },
    },
    Example {
        id: "can",
        title: "CAN bus at 500 kbit/s",
        contents: "Channel 0: ID 0x123 with 11 22 33, ID 0x7DF with 02 01 0C (an OBD-II engine speed request), and extended ID 0x18DAF110 with DE AD.",
        protocol_id: "can",
        build: || {
            let rate = 10e6;
            let mut l = Lane::high(rate);
            let t = can_frame(&mut l, 0.001, 500e3, 0x123, &[0x11, 0x22, 0x33], false);
            let t = can_frame(&mut l, t + 0.0005, 500e3, 0x7df, &[0x02, 0x01, 0x0c, 0, 0, 0, 0, 0], false);
            can_frame(&mut l, t + 0.0005, 500e3, 0x18da_f110, &[0xde, 0xad], true);
            make_capture("Example: CAN bus", rate, 0.005, vec![(0, "CAN RX", l)])
        },
    },
    Example {
        id: "i2s",
        title: "I2S audio",
        contents: "Channels 0 to 2: 48 kHz stereo, 16-bit samples in 32-bit slots. Left counts up by 1000, right is its negative.",
        protocol_id: "i2s",
        build: || {
            let rate = 50e6;
            let (mut sck, mut ws, mut sd) = (Lane::low(rate), Lane::low(rate), Lane::low(rate));
            let samples: Vec<(i64, i64)> = (0..48).map(|i| (i * 1000, -i * 1000)).collect();
            i2s_stream(&mut sck, &mut ws, &mut sd, 0.0, 48000.0 * 64.0, 32, &samples, 16);
            make_capture(
                "Example: I2S audio",
                rate,
                0.001,
                vec![(0, "BCLK", sck), (1, "WS", ws), (2, "SD", sd)],
            )
        },
    },
    Example {
        id: "swd",
        title: "SWD debug probe",
        contents: "Channels 0 and 1: line reset, read IDCODE (0x2BA01477), write ABORT, write SELECT, then an access-port read that gets WAIT.",
        protocol_id: "swd",
        build: || {
            let rate = 20e6;
            let (mut clk, mut io) = (Lane::low(rate), Lane::low(rate));
            let hz = 1e6;
            let op = |ap, read, reg, data, ack| SwdOp { ap, read, reg, data, ack };
            let t = swd_line_reset(&mut clk, &mut io, 0.0001, hz, 56);
            let t = swd_transfer(&mut clk, &mut io, t, hz, op(false, true, 0, 0x2ba0_1477, 1));
            let t = swd_transfer(&mut clk, &mut io, t + 5e-6, hz, op(false, false, 0, 0x1e, 1));
            let t = swd_transfer(&mut clk, &mut io, t + 5e-6, hz, op(false, false, 8, 0, 1));
            swd_transfer(&mut clk, &mut io, t + 5e-6, hz, op(true, true, 0xc, 0, 2));
            make_capture("Example: SWD", rate, 0.001, vec![(0, "SWCLK", clk), (1, "SWDIO", io)])
        },
    },
    Example {
        id: "jtag",
        title: "JTAG IDCODE read",
        contents: "Channels 0 to 3: TAP reset, instruction 0xE (IDCODE on ARM chips) shifted into a 4-bit IR, then the 32-bit IDCODE 0x4BA00477 shifted out of DR.",
        protocol_id: "jtag",
        build: || {
            let rate = 20e6;
            let (mut tck, mut tms, mut tdi, mut tdo) =
                (Lane::low(rate), Lane::high(rate), Lane::low(rate), Lane::high(rate));
            let hz = 1e6;
            let id: u32 = 0x4ba0_0477;
            let id_bits: Vec<u8> = (0..32).map(|k| u8::from((id >> k) & 1 == 1)).collect();
            let s = |tms| JtagStep { tms, tdi: None, tdo: None };
            let t = jtag_clock(
                &mut tck,
                &mut tms,
                &mut tdi,
                None,
                0.0001,
                hz,
                &[s(1), s(1), s(1), s(1), s(1), s(0)],
            );
            // IR shifts out its capture value 0b0001 (ends in 01, as the standard requires) while 0xE shifts in
            let t = jtag_clock(
                &mut tck,
                &mut tms,
                &mut tdi,
                Some(&mut tdo),
                t,
                hz,
                &jtag_shift(true, &[0, 1, 1, 1], &[1, 0, 0, 0]),
            );
            let t = jtag_clock(
                &mut tck,
                &mut tms,
                &mut tdi,
                Some(&mut tdo),
                t + 2e-6,
                hz,
                &jtag_shift(false, &[0; 32], &id_bits),
            );
            tdo.set(t, Level::High);
            make_capture(
                "Example: JTAG",
                rate,
                0.001,
                vec![(0, "TCK", tck), (1, "TMS", tms), (2, "TDI", tdi), (3, "TDO", tdo)],
            )
        },
    },
    Example {
        id: "dmx512",
        title: "DMX512 lighting",
        contents: "Channel 0: two DMX packets: a 100 µs break, start code 00, then channel values FF 80 40 00.",
        protocol_id: "dmx512",
        build: || {
            let rate = 10e6;
            let mut l = Lane::high(rate);
            let mut t = 0.0002;
            for _ in 0..2 {
                l.set(t, 0u8);
                l.set(t + 100e-6, 1u8); // break, then mark after break
                t = uart_send(
                    &mut l,
                    t + 112e-6,
                    &[0x00, 0xff, 0x80, 0x40, 0x00],
                    UartOptions { stop_bits: 2, ..UartOptions::baud(250_000.0) },
                );
                t += 0.001;
            }
            make_capture("Example: DMX512", rate, 0.005, vec![(0, "DMX", l)])
        },
    },
    Example {
        id: "servo",
        title: "Servo PWM",
        contents: "Channel 0: 50 Hz with a 1.5 ms pulse, the centre position of a hobby servo (7.5 % duty).",
        protocol_id: "pwm",
        build: || {
            let rate = 1e6;
            let mut l = Lane::low(rate);
            pwm_signal(&mut l, 0.001, 0.5, 50.0, 0.075);
            make_capture("Example: servo PWM", rate, 0.5, vec![(0, "SERVO", l)])
        },
    },
];

pub fn example(id: &str) -> Option<&'static Example> {
    EXAMPLES.iter().find(|e| e.id == id)
}
