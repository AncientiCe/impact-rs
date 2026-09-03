# Changelog

All notable changes to `impact` are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.5.1] - 2026-09-03

### Fixed

- `cargo test`/`make check` in this checkout no longer pollutes a developer's real
  `~/.impact/analytics.sqlite`: dozens of pre-existing integration tests invoke the
  compiled `impact` binary via `assert_cmd` without knowing anything about usage
  analytics, so every one of them fell through to `analytics::db_path`'s real default
  and quietly recorded test noise into it on every test run — including MCP-sourced
  rows attributed to `"unknown"` (`tests/mcp.rs`'s hand-rolled JSON-RPC helper never
  sends `clientInfo`, unlike a real MCP client). New `.cargo/config.toml` sets
  `IMPACT_ANALYTICS_DB` to a `target/`-local, gitignored path for every `cargo`
  invocation in this workspace (`cargo run`/`cargo test` alike), so only a properly
  installed/released `impact` binary — never a dev checkout's build or test suite —
  writes to the real global DB. `crates/impact-cli/tests/analytics.rs`'s own tests
  already set `IMPACT_ANALYTICS_DB` explicitly per test and are unaffected.

### Changed

- `impact gain`'s human (non-`--json`) output is now a colored bar chart instead of a
  plain aligned list: each `BY CLIENT`/`BY COMMAND` entry gets a `█`/`░` bar plus a
  percentage of the bucket's total, bucket totals are thousands-grouped (`1,234 calls`),
  and a bucket with any failed calls shows the count in yellow. Color (bold headers, a
  dim divider rule, a cyan bar fill) is only emitted when stdout is a real terminal and
  `NO_COLOR` isn't set — piped/redirected output (scripts, `impact gain | less`, this
  project's own tests) stays plain ASCII with no ANSI escapes. 1 new behavior test
  (`crates/impact-cli/tests/analytics.rs`) asserting the piped output is escape-free and
  contains the bar/percentage columns.

## [0.5.0] - 2026-09-03

### Added

- `impact gain`: local usage analytics for the four analysis operations (`index`/
  `file`/`change`/`diff`), recorded from both the CLI (`impact index`/`query`/`change`/
  `diff`) and the MCP tools (`impact_index`/`impact_file`/`impact_change`/`impact_diff`)
  into a new global SQLite DB (`~/.impact/analytics.sqlite` by default, overridable via
  `IMPACT_ANALYTICS_DB`; disable entirely with `IMPACT_NO_ANALYTICS=1`). Each event
  carries which command ran, whether it came via the CLI or MCP, the reporting AI
  client's name (taken from the MCP `initialize` request's `clientInfo`, so usage can be
  broken down "by all the AIs using this" — falls back to `"cli"`/`"unknown"` when no
  client identifies itself), duration, and success. `impact gain` rolls these up by day/
  week/month (`--daily`/`--weekly`/`--monthly`; defaults to monthly) into per-bucket
  totals plus `by_client`/`by_command` breakdowns, printed as tree-text or `--json`.
  Recording is best-effort and silent on failure — it can never fail the real command
  that triggered it, and never makes a network call. `install`/`uninstall`/`doctor`
  stay untracked (one-off setup, not repeated "usage"). New `crates/impact-cli/src/
  analytics.rs` module and `crates/impact-cli/tests/analytics.rs` behavior tests
  (CLI recording + rollup, period label shapes, the opt-out env var, and the MCP
  `clientInfo` attribution) driving the compiled binary and the real MCP stdio protocol.

## [0.4.1] - 2026-09-03

### Changed

- The installed agent rule (`CLAUDE.md`/`AGENTS.md`/Cursor's `impact.mdc`) and the MCP server's `initialize` `instructions` now widen the "before editing" trigger to also cover *proposing* a fix: once a proposed fix is concrete enough to state as a rename/remove/signature-change target, agents are told to run impact analysis before presenting the proposal, even if no code has been written yet. Vague, exploratory "here's roughly how I'd approach it" discussion still doesn't need it. Previously the trigger only fired once an edit had actually started, which could leave a concrete proposal evaluated without blast-radius info. 2 new behavior tests (`crates/impact-cli/tests/install.rs`, `crates/impact-cli/tests/mcp.rs`) asserting the installed rule text and the MCP instructions both mention proposing while still excluding vague/exploratory discussion.

## [0.4.0] - 2026-09-02

### Added

- The SQLite index cache now carries a schema version (`PRAGMA user_version`, `impact_core::cache::SCHEMA_VERSION`). `Cache::migrate` compares it against the current version on open and, if a previously-created cache is stale, wipes every table and re-stamps it before continuing — instead of silently trying to reuse rows in a shape the running binary no longer understands. Prints a one-line notice to stderr when this happens; a brand-new cache (nothing to wipe) stays silent. This is the precondition for evolving the cache schema (e.g. adding new columns) without requiring every user to remember `impact index --force` after an upgrade. 2 new behavior tests (`crates/impact-cli/tests/schema_version.rs`) hand-writing a pre-versioning-shaped cache.sqlite and confirming both the wipe-and-reindex path and the fresh-cache silent path.
- Every `Dependent` (a DIRECT/INDIRECT entry) now carries `file`/`line` alongside `path`/`confidence`, taken from the graph node it resolved to. Tree-text output prints them inline (`caller::maybe_this  src/caller.rs:12`) instead of a bare qualified path, so an agent (or human) can jump straight to the location without a second JSON round-trip. Additive JSON change — existing `path`/`confidence` fields are untouched.
- `ImpactReport` gains `affected_tests: Vec<Dependent>` alongside the existing `tests: usize` count — the actual test dependents (already carrying their own file/line), not just how many there are, so an agent can run exactly those tests instead of the whole suite. `tests` now always equals `affected_tests.len()`, including after `--min-confidence` filtering (which now also filters `affected_tests`). Tree-text output lists them under `TESTS`, below the existing count line. 1 new behavior test (`crates/impact-cli/tests/contracts.rs`) asserting both the JSON `affected_tests` field and the tree-text rendering for a real cross-file test dependent.
- `impact query`/`impact change`/`impact diff --explain` and the equivalent `explain: true` argument on the `impact_file`/`impact_change`/`impact_diff` MCP tools: populates each INDIRECT `Dependent`'s new `via` field with the chain of intermediate dependents back to its nearest DIRECT ancestor (the shortest path the BFS blast-radius walk already found), so an agent can verify a `[heuristic]` entry's connection to what it queried without a second read of the code. `via` is computed unconditionally by the engine (cheap — following existing BFS parent pointers) but cleared by `apply_explain` unless `explain` is requested, and omitted from JSON entirely when empty (`#[serde(skip_serializing_if)]`), so a default report's shape is unchanged. Tree-text output prints it as an indented `via a -> b` line under the entry. 3 new behavior tests (2 CLI in `crates/impact-cli/tests/query.rs`, 1 MCP in `crates/impact-cli/tests/mcp.rs`) covering the default-empty case, `--explain` populating a real 2-hop chain, and the MCP argument reaching the same mechanism.
- `impact-lang-python` now detects one API contract shape (gated on `impact.toml`'s `api_frameworks` containing `"fastapi"` and/or `"flask"`, both on by default): a route decorator directly above a `def` — `@app.get("/payments")`-style verb aliases (both frameworks share this shape) and Flask's original `@app.route("/payments", methods=["POST"])` form (verb read from the first string in `methods=[...]`, defaulting to `GET` when absent). The decorated function is the handler, so `symbol_name` is its own qualified path — no separate handler argument to extract, unlike axum/net-http's `.route(path, verb(handler))` shape. `PythonAdapter` now takes a `DetectorConfig` (`PythonAdapter::new(config)`), mirroring `RustAdapter`/`GoAdapter`; the config-less form remains available via `PythonAdapter::default()`. New `python_contracts` fixture and 1 new behavior test covering both decorator forms.
- `impact-lang-ts` now detects one API contract shape (same `api_frameworks` gate, `"express"` and/or `"fastify"`, both on by default): an `app.get(path, handler)`-style route registration call — Express's method-chaining API and Fastify's shortcut methods share the exact same call shape, so one detector covers both. Only a named-function or `object.method` handler reference is recognized; an inline arrow/function-expression handler yields no route rather than guessing at one of its inner identifiers. `TsAdapter` now takes a `DetectorConfig` the same way. New `ts_contracts` fixture and 1 new behavior test covering both the recognized and unrecognized handler forms. Both adapters' route detectors were written only after dumping a real tree-sitter parse tree for each decorator/call shape first, per this project's established practice.
- `impact-lang-ts` now detects tests by file-naming convention: every function/method declared in a file whose name contains `.test.` or `.spec.` (`foo.test.ts`, `foo.spec.tsx`), or that lives under a `__tests__/` directory, is marked `is_test` — the one convention Jest, Vitest, and Mocha's default configs all actually share, unlike each framework's own call-based marker (`test()`/`it()`) which this adapter still doesn't attempt to recognize (would need call-site analysis, and the three frameworks disagree on it). New `ts_test_detection` fixture (one caller per recognized convention, plus a same-shaped caller in an ordinarily-named file to prove this isn't "every caller gets marked") and 1 new behavior test.
- `impact.toml`'s new `[index]` table: `exclude = [...]`, extra glob patterns (matched the same way a `LanguageAdapter`'s own `file_globs` are) for files the indexer skips even though some adapter's globs would otherwise claim them — for vendored or generated code that isn't already covered by `.gitignore` (which the indexer's file walker already respects on its own). `Indexer::new` gains a `with_exclude` builder method; `impact_core::IndexConfig::load` reads the new table the same way `DetectorConfig::load` reads `[detectors]` (both now share one `parse_impact_toml` helper so the file is only read/parsed once). New `index_exclude` fixture and 1 new behavior test comparing indexing the fixture as-is against an identical copy with `impact.toml` removed.
- `SymbolDecl`/`Node` now carry `end_line` — the 1-indexed, inclusive last line of a symbol's own declaration span — alongside the existing `line`, populated by all seven language adapters from their tree-sitter node's `end_position()` (bumps the cache schema to v2, wiping any v1 cache on next open per the versioning above). `compute_diff_impact` now matches a diff's touched lines against each candidate symbol's real `[line, end_line]` span instead of falling back to "the nearest declaration at or before the touched line" for anything inside a symbol's body — a strictly more precise structural approximation, not a claim of full AST containment (an `impl` block's own boundary, for instance, still isn't a symbol of its own). 1 new behavior test (`crates/impact-cli/tests/diff.rs`) touching a line past a function's closing brace but before the next declaration, which the old nearest-preceding-declaration fallback would have wrongly attributed to the earlier function; span matching correctly reports no impact.
- `release.yml`'s build matrix now also produces `aarch64-unknown-linux-gnu` binaries (cross-compiled on the `ubuntu-latest` x86_64 runner via an installed `gcc-aarch64-linux-gnu` toolchain, needed both for linking and for building rusqlite's bundled SQLite through `cc`). `scripts/install.sh` no longer refuses Linux ARM64 with a "not shipped yet" error — it resolves the target and installs normally, same as every other supported platform.

## [0.3.0] - 2026-09-02

### Added

- `ImpactReport.direct`/`.indirect` entries now carry a `confidence` tier (`Exact`/`Probable`/`Heuristic`) alongside `path`, surfacing data the linker already computed (`Confidence::Exact`/`Heuristic` per edge, see `impact-core::linker::Resolver`) but previously discarded before it reached the CLI/MCP output. A multi-hop chain's confidence is its weakest link, not just the last hop's — one heuristic hop (a bare short-name call site matching more than one candidate) makes the whole chain only as trustworthy as that hop.
- `impact query`/`impact change --min-confidence exact|heuristic` and the equivalent `min_confidence` argument on the `impact_file`/`impact_change` MCP tools: filters DIRECT/INDIRECT entries below the given confidence, so an agent (or human) can ask for only unambiguous dependents instead of reading every heuristic match.
- Tree-text output tags non-`Exact` entries inline (`caller::maybe_this [heuristic]`) rather than leaving confidence JSON-only.
- New `confidence` fixture and 2 new behavior tests (`crates/impact-cli/tests/confidence.rs`) hand-verifying a single query surfacing both an `Exact` and a `Heuristic` DIRECT entry, and `--min-confidence exact` filtering the heuristic one out. Plus 1 new MCP-level test confirming `impact_file`'s `min_confidence` argument reaches the same filter.
- `impact-lang-go` now detects one API contract shape: `net/http`'s Go 1.22+ method-prefixed routing (`mux.HandleFunc("POST /payments", handler)`), gated on `impact.toml`'s `api_frameworks` containing `"net/http"` (on by default, alongside `axum`, so both Rust and Go projects get useful API detection with no config file). The pattern's verb-prefixed form already matches the exact `"{VERB} {path}"` identity string `impact-lang-rust`'s axum detector produces, so a Go and a Rust service registering the same route are identity-matchable across a `workspace.toml` with no extra normalization. `GoAdapter` now takes a `DetectorConfig` (`GoAdapter::new(config)`), mirroring `RustAdapter`; the config-less form remains available via `GoAdapter::default()`. Method-less patterns (`mux.HandleFunc("/path", handler)`) are deliberately not recognized — there's no verb to report. New `go_contracts` fixture and 1 new behavior test.
- `impact diff` CLI subcommand and `impact_diff` MCP tool: the combined blast radius of a unified diff (`git diff | impact diff`, or `impact diff --file some.patch`; the MCP tool takes the diff text directly as a `diff` argument) — every symbol the diff's touched lines fall inside, across every file it mentions, computed in one call instead of one `impact query`/`impact_file` per touched file. New `impact_core::diff` module (`parse_unified_diff`, hand-rolled — recognizes `+++ b/path` headers and `@@ -o,oc +n,nc @@` hunk headers, nothing else) and `compute_diff_impact` (maps each touched line to the nearest symbol declaration at or before it in the same file — there's no end-line span recorded per symbol, so this is a structural approximation, same as every other adapter's scope, not precise AST containment). Same `--workspace`/`--min-confidence`/`--json` support as `query`/`change`. 5 new behavior tests (3 CLI, 1 MCP, reusing the existing `multi_file` fixture with hand-crafted diffs) covering a diff that only touches a function's body (not its declaration line, to prove the nearest-preceding-declaration fallback), a leaf file with no callers, and the `--file`-vs-stdin input paths.

### Changed

- **Breaking**: `ImpactReport.direct`/`.indirect` changed from `Vec<String>` to `Vec<Dependent>` (`{path, confidence}`) in both the Rust API and the JSON output — every existing CLI/MCP consumer parsing these fields as bare strings needs to read `.path` instead.

## [0.2.0] - 2026-09-01

### Added

- `impact install`/`impact uninstall`/`impact doctor` CLI subcommands: register (or remove) the `impact` MCP server at user (global) or project scope in Cursor (`~/.cursor/mcp.json`), Codex (`~/.codex/config.toml`, via `toml_edit` to preserve comments/formatting), Claude Code (`~/.claude.json`), and Claude Desktop (platform-specific config path; unsupported on Linux) — plus an `impact`-owned agent rule (Cursor: standalone `.cursor/rules/impact.mdc`; Codex/Claude: a managed `<!-- BEGIN IMPACT -->`/`<!-- END IMPACT -->` block in `AGENTS.md`/`CLAUDE.md`) telling the agent to call `impact_index`/`impact_file`/`impact_change` before and after editing code, in every project automatically instead of only ones wired up by hand. `impact doctor` reports per-client configured/missing/rule-installed/rule-current status. `--dry-run`, `--no-rule`, `--scope user|project`, `--home-dir` (portable/non-standard profile support), and `--json` supported across all three subcommands.

## [0.1.0] - 2026-09-01

### Added

- Installation: `scripts/install.sh` / `scripts/install.ps1` (download, checksum-verify, and install a release binary, or install from a local archive via `IMPACT_VERSION=local`), a tag-triggered `.github/workflows/release.yml` building linux x86_64 / macOS x86_64+arm64 / Windows x86_64 binaries with checksums, and `.github/workflows/update-homebrew.yml` which opens a PR against the [`homebrew-impact`](https://github.com/AncientiCe/homebrew-impact) tap after each release. CI's `install` job runs the install scripts against a freshly built binary on Linux, macOS, and Windows on every push; `release.yml`'s `install-smoke` job additionally proves them against a real published release on all three OSes.
- Project rules (`AGENTS.md`, `CLAUDE.md`): TDD via behavior tests only, quality gates (fmt/clippy/audit/test/build), internal API discipline, no mocks/placeholders/unsafe unwrap, changelog discipline.
- Quality gate tooling: `Makefile` (`make check` running all five gates) and CI (`.github/workflows/ci.yml`, one job per gate).
- Phase 0 scaffolding: Cargo workspace with `impact-core` (language-agnostic symbol graph, `LanguageAdapter` trait, SQLite-backed cache with content-hash-gated re-indexing, file-walking indexer), `impact-lang-rust` (tree-sitter-based Rust adapter extracting functions/structs/enums/traits with approximate module-path qualification), and `impact-cli` (the `impact` binary's `index` subcommand).
- Behavior tests for `impact index`: symbol count against a hand-counted fixture crate, and unchanged-file skip on a second run — both via `assert_cmd` against the real binary.
- Phase 1: call-reference extraction (`LanguageAdapter::extract_references`, `RefDecl`), a structural linker (`impact_core::link`) that resolves call sites to graph `Calls` edges with `Exact`/`Heuristic` confidence, and a `BlastRadiusEngine` (`compute_file_impact`) doing reverse BFS for DIRECT (1-hop) and INDIRECT (2+-hop) callers.
- `impact query <file>` CLI subcommand: prints the DIRECT/INDIRECT tree-text report (or `--json`) for a file's blast radius, against an already-built index.
- Behavior tests for `impact query`: DIRECT/INDIRECT callers traced across a 3-file fixture, a leaf file with no callers, and a generic-`impl` regression (found by dogfooding against `impact-core` itself: `impl<'a> Indexer<'a>` was leaking `<'a>` into the reported qualified path) fixed via `impl_type_name`.

- Phase 2: `impact.toml`-driven contract detectors (`impact_core::config::DetectorConfig`) for all three remaining report sections — API (axum `.route(path, verb(handler))` registrations), EVENTS (marker-trait `impl Event for X`, or a configurable naming-suffix convention), DATABASE (`sqlx::query!`-family macros, naive `FROM`/`JOIN`/`INTO`/`UPDATE` table extraction) — plus TESTS (`#[test]`/`#[tokio::test]` detection feeding a reverse-reachability count). `ImpactReport` now carries `api`/`events`/`database`/`tests` alongside `direct`/`indirect`.
- The blast-radius engine's reverse-BFS now walks `Produces`/`Consumes`/`Reads`/`Writes` edges as reverse-dependency edges too, not just `Calls`/`References` — otherwise querying the file that *declares* an event or is the only writer of a table (rather than the file that calls into it) reported nothing, even though every producer/consumer of that contract is exactly its blast radius.
- 4 new fixtures (`contracts`, `naming_events`) and 4 new behavior tests covering all four contract kinds, the `NamingConvention` event strategy read from a real `impact.toml`, and the cross-cutting Produces/Consumes reverse-dependency behavior.

### Fixed

- Generic `impl<T> Type<T>` blocks no longer leak their type-parameter list into extracted qualified paths.
- Calls wrapped in a macro invocation (`assert!(handler.charge())`, `assert_eq!(...)`) were invisible to the call graph — tree-sitter doesn't parse macro arguments into `call_expression` nodes, just a flat token sequence — so every test written the normal way (`assert!(some_call())`) was silently excluded from TESTS counts and caller chains. Fixed with a dedicated macro-token-tree call scanner (`scan_macro_calls`), found by dogfooding the `contracts` fixture and noticing an expected test caller was missing.
- `#[test]`/`#[tokio::test]` detection was reading a `path` field on the `attribute` node that doesn't exist in this tree-sitter-rust grammar (silently always `None`), so no function was ever marked as a test. Fixed to read the `attribute` node's own text instead.

- Phase 3: a deterministic `--change` grammar (`impact_core::change`, `ChangeSpec`/`parse_change`) — `rename`, `remove`, `remove variant <Enum>::<Variant>`, `remove field <Type>.<field>`, `change signature of` — parsed by hand, never NLP, so unparseable input is a hard error with a usage hint rather than a best-effort guess. `impact change "<description>"` CLI subcommand.
- Enum variants are now indexed as their own symbols, and match-arm patterns (`Enum::Variant`) are tracked as `References` edges — needed so `remove variant Enum::Variant` has something precise to resolve against instead of only working at whole-enum granularity.
- `Resolver` (refactored out of the linker for reuse by symbol-mode queries): a three-tier name resolver — exact qualified path, then `Enum::Variant`-shaped "last two segments", then bare short name — shared by call-site linking and `--change`/symbol-mode target resolution.
- `compute_symbol_impact`/`compute_change_impact`: the same blast-radius engine as file-mode, seeded from one resolved symbol instead of a whole file — `rename`, `remove`, and `change signature of` all reduce to this identical computation, differing only in what a human/agent should read into the result.
- 1 new fixture (`enum_variants`) and 4 new behavior tests covering variant removal across files (including through a `format!` macro argument), all three symbol-level change kinds agreeing on the same target, and both error paths (unparseable grammar, unresolved target).

### Fixed

- `Resolver` (and its linker predecessor) only considered `Function`/`Field`/`Contract` nodes, so a `--change` target naming a type directly (`remove PaymentService`, or `remove field`'s type-path fallback) always failed to resolve even when the type was indexed. Now every non-`Module` node kind is a valid resolution target.

- Phase 4: MCP stdio server (`impact mcp`), hand-rolled line-delimited JSON-RPC 2.0 (no MCP SDK dependency, matching the `palace-rs` sibling project's own server) exposing `impact_index`/`impact_file`/`impact_change` as tools with full JSON Schema. `impact index --force` wipes and fully re-indexes a project (`Cache::clear`), for when the cache itself might be stale in a way content hashes can't detect (e.g. after upgrading `impact`).
- `impact-cli`'s three subcommands and the three MCP tools now share one computation layer (`ops.rs`) instead of each reimplementing indexing/querying/change-resolution — the CLI and MCP surfaces render the same `IndexStats`/`ImpactReport` values, never independently recomputed.
- 4 new behavior tests driving `impact mcp` over its real stdio protocol: `initialize`/`tools/list` describe the server, an unknown method is a proper JSON-RPC error, all three tools produce results matching the CLI's already-verified output for the same fixture, and a tool-level failure (bad grammar) comes back inside the tool result rather than as a protocol error.

- Phase 5: cross-project workspace matching (`impact_core::workspace`). A `workspace.toml` registers sibling projects (each with an optional `cache_dir` override); `--workspace`/`workspace_path` on `query`/`change` and the `impact_file`/`impact_change` MCP tools extends the local report with which other registered projects share an API route/event/table identity, tiered `Declared` (a `[[links]]` entry names the exact contract) / `Strong` (a link relates the two projects generally) / `Weak` (identity match only, nothing declared) — deterministic given the same workspace and indexed graphs, never a fuzzy score. A sibling project with no cache yet is silently skipped, not an error.
- 3 new fixtures (`workspace_backend`/`workspace_web`/`workspace_reporting`, sharing event contracts by name) and 4 new behavior tests: all three confidence tiers from one workspace, an unindexed sibling being skipped rather than erroring, and the MCP `workspace_path` plumbing reaching the same matcher the CLI tests already verified in full.

- Phase 6: `impact-lang-ts`, a second `LanguageAdapter` (TypeScript — functions, classes, methods, cross-file call resolution) written specifically to prove the adapter boundary: nothing in `impact-core` changed to support it. Registered alongside `RustAdapter` in the same `Indexer`, so a project mixing both languages gets both indexed, linked, and queried through identical machinery. Deliberately scoped down from the Rust adapter — no contract detection (an honest empty result, not a stub) and no test-attribute detection (JS/TS test conventions vary too much to guess).
- 1 new fixture (`mixed_lang`, one Rust file plus a 3-file TypeScript call chain including a class method) and 2 new behavior tests: cross-file TypeScript call resolution through the same DIRECT/INDIRECT engine Rust uses, and confirmation that the Rust file in the same project is indexed by `RustAdapter` unaffected by `TsAdapter` also being registered.

All seven phases of the original plan are now complete: scaffolding, the call-graph engine, contract detectors (API/EVENTS/DATABASE/TESTS), the `--change` grammar, the MCP server, cross-project workspace matching, and a second language adapter proving the core is actually language-agnostic. 21 behavior tests across 11 fixtures, all five quality gates green throughout.

- `README.md`: what the tool does, how it works (structural resolution, over-report philosophy), CLI/MCP/`--change`-grammar reference, `impact.toml`/`workspace.toml` config shapes, architecture table, development gates.
- `LICENSE` (MIT) — `Cargo.toml` already declared `license = "MIT"` with no accompanying file.
- Per-crate `description`/`keywords`/`categories`/`homepage` metadata on all four crates.

- Multi-language expansion, phase 1: React and React Native support in `impact-lang-ts` (not a new crate — both are TypeScript/JavaScript with JSX). `TsAdapter` now claims `.tsx`/`.jsx`/`.js`/`.mjs` alongside `.ts`, selecting `tree-sitter-typescript`'s TSX grammar per file (verified empirically to parse plain JSX-containing JavaScript with zero errors and identical node kinds to what the adapter already handled) except for `.ts` itself, which keeps the plain TypeScript grammar because it and TSX genuinely disagree on `<Foo>bar` legacy type-assertion syntax. No new extraction logic needed.
- 2 new behavior tests: cross-file call resolution through a JSX expression in both a `.tsx` function component and a plain `.jsx` file (no TypeScript syntax at all), and updated symbol/file counts on the existing mixed-language test.

- Multi-language expansion, phase 2: `impact-lang-python`, a new `LanguageAdapter` (`tree-sitter-python`) mirroring `impact-lang-ts`'s structure and scope boundaries — functions, classes, methods, cross-file call resolution, no contract detection. Unlike TypeScript, this adapter *does* detect tests: pytest's real discovery rule ("any function/method name starting with `test`") is unambiguous and framework-independent, unlike JS/TS's fragmented Jest/Vitest/Mocha conventions, so `is_test` is wired up here.
- 1 new fixture (`python_lang`, a 4-file chain including a class method and a pytest-style test in its own file) and 1 new behavior test covering cross-file call resolution and test detection together.

- Multi-language expansion, phase 3: `impact-lang-go`, a new `LanguageAdapter` (`tree-sitter-go`). Structurally different from every other adapter: Go methods are top-level declarations carrying a receiver (`func (t T) Method()`), not nested inside a class/impl body, so the qualified path is built from the receiver's type name directly rather than via recursion into an enclosing block. Detects tests via the standard-library `go test` convention (`TestXxx` in a `_test.go` file) — unambiguous, not a third-party framework choice.
- 1 new fixture (`go_lang`, a 4-file same-package chain including an unqualified same-package call, a receiver method, and a `_test.go` file) and 1 new behavior test.

- Multi-language expansion, phase 4: `impact-lang-kotlin` (Android — Java explicitly out of scope), a new `LanguageAdapter` (`tree-sitter-kotlin-ng`). This grammar doesn't field-name a function's/class's body at all (`child_by_field_name("body")` returns `None`); found via a real parse-tree dump before writing any extraction code, so the adapter finds `function_body`/`class_body` by child *kind* instead. Detects tests via the `@Test` annotation (JUnit, matching both JUnit4's bare `@Test` and JUnit5's fully-qualified form by extracting the rightmost path segment) — unambiguous and near-universal on Android, unlike TypeScript's fragmented test-framework situation.
- 1 new fixture (`kotlin_lang`, a 4-file chain including a class method and a `@Test`-annotated JUnit test) and 1 new behavior test.

- Multi-language expansion, phase 5 (final): `impact-lang-swift`, a new `LanguageAdapter` (`tree-sitter-swift`) — sequenced last since it's the only community-maintained (not tree-sitter-org) grammar of the six languages now supported, and turned up the sharpest API surprise: `call_expression` has no `function`/`arguments` fields at all, so the callee is purely positional (`child(0)`), matching the same defensive choice `impact-lang-kotlin` already needed. Detects tests via XCTest's real convention — a `test`-prefixed method inside a class that inherits `XCTestCase` (checked via its `inheritance_specifier` children), not just any `test`-prefixed method anywhere.
- 1 new fixture (`swift_lang`, a 4-file chain including a class method and an `XCTestCase` subclass whose test calls through a nested `XCTAssertTrue(...)` wrapper, proving nested-call resolution) and 1 new behavior test.

All five phases of the multi-language expansion are now complete: React/React Native (via `impact-lang-ts`), Python, Go, Kotlin (Android), and Swift. `impact` now indexes, links, and queries seven languages (Rust, TypeScript/TSX, JavaScript/JSX, Python, Go, Kotlin, Swift) through one unchanged core — every new adapter added zero lines to `impact-core`, the strongest evidence yet that the language-agnostic architecture holds. 42 behavior tests across 15 fixtures, all five quality gates green throughout. Every new adapter's field names and node shapes were verified against real tree-sitter parse-tree dumps before any extraction code was written, catching real API surprises (Kotlin's and Swift's ungated `body`/`function` fields) before they became silent bugs rather than after.
