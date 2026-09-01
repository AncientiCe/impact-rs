# Agent Rules

When working on this codebase, follow these rules on every task. Binding for every agent (Claude Code, Cursor, Codex, or otherwise) and every human. The architecture this project follows lives in the plan agreed with the user (kept outside this repo per rule 3, in Claude Code's plan store). These rules govern *how* that plan gets built.

---

## 1. Test-Driven Development (TDD) — Behavior Tests Only

- **Write behavioral tests first.** Define the expected behavior in tests before implementing.
- **See them fail.** Run the test suite and confirm the new tests fail (red) — a test that already passes before the implementation exists is not a valid starting point.
- **Implement.** Write the minimum code to make the tests pass.
- **See them pass.** Run the test suite and confirm all tests pass (green).

Do not implement behavior without a failing test that defines it.

**"Behavioral" means exercising `impact` from the outside** — a CLI invocation (`assert_cmd` against the `impact` binary) or an MCP JSON-RPC call over stdio — and asserting on the report `impact` actually produces (`ImpactReport` tree-text or JSON) against a checked-in fixture project under `tests/fixtures/<name>/` with hand-verified expected relationships. It does **not** mean unit-testing internal functions.

Forbidden — these are not what this project means by "test":
- Unit tests calling private functions or internal modules (`impact_core::linker::resolve_edge(...)`, raw `graph.nodes[...]`) directly.
- Snapshot tests of internal data structures (raw `SymbolGraph` dumps, intermediate parse trees).
- Tests asserting on intermediate state (edge counts, node kinds) instead of the final report a caller would see.
- `#[cfg(test)] pub` exposure hacks added purely so a test can reach something outside the public CLI/MCP/`impact-core` API surface.

Exception: pure, stateless, non-domain helpers with no behavior of their own worth specifying at the report level (e.g. the `blake3` `NodeId` hash, the `--change` grammar's tokenizer) may get direct unit tests. This is the exception, not the default — when in doubt, write the behavior test.

---

## 2. Quality Gates on Every Task

Before considering a task done, ensure all of the following pass:

- **`cargo fmt --all -- --check`** — code is formatted.
- **`cargo clippy --all-targets --all-features --workspace -- -D warnings`** — no clippy warnings or errors.
- **`cargo audit`** — no known security advisories in dependencies.
- **`cargo test --workspace`** — full behavior-test suite passes.
- **`cargo build --workspace --locked`** — builds reproducibly from the committed `Cargo.lock`, no silent lockfile drift.

Run all five with `make check` (see `Makefile`) — that target is the actual definition of "green," not a subset chosen by judgment call in the moment. Match CI locally with the exact commands above (see `.github/workflows/ci.yml`).

No `--no-verify`, no skipping a gate, no `#[allow(clippy::...)]` without a comment explaining why the lint doesn't apply, no narrowing a test's assertions or deleting a fixture just to turn a red gate green — if a gate is red, the code is wrong, not the test, unless investigation shows the test encoded the wrong expected behavior, in which case fix it deliberately and say so. `cargo audit` findings get triaged (upgrade, or document an accepted risk), never just ignored.

Fix any failure before marking the task complete.

---

## 3. No Plan Markdown Files

- **Do not create `.md` files for plans** (e.g. `PLAN.md`, `TODO.md`, task plans) inside this repo.
- Create markdown only for **documentation** (API, README, runbooks, etc.) when necessary.
- Keep planning in conversation, tickets, or Claude Code's own plan-mode files (outside the repo) — not as standalone plan documents committed here.

---

## 4. Internal API Discipline

- `impact` is a **CLI/MCP tool** shipped as compiled binaries.
- The workspace is split into `impact-core` (the graph model, `LanguageAdapter`/`ContractDetector` traits, `BlastRadiusEngine` — the public engine API), `impact-lang-rust` (the Rust tree-sitter adapter, built against `impact-core`'s public traits), `impact-cli` (the `impact` binary), and `impact-mcp` (the stdio JSON-RPC server). Both `impact-cli` and `impact-mcp` build on `impact-core`'s public API, so keep it clean and minimal.
- Before changing public types, functions, or module structure in `impact-core`: consider `impact-cli` and `impact-mcp` call sites, the SQLite cache schema (`.impact/cache.sqlite`), and every language adapter that depends on the `LanguageAdapter`/`ContractDetector` trait shape.

---

## 5. No Unused Variables or Dead Code

- **No unused variables.** Every declared variable must be used; remove or replace with `_` if intentionally unused.
- **No dead code.** Remove unreachable functions, branches, types, and imports — do not leave them commented out or hidden behind `#[allow(dead_code)]`.
- Treat compiler warnings for unused items as errors: they must be resolved before a task is complete.

---

## 6. No Mocks

- **Do not use mocks** in tests (mock objects, mock servers, mock crates).
- Prefer **real implementations**, tests against real fixture projects (see rule 1), or explicit, minimal test doubles (fakes, stubs) where a real dependency genuinely can't be used.
- Tests must exercise real behavior where practical; avoid substituting dependencies with mocks that hide integration or behavior. Use in-memory SQLite (`:memory:`) for cache/database tests that don't need to verify on-disk persistence.

---

## 7. No Placeholders

- **No placeholders. Ever.** Do not leave `todo!()`, `unimplemented!()`, stub returns, or "coming soon" code in the codebase.
- Deliver **only real implementations**. Every committed code path must do the real work or explicitly fail in a defined way (e.g. return `Err`, not panic with "unimplemented").
- If a feature is not ready, do not merge it; do not merge placeholder code. Ship a phase (per the plan) only when its exit criteria are for-real met.

---

## 8. No Unsafe `unwrap`/`expect`

- **Do not use `unwrap()` or `expect()` in production code.** These can panic and crash the process — including mid-way through indexing a user's real project, or inside a long-running MCP server.
- Handle failures safely using explicit error propagation (`Result`/`?`), recoverable branches, or well-defined fallbacks (e.g. a file that fails to parse becomes a skipped-file warning in the report, not a crash).
- If a value is logically guaranteed, prove it through types/validation rather than runtime panics.
- `unwrap()` and `expect()` are acceptable **only** inside `#[cfg(test)]` test code and examples.

---

## 9. Close Running Instances When Done

- If you start a long-running process for verification (the MCP stdio server, a `cargo watch`, an indexing run against a real project), ensure it is stopped/killed before marking the task complete.
- Avoid leaving stuck processes running after verification; resolve or terminate them so they don't interfere with future tasks.

---

## 10. Keep the Changelog Current

- **Update `CHANGELOG.md` as part of every feature, fix, or behavior change** — not as an afterthought before release.
- Add entries under a top **`## [Unreleased]`** section while work is in progress (create it if missing, above the most recent released version). Do not invent a version number or date for in-progress work.
- Use the Keep a Changelog headings (`### Added`, `### Changed`, `### Fixed`, `### Removed`, `### Deprecated`).
- When a release is tagged, `## [Unreleased]` entries get renamed to `## [x.y.z] - YYYY-MM-DD`.

---

## Definition of Done

A change (a phase increment, a bug fix, anything) is done only when **all** of:
1. It corresponds to a specific exit criterion in the plan (or an explicitly agreed deviation).
2. A behavior test exists per rule 1, was seen failing before the implementation, and passes now.
3. All five gates in rule 2 are green, run via `make check`.
4. Nothing was skipped, allowed, or narrowed to get there.
5. `CHANGELOG.md` reflects the change under `## [Unreleased]`.

"I implemented it" is not done. "The gates are green" is not done on its own either — a green gate suite with no new behavior test just means nothing new was verified. All of the above, together, is done.

---

## Quick Reference

| Rule | Action |
|------|--------|
| TDD (behavior tests) | Fixture + failing CLI/MCP-level test first → see fail → implement → see pass. No internal-function unit tests except pure stateless helpers. |
| Quality gates | `cargo fmt --all -- --check` \| `cargo clippy --all-targets --all-features --workspace -- -D warnings` \| `cargo audit` \| `cargo test --workspace` \| `cargo build --workspace --locked` — all five, via `make check` |
| No plan files | No `.md` for plans in this repo; only real documentation |
| Internal API discipline | `impact-core`'s public API is load-bearing for `impact-cli`, `impact-mcp`, and every language adapter — change it deliberately |
| No dead code | No unused variables, dead code, or `#[allow(dead_code)]` |
| No mocks | Real implementations, fixture-backed tests, or explicit minimal test doubles |
| No placeholders | No `todo!()`/`unimplemented!()`/stubs; ship only real, complete work |
| No unsafe unwrap/expect | Never in production code; explicit `Result` handling instead |
| Close running instances | Stop any long-running verification processes (MCP server, watchers) when done |
| Keep changelog current | `### Added/Changed/Fixed` under `## [Unreleased]` in `CHANGELOG.md` as you build |
