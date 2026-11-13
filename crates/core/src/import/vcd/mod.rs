//! Value change dump files (IEEE 1364 VCD, doc 09). Format facts and choices in PROVENANCE.md.
//!
//! Read in two passes, a piece at a time: the first reads the definitions and the timestamps (the
//! first timestamp is sample 0; one sample is the greatest common divisor of the time steps), the
//! second writes the value changes into the store. One-bit variables become channels, vectors one
//! channel per bit; x and z read as low (counted in a note); real variables are skipped. Extended
//! VCD (signal strengths) is not handled.

use std::collections::HashMap;

use super::{ImportError, Imported, Progress, StoreTarget, info, warn};
use crate::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, Source, SourceKind};
use crate::io::RandomAccess;
use crate::store::{CaptureStore, Level, Sample};

const PIECE: usize = 1 << 20;
const COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];

/// Whitespace-separated tokens, read from the file a piece at a time.
struct Tokens<'a> {
    file: &'a dyn RandomAccess,
    size: u64,
    pos: u64,
    buf: Vec<u8>,
    i: usize,
    tok: Vec<u8>,
}

impl<'a> Tokens<'a> {
    fn new(file: &'a dyn RandomAccess) -> Result<Self, ImportError> {
        Ok(Tokens {
            file,
            size: file.size().map_err(ImportError::io)?,
            pos: 0,
            buf: Vec::new(),
            i: 0,
            tok: Vec::new(),
        })
    }

    fn fill(&mut self) -> Result<bool, ImportError> {
        if self.pos >= self.size {
            return Ok(false);
        }
        let n = usize::try_from((self.size - self.pos).min(PIECE as u64)).unwrap_or(PIECE);
        self.buf.resize(n, 0);
        self.file.read_exact_at(self.pos, &mut self.buf).map_err(ImportError::io)?;
        self.pos += n as u64;
        self.i = 0;
        Ok(true)
    }

    /// Reads the next token into `self.tok`; `false` at the end of the file.
    fn next(&mut self) -> Result<bool, ImportError> {
        self.tok.clear();
        loop {
            if self.i >= self.buf.len() && !self.fill()? {
                return Ok(!self.tok.is_empty());
            }
            let b = self.buf[self.i];
            self.i += 1;
            if b.is_ascii_whitespace() {
                if !self.tok.is_empty() {
                    return Ok(true);
                }
            } else {
                self.tok.push(b);
            }
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.tok).into_owned()
    }

    /// Skips to the next `$end`, returning the tokens before it.
    fn until_end(&mut self, what: &str) -> Result<Vec<String>, ImportError> {
        let mut out = Vec::new();
        loop {
            if !self.next()? {
                return Err(ImportError::new(format!(
                    "The VCD file ends inside a {what} command (no $end)."
                )));
            }
            if self.tok == b"$end" {
                return Ok(out);
            }
            out.push(self.text());
        }
    }

    fn progress(&self) -> f64 {
        if self.size == 0 {
            1.0
        } else {
            (self.pos as f64 - (self.buf.len() - self.i) as f64) / self.size as f64
        }
    }
}

struct Var {
    kind: String,
    size: u32,
    id: String,
    reference: String,
    /// Declared bit range `[left:right]`, if any.
    range: Option<(i64, i64)>,
    scope: Vec<String>,
}

/// Seconds per time unit as a power of ten and a factor: "10 ns" is (10, -9).
fn parse_timescale(parts: &[String]) -> Option<(u64, i32)> {
    let joined: String = parts.concat();
    let split = joined.find(|c: char| !c.is_ascii_digit())?;
    let factor: u64 = joined[..split].parse().ok()?;
    let exp = match &joined[split..] {
        "s" => 0,
        "ms" => -3,
        "us" => -6,
        "ns" => -9,
        "ps" => -12,
        "fs" => -15,
        _ => return None,
    };
    [1, 10, 100].contains(&factor).then_some((factor, exp))
}

fn parse_range(s: &str) -> Option<(i64, i64)> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    match inner.split_once(':') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => {
            let v = inner.trim().parse().ok()?;
            Some((v, v))
        }
    }
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// What a value token is.
enum Change<'t> {
    Scalar(u8),
    Vector(&'t [u8]),
    Real,
}

fn classify(tok: &[u8]) -> Result<Change<'_>, ImportError> {
    match tok.first() {
        Some(b'0' | b'1' | b'x' | b'X' | b'z' | b'Z') => Ok(Change::Scalar(tok[0])),
        Some(b'b' | b'B') => Ok(Change::Vector(&tok[1..])),
        Some(b'r' | b'R') => Ok(Change::Real),
        Some(b'p' | b'P') => Err(ImportError::new(
            "This is an extended VCD file (with signal strengths), which this tool does not read.",
        )),
        _ => Err(ImportError::new(format!(
            "The VCD file has a value this tool cannot read: \"{}\".",
            String::from_utf8_lossy(tok)
        ))),
    }
}

/// The definitions and the timestamps of a VCD file (the first pass).
struct Header {
    timescale: (u64, i32),
    vars: Vec<Var>,
    t0: Option<u64>,
    last: u64,
    step: u64,
    changes_at_last: bool,
    any_change: bool,
    unknown: u64,
}

fn first_pass(file: &dyn RandomAccess, progress: Progress<'_>) -> Result<Header, ImportError> {
    let mut t = Tokens::new(file)?;
    let mut timescale = None;
    let mut scope: Vec<String> = Vec::new();
    let mut vars = Vec::new();
    let mut first = true;
    loop {
        if !t.next()? {
            return Err(ImportError::new(
                "The VCD file ends before its definitions are complete ($enddefinitions is missing).",
            ));
        }
        if first && !t.tok.starts_with(b"$") {
            return Err(ImportError::new("This is not a VCD file: it does not start with a $ command."));
        }
        first = false;
        match t.tok.as_slice() {
            b"$timescale" => {
                let parts = t.until_end("$timescale")?;
                timescale = Some(parse_timescale(&parts).ok_or_else(|| {
                    ImportError::new(format!("The VCD timescale \"{}\" cannot be read.", parts.join(" ")))
                })?);
            }
            b"$scope" => {
                let parts = t.until_end("$scope")?;
                scope.push(parts.get(1).cloned().unwrap_or_default());
            }
            b"$upscope" => {
                t.until_end("$upscope")?;
                scope.pop();
            }
            b"$var" => {
                let p = t.until_end("$var")?;
                if p.len() < 4 {
                    return Err(ImportError::new(format!(
                        "A VCD $var line is incomplete: \"{}\".",
                        p.join(" ")
                    )));
                }
                let size = p[1].parse::<u32>().map_err(|_| {
                    ImportError::new(format!("A VCD $var has an unreadable size: \"{}\".", p[1]))
                })?;
                let (reference, range) = match p.get(4) {
                    Some(r) => (p[3].clone(), parse_range(r)),
                    None => match p[3].find('[') {
                        Some(i) => (p[3][..i].to_string(), parse_range(&p[3][i..])),
                        None => (p[3].clone(), None),
                    },
                };
                vars.push(Var {
                    kind: p[0].clone(),
                    size,
                    id: p[2].clone(),
                    reference,
                    range,
                    scope: scope.clone(),
                });
            }
            b"$enddefinitions" => {
                t.until_end("$enddefinitions")?;
                break;
            }
            tok if tok.starts_with(b"$") => {
                let what = t.text();
                t.until_end(&what)?;
            }
            _ => {
                return Err(ImportError::new(format!(
                    "The VCD file has \"{}\" among its definitions, where only $ commands belong.",
                    t.text()
                )));
            }
        }
    }
    let timescale = timescale.unwrap_or((1, -9));
    let mut h = Header {
        timescale,
        vars,
        t0: None,
        last: 0,
        step: 0,
        changes_at_last: false,
        any_change: false,
        unknown: 0,
    };
    while t.next()? {
        match t.tok.first() {
            Some(b'#') => {
                let v: u64 =
                    std::str::from_utf8(&t.tok[1..]).ok().and_then(|s| s.parse().ok()).ok_or_else(|| {
                        ImportError::new(format!("The VCD timestamp \"{}\" cannot be read.", t.text()))
                    })?;
                match h.t0 {
                    None => h.t0 = Some(v),
                    Some(_) if v < h.last => {
                        return Err(ImportError::new(format!(
                            "Time goes backwards in the VCD file (#{v} after #{}).",
                            h.last
                        )));
                    }
                    Some(_) => {
                        if v > h.last {
                            h.step = gcd(h.step, v - h.last);
                            h.changes_at_last = false;
                        }
                    }
                }
                h.last = v;
            }
            Some(b'$') => {
                if t.tok == b"$comment" {
                    t.until_end("$comment")?;
                }
            }
            _ => {
                let unknown = match classify(&t.tok)? {
                    Change::Scalar(c) => u64::from(!matches!(c, b'0' | b'1')),
                    Change::Vector(bits) => bits.iter().filter(|c| !matches!(c, b'0' | b'1')).count() as u64,
                    Change::Real => 0,
                };
                h.unknown += unknown;
                if !matches!(t.tok.first(), Some(b'0' | b'1' | b'x' | b'X' | b'z' | b'Z')) && !t.next()? {
                    return Err(ImportError::new("The VCD file ends in the middle of a value change."));
                }
                h.any_change = true;
                h.changes_at_last = true;
            }
        }
        progress(t.progress() * 0.5, "Reading the VCD time steps");
    }
    Ok(h)
}

/// One channel and where its level comes from.
struct Target {
    slot: usize,
    /// Position from the right (least significant) end of the variable's value.
    bit: u32,
}

pub fn import(
    file: &dyn RandomAccess,
    title: &str,
    target: &StoreTarget,
    progress: Progress<'_>,
) -> Result<Imported, ImportError> {
    let h = first_pass(file, progress)?;
    let mut notes: Vec<Note> = Vec::new();

    // Channels: one per one-bit variable, one per bit of a vector.
    let mut channels: Vec<(String, Vec<String>)> = Vec::new(); // name, scope
    let mut by_id: HashMap<String, Vec<Target>> = HashMap::new();
    for v in &h.vars {
        if v.kind == "real" || v.kind == "realtime" {
            info(
                &mut notes,
                format!("{} is a real-valued variable; it is not shown (digital signals only).", v.reference),
            );
            continue;
        }
        if v.size == 0 {
            continue;
        }
        let (left, right) = v.range.unwrap_or((i64::from(v.size) - 1, 0));
        let dir: i64 = if left >= right { 1 } else { -1 };
        for p in 0..v.size {
            let name = if v.size == 1 && v.range.is_none_or(|(a, b)| a == b) {
                v.reference.clone()
            } else {
                format!("{}[{}]", v.reference, right + dir * i64::from(p))
            };
            by_id.entry(v.id.clone()).or_default().push(Target { slot: channels.len(), bit: p });
            channels.push((name, v.scope.clone()));
        }
    }
    if channels.is_empty() {
        return Err(ImportError::new("The VCD file declares no signals that can be shown."));
    }
    // A name used in more than one scope gets its scope path.
    let names: Vec<String> = channels
        .iter()
        .map(|(n, scope)| {
            if channels.iter().filter(|(m, _)| m == n).count() > 1 {
                let mut path = scope.clone();
                path.push(n.clone());
                path.join(".")
            } else {
                n.clone()
            }
        })
        .collect();

    let t0 = h.t0.unwrap_or(0);
    let step = h.step.max(1);
    let span = (h.last - t0) / step;
    let count = span + u64::from(h.changes_at_last || (h.any_change && span == 0));
    if count >= 1 << 53 {
        return Err(ImportError::new("This VCD file spans more samples than this tool can index (2^53)."));
    }
    let (factor, exp) = h.timescale;
    let rate = 10f64.powi(-exp) / (factor as f64 * step as f64);
    if h.unknown > 0 {
        warn(
            &mut notes,
            format!("{} value(s) were x or z (unknown or high impedance); they are shown as low.", h.unknown),
        );
    }

    // Second pass: the values.
    let mut store = target.create(&vec![Level::Low; channels.len()])?;
    let mut level = vec![Level::Low; channels.len()];
    let mut emitted = vec![Level::Low; channels.len()];
    let mut dirty: Vec<usize> = Vec::new();
    let mut pending: Vec<Vec<Sample>> = vec![Vec::new(); channels.len()];
    let mut unknown_ids = 0u64;
    let mut t = Tokens::new(file)?;
    // skip the definitions
    while t.next()? && t.tok != b"$enddefinitions" {}
    t.until_end("$enddefinitions")?;
    let mut sample: Sample = 0;
    let flush_time = |sample: Sample,
                      dirty: &mut Vec<usize>,
                      level: &[Level],
                      emitted: &mut [Level],
                      pending: &mut [Vec<Sample>],
                      store: &mut CaptureStore|
     -> Result<(), ImportError> {
        for &slot in dirty.iter() {
            if level[slot] != emitted[slot] {
                emitted[slot] = level[slot];
                pending[slot].push(sample);
                if pending[slot].len() >= 1 << 16 {
                    store.push_edges(slot, &pending[slot])?;
                    pending[slot].clear();
                }
            }
        }
        dirty.clear();
        Ok(())
    };
    while t.next()? {
        match t.tok.first() {
            Some(b'#') => {
                let v: u64 = std::str::from_utf8(&t.tok[1..]).ok().and_then(|s| s.parse().ok()).unwrap_or(t0);
                flush_time(sample, &mut dirty, &level, &mut emitted, &mut pending, &mut store)?;
                sample = (v - t0) / step;
            }
            Some(b'$') => {
                if t.tok == b"$comment" {
                    t.until_end("$comment")?;
                }
            }
            _ => {
                let tok = t.tok.clone();
                let change = classify(&tok)?;
                let id = match change {
                    Change::Scalar(_) => String::from_utf8_lossy(&tok[1..]).into_owned(),
                    _ => {
                        t.next()?;
                        t.text()
                    }
                };
                let Some(targets) = by_id.get(&id) else {
                    if !matches!(change, Change::Real) {
                        unknown_ids += 1;
                    }
                    continue;
                };
                for tg in targets {
                    let c = match change {
                        Change::Scalar(c) => c,
                        Change::Vector(bits) => {
                            // VCD extends a short vector on the left: with 0 after a 1, else with its first bit
                            let n = u32::try_from(bits.len()).unwrap_or(u32::MAX);
                            if tg.bit < n {
                                bits[(n - 1 - tg.bit) as usize]
                            } else {
                                match bits.first() {
                                    Some(b'x' | b'X') => b'x',
                                    Some(b'z' | b'Z') => b'z',
                                    _ => b'0',
                                }
                            }
                        }
                        Change::Real => continue,
                    };
                    level[tg.slot] = Level::from(c == b'1');
                    dirty.push(tg.slot);
                }
            }
        }
        progress(0.5 + t.progress() * 0.5, "Reading the VCD values");
    }
    flush_time(sample, &mut dirty, &level, &mut emitted, &mut pending, &mut store)?;
    for (slot, p) in pending.iter().enumerate() {
        let inside: Vec<Sample> = p.iter().copied().filter(|&s| s < count).collect();
        store.push_edges(slot, &inside)?;
    }
    store.finish(count)?;
    if unknown_ids > 0 {
        warn(
            &mut notes,
            format!(
                "{unknown_ids} value change(s) name identifiers that were never declared; they were ignored."
            ),
        );
    }

    let meta = CaptureMeta {
        id: title.to_string(),
        title: title.to_string(),
        source: Source { kind: SourceKind::File, detail: "value change dump (VCD)".into() },
        sample_rate_hz: rate,
        trigger: None,
        channels: names
            .into_iter()
            .enumerate()
            .map(|(i, name)| ChannelInfo {
                index: u32::try_from(i).unwrap_or(u32::MAX),
                name,
                colour: Some(COLOURS[i % 4].to_string()),
                enabled: true,
                threshold_volts: None,
            })
            .collect(),
        segments: crate::capture::whole(count),
        notes,
        state: CaptureState::Complete,
        artefact: None,
        saved_decoders: Vec::new(),
    };
    Ok(Imported { store, meta })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timescales_and_ranges() {
        let ts = |s: &str| parse_timescale(&s.split(' ').map(String::from).collect::<Vec<_>>());
        assert_eq!(ts("10 ns"), Some((10, -9)));
        assert_eq!(ts("1us"), Some((1, -6)));
        assert_eq!(ts("100 fs"), Some((100, -15)));
        assert_eq!(ts("3 ns"), None);
        assert_eq!(ts("1 parsec"), None);
        assert_eq!(parse_range("[3:0]"), Some((3, 0)));
        assert_eq!(parse_range("[0:7]"), Some((0, 7)));
        assert_eq!(parse_range("[5]"), Some((5, 5)));
        assert_eq!(gcd(0, 20), 20);
        assert_eq!(gcd(20, 30), 10);
    }
}
