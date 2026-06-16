//! The cross-file reference graph.
//!
//! For every node in the [`World`] we scan its heading inlines and body blocks
//! for `[[...]]` [`FileRef`]s and resolve each one against the workspace,
//! producing directed edges `referrer → referent`. Edges that cannot be
//! resolved become [`Issue`]s (dangling / ambiguous / out-of-range).
//!
//! The graph stores both forward edges (for reporting and cycle detection) and
//! a reverse adjacency map (so the incremental driver can ask "who depends on
//! this dirty node?"). Cycles are found with an iterative Kosaraju SCC pass
//! over the resolved edges; like `make`, circular dependencies are treated as
//! errors.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use nlc_parser::ast::{Block, FileRef, FileRefTarget};

use crate::inline_text::slugify;
use crate::model::{FileKind, NodeId, NodeRef, World};

/// A single resolved-or-not `[[...]]` reference originating from `from`.
#[derive(Debug, Clone)]
pub struct Edge {
    pub from: NodeId,
    /// The resolved target, if any.
    pub to: Option<NodeId>,
    /// The reconstructed `[[...]]` text, for human-friendly reporting.
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    MissingFile,
    MissingSection,
    AmbiguousSection,
    LineOutOfRange,
    RangeOutOfRange,
    /// A named/section target (e.g. `[[code.rs#main]]`) was used against a
    /// non-Markdown (code) file. Code files only support `#L<line>` /
    /// `#L<a>-L<b>` line locators.
    UnsupportedCodeTarget,
    /// A circular dependency group; the members are listed in
    /// [`Issue::members`] (sorted for stable reporting).
    Cycle,
}

#[derive(Debug, Clone)]
pub struct Issue {
    #[allow(dead_code)]
    pub kind: IssueKind,
    /// Node on which the problem was discovered.
    pub source: NodeId,
    pub message: String,
    /// Populated for [`IssueKind::Cycle`].
    #[allow(dead_code)]
    pub members: Vec<NodeId>,
}

/// The fully-built reference graph.
#[derive(Debug, Default)]
pub struct Graph {
    /// `referrer → all edges leaving it`.
    pub forward: BTreeMap<NodeId, Vec<Edge>>,
    /// `referent → all referrers of it` (resolved edges only).
    pub reverse: BTreeMap<NodeId, Vec<NodeId>>,
    /// Every validation issue, in source order.
    pub issues: Vec<Issue>,
}

impl Graph {
    /// All referrers that point at `target` (resolved edges only).
    #[allow(dead_code)]
    pub fn referrers(&self, target: &NodeId) -> &[NodeId] {
        self.reverse
            .get(target)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// True if any validation issue is an error (all current kinds are errors).
    #[allow(dead_code)]
    pub fn has_errors(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// Build the reference graph for an entire [`World`].
pub fn build(world: &World) -> Graph {
    let mut forward: BTreeMap<NodeId, Vec<Edge>> = BTreeMap::new();
    let mut reverse: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut issues: Vec<Issue> = Vec::new();

    for (id, node) in world.nodes() {
        let mut edges_here = Vec::new();
        for r in node_refs(node) {
            let raw = render_fileref(r);
            let (to_opt, issue_opt) = resolve(world, &id, r);
            if let Some(issue) = issue_opt {
                issues.push(issue);
            }
            if let Some(target) = &to_opt {
                reverse.entry(target.clone()).or_default().push(id.clone());
            }
            edges_here.push(Edge {
                from: id.clone(),
                to: to_opt,
                raw,
            });
        }
        forward.insert(id, edges_here);
    }

    // Cycle detection over resolved edges.
    for cycle in find_cycles(&forward) {
        let members: Vec<NodeId> = cycle.to_vec();
        let display = members
            .iter()
            .map(|m| format!("  - {m}"))
            .collect::<Vec<_>>()
            .join("\n");
        // Attribute the cycle issue to its lexicographically smallest member
        // so the report has a stable anchor.
        let mut sorted = members.clone();
        sorted.sort();
        let source = sorted.first().cloned().unwrap_or_else(|| NodeId(String::new()));
        issues.push(Issue {
            kind: IssueKind::Cycle,
            source,
            message: format!(
                "circular dependency among {} node(s):\n{display}",
                members.len()
            ),
            members,
        });
    }

    Graph {
        forward,
        reverse,
        issues,
    }
}

/// Extract the file path component from a [`NodeId`]'s canonical string
/// (everything before the `::` separator, or the whole string for file roots).
fn file_of(id: &NodeId) -> &str {
    match id.as_str().split_once("::") {
        Some((file, _)) => file,
        None => id.as_str(),
    }
}

/// Reconstruct a human-readable `[[...]]` string from a parsed [`FileRef`].
fn render_fileref(r: &FileRef) -> String {
    let mut s = String::from("[[");
    if let Some(p) = &r.path {
        s.push_str(p);
    }
    match &r.target {
        FileRefTarget::Document => {}
        FileRefTarget::Section(sec) => {
            s.push('#');
            s.push_str(sec);
        }
        FileRefTarget::Line(n) => {
            s.push_str(&format!("#L{n}"));
        }
        FileRefTarget::LineRange(a, b) => {
            s.push_str(&format!("#L{a}-L{b}"));
        }
    }
    if let Some(a) = &r.alias {
        s.push('|');
        s.push_str(a);
    }
    s.push_str("]]");
    s
}

/// All `FileRef`s belonging to a node (heading + body for sections; preamble
/// for file roots). Children are NOT included — they are separate nodes.
fn node_refs<'a>(node: NodeRef<'a>) -> Vec<&'a FileRef> {
    let mut out = Vec::new();
    match node {
        NodeRef::File(f) => refs_in_blocks(&f.preamble, &mut out),
        NodeRef::Section(s) => {
            refs_in_inlines(&s.heading, &mut out);
            refs_in_blocks(&s.body, &mut out);
        }
    }
    out
}

fn refs_in_blocks<'a>(blocks: &'a [Block], out: &mut Vec<&'a FileRef>) {
    for b in blocks {
        refs_in_block(b, out);
    }
}

fn refs_in_block<'a>(b: &'a Block, out: &mut Vec<&'a FileRef>) {
    match b {
        Block::Paragraph(v) | Block::Heading { inlines: v, .. } => refs_in_inlines(v, out),
        Block::BlockQuote(inner) => refs_in_blocks(inner, out),
        Block::List { items, .. } => {
            for item in items {
                refs_in_blocks(&item.blocks, out);
            }
        }
        // CodeBlock / ThematicBreak / HtmlBlock: no inline refs.
        _ => {}
    }
}

fn refs_in_inlines<'a>(
    inlines: &'a [nlc_parser::ast::Inline],
    out: &mut Vec<&'a FileRef>,
) {
    use nlc_parser::ast::Inline;
    for i in inlines {
        match i {
            Inline::FileRef(r) => out.push(r),
            Inline::Emphasis(v) | Inline::Strong(v) | Inline::Link { text: v, .. } => {
                refs_in_inlines(v, out);
            }
            _ => {}
        }
    }
}

/// Resolve a single [`FileRef`] to a [`NodeId`] (on success) or an [`Issue`]
/// (on failure). A reference may resolve successfully AND still produce no
/// issue.
///
/// `source_id` is the exact node (file root or section) that owns this ref —
/// issues are attributed to it so the incremental report can pin them to the
/// right place.
///
/// Resolution branches on the target file's kind:
///  * **Markdown** targets keep their existing semantics (`Document` /
///    `Section` / `Line` / `LineRange` all collapse to a file-root or section
///    [`NodeId`]).
///  * **Code** targets are line-only: `Document` resolves to the file root,
///    `Line`/`LineRange` resolve to granular `code.rs::L<n>` [`NodeId`]s after
///    a bounds check, and a named (`Section`) target is an
///    [`IssueKind::UnsupportedCodeTarget`] error since code files have no
///    sections.
fn resolve(
    world: &World,
    source_id: &NodeId,
    r: &FileRef,
) -> (Option<NodeId>, Option<Issue>) {
    let source_file = file_of(source_id);
    let Some(resolved) = resolve_file(world, source_file, r.path.as_deref()) else {
        let raw_path = r.path.clone().unwrap_or_else(|| source_file.to_string());
        return (
            None,
            Some(Issue {
                kind: IssueKind::MissingFile,
                source: source_id.clone(),
                message: format!("`{}` references missing file `{raw_path}`", source_id),
                members: Vec::new(),
            }),
        );
    };

    match resolved.kind {
        FileKind::Markdown(file_node) => {
            resolve_markdown(source_id, resolved.key, file_node, r)
        }
        FileKind::Code(code_file) => resolve_code(source_id, resolved.key, code_file, r),
    }
}

/// Resolve a [`FileRef`] against a Markdown target file.
fn resolve_markdown(
    source_id: &NodeId,
    file: &str,
    file_node: &crate::model::FileNode,
    r: &FileRef,
) -> (Option<NodeId>, Option<Issue>) {
    match &r.target {
        FileRefTarget::Document => (Some(NodeId::file(file)), None),
        FileRefTarget::Section(text) => {
            let want = slugify(text);
            let matches: Vec<&crate::model::Section> = file_node
                .walk_sections()
                .filter(|s| s.slug == want)
                .collect();
            match matches.len() {
                0 => (
                    None,
                    Some(Issue {
                        kind: IssueKind::MissingSection,
                        source: source_id.clone(),
                        message: format!(
                            "`{}` references missing section `{}` in {} (slug `{}`)",
                            source_id, text, file, want
                        ),
                        members: Vec::new(),
                    }),
                ),
                1 => (Some(matches[0].id(file)), None),
                _ => {
                    let candidates: Vec<NodeId> =
                        matches.iter().map(|s| s.id(file)).collect();
                    (
                        None,
                        Some(Issue {
                            kind: IssueKind::AmbiguousSection,
                            source: source_id.clone(),
                            message: format!(
                                "`{}` ambiguous section `{}` in {}: matches {} node(s)",
                                source_id,
                                text,
                                file,
                                candidates.len()
                            ),
                            members: candidates,
                        }),
                    )
                }
            }
        }
        FileRefTarget::Line(n) => {
            if *n == 0 || *n as usize > file_node.line_count {
                (
                    None,
                    Some(Issue {
                        kind: IssueKind::LineOutOfRange,
                        source: source_id.clone(),
                        message: format!(
                            "`{}` references line {} in {}, which has only {} line(s)",
                            source_id, n, file, file_node.line_count
                        ),
                        members: Vec::new(),
                    }),
                )
            } else {
                (Some(NodeId::file(file)), None)
            }
        }
        FileRefTarget::LineRange(a, b) => {
            if a > b || *b as usize > file_node.line_count {
                (
                    None,
                    Some(Issue {
                        kind: IssueKind::RangeOutOfRange,
                        source: source_id.clone(),
                        message: format!(
                            "`{}` references line range L{}-L{} in {}, which has only {} line(s)",
                            source_id, a, b, file, file_node.line_count
                        ),
                        members: Vec::new(),
                    }),
                )
            } else {
                (Some(NodeId::file(file)), None)
            }
        }
    }
}

/// Resolve a [`FileRef`] against a non-Markdown (code) target file. Code files
/// are line-only reference targets: named/section targets are unsupported, and
/// line/line-range targets resolve to granular `code.rs::L<n>` [`NodeId`]s
/// after a bounds check against the file's line count.
fn resolve_code(
    source_id: &NodeId,
    file: &str,
    code_file: &crate::model::CodeFile,
    r: &FileRef,
) -> (Option<NodeId>, Option<Issue>) {
    match &r.target {
        FileRefTarget::Document => (Some(NodeId::file(file)), None),
        FileRefTarget::Section(text) => (
            None,
            Some(Issue {
                kind: IssueKind::UnsupportedCodeTarget,
                source: source_id.clone(),
                message: format!(
                    "`{}` references `{}` by name `{}`, but code files only support `#L<line>` / `#L<a>-L<b>` targets",
                    source_id, file, text
                ),
                members: Vec::new(),
            }),
        ),
        FileRefTarget::Line(n) => {
            if *n == 0 || *n as usize > code_file.line_count {
                (
                    None,
                    Some(Issue {
                        kind: IssueKind::LineOutOfRange,
                        source: source_id.clone(),
                        message: format!(
                            "`{}` references line {} in {}, which has only {} line(s)",
                            source_id, n, file, code_file.line_count
                        ),
                        members: Vec::new(),
                    }),
                )
            } else {
                (Some(NodeId::code_line(file, *n)), None)
            }
        }
        FileRefTarget::LineRange(a, b) => {
            if a > b || *b as usize > code_file.line_count {
                (
                    None,
                    Some(Issue {
                        kind: IssueKind::RangeOutOfRange,
                        source: source_id.clone(),
                        message: format!(
                            "`{}` references line range L{}-L{} in {}, which has only {} line(s)",
                            source_id, a, b, file, code_file.line_count
                        ),
                        members: Vec::new(),
                    }),
                )
            } else {
                (Some(NodeId::code_range(file, *a, *b)), None)
            }
        }
    }
}

/// A file resolved from a `[[...]]` reference path: its canonical workspace key
/// plus the [`FileKind`], so [`resolve`] can apply Markdown vs code semantics.
struct ResolvedFile<'w> {
    key: &'w str,
    kind: FileKind<'w>,
}

/// Normalize a wikilink path to a workspace-relative file key, looking it up
/// across both Markdown and code files. Supports `[[name]]` → `name.md`
/// Markdown shorthand and strips a leading `./`.
fn resolve_file<'w>(world: &'w World, _source: &str, path: Option<&str>) -> Option<ResolvedFile<'w>> {
    let raw = path.unwrap_or(_source);
    let cleaned = raw.replace('\\', "/");
    let stripped = cleaned.strip_prefix("./").unwrap_or(&cleaned).to_string();

    if let Some((key, kind)) = world.file_kind(&stripped) {
        return Some(ResolvedFile { key, kind });
    }
    // Markdown-only shorthand: `[[name]]` → `name.md`. A `.md` path is always
    // routed to the Markdown map by collect, so this can only match Markdown.
    let with_md = format!("{stripped}.md");
    if let Some((key, kind)) = world.file_kind(&with_md) {
        return Some(ResolvedFile { key, kind });
    }
    if let Some((key, kind)) = world.file_kind(raw) {
        return Some(ResolvedFile { key, kind });
    }
    None
}

/// Find cyclic SCCs in the resolved-edge graph using iterative Kosaraju.
/// Returns each cyclic component (size > 1, plus self-loops) as a sorted
/// vector.
fn find_cycles(forward: &BTreeMap<NodeId, Vec<Edge>>) -> Vec<Vec<NodeId>> {
    // Build resolved-only forward and reverse adjacency.
    let mut fwd: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut rev: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut all_nodes: BTreeSet<NodeId> = BTreeSet::new();
    let mut self_loops: Vec<NodeId> = Vec::new();

    for (from, edges) in forward {
        all_nodes.insert(from.clone());
        for e in edges {
            if let Some(to) = &e.to {
                all_nodes.insert(to.clone());
                if to == from {
                    self_loops.push(from.clone());
                }
                fwd.entry(from.clone()).or_default().push(to.clone());
                rev.entry(to.clone()).or_default().push(from.clone());
            }
        }
    }

    // Pass 1: iterative DFS over forward graph, record finish order.
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut finish_order: Vec<NodeId> = Vec::new();
    for start in &all_nodes {
        if visited.contains(start) {
            continue;
        }
        let mut stack: Vec<(NodeId, std::vec::IntoIter<NodeId>)> = vec![(
            start.clone(),
            fwd.get(start).cloned().unwrap_or_default().into_iter(),
        )];
        visited.insert(start.clone());
        while let Some(frame) = stack.last_mut() {
            let neighbor = frame.1.next();
            match neighbor {
                None => {
                    let (v, _) = stack.pop().unwrap();
                    finish_order.push(v);
                }
                Some(w) => {
                    if !visited.contains(&w) {
                        visited.insert(w.clone());
                        stack.push((
                            w.clone(),
                            fwd.get(&w).cloned().unwrap_or_default().into_iter(),
                        ));
                    }
                }
            }
        }
    }

    // Pass 2: DFS over reverse graph in reverse finish order.
    let mut assigned: HashSet<NodeId> = HashSet::new();
    let mut sccs: Vec<Vec<NodeId>> = Vec::new();
    while let Some(start) = finish_order.pop() {
        if assigned.contains(&start) {
            continue;
        }
        let mut scc: Vec<NodeId> = Vec::new();
        let mut stack = vec![start.clone()];
        assigned.insert(start.clone());
        while let Some(v) = stack.pop() {
            scc.push(v.clone());
            if let Some(preds) = rev.get(&v) {
                for p in preds {
                    if !assigned.contains(p) {
                        assigned.insert(p.clone());
                        stack.push(p.clone());
                    }
                }
            }
        }
        sccs.push(scc);
    }

    // Cyclic components: SCCs of size > 1, plus singleton self-loops.
    let mut cycles: Vec<Vec<NodeId>> = sccs
        .into_iter()
        .filter(|scc| {
            if scc.len() > 1 {
                return true;
            }
            // Singleton: cycle only if it has a self-edge.
            let n = &scc[0];
            self_loops.iter().any(|s| s == n)
        })
        .map(|mut scc| {
            scc.sort();
            scc
        })
        .collect();
    cycles.sort();
    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::build_file_node;

    fn world_from<I>(files: I) -> World
    where
        I: IntoIterator<Item = (&'static str, &'static str)>,
    {
        let mut w = World::default();
        for (path, src) in files {
            let doc = nlc_parser::parse(src).unwrap();
            let f = build_file_node(path.to_string(), doc.blocks, src.lines().count());
            w.files.insert(path.to_string(), f);
        }
        w
    }

    /// Register code files (path, source) on a world; line_count is derived
    /// from the source, matching how `collect` builds them.
    fn with_code(w: &mut World, files: &[(&str, &str)]) {
        for (path, src) in files {
            w.code_files.insert(
                path.to_string(),
                crate::model::CodeFile {
                    line_count: src.lines().count(),
                },
            );
        }
    }

    #[test]
    fn resolves_section_ref() {
        let w = world_from([
            ("a.md", "See [[b.md#Setup]] for details.\n"),
            ("b.md", "# Setup\nbody\n"),
        ]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
        let a_root = NodeId::file("a.md");
        let edges = g.forward.get(&a_root).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to.as_ref().unwrap().as_str(), "b.md::setup");
    }

    #[test]
    fn detects_dangling_section() {
        let w = world_from([("a.md", "[[b.md#Nope]]\n"), ("b.md", "# Setup\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::MissingSection);
    }

    #[test]
    fn detects_missing_file() {
        let w = world_from([("a.md", "[[ghost.md]]\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::MissingFile);
    }

    #[test]
    fn detects_ambiguous_section() {
        let w = world_from([
            ("a.md", "[[b.md#Setup]]\n"),
            ("b.md", "# Top\n## Setup\none\n# Other\n## Setup\ntwo\n"),
        ]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::AmbiguousSection);
    }

    #[test]
    fn self_ref_via_section_in_own_doc() {
        let w = world_from([("a.md", "# Intro\nsee [[#Next]]\n## Next\nbody\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
    }

    #[test]
    fn detects_cycle() {
        // Document-level refs in the preamble form a 2-node cycle:
        // a.md (root) -> b.md (root) -> a.md (root).
        let w = world_from([
            ("a.md", "[[b.md]]\n"),
            ("b.md", "[[a.md]]\n"),
        ]);
        let g = build(&w);
        let cycles: Vec<_> = g
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Cycle)
            .collect();
        assert_eq!(cycles.len(), 1, "{:?}", g.issues);
        assert_eq!(cycles[0].members.len(), 2);
    }

    #[test]
    fn detects_cycle_between_sections() {
        let w = world_from([
            ("a.md", "# A\n[[b.md#B]]\n"),
            ("b.md", "# B\n[[a.md#A]]\n"),
        ]);
        let g = build(&w);
        let cycles: Vec<_> = g
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Cycle)
            .collect();
        assert_eq!(cycles.len(), 1, "{:?}", g.issues);
    }

    #[test]
    fn line_range_out_of_bounds() {
        let w = world_from([
            ("a.md", "[[b.md#L10-L20]]\n"),
            ("b.md", "only\none\nline\n"),
        ]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::RangeOutOfRange);
    }

    #[test]
    fn md_extension_shorthand() {
        let w = world_from([("a.md", "[[b]]\n"), ("b.md", "hello\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
    }

    #[test]
    fn reverse_edges_populated() {
        let w = world_from([
            ("a.md", "[[b.md]]\n"),
            ("b.md", "hi\n"),
        ]);
        let g = build(&w);
        let b_root = NodeId::file("b.md");
        let referrers = g.referrers(&b_root);
        assert_eq!(referrers, &[NodeId::file("a.md")]);
    }

    #[test]
    fn resolves_code_line_ref() {
        let mut w = world_from([("a.md", "[[main.rs#L2]]\n")]);
        with_code(&mut w, &[("main.rs", "fn main() {}\nfn other() {}\nfn third() {}\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
        let edges = g.forward.get(&NodeId::file("a.md")).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to.as_ref().unwrap().as_str(), "main.rs::L2");
    }

    #[test]
    fn resolves_code_line_range_ref() {
        let mut w = world_from([("a.md", "[[main.rs#L1-L3]]\n")]);
        with_code(&mut w, &[("main.rs", "a\nb\nc\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
        let edges = g.forward.get(&NodeId::file("a.md")).unwrap();
        assert_eq!(edges[0].to.as_ref().unwrap().as_str(), "main.rs::L1-3");
    }

    #[test]
    fn resolves_code_document_ref() {
        let mut w = world_from([("a.md", "[[main.rs]]\n")]);
        with_code(&mut w, &[("main.rs", "fn main() {}\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
        // A whole-code-file ref resolves to the file root NodeId.
        assert_eq!(
            g.forward.get(&NodeId::file("a.md")).unwrap()[0]
                .to
                .as_ref()
                .unwrap()
                .as_str(),
            "main.rs"
        );
    }

    #[test]
    fn code_line_out_of_range() {
        let mut w = world_from([("a.md", "[[main.rs#L99]]\n")]);
        with_code(&mut w, &[("main.rs", "only\nthree\nlines\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::LineOutOfRange);
    }

    #[test]
    fn code_line_range_out_of_bounds() {
        let mut w = world_from([("a.md", "[[main.rs#L2-L50]]\n")]);
        with_code(&mut w, &[("main.rs", "a\nb\nc\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::RangeOutOfRange);
    }

    #[test]
    fn code_named_target_is_unsupported() {
        let mut w = world_from([("a.md", "[[main.rs#main]]\n")]);
        with_code(&mut w, &[("main.rs", "fn main() {}\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::UnsupportedCodeTarget);
        assert!(g.issues[0].message.contains("#L<line>"));
    }

    #[test]
    fn missing_code_file() {
        let w = world_from([("a.md", "[[ghost.rs#L1]]\n")]);
        let g = build(&w);
        assert_eq!(g.issues.len(), 1);
        assert_eq!(g.issues[0].kind, IssueKind::MissingFile);
    }

    #[test]
    fn markdown_line_ref_still_collapses_to_file_root() {
        // Markdown line refs keep their pre-existing semantics: an in-range
        // line/range ref resolves to the file root, NOT a granular node.
        let w = world_from([("a.md", "[[b.md#L1]]\n"), ("b.md", "hi\n")]);
        let g = build(&w);
        assert!(g.issues.is_empty(), "{:?}", g.issues);
        assert_eq!(
            g.forward.get(&NodeId::file("a.md")).unwrap()[0]
                .to
                .as_ref()
                .unwrap()
                .as_str(),
            "b.md"
        );
    }
}
