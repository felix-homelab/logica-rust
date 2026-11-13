//! `render` (doc 07): a PNG of a range of a capture with its channels, decoder rows and markers,
//! drawn by the engine so it works without a window. Waveforms come from the store's LOD, so the
//! cost depends on the width in pixels, not on the length of the range.

use logica_core::Sample;
use logica_core::store::Level;
use logica_protocols::types::{Frame, FrameKind};
use logica_protocols::{format, num};

use crate::state::Capture;

type Rgb = [u8; 3];

const BG: Rgb = [0x16, 0x18, 0x1d];
const GRID: Rgb = [0x2a, 0x2e, 0x36];
const TEXT: Rgb = [0xc8, 0xcc, 0xd4];
const DIM: Rgb = [0x80, 0x86, 0x90];
const MARKER: Rgb = [0xff, 0xb0, 0x20];
const PROBLEM: Rgb = [0xff, 0x4d, 0x4d];
const DEFAULT_WAVE: Rgb = [0x3d, 0xdc, 0x84];

const GUTTER: u32 = 96;
const AXIS: u32 = 18;
const CHANNEL_ROW: u32 = 30;
const DECODER_ROW: u32 = 22;
const MARKER_ROW: u32 = 14;
const MAX_FRAMES: usize = 20_000;

/// A picture in memory, RGB.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    px: Vec<u8>,
}

impl Raster {
    pub fn new(width: u32, height: u32, bg: Rgb) -> Self {
        let n = width as usize * height as usize;
        let mut px = Vec::with_capacity(n * 3);
        for _ in 0..n {
            px.extend_from_slice(&bg);
        }
        Raster { width, height, px }
    }

    pub fn get(&self, x: u32, y: u32) -> Rgb {
        let i = (y as usize * self.width as usize + x as usize) * 3;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }

    fn set(&mut self, x: i64, y: i64, c: Rgb) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = (usize::try_from(y).unwrap_or(0) * self.width as usize + usize::try_from(x).unwrap_or(0)) * 3;
        self.px[i..i + 3].copy_from_slice(&c);
    }

    fn rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                self.set(x, y, c);
            }
        }
    }

    fn outline(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        self.rect(x0, y0, x1, y0 + 1, c);
        self.rect(x0, y1 - 1, x1, y1, c);
        self.rect(x0, y0, x0 + 1, y1, c);
        self.rect(x1 - 1, y0, x1, y1, c);
    }

    /// Text in the 5×7 font, one pixel between letters; returns the width drawn.
    fn text(&mut self, x: i64, y: i64, s: &str, c: Rgb) -> i64 {
        let mut cx = x;
        for ch in s.chars() {
            let g = glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        self.set(cx + col, y + row as i64, c);
                    }
                }
            }
            cx += 6;
        }
        cx - x
    }
}

/// Width in pixels of `s` in the font.
fn text_width(s: &str) -> i64 {
    s.chars().count() as i64 * 6
}

/// The 5×7 font: one byte per row, the five low bits from left to right. Lower case is drawn as
/// upper case; a character it lacks is a small box.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'D' => [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0a, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        'µ' | 'μ' => [0x00, 0x00, 0x12, 0x12, 0x12, 0x1d, 0x10],
        '.' => [0, 0, 0, 0, 0, 0x0c, 0x0c],
        ',' => [0, 0, 0, 0, 0x0c, 0x04, 0x08],
        ':' => [0, 0x0c, 0x0c, 0, 0x0c, 0x0c, 0],
        '-' => [0, 0, 0, 0x1f, 0, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1f, 0x04, 0x04, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0x1f],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '[' => [0x0e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0e],
        ']' => [0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e],
        '=' => [0, 0, 0x1f, 0, 0x1f, 0, 0],
        '#' => [0x0a, 0x0a, 0x1f, 0x0a, 0x1f, 0x0a, 0x0a],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
        '\'' => [0x04, 0x04, 0x08, 0, 0, 0, 0],
        '"' => [0x0a, 0x0a, 0, 0, 0, 0, 0],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '·' => [0, 0, 0, 0x04, 0, 0, 0],
        _ => [0, 0x0e, 0x0a, 0x0a, 0x0a, 0x0e, 0],
    }
}

/// Standard base64, for the PNG on the wire.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(char::from(A[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The raster as a PNG (8-bit RGB, no filtering, zlib from `miniz_oxide`).
pub fn png(r: &Raster) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_be_bytes());
        let mut h = crc32fast::Hasher::new();
        h.update(kind);
        h.update(data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&h.finalize().to_be_bytes());
    }
    let row = r.width as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * r.height as usize);
    for y in 0..r.height as usize {
        raw.push(0);
        raw.extend_from_slice(&r.px[y * row..(y + 1) * row]);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&r.width.to_be_bytes());
    ihdr.extend_from_slice(&r.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn colour(hex: Option<&str>) -> Rgb {
    let Some(h) = hex.and_then(|h| h.strip_prefix('#')).filter(|h| h.len() == 6) else { return DEFAULT_WAVE };
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0x80);
    [p(0), p(2), p(4)]
}

fn kind_colour(k: FrameKind) -> Rgb {
    match k {
        FrameKind::Data | FrameKind::Field | FrameKind::Sample => [0x2c, 0x5c, 0x9e],
        FrameKind::Address | FrameKind::Command => [0x6a, 0x3f, 0xa0],
        FrameKind::Start | FrameKind::RepeatedStart | FrameKind::Reset | FrameKind::Presence => {
            [0x2e, 0x7d, 0x4f]
        }
        FrameKind::Stop | FrameKind::Break => [0x8a, 0x5a, 0x1e],
        FrameKind::Ack => [0x3a, 0x6e, 0x6e],
        FrameKind::Nack => [0x8a, 0x34, 0x34],
        FrameKind::Cycle | FrameKind::Info => [0x44, 0x4a, 0x56],
    }
}

/// A time at a tick of `step` seconds, with as many decimals as the step needs ("660.005 ms").
pub fn tick_label(t: f64, step: f64) -> String {
    if t == 0.0 {
        return "0".into();
    }
    let (unit, name) = match t.abs() {
        a if a >= 1.0 => (1.0, "s"),
        a if a >= 1e-3 => (1e-3, "ms"),
        a if a >= 1e-6 => (1e-6, "µs"),
        _ => (1e-9, "ns"),
    };
    let decimals = (-(step / unit).log10()).ceil().clamp(0.0, 9.0);
    format!("{} {name}", format::fixed(t / unit, usize::try_from(num::to_u64(decimals)).unwrap_or(0)))
}

/// A channel's label: its number, and its name when that says more ("CH4 SDA").
fn channel_label(ch: &logica_core::capture::ChannelInfo) -> String {
    let plain = format!("CH{}", ch.index);
    if ch.name.is_empty() || ch.name == plain || ch.name == format!("channel {}", ch.index) {
        plain
    } else {
        format!("{plain} {}", ch.name)
    }
}

/// A rendered picture and what its rows show, top to bottom.
pub struct Picture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub rows: Vec<String>,
}

/// Draws `[from, to)` of a capture `width` pixels wide.
pub fn render(c: &Capture, from: Sample, to: Sample, width: u32) -> Result<Picture, String> {
    let count = c.store.sample_count();
    let to = to.min(count);
    if from >= to {
        return Err(format!("The range is empty: choose from < to inside 0 to {count}."));
    }
    let rate = c.meta.sample_rate_hz;
    let width = width.clamp(320, 4000);
    let plot = i64::from(width - GUTTER - 8);
    let span = to - from;
    let x_of = |s: Sample| -> i64 {
        let rel = s.saturating_sub(from).min(span) as f64 / span as f64;
        i64::from(GUTTER) + i64::try_from(num::floor(rel * plot as f64)).unwrap_or(0)
    };
    let channels: Vec<(usize, &logica_core::capture::ChannelInfo)> =
        c.meta.channels.iter().enumerate().filter(|(_, ch)| !c.hidden.contains(&ch.index)).collect();
    let decoders: Vec<(&String, &crate::state::Decoder)> = c.decoders.iter().collect();
    let rows_of = |n: usize, h: u32| u32::try_from(n).unwrap_or(u32::MAX).saturating_mul(h);
    let height = (AXIS + MARKER_ROW + 6)
        .saturating_add(rows_of(channels.len(), CHANNEL_ROW))
        .saturating_add(rows_of(decoders.len(), DECODER_ROW));
    let mut r = Raster::new(width, height, BG);

    // time axis with about eight ticks at round steps
    let seconds = span as f64 / rate;
    let raw_step = seconds / 8.0;
    let mag = 10f64.powf(raw_step.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw_step).unwrap_or(mag * 10.0);
    let mut rows = vec![format!(
        "time axis, samples {from} to {to} ({} to {}), ticks every {}",
        tick_label(from as f64 / rate, step),
        tick_label(to as f64 / rate, step),
        format::time(step)
    )];
    let mut k = (from as f64 / rate / step).ceil();
    while k * step <= to as f64 / rate {
        let s = num::to_u64(k * step * rate);
        let x = x_of(s);
        r.rect(x, i64::from(AXIS) - 4, x + 1, i64::from(height), GRID);
        let label = tick_label(k * step, step);
        if x + 2 + text_width(&label) <= i64::from(width) {
            r.text(x + 2, 2, &label, DIM);
        }
        k += 1.0;
    }
    r.rect(i64::from(GUTTER), i64::from(AXIS) - 1, i64::from(width), i64::from(AXIS), GRID);

    // markers under the waveforms (so they hide nothing), labels in their own row under the axis
    for m in c.markers.values().filter(|m| m.at < to && m.end.unwrap_or(m.at) >= from) {
        let x = x_of(m.at.max(from));
        if let Some(end) = m.end {
            let xe = x_of(end.min(to));
            r.rect(x, i64::from(AXIS + MARKER_ROW) - 2, xe.max(x + 1), i64::from(AXIS + MARKER_ROW), MARKER);
        }
        r.rect(x, i64::from(AXIS), x + 1, i64::from(height), MARKER);
        r.text(x + 3, i64::from(AXIS) + 3, &m.label.chars().take(24).collect::<String>(), MARKER);
        rows.push(format!("marker {} \"{}\" at sample {}", m.id, m.label, m.at));
    }
    // channels from the LOD: a column per bucket of samples
    let buckets = u64::try_from(plot).unwrap_or(1).min(span).max(1);
    let bucket = span.div_ceil(buckets);
    let n = usize::try_from(span.div_ceil(bucket)).unwrap_or(1);
    let mut y = i64::from(AXIS + MARKER_ROW);
    for (slot, ch) in &channels {
        let wave = colour(ch.colour.as_deref());
        let (hi, lo) = (y + 6, y + i64::from(CHANNEL_ROW) - 8);
        let name = channel_label(ch);
        r.text(4, y + 10, &name.chars().take(15).collect::<String>(), TEXT);
        rows.push(format!("channel {name}"));
        let lod = c.store.lod(*slot, from, bucket, n).map_err(|e| e.to_string())?;
        for (i, b) in lod.iter().enumerate() {
            let s0 = from + i as u64 * bucket;
            let (x0, x1) = (x_of(s0), x_of((s0 + bucket).min(to)).max(x_of(s0) + 1));
            let yl = |l: Level| if l == Level::High { hi } else { lo };
            match b.transitions {
                0 => r.rect(x0, yl(b.start_level), x1, yl(b.start_level) + 1, wave),
                // zoomed in, a column is one sample: its edge is at its start and its level is
                // the level after it
                1 if bucket == 1 => {
                    r.rect(x0, hi, x0 + 1, lo + 1, wave);
                    r.rect(x0, yl(b.start_level), x1, yl(b.start_level) + 1, wave);
                }
                _ => {
                    // edges inside one column: a band, solid where the column is too narrow to show
                    // anything else, so short bursts stay visible in a long range
                    let fill = if x1 - x0 <= 2 { wave } else { [wave[0] / 2, wave[1] / 2, wave[2] / 2] };
                    r.rect(x0, hi, x1, lo + 1, fill);
                    r.rect(x0, hi, x1, hi + 1, wave);
                    r.rect(x0, lo, x1, lo + 1, wave);
                }
            }
        }
        y += i64::from(CHANNEL_ROW);
    }

    // decoder rows: frames of row 0 as boxes, labels where they fit
    for (id, d) in &decoders {
        r.text(4, y + 7, &d.config.name.chars().take(15).collect::<String>(), TEXT);
        rows.push(format!("decoder {id} ({})", d.config.name));
        let frames: Vec<&Frame> = d
            .result
            .frames
            .iter()
            .filter(|f| f.row == 0 && f.end > from && f.start < to)
            .take(MAX_FRAMES)
            .collect();
        for f in frames {
            let (x0, x1) = (x_of(f.start.max(from)), x_of(f.end.min(to)).max(x_of(f.start.max(from)) + 1));
            let (y0, y1) = (y + 3, y + i64::from(DECODER_ROW) - 3);
            r.rect(x0, y0, x1, y1, kind_colour(f.kind));
            if !f.problems.is_empty() {
                r.outline(x0, y0, x1, y1, PROBLEM);
            }
            if text_width(&f.label) + 4 <= x1 - x0 {
                let tx = x0 + (x1 - x0 - text_width(&f.label)) / 2;
                r.text(tx, y0 + 4, &f.label, TEXT);
            }
        }
        y += i64::from(DECODER_ROW);
    }

    Ok(Picture { png: png(&r), width, height, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64("Grüße".as_bytes()), "R3LDvMOfZQ==");
    }

    #[test]
    fn png_is_well_formed_and_holds_the_pixels() {
        let mut r = Raster::new(3, 2, BG);
        r.rect(1, 1, 2, 2, MARKER);
        let p = png(&r);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes([p[16], p[17], p[18], p[19]]), 3);
        assert_eq!(u32::from_be_bytes([p[20], p[21], p[22], p[23]]), 2);
        // every chunk's CRC checks out, and the image data inflates to the rows
        let mut i = 8;
        let mut idat = Vec::new();
        while i < p.len() {
            let len = u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]) as usize;
            let kind = &p[i + 4..i + 8];
            let data = &p[i + 8..i + 8 + len];
            let crc = u32::from_be_bytes([p[i + 8 + len], p[i + 9 + len], p[i + 10 + len], p[i + 11 + len]]);
            assert_eq!(crc, crc32fast::hash(&[kind, data].concat()));
            if kind == b"IDAT" {
                idat.extend_from_slice(data);
            }
            i += 12 + len;
        }
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).unwrap();
        assert_eq!(raw.len(), 2 * (1 + 9));
        assert_eq!(&raw[10 + 1 + 3..10 + 1 + 6], &MARKER);
        assert_eq!(&raw[1..4], &BG);
    }

    #[test]
    fn tick_labels_carry_the_decimals_their_step_needs() {
        assert_eq!(tick_label(0.0, 1e-3), "0");
        assert_eq!(tick_label(0.660005, 5e-6), "660.005 ms");
        assert_eq!(tick_label(0.66, 0.002), "660 ms");
        assert_eq!(tick_label(1.5e-3, 5e-4), "1.5 ms");
        assert_eq!(tick_label(2.0, 0.5), "2.0 s");
        assert_eq!(tick_label(250e-9, 50e-9), "250 ns");
    }

    #[test]
    fn text_draws_inside_and_never_panics_at_the_edges() {
        let mut r = Raster::new(40, 10, BG);
        assert_eq!(r.text(1, 1, "I2C", TEXT), 18);
        assert_eq!(r.get(3, 1), TEXT); // the top bar of the I
        r.text(-3, -3, "edge µs ✓", TEXT);
        r.text(38, 8, "WW", TEXT);
    }
}
