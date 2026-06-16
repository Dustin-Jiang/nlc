# AGENTS.md

Compact guide for OpenCode sessions working in this repo.

## Workspace

Two-crate Cargo workspace, both `edition = "2024"`:

- **`nlc-parser/`** — the Markdown parser library. Extends CommonMark with
  `[[...]]` cross-file references (`FileRef` / `FileRefTarget` in
  `src/ast.rs`). Built with LALRPOP.
- **`nlc/`** — the CLI binary. Depends on `nlc-parser` + `sha2`. Default
  `nlc` is read-only; `nlc check` persists `.nlc-cache`.

## Commands

```
cargo build
cargo test                       # all crates, integration + inline unit tests
cargo test -p nlc <name>         # one test, e.g. -p nlc tree::tests
cargo test -p nlc-parser markdown::atx_heading
cargo clippy --all-targets       # MUST be warning-free before finishing
```

There is no separate lint/typecheck config — clippy is the linter. Expect
**0 warnings**; the repo keeps it clean.

## LALRPOP codegen (important)

`nlc-parser` has two hand-written grammars:

- `src/parser_block.lalrpop`
- `src/parser_inline.lalrpop`

`build.rs` runs `lalrpop::process_root()`, which regenerates the matching
`src/parser_block.rs` and `src/parser_inline.rs` on build. **Those generated
`.rs` files are git-tracked**, not gitignored.

- To change parsing: edit the `.lalrpop`, run `cargo build`, then commit **both**
  the grammar and the regenerated `.rs`.
- Never hand-edit `parser_block.rs` / `parser_inline.rs` — they will be
  overwritten.

The lexers (`src/block/lexer.rs`, `src/inline/lexer.rs`) do the
context-sensitive work (indentation, nesting, emphasis flanking); the LALRPOP
grammars only build the tree on top of the flat token stream.

## Architecture: `nlc` CLI

One [`Snapshot`] is built per run (`snapshot.rs`) running the full pipeline:
`collect` → `graph` → `hash` → cache-diff. Every subcommand is a `Printer`
struct (`printer/`) that renders the `Snapshot` to text.

- Add a command = new `printer/*.rs` struct implementing `Printer` + a
  `Command` variant in `cli.rs` + a dispatch arm in `main.rs`.
- `model.rs` owns `NodeId` / `Section` / `FileNode` / `World`. NodeIds are
  canonical strings: `guide.md` (file root) or `guide.md::intro/setup`
  (section, slug-path joined by `/`).
- Section hashing is **hierarchical**: a child change bubbles up through
  ancestor hashes, so the diff only compares per-node hashes.
- `graph.rs` resolves `[[...]]` refs and detects cycles (Kosaraju SCC).
  Dangling refs / missing files / cycles are **errors** (exit 1).

## Tests

- `nlc-parser/tests/markdown.rs` — parser integration tests (AST fixtures).
- All `nlc` tests are inline `#[cfg(test)] mod tests` per module.
- `nlc/tests/repo/` is a **manual demo** markdown knowledge base (Chinese
  filenames), not referenced by any automated test. Don't treat it as a test
  harness.

## Conventions

- **Commits (important)**: Conventional Commits with the crate as scope —
  `feat(nlc):`, `refactor(nlc-parser):`, `fix(nlc):`, `test(...)`,
  `chore(...)`.
  - **Split work into the smallest logical feature points** — one feature per
    commit. Don't land multiple concerns in a single commit; if a change spans
    several features, stage/reconstruct them into separate commits in
    dependency order.
  - **Every commit must compile and pass `cargo test` + `cargo clippy
    --all-targets`** on its own (no broken intermediate states).
  - **Leave the working tree clean** when you finish: everything committed,
    `git status` empty. No stray uncommitted changes or untracked files.
- Edition 2024 let-chains (`if let .. && ..`) are used in a few places — a
  recent Rust toolchain (1.85+) is required.
- `.nlc-cache` at the workspace root is runtime state (gitignored); don't
  commit it.
