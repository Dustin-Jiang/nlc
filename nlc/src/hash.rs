//! Deterministic content hashing for nodes.
//!
//! Every node — a file root or a section — is hashed with SHA-256 over a
//! canonical serialization of its content. Hashing is *hierarchical*: a
//! section's hash incorporates the hashes of all of its children, so a change
//! deep in a subtree automatically changes every ancestor's hash. That means
//! the diff phase only needs to compare hashes per [`NodeId`]; the upward
//! propagation is already baked in.
//!
//! The serialization is stable and unambiguous: every string is length-prefixed
//! (little-endian `u32`), and every enum variant carries a short tag plus open
//! / close markers for the recursive cases. We deliberately exclude derived
//! data (slugs, line counts, file paths) so the hash reflects *content* only.

use sha2::{Digest, Sha256};

use nlc_parser::ast::{Block, FileRef, FileRefTarget, Inline};

use crate::model::{FileNode, NodeId, Section};

fn tag(d: &mut Sha256, t: &str) {
    d.update(t.as_bytes());
    d.update(b"\x00");
}

fn write_len(d: &mut Sha256, n: usize) {
    d.update((n as u32).to_le_bytes());
}

fn write_str(d: &mut Sha256, s: &str) {
    write_len(d, s.len());
    d.update(s.as_bytes());
}

fn write_bool(d: &mut Sha256, b: bool) {
    d.update(if b { &[1] } else { &[0] });
}

fn write_opt_str(d: &mut Sha256, s: &Option<String>) {
    match s {
        Some(v) => {
            write_bool(d, true);
            write_str(d, v);
        }
        None => write_bool(d, false),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hash_inlines(d: &mut Sha256, inlines: &[Inline]) {
    write_len(d, inlines.len());
    for i in inlines {
        hash_inline(d, i);
    }
}

fn hash_inline(d: &mut Sha256, i: &Inline) {
    match i {
        Inline::Text(t) => {
            tag(d, "t");
            write_str(d, t);
        }
        Inline::Emphasis(v) => {
            tag(d, "em");
            hash_inlines(d, v);
        }
        Inline::Strong(v) => {
            tag(d, "st");
            hash_inlines(d, v);
        }
        Inline::Code(c) => {
            tag(d, "code");
            write_str(d, c);
        }
        Inline::Link {
            text,
            destination,
            title,
        } => {
            tag(d, "link");
            hash_inlines(d, text);
            write_str(d, destination);
            write_opt_str(d, title);
        }
        Inline::Image {
            alt,
            destination,
            title,
        } => {
            tag(d, "img");
            write_str(d, alt);
            write_str(d, destination);
            write_opt_str(d, title);
        }
        Inline::Autolink(s) => {
            tag(d, "auto");
            write_str(d, s);
        }
        Inline::RawHtml(s) => {
            tag(d, "html");
            write_str(d, s);
        }
        Inline::HardBreak => tag(d, "hbr"),
        Inline::SoftBreak => tag(d, "sbr"),
        Inline::FileRef(r) => {
            tag(d, "fref");
            hash_fileref(d, r);
        }
    }
}

fn hash_fileref(d: &mut Sha256, r: &FileRef) {
    write_opt_str(d, &r.path);
    match &r.target {
        FileRefTarget::Document => tag(d, "doc"),
        FileRefTarget::Section(s) => {
            tag(d, "sec");
            write_str(d, s);
        }
        FileRefTarget::Line(n) => {
            tag(d, "ln");
            d.update(n.to_le_bytes());
        }
        FileRefTarget::LineRange(a, b) => {
            tag(d, "lr");
            d.update(a.to_le_bytes());
            d.update(b.to_le_bytes());
        }
    }
    write_opt_str(d, &r.alias);
}

fn hash_block(d: &mut Sha256, b: &Block) {
    match b {
        Block::Paragraph(v) => {
            tag(d, "P");
            hash_inlines(d, v);
        }
        Block::Heading { level, inlines } => {
            tag(d, "H");
            d.update([*level]);
            hash_inlines(d, inlines);
        }
        Block::CodeBlock { info, code } => {
            tag(d, "CB");
            write_str(d, info);
            write_str(d, code);
        }
        Block::ThematicBreak => tag(d, "TB"),
        Block::BlockQuote(blocks) => {
            tag(d, "BQ");
            write_len(d, blocks.len());
            for b in blocks {
                hash_block(d, b);
            }
        }
        Block::List {
            items,
            ordered,
            start,
            tight,
        } => {
            tag(d, "L");
            write_bool(d, *ordered);
            d.update(start.to_le_bytes());
            write_bool(d, *tight);
            write_len(d, items.len());
            for item in items {
                match item.task {
                    None => write_len(d, 0),
                    Some(true) => write_len(d, 1),
                    Some(false) => write_len(d, 2),
                }
                write_len(d, item.blocks.len());
                for b in &item.blocks {
                    hash_block(d, b);
                }
            }
        }
        Block::HtmlBlock(h) => {
            tag(d, "HB");
            write_str(d, h);
        }
    }
}

/// Hash a single section given its children's already-computed hashes.
fn hash_section(s: &Section, child_hashes: &[String]) -> String {
    let mut d = Sha256::new();
    tag(&mut d, "section");
    d.update([s.level]);
    hash_inlines(&mut d, &s.heading);
    write_len(&mut d, s.body.len());
    for b in &s.body {
        hash_block(&mut d, b);
    }
    tag(&mut d, "kids");
    write_len(&mut d, child_hashes.len());
    for h in child_hashes {
        write_str(&mut d, h);
    }
    hex(&d.finalize())
}

/// Hash a file's root (preamble + top-level sections) given the top sections'
/// already-computed hashes.
fn hash_file(preamble: &[Block], top_hashes: &[String]) -> String {
    let mut d = Sha256::new();
    tag(&mut d, "file");
    write_len(&mut d, preamble.len());
    for b in preamble {
        hash_block(&mut d, b);
    }
    tag(&mut d, "tops");
    write_len(&mut d, top_hashes.len());
    for h in top_hashes {
        write_str(&mut d, h);
    }
    hex(&d.finalize())
}

/// Compute hashes for the file root and every section in `f`, bottom-up so
/// that each parent's hash can absorb its children's. Returns a `Vec` in
/// pre-order (root last).
pub fn hash_file_tree(f: &FileNode) -> Vec<(NodeId, String)> {
    let mut out = Vec::new();
    let mut top_hashes = Vec::with_capacity(f.sections.len());
    for s in &f.sections {
        let h = hash_section_tree(&f.path, s, &mut out);
        top_hashes.push(h);
    }
    let root = hash_file(&f.preamble, &top_hashes);
    out.push((f.root_id(), root));
    out
}

fn hash_section_tree(
    path: &str,
    s: &Section,
    out: &mut Vec<(NodeId, String)>,
) -> String {
    let mut child_hashes = Vec::with_capacity(s.children.len());
    for c in &s.children {
        let h = hash_section_tree(path, c, out);
        child_hashes.push(h);
    }
    let h = hash_section(s, &child_hashes);
    out.push((s.id(path), h.clone()));
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::build_file_node;

    fn hashes_for(src: &str) -> Vec<(NodeId, String)> {
        let doc = nlc_parser::parse(src).unwrap();
        let f = build_file_node("t.md".into(), doc.blocks, 0);
        hash_file_tree(&f)
    }

    #[test]
    fn stable_across_calls() {
        let a = hashes_for("# A\nhello\n");
        let b = hashes_for("# A\nhello\n");
        assert_eq!(a, b);
    }

    #[test]
    fn body_change_propagates_upward() {
        let before = hashes_for("# A\n## B\nbody\n");
        let after = hashes_for("# A\n## B\nchanged body\n");
        let root_before = before.last().unwrap();
        let root_after = after.last().unwrap();
        assert_eq!(root_before.0, root_after.0, "root NodeId must match");
        assert_ne!(root_before.1, root_after.1, "root hash must differ");

        let b_before = before
            .iter()
            .find(|(id, _)| id.as_str().ends_with("a::b"))
            .unwrap();
        let b_after = after
            .iter()
            .find(|(id, _)| id.as_str().ends_with("a::b"))
            .unwrap();
        assert_ne!(b_before.1, b_after.1);
    }

    #[test]
    fn unrelated_section_unaffected() {
        let before = hashes_for("# A\nbody a\n# B\nbody b\n");
        let after = hashes_for("# A\nbody a\n# B\nnew body b\n");
        let a_before = before.iter().find(|(id, _)| id.as_str().ends_with("a")).unwrap();
        let a_after = after.iter().find(|(id, _)| id.as_str().ends_with("a")).unwrap();
        assert_eq!(a_before.1, a_after.1);
    }
}
