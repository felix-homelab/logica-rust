//! FNIRSI DLA_Logic `.demo` captures (doc 09). Format facts in PROVENANCE.md.
//!
//! ```text
//! channel.ini   SessionName, SamplingFrequency (kHz), SamplingDepth, TriggerSamplingDepth
//! set.ini       key=<json> lines: channelsSet, settingData.setHz, channelsDataColor, decodeJson
//! N/channel.ini name, "0", sample count, then 320 "mask,value" lines (unsigned 64-bit)
//! N/0-K.bin     block K: 8,388,608 samples, one bit each, least significant bit first
//! ```
//!
//! The masks form one bitmap of 20,480 blocks: block K is bit K % 64 of mask K / 64; a clear bit
//! means the block file exists. A set bit inside the sample range is read as "no file, constant
//! level from the same bit of `value`"; no sample exercises that, so it gets a warning.

use serde_json::Value;

use super::zip::ZipArchive;
use super::{ImportError, Imported, Progress, StoreTarget, info, ini_get, parse_ini, warn};
use crate::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, SavedDecoder, Source, SourceKind};
use crate::io::RandomAccess;
use crate::store::{CaptureStore, Level, Sample};

/// Samples per block file.
pub const BLOCK_SAMPLES: u64 = 8_388_608;
const BLOCK_BYTES: usize = 1 << 20;
/// DLA_Logic's four trace colours, the default channel palette (doc 08).
const DEFAULT_COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];
const WHAT: &str = "a DLA_Logic .demo file";

struct ChannelIni {
    name: String,
    sample_count: u64,
    masks: Vec<u64>,
    values: Vec<u64>,
}

fn parse_channel_ini(text: &str) -> Option<ChannelIni> {
    let lines: Vec<&str> = text.lines().collect();
    let name = lines.first().map_or("", |l| l.trim()).to_string();
    let sample_count = lines.get(2)?.trim().parse::<u64>().ok()?;
    let (mut masks, mut values) = (Vec::new(), Vec::new());
    for line in lines.iter().skip(3) {
        let Some((m, v)) = line.trim().split_once(',') else {
            continue;
        };
        if let (Ok(m), Ok(v)) = (m.parse::<u64>(), v.parse::<u64>()) {
            masks.push(m);
            values.push(v);
        }
    }
    Some(ChannelIni {
        name,
        sample_count,
        masks,
        values,
    })
}

/// Bit `k` of the block bitmap; blocks past the listed words count as "no file".
fn block_bit(words: &[u64], k: u64) -> bool {
    usize::try_from(k / 64)
        .ok()
        .and_then(|i| words.get(i))
        .is_none_or(|w| (w >> (k % 64)) & 1 == 1)
}

fn json_of(set: &[(String, String)], key: &str) -> Option<Value> {
    serde_json::from_str(ini_get(set, key)?).ok()
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn saved_decoders(raw: Option<Value>, notes: &mut Vec<Note>) -> Vec<SavedDecoder> {
    let Some(Value::Array(items)) = raw else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &items {
        let Some(Value::Object(dj)) = item.get("decodeJson") else {
            continue;
        };
        for (key, val) in dj {
            if key == "main" || !val.is_object() {
                continue;
            }
            let id = val
                .get("main")
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(key);
            let mut channels = Vec::new();
            for list in ["channels", "opt_channels"] {
                for c in val.get(list).and_then(Value::as_array).into_iter().flatten() {
                    let Some(cid) = c.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let n = c.get("value").map(value_text).and_then(|t| t.parse::<u32>().ok());
                    channels.push((cid.to_string(), n));
                }
            }
            let mut options = Vec::new();
            for o in val.get("options").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(oid), Some(v)) = (o.get("id").and_then(Value::as_str), o.get("value")) {
                    options.push((oid.to_string(), value_text(v)));
                }
            }
            out.push(SavedDecoder {
                protocol: id.to_lowercase(),
                channels,
                options,
            });
        }
    }
    if !items.is_empty() && out.is_empty() {
        info(
            notes,
            "The file has saved decoder settings in a layout this tool does not recognise.",
        );
    }
    out
}

/// Imports a `.demo` file.
pub fn import(
    file: &dyn RandomAccess,
    title: &str,
    target: &StoreTarget,
    progress: Progress<'_>,
) -> Result<Imported, ImportError> {
    let zip = ZipArchive::open(file, WHAT)?;
    let mut notes = Vec::new();
    let root_text = zip.read_text("channel.ini")?.ok_or_else(|| {
        ImportError::new("This ZIP has no channel.ini at the top level, so it is not a DLA_Logic capture.")
    })?;
    let root = parse_ini(&root_text);
    let set = match zip.read_text("set.ini")? {
        Some(t) => parse_ini(&t),
        None => {
            warn(
                &mut notes,
                "set.ini is missing; the channel list and sample rate come from channel.ini only.",
            );
            Vec::new()
        }
    };

    let set_hz = json_of(&set, "settingData")
        .and_then(|v| v.get("setHz").and_then(Value::as_f64))
        .filter(|&v| v > 0.0);
    let root_hz = ini_get(&root, "SamplingFrequency")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&k| k > 0.0)
        .map(|k| k * 1000.0);
    let rate = set_hz
        .or(root_hz)
        .ok_or_else(|| ImportError::new("The capture does not say what its sample rate is."))?;
    if let (Some(s), Some(r)) = (set_hz, root_hz)
        && (s - r).abs() > 1.0
    {
        warn(
            &mut notes,
            format!("set.ini says {s} Hz but channel.ini says {r} Hz. Using {s} Hz."),
        );
    }
    let depth = ini_get(&root, "SamplingDepth").and_then(|v| v.parse::<u64>().ok());

    let enabled: Vec<(u64, bool)> = json_of(&set, "channelsSet")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| {
            Some((
                c.get("id")?.as_u64()?,
                c.get("enable").and_then(Value::as_bool) == Some(true),
            ))
        })
        .collect();
    let flag = |i: u64| enabled.iter().find(|(id, _)| *id == i).map(|(_, e)| *e);
    let colours = json_of(&set, "channelsDataColor")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let colour_of = |i: usize| -> String {
        colours
            .get(i)
            .and_then(Value::as_str)
            .filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit()))
            .map_or_else(|| DEFAULT_COLOURS[i % 4].to_string(), str::to_string)
    };

    // Which channels have data?
    let mut chans: Vec<(u32, ChannelIni)> = Vec::new();
    for i in 0u32..32 {
        let Some(text) = zip.read_text(&format!("{i}/channel.ini"))? else {
            continue;
        };
        let Some(ini) = parse_channel_ini(&text) else {
            warn(
                &mut notes,
                format!("Channel {i} has an unreadable channel.ini and was skipped."),
            );
            continue;
        };
        let flagged = flag(u64::from(i));
        if ini.sample_count > 0 {
            if flagged == Some(false) {
                info(
                    &mut notes,
                    format!("Channel {i} has data but is marked as disabled in set.ini. It is shown anyway."),
                );
            }
            chans.push((i, ini));
        } else if flagged == Some(true) {
            warn(
                &mut notes,
                format!("Channel {i} is marked as enabled but has no samples."),
            );
        }
    }
    if chans.is_empty() {
        return Err(ImportError::new("No channel in this capture has any samples."));
    }
    let sample_count = chans.iter().map(|(_, c)| c.sample_count).max().unwrap_or(0);
    if chans.iter().any(|(_, c)| c.sample_count != sample_count) {
        warn(
            &mut notes,
            "The channels have different sample counts. Shorter channels hold their last level to the end.",
        );
    }
    if let Some(d) = depth.filter(|&d| d > 0 && sample_count != d + 8 && sample_count != d) {
        info(
            &mut notes,
            format!("The channels hold {sample_count} samples; the capture depth is {d}."),
        );
    }

    let mut store = target.create(&vec![Level::Low; chans.len()])?;
    let total_blocks: u64 = chans
        .iter()
        .map(|(_, c)| c.sample_count.div_ceil(BLOCK_SAMPLES))
        .sum();
    let mut done_blocks = 0u64;
    let mut words = vec![0u64; BLOCK_BYTES / 8];
    for (slot, (index, ini)) in chans.iter().enumerate() {
        let blocks = ini.sample_count.div_ceil(BLOCK_SAMPLES);
        let mut level: Option<Level> = None;
        for k in 0..blocks {
            let base = k * BLOCK_SAMPLES;
            let n = BLOCK_SAMPLES.min(ini.sample_count - base);
            let name = format!("{index}/0-{k}.bin");
            let no_file = block_bit(&ini.masks, k);
            let data = if no_file {
                None
            } else {
                zip.read(&name, BLOCK_BYTES as u64)?
            };
            match data {
                Some(bytes) => {
                    let have = bytes.len() as u64 * 8;
                    if bytes.len() != BLOCK_BYTES && base + have < ini.sample_count {
                        warn(
                            &mut notes,
                            format!(
                                "Block {k} of channel {index} is {} bytes instead of {BLOCK_BYTES}. The rest of that block is treated as unchanged.",
                                bytes.len()
                            ),
                        );
                    }
                    let usable = n.min(have);
                    words.iter_mut().for_each(|w| *w = 0);
                    let (whole, rest) = bytes.as_chunks::<8>();
                    for (w, c) in words.iter_mut().zip(whole) {
                        *w = u64::from_le_bytes(*c);
                    }
                    if !rest.is_empty() {
                        let mut last = [0u8; 8];
                        last[..rest.len()].copy_from_slice(rest);
                        words[whole.len()] = u64::from_le_bytes(last);
                    }
                    if usable > 0 {
                        store.push_bits(slot, base, &words, usable)?;
                        let b = usize::try_from(usable - 1).unwrap_or(0);
                        level = Some(Level::from_bit(words[b / 64] >> (b % 64)));
                    }
                    store.extend(slot, base + n)?;
                }
                None => {
                    let constant = if no_file {
                        let c = Level::from(block_bit(&ini.values, k));
                        warn(
                            &mut notes,
                            format!(
                                "Channel {index} has no data file for block {k} (samples {base} onward). Assumed constant {}, as the file's value bit suggests. This reading of the format is unverified.",
                                if c.is_high() { "high" } else { "low" }
                            ),
                        );
                        c
                    } else {
                        warn(
                            &mut notes,
                            format!(
                                "Channel {index} should have {name} but it is missing. That stretch is shown as unchanged."
                            ),
                        );
                        level.unwrap_or(Level::Low)
                    };
                    if level.unwrap_or(Level::Low) != constant {
                        store.push_edges(slot, &[base])?;
                    }
                    level = Some(constant);
                    store.extend(slot, base + n)?;
                }
            }
            done_blocks += 1;
            progress(
                done_blocks as f64 / total_blocks as f64,
                &format!("Reading channel {index}, block {} of {blocks}", k + 1),
            );
        }
        if zip.names().any(|n| {
            n.starts_with(&format!("{index}/"))
                && n.ends_with(".bin")
                && !n.starts_with(&format!("{index}/0-"))
        }) {
            info(
                &mut notes,
                format!("Channel {index} has data files that do not start with \"0-\". They were ignored."),
            );
        }
    }
    store.finish(sample_count)?;

    let saved = saved_decoders(json_of(&set, "decodeJson"), &mut notes);
    let artefact = detect_start_artefact(&store)?;
    if let Some(a) = &artefact {
        info(
            &mut notes,
            format!(
                "The first {} samples show a regular 8-sample pattern that looks like a recording artefact, not signal. They are shaded and left out of the measurements.",
                a.end
            ),
        );
    }
    let session = ini_get(&root, "SessionName").unwrap_or("").to_string();
    let meta = CaptureMeta {
        id: title.to_string(),
        title: title.to_string(),
        source: Source {
            kind: SourceKind::File,
            detail: if session.is_empty() {
                "FNIRSI DLA_Logic .demo file".into()
            } else {
                format!("FNIRSI DLA_Logic .demo file, session \"{session}\"")
            },
        },
        sample_rate_hz: rate,
        trigger: None,
        channels: chans
            .iter()
            .map(|(i, ini)| ChannelInfo {
                index: *i,
                name: if ini.name.is_empty() {
                    format!("channel {i}")
                } else {
                    ini.name.clone()
                },
                colour: Some(colour_of(*i as usize)),
                enabled: true,
                threshold_volts: None,
            })
            .collect(),
        segments: crate::capture::whole(sample_count),
        notes,
        state: CaptureState::Complete,
        artefact,
        saved_decoders: saved,
    };
    Ok(Imported { store, meta })
}

/// Some real DLA captures start with a short burst of junk: every edge on a multiple of 8 samples,
/// pulses 8 to 64 samples wide, on several channels at once (the first 2,048 samples of `I2C.demo`).
/// The rule is narrow so real signal is never hidden: the stretch starts within the first 8
/// samples, has at least 16 edges, ends before sample 8,192, and is followed by a quiet gap at
/// least 4 times its longest pulse. Returns the samples to leave out of analysis.
pub fn detect_start_artefact(store: &CaptureStore) -> Result<Option<std::ops::Range<Sample>>, ImportError> {
    let mut end = 0u64;
    let mut hits = 0;
    for slot in 0..store.slots() {
        let mut it = store.edges(slot, 0, store.sample_count())?;
        let mut e: Vec<Sample> = Vec::new();
        // the stretch, plus the first edge after it
        while let Some(x) = it.next().transpose()? {
            e.push(x);
            let n = e.len();
            if n >= 2 {
                let (a, b) = (e[n - 2], e[n - 1]);
                if a % 8 != 0 || b % 8 != 0 || b - a > 64 || b > 8192 {
                    break;
                }
            }
        }
        if e.len() < 16 || e[0] > 8 {
            continue;
        }
        let mut k = 0usize;
        let mut max_gap = 0u64;
        while k + 1 < e.len() {
            let (a, b) = (e[k], e[k + 1]);
            if a % 8 != 0 || b % 8 != 0 || b - a > 64 || b > 8192 {
                break;
            }
            max_gap = max_gap.max(b - a);
            k += 1;
        }
        if k + 1 < 16 {
            continue;
        }
        let last = e[k];
        if let Some(&next) = e.get(k + 1)
            && next - last < 4 * max_gap
        {
            continue;
        }
        end = end.max(last);
        hits += 1;
    }
    Ok((hits > 0).then_some(0..end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::zip::tests::make_zip;
    use crate::io::MemFile;

    fn channel_ini(name: &str, count: u64, mask0: u64, value0: u64, rest: u64) -> String {
        let mut lines = vec![
            name.to_string(),
            "0".into(),
            count.to_string(),
            format!("{mask0},{value0}"),
        ];
        for _ in 1..320 {
            lines.push(if count > 0 {
                format!("{rest},0")
            } else {
                "0,0".into()
            });
        }
        lines.join("\r\n") + "\r\n"
    }
    const ROOT: &str =
        "SessionName=Test\r\nSamplingFrequency=1000\r\nSamplingDepth=100\r\nTriggerSamplingDepth=1\r\n";
    fn set(enabled: &[u32]) -> String {
        let cs: Vec<String> = (0..32)
            .map(|i| {
                format!(
                    "{{\"enable\":{},\"id\":{i},\"triggerType\":0}}",
                    enabled.contains(&i)
                )
            })
            .collect();
        format!(
            "\n[DLA32_Plus]\nchannelsSet=[{}]\nsettingData={{\"setHz\":1000000}}\ndecodeJson=[]\n",
            cs.join(",")
        )
    }
    fn run(entries: &[(&str, &[u8])]) -> Result<Imported, ImportError> {
        let z = MemFile::from_bytes(make_zip(entries, true));
        import(&z, "t", &StoreTarget::memory(), &mut |_, _| {})
    }

    #[test]
    fn bits_are_lsb_first_and_padding_is_ignored() {
        let mut block = vec![0u8; BLOCK_BYTES];
        block[0] = 0b0000_1110; // samples 1 to 3 high
        block[20] = 0xff; // past sample 108: padding
        let ci = channel_ini("A", 108, u64::MAX - 1, 0, u64::MAX);
        let s = set(&[0]);
        let r = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("set.ini", s.as_bytes()),
            ("0/channel.ini", ci.as_bytes()),
            ("0/0-0.bin", &block),
        ])
        .unwrap();
        assert_eq!(r.store.sample_count(), 108);
        assert_eq!(r.store.edges_in(0, 0, 108).unwrap(), vec![1, 4]);
        assert_eq!(r.store.initial_level(0).unwrap(), Level::Low);
        assert_eq!(r.meta.sample_rate_hz, 1e6);
        assert_eq!(r.meta.channels[0].name, "A");
        assert_eq!(r.meta.channels[0].colour.as_deref(), Some("#e54242"));
    }

    #[test]
    fn a_block_without_a_file_is_constant_and_says_so() {
        let count = BLOCK_SAMPLES + 100;
        let block = vec![0u8; BLOCK_BYTES];
        // mask bit 1 set = no file for block 1; value bit 1 set = high
        let ci = channel_ini("A", count, u64::MAX - 1, 2, u64::MAX);
        let s = set(&[0]);
        let r = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("set.ini", s.as_bytes()),
            ("0/channel.ini", ci.as_bytes()),
            ("0/0-0.bin", &block),
        ])
        .unwrap();
        assert_eq!(r.store.edges_in(0, 0, count).unwrap(), vec![BLOCK_SAMPLES]);
        assert!(
            r.meta
                .notes
                .iter()
                .any(|n| n.level == crate::capture::NoteLevel::Warning && n.message.contains("unverified"))
        );
    }

    #[test]
    fn a_missing_block_file_is_survived() {
        let ci = channel_ini("A", 100, u64::MAX - 1, 0, u64::MAX);
        let s = set(&[0]);
        let r = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("set.ini", s.as_bytes()),
            ("0/channel.ini", ci.as_bytes()),
        ])
        .unwrap();
        assert!(r.store.edges_in(0, 0, 100).unwrap().is_empty());
        assert!(r.meta.notes.iter().any(|n| n.message.contains("missing")));
    }

    #[test]
    fn things_that_are_not_captures_get_clear_errors() {
        let z = MemFile::from_bytes(b"hello, not a zip at all, just some text".to_vec());
        let e = import(&z, "t", &StoreTarget::memory(), &mut |_, _| {}).unwrap_err();
        assert_eq!(
            e.to_string(),
            "This is not a ZIP archive, so it cannot be a DLA_Logic .demo file."
        );
        let z = MemFile::from_bytes(vec![0; 3]);
        assert!(
            import(&z, "t", &StoreTarget::memory(), &mut |_, _| {})
                .unwrap_err()
                .to_string()
                .contains("too small")
        );
        assert!(
            run(&[("readme.txt", b"x")])
                .unwrap_err()
                .to_string()
                .contains("no channel.ini")
        );
        assert!(
            run(&[("channel.ini", b"SessionName=x\r\n")])
                .unwrap_err()
                .to_string()
                .contains("sample rate")
        );
        let ci = channel_ini("channel 0", 0, 0, 0, 0);
        let s = set(&[]);
        let e = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("set.ini", s.as_bytes()),
            ("0/channel.ini", ci.as_bytes()),
        ])
        .unwrap_err();
        assert_eq!(e.to_string(), "No channel in this capture has any samples.");
        let e = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("0/channel.ini", b"A\r\n0\r\nabc\r\n"),
        ])
        .unwrap_err();
        assert_eq!(e.to_string(), "No channel in this capture has any samples.");
    }

    #[test]
    fn without_set_ini_channel_ini_is_used() {
        let block = vec![0u8; BLOCK_BYTES];
        let ci = channel_ini("A", 100, u64::MAX - 1, 0, u64::MAX);
        let r = run(&[
            ("channel.ini", ROOT.as_bytes()),
            ("0/channel.ini", ci.as_bytes()),
            ("0/0-0.bin", &block),
        ])
        .unwrap();
        assert_eq!(r.meta.sample_rate_hz, 1e6);
        assert!(
            r.meta
                .notes
                .iter()
                .any(|n| n.message.contains("set.ini is missing"))
        );
    }

    #[test]
    fn the_artefact_rule_ignores_ordinary_signals() {
        let mut s = CaptureStore::new(&[Level::High], std::sync::Arc::new(MemFile::new()), 16, 1 << 20);
        s.push_edges(0, &(0..40).map(|i| 1000 + i * 8).collect::<Vec<_>>())
            .unwrap();
        s.finish(100_000).unwrap();
        assert_eq!(detect_start_artefact(&s).unwrap(), None);
        // a regular signal that simply carries on past the start
        let mut s = CaptureStore::new(&[Level::High], std::sync::Arc::new(MemFile::new()), 16, 1 << 20);
        s.push_edges(0, &(0..4000).map(|i| 8 + i * 8).collect::<Vec<_>>())
            .unwrap();
        s.finish(100_000).unwrap();
        assert_eq!(detect_start_artefact(&s).unwrap(), None);
        // a burst of 20 edges every 8 samples, then quiet
        let mut s = CaptureStore::new(&[Level::High], std::sync::Arc::new(MemFile::new()), 16, 1 << 20);
        s.push_edges(0, &(0..20).map(|i| 8 + i * 8).chain([5000]).collect::<Vec<_>>())
            .unwrap();
        s.finish(100_000).unwrap();
        assert_eq!(detect_start_artefact(&s).unwrap(), Some(0..161));
    }
}
