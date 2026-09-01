# Changelog

All notable changes to `impact` are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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
