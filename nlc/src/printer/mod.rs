//! Printable views over a [`Snapshot`].
//!
//! Each command is implemented as a [`Printer`] struct that holds its options
//! and renders text into a buffer. This keeps the rendering logic fully
//! decoupled from analysis (which lives in [`crate::snapshot`]) and from I/O
//! (which lives in `main`). Adding a new command means: add a struct, implement
//! `Printer`, wire it into the CLI dispatcher — nothing else changes.

use std::fmt::Write;

use crate::snapshot::Snapshot;

pub mod graph;
pub mod list;
pub mod status;

pub use graph::GraphPrinter;
pub use list::ListPrinter;
pub use status::StatusPrinter;

/// Render a [`Snapshot`] to text.
///
/// Implementations write into the provided `String` buffer (so output is
/// testable without capturing stdout) and return an exit code reflecting
/// whether anything was wrong with the snapshot (0 ok, 1 errors).
pub trait Printer {
    fn print(&self, snapshot: &Snapshot, out: &mut String) -> i32;
}

/// Small helper: push a line to a `String` buffer.
fn line(out: &mut String, s: &str) {
    let _ = writeln!(out, "{s}");
}
