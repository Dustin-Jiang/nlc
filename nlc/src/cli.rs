//! Hand-rolled argument parser.
//!
//! We avoid pulling in `clap` to match the workspace's minimalist-dependency
//! style. The surface area is small enough that a bespoke parser stays
//! readable.

/// The parsed subcommand. Variants carry their own options as plain fields so
/// the dispatcher in `main` can construct the matching printer directly.
#[derive(Debug)]
pub enum Command {
    /// Analyze + report AND persist the `.nlc-cache` (default).
    Check { full: bool },
    /// Read-only analyze + report.
    Status { full: bool },
    /// Print the section tree of one file, or every file when none is given.
    List { file: Option<String> },
    /// Print all resolved reference edges.
    Graph,
    /// Delete the `.nlc-cache` file.
    Clean,
    /// Debug: parse one file and dump its AST (preserves the original CLI).
    Ast(String),
    /// `--help` / `-h` / `help`.
    Help,
}

const HELP: &str = "\
nlc — incremental markdown dependency linter

USAGE:
    nlc                       Run `check` in the current directory.
    nlc check [--full]        Scan, validate, report, AND write .nlc-cache.
    nlc status [--full]       Like `check` but does NOT write the cache.
    nlc list [<file>]         Print the section tree.
    nlc graph                 Print all cross-file reference edges.
    nlc clean                 Remove the .nlc-cache file.
    nlc ast <file>            Parse one file and dump its AST (debug).

FLAGS:
    --full                    Re-report every node's issues, not only the delta.
    -h, --help                Show this help text.

EXIT CODES:
    0   clean
    1   validation errors (dangling refs, cycles, parse errors)
    2   usage error
";

/// Parse `std::env::args` into a [`Command`]. Returns an error message for
/// invalid usage (the caller exits with code 2).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut it = args.into_iter();
    let _program = it.next();
    let Some(sub) = it.next() else {
        return Ok(Command::Check { full: false });
    };
    match sub.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "check" | "status" => {
            let mut full = false;
            for arg in it {
                match arg.as_str() {
                    "--full" => full = true,
                    "-h" | "--help" => return Ok(Command::Help),
                    other => return Err(format!("unknown argument `{other}`")),
                }
            }
            if sub == "check" {
                Ok(Command::Check { full })
            } else {
                Ok(Command::Status { full })
            }
        }
        "list" => {
            let file = it.next();
            if it.next().is_some() {
                return Err("`list` takes at most one file argument".into());
            }
            Ok(Command::List { file })
        }
        "graph" => {
            if it.next().is_some() {
                return Err("`graph` takes no arguments".into());
            }
            Ok(Command::Graph)
        }
        "clean" => {
            if it.next().is_some() {
                return Err("`clean` takes no arguments".into());
            }
            Ok(Command::Clean)
        }
        "ast" => {
            let Some(file) = it.next() else {
                return Err("`ast` requires a file argument".into());
            };
            if it.next().is_some() {
                return Err("`ast` takes exactly one file argument".into());
            }
            Ok(Command::Ast(file))
        }
        other => Err(format!(
            "unknown subcommand `{other}` (see `nlc --help`)"
        )),
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
    fn no_args_is_check() {
        assert!(matches!(parse(args(&[])), Ok(Command::Check { full: false })));
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
    fn ast_requires_file() {
        assert!(parse(args(&["ast"])).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(parse(args(&["frobnicate"])).is_err());
    }
}
