//! Small calculations shared by detectors: standard baud rates and SPI modes.

pub const STANDARD_BAUDS: [f64; 19] = [
    300.0, 600.0, 1200.0, 2400.0, 4800.0, 9600.0, 14400.0, 19200.0, 28800.0, 38400.0, 57600.0, 115200.0,
    230400.0, 250000.0, 460800.0, 500000.0, 921600.0, 1000000.0, 2000000.0,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaudMatch {
    pub measured: f64,
    pub standard: f64,
    /// Signed error from the standard rate, in percent.
    pub error_pct: f64,
    /// Within the ±3 % a UART tolerates.
    pub within_tolerance: bool,
}

pub fn nearest_baud(baud: f64) -> BaudMatch {
    let mut best = STANDARD_BAUDS[0];
    for &x in &STANDARD_BAUDS {
        if (x / baud).ln().abs() < (best / baud).ln().abs() {
            best = x;
        }
    }
    let error_pct = (baud - best) / best * 100.0;
    BaudMatch { measured: baud, standard: best, error_pct, within_tolerance: error_pct.abs() < 3.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockEdge {
    Rising,
    Falling,
}

impl ClockEdge {
    pub fn name(self) -> &'static str {
        match self {
            ClockEdge::Rising => "rising",
            ClockEdge::Falling => "falling",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiMode {
    pub mode: u8,
    pub cpol: u8,
    pub cpha: u8,
    pub sample_edge: ClockEdge,
}

/// The mode from the clock rest level and the edge on which the data lines change.
pub fn spi_mode(cpol: u8, changes_on: ClockEdge) -> SpiMode {
    let sample_edge = if changes_on == ClockEdge::Falling { ClockEdge::Rising } else { ClockEdge::Falling };
    let leading = if cpol == 1 { ClockEdge::Falling } else { ClockEdge::Rising };
    let cpha = u8::from(sample_edge != leading);
    SpiMode { mode: cpol * 2 + cpha, cpol, cpha, sample_edge }
}

/// Clock rest level and sampling edge of a mode; the inverse of [`spi_mode`].
pub fn spi_mode_parts(mode: u8) -> SpiMode {
    let cpol = (mode >> 1) & 1;
    let cpha = mode & 1;
    let sample_edge = if (cpol == 0) == (cpha == 0) { ClockEdge::Rising } else { ClockEdge::Falling };
    SpiMode { mode: mode & 3, cpol, cpha, sample_edge }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bauds_and_modes() {
        let m = nearest_baud(57_870.0);
        assert_eq!(m.standard, 57_600.0);
        assert!(m.within_tolerance);
        assert!(!nearest_baud(70_000.0).within_tolerance);
        for mode in 0..4 {
            let p = spi_mode_parts(mode);
            let changes =
                if p.sample_edge == ClockEdge::Rising { ClockEdge::Falling } else { ClockEdge::Rising };
            assert_eq!(spi_mode(p.cpol, changes).mode, mode);
        }
        assert_eq!(spi_mode_parts(1).sample_edge, ClockEdge::Falling);
        assert_eq!(spi_mode_parts(3).sample_edge, ClockEdge::Rising);
    }
}
