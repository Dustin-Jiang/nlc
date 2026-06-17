---
name: nlc
description: Use nlc to keep Markdown documentation and source code in sync via [[...]] cross-references. Invoke when the user runs or asks about nlc (status/check/graph/tree/list/clean), needs to fix nlc-reported errors (dangling refs, line out of range, ambiguous section, circular dependency), wants to update code after editing docs, or wants to update a doc's [[code#L..]] line references after editing code. Covers commands, Markdown↔Markdown and Markdown→code reference syntax, and the bidirectional doc⇄code synchronization workflow.
---

# nlc — Markdown ⇄ Code Cross-Reference Tracker

`nlc` lints a workspace of Markdown files that reference each other and arbitrary
code files via `[[...]]` wikilinks. Use it to keep documentation and code in
lockstep: it reports which documentation changed, validates that every reference
resolves, and lets you navigate from any doc to the exact code lines it describes
(and back).

> `nlc` treats its **cwd as the workspace root**. If `nlc` isn't on PATH, run it
> from the source repo as `cargo run -q -- <args>` (build first with `cargo build`).

## When to Use

Invoke this skill when the user:

- runs or asks about `nlc`, `nlc status`, `nlc check`, `nlc graph`, `nlc tree`, …
- needs to fix errors nlc reports: *dangling/missing file*, *missing/ambiguous
  section*, *line out of range*, *named target on a code file*, *circular
  dependency*
- **edited Markdown** and wants to propagate the documented change into the code
  it references
- **edited code** and wants to update the `[[code.rs#L..]]` line references in the
  docs that point at it
- wants to find which documents describe a given piece of code, or which code a
  given document describes

## Mental Model (read this first)

- **Markdown files are nodes.** Each `.md` file splits into a file root + nested
  sections. Every node is content-hashed and recorded in `.nlc-cache`, so `nlc`
  reports a precise *delta* each run.
- **Code files are targets only.** Any readable non-`.md` text file (`.rs`,
  `.py`, `Makefile`, `Dockerfile`, …) is a reference *target* — never hashed,
  never cached. `nlc` only checks that a code ref points at an existing file with
  an in-range line number.
- **`[[...]]` wikilinks form a directed graph** from Markdown nodes to Markdown
  nodes or code locations. `nlc graph` prints every edge; `nlc tree <file>`
  expands one file's transitive deps.
- ⚠️ **`nlc` does NOT detect code edits.** It catches missing files and
  out-of-range line numbers, but a code line whose *content* changed at the same
  number is invisible to it. Re-deriving the correct line after code edits is the
  agent's job (Workflow B).

## Commands

| Command | What it does | Writes cache? |
|---|---|---|
| `nlc` / `nlc status [--full]` | Incremental status: changed / affected / up-to-date nodes + issues. | no |
| `nlc check [--full]` | Same report, then persist `.nlc-cache`. Run at the end of a change unit. | **yes** |
| `nlc graph` | Every cross-file edge: `from -> to` (or `from -> <unresolved>  ([[raw]])`). | no |
| `nlc tree <file>` | ASCII tree of one file's nodes, recursively expanding forward deps. | no |
| `nlc list [<file>]` | Plain section-tree dump (one file or every file). | no |
| `nlc clean` | Delete `.nlc-cache` (force full re-validation next run). | — |
| `nlc ast <file>` | Debug: dump the parsed Markdown AST. | no |

Exit codes: `0` clean · `1` validation/parse errors · `2` usage error.
`--full` shows issues on up-to-date nodes too (otherwise only revalidated ones).

## Reference Syntax

Every reference is a `[[ ... ]]` wikilink: an optional file path, an optional
`#`-locator, and an optional `|alias`.

### Markdown ⇄ Markdown

| Form | Example | Resolves to |
|---|---|---|
| whole document | `[[guide.md]]` | `guide.md` (file root) |
| `.md` shorthand | `[[guide]]` | `guide.md` |
| section by heading | `[[guide.md#Setup]]` | `guide.md::guide::setup` (matches the **last** slug) |
| same-file section | `[[#Setup]]` | a section in the current doc |
| line in a `.md` file | `[[guide.md#L12]]` | the **file root** `guide.md` (whole-file dependency) |

Section matching slugifies the heading text and compares it to each section's
*last* slug component. Two headings that slugify the same under different parents
→ **ambiguous** error.

### Markdown → Code files (any non-`.md` text file)

| Form | Example | Resolves to |
|---|---|---|
| whole code file | `[[src/main.rs]]` | `src/main.rs` |
| single line | `[[src/main.rs#L42]]` | `src/main.rs::L42` |
| line range | `[[src/main.rs#L10-L20]]` | `src/main.rs::L10-20` |

Code files support **line locators only**. `[[src/main.rs#main]]` (a name) is an
**error** — code files have no sections/AST. Always use `#L<n>` / `#L<a>-L<b>`.

Aliases work everywhere: `[[src/main.rs#L42|entry point]]`.

> `::` separates every level of the hierarchy uniformly:
> `guide.md::guide::setup`, `src/main.rs::L10-20`.

## Workflow: keep docs and code in sync

The loop has two directions. Run `nlc check` at the end of each direction to set
the baseline for the next delta. Treat a non-zero exit like a compiler error:
don't commit until `nlc` reports `ok`.

### Direction A — a document changed → update the code it describes

Markdown changed (a spec, a description, steps) and the code must follow.

1. `nlc check` — mark the pre-edit state clean (skip if already clean).
2. Edit the Markdown section.
3. `nlc` → status lists `[modified] <file>::<section>`. That's the change set.
4. `nlc tree <file>` (or `nlc graph | grep <file>`) → read off that section's
   code refs, e.g. `src/main.rs::L42`.
5. Open the referenced code at those lines; update it to match the new documented
   behavior.
6. `nlc check` — record the new baseline.

### Direction B — code moved → update the doc's line references

Code changed (inserted/deleted lines, moved a function) and the docs pointing at
it are now stale.

1. Find every doc referencing the changed file:
   `nlc graph | grep src/main.rs`
   Each `from -> src/main.rs::L..` line names the doc section and the
   (possibly stale) line.
2. For each referencing doc, open it, find the `[[src/main.rs#L..]]`, and
   re-derive the correct line/range from the **current** code.
3. `nlc` → a ref now past EOF yields `LineOutOfRange` (fix it). A ref that is
   *in range but wrong* is **not** caught — you must re-verify against the code.
4. `nlc check`.

### Daily loop

Edit docs/code → `nlc` to see the delta and any broken refs → fix → `nlc check`.

## Reading the status report

- **Header**: `scanned N markdown file(s), M code file(s), K section(s)`.
- **changed / revalidated / up-to-date**: the delta vs `.nlc-cache`.
  `revalidated = changed ∪ affected` (affected = pulled in by reverse edges).
- **changed nodes**: per-node `[added|modified|removed]`.
- **issues**: validation errors, filtered to revalidated nodes (`--full` for all).
- **regressions / resolutions**: nodes that flipped ok↔broken since last run.
- **verdict**: `ok` or `N error(s)`.

## Validation errors → fixes

| Error message contains | Cause | Fix |
|---|---|---|
| `references missing file` | path matches no `.md` or code file | correct the path or create the file |
| `references missing section` | heading text doesn't slug-match any section | fix the heading text or the ref |
| `ambiguous section` | two sections share the slug under different parents | disambiguate the heading or use a more specific target |
| `line … which has only N line(s)` | `#L<n>` / `#L<a>-L<b>` out of range | update to current line numbers (Workflow B) |
| `by name … only support #L<line>` | `[[code.rs#name]]` used on a code file | switch to `#L<n>` (named/AST targets aren't supported) |
| `circular dependency` | refs form a cycle | break the cycle (cycles are errors, like `make`) |
