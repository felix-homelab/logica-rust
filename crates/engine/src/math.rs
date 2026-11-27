//! Math channels (ADR 0010, doc 06): virtual analog channels computed from other ones with a typed
//! expression. Inputs sampled at different moments are combined by sample and hold: a value
//! wherever any input has a new one, every input holding its last value, and none before every
//! input has its first. A math channel reads other channels, not edges, so the engine computes it,
//! again whenever an input changes, in the order the channels depend on each other.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;
use logica_core::analog::ValueSeries;
use logica_protocols::types::{DecodeResult, Frame, FrameKind, ProblemCode, problem};

use crate::analog::Analog;
use crate::state::Capture;

/// The source id of math channels.
pub const MATH: &str = "math";

/// The deepest an expression may nest.
const MAX_DEPTH: usize = 32;

/// A formula over virtual analog channels, in their units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Expr {
    /// Another virtual analog channel's value, by its id.
    Channel(String),
    Constant(f64),
    /// The sum of two or more.
    Add(Vec<Expr>),
    /// The first minus the second.
    Sub(Vec<Expr>),
    /// The product of two or more.
    Mul(Vec<Expr>),
    /// The first divided by the second; no value where the second is 0.
    Div(Vec<Expr>),
    Min(Vec<Expr>),
    Max(Vec<Expr>),
    Abs(Box<Expr>),
    Neg(Box<Expr>),
}

impl Expr {
    /// Checks the shape: operand counts, finite constants, depth, at least one channel.
    pub fn check(&self) -> Result<(), String> {
        fn walk(e: &Expr, depth: usize) -> Result<(), String> {
            if depth > MAX_DEPTH {
                return Err(format!("The formula nests deeper than {MAX_DEPTH} levels."));
            }
            let (name, list, exact) = match e {
                Expr::Channel(id) if id.is_empty() => {
                    return Err("A channel in the formula has no id.".into());
                }
                Expr::Channel(_) => return Ok(()),
                Expr::Constant(c) if c.is_finite() => return Ok(()),
                Expr::Constant(_) => {
                    return Err("A number in the formula is infinite or not a number.".into());
                }
                Expr::Abs(x) | Expr::Neg(x) => return walk(x, depth + 1),
                Expr::Sub(l) => ("sub", l, true),
                Expr::Div(l) => ("div", l, true),
                Expr::Add(l) => ("add", l, false),
                Expr::Mul(l) => ("mul", l, false),
                Expr::Min(l) => ("min", l, false),
                Expr::Max(l) => ("max", l, false),
            };
            if exact && list.len() != 2 {
                return Err(format!("{name} takes exactly two parts; it has {}.", list.len()));
            }
            if list.len() < 2 {
                return Err(format!("{name} takes two or more parts; it has {}.", list.len()));
            }
            list.iter().try_for_each(|x| walk(x, depth + 1))
        }
        walk(self, 0)?;
        if self.channels().is_empty() {
            return Err("The formula reads no channel; a math channel needs at least one.".into());
        }
        Ok(())
    }

    /// The channel ids it reads.
    pub fn channels(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        fn walk(e: &Expr, out: &mut BTreeSet<String>) {
            match e {
                Expr::Channel(id) => {
                    out.insert(id.clone());
                }
                Expr::Constant(_) => {}
                Expr::Abs(x) | Expr::Neg(x) => walk(x, out),
                Expr::Add(l) | Expr::Sub(l) | Expr::Mul(l) | Expr::Div(l) | Expr::Min(l) | Expr::Max(l) => {
                    l.iter().for_each(|x| walk(x, out));
                }
            }
        }
        walk(self, &mut out);
        out
    }

    /// The value with `value` giving each channel's; `Ok(None)` where an input has none, an error
    /// that says why where there is no value (a division by zero).
    pub fn eval(&self, value: &impl Fn(&str) -> Option<f64>) -> Result<Option<f64>, String> {
        let all = |l: &[Expr]| -> Result<Option<Vec<f64>>, String> {
            let mut v = Vec::with_capacity(l.len());
            for x in l {
                match x.eval(value)? {
                    Some(y) => v.push(y),
                    None => return Ok(None),
                }
            }
            Ok(Some(v))
        };
        Ok(match self {
            Expr::Channel(id) => value(id),
            Expr::Constant(c) => Some(*c),
            Expr::Abs(x) => x.eval(value)?.map(f64::abs),
            Expr::Neg(x) => x.eval(value)?.map(|y| -y),
            Expr::Add(l) => all(l)?.map(|v| v.iter().sum()),
            Expr::Mul(l) => all(l)?.map(|v| v.iter().product()),
            Expr::Min(l) => all(l)?.map(|v| v.iter().copied().fold(f64::INFINITY, f64::min)),
            Expr::Max(l) => all(l)?.map(|v| v.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            Expr::Sub(l) => all(l)?.map(|v| v[0] - v[1]),
            Expr::Div(l) => match all(l)? {
                Some(v) if v[1] == 0.0 => return Err("Division by zero: the divisor is 0 here.".into()),
                Some(v) => Some(v[0] / v[1]),
                None => None,
            },
        })
    }
}

/// The chain through which `id` would read itself with `expr`, such as ["analog-4", "analog-3"].
pub fn cycle(c: &Capture, id: &str, expr: &Expr) -> Option<Vec<String>> {
    fn reach(c: &Capture, from: &str, id: &str, seen: &mut BTreeSet<String>, path: &mut Vec<String>) -> bool {
        if from == id {
            return true;
        }
        if !seen.insert(from.to_string()) {
            return false;
        }
        path.push(from.to_string());
        if let Some(e) = c.analog.get(from).and_then(|a| a.config.expression.as_ref()) {
            for next in e.channels() {
                if reach(c, &next, id, seen, path) {
                    return true;
                }
            }
        }
        path.pop();
        false
    }
    let mut seen = BTreeSet::new();
    for start in expr.channels() {
        let mut path = Vec::new();
        if reach(c, &start, id, &mut seen, &mut path) {
            return Some(path);
        }
    }
    None
}

/// The series of a math channel from its inputs as they are now.
fn compute(c: &Capture, expr: &Expr) -> (ValueSeries, DecodeResult) {
    let mut series = ValueSeries::new();
    let mut result = DecodeResult::default();
    let ids: Vec<String> = expr.channels().into_iter().collect();
    let missing: Vec<&String> = ids.iter().filter(|id| !c.analog.contains_key(*id)).collect();
    if !missing.is_empty() {
        let list: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
        result.notes.push(format!(
            "The formula reads {}, which this capture does not have (removed?); there are no values until it is back.",
            list.join(", ")
        ));
        return (series, result);
    }
    let inputs: Vec<&Analog> = ids.iter().map(|id| &c.analog[id]).collect();
    let mut iters: Vec<_> = inputs.iter().map(|a| a.series.points(0, Sample::MAX).peekable()).collect();
    let mut current: Vec<Option<f64>> = vec![None; inputs.len()];
    let mut open: Option<(Sample, String)> = None;
    let mut values = 0u64;
    let problem_frame = |start: Sample, end: Sample, message: String, result: &mut DecodeResult| {
        *result.problem_counts.entry(ProblemCode::Protocol).or_default() += 1;
        result.frames.push(Frame {
            start,
            end,
            lane: 0,
            kind: FrameKind::Info,
            value: None,
            label: "no value".into(),
            detail: None,
            problems: vec![problem(ProblemCode::Protocol, message)],
            packet: 0,
            row: 0,
        });
    };
    while let Some(s) = iters.iter_mut().filter_map(|i| i.peek().map(|p| p.sample)).min() {
        for (k, i) in iters.iter_mut().enumerate() {
            if let Some(p) = i.next_if(|p| p.sample == s) {
                current[k] = p.x.map(|x| inputs[k].config.scaling.apply(x));
            }
        }
        if let Some((start, message)) = open.take() {
            problem_frame(start, s, message, &mut result);
        }
        let lookup = |id: &str| ids.iter().position(|x| x == id).and_then(|k| current[k]);
        match expr.eval(&lookup) {
            Ok(Some(v)) if v.is_finite() => {
                // in order and finite, so pushing cannot fail
                let _ = series.push(s, v);
                values += 1;
            }
            Ok(_) => {
                let _ = series.gap(s);
            }
            Err(message) => {
                let _ = series.gap(s);
                open = Some((s, message));
            }
        }
    }
    if let Some((start, message)) = open {
        problem_frame(start, start + 1, message, &mut result);
    }
    let names: Vec<&str> = inputs.iter().map(|a| a.config.name.as_str()).collect();
    result.notes.push(format!("{values} values from {}.", names.join(", ")));
    (series, result)
}

/// The math channels of `c` in an order where every channel comes after the ones it reads.
fn order(c: &Capture) -> Vec<String> {
    let math: BTreeMap<&String, BTreeSet<String>> = c
        .analog
        .iter()
        .filter_map(|(id, a)| a.config.expression.as_ref().map(|e| (id, e.channels())))
        .collect();
    let mut done: Vec<String> = Vec::new();
    let mut left: Vec<&String> = math.keys().copied().collect();
    while !left.is_empty() {
        let before = left.len();
        left.retain(|id| {
            let ready = math[*id].iter().all(|d| !math.contains_key(d) || done.contains(d));
            if ready {
                done.push((*id).clone());
            }
            !ready
        });
        if left.len() == before {
            // a cycle (only from a damaged file): computed in name order, each says so
            done.extend(left.drain(..).cloned());
        }
    }
    done
}

/// Computes every math channel of `c` again from its inputs; returns their ids.
pub fn refresh(c: &mut Capture) -> Vec<String> {
    let ids = order(c);
    for id in &ids {
        let Some(expr) = c.analog.get(id).and_then(|a| a.config.expression.clone()) else { continue };
        let (series, mut result) = compute(c, &expr);
        if cycle(c, id, &expr).is_some() {
            result.notes.push(
                "The formula reads this channel itself, through other channels; change one of them.".into(),
            );
        }
        if let Some(a) = c.analog.get_mut(id) {
            a.series = series;
            a.result = result;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(id: &str) -> Expr {
        Expr::Channel(id.into())
    }

    #[test]
    fn expressions_have_a_shape_that_is_checked() {
        assert_eq!(Expr::Sub(vec![ch("a"), ch("b")]).check(), Ok(()));
        assert_eq!(Expr::Sub(vec![ch("a")]).check().unwrap_err(), "sub takes exactly two parts; it has 1.");
        assert_eq!(Expr::Add(vec![ch("a")]).check().unwrap_err(), "add takes two or more parts; it has 1.");
        assert!(Expr::Constant(f64::NAN).check().is_err());
        assert!(
            Expr::Add(vec![Expr::Constant(1.0), Expr::Constant(2.0)])
                .check()
                .unwrap_err()
                .contains("reads no channel")
        );
        let mut deep = ch("a");
        for _ in 0..40 {
            deep = Expr::Neg(Box::new(deep));
        }
        assert!(deep.check().unwrap_err().contains("deeper than 32"));
        assert!(ch("").check().is_err());
    }

    #[test]
    fn evaluation_with_missing_values_and_division_by_zero() {
        let vals = |id: &str| match id {
            "a" => Some(3.0),
            "b" => Some(-1.0),
            "z" => Some(0.0),
            _ => None,
        };
        let e = |x: Expr| x.eval(&vals);
        assert_eq!(e(Expr::Sub(vec![ch("a"), ch("b")])), Ok(Some(4.0)));
        assert_eq!(e(Expr::Mul(vec![ch("a"), Expr::Constant(2.0), ch("b")])), Ok(Some(-6.0)));
        assert_eq!(e(Expr::Abs(Box::new(ch("b")))), Ok(Some(1.0)));
        assert_eq!(e(Expr::Max(vec![ch("a"), ch("b"), Expr::Constant(5.0)])), Ok(Some(5.0)));
        assert_eq!(e(Expr::Min(vec![ch("a"), ch("b")])), Ok(Some(-1.0)));
        assert_eq!(e(Expr::Neg(Box::new(ch("a")))), Ok(Some(-3.0)));
        assert_eq!(e(Expr::Div(vec![ch("a"), Expr::Constant(2.0)])), Ok(Some(1.5)));
        // an input without a value gives none; a zero divisor says why
        assert_eq!(e(Expr::Add(vec![ch("a"), ch("missing")])), Ok(None));
        assert!(e(Expr::Div(vec![ch("a"), ch("z")])).unwrap_err().contains("Division by zero"));
        assert_eq!(Expr::Sub(vec![ch("a"), Expr::Add(vec![ch("b"), ch("a")])]).channels().len(), 2);
    }

    #[test]
    fn serialised_shape() {
        let e: Expr =
            serde_json::from_str(r#"{ "sub": [ { "channel": "analog-3" }, { "constant": 1.5 } ] }"#).unwrap();
        assert_eq!(e, Expr::Sub(vec![ch("analog-3"), Expr::Constant(1.5)]));
        let a: Expr = serde_json::from_str(r#"{ "abs": { "channel": "x" } }"#).unwrap();
        assert_eq!(a, Expr::Abs(Box::new(ch("x"))));
    }
}
