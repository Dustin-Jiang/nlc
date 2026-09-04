//! Hand-rolled argument parser.
//!
//! We avoid pulling in `clap` to match the workspace's minimalist-dependency
//! style. The surface area is small enough that a bespoke parser stays
//! readable. Every subcommand carries its own detailed help (`nlc <command>
//! --help` or `nlc help <command>`), and a usage error prints the help of
//! the subcommand it occurred in.

/// The parsed subcommand. Variants carry their own options as plain fields so
/// the dispatcher in `main` can construct the matching printer directly.
#[derive(Debug)]
pub enum Command {
    /// Read-only analyze + report (the default when no subcommand is given).
    Status { full: bool },
    /// Analyze + report AND persist the `.nlc-cache`.
    Check { full: bool },
    /// Print the section tree of one file, or every file when none is given.
    List { file: Option<String> },
    /// Tree-view of one file's nodes with recursive dependency expansion.
    Tree { file: String },
    /// Print all resolved reference edges.
    Graph,
    /// Delete the `.nlc-cache` file.
    Clean,
    /// Debug: parse one file and dump its AST (preserves the original CLI).
    Ast(String),
    /// `--help` / `-h` / `help [<command>]`.
    Help { sub: Option<String> },
}

/// Invalid usage: a message plus the subcommand it occurred in, so the
/// caller can print that subcommand's detailed help.
#[derive(Debug)]
pub struct UsageError {
    pub message: String,
    /// Canonical subcommand name, if the error happened inside one.
    pub sub: Option<String>,
}

fn usage(message: impl Into<String>, sub: &str) -> UsageError {
    UsageError {
        message: message.into(),
        sub: Some(sub.to_string()),
    }
}

const HELP: &str = "\
nlc — incremental markdown dependency linter

USAGE:
    nlc [command] [args]

    With no command, `nlc` runs a read-only status report (same as
    `nlc status`).

COMMANDS:
    status      Analyze the workspace and report issues (read-only).
    check       Analyze, report, and persist .nlc-cache.
    list        Print the section outline of one file, or all files.
    tree        Print a file's node tree with recursive dep expansion.
    graph       Print every cross-file reference edge.
    clean       Delete the .nlc-cache file.
    ast         Parse one file and dump its AST (debug).
    help        Show help for nlc or one of its commands.

    Run `nlc <command> --help` for details on a command.

FLAGS:
    --full      status/check: re-report every node's issues, not only the
                delta since the cached run.
    -h, --help  Show help; with a command, show that command's help.

EXIT CODES:
    0   clean
    1   validation errors (dangling refs, cycles, parse errors)
    2   usage error

EXAMPLES:
    nlc check                 # report and persist .nlc-cache
    nlc --full                # re-report everything, ignoring the delta
    nlc tree docs/guide.md    # dependency tree of one file
    nlc help tree             # detailed help for `tree`
";

const STATUS_HELP: &str = "\
nlc status — analyze the workspace and report issues (read-only)

USAGE:
    nlc status [--full]
    nlc [--full]                (default when no command is given)

DESCRIPTION:
    Collects every Markdown file under the workspace (honoring
    .gitignore), resolves all [[...]] cross-file references, and reports
    issues: missing files or sections, ambiguous targets, code-line
    targets out of range, unsupported code targets, and cycles.

    With a `.nlc-cache` written by a previous `nlc check`, only nodes
    whose content — or whose transitive dependencies — changed are
    re-reported. Never writes the cache.

OPTIONS:
    --full        Re-report every node's issues, ignoring the cache delta.
    -h, --help    Show this help.

EXIT CODES:
    0   clean
    1   validation errors
    2   usage error
";

const CHECK_HELP: &str = "\
nlc check — analyze, report, and persist .nlc-cache

USAGE:
    nlc check [--full]

DESCRIPTION:
    Same report as `nlc status`, but also writes `.nlc-cache` at the
    workspace root. Run it after edits so subsequent read-only `nlc` /
    `nlc status` runs can report only what changed.

OPTIONS:
    --full        Re-report every node's issues, ignoring the cache delta.
    -h, --help    Show this help.

EXIT CODES:
    0   clean
    1   validation errors (cache is still written)
    2   usage error
";

const LIST_HELP: &str = "\
nlc list — print section outlines

USAGE:
    nlc list [<file>]

DESCRIPTION:
    Prints the heading outline of one Markdown file — each section as an
    indented `#`-heading line, with `(N block(s))` under headings that
    have body content — or of every workspace file when no file is
    given. A non-empty preamble is noted as `(preamble: N block(s))`.

ARGS:
    <file>    File to outline, relative to the workspace root. Optional.

EXIT CODES:
    0   success
    2   unknown file / usage error
";

const TREE_HELP: &str = "\
nlc tree — print a file's node tree with recursive dependency expansion

USAGE:
    nlc tree <file>

DESCRIPTION:
    Renders one file's section hierarchy and, under every node, expands
    its forward [[...]] dependencies — following edges across files — to
    show the full transitive dependency footprint.

    * A Markdown target (a document or one of its sections) expands its
      own section tree as well; code-file targets (e.g. src/main.rs,
      src/ui.rs::L12-20) are leaves.
    * A node already expanded higher in the tree is not expanded twice;
      the repeat is labeled `(cycle)`.
    * A reference that cannot be resolved prints as
      `<unresolved>  (original [[...]] text)`.

ARGS:
    <file>    File to render, relative to the workspace root.

EXAMPLES:
    nlc tree docs/001-architecture.md

EXIT CODES:
    0   success
    2   unknown file / usage error
";

const GRAPH_HELP: &str = "\
nlc graph — print every cross-file reference edge

USAGE:
    nlc graph

DESCRIPTION:
    Prints the resolved reference graph as one `from -> to` edge per
    line, sorted by source node. Unresolvable references appear as
    `from -> <unresolved>  ([[...]])`. Prints `(no cross-file
    references)` for an edge-less workspace.

EXIT CODES:
    0   success (unresolved references are reported inline, not fatal)
";

const CLEAN_HELP: &str = "\
nlc clean — delete .nlc-cache

USAGE:
    nlc clean

DESCRIPTION:
    Removes the `.nlc-cache` file at the workspace root, so the next
    status run reports everything again. Succeeds with a note when
    there is no cache to remove.

EXIT CODES:
    0   removed, or nothing to remove
    1   cache file could not be removed
";

const AST_HELP: &str = "\
nlc ast — parse one file and dump its AST (debug)

USAGE:
    nlc ast <file>

DESCRIPTION:
    Reads the file directly from disk — no workspace scan, no cache —
    and pretty-prints the nlc-parser document AST as Rust debug output.

ARGS:
    <file>    Path to parse, as given (relative to the current directory).

EXIT CODES:
    0   success
    1   parse error
    2   file could not be read / usage error
";

const HELP_HELP: &str = "\
nlc help — show help

USAGE:
    nlc help [<command>]
    nlc [<command>] --help

DESCRIPTION:
    With no argument, prints the overview. With a command name, prints
    that command's detailed help.
";

/// Detailed help text per subcommand, keyed by canonical name.
const SUB_HELP: &[(&str, &str)] = &[
    ("status", STATUS_HELP),
    ("check", CHECK_HELP),
    ("list", LIST_HELP),
    ("tree", TREE_HELP),
    ("graph", GRAPH_HELP),
    ("clean", CLEAN_HELP),
    ("ast", AST_HELP),
    ("help", HELP_HELP),
];

/// Detailed help for one subcommand, or `None` for an unknown name.
pub fn subcommand_help(sub: &str) -> Option<&'static str> {
    SUB_HELP
        .iter()
        .find(|(name, _)| *name == sub)
        .map(|(_, text)| *text)
}

/// Parse an argument list into a [`Command`]. Returns a [`UsageError`] for
/// invalid usage (the caller exits with code 2 and prints the error's
/// subcommand help).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, UsageError> {
    let mut it = args.into_iter();
    let _program = it.next();
    let Some(sub) = it.next() else {
        return Ok(Command::Status { full: false });
    };
    let rest: Vec<String> = it.collect();
    let wants_help = rest.iter().any(|a| a == "-h" || a == "--help");

    match sub.as_str() {
        "-h" | "--help" => Ok(Command::Help { sub: None }),
        // Bare `nlc --full` behaves like `nlc status --full`.
        "--full" => Ok(Command::Status { full: true }),
        "help" => match rest.len() {
            0 => Ok(Command::Help { sub: None }),
            1 if subcommand_help(&rest[0]).is_some() => {
                Ok(Command::Help { sub: Some(rest[0].clone()) })
            }
            1 => Err(UsageError {
                message: format!("unknown command `{}` (see `nlc --help`)", rest[0]),
                sub: None,
            }),
            _ => Err(usage("`help` takes at most one command name", "help")),
        },
        "check" | "status" => {
            let name = if sub == "check" { "check" } else { "status" };
            if wants_help {
                return Ok(Command::Help { sub: Some(name.to_string()) });
            }
            let mut full = false;
            for arg in &rest {
                match arg.as_str() {
                    "--full" => full = true,
                    other => {
                        return Err(usage(format!("unknown argument `{other}`"), name));
                    }
                }
            }
            if name == "check" {
                Ok(Command::Check { full })
            } else {
                Ok(Command::Status { full })
            }
        }
        "list" => {
            if wants_help {
                return Ok(Command::Help { sub: Some("list".into()) });
            }
            match rest.len() {
                0 => Ok(Command::List { file: None }),
                1 => Ok(Command::List { file: Some(rest[0].clone()) }),
                _ => Err(usage("`list` takes at most one file argument", "list")),
            }
        }
        "tree" => {
            if wants_help {
                return Ok(Command::Help { sub: Some("tree".into()) });
            }
            match rest.len() {
                0 => Err(usage("`tree` requires a file argument", "tree")),
                1 => Ok(Command::Tree { file: rest[0].clone() }),
                _ => Err(usage("`tree` takes exactly one file argument", "tree")),
            }
        }
        "graph" => {
            if wants_help {
                return Ok(Command::Help { sub: Some("graph".into()) });
            }
            if rest.is_empty() {
                Ok(Command::Graph)
            } else {
                Err(usage("`graph` takes no arguments", "graph"))
            }
        }
        "clean" => {
            if wants_help {
                return Ok(Command::Help { sub: Some("clean".into()) });
            }
            if rest.is_empty() {
                Ok(Command::Clean)
            } else {
                Err(usage("`clean` takes no arguments", "clean"))
            }
        }
        "ast" => {
            if wants_help {
                return Ok(Command::Help { sub: Some("ast".into()) });
            }
            match rest.len() {
                0 => Err(usage("`ast` requires a file argument", "ast")),
                1 => Ok(Command::Ast(rest[0].clone())),
                _ => Err(usage("`ast` takes exactly one file argument", "ast")),
            }
        }
        other => Err(UsageError {
            message: format!("unknown subcommand `{other}` (see `nlc --help`)"),
            sub: None,
        }),
    }
}

pub fn help_text() -> &'static str {
    HELP
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        std::iter::once("nlc".to_string())
            .chain(parts.iter().map(|s| s.to_string()))
            .collect()
    }

    #[test]
    fn no_args_is_status() {
        assert!(matches!(parse(args(&[])), Ok(Command::Status { full: false })));
    }

    #[test]
    fn bare_full_is_status_full() {
        assert!(matches!(parse(args(&["--full"])), Ok(Command::Status { full: true })));
    }

    #[test]
    fn check_full() {
        assert!(matches!(parse(args(&["check", "--full"])), Ok(Command::Check { full: true })));
    }

    #[test]
    fn status_no_full() {
        assert!(matches!(parse(args(&["status"])), Ok(Command::Status { full: false })));
    }

    #[test]
    fn list_one_file() {
        assert!(matches!(parse(args(&["list", "a.md"])), Ok(Command::List { .. })));
    }

    #[test]
    fn tree_requires_file() {
        assert!(parse(args(&["tree"])).is_err());
        assert!(matches!(
            parse(args(&["tree", "a.md"])),
            Ok(Command::Tree { .. })
        ));
    }

    #[test]
    fn ast_requires_file() {
        assert!(parse(args(&["ast"])).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(parse(args(&["frobnicate"])).is_err());
    }

    #[test]
    fn per_command_help_flags() {
        assert!(matches!(
            parse(args(&["tree", "--help"])),
            Ok(Command::Help { sub: Some(s) }) if s == "tree"
        ));
        assert!(matches!(
            parse(args(&["list", "-h"])),
            Ok(Command::Help { sub: Some(s) }) if s == "list"
        ));
        assert!(matches!(
            parse(args(&["check", "--help"])),
            Ok(Command::Help { sub: Some(s) }) if s == "check"
        ));
    }

    #[test]
    fn help_subcommand_takes_command_name() {
        assert!(matches!(
            parse(args(&["help", "check"])),
            Ok(Command::Help { sub: Some(s) }) if s == "check"
        ));
        assert!(matches!(
            parse(args(&["help"])),
            Ok(Command::Help { sub: None })
        ));
        assert!(parse(args(&["help", "bogus"])).is_err());
    }

    #[test]
    fn every_command_has_help_text() {
        for name in ["status", "check", "list", "tree", "graph", "clean", "ast", "help"] {
            assert!(subcommand_help(name).is_some(), "{name}");
        }
        assert!(subcommand_help("bogus").is_none());
    }

    #[test]
    fn usage_errors_carry_subcommand() {
        match parse(args(&["tree"])) {
            Err(e) => assert_eq!(e.sub.as_deref(), Some("tree")),
            Ok(_) => panic!("expected error"),
        }
        match parse(args(&["check", "--nope"])) {
            Err(e) => assert_eq!(e.sub.as_deref(), Some("check")),
            Ok(_) => panic!("expected error"),
        }
        match parse(args(&["frobnicate"])) {
            Err(e) => assert_eq!(e.sub, None),
            Ok(_) => panic!("expected error"),
        }
    }
}
