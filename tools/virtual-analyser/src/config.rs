//! What the virtual analyser generates: a JSON file (or the built-in default), checked before
//! anything listens, with every problem explained.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// Shown in Logica's device list as "Virtual analyser: <name>".
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Channels 0 to `channelCount - 1`; those no signal uses stay low.
    #[serde(default = "default_channel_count")]
    pub channel_count: u32,
    /// Rates offered; rates too slow for a signal are left out at start.
    #[serde(default = "default_rates")]
    pub sample_rates_hz: Vec<f64>,
    /// The same seed gives the same signals in every capture; without one each capture differs.
    #[serde(default)]
    pub seed: Option<u64>,
    /// 1 is real time, 2 twice as fast; 0 sends as fast as the connection takes it.
    #[serde(default = "one")]
    pub speed: f64,
    /// Signal time per batch sent, in milliseconds.
    #[serde(default = "default_tile_ms")]
    pub tile_ms: f64,
    /// How far this analyser's sample clock is off, in parts per million (positive runs fast). It
    /// shows only in `shared` signals, the ones that come from outside: so lining several analysers
    /// up by a shared signal has drift to correct, as with real devices.
    #[serde(default)]
    pub clock_ppm: f64,
    pub signals: Vec<Signal>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Uart {
    pub channel: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_baud")]
    pub baud: f64,
    #[serde(default = "eight")]
    pub data_bits: u32,
    #[serde(default)]
    pub parity: Parity,
    #[serde(default = "one_u32")]
    pub stop_bits: u32,
    #[serde(default)]
    pub invert: bool,
    /// Messages sent in random order; random printable bytes when empty.
    #[serde(default)]
    pub messages: Vec<String>,
    /// Length of a random message: fewest and most bytes.
    #[serde(default = "default_random_bytes")]
    pub random_bytes: [u32; 2],
    /// Pause between messages: shortest and longest, in milliseconds.
    #[serde(default = "default_every")]
    pub every_ms: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct I2c {
    pub scl: u32,
    pub sda: u32,
    #[serde(default = "default_i2c_hz")]
    pub clock_hz: f64,
    /// 7-bit addresses picked at random.
    #[serde(default = "default_addresses")]
    pub addresses: Vec<u8>,
    /// Data bytes per transfer: fewest and most.
    #[serde(default = "default_bytes")]
    pub bytes: [u32; 2],
    /// Share of transfers that read.
    #[serde(default = "default_read_chance")]
    pub read_chance: f64,
    /// Share of transfers no device answers.
    #[serde(default = "default_nack_chance")]
    pub nack_chance: f64,
    #[serde(default = "default_every")]
    pub every_ms: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Spi {
    pub clk: u32,
    #[serde(default)]
    pub cs: Option<u32>,
    #[serde(default)]
    pub mosi: Option<u32>,
    #[serde(default)]
    pub miso: Option<u32>,
    #[serde(default = "default_spi_hz")]
    pub clock_hz: f64,
    /// SPI mode 0 to 3.
    #[serde(default)]
    pub mode: u8,
    #[serde(default = "yes")]
    pub msb_first: bool,
    #[serde(default = "default_bytes")]
    pub bytes: [u32; 2],
    #[serde(default = "default_every")]
    pub every_ms: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pwm {
    pub channel: u32,
    #[serde(default)]
    pub name: Option<String>,
    pub frequency_hz: f64,
    /// Share of each period spent high, above 0 and below 1.
    #[serde(default = "half")]
    pub duty: f64,
    /// PWM only: when given, the duty wanders between these two shares instead of staying at
    /// `duty`, gliding from one random value to the next (a level that changes, for virtual analog
    /// channels).
    #[serde(default)]
    pub duty_range: Option<[f64; 2]>,
    /// How long each glide to a new random duty takes, in milliseconds (random between the two).
    #[serde(default = "default_change_ms")]
    pub change_ms: [f64; 2],
}

/// A clock that follows the computer's clock instead of the capture's: analysers on the same
/// computer see the same edges, each at its own samples, as if one clock were wired to both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Shared {
    pub channel: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_shared_hz")]
    pub frequency_hz: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Noise {
    pub channel: u32,
    #[serde(default)]
    pub name: Option<String>,
    /// Mean time between random level changes, in microseconds.
    #[serde(default = "default_noise_us")]
    pub mean_us: f64,
}

/// One generated signal and the channels it drives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Signal {
    Uart(Uart),
    I2c(I2c),
    Spi(Spi),
    /// A steady square wave with a duty cycle.
    Pwm(Pwm),
    /// A square wave at 50 % duty.
    Clock(Pwm),
    /// Random level changes.
    Noise(Noise),
    /// A clock shared with other analysers on this computer (it follows the computer's clock), to
    /// line several up in one capture.
    Shared(Shared),
}

fn default_name() -> String {
    "Bench".into()
}
fn default_listen() -> String {
    "127.0.0.1:7171".into()
}
fn default_channel_count() -> u32 {
    8
}
fn default_rates() -> Vec<f64> {
    vec![1e6, 4e6, 10e6, 25e6, 100e6]
}
fn one() -> f64 {
    1.0
}
fn one_u32() -> u32 {
    1
}
fn eight() -> u32 {
    8
}
fn half() -> f64 {
    0.5
}
fn default_change_ms() -> [f64; 2] {
    [200.0, 1500.0]
}
fn yes() -> bool {
    true
}
fn default_tile_ms() -> f64 {
    20.0
}
fn default_baud() -> f64 {
    115_200.0
}
fn default_random_bytes() -> [u32; 2] {
    [4, 16]
}
fn default_every() -> [f64; 2] {
    [20.0, 200.0]
}
fn default_i2c_hz() -> f64 {
    100e3
}
fn default_spi_hz() -> f64 {
    500e3
}
fn default_addresses() -> Vec<u8> {
    vec![0x50]
}
fn default_bytes() -> [u32; 2] {
    [1, 4]
}
fn default_read_chance() -> f64 {
    0.3
}
fn default_nack_chance() -> f64 {
    0.05
}
fn default_shared_hz() -> f64 {
    100.0
}
fn default_noise_us() -> f64 {
    200.0
}

/// Samples needed per shortest step of a signal (a quarter clock, a UART bit's eighth).
const STEPS: f64 = 2.0;

impl Signal {
    /// The channels it drives, with their default names.
    pub fn channels(&self) -> Vec<(u32, String)> {
        let named =
            |ch: u32, name: &Option<String>, def: &str| (ch, name.clone().unwrap_or_else(|| def.into()));
        match self {
            Signal::Uart(u) => vec![named(u.channel, &u.name, "TX")],
            Signal::I2c(i) => vec![(i.scl, "SCL".into()), (i.sda, "SDA".into())],
            Signal::Spi(s) => {
                let mut v = vec![(s.clk, "CLK".to_string())];
                v.extend(s.cs.map(|c| (c, "CS".into())));
                v.extend(s.mosi.map(|c| (c, "MOSI".into())));
                v.extend(s.miso.map(|c| (c, "MISO".into())));
                v
            }
            Signal::Pwm(p) => vec![named(p.channel, &p.name, "PWM")],
            Signal::Clock(p) => vec![named(p.channel, &p.name, "CLOCK")],
            Signal::Noise(n) => vec![named(n.channel, &n.name, "NOISE")],
            Signal::Shared(s) => vec![named(s.channel, &s.name, "SYNC")],
        }
    }

    /// The slowest sample rate that draws it faithfully.
    pub fn min_rate_hz(&self) -> f64 {
        match self {
            // decoders sample a bit near its middle; eight samples a bit keeps that exact
            Signal::Uart(u) => 8.0 * u.baud,
            // the generators place edges on quarter clock periods
            Signal::I2c(i) => 4.0 * STEPS * i.clock_hz,
            Signal::Spi(s) => 4.0 * STEPS * s.clock_hz,
            Signal::Pwm(p) => {
                let [lo, hi] = p.duty_range.unwrap_or([p.duty, p.duty]);
                STEPS * p.frequency_hz / lo.min(1.0 - hi)
            }
            Signal::Clock(p) => 2.0 * STEPS * p.frequency_hz,
            // random changes closer than a sample simply cancel
            Signal::Noise(_) => 0.0,
            Signal::Shared(s) => 2.0 * STEPS * s.frequency_hz,
        }
    }

    /// One line for the start-up summary.
    pub fn describe(&self) -> String {
        let every = |e: &[f64; 2]| format!("every {}–{} ms", e[0], e[1]);
        match self {
            Signal::Uart(u) => {
                let p = match u.parity {
                    Parity::None => 'N',
                    Parity::Even => 'E',
                    Parity::Odd => 'O',
                };
                let what = if u.messages.is_empty() {
                    format!("{}–{} random characters", u.random_bytes[0], u.random_bytes[1])
                } else {
                    format!("{} messages", u.messages.len())
                };
                format!(
                    "UART {} baud {}{p}{}{}, {what} {}",
                    u.baud,
                    u.data_bits,
                    u.stop_bits,
                    if u.invert { " inverted" } else { "" },
                    every(&u.every_ms)
                )
            }
            Signal::I2c(i) => format!(
                "I2C {} kHz to {}, {}–{} bytes {}",
                i.clock_hz / 1e3,
                i.addresses.iter().map(|a| format!("0x{a:02X}")).collect::<Vec<_>>().join(", "),
                i.bytes[0],
                i.bytes[1],
                every(&i.every_ms)
            ),
            Signal::Spi(s) => format!(
                "SPI {} kHz mode {}, {}–{} bytes {}",
                s.clock_hz / 1e3,
                s.mode,
                s.bytes[0],
                s.bytes[1],
                every(&s.every_ms)
            ),
            Signal::Pwm(p) => match p.duty_range {
                Some([lo, hi]) => format!(
                    "PWM {} Hz, duty wandering between {} % and {} % (a glide every {}–{} ms)",
                    p.frequency_hz,
                    lo * 100.0,
                    hi * 100.0,
                    p.change_ms[0],
                    p.change_ms[1]
                ),
                None => format!("PWM {} Hz at {} % duty", p.frequency_hz, p.duty * 100.0),
            },
            Signal::Clock(p) => format!("clock {} Hz", p.frequency_hz),
            Signal::Noise(n) => format!("random changes about every {} µs", n.mean_us),
            Signal::Shared(s) => format!("shared clock {} Hz (follows the computer's clock)", s.frequency_hz),
        }
    }
}

fn range_ok(r: &[f64; 2]) -> bool {
    r[0].is_finite() && r[1].is_finite() && r[0] >= 0.0 && r[0] <= r[1]
}

fn positive(x: f64) -> bool {
    x.is_finite() && x > 0.0
}

impl Config {
    /// Every problem with the configuration, worded so it can be fixed; empty when it is fine.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.name.trim().is_empty() {
            p.push("Give the analyser a name.".into());
        }
        if self.channel_count == 0 || self.channel_count > 64 {
            p.push(format!("channelCount is {}; choose 1 to 64.", self.channel_count));
        }
        if self.sample_rates_hz.is_empty() || !self.sample_rates_hz.iter().all(|&r| positive(r)) {
            p.push("sampleRatesHz must list at least one rate above 0.".into());
        }
        if !self.speed.is_finite() || self.speed < 0.0 {
            p.push("speed must be 0 (as fast as possible) or above.".into());
        }
        if !(self.clock_ppm.is_finite() && self.clock_ppm.abs() <= 1000.0) {
            p.push(format!("clockPpm is {}; choose -1000 to 1000.", self.clock_ppm));
        }
        if !(1.0..=1000.0).contains(&self.tile_ms) {
            p.push(format!("tileMs is {}; choose 1 to 1000.", self.tile_ms));
        }
        if self.signals.is_empty() {
            p.push("Add at least one signal.".into());
        }
        let mut used: Vec<(u32, usize)> = Vec::new();
        for (k, s) in self.signals.iter().enumerate() {
            let n = k + 1;
            for (ch, _) in s.channels() {
                if ch >= self.channel_count {
                    p.push(format!(
                        "Signal {n} uses channel {ch}, but channels go from 0 to {}.",
                        self.channel_count.saturating_sub(1)
                    ));
                }
                match used.iter().find(|u| u.0 == ch) {
                    Some(&(_, other)) if other == k => {
                        p.push(format!("Signal {n} uses channel {ch} twice."));
                    }
                    Some(&(_, other)) => {
                        p.push(format!(
                            "Signals {} and {n} both drive channel {ch}; give each its own.",
                            other + 1
                        ));
                    }
                    None => used.push((ch, k)),
                }
            }
            p.extend(signal_problems(s).into_iter().map(|m| format!("Signal {n}: {m}")));
        }
        p
    }

    /// The configured rates fast enough for every signal, ascending.
    pub fn usable_rates(&self) -> Vec<f64> {
        let need = self.min_rate_hz();
        let mut r: Vec<f64> = self.sample_rates_hz.iter().copied().filter(|&r| r >= need).collect();
        r.sort_by(f64::total_cmp);
        r.dedup();
        r
    }

    /// The slowest rate every signal needs.
    pub fn min_rate_hz(&self) -> f64 {
        self.signals.iter().map(Signal::min_rate_hz).fold(0.0, f64::max)
    }

    /// Channel names: the signal's name for driven channels, "CH<n>" for idle ones.
    pub fn channel_names(&self) -> Vec<String> {
        (0..self.channel_count)
            .map(|ch| {
                self.signals
                    .iter()
                    .flat_map(Signal::channels)
                    .find(|(c, _)| *c == ch)
                    .map_or_else(|| format!("CH{ch}"), |(_, n)| n)
            })
            .collect()
    }

    /// The built-in configuration: one of each signal on twelve channels, and a PWM whose duty
    /// wanders, to try virtual analog channels on.
    pub fn example() -> Self {
        Config {
            name: default_name(),
            listen: default_listen(),
            channel_count: 12,
            sample_rates_hz: default_rates(),
            seed: None,
            speed: 1.0,
            tile_ms: default_tile_ms(),
            clock_ppm: 0.0,
            signals: vec![
                Signal::Uart(Uart {
                    channel: 0,
                    name: None,
                    baud: 115_200.0,
                    data_bits: 8,
                    parity: Parity::None,
                    stop_bits: 1,
                    invert: false,
                    messages: vec![
                        "Hello from the virtual analyser\r\n".into(),
                        "temp=21.5C humidity=40%\r\n".into(),
                        "status OK\r\n".into(),
                    ],
                    random_bytes: default_random_bytes(),
                    every_ms: [30.0, 150.0],
                }),
                Signal::I2c(I2c {
                    scl: 1,
                    sda: 2,
                    clock_hz: 100e3,
                    addresses: vec![0x50, 0x68],
                    bytes: [1, 4],
                    read_chance: 0.3,
                    nack_chance: 0.05,
                    every_ms: [20.0, 120.0],
                }),
                Signal::Spi(Spi {
                    clk: 3,
                    cs: Some(4),
                    mosi: Some(5),
                    miso: Some(6),
                    clock_hz: 500e3,
                    mode: 0,
                    msb_first: true,
                    bytes: [1, 6],
                    every_ms: [15.0, 80.0],
                }),
                Signal::Pwm(Pwm {
                    channel: 7,
                    name: None,
                    frequency_hz: 1000.0,
                    duty: 0.25,
                    duty_range: None,
                    change_ms: default_change_ms(),
                }),
                Signal::Clock(Pwm {
                    channel: 8,
                    name: None,
                    frequency_hz: 10e3,
                    duty: 0.5,
                    duty_range: None,
                    change_ms: default_change_ms(),
                }),
                Signal::Noise(Noise { channel: 9, name: None, mean_us: 500.0 }),
                Signal::Shared(Shared { channel: 10, name: None, frequency_hz: default_shared_hz() }),
                Signal::Pwm(Pwm {
                    channel: 11,
                    name: Some("LEVEL".into()),
                    frequency_hz: 1000.0,
                    duty: 0.5,
                    duty_range: Some([0.05, 0.95]),
                    change_ms: default_change_ms(),
                }),
            ],
        }
    }
}

fn signal_problems(s: &Signal) -> Vec<String> {
    let mut p = Vec::new();
    let every = |p: &mut Vec<String>, e: &[f64; 2]| {
        if !range_ok(e) {
            p.push(format!("everyMs [{}, {}] must be two pauses from 0 up, shortest first.", e[0], e[1]));
        }
    };
    let bytes = |p: &mut Vec<String>, b: &[u32; 2], least: u32| {
        if b[0] < least || b[0] > b[1] || b[1] > 256 {
            p.push(format!("bytes [{}, {}] must go from {least} to at most 256, fewest first.", b[0], b[1]));
        }
    };
    match s {
        Signal::Uart(u) => {
            if !positive(u.baud) {
                p.push("baud must be above 0.".into());
            }
            if !(5..=9).contains(&u.data_bits) {
                p.push(format!("dataBits is {}; UART carries 5 to 9.", u.data_bits));
            }
            if !(1..=2).contains(&u.stop_bits) {
                p.push(format!("stopBits is {}; choose 1 or 2.", u.stop_bits));
            }
            if u.messages.is_empty() {
                bytes(&mut p, &u.random_bytes, 1);
            }
            every(&mut p, &u.every_ms);
        }
        Signal::I2c(i) => {
            if !positive(i.clock_hz) {
                p.push("clockHz must be above 0.".into());
            }
            if i.addresses.is_empty() {
                p.push("List at least one address.".into());
            }
            if let Some(a) = i.addresses.iter().find(|&&a| a > 0x7f) {
                p.push(format!("0x{a:02X} is not a 7-bit address (0x00 to 0x7F)."));
            }
            for (what, c) in [("readChance", i.read_chance), ("nackChance", i.nack_chance)] {
                if !(0.0..=1.0).contains(&c) {
                    p.push(format!("{what} is {c}; it is a share from 0 to 1."));
                }
            }
            bytes(&mut p, &i.bytes, 0);
            every(&mut p, &i.every_ms);
        }
        Signal::Spi(s) => {
            if !positive(s.clock_hz) {
                p.push("clockHz must be above 0.".into());
            }
            if s.mode > 3 {
                p.push(format!("mode is {}; SPI has modes 0 to 3.", s.mode));
            }
            if s.mosi.is_none() && s.miso.is_none() {
                p.push("Give SPI a MOSI or a MISO channel, or both.".into());
            }
            bytes(&mut p, &s.bytes, 1);
            every(&mut p, &s.every_ms);
        }
        Signal::Pwm(w) | Signal::Clock(w) => {
            if !positive(w.frequency_hz) {
                p.push("frequencyHz must be above 0.".into());
            }
            if matches!(s, Signal::Pwm(_)) && !(w.duty > 0.0 && w.duty < 1.0) {
                p.push(format!("duty is {}; it must be above 0 and below 1.", w.duty));
            }
            if let Some([lo, hi]) = w.duty_range {
                if matches!(s, Signal::Clock(_)) {
                    p.push("dutyRange is for pwm; a clock always runs at 50 % duty.".into());
                } else if !(lo > 0.0 && lo <= hi && hi < 1.0) {
                    p.push(format!(
                        "dutyRange is [{lo}, {hi}]; both must be above 0 and below 1, the first not above the second."
                    ));
                }
            }
            if !(range_ok(&w.change_ms) && w.change_ms[0] > 0.0) {
                p.push(format!(
                    "changeMs is [{}, {}]; give two times above 0 ms, the first not above the second.",
                    w.change_ms[0], w.change_ms[1]
                ));
            }
        }
        Signal::Noise(n) => {
            if !positive(n.mean_us) {
                p.push("meanUs must be above 0.".into());
            }
        }
        Signal::Shared(s) => {
            if !positive(s.frequency_hz) {
                p.push("frequencyHz must be above 0.".into());
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_is_valid_and_survives_json() {
        let c = Config::example();
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        let text = serde_json::to_string_pretty(&c).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, c);
        // SPI at 500 kHz needs 4 MS/s, so 1 MS/s is left out
        assert_eq!(c.min_rate_hz(), 4e6);
        assert_eq!(c.usable_rates(), vec![4e6, 10e6, 25e6, 100e6]);
        assert_eq!(c.channel_names()[0], "TX");
        assert_eq!(c.channel_names()[6], "MISO");
    }

    #[test]
    fn defaults_fill_what_a_short_file_leaves_out() {
        let c: Config = serde_json::from_str(
            r#"{"signals":[{"kind":"uart","channel":1},{"kind":"clock","channel":0,"frequencyHz":1000}]}"#,
        )
        .unwrap();
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        assert_eq!(c.listen, "127.0.0.1:7171");
        assert_eq!(c.channel_count, 8);
        let Signal::Uart(u) = &c.signals[0] else { panic!() };
        assert_eq!((u.baud, u.data_bits, u.stop_bits, u.parity), (115_200.0, 8, 1, Parity::None));
        assert_eq!(c.channel_names()[..3], ["CLOCK", "TX", "CH2"]);
    }

    #[test]
    fn unknown_fields_and_kinds_are_refused() {
        let e = serde_json::from_str::<Config>(r#"{"signals":[{"kind":"uart","channel":1,"bad":2}]}"#)
            .unwrap_err();
        assert!(e.to_string().contains("unknown field `bad`"), "{e}");
        let e = serde_json::from_str::<Config>(r#"{"signals":[{"kind":"can","channel":1}]}"#).unwrap_err();
        assert!(e.to_string().contains("unknown variant `can`"), "{e}");
        let e = serde_json::from_str::<Config>(r#"{"signal":[]}"#).unwrap_err();
        assert!(e.to_string().contains("unknown field `signal`"), "{e}");
    }

    #[test]
    fn every_problem_is_explained() {
        let c: Config = serde_json::from_str(
            r#"{"channelCount":4,"sampleRatesHz":[0],"speed":-1,"tileMs":0,"name":" ","signals":[
                {"kind":"uart","channel":4,"baud":0,"dataBits":12,"stopBits":3,"randomBytes":[5,2],"everyMs":[9,1]},
                {"kind":"i2c","scl":1,"sda":1,"addresses":[200],"readChance":2,"bytes":[0,300]},
                {"kind":"spi","clk":2,"mode":5},
                {"kind":"pwm","channel":3,"frequencyHz":10,"duty":1},
                {"kind":"noise","channel":2,"meanUs":0}
            ]}"#,
        )
        .unwrap();
        let p = c.problems().join("\n");
        for want in [
            "Give the analyser a name.",
            "sampleRatesHz must list at least one rate above 0.",
            "speed must be 0",
            "tileMs is 0",
            "Signal 1 uses channel 4, but channels go from 0 to 3.",
            "Signal 1: baud must be above 0.",
            "dataBits is 12",
            "stopBits is 3",
            "bytes [5, 2]",
            "everyMs [9, 1]",
            "Signal 2 uses channel 1 twice.",
            "0xC8 is not a 7-bit address",
            "readChance is 2",
            "bytes [0, 300]",
            "mode is 5",
            "Give SPI a MOSI or a MISO channel",
            "duty is 1",
            "Signals 3 and 5 both drive channel 2",
            "meanUs must be above 0.",
        ] {
            assert!(p.contains(want), "missing {want:?} in:\n{p}");
        }
    }

    #[test]
    fn a_wandering_duty_is_checked_and_described() {
        let c: Config = serde_json::from_str(
            r#"{"channelCount":4,"signals":[
                {"kind":"pwm","channel":0,"frequencyHz":100,"dutyRange":[0.9,0.1]},
                {"kind":"pwm","channel":1,"frequencyHz":100,"dutyRange":[0,0.5],"changeMs":[0,10]},
                {"kind":"clock","channel":2,"frequencyHz":100,"dutyRange":[0.2,0.4]},
                {"kind":"pwm","channel":3,"frequencyHz":1000,"dutyRange":[0.1,0.8],"changeMs":[50,60]}
            ]}"#,
        )
        .unwrap();
        let p = c.problems().join("\n");
        for want in [
            "Signal 1: dutyRange is [0.9, 0.1]",
            "Signal 2: dutyRange is [0, 0.5]",
            "Signal 2: changeMs is [0, 10]",
            "Signal 3: dutyRange is for pwm",
        ] {
            assert!(p.contains(want), "missing {want:?} in:\n{p}");
        }
        assert!(!p.contains("Signal 4"), "{p}");
        assert_eq!(
            c.signals[3].describe(),
            "PWM 1000 Hz, duty wandering between 10 % and 80 % (a glide every 50–60 ms)"
        );
        // the shortest pulse (10 % of 1 ms) sets the slowest faithful rate
        assert_eq!(c.signals[3].min_rate_hz(), STEPS * 1000.0 / 0.1);
    }

    #[test]
    fn rates_too_slow_for_a_signal_are_left_out() {
        let c: Config = serde_json::from_str(
            r#"{"sampleRatesHz":[100000,1000000,2000000],"signals":[{"kind":"uart","channel":0,"baud":115200}]}"#,
        )
        .unwrap();
        assert_eq!(c.usable_rates(), vec![1e6, 2e6]);
        let c: Config =
            serde_json::from_str(r#"{"sampleRatesHz":[1000],"signals":[{"kind":"uart","channel":0}]}"#)
                .unwrap();
        assert!(c.usable_rates().is_empty());
    }
}
