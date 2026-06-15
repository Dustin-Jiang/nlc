//! CLI entry point.
//!
//! Dispatches to the subcommands defined in [`cli::Command`]. The default
//! `check` command is the incremental markdown dependency linter; `ast`
//! preserves the original single-file AST dumper for debugging.

mod cache;
mod check;
mod cli;
mod collect;
mod graph;
mod hash;
mod inline_text;
mod model;

use std::path::PathBuf;
use std::process::ExitCode;

use cli::Command;

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

    let code = match command {
        Command::Help => {
            print!("{}", cli::help_text());
            0
        }
        Command::Check(opts) => check::run(&cwd(), &opts),
        Command::Status(opts) => run_status(&cwd(), &opts),
        Command::List { file } => run_list(&cwd(), file.as_deref()),
        Command::Graph => run_graph(&cwd()),
        Command::Clean => run_clean(&cwd()),
        Command::Ast(path) => run_ast(&path),
    };
    ExitCode::from(code as u8)
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `nlc status`: analyze and report without writing the cache.
fn run_status(root: &std::path::Path, opts: &check::CheckOptions) -> i32 {
    let cache_path = root.join(".nlc-cache");
    let prev = cache::Cache::load(&cache_path).cache;
    let collected = collect::collect(root);
    let (mut analysis, _next) = check::analyze(&collected.world, collected.errors, &prev, opts);
    analysis.cache_existed = std::path::Path::new(&cache_path).exists();
    check::print_report(&analysis, opts)
}

/// `nlc list [<file>]`: print the section tree.
fn run_list(root: &std::path::Path, file: Option<&str>) -> i32 {
    let collected = collect::collect(root);
    let world = &collected.world;
    let target_files: Vec<String> = match file {
        Some(one) => {
            if !world.files.contains_key(one) {
                eprintln!("nlc: no such file `{one}`");
                return 2;
            }
            vec![one.to_string()]
        }
        None => world.files.keys().cloned().collect(),
    };

    for (i, path) in target_files.iter().enumerate() {
        if i > 0 {
            println!();
        }
        let f = world.file(path).unwrap();
        println!("{path}");
        if !f.preamble.is_empty() {
            println!("  (preamble: {} block(s))", f.preamble.len());
        }
        for s in &f.sections {
            print_section(s, 1);
        }
    }
    0
}

fn print_section(s: &model::Section, depth: usize) {
    let indent = "  ".repeat(depth);
    let title = s.title();
    println!("{indent}{} {title}", "#".repeat(s.level as usize));
    if !s.body.is_empty() {
        println!("{indent}  ({} block(s))", s.body.len());
    }
    for child in &s.children {
        print_section(child, depth + 1);
    }
}

/// `nlc graph`: print all reference edges.
fn run_graph(root: &std::path::Path) -> i32 {
    let collected = collect::collect(root);
    let g = graph::build(&collected.world);
    let mut keys: Vec<&model::NodeId> = g.forward.keys().collect();
    keys.sort();
    let mut printed_any = false;
    for from in keys {
        let edges = &g.forward[from];
        if edges.is_empty() {
            continue;
        }
        printed_any = true;
        for e in edges {
            match &e.to {
                Some(to) => println!("{} -> {}", e.from, to),
                None => println!("{} -> <unresolved>  ({})", e.from, e.raw),
            }
        }
    }
    if !printed_any {
        println!("(no cross-file references)");
    }
    0
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
