.PHONY: fmt fmt-check clippy test audit build check

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features --workspace -- -D warnings

test:
	cargo test --workspace

audit:
	cargo audit

build:
	cargo build --workspace --locked

# The full gate suite — see CLAUDE.md §2. All five must pass for a change to be done.
check: fmt-check clippy test audit build
