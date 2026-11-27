//! Virtual analog values as CSV (doc 09): one table, a row wherever any chosen channel has a new
//! value, every column holding its value until the next one; a gap is an empty cell.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::analog::{PointKind, code_of_fraction};

use crate::analog::Analog;
use crate::contract::{CommandError, ErrorCode};
use crate::engine::{Ctx, Done, Engine, err};

use super::out;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalogExportIn {
    pub capture: String,
    /// Where to write the CSV file, on the machine running the engine.
    pub path: String,
    /// The channels, in this order; all of the capture's when absent.
    pub analogs: Option<Vec<String>>,
    /// The range; the whole capture when absent.
    pub from: Option<Sample>,
    /// Exclusive.
    pub to: Option<Sample>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AnalogExportOut {
    pub path: String,
    /// Rows of values, without the header.
    pub rows: u64,
    pub bytes: u64,
}

/// A CSV field: quoted when it holds a comma, a quote or a line break.
fn field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// The CSV text of `channels` over `from..to`, written to `w`; returns the rows written.
fn write_csv(
    w: &mut impl Write,
    channels: &[&Analog],
    rate: f64,
    from: Sample,
    to: Sample,
) -> std::io::Result<u64> {
    let codes: Vec<Option<u32>> = channels
        .iter()
        .map(|a| match a.output.kind {
            PointKind::Fraction => a.output.bits,
            PointKind::Quantity => None,
        })
        .collect();
    let mut header = vec!["time_s".to_string(), "sample".to_string()];
    for (a, code) in channels.iter().zip(&codes) {
        let unit = if a.config.unit.is_empty() { String::new() } else { format!(" ({})", a.config.unit) };
        header.push(field(&format!("{}{unit}", a.config.name)));
        if code.is_some() {
            header.push(field(&format!("{} code", a.config.name)));
        }
    }
    writeln!(w, "{}", header.join(","))?;
    let mut iters: Vec<_> = channels.iter().map(|a| a.series.points(from, to).peekable()).collect();
    let mut current: Vec<Option<f64>> = channels.iter().map(|a| a.series.held_before(from)).collect();
    let mut rows = 0;
    let mut row = |w: &mut dyn Write, s: Sample, current: &[Option<f64>]| -> std::io::Result<()> {
        // sample indices stay below 2^53, so the time is exact enough as f64
        #[allow(clippy::cast_precision_loss)]
        let t = s as f64 / rate;
        let mut cells = vec![format!("{t}"), s.to_string()];
        for ((a, x), code) in channels.iter().zip(current).zip(&codes) {
            cells.push(x.map(|x| a.config.scaling.apply(x).to_string()).unwrap_or_default());
            if let Some(bits) = code {
                cells.push(
                    x.and_then(|x| code_of_fraction(x, *bits, a.output.encoding))
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                );
            }
        }
        rows += 1;
        writeln!(w, "{}", cells.join(","))
    };
    // a first row at the start when a value is held into the range
    let starts_at_from = iters.iter_mut().any(|i| i.peek().is_some_and(|p| p.sample == from));
    if !starts_at_from && current.iter().any(Option::is_some) {
        row(w, from, &current)?;
    }
    while let Some(s) = iters.iter_mut().filter_map(|i| i.peek().map(|p| p.sample)).min() {
        for (k, i) in iters.iter_mut().enumerate() {
            if let Some(p) = i.next_if(|p| p.sample == s) {
                current[k] = p.x;
            }
        }
        row(w, s, &current)?;
    }
    Ok(rows)
}

pub(crate) fn export(e: &mut Engine, _: &Ctx, i: AnalogExportIn) -> Result<Done, CommandError> {
    let c = e.capture(&i.capture)?;
    let ids: Vec<String> = i.analogs.clone().unwrap_or_else(|| c.analog.keys().cloned().collect());
    if ids.is_empty() {
        return Err(err(
            ErrorCode::InvalidInput,
            "This capture has no virtual analog channel to export; add one first.",
        ));
    }
    let channels: Vec<&Analog> = ids
        .iter()
        .map(|id| {
            c.analog.get(id).ok_or_else(|| {
                err(
                    ErrorCode::NotFound,
                    format!("There is no virtual analog channel \"{id}\" on this capture."),
                )
            })
        })
        .collect::<Result<_, _>>()?;
    let from = i.from.unwrap_or(0);
    let to = i.to.unwrap_or(c.store.sample_count());
    if to <= from {
        return Err(err(ErrorCode::InvalidInput, "The range is empty: `to` must be after `from`."));
    }
    let path = PathBuf::from(&i.path);
    let shown = path.display().to_string();
    let fail = |x: std::io::Error| err(ErrorCode::Failed, format!("{shown} could not be written: {x}."));
    let file = File::create(&path).map_err(fail)?;
    let mut w = BufWriter::new(file);
    let rows = write_csv(&mut w, &channels, c.meta.sample_rate_hz, from, to).map_err(fail)?;
    w.flush().map_err(fail)?;
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Done::never(
        out(&AnalogExportOut { path: shown.clone(), rows, bytes }),
        format!("exported {rows} rows of virtual analog values to {shown}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(field("Fan (%)"), "Fan (%)");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
