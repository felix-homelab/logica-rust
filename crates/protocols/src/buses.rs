//! Grouping channels into buses (doc 06): channels busy at the same moments, or whose edges line
//! up with a clock, belong together. Behaviour follows Signal Detective (decision D12).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use logica_core::measure::{ChannelMeasurement, Span, total_length};

use crate::window::{WinChannel, Window};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bus {
    pub id: String,
    /// Physical channel numbers, ascending.
    pub channels: Vec<u32>,
    /// Why the channels were put together (or left alone), in plain words.
    pub reason: String,
}

/// Fraction of `a`'s spans that overlap any of `b`'s.
fn overlap_share(a: &[Span], b: &[Span], pad: f64) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let mut hit = 0;
    let mut j = 0;
    for x in a {
        while j < b.len() && b[j].end + pad < x.start {
            j += 1;
        }
        if b.get(j).is_some_and(|y| y.start - pad <= x.end) {
            hit += 1;
        }
    }
    f64::from(hit) / a.len() as f64
}

/// Fraction of `b`'s edges that land within `w` samples of an edge of `a`.
fn edge_sync(a: &[u64], b: &[u64], w: u64) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let hit = b
        .iter()
        .filter(|&&s| {
            let k = a.partition_point(|&x| x < s.saturating_sub(w));
            a.get(k).is_some_and(|&x| x <= s + w)
        })
        .count();
    hit as f64 / b.len() as f64
}

/// Puts channels that are busy at the same moments into one bus. Two tests, either links a pair:
/// most bursts of one channel overlap bursts of the other (channels busy nearly all the time are
/// left out of this test, as they overlap everything by chance); or one channel is a regular
/// clock and most edges of the other line up with its edges, clearly more often than chance.
pub fn group_buses(window: &Window, m: &BTreeMap<u32, ChannelMeasurement>) -> Vec<Bus> {
    let samples = window.samples().max(1) as f64;
    let active: Vec<&WinChannel> = window.channels.iter().filter(|c| !c.edges.is_empty()).collect();
    let mut parent: BTreeMap<u32, u32> = active.iter().map(|c| (c.index, c.index)).collect();
    fn find(parent: &mut BTreeMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while parent[&r] != r {
            r = parent[&r];
        }
        parent.insert(x, r);
        r
    }
    let mut why: BTreeMap<(u32, u32), String> = BTreeMap::new();
    let mut link = |parent: &mut BTreeMap<u32, u32>, a: u32, b: u32, reason: String| {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            parent.insert(rb, ra);
        }
        why.insert((a.min(b), a.max(b)), reason);
    };

    for i in 0..active.len() {
        for j in i + 1..active.len() {
            let (a, b) = (active[i], active[j]);
            let (ma, mb) = (&m[&a.index], &m[&b.index]);
            let busy_a = total_length(&ma.bursts) / samples;
            let busy_b = total_length(&mb.bursts) / samples;
            if busy_a < 0.6 && busy_b < 0.6 {
                let pad =
                    ma.narrowest.map_or(1.0, |n| n.samples).max(mb.narrowest.map_or(1.0, |n| n.samples));
                let ab = overlap_share(&ma.bursts, &mb.bursts, pad);
                let ba = overlap_share(&mb.bursts, &ma.bursts, pad);
                if ab.max(ba) >= 0.8 && ab.min(ba) >= 0.2 {
                    link(
                        &mut parent,
                        a.index,
                        b.index,
                        format!("channels {} and {} are busy at the same moments", a.index, b.index),
                    );
                    continue;
                }
            }
            for (clk, other, mc, mo) in [(a, b, ma, mb), (b, a, mb, ma)] {
                let Some(p) = &mc.periodic else { continue };
                if other.edges.len() < 8
                    || mo.periodic.as_ref().is_some_and(|q| q.period_samples == p.period_samples)
                {
                    continue;
                }
                let w = (0.05 * p.period_samples)
                    .min(0.1 * mo.narrowest.map_or(10.0, |n| n.samples))
                    .round()
                    .max(1.0);
                let sync = edge_sync(&clk.edges, &other.edges, crate::num::to_u64(w));
                let chance = (clk.edges.len() as f64 / samples * (2.0 * w + 1.0)).min(1.0);
                if sync >= 0.8 && sync - chance >= 0.3 {
                    link(
                        &mut parent,
                        clk.index,
                        other.index,
                        format!(
                            "the edges of channel {} line up with the clock on channel {}",
                            other.index, clk.index
                        ),
                    );
                    break;
                }
            }
        }
    }

    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for c in &active {
        let r = find(&mut parent, c.index);
        groups.entry(r).or_default().push(c.index);
    }
    let mut buses: Vec<Bus> = Vec::new();
    for mut list in groups.into_values() {
        list.sort_unstable();
        let reason = if list.len() == 1 {
            format!(
                "channel {} is not busy at the same moments as any other channel, so it is on its own",
                list[0]
            )
        } else {
            why.iter()
                .filter(|((a, _), _)| list.contains(a))
                .map(|(_, v)| v.clone())
                .take(3)
                .collect::<Vec<_>>()
                .join("; ")
        };
        buses.push(Bus {
            id: format!("bus-{}", list.iter().map(u32::to_string).collect::<Vec<_>>().join("-")),
            channels: list,
            reason,
        });
    }
    // Channels that never change get a bus of their own so they still appear in the results.
    for c in &window.channels {
        if c.edges.is_empty() {
            buses.push(Bus {
                id: format!("bus-{}", c.index),
                channels: vec![c.index],
                reason: format!("channel {} never changes level", c.index),
            });
        }
    }
    buses.sort_by_key(|b| b.channels[0]);
    buses
}
