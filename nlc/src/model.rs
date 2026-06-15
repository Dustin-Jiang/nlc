//! Core data model: a hierarchical section tree per Markdown file, plus the
//! [`World`] container holding every parsed file in the workspace.
//!
//! A file is split into a (possibly empty) preamble — the blocks appearing
//! before the first heading — and a forest of top-level sections. Each
//! [`Section`] owns the blocks belonging directly to it (everything up to the
//! next heading at the same or shallower level) plus a list of nested child
//! sections (any deeper headings encountered while scanning its body).
//!
//! Every node — a file root or a section — is identified by a stable
//! [`NodeId`], which is what the dependency graph, the hasher, and the on-disk
//! cache all key off.

use std::collections::BTreeMap;
use std::path::PathBuf;

use nlc_parser::ast::{Block, Inline};

/// Stable identifier for a node in the workspace.
///
/// The canonical string form is:
///  * the file's relative path for a *file root* (e.g. `guide.md`), and
///  * `<rel-path>::<slug1>/<slug2>/...` for a *section*
///    (e.g. `guide.md::intro/setup`).
///
/// `NodeId`s are cheap to clone and compare; build them with [`NodeId::file`]
/// and [`NodeId::section`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub String);

impl NodeId {
    /// Identifier for the root of a file (aggregates preamble + top sections).
    pub fn file(rel_path: &str) -> Self {
        NodeId(rel_path.to_string())
    }

    /// Identifier for a section, given the file's relative path and the
    /// section's slug chain (each slug is the last path component of an
    /// ancestor or self).
    pub fn section(rel_path: &str, slug_path: &[String]) -> Self {
        if slug_path.is_empty() {
            NodeId(rel_path.to_string())
        } else {
            NodeId(format!("{}::{}", rel_path, slug_path.join("/")))
        }
    }

    /// Raw canonical string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A single heading-bounded section.
#[derive(Debug, Clone)]
pub struct Section {
    /// ATX level (1-6).
    pub level: u8,
    /// Slug of this section's own heading (last path component).
    pub slug: String,
    /// Full slug chain from the file root down to and including this section.
    pub slug_path: Vec<String>,
    /// The heading's parsed inline content.
    pub heading: Vec<Inline>,
    /// Blocks belonging directly to this section (non-heading blocks up to the
    /// next heading at the same or shallower level). May contain nested
    /// non-heading blocks of deeper sections? No — those are routed to
    /// [`Section::children`]. Body holds only "leaf prose" between sub-headings.
    pub body: Vec<Block>,
    /// Nested deeper-level sections.
    pub children: Vec<Section>,
}

impl Section {
    /// This section's [`NodeId`].
    pub fn id(&self, rel_path: &str) -> NodeId {
        NodeId::section(rel_path, &self.slug_path)
    }

    /// Display title (flattened heading inlines).
    pub fn title(&self) -> String {
        crate::inline_text::inline_text(&self.heading)
    }

    /// Depth-first visitation of this section and all of its descendants.
    pub fn walk(&self) -> impl Iterator<Item = &Section> {
        std::iter::once(self)
            .chain(self.children.iter().flat_map(|c| c.walk()))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// A parsed Markdown file rendered as a section tree.
#[derive(Debug, Clone)]
pub struct FileNode {
    /// Workspace-relative path with forward slashes (e.g. `docs/guide.md`).
    pub path: String,
    /// Blocks before the first heading (hash-only, never referenced by slug).
    pub preamble: Vec<Block>,
    /// Top-level (shallowest) sections.
    pub sections: Vec<Section>,
    /// Number of lines in the source text — used to validate `[[f#L42]]`.
    pub line_count: usize,
}

impl FileNode {
    pub fn root_id(&self) -> NodeId {
        NodeId::file(&self.path)
    }

    /// Flatten every node (root + all sections) into a list of `(id, node_ref)`
    /// pairs in a deterministic pre-order.
    pub fn nodes(&self) -> Vec<(NodeId, NodeRef<'_>)> {
        let mut out = vec![(self.root_id(), NodeRef::File(self))];
        for s in &self.sections {
            self.push_section(s, &mut out);
        }
        out
    }

    fn push_section<'a>(&'a self, s: &'a Section, out: &mut Vec<(NodeId, NodeRef<'a>)>) {
        out.push((s.id(&self.path), NodeRef::Section(s)));
        for c in &s.children {
            self.push_section(c, out);
        }
    }

    /// Depth-first iterator over every [`Section`] in the file.
    pub fn walk_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections
            .iter()
            .flat_map(|s| s.walk())
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// A borrowed reference to either kind of node.
#[derive(Debug, Clone, Copy)]
pub enum NodeRef<'a> {
    File(&'a FileNode),
    Section(&'a Section),
}

impl<'a> NodeRef<'a> {
    #[allow(dead_code)]
    pub fn id(&self, file_path: &str) -> NodeId {
        match self {
            NodeRef::File(_) => NodeId::file(file_path),
            NodeRef::Section(s) => s.id(file_path),
        }
    }

    /// All blocks that directly belong to this node (preamble + top sections
    /// for a file; the section body for a section).
    #[allow(dead_code)]
    pub fn own_blocks(&self) -> &[Block] {
        match self {
            NodeRef::File(f) => &f.preamble,
            NodeRef::Section(s) => &s.body,
        }
    }
}

/// The full parsed workspace.
#[derive(Debug, Default)]
pub struct World {
    /// Workspace root (cwd at scan time).
    #[allow(dead_code)]
    pub root: PathBuf,
    /// Files keyed by workspace-relative path.
    pub files: BTreeMap<String, FileNode>,
}

impl World {
    pub fn file(&self, rel_path: &str) -> Option<&FileNode> {
        self.files.get(rel_path)
    }

    /// Total number of sections across all files.
    pub fn section_count(&self) -> usize {
        self.files
            .values()
            .map(|f| f.walk_sections().count())
            .sum()
    }

    /// Iterate `(node_id, NodeRef)` over every node in deterministic order
    /// (file order, then pre-order within each file).
    pub fn nodes(&self) -> Vec<(NodeId, NodeRef<'_>)> {
        self.files.values().flat_map(|f| f.nodes()).collect()
    }
}
