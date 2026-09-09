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
- **`impact diff`** — the combined blast radius of a unified diff (`git diff | impact diff`, or `impact diff --file some.patch`): every symbol the diff's touched lines fall inside, across every file it mentions, in one call instead of one `impact query` per touched file. Requires the project to be indexed against the diff's *new* side — the working tree as it currently stands, which is what `git diff` on uncommitted changes already matches.
- **`impact mcp`** — an MCP stdio server exposing `impact_index` / `impact_file` / `impact_change` / `impact_diff` as tools, so an agent can call this directly instead of reading the whole codebase to guess what a change affects.
- **`impact hook pre-tool-use`** — a Claude Code `PreToolUse` hook: reads the hook payload on stdin and reminds the agent to check blast radius at the two moments the protocol names — the session's first file edit, and any `git commit`. `impact install` registers it in `settings.json`; a rule can be read once and forgotten, a hook fires whether or not the agent remembered.
- **Cross-project impact** — register sibling repos in a `workspace.toml` and `--workspace` extends a report with which *other* projects share the same API route / event / table identity, confidence-tiered (`Declared` / `Strong` / `Weak`) so identity coincidences don't masquerade as real dependencies.

Supports Rust, TypeScript/TSX (React), JavaScript/JSX (React Native), Python, Go, Kotlin (Android), and Swift today. The core (`impact-core`) is language-agnostic by design — each language is a pluggable adapter (tree-sitter-based symbol/call extraction), and adding another language means writing one more adapter crate, not touching the engine, linker, or MCP surface. Every adapter after the first (Rust) proved that boundary holds by adding zero lines to `impact-core`.

Full API/EVENTS/DATABASE contract detection (axum/sqlx/event conventions) is currently Rust-only. Go additionally detects one API shape — `net/http`'s Go 1.22+ method-prefixed routing (`mux.HandleFunc("POST /payments", handler)`) — in the same `"{VERB} {path}"` identity format the Rust detector uses, so a Go and a Rust service registering the same route are identity-matchable across a `workspace.toml`; EVENTS/DATABASE detection isn't wired up for Go yet. The remaining five languages get DIRECT/INDIRECT/TESTS only. Test detection follows whatever convention a language actually has one unambiguous answer for — pytest's `test`-prefix, `go test`'s `TestXxx` in `_test.go`, JUnit's `@Test`, XCTest's `test`-prefixed `XCTestCase` methods — and is intentionally left off for TypeScript/JavaScript, where Jest/Vitest/Mocha disagree.

## How it works

Structural resolution, not a compiler: `impact` parses source with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), extracts symbols and call sites, and resolves references using what the calling file itself says about a name — its `import`/`require`/`use` statements, its own declarations, and the declared type of the field or binding a method is called on — falling back to a bare short name only when none of that ties the call to a specific target. It doesn't type-check, so it can't always tell which of several same-named candidates a call resolves to — when that happens, it reports *all* of them rather than guessing wrong and staying silent. A blast-radius tool should over-report, not under-report: a false positive is visible and easy to dismiss, a false negative is invisible and costs you later.

Every DIRECT/INDIRECT entry carries the confidence behind it, and a multi-hop chain is only as trustworthy as its weakest hop: `Exact` when an import, a declared field/binding type, or a same-file declaration tied every hop back to what you queried, `Probable` when a hop matched only a project-unique bare name, `Heuristic` when a hop matched a bare short name shared by more than one candidate. Tree-text output tags anything below `Exact` inline (`caller::maybe_this [heuristic]`); `--min-confidence exact|probable` (CLI) or `min_confidence: "exact"|"probable"` (MCP) drops the weaker tiers when you only want what's certain.

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
| `impact query <file> [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--min-confidence exact\|probable\|heuristic] [--json]` | Blast radius of a file. |
| `impact change "<description>" [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--min-confidence exact\|probable\|heuristic] [--json]` | Blast radius of one symbol-level change. |
| `impact diff [--file <path>] [--project <dir>] [--cache-dir <dir>] [--workspace <toml>] [--min-confidence exact\|probable\|heuristic] [--json]` | Blast radius of a unified diff — reads from `--file`, or stdin if omitted. |
| `impact mcp` | Start the MCP stdio server. Blocks until stdin closes. |
| `impact gain [--daily\|--weekly\|--monthly] [--json]` | Usage analytics for `index`/`query`/`change`/`diff` (CLI and MCP combined), rolled up by day/week/month and broken down by client. Defaults to monthly. Recorded locally to `~/.impact/analytics.sqlite`; disable with `IMPACT_NO_ANALYTICS=1`. |

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

`<path>` is a `::`-joined qualified symbol path from the language's own indexed structure — module/package segments followed by the symbol, e.g. Rust's `some::Type::variant` or, for a Go method reached through nested packages, `service::repositories::changes::repository::Repository::AddOperation` (exactly what `impact_file`/`impact query` print in a report's `direct`/`indirect` entries — copy a path from there when unsure). It is never a filesystem path: `some/file.go::Symbol` does not resolve. A shorter form also works — either the last two segments (`Repository::AddOperation`) or just the bare symbol name (`AddOperation`) — but the fewer segments given, the more likely the name matches more than one symbol project-wide, which resolves to all of them at `Heuristic` confidence instead of one `Exact` match.

## MCP tools

| Tool | Description |
|---|---|
| `impact_index` | Index (or re-index) a project. |
| `impact_file` | Blast radius of a file, optionally extended with `workspace_path` for cross-project matches; `min_confidence: "exact"\|"probable"\|"heuristic"` filters DIRECT/INDIRECT entries. |
| `impact_change` | Blast radius of a `--change`-style description, same `workspace_path`/`min_confidence` support. |
| `impact_diff` | Blast radius of a unified diff (`diff` argument — the raw text, e.g. `git diff` output), same `workspace_path`/`min_confidence` support. |

Only calls `impact` can see syntactically become edges. A call reached through a registry or selector indirection (`getSelectors(state).canSchedule(...)`), or a function handed to something else as a value (`transform: camelizeOrder`) rather than called, leaves no edge — so an empty or thin blast radius is not proof that nothing consumes the symbol. Cross-check the symbol name with `grep` before concluding a change is safe.

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
impact install --no-hook       # rule and MCP server only, no Claude Code hook
impact doctor                  # check what's configured and whether the rule and hook are current
```

For Claude Code it also registers a `PreToolUse` hook in `settings.json`, merging into whatever hooks are already there. That is the part that does not depend on the agent remembering anything: the client runs `impact hook pre-tool-use` before the session's first file edit and before any `git commit`, and the reminder comes back as context whether or not the rule above was still in mind. `impact uninstall` removes only impact's own entry.

The rule text `impact install` writes — reproduced here for any other MCP-speaking agent (or CI system prompt) it doesn't have a built-in installer for:

```text
# Impact Blast-Radius Protocol — MANDATORY

**MANDATORY — three hard triggers, every task, no exceptions.**

## SESSION START
*Unconditional. Once, when you first start working in a project — before you know
whether this task will touch code at all.*
→ Load impact's tools now (if your client hides MCP tools behind a tool search, search
  for `impact_index` and load them), then call `impact_index` once with the project root.
→ Do this even when the task looks read-only. A tool you never loaded is not there to
  reach for when the triggers below fire, and an unindexed project makes them useless.

## BEFORE EDITING
*Two checkpoints that need no judgment call: before the first Edit/Write of a session,
and before any commit. Beyond those, whenever you rename, remove, or change the signature
of any function, type, enum variant, or field — or touch code behind an API route, event,
or database table. Don't wait to classify your own change first; run it and read the
report. This also covers proposing such a change: once your proposed fix is concrete enough to state
as a rename/remove/signature-change target, run this before presenting the proposal,
even if you haven't written any code yet. Vague, exploratory "here's roughly how I'd
approach it" discussion that hasn't settled on a concrete target doesn't need it.*
→ If this project hasn't been indexed yet this session (or has changed since), call
  `impact_index` once with the project root.
→ Then call `impact_file` (blast radius of a file) or `impact_change` (blast radius of a
  specific rename/remove/signature change — e.g. `"rename PaymentStatus::Failed"`,
  `"remove field User.email"`, `"change signature of PaymentService::charge"`) to see
  direct/indirect callers, API routes, event types, database tables, and affected tests
  before writing (or proposing) the change.
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

### `impact.toml` (per-project detector and index config)

Controls how API routes, events, and database tables are recognized, and which files the indexer skips. All fields are optional — a project with no `impact.toml` still gets useful detection from the defaults shown below.

```toml
[detectors.api]
# default — axum/net-http (Rust/Go), FastAPI+Flask (Python), Express+Fastify (TS/JS)
frameworks = ["axum", "net/http", "fastapi", "flask", "express", "fastify"]

[detectors.events]
strategy = "marker_trait"      # or "naming_convention"
marker_trait = "Event"         # default, used when strategy = "marker_trait"
naming_suffix = "Event"        # default, used when strategy = "naming_convention"

[detectors.database]
macros = ["query", "query_as", "query_scalar"]   # default (sqlx-family)

[index]
exclude = []   # default — extra glob patterns to skip beyond what .gitignore already covers
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
| [`impact-lang-ts`](crates/impact-lang-ts) | TypeScript/TSX and JavaScript/JSX (React, React Native): functions, classes, methods, cross-file call resolution — including calls hidden inside JSX expressions — Express/Fastify named-handler route detection, `*.test.*`/`*.spec.*`/`__tests__/` file-convention test detection. |
| [`impact-lang-python`](crates/impact-lang-python) | Functions, classes, methods, cross-file call resolution, pytest-style test detection, FastAPI/Flask route decorator detection. |
| [`impact-lang-go`](crates/impact-lang-go) | Functions, types, receiver methods (Go's top-level `method_declaration`, not nested in a class body), `go test`-style test detection, `net/http` method-prefixed route detection. |
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
