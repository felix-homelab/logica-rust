//! Bit-level helpers for raw sample bits: sample k is bit k % 64 of word k / 64.
//!
//! A transition is a sample whose level differs from the sample before it. For a word `x` and the
//! level `prev` of the sample before its bit 0, `x ^ (x << 1 | prev)` has a one exactly at the
//! transitions; popcount counts them without extracting positions (doc 04).

use super::Level;

/// Reads 64 bits starting at bit `pos`; bits past the end of `src` read as 0.
#[inline]
pub fn window(src: &[u64], pos: u64) -> u64 {
    let w = idx(pos / 64);
    let sh = pos % 64;
    let Some(&lo) = src.get(w) else { return 0 };
    if sh == 0 {
        return lo;
    }
    let hi = src.get(w + 1).copied().unwrap_or(0);
    (lo >> sh) | (hi << (64 - sh))
}

/// The level of bit `pos`.
#[inline]
pub fn get(src: &[u64], pos: u64) -> Level {
    Level::from_bit(src[idx(pos / 64)] >> (pos % 64))
}

/// A mask of the lowest `n` bits, `n` in 0..=64.
#[inline]
pub const fn low_mask(n: u64) -> u64 {
    if n >= 64 { u64::MAX } else { (1u64 << n) - 1 }
}

/// Converts a word or bit index that is known to fit in memory.
#[inline]
#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn idx(v: u64) -> usize {
    v as usize
}

/// Converts a position inside a chunk (below 2^24) or a count up to 2^24 to `u32`.
#[inline]
#[allow(clippy::cast_possible_truncation)]
pub(crate) const fn off32(v: u64) -> u32 {
    debug_assert!(v <= 1 << 24);
    v as u32
}

/// Transitions among the `n` bits of `src` starting at bit `off`, given the level of the sample
/// before them. Returns the count and the level of the last bit (or `prev` if `n` is 0).
pub fn count_transitions(src: &[u64], off: u64, n: u64, prev: Level) -> (u64, Level) {
    let mut count = 0u64;
    let mut prev_bit = prev as u64;
    let mut p = 0u64;
    if off.is_multiple_of(64) {
        // Aligned: whole words straight from the slice.
        let start = idx(off / 64);
        let whole = idx(n / 64);
        for &x in &src[start..start + whole] {
            count += u64::from((x ^ ((x << 1) | prev_bit)).count_ones());
            prev_bit = x >> 63;
        }
        p = n / 64 * 64;
    }
    while p < n {
        let valid = (n - p).min(64);
        let x = window(src, off + p) & low_mask(valid);
        let t = (x ^ ((x << 1) | prev_bit)) & low_mask(valid);
        count += u64::from(t.count_ones());
        prev_bit = (x >> (valid - 1)) & 1;
        p += valid;
    }
    (count, Level::from_bit(prev_bit))
}

/// Calls `f` with the position (relative to `off`) of every transition among the `n` bits of `src`
/// starting at bit `off`, in order. Returns the level of the last bit (or `prev` if `n` is 0).
pub fn for_each_transition(src: &[u64], off: u64, n: u64, prev: Level, mut f: impl FnMut(u64)) -> Level {
    let mut prev_bit = prev as u64;
    let mut p = 0u64;
    while p < n {
        let valid = (n - p).min(64);
        let x = window(src, off + p) & low_mask(valid);
        let mut t = (x ^ ((x << 1) | prev_bit)) & low_mask(valid);
        while t != 0 {
            f(p + u64::from(t.trailing_zeros()));
            t &= t - 1;
        }
        prev_bit = (x >> (valid - 1)) & 1;
        p += valid;
    }
    Level::from_bit(prev_bit)
}

/// Sets bits `[from, to)` of `dst` to `level`.
pub fn fill(dst: &mut [u64], from: u64, to: u64, level: Level) {
    if from >= to {
        return;
    }
    let fill = level.fill_word();
    let (w0, w1) = (idx(from / 64), idx((to - 1) / 64));
    let head = !low_mask(from % 64);
    let tail = low_mask((to - 1) % 64 + 1);
    if w0 == w1 {
        let m = head & tail;
        dst[w0] = (dst[w0] & !m) | (fill & m);
        return;
    }
    dst[w0] = (dst[w0] & !head) | (fill & head);
    for w in &mut dst[w0 + 1..w1] {
        *w = fill;
    }
    dst[w1] = (dst[w1] & !tail) | (fill & tail);
}

/// Copies `n` bits from `src` at bit `src_off` to `dst` at bit `dst_off`.
pub fn copy(dst: &mut [u64], dst_off: u64, src: &[u64], src_off: u64, n: u64) {
    let mut p = 0u64;
    // Head: bring the destination to a word boundary.
    if !dst_off.is_multiple_of(64) && n > 0 {
        let k = (64 - dst_off % 64).min(n);
        let x = window(src, src_off) & low_mask(k);
        let w = idx(dst_off / 64);
        let sh = dst_off % 64;
        let m = low_mask(k) << sh;
        dst[w] = (dst[w] & !m) | (x << sh);
        p = k;
    }
    while n - p >= 64 {
        dst[idx((dst_off + p) / 64)] = window(src, src_off + p);
        p += 64;
    }
    if p < n {
        let k = n - p;
        let w = idx((dst_off + p) / 64);
        let m = low_mask(k);
        dst[w] = (dst[w] & !m) | (window(src, src_off + p) & m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_bits(levels: &[u8]) -> Vec<u64> {
        let mut v = vec![0u64; levels.len().div_ceil(64) + 1];
        for (i, &l) in levels.iter().enumerate() {
            v[i / 64] |= u64::from(l) << (i % 64);
        }
        v
    }

    fn pseudo_random(n: usize, seed: u64, density: u64) -> Vec<u8> {
        let mut s = seed;
        let mut level = 0u8;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                if (s >> 33).is_multiple_of(density) {
                    level ^= 1;
                }
                level
            })
            .collect()
    }

    fn reference_transitions(levels: &[u8], prev: u8) -> Vec<u64> {
        let mut p = prev;
        let mut out = Vec::new();
        for (i, &l) in levels.iter().enumerate() {
            if l != p {
                out.push(i as u64);
            }
            p = l;
        }
        out
    }

    #[test]
    fn counting_and_extracting_agree_with_a_sample_by_sample_reference() {
        for (seed, density) in [(1, 2), (2, 7), (3, 100), (4, 1)] {
            let levels = pseudo_random(1000, seed, density);
            let bits = to_bits(&levels);
            for off in [0u64, 1, 63, 64, 65, 200] {
                for n in [0u64, 1, 5, 64, 129, 700] {
                    let slice = &levels[off as usize..(off + n) as usize];
                    for prev in [Level::Low, Level::High] {
                        let want = reference_transitions(slice, prev as u8);
                        let (c, last) = count_transitions(&bits, off, n, prev);
                        assert_eq!(c, want.len() as u64, "count off {off} n {n}");
                        let mut got = Vec::new();
                        let last2 = for_each_transition(&bits, off, n, prev, |p| got.push(p));
                        assert_eq!(got, want, "positions off {off} n {n}");
                        let want_last = slice.last().map_or(prev, |&l| Level::from_bit(u64::from(l)));
                        assert_eq!(last, want_last);
                        assert_eq!(last2, want_last);
                    }
                }
            }
        }
    }

    #[test]
    fn fill_and_copy_at_every_alignment() {
        let levels = pseudo_random(400, 9, 3);
        let src = to_bits(&levels);
        for dst_off in [0u64, 1, 31, 63, 64, 100] {
            for src_off in [0u64, 5, 64, 77] {
                for n in [0u64, 1, 63, 64, 65, 190] {
                    let mut dst = vec![0xAAAA_AAAA_AAAA_AAAAu64; 8];
                    let before: Vec<Level> = (0..512).map(|i| get(&dst, i)).collect();
                    copy(&mut dst, dst_off, &src, src_off, n);
                    for i in 0..512u64 {
                        let want = if i >= dst_off && i < dst_off + n {
                            Level::from_bit(u64::from(levels[(src_off + i - dst_off) as usize]))
                        } else {
                            before[i as usize]
                        };
                        assert_eq!(get(&dst, i), want, "copy dst {dst_off} src {src_off} n {n} bit {i}");
                    }
                }
            }
        }
        for (from, to) in [(0u64, 0u64), (3, 4), (0, 64), (5, 200), (64, 128), (63, 65)] {
            let mut dst = vec![0u64; 4];
            fill(&mut dst, from, to, Level::High);
            for i in 0..256u64 {
                assert_eq!(get(&dst, i).is_high(), i >= from && i < to, "fill {from}..{to} bit {i}");
            }
        }
    }

    #[test]
    fn window_reads_zero_past_the_end() {
        let src = [u64::MAX];
        assert_eq!(window(&src, 32), u64::MAX >> 32);
        assert_eq!(window(&src, 64), 0);
    }
}
