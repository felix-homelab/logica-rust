//! Generates the configured signals batch by batch, without end: every signal schedules its next
//! burst at a random pause after the last one, and its edges wait in the channel's lane until the
//! batch they fall in is sent. Edges come from Logica's own generators (`logica_protocols::synth`).

use logica_core::{Level, Sample};
use logica_protocols::synth::{I2cTransfer, Lane, UartOptions, i2c_transaction, spi_frame, uart_send};

use crate::config::{Config, I2c, Noise, Parity, Pwm, Shared, Signal, Spi, Uart};
use crate::rng::Rng;

enum Source {
    Uart(Uart),
    I2c(I2c),
    Spi(Spi),
    Square {
        channel: u32,
        period: f64,
        high: f64,
        /// A duty that wanders instead of staying at `high / period`.
        wander: Option<Wander>,
    },
    Noise(Noise),
    /// Half-periods of a clock on the computer's time: half-period `k` starts at `k * half_ns`
    /// nanoseconds after 1970 and is high when `k` is even.
    Shared {
        channel: u32,
        half_ns: u128,
        k: u128,
        clock: Clock,
    },
}

/// A duty gliding from `from` to `to` between the times `start` and `end` (seconds); then on to a
/// new random value in `range`.
struct Wander {
    range: [f64; 2],
    change_ms: [f64; 2],
    from: f64,
    to: f64,
    start: f64,
    end: f64,
}

impl Wander {
    /// The duty of the period starting at `t`.
    fn duty(&mut self, rng: &mut Rng, t: f64) -> f64 {
        while t >= self.end {
            self.from = self.to;
            self.to = rng.between(self.range[0], self.range[1]);
            self.start = self.end;
            self.end = self.start + pause(rng, self.change_ms);
        }
        let k = (t - self.start) / (self.end - self.start);
        self.from + (self.to - self.from) * k.clamp(0.0, 1.0)
    }
}

/// Where the capture sits on the computer's clock: sample 0 at `epoch_ns` (nanoseconds since
/// 1970), and the sample clock off by `scale` (1 + ppm / 1e6).
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub epoch_ns: u128,
    pub scale: f64,
}

impl Clock {
    /// Capture time in seconds of the computer's time `t_ns`, at or after the start.
    fn seconds(&self, t_ns: u128) -> f64 {
        // differences only: f64 keeps nanoseconds for days
        let d = t_ns.saturating_sub(self.epoch_ns);
        u64::try_from(d).map_or(f64::MAX, |d| d as f64) * 1e-9 * self.scale
    }
}

/// A signal and the time of its next burst, period or change, in seconds.
struct Scheduled {
    source: Source,
    next: f64,
    /// Its own random stream, so batch sizes and other signals do not change its data.
    rng: Rng,
}

pub struct Generator {
    rate: f64,
    /// One lane per channel; a lane's edges are the ones not sent yet.
    lanes: Vec<Lane>,
    sources: Vec<Scheduled>,
}

fn pause(rng: &mut Rng, every_ms: [f64; 2]) -> f64 {
    rng.between(every_ms[0], every_ms[1]) / 1e3
}

fn random_bytes(rng: &mut Rng, range: [u32; 2], printable: bool) -> Vec<u8> {
    let n = rng.int(range[0], range[1]);
    (0..n)
        .map(|_| {
            if printable {
                // letters, digits and punctuation, so the ASCII text row stays readable
                u8::try_from(rng.int(0x21, 0x7e)).unwrap_or(b'?')
            } else {
                u8::try_from(rng.int(0, 255)).unwrap_or(0)
            }
        })
        .collect()
}

impl Generator {
    /// Everything at its idle level; the first bursts come after a random pause. `clock` places the
    /// capture on the computer's time, for `shared` signals.
    pub fn new(config: &Config, rate: f64, seed: u64, clock: Clock) -> Self {
        let mut seeds = Rng::new(seed);
        let mut lanes: Vec<Lane> = (0..config.channel_count).map(|_| Lane::low(rate)).collect();
        let mut sources = Vec::new();
        for s in &config.signals {
            let mut rng = Rng::new(seeds.next_u64());
            let (source, first) = match s.clone() {
                Signal::Uart(u) => {
                    lanes[u.channel as usize] = Lane::new(rate, Level::from(!u.invert));
                    let first = pause(&mut rng, u.every_ms);
                    (Source::Uart(u), first)
                }
                Signal::I2c(i) => {
                    lanes[i.scl as usize] = Lane::high(rate);
                    lanes[i.sda as usize] = Lane::high(rate);
                    let first = pause(&mut rng, i.every_ms);
                    (Source::I2c(i), first)
                }
                Signal::Spi(sp) => {
                    // the clock idles at its polarity, chip select high
                    lanes[sp.clk as usize] = Lane::new(rate, Level::from(sp.mode >= 2));
                    if let Some(cs) = sp.cs {
                        lanes[cs as usize] = Lane::high(rate);
                    }
                    let first = pause(&mut rng, sp.every_ms);
                    (Source::Spi(sp), first)
                }
                // square waves rise after one period: a capture never has an edge at sample 0
                Signal::Pwm(Pwm { channel, frequency_hz, duty, duty_range, change_ms, .. }) => {
                    let period = 1.0 / frequency_hz;
                    let wander = duty_range.map(|range| {
                        let to = rng.between(range[0], range[1]);
                        Wander { range, change_ms, from: to, to, start: 0.0, end: 0.0 }
                    });
                    (Source::Square { channel, period, high: duty * period, wander }, period)
                }
                Signal::Clock(Pwm { channel, frequency_hz, .. }) => {
                    let period = 1.0 / frequency_hz;
                    (Source::Square { channel, period, high: 0.5 * period, wander: None }, period)
                }
                Signal::Noise(n) => {
                    let first = rng.exponential(n.mean_us * 1e-6);
                    (Source::Noise(n), first)
                }
                Signal::Shared(Shared { channel, frequency_hz, .. }) => {
                    let half_ns = u128::from(logica_protocols::num::round(5e8 / frequency_hz).max(1));
                    let now = clock.epoch_ns / half_ns;
                    lanes[channel as usize] = Lane::new(rate, Level::from(now.is_multiple_of(2)));
                    let k = now + 1;
                    (Source::Shared { channel, half_ns, k, clock }, clock.seconds(k * half_ns))
                }
            };
            sources.push(Scheduled { source, next: first, rng });
        }
        Generator { rate, lanes, sources }
    }

    /// Levels of the channels at sample 0.
    pub fn initial(&self, channels: &[u32]) -> Vec<bool> {
        channels.iter().map(|&c| self.lanes[c as usize].initial == Level::High).collect()
    }

    /// Generates every burst that starts before `until` seconds. Bursts may run past it; their
    /// later edges wait in the lanes. Nothing generated later lands before `until`.
    pub fn fill(&mut self, until: f64) {
        let lanes = &mut self.lanes;
        for s in &mut self.sources {
            let rng = &mut s.rng;
            while s.next < until {
                let t = s.next;
                s.next = match &mut s.source {
                    Source::Uart(u) => {
                        let bytes = if u.messages.is_empty() {
                            random_bytes(rng, u.random_bytes, true)
                        } else {
                            let k = rng.int(0, u32::try_from(u.messages.len() - 1).unwrap_or(0));
                            u.messages[k as usize].as_bytes().to_vec()
                        };
                        let o = UartOptions {
                            baud: u.baud,
                            data_bits: u.data_bits,
                            parity: match u.parity {
                                Parity::None => "none",
                                Parity::Even => "even",
                                Parity::Odd => "odd",
                            },
                            stop_bits: u.stop_bits,
                            invert: u.invert,
                            gap_bits: 0.0,
                        };
                        let end = uart_send(&mut lanes[u.channel as usize], t, &bytes, o);
                        end + pause(rng, u.every_ms)
                    }
                    Source::I2c(i) => {
                        let k = rng.int(0, u32::try_from(i.addresses.len() - 1).unwrap_or(0));
                        let x = I2cTransfer {
                            address: i.addresses[k as usize],
                            read: rng.chance(i.read_chance),
                            data: random_bytes(rng, i.bytes, false),
                            nack: rng.chance(i.nack_chance),
                        };
                        let [scl, sda] = lanes
                            .get_disjoint_mut([i.scl as usize, i.sda as usize])
                            .expect("checked: I2C uses two different channels");
                        let end = i2c_transaction(scl, sda, t, i.clock_hz, &[x]);
                        end + pause(rng, i.every_ms)
                    }
                    Source::Spi(sp) => {
                        let mosi_bytes = random_bytes(rng, sp.bytes, false);
                        let miso_bytes = random_bytes(
                            rng,
                            [u32::try_from(mosi_bytes.len()).unwrap_or(u32::MAX); 2],
                            false,
                        );
                        let end = spi_burst(lanes, sp, t, &mosi_bytes, &miso_bytes);
                        end + pause(rng, sp.every_ms)
                    }
                    Source::Square { channel, period, high, wander } => {
                        let high = match wander {
                            Some(w) => w.duty(rng, t) * *period,
                            None => *high,
                        };
                        let lane = &mut lanes[*channel as usize];
                        lane.set(t, Level::High);
                        lane.set(t + high, Level::Low);
                        t + *period
                    }
                    Source::Noise(n) => {
                        let lane = &mut lanes[n.channel as usize];
                        let to = !lane.level;
                        lane.set(t, to);
                        t + rng.exponential(n.mean_us * 1e-6)
                    }
                    Source::Shared { channel, half_ns, k, clock } => {
                        lanes[*channel as usize].set(t, Level::from(k.is_multiple_of(2)));
                        *k += 1;
                        clock.seconds(*k * *half_ns)
                    }
                };
            }
        }
    }

    /// Takes the waiting edges of `channel` before sample `end`.
    pub fn take(&mut self, channel: u32, end: Sample) -> Vec<Sample> {
        let lane = &mut self.lanes[channel as usize];
        let n = lane.edges.partition_point(|&e| e < end);
        lane.edges.drain(..n).collect()
    }

    /// The sample at time `t` seconds, as the generators round it.
    pub fn sample_at(&self, t: f64) -> Sample {
        // the same rounding as the generators' edges
        logica_protocols::num::round(t * self.rate)
    }
}

/// One SPI frame on whichever of CS, MOSI and MISO are configured.
fn spi_burst(lanes: &mut [Lane], sp: &Spi, t: f64, mosi: &[u8], miso: &[u8]) -> f64 {
    // take the lanes out, so the generator can hold them all at once, then put them back
    let take = |lanes: &mut [Lane], c: Option<u32>| {
        c.map(|c| std::mem::replace(&mut lanes[c as usize], Lane::low(0.0)))
    };
    let mut clk = take(lanes, Some(sp.clk)).expect("the clock is always there");
    let mut cs = take(lanes, sp.cs);
    let mut mo = take(lanes, sp.mosi);
    let mut mi = take(lanes, sp.miso);
    let end = spi_frame(
        &mut clk,
        cs.as_mut(),
        mo.as_mut(),
        mi.as_mut(),
        t,
        sp.clock_hz,
        sp.mode,
        mosi,
        miso,
        sp.msb_first,
    );
    let mut put = |c: Option<u32>, l: Option<Lane>| {
        if let (Some(c), Some(l)) = (c, l) {
            lanes[c as usize] = l;
        }
    };
    put(Some(sp.clk), Some(clk));
    put(sp.cs, cs);
    put(sp.mosi, mo);
    put(sp.miso, mi);
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use logica_core::capture::{
        CaptureMeta, CaptureState, ChannelInfo, Source as CapSource, SourceKind, whole,
    };
    use logica_core::store::CaptureStore;
    use logica_protocols::analyse::{Batching, decode_store};
    use logica_protocols::types::{ProblemCode, Roles, settings};

    /// Runs the generator for `seconds` in batches of `tile` seconds, as the server does.
    fn capture(
        config: &Config,
        rate: f64,
        seconds: f64,
        tile: f64,
        seed: u64,
    ) -> (CaptureStore, CaptureMeta) {
        let mut g =
            Generator::new(config, rate, seed, Clock { epoch_ns: 1_700_000_000_123_456_789, scale: 1.0 });
        let channels: Vec<u32> = (0..config.channel_count).collect();
        let initial: Vec<Level> = g.initial(&channels).into_iter().map(Level::from).collect();
        let mut store =
            CaptureStore::new(&initial, std::sync::Arc::new(logica_core::io::MemFile::new()), 16, 64 << 20);
        let mut t = 0.0;
        let total = g.sample_at(seconds);
        while t < seconds {
            t = (t + tile).min(seconds);
            g.fill(t);
            let end = g.sample_at(t).min(total);
            for (slot, &c) in channels.iter().enumerate() {
                store.push_edges(slot, &g.take(c, end)).unwrap();
            }
            store.advance_to(end).unwrap();
        }
        store.finish(total).unwrap();
        let names = config.channel_names();
        let meta = CaptureMeta {
            id: "t".into(),
            title: "t".into(),
            source: CapSource { kind: SourceKind::Synthetic, detail: "test".into() },
            sample_rate_hz: rate,
            trigger: None,
            channels: channels
                .iter()
                .map(|&i| ChannelInfo {
                    index: i,
                    name: names[i as usize].clone(),
                    colour: None,
                    enabled: true,
                    threshold_volts: None,
                })
                .collect(),
            segments: whole(total),
            notes: Vec::new(),
            state: CaptureState::Complete,
            artefact: None,
            saved_decoders: Vec::new(),
        };
        (store, meta)
    }

    fn roles(list: &[(&str, u32)]) -> Roles {
        list.iter().map(|&(k, c)| (k.to_string(), Some(c))).collect()
    }

    fn text_of(r: &logica_protocols::types::DecodeResult) -> String {
        r.frames
            .iter()
            .filter(|f| f.kind == logica_protocols::types::FrameKind::Data)
            .filter_map(|f| f.value)
            .map(|v| char::from(u8::try_from(logica_protocols::num::round(v)).unwrap_or(b'?')))
            .collect()
    }

    #[test]
    fn the_example_decodes_cleanly_across_batches() {
        let mut c = Config::example();
        c.seed = Some(7);
        let reg = logica_protocols::registry();
        let (store, meta) = capture(&c, 4e6, 0.6, 0.013, 7);
        let s = Default::default();
        let uart =
            decode_store(&reg, &store, &meta, "uart", &roles(&[("data", 0)]), &s, Batching::Whole).unwrap();
        let text = text_of(&uart);
        assert!(
            text.contains("Hello from the virtual analyser")
                || text.contains("status OK")
                || text.contains("temp="),
            "{text:?}"
        );
        assert!(uart.problem_counts.is_empty(), "{:?}", uart.problem_counts);
        let i2c =
            decode_store(&reg, &store, &meta, "i2c", &roles(&[("scl", 1), ("sda", 2)]), &s, Batching::Whole)
                .unwrap();
        assert!(i2c.packets.len() >= 3, "{} I2C packets", i2c.packets.len());
        // only a transfer cut off by the end of the capture may be incomplete
        assert!(i2c.problem_counts.keys().all(|&p| p == ProblemCode::Incomplete), "{:?}", i2c.problem_counts);
        let spi = decode_store(
            &reg,
            &store,
            &meta,
            "spi",
            &roles(&[("clk", 3), ("cs", 4), ("mosi", 5), ("miso", 6)]),
            &s,
            Batching::Whole,
        )
        .unwrap();
        assert!(spi.packets.len() >= 5, "{} SPI packets", spi.packets.len());
        assert!(spi.problem_counts.is_empty(), "{:?}", spi.problem_counts);
        // PWM at 1 kHz, 25 %: two edges a period
        let pwm = store.edges_in(7, 0, store.sample_count()).unwrap();
        assert!((1190..=1201).contains(&pwm.len()), "{} PWM edges", pwm.len());
        assert_eq!(pwm[1] - pwm[0], 1000, "high for 250 µs at 4 MS/s");
    }

    #[test]
    fn the_level_pwm_wanders_smoothly_inside_its_range() {
        // channel 11 of the example: 1 kHz, duty between 5 % and 95 %
        let (store, _) = capture(&Config::example(), 1e6, 3.0, 0.02, 5);
        let e = store.edges_in(11, 0, store.sample_count()).unwrap();
        let duties: Vec<f64> =
            e.windows(3).step_by(2).map(|w| (w[1] - w[0]) as f64 / (w[2] - w[0]) as f64).collect();
        assert!(duties.len() > 2900, "{} periods", duties.len());
        let lo = duties.iter().copied().fold(f64::MAX, f64::min);
        let hi = duties.iter().copied().fold(f64::MIN, f64::max);
        assert!(lo >= 0.049 && hi <= 0.951, "duty from {lo} to {hi}");
        assert!(hi - lo > 0.3, "it wanders: {lo} to {hi}");
        // a glide takes at least 200 periods, so neighbours differ by little
        let step = duties.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(step < 0.01, "largest change between periods {step}");
        // every period is still 1 ms
        assert!(e.windows(3).step_by(2).all(|w| w[2] - w[0] == 1000));
    }

    #[test]
    fn a_seed_repeats_and_batch_size_does_not_matter() {
        let c = Config::example();
        let (a, _) = capture(&c, 10e6, 0.3, 0.02, 99);
        let (b, _) = capture(&c, 10e6, 0.3, 0.0031, 99);
        let (d, _) = capture(&c, 10e6, 0.3, 0.02, 100);
        let edges = |s: &CaptureStore| {
            (0..c.channel_count as usize)
                .map(|k| s.edges_in(k, 0, s.sample_count()).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(edges(&a), edges(&b));
        assert_ne!(edges(&a), edges(&d), "another seed gives other data");
    }

    #[test]
    fn analysers_on_one_computer_see_the_same_shared_clock_at_their_own_samples() {
        let config = Config::example();
        let rate = 1e6;
        let epoch = 1_700_000_000_123_456_789;
        // B starts 3.21 ms later, and its sample clock runs 200 ppm fast
        let mut a = Generator::new(&config, rate, 1, Clock { epoch_ns: epoch, scale: 1.0 });
        let mut b =
            Generator::new(&config, rate, 2, Clock { epoch_ns: epoch + 3_210_000, scale: 1.0 + 200e-6 });
        a.fill(1.0);
        b.fill(1.0);
        let (ea, eb) = (a.take(10, 1_000_000), b.take(10, 1_000_000));
        // 100 Hz: two edges every 10 ms
        assert!((198..=201).contains(&ea.len()), "{}", ea.len());
        let mut matched = 0;
        for &x in &ea {
            // the same moment on B's clock
            let y = (x as f64 / rate - 3.21e-3) * rate * (1.0 + 200e-6);
            if y < 0.0 || y > 999_000.0 {
                continue;
            }
            let near = eb.iter().map(|&e| (e as f64 - y).abs()).fold(f64::MAX, f64::min);
            assert!(near <= 1.0, "A's edge {x} is {near} samples from B's");
            matched += 1;
        }
        assert!(matched > 190, "{matched}");
        // the level at sample 0 is the shared clock's at that moment
        let phase_ms = (epoch / 1_000_000) % 10;
        assert_eq!(a.initial(&[10]), [phase_ms < 5]);
    }

    #[test]
    fn uart_messages_arrive_in_full() {
        let c: Config = serde_json::from_str(
            r#"{"signals":[{"kind":"uart","channel":0,"baud":9600,"parity":"even","messages":["ping\n"],"everyMs":[1,2]}]}"#,
        )
        .unwrap();
        let (store, meta) = capture(&c, 1e6, 0.1, 0.007, 1);
        let reg = logica_protocols::registry();
        let s = settings(&[("baud", 9600.0.into()), ("parity", "even".into())]);
        let r =
            decode_store(&reg, &store, &meta, "uart", &roles(&[("data", 0)]), &s, Batching::Whole).unwrap();
        assert!(text_of(&r).starts_with("ping\nping\n"), "{:?}", text_of(&r));
        // only a byte cut off by the end of the capture may be wrong (11 bits at 9600 baud)
        let cut = store.sample_count() - 1150;
        assert!(r.frames.iter().filter(|f| !f.problems.is_empty()).all(|f| f.end >= cut));
    }
}
