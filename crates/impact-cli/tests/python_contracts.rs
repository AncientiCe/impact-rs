use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_contracts")
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

/// `python_contracts` wires (hand-traced from the source): `handlers.py`'s
/// `create_payment_route` is decorated `@app.get("/payments")` (FastAPI/Flask 2.0+ verb
/// alias — no explicit `methods=[...]`) and calls `repo.py`'s `save_payment`;
/// `create_order_route` is decorated `@app.route("/orders", methods=["POST"])` (Flask's
/// original form) and also calls `save_payment`; `legacy_handler` has no decorator at
/// all and is never referenced by either route.
///
/// Querying `repo.py` should surface both decorated functions as DIRECT callers, and
/// both routes (verb read from the decorator name for the first, from `methods=[...]`
/// for the second) in `api`.
#[test]
fn fastapi_and_flask_route_decorators_are_detected() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "repo.py");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "handlers::create_order_route", "file": "handlers.py", "line": 10, "confidence": "Exact"},
            {"path": "handlers::create_payment_route", "file": "handlers.py", "line": 5, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        report["api"],
        serde_json::json!(["GET /payments", "POST /orders"])
    );
}
