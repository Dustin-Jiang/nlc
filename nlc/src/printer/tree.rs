//! The `nlc tree <file>` view.
//!
//! expands its forward dependencies — following edges across files — to show
//! the full transitive dependency footprint. A Markdown target (a document
//! file or one of its sections) additionally expands its own section tree,
//! so referenced documents are rendered in full; code-file targets remain
//! leaves. Cycles are detected per-branch so traversal always terminates.

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

        // The file root's children: its own forward dependencies (from the
        // preamble) followed by its top-level sections — the same expansion
        // any Markdown dep target receives.
        let root_id = f.root_id();
        let children = markdown_children(s, &root_id).unwrap_or_default();
        if children.is_empty() {
            let _ = writeln!(out, "└── (no sections, no dependencies)");
        }

        let ctx = Ctx { snap: s };
        let mut visited = HashSet::new();
        visited.insert(root_id);
        print_children(&children, "", &ctx, &mut visited, out);

        0
    }
}

/// One printable child of a node: either a structural sub-section (tagged
/// with the Markdown file it belongs to) or a resolved/unresolved forward
/// dependency.
enum Child<'a> {
    Section {
        file: &'a str,
        section: &'a Section,
    },
    Dep {
        to: Option<NodeId>,
        raw: String,
    },
}

struct Ctx<'a> {
    snap: &'a Snapshot,
}

/// Forward-dependency children of any node.
fn dep_children<'a>(snap: &'a Snapshot, id: &NodeId) -> Vec<Child<'a>> {
    snap.graph
        .forward
        .get(id)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|e| Child::Dep {
            to: e.to.clone(),
            raw: e.raw.clone(),
        })
        .collect()
}

/// Children of a Markdown node (file root or section): the node's own
/// forward dependencies followed by its sub-sections. `None` if `id` is not
/// a Markdown node in the workspace — code-file targets (`src/main.rs`,
/// `src/main.rs::L12`) have no section tree and no outgoing edges.
fn markdown_children<'a>(snap: &'a Snapshot, id: &NodeId) -> Option<Vec<Child<'a>>> {
    let (path, slugs) = match id.as_str().split_once("::") {
        Some((path, rest)) => (path, Some(rest)),
        None => (id.as_str(), None),
    };
    let f = snap.world.file(path)?;
    let mut subs: &[Section] = &f.sections;
    if let Some(rest) = slugs {
        for slug in rest.split("::") {
            subs = &subs.iter().find(|s| s.slug == slug)?.children;
        }
    }
    let mut children = dep_children(snap, id);
    children.extend(
        subs.iter()
            .map(|section| Child::Section {
                file: f.path.as_str(),
                section,
            }),
    );
    Some(children)
}

fn print_children(children: &[Child<'_>], prefix: &str, ctx: &Ctx<'_>, visited: &mut HashSet<NodeId>, out: &mut String) {
    let last_idx = children.len().saturating_sub(1);
    for (i, child) in children.iter().enumerate() {
        let is_last = i == last_idx;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });

        match child {
            Child::Section { file, section } => {
                let title = section.title();
                let _ = writeln!(
                    out,
                    "{prefix}{connector}{} {title}",
                    "#".repeat(section.level as usize)
                );
                // A section's children: its own forward deps, then its
                // sub-sections. The section itself counts as shown.
                let id = section.id(file);
                visited.insert(id.clone());
                let mut subs = dep_children(ctx.snap, &id);
                subs.extend(
                    section
                        .children
                        .iter()
                        .map(|c| Child::Section { file, section: c }),
                );
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
                            // Markdown targets expand their own section
                            // tree on top of their forward deps; other
                            // targets (code lines) have no outgoing edges.
                            let subs =
                                markdown_children(ctx.snap, target).unwrap_or_default();
                            if !subs.is_empty() {
                                print_children(&subs, &child_prefix, ctx, visited, out);
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
        assert!(out.contains("api.md::api::install"), "{out}");
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

    #[test]
    fn tree_expands_markdown_target_section_tree() {
        let s = snapshot_from([
            ("guide.md", "# Guide\nRead [[api.md]].\n"),
            (
                "api.md",
                "# API\n## Install\nSee [[api.md#Requirements]].\n### Requirements\nNeed rust.\n",
            ),
        ]);
        let printer = TreePrinter {
            file: "guide.md".into(),
        };
        let mut out = String::new();
        printer.print(&s, &mut out);
        let expected = "\
guide.md
└── # Guide
    └── api.md
        └── # API
            └── ## Install
                ├── api.md::api::install::requirements
                └── ### Requirements
";
        assert_eq!(out, expected);
    }

    #[test]
    fn tree_code_target_stays_leaf() {
        let s = snapshot_with_code(
            [("guide.md", "# Guide\nUse [[main.rs]].\n")],
            [("main.rs", 10)],
        );
        let printer = TreePrinter {
            file: "guide.md".into(),
        };
        let mut out = String::new();
        printer.print(&s, &mut out);
        assert_eq!(out, "guide.md\n└── # Guide\n    └── main.rs\n");
    }

    fn snapshot_with_code<I, J>(files: I, code: J) -> Snapshot
    where
        I: IntoIterator<Item = (&'static str, &'static str)>,
        J: IntoIterator<Item = (&'static str, usize)>,
    {
        use crate::cache::Cache;
        use crate::model::CodeFile;
        use crate::snapshot::CacheMeta;
        let mut world = World::default();
        for (path, src) in files {
            let doc = nlc_parser::parse(src).unwrap();
            let f = build_file_node(path.to_string(), doc.blocks, src.lines().count());
            world.files.insert(path.to_string(), f);
        }
        for (path, line_count) in code {
            world
                .code_files
                .insert(path.to_string(), CodeFile { line_count });
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
}
