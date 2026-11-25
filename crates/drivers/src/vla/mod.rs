//! The virtual analyser driver (doc 05): streams from `logica-vla` (`tools/virtual-analyser`), a
//! separate program that behaves like a streaming logic analyser on a TCP port, so live capture
//! and live decoding can be built and tried without hardware. The engine connects out to it as a
//! client; nothing connects into the engine (ADR 0006 is unaffected).
//!
//! Where to look: `LOGICA_VLA` (comma-separated `host:port`), else `127.0.0.1:7171`. The driver
//! lists an analyser only while it answers, so starting or stopping the program shows in the
//! next `device.list`.

pub mod wire;

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use logica_core::capture::{ChannelInfo, Note, NoteLevel};
use logica_core::device::{
    Block, BlockSink, Capabilities, CaptureConfig, CaptureMode, Device, DeviceError, DeviceInfo, RunOutcome,
    StopSignal, VirtualDriver,
};
use logica_core::{Level, Sample};

use wire::{Hello, MAX_LINE, Message, PROTOCOL, Request, Start, VERSION};

/// Where the analyser listens unless `LOGICA_VLA` says otherwise.
pub const DEFAULT_ADDRESS: &str = "127.0.0.1:7171";
/// The environment variable with the addresses to look at.
pub const ENV: &str = "LOGICA_VLA";

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// How often a running capture looks at its stop signal.
const STOP_POLL: Duration = Duration::from_millis(50);
/// How long a stopped capture waits for the analyser's end message.
const STOP_GRACE: Duration = Duration::from_secs(5);

const COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];

pub struct VlaDriver {
    addresses: Vec<String>,
}

impl VlaDriver {
    pub fn new(addresses: Vec<String>) -> Self {
        VlaDriver { addresses }
    }
}

/// Addresses from `LOGICA_VLA` (comma-separated `host:port`), else [`DEFAULT_ADDRESS`].
pub fn addresses_from_env() -> Vec<String> {
    parse_addresses(&std::env::var(ENV).unwrap_or_default())
}

fn parse_addresses(list: &str) -> Vec<String> {
    let addresses: Vec<String> =
        list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    if addresses.is_empty() { vec![DEFAULT_ADDRESS.into()] } else { addresses }
}

fn resolve(address: &str) -> Result<SocketAddr, DeviceError> {
    address
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .ok_or_else(|| DeviceError::new(format!("\"{address}\" is not an address such as 127.0.0.1:7171.")))
}

/// Connects and reads the greeting.
fn greet(address: &str) -> Result<(BufReader<TcpStream>, Hello), DeviceError> {
    let addr = resolve(address)?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| {
        DeviceError::new(format!(
            "No virtual analyser answers at {address} ({e}). Start it with `cargo run -p logica-vla`."
        ))
    })?;
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(HELLO_TIMEOUT)).map_err(io)?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    (&mut reader).take(MAX_LINE as u64).read_until(b'\n', &mut line).map_err(|e| {
        DeviceError::new(format!("The program at {address} did not greet like a virtual analyser ({e})."))
    })?;
    let hello: Hello = serde_json::from_slice(&line).map_err(|_| {
        DeviceError::new(format!(
            "The program at {address} is not a virtual analyser: its greeting is not one."
        ))
    })?;
    if hello.protocol != PROTOCOL {
        return Err(DeviceError::new(format!(
            "The program at {address} speaks \"{}\", not {PROTOCOL}.",
            hello.protocol
        )));
    }
    if hello.version != VERSION {
        return Err(DeviceError::new(format!(
            "The virtual analyser at {address} speaks version {} of its protocol; this Logica speaks version {VERSION}.",
            hello.version
        )));
    }
    if hello.channels.is_empty() || hello.sample_rates_hz.is_empty() {
        return Err(DeviceError::new(format!(
            "The virtual analyser at {address} offers no channels or rates."
        )));
    }
    Ok((reader, hello))
}

fn io(e: std::io::Error) -> DeviceError {
    DeviceError::new(format!("The connection to the virtual analyser failed: {e}."))
}

fn info(address: &str, hello: &Hello) -> DeviceInfo {
    DeviceInfo {
        id: format!("vla/{address}"),
        driver: "vla".into(),
        model: format!("Virtual analyser: {}", hello.model),
        serial: Some(hello.serial.clone()),
        firmware: Some(format!("{PROTOCOL} {}", hello.version)),
        channels: u32::try_from(hello.channels.len()).unwrap_or(u32::MAX),
    }
}

impl VirtualDriver for VlaDriver {
    fn id(&self) -> &str {
        "vla"
    }
    fn name(&self) -> &str {
        "Virtual analyser"
    }
    fn hotplug(&self) -> bool {
        true
    }
    fn enumerate(&self) -> Vec<DeviceInfo> {
        self.addresses.iter().filter_map(|a| greet(a).ok().map(|(_, h)| info(a, &h))).collect()
    }
    fn open(&self, id: &str) -> Result<Box<dyn Device>, DeviceError> {
        let address = id.strip_prefix("vla/").unwrap_or(id);
        let (_, hello) = greet(address)?;
        Ok(Box::new(VlaDevice::new(address.into(), hello)))
    }
}

pub struct VlaDevice {
    address: String,
    hello: Hello,
    info: DeviceInfo,
    caps: Capabilities,
    config: Option<CaptureConfig>,
}

impl VlaDevice {
    pub fn new(address: String, hello: Hello) -> Self {
        let caps = Capabilities {
            channels: hello.channels.iter().map(|c| c.index).collect(),
            rates_hz: hello.sample_rates_hz.clone(),
            limits: Vec::new(),
            max_samples: None,
            modes: vec![CaptureMode::Stream],
            threshold: None,
            outputs: 0,
        };
        VlaDevice { info: info(&address, &hello), address, hello, caps, config: None }
    }
}

/// Bytes read from the analyser, cut into messages; a read that times out is not an error.
struct Inbox {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Inbox {
    /// The next message, `Ok(None)` when nothing complete arrived within the read timeout, or an
    /// error when the connection closed.
    fn next(&mut self) -> Result<Option<Message>, DeviceError> {
        loop {
            if let Some((m, n)) = Message::decode(&self.buf).map_err(DeviceError::new)? {
                self.buf.drain(..n);
                return Ok(Some(m));
            }
            let mut chunk = [0u8; 64 << 10];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(DeviceError::new("The virtual analyser closed the connection.")),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(io(e)),
            }
        }
    }
}

impl Device for VlaDevice {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn configure(&mut self, config: &CaptureConfig) -> Result<(), DeviceError> {
        self.caps.check(config)?;
        self.config = Some(config.clone());
        Ok(())
    }

    fn run(&mut self, sink: &mut dyn BlockSink, stop: &StopSignal) -> Result<RunOutcome, DeviceError> {
        let cfg =
            self.config.clone().ok_or_else(|| DeviceError::new("Configure the device before capturing."))?;
        let (reader, hello) = greet(&self.address)?;
        if hello.channels != self.hello.channels || hello.sample_rates_hz != self.hello.sample_rates_hz {
            return Err(DeviceError::new(
                "The virtual analyser was restarted with other channels or rates; connect to it again.",
            ));
        }
        // anything after the greeting line is already capture data
        let early = reader.buffer().to_vec();
        let mut stream = reader.into_inner();
        stream.set_read_timeout(Some(STOP_POLL)).map_err(io)?;
        let start = Start {
            channels: cfg.channels.clone(),
            sample_rate_hz: cfg.sample_rate_hz,
            samples: cfg.samples,
        };
        stream.write_all(&wire::line(&Request::Start(start))).map_err(io)?;
        let mut inbox = Inbox { stream, buf: early };

        let infos: Vec<ChannelInfo> = cfg
            .channels
            .iter()
            .map(|&ch| ChannelInfo {
                index: ch,
                name: hello
                    .channels
                    .iter()
                    .find(|c| c.index == ch)
                    .map_or_else(|| format!("CH{ch}"), |c| c.name.clone()),
                colour: Some(COLOURS[ch as usize % 4].into()),
                enabled: true,
                threshold_volts: None,
            })
            .collect();
        let mut begun = false;
        let mut known: Sample = 0;
        let mut stop_sent: Option<Instant> = None;
        loop {
            if stop.is_stopped() && stop_sent.is_none() {
                // the analyser may already be gone; its end message or the closed socket ends the loop
                let _ = inbox.stream.write_all(&wire::line(&Request::Stop(true)));
                stop_sent = Some(Instant::now());
            }
            if let Some(t) = stop_sent
                && t.elapsed() > STOP_GRACE
            {
                sink.note(Note {
                    level: NoteLevel::Warning,
                    message: format!(
                        "The virtual analyser did not confirm the stop; the capture ends at {known} samples."
                    ),
                });
                return Ok(RunOutcome { samples: known, stopped: true });
            }
            let m = match inbox.next() {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(e) if begun => {
                    sink.note(Note {
                        level: NoteLevel::Warning,
                        message: format!(
                            "{} The capture ends at {known} samples, the last data received.",
                            e.0
                        ),
                    });
                    return Ok(RunOutcome { samples: known, stopped: true });
                }
                Err(e) => return Err(e),
            };
            match m {
                Message::Begin { initial } => {
                    if begun || initial.len() != infos.len() {
                        return Err(DeviceError::new(
                            "The virtual analyser began the capture with other channels than were asked for.",
                        ));
                    }
                    let levels: Vec<Level> = initial.into_iter().map(Level::from).collect();
                    sink.begin(&infos, &levels, cfg.sample_rate_hz)?;
                    begun = true;
                }
                Message::Edges { slot, samples } => {
                    let slot = usize::from(slot);
                    if !begun || slot >= infos.len() {
                        return Err(DeviceError::new(
                            "The virtual analyser sent edges for a channel it did not begin.",
                        ));
                    }
                    sink.block(Block::Transitions { slot, edges: &samples })?;
                }
                Message::Advance { end } => {
                    if !begun {
                        return Err(DeviceError::new(
                            "The virtual analyser sent data before it began the capture.",
                        ));
                    }
                    sink.block(Block::Advance { end })?;
                    known = end;
                }
                Message::Note { text } => sink.note(Note { level: NoteLevel::Info, message: text }),
                Message::End { samples } => {
                    if !begun {
                        return Err(DeviceError::new("The virtual analyser ended a capture it never began."));
                    }
                    if samples > known {
                        sink.block(Block::Advance { end: samples })?;
                    }
                    let samples = samples.max(known);
                    return Ok(RunOutcome { samples, stopped: stop.is_stopped() || samples < cfg.samples });
                }
                Message::Error { text } => return Err(DeviceError::new(text)),
            }
        }
    }

    fn close(&mut self) {
        self.config = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_come_from_a_list_or_the_default() {
        assert_eq!(super::parse_addresses(""), ["127.0.0.1:7171"]);
        assert_eq!(super::parse_addresses(" , "), ["127.0.0.1:7171"]);
        assert_eq!(super::parse_addresses("a:1, b:2,"), ["a:1", "b:2"]);
    }
}
