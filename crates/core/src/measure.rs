//! Per-channel measurements (doc 03, doc 06): pulse widths grouped into clusters, the narrowest
//! repeated pulse, glitches, bursts of activity, the rest level and whether the line is a regular
//! clock. They work on the edges of one channel inside a window `[start, end)`, so detection on a
//! huge capture stays bounded. Behaviour follows Signal Detective's measurements (decision D12).

use serde::{Deserialize, Serialize};

use crate::{Level, Sample};

/// A half-open stretch of samples; fractional ends come from averaged periods.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

/// Covered length of sorted, non-overlapping spans.
pub fn total_length(spans: &[Span]) -> f64 {
    spans.iter().map(|s| s.end - s.start).sum()
}

/// A group of similar pulse widths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidthCluster {
    pub min: u64,
    pub max: u64,
    pub mean: f64,
    pub count: u64,
    pub high: u64,
    pub low: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Periodicity {
    /// Average rising-to-rising period inside bursts, in samples.
    pub period_samples: f64,
    /// Average high time, in samples.
    pub high_samples: f64,
    pub duty_cycle: f64,
    /// Fraction of periods within a few percent of the typical one (1 = perfectly regular).
    pub regularity: f64,
    /// The signal never pauses between its first and last edge and spans most of the window.
    pub continuous: bool,
    /// Clock pulses per burst (bursts split at pauses longer than 3 periods).
    pub pulses_per_burst: Vec<u64>,
    pub bursts: Vec<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Narrowest {
    pub samples: f64,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMeasurement {
    /// Physical channel number.
    pub channel: u32,
    pub edge_count: u64,
    /// Level during quiet stretches.
    pub rest_level: Level,
    pub first_edge: Option<Sample>,
    pub last_edge: Option<Sample>,
    /// Narrowest pulse that occurs repeatedly (single glitches ignored).
    pub narrowest: Option<Narrowest>,
    /// Pulses well under half the narrowest repeated width.
    pub glitches: u64,
    /// Pulse widths grouped, narrowest first.
    pub clusters: Vec<WidthCluster>,
    /// Fraction of the active stretch spent high.
    pub high_fraction: f64,
    /// Stretches of activity, split at pauses longer than `burst_gap`.
    pub bursts: Vec<Span>,
    pub burst_gap: f64,
    pub periodic: Option<Periodicity>,
}

/// Level of pulse `i`, the stretch from edge `i` to edge `i + 1`.
pub const fn pulse_level(initial: Level, i: usize) -> Level {
    if i.is_multiple_of(2) {
        match initial {
            Level::Low => Level::High,
            Level::High => Level::Low,
        }
    } else {
        initial
    }
}

fn near(x: f64, reference: f64, rel: f64) -> bool {
    (x - reference).abs() <= reference * rel + 1.0
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut s = values.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() >> 1]
}

/// Groups sorted widths: neighbours within 6 % (or 1 sample) belong together.
pub fn cluster_widths(widths: &std::collections::BTreeMap<u64, (u64, u64)>) -> Vec<WidthCluster> {
    let mut out: Vec<WidthCluster> = Vec::new();
    let mut sum = 0f64;
    for (&w, &(high, low)) in widths {
        let n = high + low;
        match out.last_mut() {
            Some(cur) if w as f64 <= cur.max as f64 * 1.06 + 1.0 => {
                cur.max = w;
                cur.count += n;
                cur.high += high;
                cur.low += low;
                sum += w as f64 * n as f64;
                cur.mean = sum / cur.count as f64;
            }
            _ => {
                out.push(WidthCluster { min: w, max: w, mean: w as f64, count: n, high, low });
                sum = w as f64 * n as f64;
            }
        }
    }
    out
}

/// How clock-like a channel is; `None` for anything that is not regular.
pub fn measure_periodicity(edges: &[Sample], initial: Level, window: u64) -> Option<Periodicity> {
    let rises: Vec<Sample> = edges
        .iter()
        .enumerate()
        .filter(|&(i, _)| pulse_level(initial, i) == Level::High)
        .map(|(_, &e)| e)
        .collect();
    if rises.len() < 9 {
        return None;
    }
    let periods: Vec<f64> = rises.windows(2).map(|w| (w[1] - w[0]) as f64).collect();
    let p0 = median(&periods);
    let in_burst: Vec<f64> = periods.iter().copied().filter(|&p| p < 3.0 * p0).collect();
    let regular: Vec<f64> = in_burst.iter().copied().filter(|&p| near(p, p0, 0.2)).collect();
    let regularity = regular.len() as f64 / in_burst.len().max(1) as f64;
    let highs: Vec<f64> = (0..edges.len().saturating_sub(1))
        .filter(|&i| pulse_level(initial, i) == Level::High)
        .map(|i| (edges[i + 1] - edges[i]) as f64)
        .filter(|&w| w < 3.0 * p0)
        .collect();
    let h0 = median(&highs);
    let good_highs: Vec<f64> = highs.iter().copied().filter(|&w| near(w, h0, 0.2)).collect();
    let high_reg = good_highs.len() as f64 / highs.len().max(1) as f64;
    if regularity < 0.85 || high_reg < 0.85 || regular.len() < 8 {
        return None;
    }
    let period_samples = regular.iter().sum::<f64>() / regular.len() as f64;
    let high_samples = good_highs.iter().sum::<f64>() / good_highs.len() as f64;
    let mut bursts = Vec::new();
    let mut pulses_per_burst = Vec::new();
    let mut start = rises[0] as f64;
    let mut n = 1u64;
    for i in 1..rises.len() {
        let gap = (rises[i] - rises[i - 1]) as f64;
        if gap > 3.0 * period_samples {
            bursts.push(Span { start, end: rises[i - 1] as f64 + period_samples });
            pulses_per_burst.push(n);
            start = rises[i] as f64;
            n = 0;
        }
        n += 1;
    }
    bursts.push(Span { start, end: rises[rises.len() - 1] as f64 + period_samples });
    pulses_per_burst.push(n);
    let span = (edges[edges.len() - 1] - edges[0]) as f64;
    let continuous = bursts.len() == 1 && span > 0.5 * window as f64;
    Some(Periodicity {
        period_samples,
        high_samples,
        duty_cycle: high_samples / period_samples,
        regularity,
        continuous,
        pulses_per_burst,
        bursts,
    })
}

/// Measures one channel: `edges` ascending inside `[start, end)`, `initial` the level before them.
pub fn measure_channel(
    channel: u32,
    edges: &[Sample],
    initial: Level,
    start: Sample,
    end: Sample,
) -> ChannelMeasurement {
    let n = edges.len();
    let window = end.saturating_sub(start);
    let mut m = ChannelMeasurement {
        channel,
        edge_count: n as u64,
        rest_level: initial,
        first_edge: edges.first().copied(),
        last_edge: edges.last().copied(),
        narrowest: None,
        glitches: 0,
        clusters: Vec::new(),
        high_fraction: if initial.is_high() { 1.0 } else { 0.0 },
        bursts: Vec::new(),
        burst_gap: 0.0,
        periodic: None,
    };
    if n == 0 {
        return m;
    }
    let mut widths: std::collections::BTreeMap<u64, (u64, u64)> = std::collections::BTreeMap::new();
    let mut high_time = 0u64;
    for i in 0..n - 1 {
        let w = edges[i + 1] - edges[i];
        let c = widths.entry(w).or_default();
        if pulse_level(initial, i).is_high() {
            c.0 += 1;
            high_time += w;
        } else {
            c.1 += 1;
        }
    }
    let clusters = cluster_widths(&widths);
    let min_count = u64::try_from(n.saturating_sub(1).div_ceil(500)).unwrap_or(u64::MAX).max(2);
    let first_real = clusters.iter().find(|c| c.count >= min_count).or(clusters.first());
    let glitches =
        first_real.map_or(0, |f| clusters.iter().filter(|c| c.mean < f.mean * 0.5).map(|c| c.count).sum());
    let narrowest = first_real.map(|f| Narrowest { samples: f.mean, count: f.count });
    let unit = narrowest.map_or(1.0, |x| x.samples);
    let periodic = measure_periodicity(edges, initial, window);
    let burst_gap = 20f64.max(30.0 * unit).max(periodic.as_ref().map_or(0.0, |p| 3.0 * p.period_samples));
    let mut bursts = Vec::new();
    let mut bs = edges[0];
    for i in 1..n {
        if (edges[i] - edges[i - 1]) as f64 > burst_gap {
            bursts.push(Span { start: bs as f64, end: (edges[i - 1] + 1) as f64 });
            bs = edges[i];
        }
    }
    bursts.push(Span { start: bs as f64, end: (edges[n - 1] + 1) as f64 });

    // Rest level: the level before the first edge and after the last, if they agree; otherwise
    // the level of the long quiet stretches.
    let start_level = initial;
    let end_level = if n % 2 == 1 { !initial } else { initial };
    let mut rest = start_level;
    if start_level != end_level {
        let mut quiet = [0f64; 2];
        quiet[start_level as usize] += (edges[0] - start) as f64;
        quiet[end_level as usize] += end.saturating_sub(edges[n - 1]) as f64;
        for i in 0..n - 1 {
            let w = (edges[i + 1] - edges[i]) as f64;
            if w > burst_gap {
                quiet[pulse_level(initial, i) as usize] += w;
            }
        }
        rest = Level::from(quiet[1] > quiet[0]);
    }
    let span = (edges[n - 1] - edges[0]) as f64;
    m.rest_level = rest;
    m.narrowest = narrowest;
    m.glitches = glitches;
    m.clusters = clusters;
    m.high_fraction = if span > 0.0 { high_time as f64 / span } else { m.high_fraction };
    m.bursts = bursts;
    m.burst_gap = burst_gap;
    m.periodic = periodic;
    m
}

/// The fraction of pulses (by count) whose width is close to a whole multiple of `unit`.
pub fn multiple_fit(clusters: &[WidthCluster], unit: f64, tolerance: f64) -> f64 {
    let (mut ok, mut all) = (0u64, 0u64);
    for c in clusters {
        all += c.count;
        let k = (c.mean / unit).round();
        if k >= 1.0 && (c.mean - k * unit).abs() <= tolerance * unit + 1.0 {
            ok += c.count;
        }
    }
    if all == 0 { 0.0 } else { ok as f64 / all as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_clock_is_periodic() {
        // rising every 10 samples, high for 3, 12 periods, in a window of 1000
        let mut e = Vec::new();
        for k in 0..12u64 {
            e.push(100 + 10 * k);
            e.push(103 + 10 * k);
        }
        let m = measure_channel(3, &e, Level::Low, 0, 1000);
        let p = m.periodic.expect("periodic");
        assert_eq!(p.period_samples, 10.0);
        assert_eq!(p.high_samples, 3.0);
        assert!((p.duty_cycle - 0.3).abs() < 1e-9);
        assert!(!p.continuous);
        assert_eq!(p.pulses_per_burst, vec![12]);
        assert_eq!(m.rest_level, Level::Low);
        assert_eq!(m.narrowest.unwrap().samples, 3.0);
        assert_eq!(m.edge_count, 24);
    }

    #[test]
    fn widths_cluster_within_6_percent() {
        let mut w = std::collections::BTreeMap::new();
        w.insert(100, (1, 0));
        w.insert(105, (0, 1));
        w.insert(200, (2, 0));
        let c = cluster_widths(&w);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].min, c[0].max, c[0].count), (100, 105, 2));
        assert_eq!(c[0].mean, 102.5);
        assert!((multiple_fit(&c, 100.0, 0.18) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_quiet_line_has_no_pulses() {
        let m = measure_channel(0, &[], Level::High, 0, 50);
        assert_eq!(m.rest_level, Level::High);
        assert!(m.periodic.is_none() && m.narrowest.is_none());
        assert_eq!(m.high_fraction, 1.0);
    }

    #[test]
    fn rest_level_comes_from_long_quiet_stretches() {
        // starts low, ends high; long high stretches dominate
        let e = vec![10, 12, 14, 5000, 5002];
        let m = measure_channel(0, &e, Level::Low, 0, 20_000);
        assert_eq!(m.rest_level, Level::High);
    }
}
