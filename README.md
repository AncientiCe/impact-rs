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

Supports Rust and TypeScript today. The core (`impact-core`) is language-agnostic by design — each language is a pluggable adapter (tree-sitter-based symbol/call extraction), and adding another language means writing one more adapter crate, not touching the engine, linker, or MCP surface. See [`impact-lang-ts`](crates/impact-lang-ts) — it was built specifically to prove that boundary holds.

## How it works

Structural resolution, not a compiler: `impact` parses source with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), extracts symbols and call sites, and resolves references by name (exact qualified path first, falling back to a bare short name when a call site doesn't fully qualify its target). It doesn't type-check, so it can't always tell which of several same-named candidates a call resolves to — when that happens, it reports *all* of them rather than guessing wrong and staying silent. A blast-radius tool should over-report, not under-report: a false positive is visible and easy to dismiss, a false negative is invisible and costs you later.

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
| `impact query <file> [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--json]` | Blast radius of a file. |
| `impact change "<description>" [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--json]` | Blast radius of one symbol-level change. |
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
| `impact_file` | Blast radius of a file, optionally extended with `workspace_path` for cross-project matches. |
| `impact_change` | Blast radius of a `--change`-style description, same `workspace_path` support. |

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
| [`impact-lang-ts`](crates/impact-lang-ts) | The TypeScript adapter: functions, classes, methods, cross-file call resolution. Proof that a second language needs zero `impact-core` changes. |
| [`impact-cli`](crates/impact-cli) | The `impact` binary — CLI subcommands and the MCP server, both built on one shared computation layer (`ops.rs`) so they can never drift from each other. |

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
