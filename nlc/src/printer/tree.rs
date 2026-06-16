//! The `nlc tree <file>` view.
//!
//! Renders a file's section hierarchy and, under every node, recursively
//! expands its forward dependencies — following edges across files — to show
//! the full transitive dependency footprint. Cycles are detected per-branch
//! so traversal always terminates.

use std::collections::HashSet;
use std::fmt::Write;

use super::Printer;
use crate::model::{NodeId, Section};
use crate::snapshot::Snapshot;

/// Renders the node tree of a single file with recursive dependency expansion.
pub struct TreePrinter {
    pub file: String,
}

impl Printer for TreePrinter {
    fn print(&self, s: &Snapshot, out: &mut String) -> i32 {
        let Some((path, f)) = s.world.find_file(&self.file) else {
            let _ = writeln!(out, "nlc: no such file `{}`", self.file);
            return 2;
        };

        let _ = writeln!(out, "{}", path);

        // Collect the file root's top-level "children": its own forward
        // dependencies (from the preamble) followed by its top-level sections.
        let root_id = f.root_id();
        let mut children: Vec<Child> = Vec::new();
        for edge in s.graph.forward.get(&root_id).map(Vec::as_slice).unwrap_or(&[]) {
            children.push(Child::Dep {
                to: edge.to.clone(),
                raw: edge.raw.clone(),
            });
        }
        for section in &f.sections {
            children.push(Child::Section(section));
        }
        if children.is_empty() {
            let _ = writeln!(out, "└── (no sections, no dependencies)");
        }

        let ctx = Ctx { snap: s, file: path };
        let mut visited = HashSet::new();
        visited.insert(root_id);
        print_children(&children, "", &ctx, &mut visited, out);

        0
    }
}

/// One printable child of a node: either a structural sub-section or a
/// resolved/unresolved forward dependency.
enum Child<'a> {
    Section(&'a Section),
    Dep {
        to: Option<NodeId>,
        raw: String,
    },
}

struct Ctx<'a> {
    snap: &'a Snapshot,
    file: &'a str,
}

fn print_children(children: &[Child<'_>], prefix: &str, ctx: &Ctx<'_>, visited: &mut HashSet<NodeId>, out: &mut String) {
    let last_idx = children.len().saturating_sub(1);
    for (i, child) in children.iter().enumerate() {
        let is_last = i == last_idx;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });

        match child {
            Child::Section(section) => {
                let title = section.title();
                let _ = writeln!(
                    out,
                    "{prefix}{connector}{} {title}",
                    "#".repeat(section.level as usize)
                );
                // A section's children: its own forward deps, then its sub-sections.
                let id = section.id(ctx.file);
                let mut subs: Vec<Child> = Vec::new();
                for edge in ctx.snap.graph.forward.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                    subs.push(Child::Dep {
                        to: edge.to.clone(),
                        raw: edge.raw.clone(),
                    });
                }
                for sub in &section.children {
                    subs.push(Child::Section(sub));
                }
                if !subs.is_empty() {
                    print_children(&subs, &child_prefix, ctx, visited, out);
                }
            }
            Child::Dep { to, raw } => {
                match to {
                    Some(target) => {
                        if visited.contains(target) {
                            let _ = writeln!(out, "{prefix}{connector}{target} (cycle)");
                        } else {
                            visited.insert(target.clone());
                            let _ = writeln!(out, "{prefix}{connector}{target}");
                            // Recursively expand the target's forward deps.
                            let target_deps: Vec<Child> = ctx
                                .snap
                                .graph
                                .forward
                                .get(target)
                                .map(Vec::as_slice)
                                .unwrap_or(&[])
                                .iter()
                                .map(|e| Child::Dep {
                                    to: e.to.clone(),
                                    raw: e.raw.clone(),
                                })
                                .collect();
                            if !target_deps.is_empty() {
                                print_children(&target_deps, &child_prefix, ctx, visited, out);
                            }
                        }
                    }
                    None => {
                        let _ = writeln!(out, "{prefix}{connector}<unresolved>  ({raw})");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::build_file_node;
    use crate::model::World;

    fn snapshot_from<I>(files: I) -> Snapshot
    where
        I: IntoIterator<Item = (&'static str, &'static str)>,
    {
        use crate::cache::Cache;
        use crate::snapshot::CacheMeta;
        let mut world = World::default();
        for (path, src) in files {
            let doc = nlc_parser::parse(src).unwrap();
            let f = build_file_node(path.to_string(), doc.blocks, src.lines().count());
            world.files.insert(path.to_string(), f);
        }
        Snapshot::analyze(
            std::path::PathBuf::from("."),
            std::path::PathBuf::from(".nlc-cache"),
            world,
            Vec::new(),
            Cache::default(),
            CacheMeta::default(),
        )
    }

    #[test]
    fn tree_shows_sections_and_deps() {
        let s = snapshot_from([
            ("guide.md", "# Guide\n## Setup\nSee [[api.md#Install]].\n## Run\n"),
            ("api.md", "# API\n## Install\nDo it.\n"),
        ]);
        let printer = TreePrinter { file: "guide.md".into() };
        let mut out = String::new();
        printer.print(&s, &mut out);
        assert!(out.contains("Guide"), "{out}");
        assert!(out.contains("Setup"), "{out}");
        assert!(out.contains("api.md::api/install"), "{out}");
        assert!(!out.contains("→"), "dependency lines should not use arrows: {out}");
        assert!(!out.contains("(leaf)"), "{out}");
        assert!(!out.contains("(no dependencies)"), "{out}");
    }

    #[test]
    fn tree_handles_cycle() {
        let s = snapshot_from([
            ("a.md", "# A\n[[b.md#B]]\n"),
            ("b.md", "# B\n[[a.md#A]]\n"),
        ]);
        let printer = TreePrinter { file: "a.md".into() };
        let mut out = String::new();
        printer.print(&s, &mut out);
        assert!(out.contains("(cycle)"), "{out}");
    }

    #[test]
    fn tree_missing_file_errors() {
        let s = snapshot_from([("a.md", "# A\n")]);
        let printer = TreePrinter { file: "ghost.md".into() };
        let mut out = String::new();
        let code = printer.print(&s, &mut out);
        assert_eq!(code, 2);
        assert!(out.contains("no such file"), "{out}");
    }

    #[test]
    fn tree_shows_unresolved_dep() {
        let s = snapshot_from([("a.md", "# A\n[[ghost.md#X]]\n")]);
        let printer = TreePrinter { file: "a.md".into() };
        let mut out = String::new();
        printer.print(&s, &mut out);
        assert!(out.contains("<unresolved>"), "{out}");
    }
}
