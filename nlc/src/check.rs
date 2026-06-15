//! The incremental validation driver.
//!
//! This module ties together collection, hashing, graph building, and the
//! on-disk cache to produce the make-like "what changed, what did I rebuild"
//! experience:
//!
//! 1. Scan and parse every `.md` file in the workspace.
//! 2. Build the cross-reference graph (always — it is cheap and the source of
//!    all validation issues).
//! 3. Hash every node and diff against the cache → the *changed* set.
//! 4. Propagate dirtiness along reverse reference edges → the *affected* set.
//! 5. Classify every node's current issues against its cached verdict to spot
//!    regressions and resolutions.
//! 6. Print a focused report and persist the new cache.
//!
//! Because the graph is rebuilt every run, the set of issues reported is always
//! complete and current. The cache does not skip correctness checks — it
//! powers the *delta*: which nodes changed, which dependents were affected,
//! and which nodes flipped between ok / broken since the last run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::cache::{Cache, CacheLoadResult, Verdict};
use crate::collect::{collect, CollectError};
use crate::graph::{self, Issue};
use crate::hash;
use crate::model::{NodeId, World};

#[derive(Debug, Default, Clone)]
pub struct CheckOptions {
    /// `--full`: report every node's issues, not only the delta.
    pub full: bool,
}

/// How a single node's content changed relative to the previous run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeChange {
    Added,
    Modified,
    Removed,
}

/// A focused description of one node's place in the delta.
#[derive(Debug, Clone)]
pub struct ChangeRecord {
    pub node: NodeId,
    pub change: NodeChange,
}

/// The complete result of an analysis pass — pure data, no I/O.
#[derive(Debug, Default)]
pub struct Analysis {
    pub files_scanned: usize,
    pub sections_scanned: usize,
    pub parse_errors: Vec<CollectError>,
    pub changes: Vec<ChangeRecord>,
    pub affected: BTreeSet<NodeId>,
    /// Nodes whose cached verdict disagrees with the current issue count —
    /// i.e. they newly broke (`was_ok`) or newly fixed (`now_ok`).
    pub regressions: Vec<NodeId>,
    pub resolved: Vec<NodeId>,
    pub issues: Vec<Issue>,
    /// Current per-node issue counts (the source of truth for verdicts).
    #[allow(dead_code)]
    pub issue_counts: BTreeMap<NodeId, u32>,
    pub current_hashes: BTreeMap<NodeId, String>,
    /// Revalidation set = changed (current) ∪ affected.
    pub revalidated: BTreeSet<NodeId>,
    pub total_nodes: usize,
    pub cache_existed: bool,
    pub cache_corrupted: bool,
}

impl Analysis {
    pub fn has_errors(&self) -> bool {
        !self.issues.is_empty() || !self.parse_errors.is_empty()
    }

    pub fn up_to_date_count(&self) -> usize {
        // Nodes that are present, unchanged, and not affected.
        self.total_nodes.saturating_sub(self.revalidated.len())
    }
}

/// Pure analysis: given an already-collected [`World`] and the previous
/// [`Cache`], compute the full [`Analysis`] plus the [`Cache`] to persist for
/// next time.
pub fn analyze(
    world: &World,
    parse_errors: Vec<CollectError>,
    prev: &Cache,
    _opts: &CheckOptions,
) -> (Analysis, Cache) {
    let graph = graph::build(world);

    // Current content hashes for every node.
    let mut current_hashes: BTreeMap<NodeId, String> = BTreeMap::new();
    for file in world.files.values() {
        for (id, h) in hash::hash_file_tree(file) {
            current_hashes.insert(id, h);
        }
    }

    // Diff against the cache → changed / added / removed.
    let mut changes: Vec<ChangeRecord> = Vec::new();
    let mut changed_current: Vec<NodeId> = Vec::new();
    let mut removed: Vec<NodeId> = Vec::new();

    for (id, h) in &current_hashes {
        match prev.get(id) {
            None => {
                changes.push(ChangeRecord {
                    node: id.clone(),
                    change: NodeChange::Added,
                });
                changed_current.push(id.clone());
            }
            Some(entry) if entry.hash != *h => {
                changes.push(ChangeRecord {
                    node: id.clone(),
                    change: NodeChange::Modified,
                });
                changed_current.push(id.clone());
            }
            _ => {}
        }
    }
    for id in prev.entries.keys() {
        if !current_hashes.contains_key(id) {
            changes.push(ChangeRecord {
                node: id.clone(),
                change: NodeChange::Removed,
            });
            removed.push(id.clone());
        }
    }

    // Propagate dirtiness along reverse edges. Seeds include removed nodes so
    // that (when their referrers are discoverable in the current graph) the
    // ripple is reported. Referrers of fully-vanished nodes are not in the
    // current reverse map; their newly-dangling edges still surface as issues.
    let affected = propagate_affected(&graph.reverse, &changed_current, &removed);

    let mut revalidated: BTreeSet<NodeId> = BTreeSet::new();
    for id in &changed_current {
        revalidated.insert(id.clone());
    }
    for id in &affected {
        revalidated.insert(id.clone());
    }

    // Per-node issue counts from the freshly-built graph.
    let mut issue_counts: BTreeMap<NodeId, u32> = BTreeMap::new();
    for issue in &graph.issues {
        *issue_counts.entry(issue.source.clone()).or_default() += 1;
    }

    // Regressions / resolutions vs cached verdicts.
    let mut regressions: Vec<NodeId> = Vec::new();
    let mut resolved: Vec<NodeId> = Vec::new();
    for (id, &count) in &issue_counts {
        let was_ok = prev.get(id).map(|e| e.verdict.is_ok()).unwrap_or(true);
        if was_ok && count > 0 {
            regressions.push(id.clone());
        }
    }
    for id in current_hashes.keys() {
        if let Some(entry) = prev.get(id)
            && !entry.verdict.is_ok()
            && issue_counts.get(id).copied().unwrap_or(0) == 0
        {
            resolved.push(id.clone());
        }
    }
    // Build the next cache: every current node gets its fresh hash + verdict.
    let mut next = Cache::default();
    for (id, h) in &current_hashes {
        let count = issue_counts.get(id).copied().unwrap_or(0);
        let verdict = if count == 0 {
            Verdict::Ok
        } else {
            Verdict::Err(count)
        };
        next.insert(id.clone(), h.clone(), verdict);
    }

    let analysis = Analysis {
        files_scanned: world.files.len(),
        sections_scanned: world.section_count(),
        parse_errors,
        changes,
        affected,
        regressions,
        resolved,
        issues: graph.issues,
        issue_counts,
        current_hashes,
        revalidated,
        total_nodes: 0,
        cache_existed: false,
        cache_corrupted: false,
    };
    let mut analysis = analysis;
    analysis.total_nodes = analysis.current_hashes.len();
    (analysis, next)
}

/// Transitive closure of reverse edges starting from `seeds`. `removed_seeds`
/// are also tried (in case they still appear as targets of current edges,
/// which happens when only part of a subtree vanished).
fn propagate_affected(
    reverse: &BTreeMap<NodeId, Vec<NodeId>>,
    seeds: &[NodeId],
    removed_seeds: &[NodeId],
) -> BTreeSet<NodeId> {
    let mut out: BTreeSet<NodeId> = BTreeSet::new();
    let mut stack: Vec<NodeId> = Vec::new();
    for s in seeds.iter().chain(removed_seeds.iter()) {
        stack.push(s.clone());
    }
    while let Some(n) = stack.pop() {
        if let Some(referrers) = reverse.get(&n) {
            for r in referrers {
                if out.insert(r.clone()) {
                    stack.push(r.clone());
                }
            }
        }
    }
    out
}

/// Top-level driver: scan `root`, load the cache, analyze, persist the cache,
/// and print the report. Returns the process exit code.
pub fn run(root: &Path, opts: &CheckOptions) -> i32 {
    let cache_path = root.join(".nlc-cache");
    let CacheLoadResult {
        cache: prev,
        existed,
        corrupted,
    } = Cache::load(&cache_path);

    let collected = collect(root);
    let (mut analysis, next) = analyze(
        &collected.world,
        collected.errors,
        &prev,
        opts,
    );
    analysis.cache_existed = existed;
    analysis.cache_corrupted = corrupted;

    let exit = print_report(&analysis, opts);

    if let Err(e) = next.save(&cache_path) {
        eprintln!("nlc: warning: failed to write cache: {e}");
    }
    exit
}

/// Print the human-readable report. Returns the exit code (0 clean, 1 errors).
pub fn print_report(a: &Analysis, opts: &CheckOptions) -> i32 {
    let mut out = String::new();

    // Header / summary.
    out.push_str(&format!(
        "nlc: scanned {} file(s), {} section(s)\n",
        a.files_scanned, a.sections_scanned
    ));
    if a.cache_existed && a.cache_corrupted {
        out.push_str("nlc: warning: existing cache was corrupted; treating as a fresh run\n");
    }

    if !a.parse_errors.is_empty() {
        out.push_str(&format!("\nparse errors ({}):\n", a.parse_errors.len()));
        for e in &a.parse_errors {
            out.push_str(&format!("  {}: {}\n", e.path, e.message));
        }
    }

    // Delta.
    let added = a.changes.iter().filter(|c| c.change == NodeChange::Added).count();
    let modified = a.changes.iter().filter(|c| c.change == NodeChange::Modified).count();
    let removed = a.changes.iter().filter(|c| c.change == NodeChange::Removed).count();
    // "Affected" = pulled in by propagation but not itself content-changed.
    let changed_set: BTreeSet<&NodeId> = a
        .changes
        .iter()
        .filter(|c| c.change != NodeChange::Removed)
        .map(|c| &c.node)
        .collect();
    let affected_only: BTreeSet<&NodeId> = a
        .affected
        .iter()
        .filter(|n| !changed_set.contains(*n))
        .collect();
    out.push_str(&format!(
        "  changed: {} node(s) ({} modified, {} added, {} removed)\n",
        a.changes.len(),
        modified,
        added,
        removed
    ));
    out.push_str(&format!(
        "  revalidated: {} node(s) ({} changed + {} affected)\n",
        a.revalidated.len(),
        changed_set.len(),
        affected_only.len()
    ));
    out.push_str(&format!(
        "  up-to-date: {} node(s)\n",
        a.up_to_date_count()
    ));

    // Changed-node detail (make-style: show what triggered the rebuild).
    if !a.changes.is_empty() {
        out.push_str("\nchanged nodes:\n");
        for c in &a.changes {
            let label = match c.change {
                NodeChange::Added => "added",
                NodeChange::Modified => "modified",
                NodeChange::Removed => "removed",
            };
            out.push_str(&format!("  [{label}] {}\n", c.node));
        }
    }
    if !affected_only.is_empty() {
        out.push_str("\naffected (rebuilt by propagation):\n");
        for id in &affected_only {
            out.push_str(&format!("  {id}\n"));
        }
    }

    // Issues — full detail for revalidated nodes; for the rest, only with --full.
    let issues_to_show: Vec<&Issue> = if opts.full {
        a.issues.iter().collect()
    } else {
        a.issues
            .iter()
            .filter(|i| a.revalidated.contains(&i.source))
            .collect()
    };
    if !issues_to_show.is_empty() {
        out.push_str(&format!("\nissues ({}):\n", issues_to_show.len()));
        for issue in &issues_to_show {
            out.push_str(&format!("  error: {}\n", issue.message));
        }
        if !opts.full {
            let hidden = a.issues.len() - issues_to_show.len();
            if hidden > 0 {
                out.push_str(&format!(
                    "  ({hidden} more on up-to-date nodes; use --full to show)\n"
                ));
            }
        }
    }

    // Regressions / resolutions.
    if !a.regressions.is_empty() {
        out.push_str("\nregressions (were ok, now broken):\n");
        for id in &a.regressions {
            out.push_str(&format!("  {id}\n"));
        }
    }
    if !a.resolved.is_empty() {
        out.push_str("\nresolved (were broken, now ok):\n");
        for id in &a.resolved {
            out.push_str(&format!("  {id}\n"));
        }
    }

    // Verdict line.
    let issue_count = a.issues.len();
    let parse_err_count = a.parse_errors.len();
    let total_errors = issue_count + parse_err_count;
    if total_errors == 0 {
        out.push_str("\nok\n");
    } else {
        out.push_str(&format!("\n{total_errors} error(s)\n"));
    }

    print!("{out}");
    if a.has_errors() {
        1
    } else {
        0
    }
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

    fn opts() -> CheckOptions {
        CheckOptions::default()
    }

    #[test]
    fn first_run_marks_everything_added() {
        let w = world_from([("a.md", "# A\n[[b.md]]\n"), ("b.md", "# B\n")]);
        let (a, _next) = analyze(&w, vec![], &Cache::default(), &opts());
        assert!(a.changes.iter().all(|c| c.change == NodeChange::Added));
        // No issues: b.md resolves.
        assert!(a.issues.is_empty(), "{:?}", a.issues);
        assert_eq!(a.total_nodes, 4); // 2 roots + 2 sections
    }

    #[test]
    fn second_run_with_no_changes_is_clean() {
        let w = world_from([("a.md", "# A\n[[b.md]]\n"), ("b.md", "# B\n")]);
        let (_a1, next) = analyze(&w, vec![], &Cache::default(), &opts());
        let (a2, _next2) = analyze(&w, vec![], &next, &opts());
        assert!(a2.changes.is_empty(), "{:?}", a2.changes);
        assert!(a2.revalidated.is_empty());
        assert!(a2.issues.is_empty());
    }

    #[test]
    fn edit_propagates_to_referrer() {
        let w1 = world_from([("a.md", "# A\nsee [[b.md#Part]]\n"), ("b.md", "# Part\nold\n")]);
        let (_a1, next) = analyze(&w1, vec![], &Cache::default(), &opts());

        // Edit b.md's Part section. a.md's section A should become affected.
        let w2 = world_from([("a.md", "# A\nsee [[b.md#Part]]\n"), ("b.md", "# Part\nnew body\n")]);
        let (a2, _next2) = analyze(&w2, vec![], &next, &opts());

        let b_part = NodeId("b.md::part".into());
        let a_section = NodeId("a.md::a".into());
        // b's Part section is modified.
        assert!(a2
            .changes
            .iter()
            .any(|c| c.node == b_part && c.change == NodeChange::Modified));
        // b's root is also modified (hierarchical hash).
        assert!(a2
            .changes
            .iter()
            .any(|c| c.node == NodeId("b.md".into())));
        // a.md::a (the section containing the ref) is affected via the reverse
        // edge from b.md::part.
        assert!(a2.affected.contains(&a_section),
            "affected={:?}", a2.affected);
    }

    #[test]
    fn dangling_ref_is_regression_then_resolution() {
        // Run 1: clean.
        let w1 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Part\n")]);
        let (_a1, next) = analyze(&w1, vec![], &Cache::default(), &opts());

        // Run 2: rename Part → Other. a.md now has a dangling ref.
        let w2 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Other\n")]);
        let (a2, _next2) = analyze(&w2, vec![], &next, &opts());
        assert!(!a2.issues.is_empty());
        assert!(a2.regressions.contains(&NodeId("a.md".into())));

        // Run 3: fix a.md to point at Other.
        let w3 = world_from([("a.md", "[[b.md#Other]]\n"), ("b.md", "# Other\n")]);
        let (a3, _next3) = analyze(&w3, vec![], &_next2, &opts());
        assert!(a3.issues.is_empty(), "{:?}", a3.issues);
    }

    #[test]
    fn removed_section_dangling_detected() {
        let w1 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Part\n")]);
        let (_a1, next) = analyze(&w1, vec![], &Cache::default(), &opts());

        // Delete the Part heading entirely.
        let w2 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "no headings here\n")]);
        let (a2, _next2) = analyze(&w2, vec![], &next, &opts());
        assert!(a2.issues.iter().any(|i| {
            i.kind == crate::graph::IssueKind::MissingSection
        }));
    }
}
