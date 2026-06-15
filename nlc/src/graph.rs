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
use crate::model::{NodeId, NodeRef, World};

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
/// issue; line/range refs that are in-range resolve to the file root.
///
/// `source_id` is the exact node (file root or section) that owns this ref —
/// issues are attributed to it so the incremental report can pin them to the
/// right place.
fn resolve(
    world: &World,
    source_id: &NodeId,
    r: &FileRef,
) -> (Option<NodeId>, Option<Issue>) {
    let source_file = file_of(source_id);
    let target_file = resolve_file(world, source_file, r.path.as_deref());
    let Some(file) = target_file else {
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

    let file_node = world.file(file).expect("file present after resolve_file");

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

/// Normalize a wikilink path to a workspace-relative file key, looking it up
/// in `world.files`. Supports `[[name]]` → `name.md` shorthand and strips a
/// leading `./`.
fn resolve_file<'w>(world: &'w World, _source: &str, path: Option<&str>) -> Option<&'w str> {
    let raw = path.unwrap_or(_source);
    let cleaned = raw.replace('\\', "/");
    let stripped = cleaned.strip_prefix("./").unwrap_or(&cleaned).to_string();

    if let Some((key, _)) = world.files.get_key_value(&stripped) {
        return Some(key.as_str());
    }
    let with_md = format!("{stripped}.md");
    if let Some((key, _)) = world.files.get_key_value(&with_md) {
        return Some(key.as_str());
    }
    if let Some((key, _)) = world.files.get_key_value(raw) {
        return Some(key.as_str());
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
}
