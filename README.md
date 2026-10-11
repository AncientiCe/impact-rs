# impact

[![CI](https://github.com/AncientiCe/impact-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/AncientiCe/impact-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.82+](https://img.shields.io/badge/rust-1.82%2B-orange.svg)](https://www.rust-lang.org)

**Tell you what you're about to break, before you break it.**

`impact` answers one question, fast and the same way every time: *"if I change this, what else is affected?"* Point it at a file, a symbol, or a `git diff`, and it lists every caller (direct and transitive), the API routes, events and database tables involved, and the tests that cover it.

It works for people at a terminal and for AI coding agents over [MCP](https://modelcontextprotocol.io). Agents get one tool call with a repeatable answer, instead of reading half the codebase and guessing.

```console
$ impact query src/repo.rs

DIRECT
  handlers::PaymentHandler::create_payment_route  src/handlers.rs:6
INDIRECT
  e2e_tests::creates_payment_route_end_to_end  src/e2e_tests.rs:4
API
  POST /payments
EVENTS
  PaymentCreated
DATABASE
  payments
TESTS
  1 affected tests
  e2e_tests::creates_payment_route_end_to_end  src/e2e_tests.rs:4
```

- **Fast.** Parses locally with tree-sitter. Re-indexing an unchanged project takes about 0.1s, and a query takes tens of milliseconds ([numbers](#performance)).
- **Deterministic.** No model in the loop, so the same input always gives the same report.
- **Errs toward over-reporting.** When it can't tell which of several same-named symbols a call means, it lists all of them and labels the match, rather than guessing and missing a caller.
- **Private.** Your code never leaves your machine ([details](#privacy-and-network-access)).
- **8 languages.** Rust, TypeScript/JavaScript (including React and React Native), Python, Go, Kotlin, Swift and C++.

---

## Contents

- [Install](#install)
- [Quick start (2 minutes)](#quick-start)
- [Use it with an AI coding agent](#use-it-with-an-ai-coding-agent)
- [Reading a report](#reading-a-report)
- [Language support](#language-support)
- [Command reference](#command-reference)
- [Describing a change: the `impact change` grammar](#describing-a-change-the-impact-change-grammar)
- [MCP tools](#mcp-tools)
- [Configuration](#configuration)
- [Cross-project impact](#cross-project-impact)
- [Privacy and network access](#privacy-and-network-access)
- [Troubleshooting](#troubleshooting)
- [How it works](#how-it-works)
- [Performance](#performance)
- [Contributing](#contributing)

---

## Install

| Platform | Command |
|---|---|
| macOS / Linux (Homebrew) | `brew install ancientice/impact/impact` |
| macOS / Linux (script) | `curl -fsSL https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.sh \| sh` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.ps1 \| iex` |
| Any platform, from source (needs Rust 1.82+) | `cargo install --git https://github.com/AncientiCe/impact-rs --locked impact-cli` |

The install scripts put the binary in `~/.local/bin` on macOS/Linux and `%LOCALAPPDATA%\Programs\impact\bin` on Windows. Set `IMPACT_INSTALL_DIR` to put it somewhere else. Every [tagged release](https://github.com/AncientiCe/impact-rs/releases) has prebuilt binaries for Linux x86_64, macOS x86_64/arm64 and Windows x86_64. `impact` is not on crates.io yet.

Check that it worked:

```bash
impact --version
```

**Upgrading.** Re-run the command you installed with, or use `brew upgrade impact`. `impact` tells you when a newer release is out (see [Privacy](#privacy-and-network-access)), but it never installs anything by itself. On Windows, upgrading works even while an agent session has `impact mcp` running. Sessions that are already open keep the old version until they restart.

## Quick start

Run these from the root of any project:

```bash
# 1. Build the index (stored in ./.impact/; later runs only re-parse changed files)
impact index .

# 2. What depends on this file?
impact query src/payments/service.rs

# 3. What breaks if I rename / remove / re-sign a specific symbol?
impact change "rename PaymentStatus::Failed"
impact change "change signature of PaymentService::charge"

# 4. What does my uncommitted work affect?
git diff | impact diff
```

Two habits worth having:

- **Add `.impact/` to your `.gitignore`.** It's a local cache, rebuilt on demand.
- **Re-run `impact index .` after you change code.** A report only reflects the last index. Re-indexing is cheap because unchanged files are skipped by content hash.

Every query command also takes `--json` for scripts and CI, `--summary` for huge results, `--min-confidence exact` to hide guesses, and `--explain` to show how each indirect caller is reached.

## Use it with an AI coding agent

Agents need two things: the **tools** (an MCP server) and a **rule** telling them *when* to use the tools. `impact install` sets up both:

```bash
impact install                                   # Cursor, Codex, Claude Code and Claude Desktop, user-wide
impact install --client cursor --scope project   # one client, this project only
impact install --dry-run                         # show what would change, write nothing
impact doctor                                    # is everything registered and up to date?
impact uninstall                                 # remove only what impact added
```

| Client | MCP server | Rule | Hook |
|---|---|---|---|
| Claude Code | ✅ | managed block in `CLAUDE.md` | ✅ `PreToolUse` |
| Codex | ✅ | managed block in `AGENTS.md` | — |
| Cursor | ✅ | `.cursor/rules/impact.mdc` | — |
| Claude Desktop | ✅ | — | — |

`--client` accepts `cursor`, `codex`, `claude`, `claude-desktop` or `all` (the default). `--no-rule` and `--no-hook` skip those parts.

**Why a hook too?** An agent can lose track of a rule partway through a session, but a hook runs every time. For Claude Code, `impact install` adds a `PreToolUse` hook to `settings.json`, merged with any hooks you already have. It reminds the agent to check blast radius before the first file edit of a session, before every `git commit`, and when it opens a pull or merge request (`gh pr create`, `glab mr create`). For a PR, the reminder asks it to put the `impact_diff` result in the description.

### Any other MCP client

Register the server by hand:

```bash
claude mcp add impact -- impact mcp
```

```json
{
  "mcpServers": {
    "impact": { "command": "impact", "args": ["mcp"] }
  }
}
```

Then paste the rule below into the agent's system prompt or rules file. It is exactly what `impact install` writes ([`crates/impact-cli/src/install/rule.rs`](crates/impact-cli/src/install/rule.rs)), so the agent behaves the same as one set up by `impact install`.

<details>
<summary><b>The agent rule (click to expand)</b></summary>

```text
# Impact Blast-Radius Protocol — MANDATORY

**MANDATORY — four hard triggers, every task, no exceptions.**

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

## OPENING A PULL REQUEST
*When you open a pull request or merge request (`gh pr create`, `glab mr create`, or
any other route), whether or not you edited code in this session.*
→ Call `impact_diff` on the change it carries (`git diff <base>...HEAD`) if impact is
  available.
→ Put what it reports in the description: the callers, routes, events, tables and tests
  the change reaches, and what you did about them. A reviewer can't see the graph; the
  description is where it reaches them.
→ Report only what impact returned. If it found nothing, or missed a caller you know
  about, say that rather than smoothing it over.

## BLIND SPOT FOUND
*Only after you've manually confirmed — by reading the code or grepping, not by
assumption — that impact_file/impact_change/impact_diff missed a real caller or
reported one that doesn't exist.*
→ Run `impact report-blindspot "<short title>"` (no `--submit`) to draft the report.
→ Show the drafted title and body to the user and get their explicit go-ahead before
  ever adding `--submit` — this files a public GitHub issue and is not yours to send
  unilaterally.
→ Never report a speculative or unverified gap — only one you've confirmed by hand.

`impact_change` grammar: `rename <path>`, `rename <path> to <path>`, `remove <path>`,
`remove variant <Enum>::<Variant>`, `remove field <Type>.<field>`, `change signature of
<path>`. Not natural language — an unrecognized description is a hard error.
```

</details>

## Reading a report

| Section | What it means |
|---|---|
| `DIRECT` | Code that calls or references something in the target. Each entry gives the symbol and its `file:line`. |
| `INDIRECT` | Code that reaches the target through one or more DIRECT callers: callers of callers, all the way up. |
| `API` | HTTP routes whose handlers are in the affected code, written as `VERB /path`. |
| `EVENTS` | Event types the affected code produces or consumes. |
| `DATABASE` | Tables the affected code queries. |
| `TESTS` | How many tests reach the affected code, and which ones. |

### Confidence

`impact` doesn't type-check, so each DIRECT/INDIRECT entry carries a label for how sure the match is:

| Tier | Meaning | Shown as |
|---|---|---|
| `exact` | An import, a declared field or variable type, or a declaration in the same file ties every step back to the target. | no tag |
| `probable` | At least one step matched only by a name that is unique in the project. | `[probable]` |
| `heuristic` | At least one step matched a name shared by several symbols, so it might be one of the others. | `[heuristic]` |

A chain of calls is only as strong as its weakest step. To see only `exact` matches, use `--min-confidence exact` (or `min_confidence: "exact"` over MCP). To see the chain behind a doubtful INDIRECT entry without reading code, use `--explain`.

### What it can't see

`impact` only sees calls written out in the source. A call made through a registry or selector (`getSelectors(state).canSchedule(...)`), or a function passed as a value instead of called (`transform: camelizeOrder`), produces no edge. **An empty or short report does not prove nothing uses a symbol.** Search for the name with `grep` before you treat a change as safe. If you confirm that `impact` missed something or reported something that isn't there, please [report it](#found-a-wrong-result).

## Language support

| Language | Callers (DIRECT/INDIRECT) | Tests detected by | API routes | Events / DB tables |
|---|---|---|---|---|
| Rust | ✅ | `#[test]`, `#[tokio::test]`, `#[async_std::test]` | axum | ✅ marker trait or naming convention · sqlx `query!`-family macros |
| TypeScript / TSX / JavaScript / JSX | ✅ | `*.test.*`, `*.spec.*`, `__tests__/` file names | Express, Fastify (named handlers) | — |
| Python | ✅ | pytest `test_` prefix | FastAPI, Flask | — |
| Go | ✅ | `TestXxx` in `_test.go` | `net/http` Go 1.22+ (`mux.HandleFunc("POST /payments", h)`) | — |
| Kotlin | ✅ | JUnit `@Test` | — | — |
| Swift | ✅ | XCTest `test…` methods on `XCTestCase` | — | — |
| C++ | ✅ (including out-of-line `Type::method` definitions) | — (no single convention across GoogleTest, Catch2 and Boost.Test) | — | — |

Routes use the same `VERB /path` format in every language, so a Go service and a TypeScript client using the same route can be matched across repositories (see [Cross-project impact](#cross-project-impact)).

Files covered by `.gitignore` are skipped automatically. To skip more, see [Configuration](#configuration).

## Command reference

Run `impact <command> --help` for the full flag list of any command.

| Command | What it does |
|---|---|
| `impact index <path> [--force] [--cache-dir <dir>] [--json]` | Builds or refreshes the index. `--force` deletes the cache and re-parses everything. |
| `impact query <file>` | Blast radius of everything declared in one file. |
| `impact change "<description>"` | Blast radius of one symbol-level change ([grammar](#describing-a-change-the-impact-change-grammar)). |
| `impact diff [--file <patch>]` | Combined blast radius of a unified diff, read from stdin or `--file`. The index must match the diff's *new* side, which is your working tree as it is now. |
| `impact install` / `uninstall` / `doctor` | Set up, remove or check agent integration ([details](#use-it-with-an-ai-coding-agent)). |
| `impact mcp` | Runs the MCP stdio server. It keeps running until stdin closes. |
| `impact report-blindspot "<title>"` | Drafts a GitHub issue about a wrong result ([details](#found-a-wrong-result)). |
| `impact gain [--daily\|--weekly\|--monthly] [--json]` | Your local usage statistics, per period and per client. |
| `impact hook pre-tool-use` | The Claude Code hook entry point. `impact install` sets it up, so you don't run it yourself. |

`query`, `change` and `diff` share these options:

| Option | Effect |
|---|---|
| `--project <dir>` | The project root the index was built from. Defaults to the current directory. |
| `--cache-dir <dir>` | Where the index lives. Defaults to `<project>/.impact`. |
| `--workspace <toml>` | Also report matches in sibling projects ([details](#cross-project-impact)). |
| `--min-confidence exact\|probable\|heuristic` | Hides DIRECT/INDIRECT entries below this tier. |
| `--explain` | Shows how each INDIRECT entry is reached from its nearest DIRECT caller. |
| `--summary` | Compact output: counts per section, all DIRECT entries, and INDIRECT entries grouped by file. Use it when a widely used file produces hundreds of entries. |
| `--json` | Machine-readable output. |

## Describing a change: the `impact change` grammar

```text
rename <path>
rename <path> to <path>
remove <path>
remove variant <Enum>::<Variant>
remove field <Type>.<field>
change signature of <path>
```

This is a small fixed grammar, not natural language, so the same description always resolves the same way. Anything it can't parse is an error with a usage hint, not a guess.

`<path>` is a symbol path joined with `::`, in the form `impact` prints it: module segments, then the symbol, such as `payments::PaymentService::charge`. If you're unsure, copy a path from the DIRECT or INDIRECT section of a report. It is **never** a file path: `src/payments.rs::charge` won't resolve. Shorter forms also work, either the last two segments (`PaymentService::charge`) or just the name (`charge`). The fewer segments you give, the more likely the name matches several symbols. Those are all reported, at `heuristic` confidence.

## MCP tools

`impact mcp` provides these tools. Every query tool requires `project_path`, the same root you passed to `impact_index`.

| Tool | Arguments | What it does |
|---|---|---|
| `impact_index` | `project_path`, optional `force`, `cache_dir` | Builds or refreshes the index. Its result also lists `orphaned_events`: events that had both a producer and a consumer at the last index and have lost one of them. This often happens when an event is replaced by a direct call that leaves out something the old consumer did. |
| `impact_file` | `project_path`, `path` | Blast radius of a file. |
| `impact_change` | `project_path`, `description` | Blast radius of a [change description](#describing-a-change-the-impact-change-grammar). |
| `impact_diff` | `project_path`, `diff` (the raw `git diff` text) | Blast radius of a unified diff. |
| `impact_report_blindspot` | `title`, `body`, optional `kind`, `language`, `repo`, `submit` | Drafts a blind-spot issue. It only files one when `submit: true` is passed. |

`impact_file`, `impact_change` and `impact_diff` also accept `workspace_path`, `min_confidence`, `summary`, `explain` and `cache_dir`, which work like the CLI options of the same names. Use `summary: true` when a result could exceed your client's response size limit.

## Configuration

### `impact.toml`: detection and indexing, per project

Put this file at the project root. Every field is optional, and the values below are the defaults.

```toml
[detectors.api]
frameworks = ["axum", "net/http", "fastapi", "flask", "express", "fastify"]

[detectors.events]
strategy = "marker_trait"      # or "naming_convention"
marker_trait = "Event"         # types implementing this trait are events
naming_suffix = "Event"        # with naming_convention: types ending in this are events

[detectors.database]
macros = ["query", "query_as", "query_scalar"]   # sqlx-style macros whose SQL names tables

[index]
exclude = []   # extra glob patterns to skip, beyond .gitignore
```

### Environment variables

| Variable | Effect |
|---|---|
| `IMPACT_NO_UPDATE_CHECK=1` | Turns off the daily new-release check. |
| `IMPACT_NO_ANALYTICS=1` | Turns off local usage recording (`impact gain`). |
| `CI` | When set, the update check is skipped. |
| `IMPACT_INSTALL_DIR` | Install scripts only: where to put the binary. |

## Cross-project impact

If your system spans several repositories (a backend, a web client, a worker), register them in a `workspace.toml`:

```toml
[[projects]]
id = "backend"
path = "../payment-backend"
# cache_dir = "..."            # optional; defaults to <path>/.impact

[[projects]]
id = "web"
path = "../checkout-web"

[[links]]
produces = "backend:POST /payments"   # one exact contract on one side
consumes = "web"                      # a whole project on the other
```

Index each project, then add `--workspace workspace.toml` (or `workspace_path` over MCP) to any query. The report then lists which other projects share an affected API route, event or table, labeled by how strong the evidence is:

| Label | When |
|---|---|
| `Declared` | A `[[links]]` entry names this exact contract. |
| `Strong` | A `[[links]]` entry connects the two projects in general. |
| `Weak` | The names match but nothing links the projects. It is always shown and always labeled, because two unrelated services can both expose `POST /health`. |

## Privacy and network access

- **Your code stays local.** Indexing and queries run entirely on your machine. The index lives in `<project>/.impact/`.
- **Update check.** Once a day, `impact` makes one request to GitHub's releases API (2-second timeout). If a newer version exists, it says so on stderr after an analysis command, or, over MCP, in the first tool result of a session. It never downloads or installs anything. It stays silent when offline, is skipped when `CI` is set, and `IMPACT_NO_UPDATE_CHECK=1` turns it off.
- **Usage statistics are local.** Command counts for `impact gain` go to `~/.impact/analytics.sqlite` and are never sent anywhere. `IMPACT_NO_ANALYTICS=1` turns this off.
- **Blind-spot reports are opt-in.** `report-blindspot` only prints a draft. It sends something only when you pass `--submit`, and then it uses your own `gh` CLI.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `… has not been indexed — run impact index first` | Run `impact index <project root>`, or `impact_index` over MCP, then query again. |
| `"X" doesn't resolve to anything in the indexed project` | Check the [symbol path](#describing-a-change-the-impact-change-grammar) by copying one from a report. If the code changed since the last index, re-index. |
| A caller you just added or removed isn't reflected | Reports reflect the last index. Run `impact index .` again. |
| MCP: `project_path is required` | Pass the project root as `project_path` on every tool call. |
| The report is enormous, or the MCP response is too large | Use `--summary` / `summary: true`, and optionally `--min-confidence exact`. |
| An entry tagged `[heuristic]` looks wrong | Run the query again with `--explain` to see the chain behind it, or use `--min-confidence exact`. |
| The report is empty, but you know the symbol is used | It may be used through a registry or passed as a value ([what it can't see](#what-it-cant-see)). Search with `grep`. If it's a plain call that was missed, [report it](#found-a-wrong-result). |
| The agent never calls impact | Run `impact doctor`. If it shows something missing, run `impact install` and restart the agent. |

### Found a wrong result?

```bash
impact report-blindspot "Go: method call through embedded struct not linked" --kind missed-edge --language go
```

This prints a draft issue and touches nothing else. **Remove anything private** before you submit (local paths, internal repo, file or symbol names) and describe the code pattern in general terms instead. When the draft is ready, add `--submit`. `impact` checks for an existing report with the same fingerprint, then files the issue through `gh`.

## How it works

`impact` parses source code with [tree-sitter](https://tree-sitter.github.io/tree-sitter/). It records every symbol and call site, then works out what each call refers to from what the calling file says: its `import`/`use`/`require` statements, its own declarations, and the declared type of the variable or field a method is called on. Only when none of that settles the question does it fall back to matching the bare name. When several symbols could match, it reports all of them with a lower confidence label.

That trade-off is deliberate. An extra entry in a report is visible and easy to dismiss. A missing one goes unnoticed until the change breaks something.

The engine (`impact-core`) is language-agnostic. Each language is a separate adapter crate, and adding a language means adding one crate without changing the engine, the linker or the MCP server.

| Crate | Role |
|---|---|
| [`impact-core`](crates/impact-core) | Symbol graph, SQLite cache, linker, blast-radius engine, change grammar, cross-project matching. |
| [`impact-lang-rust`](crates/impact-lang-rust) | Rust: functions, types, traits, enum variants, match-arm references, axum/sqlx/event detection. |
| [`impact-lang-ts`](crates/impact-lang-ts) | TypeScript/TSX and JavaScript/JSX: imports and aliases, barrel re-exports, CommonJS, calls inside JSX, Express/Fastify routes. |
| [`impact-lang-python`](crates/impact-lang-python) | Python: functions, classes, methods, pytest, FastAPI/Flask routes. |
| [`impact-lang-go`](crates/impact-lang-go) | Go: functions, types, receiver methods, `go test`, `net/http` routes. |
| [`impact-lang-kotlin`](crates/impact-lang-kotlin) | Kotlin: functions, classes, methods, JUnit. |
| [`impact-lang-swift`](crates/impact-lang-swift) | Swift: functions, classes, methods, XCTest. |
| [`impact-lang-cpp`](crates/impact-lang-cpp) | C++: functions, classes/structs, methods, including out-of-line definitions. |
| [`impact-cli`](crates/impact-cli) | The `impact` binary. The CLI and the MCP server share one computation layer (`ops.rs`), so their results always agree. |

## Performance

These numbers come from a release build indexing a real Rust + TypeScript workspace of about 50k lines (207 files, 2,249 symbols) on an AMD Ryzen 7 7800X3D:

| Operation | Time |
|---|---|
| `impact index`, first run (full parse) | ~1.6s |
| `impact index`, nothing changed (content-hash skip) | ~0.1s |
| `impact query`, single file | ~35–40ms |

That's one machine and one project, so take these as orders of magnitude. The second row matters most: an agent re-indexing before every edit pays for the full parse once, and about 0.1s per call after that.

## Contributing

Bug reports, blind-spot reports and new language adapters are welcome. Before you write code, read [`AGENTS.md`](AGENTS.md). It holds the project's binding rules: behavior tests written first against real fixture projects, no mocks or placeholders, no `unwrap` in production code, and a `CHANGELOG.md` entry for every change.

```bash
make check   # fmt, clippy -D warnings, test, audit, locked build: the definition of "green"
```

The five commands it runs, if you want them one at a time:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo test --workspace
cargo audit
cargo build --workspace --locked
```

To try a local build without installing it, use `cargo run -p impact-cli -- index .` (any subcommand works). See [`CHANGELOG.md`](CHANGELOG.md) for release history.

## License

MIT. See [LICENSE](LICENSE).
