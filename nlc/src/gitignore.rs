//! `.gitignore` parsing and matching.
//!
//! nlc honours `.gitignore` files — at the workspace root and in any
//! subdirectory — so build output, virtual environments, OS metadata, and
//! other untracked noise are never collected as Markdown or code reference
//! targets. This keeps nlc's view of the workspace aligned with what git
//! actually tracks.
//!
//! The matcher implements the practical subset of gitignore syntax that
//! matters for documentation workspaces:
//!
//! - blank lines and `#` comments
//! - negation patterns (`!`) — the *last* matching rule wins, so a later
//!   `!foo` re-includes an earlier `foo`
//! - directory-only patterns (trailing `/`)
//! - anchored patterns (leading `/`, or any interior `/`) — relative to the
//!   directory holding the `.gitignore`
//! - basename patterns (no `/`) — match at any depth
//! - globs: `*` (zero or more, stops at `/`), `?` (exactly one), `**`
//!   (zero or more whole path components), and `[...]` / `[!...]` classes
//! - `\`-escaping of special characters
//!
//! No external crate is used, keeping the workspace dependency-light.

use std::path::{Path, PathBuf};

/// One compiled `.gitignore` rule.
#[derive(Debug, Clone)]
struct Rule {
    /// Pattern split on `/` into [`Seg`]ments.
    segs: Vec<Seg>,
    negation: bool,
    /// Pattern ended with `/` → only matches directories.
    dir_only: bool,
}

/// A single component of a compiled pattern.
#[derive(Debug, Clone)]
enum Seg {
    /// A single path-component glob (no `/`).
    S(String),
    /// `**` — matches zero or more whole path components.
    Double,
}

/// A parsed `.gitignore` file: the directory it lives in plus its rules.
///
/// Fields are private; matching goes through [`is_ignored`], which takes a
/// stack of these (root → current directory).
#[derive(Debug, Clone)]
pub struct Gitignore {
    dir: PathBuf,
    rules: Vec<Rule>,
}

impl Gitignore {
    /// Load and parse a `.gitignore` in `dir`, returning `None` if the file is
    /// absent or contains no usable rules.
    fn load(dir: &Path) -> Option<Gitignore> {
        let src = std::fs::read_to_string(dir.join(".gitignore")).ok()?;
        let rules = src.lines().filter_map(Rule::parse).collect::<Vec<_>>();
        if rules.is_empty() {
            None
        } else {
            Some(Gitignore {
                dir: dir.to_path_buf(),
                rules,
            })
        }
    }
}

impl Rule {
    /// Compile one physical line of a `.gitignore`. Returns `None` for blanks
    /// and comments.
    fn parse(line: &str) -> Option<Rule> {
        // Git trims trailing (unescaped) whitespace; escaped trailing space is
        // vanishingly rare in docs, so a plain trim_end is sufficient.
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        // Negation (`!`).
        let (line, negation) = match line.strip_prefix('!') {
            Some(rest) => (rest, true),
            None => (line, false),
        };
        // Directory-only (trailing `/`).
        let (line, dir_only) = match line.strip_suffix('/') {
            Some(rest) => (rest, true),
            None => (line, false),
        };
        // A leading `/` or any interior `/` anchors the pattern to this
        // directory; a bare basename matches at any depth.
        let anchored = line.contains('/');
        let line = line.strip_prefix('/').unwrap_or(line);
        if line.is_empty() {
            return None;
        }
        let mut segs: Vec<Seg> = line
            .split('/')
            .map(|s| if s == "**" { Seg::Double } else { Seg::S(s.to_string()) })
            .collect();
        if !anchored {
            // Basename pattern → behave as `**/<pattern>`.
            segs.insert(0, Seg::Double);
        }
        Some(Rule {
            segs,
            negation,
            dir_only,
        })
    }

    /// Does this rule match `rel` (a path relative to the rule's directory)?
    fn matches(&self, rel: &Path, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        let rel = rel.to_string_lossy().replace('\\', "/");
        let path_segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        match_segments(&self.segs, &path_segs)
    }
}

/// Match a compiled pattern against path segments. `**` consumes zero or more
/// segments; any other segment is matched verbatim as a single-component glob.
fn match_segments(pat: &[Seg], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((Seg::Double, rest)) => {
            match_segments(rest, path)
                || (!path.is_empty() && match_segments(pat, &path[1..]))
        }
        Some((Seg::S(s), rest)) => {
            !path.is_empty()
                && glob_match_single(s.as_bytes(), path[0].as_bytes())
                && match_segments(rest, &path[1..])
        }
    }
}

/// Match a single path-component glob (no `/`) against `text`.
///
/// Supports `*` (zero+ chars), `?` (one char), `[...]` / `[!...]` classes,
/// and `\` escaping of the next byte.
fn glob_match_single(pat: &[u8], text: &[u8]) -> bool {
    glob_match_inner(pat, 0, text, 0)
}

fn glob_match_inner(pat: &[u8], mut pi: usize, text: &[u8], mut ti: usize) -> bool {
    while pi < pat.len() {
        match pat[pi] {
            b'*' => {
                while pi < pat.len() && pat[pi] == b'*' {
                    pi += 1;
                }
                if pi == pat.len() {
                    // Trailing `*` matches the rest of the segment.
                    return true;
                }
                // Try the remainder at every suffix of `text` (incl. empty).
                while ti <= text.len() {
                    if glob_match_inner(pat, pi, text, ti) {
                        return true;
                    }
                    ti += 1;
                }
                return false;
            }
            b'?' => {
                if ti >= text.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            b'[' => {
                if ti >= text.len() {
                    return false;
                }
                let (matched, next_pi) = match_class(pat, pi, text[ti]);
                if !matched {
                    return false;
                }
                pi = next_pi;
                ti += 1;
            }
            b'\\' if pi + 1 < pat.len() => {
                if ti >= text.len() || text[ti] != pat[pi + 1] {
                    return false;
                }
                pi += 2;
                ti += 1;
            }
            c => {
                if ti >= text.len() || text[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == text.len()
}

/// Evaluate a `[...]` class starting at `pat[start] == b'['`. Returns whether
/// `c` matches and the index just past the closing `]`.
fn match_class(pat: &[u8], start: usize, c: u8) -> (bool, usize) {
    let mut i = start + 1;
    let negate = i < pat.len() && (pat[i] == b'!' || pat[i] == b'^');
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < pat.len() {
        let lo = pat[i];
        if lo == b']' && !first {
            break;
        }
        first = false;
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            let hi = pat[i + 2];
            if c >= lo && c <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if c == lo {
                matched = true;
            }
            i += 1;
        }
    }
    if i < pat.len() && pat[i] == b']' {
        i += 1;
    }
    (matched != negate, i)
}

/// Load a `.gitignore` from `dir`, if one is present.
pub fn load(dir: &Path) -> Option<Gitignore> {
    Gitignore::load(dir)
}

/// Decide whether `full` is ignored, given an ordered stack of `.gitignore`
/// files from the workspace root down to the current directory.
///
/// Rules are checked in stack order and, within each file, in source order;
/// the **last** matching rule wins (so a later `!foo` re-includes an earlier
/// `foo`, and a deeper file overrides a shallower one).
pub fn is_ignored(ignores: &[Gitignore], full: &Path, is_dir: bool) -> bool {
    let mut ignored = false;
    for gi in ignores {
        // Only gitignores that are ancestors of `full` scope to it; others are
        // skipped (their directory is not a prefix of `full`).
        let Ok(rel) = full.strip_prefix(&gi.dir) else {
            continue;
        };
        for rule in &gi.rules {
            if rule.matches(rel, is_dir) {
                ignored = !rule.negation;
            }
        }
    }
    ignored
}

#[cfg(test)]
impl Gitignore {
    fn from_str(dir: &str, src: &str) -> Gitignore {
        let rules = src.lines().filter_map(Rule::parse).collect();
        Gitignore {
            dir: PathBuf::from(dir),
            rules,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `.gitignore` rooted at `/r`, check each `(path, is_dir)`.
    fn check(gi: &Gitignore, cases: &[(&str, bool, bool)]) {
        let stack = vec![gi.clone()];
        for (path, is_dir, want) in cases {
            let got = is_ignored(&stack, Path::new(path), *is_dir);
            assert_eq!(
                got, *want,
                "path={path:?} is_dir={is_dir}: expected {want}, got {got}"
            );
        }
    }

    #[test]
    fn basename_matches_at_any_depth() {
        let gi = Gitignore::from_str("/r", "*.log\n");
        check(
            &gi,
            &[
                ("/r/a.log", false, true),
                ("/r/sub/a.log", false, true),
                ("/r/sub/deep/a.log", false, true),
                ("/r/a.md", false, false),
            ],
        );
    }

    #[test]
    fn negation_reincludes() {
        // `!keep.md` is a basename pattern, so it re-includes `keep.md` at
        // *any* depth, not just the root.
        let gi = Gitignore::from_str("/r", "*.md\n!keep.md\n");
        check(
            &gi,
            &[
                ("/r/gone.md", false, true),
                ("/r/keep.md", false, false),
                ("/r/sub/keep.md", false, false),
            ],
        );
    }

    #[test]
    fn dir_only_does_not_match_files() {
        let gi = Gitignore::from_str("/r", "build/\n");
        check(
            &gi,
            &[
                ("/r/build", true, true),
                ("/r/build", false, false),
                ("/r/sub/build", true, true),
            ],
        );
    }

    #[test]
    fn leading_slash_anchors_to_gitignore_dir() {
        let gi = Gitignore::from_str("/r", "/foo\n");
        check(
            &gi,
            &[
                ("/r/foo", false, true),
                ("/r/foo", true, true),
                ("/r/sub/foo", false, false),
            ],
        );
    }

    #[test]
    fn interior_slash_anchors() {
        let gi = Gitignore::from_str("/r", "src/gen/*.rs\n");
        check(
            &gi,
            &[
                ("/r/src/gen/a.rs", false, true),
                ("/r/src/a.rs", false, false),
                ("/r/gen/a.rs", false, false),
            ],
        );
    }

    #[test]
    fn double_star_spans_directories() {
        let gi = Gitignore::from_str("/r", "**/cache\n");
        check(
            &gi,
            &[
                ("/r/cache", true, true),
                ("/r/a/b/cache", true, true),
                ("/r/a/cache/x", false, false),
            ],
        );
    }

    #[test]
    fn double_star_mid_pattern() {
        let gi = Gitignore::from_str("/r", "a/**/b\n");
        check(
            &gi,
            &[
                ("/r/a/b", false, true),
                ("/r/a/x/b", false, true),
                ("/r/a/x/y/b", false, true),
                ("/r/a/b/c", false, false),
            ],
        );
    }

    #[test]
    fn question_mark_is_single_char() {
        let gi = Gitignore::from_str("/r", "a?c\n");
        check(
            &gi,
            &[
                ("/r/abc", false, true),
                ("/r/ac", false, false),
                ("/r/a/c", false, false),
                ("/r/abbc", false, false),
            ],
        );
    }

    #[test]
    fn character_classes() {
        let gi = Gitignore::from_str("/r", "*.[cho]\n[!a-c].txt\n");
        check(
            &gi,
            &[
                ("/r/main.c", false, true),
                ("/r/x.h", false, true),
                ("/r/x.o", false, true),
                ("/r/x.js", false, false),
                ("/r/d.txt", false, true),
                ("/r/a.txt", false, false),
            ],
        );
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let gi = Gitignore::from_str("/r", "# a comment\n\n   \n*.tmp\n");
        check(&gi, &[("/r/x.tmp", false, true), ("/r/x.md", false, false)]);
    }

    #[test]
    fn deeper_gitignore_overrides_shallower() {
        let root = Gitignore::from_str("/r", "*.log\n");
        let nested = Gitignore::from_str("/r/sub", "!important.log\n");
        let stack = vec![root, nested];
        // Nested negation wins → re-included.
        assert!(!is_ignored(&stack, Path::new("/r/sub/important.log"), false));
        // A log outside the nested dir stays ignored.
        assert!(is_ignored(&stack, Path::new("/r/other.log"), false));
    }

    #[test]
    fn shallower_rule_does_not_leak_into_unrelated_subtree() {
        // `/r/a` ignores only under `/r/a`; a sibling `/r/b` is unaffected.
        let gi = Gitignore::from_str("/r/a", "x\n");
        let stack = vec![gi];
        assert!(is_ignored(&stack, Path::new("/r/a/x"), false));
        assert!(!is_ignored(&stack, Path::new("/r/b/x"), false));
    }
}
