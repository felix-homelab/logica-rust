//! Decoders on a capture: add, update, remove, and paged decoded output.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_protocols::analyse::{Batching, decode_store};
use logica_protocols::format::as_text;
use logica_protocols::protocol::Symptom;
use logica_protocols::types::{
    DataFormat, DecodeResult, Frame, FrameKind, Packet, ProblemCode, Roles, Settings,
};

use crate::contract::{CommandError, ErrorCode, Event};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::{Decoder, DecoderConfig, DecoderSummary};

use super::{Nothing, inv, out};

/// A decoder from its configuration and decode, its channels in the protocol's role order.
pub(crate) fn decoder(e: &Engine, cfg: DecoderConfig, result: DecodeResult) -> Decoder {
    let order: Vec<String> = e
        .registry
        .get(&cfg.protocol)
        .map(|p| p.roles().iter().map(|r| r.key.clone()).collect())
        .unwrap_or_default();
    let keys: Vec<&str> = order.iter().map(String::as_str).collect();
    Decoder::new(cfg, result, &keys)
}

/// Decodes a whole capture with a configuration.
pub(crate) fn run_decode(
    e: &Engine,
    capture: &str,
    cfg: &DecoderConfig,
) -> Result<DecodeResult, CommandError> {
    let c = e.capture(capture)?;
    decode_store(&e.registry, &c.store, &c.meta, &cfg.protocol, &cfg.roles, &cfg.settings, Batching::Whole)
        .map_err(|m| err(ErrorCode::InvalidInput, m))
}

fn channels_text(r: &Roles) -> String {
    let mut chs: Vec<u32> = r.values().flatten().copied().collect();
    chs.sort_unstable();
    chs.dedup();
    let list: Vec<String> = chs.iter().map(|c| format!("CH{c}")).collect();
    match list.len() {
        0 => String::new(),
        1 => format!(" on {}", list[0]),
        n => format!(" on {} and {}", list[..n - 1].join(", "), list[n - 1]),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderAddIn {
    pub capture: String,
    /// A protocol id from protocol.list, such as "i2c".
    pub protocol: String,
    /// Role to physical channel, such as {"scl": 5, "sda": 4}.
    pub roles: Roles,
    #[serde(default)]
    pub settings: Settings,
    pub name: Option<String>,
    /// How its data is written in the waveform's text row (default ASCII).
    pub data_format: Option<DataFormat>,
    /// The decoder id to use (redo and undo use the id it had); a new one when absent.
    pub id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DecoderOut {
    pub decoder: DecoderSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderRef {
    pub capture: String,
    pub decoder: String,
}

pub(crate) fn add(e: &mut Engine, _: &Ctx, i: DecoderAddIn) -> Result<Done, CommandError> {
    let p = e.registry.get(&i.protocol).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("There is no protocol \"{}\". protocol.list shows them all.", i.protocol),
        )
    })?;
    let title = p.title(&i.settings);
    let cfg = DecoderConfig {
        protocol: i.protocol.clone(),
        roles: i.roles.clone(),
        settings: i.settings.clone(),
        name: i.name.clone().unwrap_or_else(|| title.clone()),
        data_format: i.data_format.unwrap_or_default(),
    };
    let result = run_decode(e, &i.capture, &cfg)?;
    let id = match &i.id {
        Some(id) => {
            e.reserve_id(id);
            id.clone()
        }
        None => e.new_id(&format!("{}-", i.protocol)),
    };
    let d = decoder(e, cfg.clone(), result);
    let c = e.capture_mut(&i.capture)?;
    if c.decoders.contains_key(&id) {
        return Err(err(ErrorCode::InvalidInput, format!("A decoder with the id \"{id}\" already exists.")));
    }
    let summary = DecoderSummary::of(&id, &d);
    c.decoders.insert(id.clone(), d);
    e.emit(Event::DecoderUpdated { capture: i.capture.clone(), decoder: id.clone() });
    let canonical = DecoderAddIn {
        id: Some(id.clone()),
        name: Some(cfg.name.clone()),
        data_format: Some(cfg.data_format),
        ..i.clone()
    };
    Ok(Done::edit(
        out(&DecoderOut { decoder: summary }),
        inv("decoder.remove", &DecoderRef { capture: i.capture.clone(), decoder: id }),
        format!("added {} {title} decoder{}", article(&title), channels_text(&i.roles)),
    )
    .canonical(out(&canonical)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderUpdateIn {
    pub capture: String,
    pub decoder: String,
    /// New roles (all of them).
    pub roles: Option<Roles>,
    /// Settings to change; the others keep their values.
    pub settings: Option<Settings>,
    /// Replace all settings instead of merging (what an undo uses).
    #[serde(default)]
    pub replace_settings: bool,
    pub name: Option<String>,
    /// How its data is written in the waveform's text row; changing only this does not decode
    /// again.
    pub data_format: Option<DataFormat>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProblemChange {
    pub code: ProblemCode,
    pub before: u64,
    pub after: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecoderUpdateOut {
    pub decoder: DecoderSummary,
    /// How the problem counts changed.
    pub problem_changes: Vec<ProblemChange>,
}

fn packet_text(p: &Packet) -> String {
    match p.streams.as_slice() {
        [one] => as_text(&one.bytes),
        many => {
            many.iter().map(|s| format!("{}: {}", s.label, as_text(&s.bytes))).collect::<Vec<_>>().join(" | ")
        }
    }
}

fn find(e: &Engine, capture: &str, decoder: &str) -> Result<(), CommandError> {
    if e.capture(capture)?.decoders.contains_key(decoder) {
        Ok(())
    } else {
        Err(err(ErrorCode::NotFound, format!("There is no decoder \"{decoder}\" on this capture.")))
    }
}

pub(crate) fn update(e: &mut Engine, _: &Ctx, i: DecoderUpdateIn) -> Result<Done, CommandError> {
    find(e, &i.capture, &i.decoder)?;
    let old = e.capture(&i.capture)?.decoders[&i.decoder].config.clone();
    let before = e.capture(&i.capture)?.decoders[&i.decoder].result.problem_counts.clone();
    let mut cfg = old.clone();
    if let Some(r) = &i.roles {
        cfg.roles = r.clone();
    }
    if let Some(s) = &i.settings {
        if i.replace_settings {
            cfg.settings = s.clone();
        } else {
            cfg.settings.extend(s.clone());
        }
    }
    if let Some(n) = &i.name {
        cfg.name = n.clone();
    }
    if let Some(f) = i.data_format {
        cfg.data_format = f;
    }
    // the name and the data format do not change what is decoded
    let same = cfg.roles == old.roles && cfg.settings == old.settings;
    let result = if same {
        let c = e.capture_mut(&i.capture)?;
        let d = c
            .decoders
            .get_mut(&i.decoder)
            .ok_or_else(|| err(ErrorCode::NotFound, "There is no such decoder."))?;
        std::mem::take(&mut d.result)
    } else {
        run_decode(e, &i.capture, &cfg)?
    };
    let after = result.problem_counts.clone();
    let d = decoder(e, cfg, result);
    let c = e.capture_mut(&i.capture)?;
    let summary = DecoderSummary::of(&i.decoder, &d);
    c.decoders.insert(i.decoder.clone(), d);
    e.emit(Event::DecoderUpdated { capture: i.capture.clone(), decoder: i.decoder.clone() });
    let codes: std::collections::BTreeSet<ProblemCode> = before.keys().chain(after.keys()).copied().collect();
    let problem_changes = codes
        .into_iter()
        .map(|code| ProblemChange {
            code,
            before: before.get(&code).copied().unwrap_or(0),
            after: after.get(&code).copied().unwrap_or(0),
        })
        .filter(|p| p.before != p.after)
        .collect();
    let back = DecoderUpdateIn {
        capture: i.capture.clone(),
        decoder: i.decoder.clone(),
        roles: Some(old.roles),
        settings: Some(old.settings),
        replace_settings: true,
        name: Some(old.name.clone()),
        data_format: Some(old.data_format),
    };
    Ok(Done::edit(
        out(&DecoderUpdateOut { decoder: summary, problem_changes }),
        inv("decoder.update", &back),
        format!("changed the {} decoder", old.name),
    ))
}

pub(crate) fn remove(e: &mut Engine, _: &Ctx, i: DecoderRef) -> Result<Done, CommandError> {
    find(e, &i.capture, &i.decoder)?;
    let c = e.capture_mut(&i.capture)?;
    let d =
        c.decoders.remove(&i.decoder).ok_or_else(|| err(ErrorCode::NotFound, "There is no such decoder."))?;
    e.emit(Event::DecoderUpdated { capture: i.capture.clone(), decoder: i.decoder.clone() });
    let back = DecoderAddIn {
        capture: i.capture.clone(),
        protocol: d.config.protocol.clone(),
        roles: d.config.roles.clone(),
        settings: d.config.settings.clone(),
        name: Some(d.config.name.clone()),
        data_format: Some(d.config.data_format),
        id: Some(i.decoder.clone()),
    };
    Ok(Done::edit(
        out(&Nothing {}),
        inv("decoder.add", &back),
        format!("removed the {} decoder", d.config.name),
    ))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoderFramesIn {
    pub capture: String,
    pub decoder: String,
    /// Packets (a UART line, an I2C transaction) instead of single frames.
    #[serde(default)]
    pub packets: bool,
    pub from: Option<Sample>,
    pub to: Option<Sample>,
    /// Only frames of this kind.
    pub kind: Option<FrameKind>,
    /// Only frames (or packets with frames) that have problems.
    #[serde(default)]
    pub problems: bool,
    /// Only items whose label, detail, title or text contains this (any case).
    pub text: Option<String>,
    /// From the previous page's `nextCursor`.
    pub cursor: Option<usize>,
    /// Items per page (default 100, at most 1,000).
    pub limit: Option<usize>,
    /// Group the problems and explain likely causes.
    #[serde(default)]
    pub explain: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PacketItem {
    pub packet: Packet,
    /// Every stream as text, labelled: "Write to 0x57: ·· | Read from 0x57: CALIBRATED 2026".
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProblemGroup {
    pub code: ProblemCode,
    pub count: u64,
    /// Sample of the first one, to jump to.
    pub first: Option<Sample>,
    /// Different messages seen (at most three).
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub problems: Vec<ProblemGroup>,
    /// Symptoms, causes and actions for this protocol (doc 06, symptom tables).
    pub causes: Vec<Symptom>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecoderFramesOut {
    /// Items that match, over all pages.
    pub total: usize,
    pub frames: Vec<Frame>,
    pub packets: Vec<PacketItem>,
    pub next_cursor: Option<usize>,
    pub explanation: Option<Explanation>,
}

pub(crate) fn frames(e: &mut Engine, _: &Ctx, i: DecoderFramesIn) -> Result<Done, CommandError> {
    find(e, &i.capture, &i.decoder)?;
    let c = e.capture(&i.capture)?;
    let d = &c.decoders[&i.decoder];
    let r = &d.result;
    let from = i.from.unwrap_or(0);
    let to = i.to.unwrap_or(Sample::MAX);
    let text = i.text.as_ref().map(|t| t.to_lowercase());
    let has = |s: &str| text.as_ref().is_none_or(|t| s.to_lowercase().contains(t));
    let limit = i.limit.unwrap_or(100).clamp(1, 1000);
    let start = i.cursor.unwrap_or(0);
    let frame_ok = |f: &Frame| {
        f.end > from
            && f.start < to
            && i.kind.is_none_or(|k| f.kind == k)
            && (!i.problems || !f.problems.is_empty())
            && (has(&f.label) || f.detail.as_deref().is_some_and(has))
    };
    let mut o = DecoderFramesOut {
        total: 0,
        frames: Vec::new(),
        packets: Vec::new(),
        next_cursor: None,
        explanation: None,
    };
    if i.packets {
        let with_problems: std::collections::BTreeSet<u64> =
            r.frames.iter().filter(|f| !f.problems.is_empty()).map(|f| f.packet).collect();
        let matching: Vec<&Packet> = r
            .packets
            .iter()
            .filter(|p| {
                let t = packet_text(p);
                p.end > from
                    && p.start < to
                    && (!i.problems || with_problems.contains(&p.index))
                    && (has(&p.title) || has(&t))
            })
            .collect();
        o.total = matching.len();
        o.packets = matching
            .iter()
            .skip(start)
            .take(limit)
            .map(|p| PacketItem { packet: (*p).clone(), text: packet_text(p) })
            .collect();
    } else {
        let matching: Vec<&Frame> = r.frames.iter().filter(|f| frame_ok(f)).collect();
        o.total = matching.len();
        o.frames = matching.iter().skip(start).take(limit).map(|f| (*f).clone()).collect();
    }
    if start + limit < o.total {
        o.next_cursor = Some(start + limit);
    }
    if i.explain {
        let mut groups: BTreeMap<ProblemCode, ProblemGroup> = BTreeMap::new();
        for f in r.frames.iter().filter(|f| f.end > from && f.start < to) {
            for p in &f.problems {
                let g = groups.entry(p.code).or_insert(ProblemGroup {
                    code: p.code,
                    count: 0,
                    first: Some(f.start),
                    messages: vec![],
                });
                g.count += 1;
                if g.messages.len() < 3 && !g.messages.contains(&p.message) {
                    g.messages.push(p.message.clone());
                }
            }
        }
        let causes =
            e.registry.get(&d.config.protocol).map(|p| p.info().symptoms.clone()).unwrap_or_default();
        o.explanation = Some(Explanation { problems: groups.into_values().collect(), causes });
    }
    Ok(Done::query(out(&o)))
}

/// "a" or "an" before a protocol name as it is spoken: initialisms by their first letter
/// ("an I2C", "an SPI", "a UART"), words by their first letter ("a CAN", "an Infrared").
pub(crate) fn article(name: &str) -> &'static str {
    const WORDS: &[&str] = &["CAN", "JTAG", "DALI", "LIN", "MIDI", "DMX512"];
    let Some(c) = name.chars().next().map(|c| c.to_ascii_uppercase()) else { return "a" };
    let first = name.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    let initialism = first.len() <= 5 && first.chars().all(|c| !c.is_lowercase()) && !WORDS.contains(&first);
    let vowel_sound = if initialism {
        // "U" is spoken "you": a UART, a USB
        "AEFHILMNORSX".contains(c)
    } else {
        "AEIO".contains(c)
    };
    if vowel_sound { "an" } else { "a" }
}

#[cfg(test)]
mod tests {
    use super::article;

    #[test]
    fn articles_follow_how_names_are_spoken() {
        for (name, a) in [
            ("I2C", "an"),
            ("SPI", "an"),
            ("UART", "a"),
            ("PWM", "a"),
            ("CAN", "a"),
            ("JTAG", "a"),
            ("SWD", "an"),
            ("NEC", "an"),
            ("1-Wire", "a"),
            ("Infrared remote", "an"),
            ("Servo PWM", "a"),
            ("DMX512", "a"),
            ("", "a"),
        ] {
            assert_eq!(article(name), a, "{name}");
        }
    }
}
