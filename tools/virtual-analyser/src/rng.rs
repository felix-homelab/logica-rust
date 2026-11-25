//! A small seeded random generator (SplitMix64): enough for test signals, and the same seed gives
//! the same signals on every machine.

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [lo, hi].
    pub fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Whole number in [lo, hi].
    pub fn int(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        let span = u64::from(hi - lo) + 1;
        lo + u32::try_from(self.next_u64() % span).unwrap_or(0)
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Exponentially distributed with the given mean (times between random events).
    pub fn exponential(&mut self, mean: f64) -> f64 {
        -mean * (1.0 - self.unit()).ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_hold_and_seeds_repeat() {
        let mut a = Rng::new(5);
        let mut b = Rng::new(5);
        for _ in 0..10_000 {
            let x = a.int(3, 7);
            assert!((3..=7).contains(&x));
            assert_eq!(x, b.int(3, 7));
            let f = a.between(1.0, 2.0);
            assert!((1.0..=2.0).contains(&f));
            b.between(1.0, 2.0);
            assert!(a.exponential(1.0) >= 0.0);
            b.exponential(1.0);
        }
        assert_eq!(a.int(9, 9), 9);
        assert_eq!(a.int(9, 2), 9, "an empty range gives its start");
        let mean = (0..20_000).map(|_| a.exponential(2.0)).sum::<f64>() / 20_000.0;
        assert!((1.9..2.1).contains(&mean), "{mean}");
        let hits = (0..20_000).filter(|_| a.chance(0.25)).count();
        assert!((4600..5400).contains(&hits), "{hits}");
    }
}
