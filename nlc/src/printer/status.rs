//! The incremental status report (default `nlc` / `nlc status` / `nlc check`).

use super::{line, Printer};
use crate::snapshot::{NodeChange, Snapshot};

/// Renders the make-style "what changed, what did I rebuild" report.
pub struct StatusPrinter {
    /// `--full`: report every node's issues, not only the delta.
    pub full: bool,
}

impl Printer for StatusPrinter {
    fn print(&self, s: &Snapshot, out: &mut String) -> i32 {
        // Header / summary.
        line(
            out,
            &format!(
                "nlc: scanned {} file(s), {} section(s)",
                s.world.files.len(),
                s.world.section_count()
            ),
        );
        if s.cache_meta.existed && s.cache_meta.corrupted {
            line(out, "nlc: warning: existing cache was corrupted; treating as a fresh run");
        }

        if !s.parse_errors.is_empty() {
            line(out, &format!("\nparse errors ({}):", s.parse_errors.len()));
            for e in &s.parse_errors {
                line(out, &format!("  {}: {}", e.path, e.message));
            }
        }

        // Delta summary.
        let d = &s.delta;
        let added = d.changes.iter().filter(|c| c.change == NodeChange::Added).count();
        let modified = d.changes.iter().filter(|c| c.change == NodeChange::Modified).count();
        let removed = d.changes.iter().filter(|c| c.change == NodeChange::Removed).count();
        let affected_only = d.affected_only();
        let changed_count = d
            .changes
            .iter()
            .filter(|c| c.change != NodeChange::Removed)
            .count();
        line(
            out,
            &format!(
                "  changed: {} node(s) ({} modified, {} added, {} removed)",
                d.changes.len(),
                modified,
                added,
                removed
            ),
        );
        line(
            out,
            &format!(
                "  revalidated: {} node(s) ({} changed + {} affected)",
                d.revalidated.len(),
                changed_count,
                affected_only.len()
            ),
        );
        line(out, &format!("  up-to-date: {} node(s)", s.up_to_date_count()));

        // Changed-node detail.
        if !d.changes.is_empty() {
            line(out, "\nchanged nodes:");
            for c in &d.changes {
                let label = match c.change {
                    NodeChange::Added => "added",
                    NodeChange::Modified => "modified",
                    NodeChange::Removed => "removed",
                };
                line(out, &format!("  [{label}] {}", c.node));
            }
        }
        if !affected_only.is_empty() {
            line(out, "\naffected (rebuilt by propagation):");
            for id in &affected_only {
                line(out, &format!("  {id}"));
            }
        }

        // Issues — full detail for revalidated nodes; everything only with --full.
        let issues_to_show: Vec<&crate::graph::Issue> = if self.full {
            s.graph.issues.iter().collect()
        } else {
            s.graph
                .issues
                .iter()
                .filter(|i| d.revalidated.contains(&i.source))
                .collect()
        };
        if !issues_to_show.is_empty() {
            line(out, &format!("\nissues ({}):", issues_to_show.len()));
            for issue in &issues_to_show {
                line(out, &format!("  error: {}", issue.message));
            }
            if !self.full {
                let hidden = s.graph.issues.len() - issues_to_show.len();
                if hidden > 0 {
                    line(out, &format!(
                        "  ({hidden} more on up-to-date nodes; use --full to show)"
                    ));
                }
            }
        }

        // Regressions / resolutions.
        if !d.regressions.is_empty() {
            line(out, "\nregressions (were ok, now broken):");
            for id in &d.regressions {
                line(out, &format!("  {id}"));
            }
        }
        if !d.resolved.is_empty() {
            line(out, "\nresolved (were broken, now ok):");
            for id in &d.resolved {
                line(out, &format!("  {id}"));
            }
        }

        // Verdict line.
        let total_errors = s.graph.issues.len() + s.parse_errors.len();
        if total_errors == 0 {
            line(out, "\nok");
        } else {
            line(out, &format!("\n{total_errors} error(s)"));
        }

        if s.has_errors() {
            1
        } else {
            0
        }
    }
}
