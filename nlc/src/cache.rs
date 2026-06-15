//! On-disk cache of per-node content hashes and last-run validation verdicts.
//!
//! The cache is a simple line-oriented text file (`.nlc-cache` in the
//! workspace root) so it is diffable and inspectable. Each entry is one line:
//!
//! ```text
//! <node-id>\t<hash>\t<verdict>
//! ```
//!
//! `verdict` is either `ok` or `err:<count>` (the number of issues that
//! affected this node on the previous run). The file is written atomically via
//! a temp-file + rename so a crashed run cannot leave a half-written cache.
//!
//! The cache only stores *facts* needed to skip work on a subsequent run: the
//! content hash (to detect changes) and a coarse verdict (so unchanged nodes
//! can be summarized without revalidation).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::model::NodeId;

const HEADER: &str = "nlc-cache v1";

/// Coarse result of validating a node on a previous run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// Validation produced this many issues attributable to the node.
    Err(u32),
}

impl Verdict {
    pub fn is_ok(self) -> bool {
        matches!(self, Verdict::Ok)
    }

    fn encode(self) -> String {
        match self {
            Verdict::Ok => "ok".to_string(),
            Verdict::Err(n) => format!("err:{n}"),
        }
    }

    fn decode(s: &str) -> Verdict {
        if let Some(rest) = s.strip_prefix("err:") {
            Verdict::Err(rest.parse().unwrap_or(0))
        } else {
            Verdict::Ok
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub hash: String,
    pub verdict: Verdict,
}

/// The in-memory mirror of the cache file: a map from [`NodeId`] to its last
/// recorded hash and verdict.
#[derive(Debug, Default, Clone)]
pub struct Cache {
    pub entries: BTreeMap<NodeId, CacheEntry>,
}

impl Cache {
    pub fn get(&self, id: &NodeId) -> Option<&CacheEntry> {
        self.entries.get(id)
    }

    pub fn insert(&mut self, id: NodeId, hash: String, verdict: Verdict) {
        self.entries.insert(id, CacheEntry { hash, verdict });
    }

    /// Read the cache file at `path`. A missing file yields an empty cache. A
    /// malformed file yields an empty cache and a warning flag — the caller
    /// decides how to surface that.
    pub fn load(path: &Path) -> CacheLoadResult {
        let Ok(contents) = fs::read_to_string(path) else {
            return CacheLoadResult {
                cache: Cache::default(),
                existed: false,
                corrupted: false,
            };
        };
        let mut lines = contents.lines();
        let header_ok = lines.next() == Some(HEADER);
        let mut entries = BTreeMap::new();
        let mut corrupted = !header_ok;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            let (Some(id), Some(hash), Some(verdict)) =
                (parts.next(), parts.next(), parts.next())
            else {
                corrupted = true;
                continue;
            };
            if hash.is_empty() {
                corrupted = true;
                continue;
            }
            entries.insert(
                NodeId(id.to_string()),
                CacheEntry {
                    hash: hash.to_string(),
                    verdict: Verdict::decode(verdict),
                },
            );
        }
        CacheLoadResult {
            cache: Cache { entries },
            existed: true,
            corrupted,
        }
    }

    /// Write the cache atomically to `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp_path = PathBuf::from(path);
        tmp_path.set_extension("cache.tmp");

        {
            let mut f = fs::File::create(&tmp_path)?;
            writeln!(f, "{HEADER}")?;
            for (id, entry) in &self.entries {
                writeln!(f, "{}\t{}\t{}", id.as_str(), entry.hash, entry.verdict.encode())?;
            }
            f.sync_all()?;
        }

        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct CacheLoadResult {
    pub cache: Cache,
    /// Whether the cache file existed at all.
    pub existed: bool,
    /// Whether the file existed but could not be fully parsed (wrong header or
    /// malformed rows). The well-formed rows are still loaded.
    pub corrupted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let tmp = tempdir();
        let path = tmp.join(".nlc-cache");

        let mut cache = Cache::default();
        cache.insert(NodeId("a.md".into()), "deadbeef".into(), Verdict::Ok);
        cache.insert(
            NodeId("a.md::setup".into()),
            "cafe".into(),
            Verdict::Err(2),
        );
        cache.save(&path).unwrap();

        let loaded = Cache::load(&path);
        assert!(loaded.existed);
        assert!(!loaded.corrupted);
        assert_eq!(loaded.cache.entries.len(), 2);
        let root = loaded.cache.get(&NodeId("a.md".into())).unwrap();
        assert_eq!(root.hash, "deadbeef");
        assert_eq!(root.verdict, Verdict::Ok);
        let sec = loaded.cache.get(&NodeId("a.md::setup".into())).unwrap();
        assert_eq!(sec.verdict, Verdict::Err(2));
    }

    #[test]
    fn missing_file_is_empty() {
        let loaded = Cache::load(Path::new("/does/not/exist/.nlc-cache"));
        assert!(!loaded.existed);
        assert!(loaded.cache.entries.is_empty());
    }

    #[test]
    fn corrupted_header_flagged_but_rows_kept() {
        let tmp = tempdir();
        let path = tmp.join(".nlc-cache");
        fs::write(&path, "wrong header\na.md\tabc\tok\n").unwrap();
        let loaded = Cache::load(&path);
        assert!(loaded.existed);
        assert!(loaded.corrupted);
        // The row is still picked up despite the bad header.
        assert_eq!(loaded.cache.entries.len(), 1);
    }

    /// Minimal temp-dir helper (avoids pulling in the `tempfile` crate).
    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nlc-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
