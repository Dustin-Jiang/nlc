//! The `nlc graph` view: every cross-file reference edge, one per line.

use super::{line, Printer};
use crate::snapshot::Snapshot;

/// Renders all reference edges as `from -> to` (or `from -> <unresolved>`).
pub struct GraphPrinter;

impl Printer for GraphPrinter {
    fn print(&self, s: &Snapshot, out: &mut String) -> i32 {
        let mut keys: Vec<&crate::model::NodeId> = s.graph.forward.keys().collect();
        keys.sort();
        let mut printed_any = false;
        for from in keys {
            let edges = &s.graph.forward[from];
            if edges.is_empty() {
                continue;
            }
            printed_any = true;
            for e in edges {
                match &e.to {
                    Some(to) => line(out, &format!("{} -> {}", e.from, to)),
                    None => line(out, &format!("{} -> <unresolved>  ({})", e.from, e.raw)),
                }
            }
        }
        if !printed_any {
            line(out, "(no cross-file references)");
        }
        0
    }
}
