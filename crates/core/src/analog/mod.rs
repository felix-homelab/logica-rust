//! Virtual analog channels (ADR 0010): the shared scaling and the value series. The sources that
//! produce points are streaming decoders in `logica-protocols` (doc 06).

pub mod scale;
pub mod series;

pub use scale::{Encoding, PointKind, ScaleError, Scaling, code_of_fraction, fraction_of_code};
pub use series::{Envelope, Point, SeriesError, SeriesStats, ValueSeries};
