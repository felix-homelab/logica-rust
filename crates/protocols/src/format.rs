//! Number formatting for explanations: "115.2 kHz", "8.68 µs", "115,200", "4F". The rules follow
//! Signal Detective's, so evidence texts read the same (four significant digits, trailing zeros
//! dropped).

/// Rounds a decimal digit string half up at `keep` digits (JavaScript's rule for `toFixed` and
/// `toPrecision`: of two equally near results, the larger magnitude). Returns the kept digits and
/// whether rounding carried into a new leading digit.
fn round_half_up(digits: &[u8], keep: usize) -> (Vec<u8>, bool) {
    let mut out: Vec<u8> = digits[..keep.min(digits.len())].to_vec();
    if digits.get(keep).is_some_and(|&d| d >= b'5') {
        let mut i = out.len();
        loop {
            if i == 0 {
                out.insert(0, b'1');
                return (out, true);
            }
            i -= 1;
            if out[i] == b'9' {
                out[i] = b'0';
            } else {
                out[i] += 1;
                break;
            }
        }
    }
    (out, false)
}

/// `x` with `digits` significant digits, trailing zeros dropped ("26.04", "300", "1"), rounded
/// like JavaScript's `toPrecision`.
pub fn sig(x: f64, digits: usize) -> String {
    if x == 0.0 || !x.is_finite() {
        return if x.is_finite() { "0".into() } else { "–".into() };
    }
    let digits = digits.max(1);
    // the exact decimal expansion, far past the digits kept
    let e = format!("{:.*e}", digits + 30, x.abs());
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let mut exp: i32 = exp.parse().unwrap_or(0);
    let all: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let (mut kept, carried) = round_half_up(&all, digits);
    if carried {
        exp += 1;
        kept.truncate(digits);
    }
    let (first, rest) = kept.split_at(1);
    let text = format!(
        "{}{}.{}e{exp}",
        if x < 0.0 { "-" } else { "" },
        String::from_utf8_lossy(first),
        String::from_utf8_lossy(rest)
    );
    // Display of f64 is the shortest exact form: "26.04", "300", "12350"
    text.parse::<f64>().map_or(text, |v| format!("{v}"))
}

/// `x` with `decimals` decimals ("30.0", "1.50"), rounded like JavaScript's `toFixed`.
pub fn fixed(x: f64, decimals: usize) -> String {
    if !x.is_finite() {
        return "–".into();
    }
    let exact = format!("{:.*}", decimals + 30, x.abs());
    let (int_part, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let all: Vec<u8> = int_part.bytes().chain(frac.bytes()).collect();
    let (kept, _) = round_half_up(&all, int_part.len() + decimals);
    let int_len = kept.len() - decimals;
    let mut out = String::new();
    if x < 0.0 && kept.iter().any(|&d| d != b'0') {
        out.push('-');
    }
    out.push_str(&String::from_utf8_lossy(&kept[..int_len]));
    if decimals > 0 {
        out.push('.');
        out.push_str(&String::from_utf8_lossy(&kept[int_len..]));
    }
    out
}

fn trim_zeros(s: String) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// A frequency with a sensible unit: "115.2 kHz", "1 MHz", "50 Hz".
pub fn hz(f: f64) -> String {
    if !f.is_finite() {
        return "–".into();
    }
    let a = f.abs();
    if a >= 1e6 {
        format!("{} MHz", trim_zeros(fixed(f / 1e6, if a >= 1e7 { 1 } else { 3 })))
    } else if a >= 1e3 {
        format!("{} kHz", trim_zeros(fixed(f / 1e3, if a >= 1e4 { 1 } else { 2 })))
    } else {
        format!("{} Hz", trim_zeros(fixed(f, 1)))
    }
}

/// A duration with four significant digits: "8.68 µs", "1.5 ms", "20 ns".
pub fn time(t: f64) -> String {
    if !t.is_finite() {
        return "–".into();
    }
    let a = t.abs();
    if a >= 1.0 {
        format!("{} s", sig(t, 4))
    } else if a >= 1e-3 {
        format!("{} ms", sig(t * 1e3, 4))
    } else if a >= 1e-6 {
        format!("{} µs", sig(t * 1e6, 4))
    } else {
        format!("{} ns", sig(t * 1e9, 4))
    }
}

/// "0x4F" style, at least `width` hex digits.
pub fn hex(n: u64, width: usize) -> String {
    format!("0x{n:0width$X}")
}

/// "4F" style.
pub fn hex2(n: u64) -> String {
    format!("{n:02X}")
}

/// A whole number with thousands separators: "115,200".
pub fn int(n: f64) -> String {
    let r = n.round();
    let neg = r < 0.0;
    let digits = format!("{:.0}", r.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

/// "30.0 %".
pub fn pct(x: f64, digits: usize) -> String {
    format!("{} %", fixed(x * 100.0, digits))
}

/// Printable form of one byte for text views.
pub fn char_of(b: u8) -> String {
    match b {
        0x0d => "\\r".into(),
        0x0a => "\\n".into(),
        0x09 => "\\t".into(),
        0x20..=0x7e => (b as char).to_string(),
        _ => "·".into(),
    }
}

pub fn as_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char_of(b)).collect()
}

/// "s" for plural counts.
pub fn s(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Capitalises the first letter.
pub fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_signal_detective() {
        assert_eq!(time(1.0 / 57600.0), "17.36 µs");
        assert_eq!(time(1.0 / 38400.0), "26.04 µs");
        assert_eq!(time(0.001), "1 ms");
        assert_eq!(time(0.0003), "300 µs");
        assert_eq!(time(2e-8), "20 ns");
        assert_eq!(time(1.5), "1.5 s");
        assert_eq!(hz(1000.0), "1 kHz");
        assert_eq!(hz(115_200.0), "115.2 kHz");
        assert_eq!(hz(100_000.0), "100 kHz");
        assert_eq!(hz(1e6), "1 MHz");
        assert_eq!(hz(12.5e6), "12.5 MHz");
        assert_eq!(hz(50.0), "50 Hz");
        assert_eq!(int(115_200.0), "115,200");
        assert_eq!(int(999.0), "999");
        assert_eq!(int(1_234_567.4), "1,234,567");
        assert_eq!(hex(0x27, 2), "0x27");
        assert_eq!(hex2(0x4f), "4F");
        assert_eq!(pct(0.3, 1), "30.0 %");
        assert_eq!(as_text(b"OK\r\n\x01"), "OK\\r\\n·");
        assert_eq!(sig(12345.0, 4), "12350");
        assert_eq!(sig(9.9996, 4), "10");
        assert_eq!(sig(-0.00012345, 3), "-0.000123");
        assert_eq!(fixed(0.25, 1), "0.3");
        assert_eq!(fixed(2.5, 0), "3");
        assert_eq!(fixed(9.96, 1), "10.0");
        assert_eq!(fixed(-1.25, 1), "-1.3");
        assert_eq!(fixed(1.0 / 3.0, 3), "0.333");
    }
}
