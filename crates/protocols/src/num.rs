//! Explicit conversions from computed sample positions (floats) to sample indices.

use logica_core::Sample;

/// `x` truncated toward zero, 0 for negative or not-a-number values, saturating at the top.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn to_u64(x: f64) -> u64 {
    if x.is_nan() || x <= 0.0 { 0 } else { x as u64 }
}

pub fn floor(x: f64) -> Sample {
    to_u64(x.floor())
}

/// Rounds half up, like JavaScript's `Math.round` for positive values.
pub fn round(x: f64) -> Sample {
    to_u64((x + 0.5).floor())
}

pub fn ceil(x: f64) -> Sample {
    to_u64(x.ceil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!((floor(2.9), round(2.5), round(2.49), ceil(2.01)), (2, 3, 2, 3));
        assert_eq!((floor(-3.0), to_u64(f64::NAN), to_u64(1e30)), (0, 0, u64::MAX));
    }
}
