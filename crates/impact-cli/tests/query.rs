use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_file")
}

fn index(cache_dir: &Path) {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Builds the JSON shape a report's `direct`/`indirect` entries now carry: `{path, file,
/// line, confidence}` objects. Every entry here resolves unambiguously in its fixture, so
/// `Exact` is the right expected confidence throughout this file.
fn exact(entries: &[(&str, &str, u64)]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|(path, file, line)| {
                serde_json::json!({"path": path, "file": file, "line": line, "confidence": "Exact"})
            })
            .collect(),
    )
}

fn query(cache_dir: &Path, file: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", file, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

/// `multi_file` wires `OrderService::checkout` -> `PaymentController::handle` ->
/// `PaymentService::charge`, each in its own file. Querying `payment/service.rs` (which
/// declares `charge`) should surface `handle` as its one direct caller and `checkout` as
/// a two-hop indirect caller — hand-traced from the fixture source, not from the
/// implementation.
#[test]
fn reports_direct_and_indirect_callers_across_files() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/payment/service.rs");

    assert_eq!(
        report["direct"],
        exact(&[(
            "payment::controller::PaymentController::handle",
            "src/payment/controller.rs",
            8
        )])
    );
    assert_eq!(
        report["indirect"],
        exact(&[("order::OrderService::checkout", "src/order.rs", 8)])
    );
}

/// Querying the file that's already the deepest caller (`order.rs`) should report no
/// callers at all — nothing in the fixture calls `OrderService::checkout`.
#[test]
fn leaf_caller_file_has_no_impact() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/order.rs");

    assert_eq!(report["direct"], serde_json::json!([]));
    assert_eq!(report["indirect"], serde_json::json!([]));
}

/// By default, an INDIRECT entry's JSON has no `via` key at all (it's skipped when
/// empty) — `--explain` isn't needed to affect the *shape* of the default report, just
/// to populate the chain.
#[test]
fn without_explain_indirect_entries_have_no_via_field() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/payment/service.rs");

    assert_eq!(report["indirect"][0].get("via"), None);
}

/// `--explain` should populate `checkout`'s `via` chain with the one DIRECT dependent
/// (`handle`) sitting between it and the queried seed — the same chain hand-traced in
/// `reports_direct_and_indirect_callers_across_files`'s doc comment — and the tree-text
/// rendering should print it as an indented `via` line under the entry.
#[test]
fn explain_flag_populates_indirect_via_chain() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/payment/service.rs", "--explain", "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(
        report["indirect"][0]["via"],
        serde_json::json!(["payment::controller::PaymentController::handle"])
    );

    let text_output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/payment/service.rs", "--explain"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(text_output.stdout).unwrap();
    assert!(
        contains("via payment::controller::PaymentController::handle").eval(&stdout),
        "expected a via line under the INDIRECT entry, got:\n{stdout}"
    );
}
