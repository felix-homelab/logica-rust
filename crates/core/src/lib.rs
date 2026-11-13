//! Logica's data model and capture store (doc 03, doc 04).
//!
//! This crate does no I/O of its own: files reach it through [`io::RandomAccess`], and it builds for
//! `wasm32-unknown-unknown`. Time inside it is the sample index ([`Sample`]); seconds appear only at
//! the edges.

#![cfg_attr(test, allow(clippy::cast_possible_truncation))]

pub mod capture;
pub mod io;
pub mod lacap;
pub mod store;

pub use store::{Level, Sample};
