//! The value series of a virtual analog channel (doc 04, "Value series").
//!
//! A series holds what the channel's source emitted: points of a sample index and a number (a
//! fraction of full scale or a measured quantity), and the places where the source had no value.
//! Scaling is applied when the series is read, so changing it costs nothing. Points are kept in
//! pages, so a long series grows without copying, and a level of detail per 2^16 samples lets a
//! zoomed-out view draw an envelope without reading every point.

use std::collections::BTreeMap;
use std::fmt;

use crate::store::{COARSE_SHIFT, Sample};

/// Points per page.
pub const PAGE_POINTS: usize = 1 << 16;
/// Samples per level-of-detail bucket, the same as the coarse LOD of the capture store.
pub const LOD_SHIFT: u32 = COARSE_SHIFT;

/// One entry of a series: a value from `sample` on, or `None` when the source has no value from
/// `sample` on (a gap). A value holds until the next entry.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Point {
    pub sample: Sample,
    pub x: Option<f64>,
}

/// What a stretch of samples holds, for drawing at any zoom level.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Envelope {
    /// The value held at the start of the stretch, from a point before it; `None` in a gap.
    pub held: Option<f64>,
    /// Values that start inside the stretch.
    pub count: u64,
    /// The lowest and highest of those values; `None` when there are none.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Whether a gap starts inside the stretch.
    pub gap: bool,
}

/// Why an entry cannot be added. Sources keep time order and emit finite numbers; this guards
/// the store against a source that does not.
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesError {
    NotIncreasing { sample: Sample, last: Sample },
    NotFinite { sample: Sample },
}

impl fmt::Display for SeriesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeriesError::NotIncreasing { sample, last } => write!(
                f,
                "A value at sample {sample} came after one at sample {last}; values must come in \
                 time order."
            ),
            SeriesError::NotFinite { sample } => {
                write!(f, "The value at sample {sample} is infinite or not a number.")
            }
        }
    }
}

impl std::error::Error for SeriesError {}

#[derive(Clone, Debug, Default)]
struct Page {
    samples: Vec<Sample>,
    /// NaN marks a gap.
    xs: Vec<f64>,
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    count: u32,
    min: f64,
    max: f64,
    gap: bool,
}

impl Bucket {
    const EMPTY: Bucket = Bucket { count: 0, min: f64::INFINITY, max: f64::NEG_INFINITY, gap: false };
}

/// The points of one virtual analog channel, in time order.
#[derive(Clone, Debug, Default)]
pub struct ValueSeries {
    pages: Vec<Page>,
    /// Only buckets that hold an entry, so memory follows the points, not the capture length.
    lod: BTreeMap<u64, Bucket>,
    len: usize,
}

impl ValueSeries {
    pub fn new() -> Self {
        Self::default()
    }

    /// Entries (values and gaps).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The last entry.
    pub fn last(&self) -> Option<Point> {
        self.len.checked_sub(1).map(|i| self.entry(i))
    }

    /// Adds a value from `sample` on.
    pub fn push(&mut self, sample: Sample, x: f64) -> Result<(), SeriesError> {
        if !x.is_finite() {
            return Err(SeriesError::NotFinite { sample });
        }
        self.check_order(sample)?;
        self.append(sample, x);
        Ok(())
    }

    /// Marks that there is no value from `sample` on. A gap before any value, or right after
    /// another gap, changes nothing.
    pub fn gap(&mut self, sample: Sample) -> Result<(), SeriesError> {
        self.check_order(sample)?;
        if matches!(self.last(), Some(Point { x: Some(_), .. })) {
            self.append(sample, f64::NAN);
        }
        Ok(())
    }

    fn check_order(&self, sample: Sample) -> Result<(), SeriesError> {
        match self.last() {
            Some(p) if p.sample >= sample => Err(SeriesError::NotIncreasing { sample, last: p.sample }),
            _ => Ok(()),
        }
    }

    fn append(&mut self, sample: Sample, x: f64) {
        if self.pages.last().is_none_or(|p| p.samples.len() == PAGE_POINTS) {
            self.pages.push(Page {
                samples: Vec::with_capacity(PAGE_POINTS.min(1024)),
                xs: Vec::with_capacity(PAGE_POINTS.min(1024)),
            });
        }
        let page = self.pages.last_mut().expect("a page was just ensured");
        page.samples.push(sample);
        page.xs.push(x);
        self.len += 1;
        let b = self.lod.entry(sample >> LOD_SHIFT).or_insert(Bucket::EMPTY);
        if x.is_nan() {
            b.gap = true;
        } else {
            b.count += 1;
            b.min = b.min.min(x);
            b.max = b.max.max(x);
        }
    }

    fn entry(&self, i: usize) -> Point {
        let page = &self.pages[i / PAGE_POINTS];
        let x = page.xs[i % PAGE_POINTS];
        Point { sample: page.samples[i % PAGE_POINTS], x: (!x.is_nan()).then_some(x) }
    }

    /// The index of the first entry at or after `sample`.
    fn lower_bound(&self, sample: Sample) -> usize {
        let p = self.pages.partition_point(|pg| pg.samples.last().is_some_and(|&s| s < sample));
        match self.pages.get(p) {
            None => self.len,
            Some(pg) => p * PAGE_POINTS + pg.samples.partition_point(|&s| s < sample),
        }
    }

    /// The value held at `sample`: from the last entry at or before it.
    pub fn value_at(&self, sample: Sample) -> Option<f64> {
        self.held_before(sample.saturating_add(1))
    }

    /// The value held just before `sample`: from the last entry before it.
    pub fn held_before(&self, sample: Sample) -> Option<f64> {
        self.lower_bound(sample).checked_sub(1).and_then(|i| self.entry(i).x)
    }

    /// The entries with `from <= sample < to`, in order.
    pub fn points(&self, from: Sample, to: Sample) -> impl Iterator<Item = Point> + '_ {
        (self.lower_bound(from)..self.len).map(|i| self.entry(i)).take_while(move |p| p.sample < to)
    }

    /// The envelopes of `n` stretches of `bucket` samples each, starting at `from`. A `bucket` of
    /// 0 gives none.
    pub fn envelopes(&self, from: Sample, bucket: u64, n: usize) -> Vec<Envelope> {
        if bucket == 0 {
            return Vec::new();
        }
        (0..n as u64)
            .map_while(|k| {
                let a = from.checked_add(k.checked_mul(bucket)?)?;
                Some(self.envelope(a, a.saturating_add(bucket)))
            })
            .collect()
    }

    /// The envelope of the samples `from..to`.
    pub fn envelope(&self, from: Sample, to: Sample) -> Envelope {
        let mut e = Envelope { held: self.held_before(from), ..Envelope::default() };
        // Whole LOD buckets inside the stretch are read from the LOD, the ends from the points.
        let first = from.div_ceil(1 << LOD_SHIFT);
        let end = to >> LOD_SHIFT;
        if first < end {
            self.scan(from, first << LOD_SHIFT, &mut e);
            for b in self.lod.range(first..end).map(|(_, b)| b) {
                add(&mut e, b);
            }
            self.scan(end << LOD_SHIFT, to, &mut e);
        } else {
            self.scan(from, to, &mut e);
        }
        e
    }

    fn scan(&self, from: Sample, to: Sample, e: &mut Envelope) {
        for p in self.points(from, to) {
            match p.x {
                None => e.gap = true,
                Some(x) => add(e, &Bucket { count: 1, min: x, max: x, gap: false }),
            }
        }
    }
}

fn add(e: &mut Envelope, b: &Bucket) {
    e.gap |= b.gap;
    if b.count > 0 {
        e.count += u64::from(b.count);
        e.min = Some(e.min.map_or(b.min, |m| m.min(b.min)));
        e.max = Some(e.max.map_or(b.max, |m| m.max(b.max)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so the tests need no random crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// The envelope from every entry, the slow and obvious way.
    fn brute(entries: &[Point], from: Sample, to: Sample) -> Envelope {
        let mut e = Envelope {
            held: entries.iter().rev().find(|p| p.sample < from).and_then(|p| p.x),
            ..Envelope::default()
        };
        for p in entries.iter().filter(|p| p.sample >= from && p.sample < to) {
            match p.x {
                None => e.gap = true,
                Some(x) => add(&mut e, &Bucket { count: 1, min: x, max: x, gap: false }),
            }
        }
        e
    }

    fn build(seed: u64, n: usize, max_step: u64) -> (ValueSeries, Vec<Point>) {
        let mut r = Lcg(seed);
        let mut s = ValueSeries::new();
        let mut sample = r.next() % 1000;
        for _ in 0..n {
            if r.next().is_multiple_of(50) {
                s.gap(sample).unwrap();
            } else {
                s.push(sample, (r.next() % 10_000) as f64 / 10_000.0).unwrap();
            }
            sample += 1 + r.next() % max_step;
        }
        let all = s.points(0, Sample::MAX).collect();
        (s, all)
    }

    #[test]
    fn values_hold_until_the_next_entry_and_gaps_end_them() {
        let mut s = ValueSeries::new();
        s.push(10, 0.25).unwrap();
        s.push(20, 0.5).unwrap();
        s.gap(30).unwrap();
        s.push(40, 0.75).unwrap();
        assert_eq!(s.value_at(9), None);
        assert_eq!(s.value_at(10), Some(0.25));
        assert_eq!(s.value_at(19), Some(0.25));
        assert_eq!(s.held_before(20), Some(0.25));
        assert_eq!(s.value_at(20), Some(0.5));
        assert_eq!(s.value_at(35), None);
        assert_eq!(s.value_at(Sample::MAX), Some(0.75));
        assert_eq!(s.len(), 4);
        assert_eq!(s.last(), Some(Point { sample: 40, x: Some(0.75) }));
    }

    #[test]
    fn leading_and_repeated_gaps_are_not_stored() {
        let mut s = ValueSeries::new();
        s.gap(0).unwrap();
        assert!(s.is_empty());
        s.push(5, 1.0).unwrap();
        s.gap(6).unwrap();
        s.gap(7).unwrap();
        assert_eq!(s.len(), 2);
        // Even a gap that would be ignored must keep time order.
        assert!(s.gap(6).is_err());
        assert!(s.gap(2).is_err());
    }

    #[test]
    fn entries_out_of_order_or_not_finite_are_refused() {
        let mut s = ValueSeries::new();
        s.push(10, 1.0).unwrap();
        let e = s.push(10, 2.0).unwrap_err();
        assert_eq!(e, SeriesError::NotIncreasing { sample: 10, last: 10 });
        assert!(e.to_string().contains("time order"), "{e}");
        assert!(s.push(9, 2.0).is_err());
        assert!(s.gap(3).is_err());
        assert_eq!(s.push(11, f64::NAN), Err(SeriesError::NotFinite { sample: 11 }));
        assert!(s.push(11, f64::INFINITY).is_err());
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn points_across_page_boundaries() {
        let mut s = ValueSeries::new();
        let n = 2 * PAGE_POINTS + 7;
        for i in 0..n as u64 {
            s.push(i * 3, i as f64).unwrap();
        }
        assert_eq!(s.len(), n);
        let got: Vec<_> = s.points(3 * (PAGE_POINTS as u64 - 2), 3 * (PAGE_POINTS as u64 + 2) + 1).collect();
        assert_eq!(got.len(), 5);
        assert_eq!(got[0].sample, 3 * (PAGE_POINTS as u64 - 2));
        assert_eq!(got[4].x, Some((PAGE_POINTS + 2) as f64));
        assert_eq!(s.points(1, 3).count(), 0);
        assert_eq!(s.points(3 * n as u64, Sample::MAX).count(), 0);
        assert_eq!(s.value_at(3 * PAGE_POINTS as u64 + 1), Some(PAGE_POINTS as f64));
    }

    #[test]
    fn envelopes_match_every_entry_for_dense_and_sparse_series() {
        for (seed, n, step) in [(1, 300_000, 3), (2, 5_000, 200_000), (3, 70_000, 40)] {
            let (s, all) = build(seed, n, step);
            let end = all.last().unwrap().sample + 10;
            for bucket in [1, 255, 1 << LOD_SHIFT, (1 << LOD_SHIFT) + 17, 3 << LOD_SHIFT, end / 7] {
                for from in [0, 1, 1 << LOD_SHIFT, end / 3 + 5] {
                    let got = s.envelopes(from, bucket, 9);
                    assert_eq!(got.len(), 9);
                    for (k, e) in got.iter().enumerate() {
                        let a = from + k as u64 * bucket;
                        assert_eq!(
                            *e,
                            brute(&all, a, a + bucket),
                            "seed {seed}, bucket {bucket}, from {from}, k {k}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn envelopes_of_an_empty_series_and_of_impossible_requests() {
        let s = ValueSeries::new();
        assert_eq!(s.envelopes(0, 100, 2), vec![Envelope::default(); 2]);
        assert!(s.envelopes(0, 0, 5).is_empty());
        // Stretches that would pass the last sample index are left out, not wrapped.
        assert_eq!(s.envelopes(Sample::MAX - 10, 4, 5).len(), 3);
    }

    #[test]
    fn memory_follows_points_not_sample_indices() {
        let mut s = ValueSeries::new();
        s.push(1, 0.5).unwrap();
        s.push(1 << 52, 0.25).unwrap();
        assert_eq!(s.lod.len(), 2);
        let e = s.envelope(0, (1 << 52) + 1);
        assert_eq!((e.count, e.min, e.max), (2, Some(0.25), Some(0.5)));
        assert_eq!(s.envelope(2, 1 << 52).held, Some(0.5));
    }
}
