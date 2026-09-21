use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_value_ref_array_and_args")
}

fn index(cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).unwrap()
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

/// Regression fixture for https://github.com/AncientiCe/impact-rs/issues/5: a saga/generator
/// function handed to something else as a *value*, rather than called directly, produced no
/// edge at all — `impact_file` reported zero direct callers and zero tests for a symbol with
/// exactly one production caller and one spec file.
///
/// `rootSaga.ts` imports `mySaga` and puts it into an array literal alongside a sibling saga
/// (`const sagas = [otherSaga, mySaga]`), later started via `sagas.map(fn => spawn(fn))` — the
/// reference lives in the array, never in a call expression naming `mySaga` itself.
/// `saga.spec.ts` imports `mySaga` and passes it directly as the sole argument to a
/// test-runner-style call (`testSaga(mySaga)`) inside an `it(...)` block, which is expected to
/// invoke it internally rather than call it by name.
///
/// Querying `saga.ts` should surface both enclosing scopes as DIRECT dependents via the
/// existing `References` edge kind — the array's bare-identifier elements and the call's
/// bare-identifier argument are both value references, the same shape as the
/// jsx_attribute/object-literal cases in `ts_jsx_registry_refs`, just carried by an array and
/// a plain positional argument instead of a prop or object value.
#[test]
fn array_registry_and_call_argument_value_references_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 3);

    let report = query(cache_dir.path(), "saga.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "rootSaga::rootSaga", "file": "rootSaga.ts", "line": 7, "confidence": "Exact"},
            {"path": "saga.spec::runs the saga", "file": "saga.spec.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
