//! File-system scanning, parsing, and construction of [`World`].
//!
//! The walker descends into the workspace root recursively, skipping hidden
//! entries and a small deny-list of build/VCS directories. Each `.md` file is
//! parsed with [`nlc_parser`] and turned into a [`FileNode`] by
//! [`build_file_node`], which splits the block stream into a preamble and a
//! forest of nested [`Section`]s using a single forward scan with a peekable
//! iterator.
//!
//! Every other readable UTF-8 file (Rust sources, configs, plain text, …) is
//! recorded as a [`CodeFile`] — a reference target only, never parsed or
//! hashed. Files that are not valid UTF-8 (binaries, images, …) are skipped
//! silently.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nlc_parser::ast::{Block, Inline};

use crate::inline_text::{inline_text, slugify};
use crate::model::{CodeFile, FileNode, Section, World};

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

/// Files that are never collected, even at the workspace root. `.nlc-cache` is
/// nlc's own runtime state (gitignored) and must not be treated as a code-file
/// reference target.
const SKIP_FILES: &[&str] = &[".nlc-cache"];

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

/// Recursively scan `root`, parsing `.md` files into [`FileNode`]s and
/// recording every other readable UTF-8 file as a [`CodeFile`]. Hidden entries
/// and [`SKIP_DIRS`] are pruned; [`SKIP_FILES`] and non-UTF-8 (binary) files
/// are skipped silently.
pub fn collect(root: &Path) -> Collected {
    let mut files = BTreeMap::new();
    let mut code_files = BTreeMap::new();
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort();
    for path in entries {
        let rel = rel_path(root, &path);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if SKIP_FILES.contains(&name) {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(src) => {
                if is_markdown(name) {
                    match nlc_parser::parse(&src) {
                        Ok(doc) => {
                            let line_count = src.lines().count();
                            let node = build_file_node(rel.clone(), doc.blocks, line_count);
                            files.insert(rel, node);
                        }
                        Err(e) => errors.push(CollectError {
                            path: rel_path(root, &path),
                            message: format!("parse error: {e}"),
                        }),
                    }
                } else {
                    let line_count = src.lines().count();
                    code_files.insert(rel, CodeFile { line_count });
                }
            }
            // Non-UTF-8 (binary) or unreadable file. A `.md` file failing to
            // read is surprising and worth reporting; anything else is just a
            // non-text artifact we silently ignore.
            Err(e) if is_markdown(name) => errors.push(CollectError {
                path: rel_path(root, &path),
                message: format!("read error: {e}"),
            }),
            Err(_) => {}
        }
    }
    Collected {
        world: World {
            root: root.to_path_buf(),
            files,
            code_files,
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
        } else {
            // Hidden files below the root are skipped (matching the directory
            // rule). At the root itself hidden files are kept, so dotfiles like
            // `.nlc-cache` are reachable for the SKIP_FILES deny-list.
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

    /// A throwaway directory under the OS temp dir, removed on drop. Keeps the
    /// workspace dependency-free (no `tempfile`).
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "nlc-collect-{label}-{}",
                std::process::id()
            ));
            // Start from a clean slate in case a prior run leaked.
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }

        fn write(&self, rel: &str, contents: &str) {
            let full = self.path.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, contents).unwrap();
        }

        fn write_bytes(&self, rel: &str, bytes: &[u8]) {
            let full = self.path.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, bytes).unwrap();
        }

        fn root(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn markdown_goes_to_files_code_goes_to_code_files() {
        let tmp = TempDir::new("md_and_code");
        tmp.write("guide.md", "# Title\nbody\n");
        tmp.write("src/main.rs", "fn main() {}\n");
        tmp.write("Makefile", "all:\n\techo hi\n");

        let collected = collect(tmp.root());
        assert!(collected.errors.is_empty(), "{:?}", collected.errors);
        assert!(collected.world.files.contains_key("guide.md"));
        assert!(!collected.world.code_files.contains_key("guide.md"));
        let rs = collected.world.code_files.get("src/main.rs").unwrap();
        assert_eq!(rs.line_count, 1);
        assert!(collected.world.code_files.contains_key("Makefile"));
    }

    #[test]
    fn code_file_line_count_counts_lines() {
        let tmp = TempDir::new("line_count");
        tmp.write("a.rs", "line1\nline2\nline3\n");
        tmp.write("b.rs", "only one, no trailing newline");
        let collected = collect(tmp.root());
        assert_eq!(collected.world.code_files["a.rs"].line_count, 3);
        // A file with no trailing newline still counts as one line.
        assert_eq!(collected.world.code_files["b.rs"].line_count, 1);
    }

    #[test]
    fn binary_files_are_silently_skipped() {
        let tmp = TempDir::new("binary");
        tmp.write("ok.md", "# Hi\n");
        // Invalid UTF-8 (a UTF-16 BOM + garbage) — read_to_string fails.
        tmp.write_bytes("blob.bin", &[0xff, 0xfe, 0x00, 0x01, 0xc3, 0x28]);
        let collected = collect(tmp.root());
        assert!(collected.errors.is_empty(), "{:?}", collected.errors);
        assert!(collected.world.files.contains_key("ok.md"));
        assert!(
            !collected.world.code_files.contains_key("blob.bin"),
            "binary must not be collected as a code file"
        );
    }

    #[test]
    fn nlc_cache_file_is_not_collected() {
        let tmp = TempDir::new("cache_skip");
        tmp.write("doc.md", "# Doc\n");
        tmp.write(".nlc-cache", "v1\nhash\nhash\n");
        let collected = collect(tmp.root());
        assert!(
            !collected.world.code_files.contains_key(".nlc-cache"),
            ".nlc-cache is runtime state and must never be a reference target"
        );
        assert!(collected.world.files.contains_key("doc.md"));
    }

    #[test]
    fn hidden_files_below_root_are_skipped() {
        let tmp = TempDir::new("hidden");
        tmp.write("doc.md", "# Doc\n");
        tmp.write("sub/.hidden.rs", "fn x() {}\n");
        tmp.write("sub/visible.rs", "fn y() {}\n");
        let collected = collect(tmp.root());
        assert!(collected.world.code_files.contains_key("sub/visible.rs"));
        assert!(
            !collected.world.code_files.contains_key("sub/.hidden.rs"),
            "hidden files below the root must be skipped"
        );
    }
}
