# Claude Notes

See `AGENTS.md` for the full rules governing this project (TDD behavior tests, quality gates, internal API discipline, no mocks/placeholders/unwrap, changelog discipline). Those rules are binding — read them before making changes.

Development commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo test --workspace
cargo audit
cargo build --workspace --locked
```

Or run all five at once: `make check`.
