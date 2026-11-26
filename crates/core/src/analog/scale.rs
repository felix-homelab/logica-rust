//! Scaling of virtual analog channels (ADR 0010, doc 06): from what a source measured to the value
//! a person reads.
//!
//! A source emits either a **fraction of full scale** (an ADC code over 2^N, a PWM duty, a PDM
//! density) or a **measured quantity** (a frequency, a period). Scaling is one linear step for
//! both, so every source shares it and a new source needs no scaling of its own.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The widest code a source can deliver: 32 bits (a parallel bus of 32 channels, a 24-bit ADC).
pub const MAX_BITS: u32 = 32;

/// How a raw ADC code stands for a fraction of full scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Encoding {
    /// Straight binary: 0 is the bottom of the range. Offset binary is this with a negative `low`.
    #[default]
    Unsigned,
    /// Two's complement: the top bit is the sign. The most negative code is the bottom of the
    /// range and 0 its middle, so `low` and `high` are the ends of a bipolar range.
    TwosComplement,
}

/// What a source's points mean, so the scaling that fits them can be checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PointKind {
    /// A fraction of full scale: an ADC code, a duty cycle, a bit density.
    Fraction,
    /// A measured quantity with a unit of its own: a frequency, a period, a pulse width.
    Quantity,
}

/// The linear step from a source's points to values in the channel's unit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scaling {
    /// For fractions: `low + fraction × (high − low) + offset`. `low` and `high` are the ends of
    /// the source's full scale (for an ADC its reference voltages); the offset is a calibration
    /// trim. Following ADC datasheets, the highest code reads one step below `high`.
    Range {
        low: f64,
        high: f64,
        #[serde(default)]
        offset: f64,
    },
    /// For measured quantities: `quantity × gain + offset`.
    Linear {
        gain: f64,
        #[serde(default)]
        offset: f64,
    },
}

/// Why a scaling or a code cannot be used. The messages say what to change.
#[derive(Clone, Debug, PartialEq)]
pub enum ScaleError {
    NotFinite,
    EmptyRange,
    ZeroGain,
    WrongKind { scaling: PointKind },
    Bits(u32),
    CodeTooWide { code: u64, bits: u32 },
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScaleError::NotFinite => {
                write!(f, "The scaling needs finite numbers; a value is infinite or not a number.")
            }
            ScaleError::EmptyRange => write!(
                f,
                "The lowest and highest value are the same, so every code would read the same \
                 value. Give the two ends of the source's full scale."
            ),
            ScaleError::ZeroGain => write!(
                f,
                "A gain of 0 turns every measurement into the offset. Give the value per unit of \
                 the measurement."
            ),
            ScaleError::WrongKind { scaling: PointKind::Fraction } => write!(
                f,
                "This source measures a quantity such as a frequency or a time, which has no \
                 lowest and highest value. Scale it with a gain and an offset."
            ),
            ScaleError::WrongKind { scaling: PointKind::Quantity } => write!(
                f,
                "This source gives a fraction of its full scale. Scale it with the lowest and \
                 highest value and an offset, not with a gain."
            ),
            ScaleError::Bits(bits) => {
                write!(f, "A code has 1 to {MAX_BITS} bits; {bits} bits is not possible.")
            }
            ScaleError::CodeTooWide { code, bits } => {
                write!(f, "The code {code} does not fit in {bits} bits.")
            }
        }
    }
}

impl std::error::Error for ScaleError {}

fn check_bits(bits: u32) -> Result<f64, ScaleError> {
    if (1..=MAX_BITS).contains(&bits) { Ok(f64::from(bits).exp2()) } else { Err(ScaleError::Bits(bits)) }
}

/// The fraction of full scale, from 0 to just under 1, that an `bits`-bit code stands for. For
/// two's complement the most negative code is 0 and the code 0 is one half.
pub fn fraction_of_code(code: u64, bits: u32, encoding: Encoding) -> Result<f64, ScaleError> {
    let full = check_bits(bits)?;
    if code >> bits != 0 {
        return Err(ScaleError::CodeTooWide { code, bits });
    }
    // Codes have at most 32 bits, so they are exact as f64.
    #[allow(clippy::cast_precision_loss)]
    let mut c = code as f64;
    if encoding == Encoding::TwosComplement {
        c += if code >> (bits - 1) != 0 { -full / 2.0 } else { full / 2.0 };
    }
    Ok(c / full)
}

/// The code a fraction stands for (signed for two's complement), the inverse of
/// [`fraction_of_code`]; `None` when the fraction is outside the code's range.
pub fn code_of_fraction(fraction: f64, bits: u32, encoding: Encoding) -> Option<i64> {
    let full = check_bits(bits).ok()?;
    let (shift, min, max) = match encoding {
        Encoding::Unsigned => (0.0, 0.0, full - 1.0),
        Encoding::TwosComplement => (full / 2.0, -full / 2.0, full / 2.0 - 1.0),
    };
    let c = (fraction * full - shift).round();
    // In range, `c` is a whole number of at most 32 bits.
    #[allow(clippy::cast_possible_truncation)]
    (c >= min && c <= max).then_some(c as i64)
}

impl Scaling {
    /// The kind of point this scaling is for.
    pub fn kind(&self) -> PointKind {
        match self {
            Scaling::Range { .. } => PointKind::Fraction,
            Scaling::Linear { .. } => PointKind::Quantity,
        }
    }

    /// Checks that the numbers make a usable scaling for points of `kind`.
    pub fn validate(&self, kind: PointKind) -> Result<(), ScaleError> {
        let numbers: &[f64] = match self {
            Scaling::Range { low, high, offset } => &[*low, *high, *offset],
            Scaling::Linear { gain, offset } => &[*gain, *offset],
        };
        if numbers.iter().any(|n| !n.is_finite()) {
            return Err(ScaleError::NotFinite);
        }
        match *self {
            Scaling::Range { low, high, .. } if low == high => return Err(ScaleError::EmptyRange),
            Scaling::Linear { gain: 0.0, .. } => return Err(ScaleError::ZeroGain),
            _ => {}
        }
        if self.kind() != kind {
            return Err(ScaleError::WrongKind { scaling: self.kind() });
        }
        Ok(())
    }

    fn slope(&self) -> f64 {
        match *self {
            Scaling::Range { low, high, .. } => high - low,
            Scaling::Linear { gain, .. } => gain,
        }
    }

    fn intercept(&self) -> f64 {
        match *self {
            Scaling::Range { low, offset, .. } => low + offset,
            Scaling::Linear { offset, .. } => offset,
        }
    }

    /// The value a point stands for.
    pub fn apply(&self, x: f64) -> f64 {
        self.intercept() + x * self.slope()
    }

    /// The values of a range of points as `(lowest, highest)`; a falling scaling swaps the ends.
    pub fn apply_range(&self, min: f64, max: f64) -> (f64, f64) {
        let (a, b) = (self.apply(min), self.apply(max));
        if a <= b { (a, b) } else { (b, a) }
    }

    /// One step of an `bits`-bit code in the channel's unit (always positive); `None` for a
    /// measured quantity or an impossible width.
    pub fn step(&self, bits: u32) -> Option<f64> {
        let full = check_bits(bits).ok()?;
        match self {
            Scaling::Range { .. } => Some(self.slope().abs() / full),
            Scaling::Linear { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V33: Scaling = Scaling::Range { low: 0.0, high: 3.3, offset: 0.0 };

    #[test]
    fn unsigned_codes_cover_zero_to_just_under_one() {
        let f = |c| fraction_of_code(c, 12, Encoding::Unsigned).unwrap();
        assert_eq!(f(0), 0.0);
        assert_eq!(f(2048), 0.5);
        assert_eq!(f(4095), 4095.0 / 4096.0);
    }

    #[test]
    fn twos_complement_codes_put_zero_in_the_middle() {
        let f = |c| fraction_of_code(c, 12, Encoding::TwosComplement).unwrap();
        assert_eq!(f(0x800), 0.0);
        assert_eq!(f(0xFFF), 2047.0 / 4096.0);
        assert_eq!(f(0), 0.5);
        assert_eq!(f(0x7FF), 4095.0 / 4096.0);
    }

    #[test]
    fn widths_at_both_ends() {
        assert_eq!(fraction_of_code(1, 1, Encoding::Unsigned).unwrap(), 0.5);
        assert_eq!(fraction_of_code(1, 1, Encoding::TwosComplement).unwrap(), 0.0);
        assert_eq!(fraction_of_code(0, 1, Encoding::TwosComplement).unwrap(), 0.5);
        let top = u64::from(u32::MAX);
        let just_under_one = (top as f64) / 2f64.powi(32);
        assert_eq!(fraction_of_code(top, 32, Encoding::Unsigned).unwrap(), just_under_one);
        assert_eq!(fraction_of_code(top, 32, Encoding::TwosComplement).unwrap(), 0.5 - 1.0 / 2f64.powi(32));
    }

    #[test]
    fn impossible_widths_and_codes_are_refused_with_a_reason() {
        assert_eq!(fraction_of_code(0, 0, Encoding::Unsigned), Err(ScaleError::Bits(0)));
        assert_eq!(fraction_of_code(0, 33, Encoding::Unsigned), Err(ScaleError::Bits(33)));
        let e = fraction_of_code(4096, 12, Encoding::Unsigned).unwrap_err();
        assert_eq!(e, ScaleError::CodeTooWide { code: 4096, bits: 12 });
        assert_eq!(e.to_string(), "The code 4096 does not fit in 12 bits.");
    }

    #[test]
    fn codes_round_trip_through_fractions() {
        for bits in 1..=10 {
            for enc in [Encoding::Unsigned, Encoding::TwosComplement] {
                for code in 0..1u64 << bits {
                    let f = fraction_of_code(code, bits, enc).unwrap();
                    let back = code_of_fraction(f, bits, enc).unwrap();
                    let expect = match enc {
                        Encoding::Unsigned => code as i64,
                        Encoding::TwosComplement if code >> (bits - 1) != 0 => code as i64 - (1 << bits),
                        Encoding::TwosComplement => code as i64,
                    };
                    assert_eq!(back, expect, "{bits} bits, {enc:?}, code {code}");
                }
            }
        }
    }

    #[test]
    fn fractions_outside_the_code_range_have_no_code() {
        assert_eq!(code_of_fraction(1.0, 8, Encoding::Unsigned), None);
        assert_eq!(code_of_fraction(-0.01, 8, Encoding::Unsigned), None);
        assert_eq!(code_of_fraction(1.0, 8, Encoding::TwosComplement), None);
        assert_eq!(code_of_fraction(-0.01, 8, Encoding::TwosComplement), None);
        assert_eq!(code_of_fraction(0.0, 8, Encoding::TwosComplement), Some(-128));
        assert_eq!(code_of_fraction(0.5, 8, Encoding::TwosComplement), Some(0));
        assert_eq!(code_of_fraction(0.5, 0, Encoding::Unsigned), None);
    }

    #[test]
    fn a_range_maps_codes_like_the_datasheet_with_a_calibration_trim() {
        let s = Scaling::Range { low: 0.0, high: 3.3, offset: -0.02 };
        let x = fraction_of_code(2048, 12, Encoding::Unsigned).unwrap();
        assert!((s.apply(x) - 1.63).abs() < 1e-12);
        assert!((s.step(12).unwrap() - 3.3 / 4096.0).abs() < 1e-15);
        // The highest code reads one step below `high`.
        let top = fraction_of_code(4095, 12, Encoding::Unsigned).unwrap();
        assert!((V33.apply(top) - (3.3 - 3.3 / 4096.0)).abs() < 1e-12);
    }

    #[test]
    fn a_bipolar_range_with_twos_complement() {
        let s = Scaling::Range { low: -10.0, high: 10.0, offset: 0.0 };
        let v = |c| s.apply(fraction_of_code(c, 16, Encoding::TwosComplement).unwrap());
        assert_eq!(v(0x8000), -10.0);
        assert_eq!(v(0), 0.0);
        assert_eq!(v(0xFFFF), -20.0 / 65536.0);
        assert_eq!(v(0x7FFF), 10.0 - 20.0 / 65536.0);
    }

    #[test]
    fn pwm_duty_in_per_cent() {
        let s = Scaling::Range { low: 0.0, high: 100.0, offset: 0.0 };
        assert_eq!(s.apply(0.25), 25.0);
        assert_eq!(s.step(0), None);
    }

    #[test]
    fn a_linear_scaling_for_measured_quantities() {
        // A flow sensor giving 7.5 Hz per litre per minute.
        let s = Scaling::Linear { gain: 1.0 / 7.5, offset: 0.0 };
        assert!((s.apply(75.0) - 10.0).abs() < 1e-12);
        assert_eq!(s.step(12), None);
        assert_eq!(s.validate(PointKind::Quantity), Ok(()));
    }

    #[test]
    fn a_falling_scaling_keeps_lowest_first() {
        let s = Scaling::Range { low: 5.0, high: 0.0, offset: 0.0 };
        assert_eq!(s.apply_range(0.0, 0.5), (2.5, 5.0));
        assert_eq!(s.step(1), Some(2.5));
        let g = Scaling::Linear { gain: -2.0, offset: 1.0 };
        assert_eq!(g.apply_range(1.0, 2.0), (-3.0, -1.0));
    }

    #[test]
    fn unusable_scalings_say_why() {
        let r = |low, high, offset| Scaling::Range { low, high, offset }.validate(PointKind::Fraction);
        assert_eq!(r(1.0, 1.0, 0.0), Err(ScaleError::EmptyRange));
        assert_eq!(r(0.0, f64::INFINITY, 0.0), Err(ScaleError::NotFinite));
        assert_eq!(r(0.0, 1.0, f64::NAN), Err(ScaleError::NotFinite));
        let zero = Scaling::Linear { gain: 0.0, offset: 1.0 };
        assert_eq!(zero.validate(PointKind::Quantity), Err(ScaleError::ZeroGain));
        let e = V33.validate(PointKind::Quantity).unwrap_err();
        assert_eq!(e, ScaleError::WrongKind { scaling: PointKind::Fraction });
        assert!(e.to_string().contains("gain and an offset"), "{e}");
        let e = Scaling::Linear { gain: 1.0, offset: 0.0 }.validate(PointKind::Fraction).unwrap_err();
        assert!(e.to_string().contains("lowest and highest"), "{e}");
    }

    #[test]
    fn serialised_shape() {
        let json = serde_json::to_value(V33).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "range", "low": 0.0, "high": 3.3, "offset": 0.0 }));
        let s: Scaling = serde_json::from_str(r#"{ "kind": "linear", "gain": 2 }"#).unwrap();
        assert_eq!(s, Scaling::Linear { gain: 2.0, offset: 0.0 });
        let e: Encoding = serde_json::from_str(r#""twosComplement""#).unwrap();
        assert_eq!(e, Encoding::TwosComplement);
    }
}
