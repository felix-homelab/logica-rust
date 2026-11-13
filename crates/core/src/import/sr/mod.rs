//! sigrok session files `.sr`, version 2 (doc 09). Format facts in PROVENANCE.md.
//!
//! A ZIP with `version` ("2"), `metadata` (INI: `[device 1]` with `capturefile`, `total probes`,
//! `samplerate` as text with a unit, `unitsize`, `probeN` = name of the channel on bit N-1,
//! `total analog`) and the data in `<capturefile>-1`, `-2`, … (or a single `<capturefile>`):
//! packed little-endian samples, `unitsize` bytes each, read through the packed ingest path.

use super::zip::ZipArchive;
use super::{ImportError, Imported, Progress, StoreTarget, info, warn};
use crate::capture::{CaptureMeta, CaptureState, ChannelInfo, Note, Source, SourceKind};
use crate::io::RandomAccess;
use crate::store::Level;

const WHAT: &str = "a sigrok .sr session";
const COLOURS: [&str; 4] = ["#e54242", "#ff7f00", "#007fff", "#26ad34"];

/// Parses a sigrok sample rate such as "1 MHz", "2.5 MHz", "10kHz" or "500 Hz" into hertz.
pub fn parse_rate(text: &str) -> Option<f64> {
    let t = text.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let value: f64 = t[..split].parse().ok()?;
    let unit = t[split..].trim().to_ascii_lowercase();
    let scale = match unit.as_str() {
        "" | "hz" => 1.0,
        "khz" => 1e3,
        "mhz" => 1e6,
        "ghz" => 1e9,
        _ => return None,
    };
    let hz = value * scale;
    (hz > 0.0 && hz.is_finite()).then_some(hz)
}

/// `[section]` → key/value lines.
fn sections(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = vec![(String::new(), Vec::new())];
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.trim().to_string(), Vec::new()));
        } else if let Some((k, v)) = line.split_once('=')
            && let Some(last) = out.last_mut()
        {
            last.1.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

pub fn import(
    file: &dyn RandomAccess,
    title: &str,
    target: &StoreTarget,
    progress: Progress<'_>,
) -> Result<Imported, ImportError> {
    let zip = ZipArchive::open(file, WHAT)?;
    let mut notes: Vec<Note> = Vec::new();
    let version = zip.read_text("version")?.ok_or_else(|| {
        ImportError::new("This ZIP has no \"version\" entry, so it is not a sigrok session (.sr) file.")
    })?;
    if version.trim() != "2" {
        return Err(ImportError::new(format!(
            "This .sr file has format version {}; this tool reads version 2 (sigrok 0.4 and later).",
            version.trim()
        )));
    }
    let metadata = zip.read_text("metadata")?.ok_or_else(|| {
        ImportError::new("This .sr file has no \"metadata\" entry, so its channels are unknown.")
    })?;
    let secs = sections(&metadata);
    let devices: Vec<&(String, Vec<(String, String)>)> =
        secs.iter().filter(|(n, _)| n.starts_with("device ")).collect();
    let Some((_, dev)) = devices.first() else {
        return Err(ImportError::new(
            "The .sr metadata describes no device, so there is nothing to show.",
        ));
    };
    if devices.len() > 1 {
        warn(
            &mut notes,
            format!(
                "The file holds {} devices; only the first is shown.",
                devices.len()
            ),
        );
    }
    let get = |k: &str| dev.iter().rev().find(|(x, _)| x == k).map(|(_, v)| v.as_str());

    let rate_text =
        get("samplerate").ok_or_else(|| ImportError::new("The .sr file does not say its sample rate."))?;
    let rate = parse_rate(rate_text).ok_or_else(|| {
        ImportError::new(format!(
            "The sample rate \"{rate_text}\" in the .sr file cannot be read."
        ))
    })?;
    let unit_size: usize = match get("unitsize").map(str::parse::<usize>) {
        Some(Ok(u)) if (1..=4).contains(&u) => u,
        Some(Ok(u)) => {
            return Err(ImportError::new(format!(
                "The .sr file stores {u} bytes per sample; this tool reads 1 to 4 (up to 32 channels)."
            )));
        }
        _ => {
            return Err(ImportError::new(
                "The .sr file does not say how many bytes each sample has (unitsize).",
            ));
        }
    };
    let total: u32 = get("total probes").and_then(|v| v.parse().ok()).unwrap_or(0);
    if total > 32 {
        return Err(ImportError::new(format!(
            "This capture has {total} channels; this tool shows at most 32 channels per capture."
        )));
    }
    if let Some(a) = get("total analog")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&a| a > 0)
    {
        info(
            &mut notes,
            format!("The file has {a} analog channel(s); they are not shown (digital channels only)."),
        );
    }
    // probeN = name of the channel on bit N-1
    let mut probes: Vec<(u32, String)> = dev
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.strip_prefix("probe")?.parse::<u32>().ok()?.checked_sub(1)?,
                v.clone(),
            ))
        })
        .collect();
    probes.sort_by_key(|(b, _)| *b);
    probes.dedup_by_key(|(b, _)| *b);
    let unit_bits = u32::try_from(unit_size * 8).unwrap_or(32);
    if let Some((b, _)) = probes.iter().find(|(b, _)| *b >= unit_bits) {
        return Err(ImportError::new(format!(
            "Channel {} is on bit {b}, which samples of {unit_size} byte(s) do not have.",
            b + 1
        )));
    }
    if probes.is_empty() {
        return Err(ImportError::new("The .sr file lists no logic channels."));
    }

    let capture_file = get("capturefile").unwrap_or("logic-1").to_string();
    let mut parts: Vec<(u64, String)> = zip
        .names()
        .filter_map(|n| {
            let rest = n.strip_prefix(&capture_file)?;
            if rest.is_empty() {
                return Some((0, n.to_string()));
            }
            Some((rest.strip_prefix('-')?.parse::<u64>().ok()?, n.to_string()))
        })
        .collect();
    parts.sort();
    if parts.is_empty() {
        return Err(ImportError::new(format!(
            "The .sr file has no sample data (no \"{capture_file}-1\" entry)."
        )));
    }
    let total_bytes: u64 = parts
        .iter()
        .filter_map(|(_, n)| zip.entry(n))
        .map(|e| e.size)
        .sum();

    let bits: Vec<u32> = probes.iter().map(|(b, _)| *b).collect();
    let mut store = target.create(&vec![Level::Low; bits.len()])?;
    let mut at = 0u64; // samples written
    let mut carry: Vec<u8> = Vec::new();
    let mut read = 0u64;
    for (_, name) in &parts {
        let mut sink = |b: &[u8]| -> Result<(), ImportError> {
            read += b.len() as u64;
            let mut data = b;
            if !carry.is_empty() {
                let need = unit_size - carry.len();
                let take = need.min(data.len());
                carry.extend_from_slice(&data[..take]);
                data = &data[take..];
                if carry.len() == unit_size {
                    store.push_packed(at, unit_size, &carry, &bits)?;
                    at += 1;
                    carry.clear();
                }
            }
            let whole = data.len() / unit_size * unit_size;
            if whole > 0 {
                store.push_packed(at, unit_size, &data[..whole], &bits)?;
                at += (whole / unit_size) as u64;
            }
            carry.extend_from_slice(&data[whole..]);
            progress(read as f64 / total_bytes.max(1) as f64, "Reading samples");
            Ok(())
        };
        zip.stream(name, &mut sink)?;
    }
    if !carry.is_empty() {
        warn(
            &mut notes,
            format!(
                "The data ends in the middle of a sample ({} stray byte(s)); they were ignored.",
                carry.len()
            ),
        );
    }
    store.finish(at)?;
    if at == 0 {
        warn(&mut notes, "The file has no samples.");
    }

    let meta = CaptureMeta {
        id: title.to_string(),
        title: title.to_string(),
        source: Source {
            kind: SourceKind::File,
            detail: "sigrok session file (.sr)".into(),
        },
        sample_rate_hz: rate,
        trigger: None,
        channels: probes
            .iter()
            .map(|(b, name)| ChannelInfo {
                index: *b,
                name: name.clone(),
                colour: Some(COLOURS[*b as usize % 4].to_string()),
                enabled: true,
                threshold_volts: None,
            })
            .collect(),
        segments: crate::capture::whole(at),
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
    fn sample_rates_with_units() {
        assert_eq!(parse_rate("1 MHz"), Some(1e6));
        assert_eq!(parse_rate("2.5 MHz"), Some(2.5e6));
        assert_eq!(parse_rate("10kHz"), Some(1e4));
        assert_eq!(parse_rate("500 Hz"), Some(500.0));
        assert_eq!(parse_rate("1 GHz"), Some(1e9));
        assert_eq!(parse_rate("fast"), None);
        assert_eq!(parse_rate("0 Hz"), None);
        assert_eq!(parse_rate("3 parsecs"), None);
    }
}
