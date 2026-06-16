//! The unified workspace snapshot.
//!
//! This is the single data structure every command consumes. Building it
//! performs the full analysis pipeline once:
//!
//! 1. Scan the workspace root for `.md` files ([`collect`]).
//! 2. Parse each into a hierarchical section tree ([`World`]).
//! 3. Build the cross-reference [`Graph`] (edges + issues).
//! 4. Hash every node ([`hash`]).
//! 5. Diff against the on-disk [`Cache`] → [`Delta`] (changed / affected /
//!    revalidated / regressions / resolutions).
//! 6. Derive the [`Cache`] to persist on the next `check`.
//!
//! Commands then design a [`crate::printer::Printer`] that reads from the
//! [`Snapshot`] and renders whatever view they need. No command re-derives the
//! world, graph, or hashes — they all share this one structure, which keeps
//! behavior consistent and makes new commands cheap to add.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::cache::{Cache, CacheLoadResult, Verdict};
use crate::collect::{collect, CollectError};
use crate::graph::{self, Graph};
use crate::hash;
use crate::model::{NodeId, World};

/// How a single node's content changed relative to the previous run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The change-delta of one analysis pass against the previous cache.
#[derive(Debug, Default, Clone)]
pub struct Delta {
    /// Per-node content change (added / modified / removed), in NodeId order.
    pub changes: Vec<ChangeRecord>,
    /// Nodes pulled in by reverse-edge propagation (not themselves changed).
    pub affected: BTreeSet<NodeId>,
    /// Revalidation set = changed-and-present ∪ affected.
    pub revalidated: BTreeSet<NodeId>,
    /// Nodes that were ok last run and are broken now.
    pub regressions: Vec<NodeId>,
    /// Nodes that were broken last run and are ok now.
    pub resolved: Vec<NodeId>,
    /// Current per-node issue counts (the source of truth for verdicts).
    #[allow(dead_code)]
    pub issue_counts: BTreeMap<NodeId, u32>,
}

impl Delta {
    /// "Affected" filtered to exclude nodes that are themselves content-changed.
    pub fn affected_only(&self) -> BTreeSet<&NodeId> {
        let changed: BTreeSet<&NodeId> = self
            .changes
            .iter()
            .filter(|c| c.change != NodeChange::Removed)
            .map(|c| &c.node)
            .collect();
        self.affected.iter().filter(|n| !changed.contains(*n)).collect()
    }
}

/// What we found on disk regarding the cache file.
#[derive(Debug, Default, Clone, Copy)]
pub struct CacheMeta {
    pub existed: bool,
    pub corrupted: bool,
}

/// A fully-analyzed workspace. Every command reads from this.
#[derive(Debug)]
pub struct Snapshot {
    #[allow(dead_code)]
    pub root: PathBuf,
    pub cache_path: PathBuf,
    pub world: World,
    pub graph: Graph,
    pub hashes: BTreeMap<NodeId, String>,
    pub parse_errors: Vec<CollectError>,
    pub delta: Delta,
    /// The cache that *should* be written by `check` to reflect this run.
    pub next_cache: Cache,
    pub cache_meta: CacheMeta,
}

impl Snapshot {
    /// Full build with I/O: scan `root`, load any existing cache, analyze.
    /// This is what the CLI calls.
    pub fn build(root: &Path) -> Snapshot {
        let cache_path = root.join(".nlc-cache");
        let CacheLoadResult {
            cache: prev,
            existed,
            corrupted,
        } = Cache::load(&cache_path);
        let collected = collect(root);
        Snapshot::analyze(
            root.to_path_buf(),
            cache_path,
            collected.world,
            collected.errors,
            prev,
            CacheMeta { existed, corrupted },
        )
    }

    /// Pure analysis from an already-collected [`World`] and previous [`Cache`].
    /// Used by the CLI (after [`collect`]) and by tests (with an in-memory
    /// world).
    pub fn analyze(
        root: PathBuf,
        cache_path: PathBuf,
        world: World,
        parse_errors: Vec<CollectError>,
        prev: Cache,
        cache_meta: CacheMeta,
    ) -> Snapshot {
        let graph = graph::build(&world);

        // Current content hashes for every node.
        let mut hashes: BTreeMap<NodeId, String> = BTreeMap::new();
        for file in world.files.values() {
            for (id, h) in hash::hash_file_tree(file) {
                hashes.insert(id, h);
            }
        }

        // Diff against the cache → changed / added / removed.
        let mut changes: Vec<ChangeRecord> = Vec::new();
        let mut changed_current: Vec<NodeId> = Vec::new();
        let mut removed: Vec<NodeId> = Vec::new();

        for (id, h) in &hashes {
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
            if !hashes.contains_key(id) {
                changes.push(ChangeRecord {
                    node: id.clone(),
                    change: NodeChange::Removed,
                });
                removed.push(id.clone());
            }
        }

        // Propagate dirtiness along reverse edges.
        let affected = propagate_affected(&graph.reverse, &changed_current, &removed);

        let mut revalidated: BTreeSet<NodeId> = BTreeSet::new();
        for id in &changed_current {
            revalidated.insert(id.clone());
        }
        for id in &affected {
            revalidated.insert(id.clone());
        }

        // Per-node issue counts.
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
        for id in hashes.keys() {
            if let Some(entry) = prev.get(id)
                && !entry.verdict.is_ok()
                && issue_counts.get(id).copied().unwrap_or(0) == 0
            {
                resolved.push(id.clone());
            }
        }

        // Build the next cache: every current node gets its fresh hash + verdict.
        let mut next_cache = Cache::default();
        for (id, h) in &hashes {
            let count = issue_counts.get(id).copied().unwrap_or(0);
            let verdict = if count == 0 {
                Verdict::Ok
            } else {
                Verdict::Err(count)
            };
            next_cache.insert(id.clone(), h.clone(), verdict);
        }

        Snapshot {
            root,
            cache_path,
            world,
            graph,
            hashes,
            parse_errors,
            delta: Delta {
                changes,
                affected,
                revalidated,
                regressions,
                resolved,
                issue_counts,
            },
            next_cache,
            cache_meta,
        }
    }

    /// Total node count (file roots + sections).
    pub fn total_nodes(&self) -> usize {
        self.hashes.len()
    }

    /// Nodes that are present, unchanged, and unaffected.
    pub fn up_to_date_count(&self) -> usize {
        self.total_nodes().saturating_sub(self.delta.revalidated.len())
    }

    /// True if there are any validation or parse errors.
    pub fn has_errors(&self) -> bool {
        !self.graph.issues.is_empty() || !self.parse_errors.is_empty()
    }
}

/// Transitive closure of reverse edges starting from `seeds`. `removed_seeds`
/// are also tried (in case they still appear as targets of current edges).
fn propagate_affected(
    reverse: &BTreeMap<NodeId, Vec<NodeId>>,
    seeds: &[NodeId],
    removed_seeds: &[NodeId],
) -> BTreeSet<NodeId> {
    let mut out: BTreeSet<NodeId> = BTreeSet::new();
    let mut stack: Vec<NodeId> = seeds.iter().chain(removed_seeds.iter()).cloned().collect();
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

    fn analyze(world: World, prev: &Cache) -> Snapshot {
        Snapshot::analyze(
            PathBuf::from("."),
            PathBuf::from(".nlc-cache"),
            world,
            Vec::new(),
            prev.clone(),
            CacheMeta::default(),
        )
    }

    #[test]
    fn first_run_marks_everything_added() {
        let w = world_from([("a.md", "# A\n[[b.md]]\n"), ("b.md", "# B\n")]);
        let s = analyze(w, &Cache::default());
        assert!(s.delta.changes.iter().all(|c| c.change == NodeChange::Added));
        assert!(s.graph.issues.is_empty(), "{:?}", s.graph.issues);
        assert_eq!(s.total_nodes(), 4);
    }

    #[test]
    fn second_run_with_no_changes_is_clean() {
        let w = world_from([("a.md", "# A\n[[b.md]]\n"), ("b.md", "# B\n")]);
        let s1 = analyze(w.clone(), &Cache::default());
        let s2 = analyze(w, &s1.next_cache);
        assert!(s2.delta.changes.is_empty(), "{:?}", s2.delta.changes);
        assert!(s2.delta.revalidated.is_empty());
        assert!(s2.graph.issues.is_empty());
    }

    #[test]
    fn edit_propagates_to_referrer() {
        let w1 = world_from([("a.md", "# A\nsee [[b.md#Part]]\n"), ("b.md", "# Part\nold\n")]);
        let s1 = analyze(w1, &Cache::default());

        let w2 = world_from([("a.md", "# A\nsee [[b.md#Part]]\n"), ("b.md", "# Part\nnew body\n")]);
        let s2 = analyze(w2, &s1.next_cache);

        let b_part = NodeId("b.md::part".into());
        let a_section = NodeId("a.md::a".into());
        assert!(s2
            .delta
            .changes
            .iter()
            .any(|c| c.node == b_part && c.change == NodeChange::Modified));
        assert!(s2
            .delta
            .changes
            .iter()
            .any(|c| c.node == NodeId("b.md".into())));
        assert!(s2.delta.affected.contains(&a_section), "affected={:?}", s2.delta.affected);
    }

    #[test]
    fn dangling_ref_is_regression_then_resolution() {
        let w1 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Part\n")]);
        let s1 = analyze(w1, &Cache::default());

        let w2 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Other\n")]);
        let s2 = analyze(w2, &s1.next_cache);
        assert!(!s2.graph.issues.is_empty());
        assert!(s2.delta.regressions.contains(&NodeId("a.md".into())));

        let w3 = world_from([("a.md", "[[b.md#Other]]\n"), ("b.md", "# Other\n")]);
        let s3 = analyze(w3, &s2.next_cache);
        assert!(s3.graph.issues.is_empty(), "{:?}", s3.graph.issues);
    }

    #[test]
    fn removed_section_dangling_detected() {
        let w1 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "# Part\n")]);
        let s1 = analyze(w1, &Cache::default());

        let w2 = world_from([("a.md", "[[b.md#Part]]\n"), ("b.md", "no headings here\n")]);
        let s2 = analyze(w2, &s1.next_cache);
        assert!(s2
            .graph
            .issues
            .iter()
            .any(|i| i.kind == crate::graph::IssueKind::MissingSection));
    }
}
