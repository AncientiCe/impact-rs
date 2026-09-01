use std::path::Path;

use assert_cmd::Command;
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

/// Builds the JSON shape a report's `direct`/`indirect` entries now carry: `{path,
/// confidence}` pairs. Every path here resolves unambiguously in its fixture, so `Exact`
/// is the right expected confidence throughout this file.
fn exact(paths: &[&str]) -> Value {
    Value::Array(
        paths
            .iter()
            .map(|p| serde_json::json!({"path": p, "confidence": "Exact"}))
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
        exact(&["payment::controller::PaymentController::handle"])
    );
    assert_eq!(
        report["indirect"],
        exact(&["order::OrderService::checkout"])
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
