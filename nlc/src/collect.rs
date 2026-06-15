//! File-system scanning, parsing, and construction of [`World`].
//!
//! The walker descends into the workspace root recursively, skipping hidden
//! entries and a small deny-list of build/VCS directories. Each `.md` file is
//! parsed with [`nlc_parser`] and turned into a [`FileNode`] by
//! [`build_file_node`], which splits the block stream into a preamble and a
//! forest of nested [`Section`]s using a single forward scan with a peekable
//! iterator.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nlc_parser::ast::{Block, Inline};

use crate::inline_text::{inline_text, slugify};
use crate::model::{FileNode, Section, World};

/// Directories that are never descended into, regardless of the
/// "skip hidden" rule. These are common build outputs and VCS state.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "out",
    "dist",
    "build",
    ".cache",
];

/// A failure encountered while gathering a single file.
#[derive(Debug, Clone)]
pub struct CollectError {
    pub path: String,
    pub message: String,
}

/// The result of scanning a workspace: the parsed [`World`] plus any per-file
/// read/parse errors (which do not abort the scan).
#[derive(Debug, Default)]
pub struct Collected {
    pub world: World,
    pub errors: Vec<CollectError>,
}

/// Recursively scan `root` for `.md` files, parse them, and assemble a
/// [`World`]. Hidden entries and [`SKIP_DIRS`] are pruned.
pub fn collect(root: &Path) -> Collected {
    let mut files = BTreeMap::new();
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort();
    for path in entries {
        let rel = rel_path(root, &path);
        match fs::read_to_string(&path) {
            Ok(src) => match nlc_parser::parse(&src) {
                Ok(doc) => {
                    let line_count = src.lines().count();
                    let node = build_file_node(rel.clone(), doc.blocks, line_count);
                    files.insert(rel, node);
                }
                Err(e) => errors.push(CollectError {
                    path: rel_path(root, &path),
                    message: format!("parse error: {e}"),
                }),
            },
            Err(e) => errors.push(CollectError {
                path: rel,
                message: format!("read error: {e}"),
            }),
        }
    }
    Collected {
        world: World {
            root: root.to_path_buf(),
            files,
        },
        errors,
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut sub_entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    sub_entries.sort();
    for path in sub_entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            if dir != root && name.starts_with('.') {
                continue;
            }
            walk(root, &path, out);
        } else if is_markdown(name) {
            // Hidden markdown files below the root are skipped too.
            if dir != root && name.starts_with('.') {
                continue;
            }
            out.push(path);
        }
    }
}

fn is_markdown(name: &str) -> bool {
    name.len() > 3 && name[name.len() - 3..].eq_ignore_ascii_case(".md")
}

fn rel_path(root: &Path, full: &Path) -> String {
    full.strip_prefix(root)
        .unwrap_or(full)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Turn a flat block list into a [`FileNode`] by building the section forest.
///
/// Sections nest by heading level: a heading at level L becomes a child of the
/// nearest ancestor section whose level is strictly less than L (or a
/// top-level section if there is none). Non-heading blocks are appended to the
/// body of the section currently being built — or to the preamble when no
/// section has been opened yet. Sibling sections with identical slugs get
/// `-1`, `-2`, ... suffixes (GitHub-style).
pub fn build_file_node(path: String, blocks: Vec<Block>, line_count: usize) -> FileNode {
    let mut iter = blocks.into_iter().peekable();

    // Preamble: everything before the first heading.
    let mut preamble = Vec::new();
    while let Some(b) = iter.peek() {
        if matches!(b, Block::Heading { .. }) {
            break;
        }
        preamble.push(iter.next().unwrap());
    }

    // Top-level sections.
    let mut top: Vec<Section> = Vec::new();
    while let Some(b) = iter.next() {
        let Block::Heading { level, inlines } = b else {
            // Non-heading blocks here were already routed into the last-opened
            // section's body via build_section. Reaching this branch means the
            // section stack is empty (we just finished a section) and a stray
            // non-heading block appeared — treat it as preamble.
            preamble.push(b);
            continue;
        };
        let base = slugify(&inline_text(&inlines));
        let slug = dedupe(&top.iter().map(|s| s.slug.clone()).collect::<Vec<_>>(), base);
        let section = build_section(level, slug, inlines, &mut iter, Vec::new());
        top.push(section);
    }

    FileNode {
        path,
        preamble,
        sections: top,
        line_count,
    }
}

/// Recursively build a single section, consuming its body and all of its
/// descendants from `iter`. The section's own `slug` and parent-derived
/// `slug_path` are supplied by the caller; this function appends `slug` to
/// `slug_path` and recurses into children with the extended path.
fn build_section(
    level: u8,
    slug: String,
    heading: Vec<Inline>,
    iter: &mut std::iter::Peekable<std::vec::IntoIter<Block>>,
    parent_path: Vec<String>,
) -> Section {
    let mut slug_path = parent_path;
    slug_path.push(slug.clone());

    let mut body: Vec<Block> = Vec::new();
    let mut children: Vec<Section> = Vec::new();

    while let Some(b) = iter.peek() {
        match b {
            // A heading at the same or shallower level closes this section.
            Block::Heading { level: l, .. } if *l <= level => break,
            // A deeper heading opens a child section.
            Block::Heading { level: l, .. } => {
                let child_level = *l;
                let Block::Heading {
                    inlines: child_inlines,
                    ..
                } = iter.next().unwrap()
                else {
                    unreachable!()
                };
                let base = slugify(&inline_text(&child_inlines));
                let existing: Vec<String> =
                    children.iter().map(|c| c.slug.clone()).collect();
                let child_slug = dedupe(&existing, base);
                let child = build_section(
                    child_level,
                    child_slug,
                    child_inlines,
                    iter,
                    slug_path.clone(),
                );
                children.push(child);
            }
            // Any other block belongs to this section's body.
            _ => {
                body.push(iter.next().unwrap());
            }
        }
    }

    Section {
        level,
        slug,
        slug_path,
        heading,
        body,
        children,
    }
}

/// If `existing` already contains `base`, append `-1`, `-2`, ... until a free
/// variant is found.
fn dedupe(existing: &[String], base: String) -> String {
    if !existing.iter().any(|s| s == &base) {
        return base;
    }
    let mut n = 1;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|s| s == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_text(s: &Section) -> String {
        use nlc_parser::ast::{Block, Inline};
        s.body
            .iter()
            .flat_map(|b| -> Vec<String> {
                match b {
                    Block::Paragraph(v) => v
                        .iter()
                        .filter_map(|i| match i {
                            Inline::Text(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn flat_sections() {
        let doc = nlc_parser::parse("# A\npara a\n# B\npara b\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        assert!(f.preamble.is_empty());
        assert_eq!(f.sections.len(), 2);
        assert_eq!(f.sections[0].slug, "a");
        assert_eq!(f.sections[0].body.len(), 1);
        assert_eq!(f.sections[1].slug, "b");
        assert_eq!(body_text(&f.sections[1]), "para b");
    }

    #[test]
    fn nested_sections_keep_body_in_parent() {
        // A paragraph after a deeper heading stays under that deeper heading
        // until a shallower heading closes it (standard "nearest preceding
        // heading" rule). So "back to B" belongs to C, not B.
        let doc = nlc_parser::parse("# A\nintro a\n## B\nintro b\n### C\nc body\nback to B\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        assert_eq!(f.sections.len(), 1);
        let a = &f.sections[0];
        assert_eq!(a.slug, "a");
        assert_eq!(body_text(a), "intro a");
        assert_eq!(a.children.len(), 1);
        let b = &a.children[0];
        assert_eq!(b.slug, "b");
        assert_eq!(body_text(b), "intro b");
        assert_eq!(b.children.len(), 1);
        let c = &b.children[0];
        assert_eq!(c.slug, "c");
        assert_eq!(body_text(c), "c body|back to B");
        assert_eq!(
            c.slug_path,
            vec!["a".to_string(), "b".into(), "c".into()]
        );
    }

    #[test]
    fn shallower_heading_closes_deeper_section() {
        // A repeated `## B` heading (after `### C`) re-opens a sibling B
        // section; content after it belongs to that new B, not to C.
        let doc = nlc_parser::parse("# A\n## B\n### C\nc body\n## B\nback to B\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        let a = &f.sections[0];
        assert_eq!(a.children.len(), 2, "second ## B becomes a sibling b-1");
        let b1 = &a.children[0];
        assert_eq!(b1.slug, "b");
        assert_eq!(b1.children.len(), 1);
        assert_eq!(b1.children[0].slug, "c");
        let b2 = &a.children[1];
        assert_eq!(b2.slug, "b-1");
        assert_eq!(body_text(b2), "back to B");
    }

    #[test]
    fn preamble_before_first_heading() {
        let doc = nlc_parser::parse("hello\n# A\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        assert_eq!(f.preamble.len(), 1);
        assert_eq!(f.sections.len(), 1);
    }

    #[test]
    fn sibling_after_deep_nesting_pops_correctly() {
        // h1 → h2 → h3, then a new h1 must pop all the way back.
        let doc = nlc_parser::parse("# A\n## B\n### C\n# D\nd body\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        assert_eq!(f.sections.len(), 2);
        assert_eq!(f.sections[0].slug, "a");
        assert_eq!(f.sections[1].slug, "d");
        assert_eq!(body_text(&f.sections[1]), "d body");
    }

    #[test]
    fn duplicate_slug_disambiguation() {
        let doc = nlc_parser::parse("# Setup\n# Setup\n# Setup\n").unwrap();
        let f = build_file_node("x.md".into(), doc.blocks, 0);
        assert_eq!(f.sections.len(), 3);
        assert_eq!(f.sections[0].slug, "setup");
        assert_eq!(f.sections[1].slug, "setup-1");
        assert_eq!(f.sections[2].slug, "setup-2");
    }
}
