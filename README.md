# impact

[![CI](https://github.com/AncientiCe/impact-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/AncientiCe/impact-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.82+](https://img.shields.io/badge/rust-1.82%2B-orange.svg)](https://www.rust-lang.org)

Tell you what you're about to break, before you break it.

`impact` is a deterministic blast-radius tool for AI coding agents (and humans). Before you change a file or rename a symbol, it answers "what depends on this?" as one fast, structural query instead of an open-ended reasoning problem — turning "read the codebase and guess" into a tool call with a trustworthy, repeatable answer.

```
$ impact query src/payment/service.rs

DIRECT
  payment::controller::PaymentController::handle
INDIRECT
  order::OrderService::checkout
API
  POST /payments
EVENTS
  PaymentCreated
DATABASE
  payments
TESTS
  3 affected tests
```

## What it does

- **`impact index <path>`** — indexes a project into a local SQLite cache (content-hash-gated, so unchanged files are skipped on re-index).
- **`impact query <file>`** — the blast radius of everything declared in one file: direct callers, transitive (indirect) callers, the API routes / event types / database tables the affected code touches, and how many tests exercise any of it.
- **`impact change "<description>"`** — the same blast radius for a specific symbol-level change, described in a small deterministic grammar (never natural language, so the same input always resolves the same way): `rename <path>`, `remove <path>`, `remove variant <Enum>::<Variant>`, `remove field <Type>.<field>`, `change signature of <path>`.
- **`impact mcp`** — an MCP stdio server exposing `impact_index` / `impact_file` / `impact_change` as tools, so an agent can call this directly instead of reading the whole codebase to guess what a change affects.
- **Cross-project impact** — register sibling repos in a `workspace.toml` and `--workspace` extends a report with which *other* projects share the same API route / event / table identity, confidence-tiered (`Declared` / `Strong` / `Weak`) so identity coincidences don't masquerade as real dependencies.

Supports Rust, TypeScript/TSX (React), JavaScript/JSX (React Native), Python, Go, Kotlin (Android), and Swift today. The core (`impact-core`) is language-agnostic by design — each language is a pluggable adapter (tree-sitter-based symbol/call extraction), and adding another language means writing one more adapter crate, not touching the engine, linker, or MCP surface. Every adapter after the first (Rust) proved that boundary holds by adding zero lines to `impact-core`.

API/EVENTS/DATABASE contract detection (axum/sqlx/event conventions) is currently Rust-only; the other six languages get DIRECT/INDIRECT/TESTS. Test detection follows whatever convention a language actually has one unambiguous answer for — pytest's `test`-prefix, `go test`'s `TestXxx` in `_test.go`, JUnit's `@Test`, XCTest's `test`-prefixed `XCTestCase` methods — and is intentionally left off for TypeScript/JavaScript, where Jest/Vitest/Mocha disagree.

## How it works

Structural resolution, not a compiler: `impact` parses source with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), extracts symbols and call sites, and resolves references by name (exact qualified path first, falling back to a bare short name when a call site doesn't fully qualify its target). It doesn't type-check, so it can't always tell which of several same-named candidates a call resolves to — when that happens, it reports *all* of them rather than guessing wrong and staying silent. A blast-radius tool should over-report, not under-report: a false positive is visible and easy to dismiss, a false negative is invisible and costs you later.

Every DIRECT/INDIRECT entry carries the confidence behind it: `Exact` when the whole chain back to what you queried resolved unambiguously, `Heuristic` when any hop along the way only matched a bare short name shared by more than one candidate — a multi-hop chain is only as trustworthy as its weakest hop. Tree-text output tags anything below `Exact` inline (`caller::maybe_this [heuristic]`); `--min-confidence exact` (CLI) or `min_confidence: "exact"` (MCP) drops heuristic entries entirely when you only want what's certain.

## Installation

**macOS / Linux (Homebrew):**

```bash
brew install ancientice/impact/impact
```

**macOS / Linux (install script):**

```bash
curl -fsSL https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.ps1 | iex
```

**From source (any platform, requires Rust):**

```bash
cargo install --git https://github.com/AncientiCe/impact-rs --locked impact-cli
```

Prebuilt binaries and the Homebrew tap are populated by [`.github/workflows/release.yml`](.github/workflows/release.yml) on each tagged release (linux x86_64, macOS x86_64/arm64, Windows x86_64); `cargo install --git` always works off the latest source. `impact` is not yet published to crates.io.

## Quick start

```bash
cargo build --workspace

cargo run -p impact-cli -- index .
cargo run -p impact-cli -- query src/some/file.rs
cargo run -p impact-cli -- change "rename some::Type::variant"
cargo run -p impact-cli -- mcp
```

## CLI reference

| Command | Description |
|---|---|
| `impact index <path> [--force] [--cache-dir <dir>]` | Index (or re-index) a project. `--force` wipes the cache and re-parses everything, ignoring content-hash skips. |
| `impact query <file> [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--min-confidence exact\|heuristic] [--json]` | Blast radius of a file. |
| `impact change "<description>" [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--min-confidence exact\|heuristic] [--json]` | Blast radius of one symbol-level change. |
| `impact mcp` | Start the MCP stdio server. Blocks until stdin closes. |

## `--change` grammar

```text
rename <path>
rename <path> to <path>
remove <path>
remove variant <path>::<ident>
remove field <path>.<ident>
change signature of <path>
```

Unparseable input is a hard error with a usage hint — never a best-effort guess. Determinism is the whole pitch: the same description must always resolve the same way, independent of any model reading it.

## MCP tools

| Tool | Description |
|---|---|
| `impact_index` | Index (or re-index) a project. |
| `impact_file` | Blast radius of a file, optionally extended with `workspace_path` for cross-project matches; `min_confidence: "exact"\|"heuristic"` filters DIRECT/INDIRECT entries. |
| `impact_change` | Blast radius of a `--change`-style description, same `workspace_path`/`min_confidence` support. |

```bash
claude mcp add impact -- impact mcp
```

or add it manually to any MCP-speaking client's config:

```json
{
  "mcpServers": {
    "impact": { "command": "impact", "args": ["mcp"] }
  }
}
```

## Using this with an agent

Registering the MCP server (above) only gives an agent the *tools*; it still needs telling *when* to call them. `impact install` does both in one step for Cursor, Codex, Claude Code, and Claude Desktop — it registers the MCP server and writes an agent rule (a standalone `.cursor/rules/impact.mdc` for Cursor, a managed block in `AGENTS.md`/`CLAUDE.md` for Codex/Claude) with the same instructions every time, so every project gets consistent behavior instead of only the ones wired up by hand:

```bash
impact install                 # all four clients, user (global) scope
impact install --client cursor --scope project
impact doctor                  # check what's configured and whether the rule is current
```

The rule text `impact install` writes — reproduced here for any other MCP-speaking agent (or CI system prompt) it doesn't have a built-in installer for:

```text
# Impact Blast-Radius Protocol — MANDATORY

**MANDATORY — two hard triggers, every task, no exceptions.**

## BEFORE EDITING
*Before renaming, removing, or changing the signature of any function, type, enum
variant, or field — or touching code behind an API route, event, or database table.*
→ If this project hasn't been indexed yet this session (or has changed since), call
  `impact_index` once with the project root.
→ Then call `impact_file` (blast radius of a file) or `impact_change` (blast radius of a
  specific rename/remove/signature change — e.g. `"rename PaymentStatus::Failed"`,
  `"remove field User.email"`, `"change signature of PaymentService::charge"`) to see
  direct/indirect callers, API routes, event types, database tables, and affected tests
  before writing the change.
→ Treat a nonzero result as a checklist: update every caller and affected test the
  report names, not just the file you were asked to change.

## AFTER EDITING
*After the change is made, before considering the task done.*
→ Re-run `impact_index` (results are only as fresh as the last index), then re-run
  `impact_file`/`impact_change` against the same target to confirm the blast radius you
  addressed matches what's reported now, and nothing new appeared.

`impact_change` grammar: `rename <path>`, `rename <path> to <path>`, `remove <path>`,
`remove variant <Enum>::<Variant>`, `remove field <Type>.<field>`, `change signature of
<path>`. Not natural language — an unrecognized description is a hard error.
```

This is exactly what `impact install` generates (`crates/impact-cli/src/install/rule.rs`), not a paraphrase — pasting it verbatim into any other agent's system prompt or rules mechanism gets the same behavior `impact install`'s supported clients get automatically.

## Configuration

### `impact.toml` (per-project detector config)

Controls how API routes, events, and database tables are recognized. All fields are optional — a project with no `impact.toml` still gets useful detection from the defaults shown below.

```toml
[detectors.api]
frameworks = ["axum"]          # default

[detectors.events]
strategy = "marker_trait"      # or "naming_convention"
marker_trait = "Event"         # default, used when strategy = "marker_trait"
naming_suffix = "Event"        # default, used when strategy = "naming_convention"

[detectors.database]
macros = ["query", "query_as", "query_scalar"]   # default (sqlx-family)
```

### `workspace.toml` (cross-project registry)

```toml
[[projects]]
id = "backend"
path = "../payment-backend"
# cache_dir = "..."            # optional override; defaults to <path>/.impact

[[projects]]
id = "web"
path = "../checkout-web"

[[links]]
produces = "backend:POST /payments"   # names one exact contract
consumes = "web"                      # bare project id — the other side, generally
```

A `[[links]]` entry naming the exact contract on one side gives a `Declared` match; a link relating two projects generally (both sides bare) gives `Strong`; an identity match with no link at all is `Weak` — always shown, but clearly labeled, since two unrelated repos both exposing `POST /health` is a real possibility identity-matching alone can't rule out.

## Architecture

| Crate | Role |
|---|---|
| [`impact-core`](crates/impact-core) | Language-agnostic symbol graph, `LanguageAdapter`/`ContractRef` traits, SQLite-backed cache, linker, blast-radius engine, `--change` grammar, workspace/cross-project matching. Depends on the generic `tree-sitter` crate, never a specific grammar. |
| [`impact-lang-rust`](crates/impact-lang-rust) | The Rust adapter: functions, types, traits, enum variants, match-arm references, axum/sqlx/event contract detectors. |
| [`impact-lang-ts`](crates/impact-lang-ts) | TypeScript/TSX and JavaScript/JSX (React, React Native): functions, classes, methods, cross-file call resolution — including calls hidden inside JSX expressions. |
| [`impact-lang-python`](crates/impact-lang-python) | Functions, classes, methods, cross-file call resolution, pytest-style test detection. |
| [`impact-lang-go`](crates/impact-lang-go) | Functions, types, receiver methods (Go's top-level `method_declaration`, not nested in a class body), `go test`-style test detection. |
| [`impact-lang-kotlin`](crates/impact-lang-kotlin) | Functions, classes, methods, JUnit `@Test` detection. |
| [`impact-lang-swift`](crates/impact-lang-swift) | Functions, classes, methods, XCTest (`XCTestCase` inheritance) test detection. |
| [`impact-cli`](crates/impact-cli) | The `impact` binary — CLI subcommands and the MCP server, both built on one shared computation layer (`ops.rs`) so they can never drift from each other. |

## Performance

Measured with a release build (`cargo build --release`) against a real ~50k-line Rust + TypeScript workspace (207 files, 2,249 indexed symbols), on a desktop-class CPU (AMD Ryzen 7 7800X3D):

| Operation | Time |
|---|---|
| `impact index` (cold, full parse) | ~1.6s |
| `impact index` (re-run, nothing changed — content-hash skip) | ~0.1s |
| `impact query` (single file, warm cache) | ~35-40ms |

One machine, one project — treat these as an order-of-magnitude sense of cost, not a guarantee. The re-index number is the one that matters most in practice: an agent calling `impact_index` before every edit (per the Blast-Radius Protocol above) pays the cold-parse cost once and the ~0.1s content-hash-skip cost on every call after, as long as most files haven't changed.

## Development

See [`AGENTS.md`](AGENTS.md) for the full rules this project holds itself to: TDD via behavior tests only (real fixture projects, real CLI/MCP invocations — no internal-function unit tests), five quality gates, no mocks/placeholders/unsafe `unwrap`, changelog discipline.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo test --workspace
cargo audit
cargo build --workspace --locked
```

Or all five at once:

```bash
make check
```

See [`CHANGELOG.md`](CHANGELOG.md) for what's shipped so far.

## License

MIT — see [LICENSE](LICENSE).
