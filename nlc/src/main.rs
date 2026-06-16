//! CLI entry point.
//!
//! The flow is uniform for every command: build a single [`Snapshot`] of the
//! workspace, then hand it to the matching [`printer::Printer`]. The only
//! command-specific side effect beyond printing is `check`, which also persists
//! the cache.

mod cache;
mod cli;
mod collect;
mod graph;
mod hash;
mod inline_text;
mod model;
mod printer;
mod snapshot;

use std::path::PathBuf;
use std::process::ExitCode;

use cli::Command;
use printer::Printer;
use snapshot::Snapshot;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args()) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("nlc: {msg}");
            eprintln!();
            eprint!("{}", cli::help_text());
            return ExitCode::from(2);
        }
    };

    let code = run(command);
    ExitCode::from(code as u8)
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn run(command: Command) -> i32 {
    match command {
        Command::Help => {
            print!("{}", cli::help_text());
            0
        }

        Command::Check { full } => {
            let snap = Snapshot::build(&cwd());
            let exit = render(&printer::StatusPrinter { full }, &snap);
            if let Err(e) = snap.next_cache.save(&snap.cache_path) {
                eprintln!("nlc: warning: failed to write cache: {e}");
            }
            exit
        }
        Command::Status { full } => {
            let snap = Snapshot::build(&cwd());
            render(&printer::StatusPrinter { full }, &snap)
        }
        Command::List { file } => {
            let snap = Snapshot::build(&cwd());
            render(&printer::ListPrinter { file }, &snap)
        }
        Command::Tree { file } => {
            let snap = Snapshot::build(&cwd());
            render(&printer::TreePrinter { file }, &snap)
        }
        Command::Graph => {
            let snap = Snapshot::build(&cwd());
            render(&printer::GraphPrinter, &snap)
        }

        Command::Clean => run_clean(&cwd()),
        Command::Ast(path) => run_ast(&path),
    }
}

/// Run a [`Printer`] against a [`Snapshot`] and flush its buffer to stdout.
fn render(p: &dyn Printer, snap: &Snapshot) -> i32 {
    let mut out = String::new();
    let code = p.print(snap, &mut out);
    print!("{out}");
    code
}

/// `nlc clean`: remove the cache file.
fn run_clean(root: &std::path::Path) -> i32 {
    let cache_path = root.join(".nlc-cache");
    match std::fs::remove_file(&cache_path) {
        Ok(()) => {
            println!("removed {}", cache_path.display());
            0
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("(no cache to remove)");
            0
        }
        Err(e) => {
            eprintln!("nlc: failed to remove cache: {e}");
            1
        }
    }
}

/// `nlc ast <file>`: parse one file and pretty-print its AST (original CLI).
fn run_ast(path: &str) -> i32 {
    let input = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nlc: failed to read {path}: {e}");
            return 2;
        }
    };
    match nlc_parser::parse(&input) {
        Ok(doc) => println!("{doc:#?}"),
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    }
    0
}
