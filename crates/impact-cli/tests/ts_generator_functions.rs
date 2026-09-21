use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_generator_functions")
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

/// Regression fixture for a blind spot found while verifying the fix for
/// https://github.com/AncientiCe/impact-rs/issues/5 against the real ~2800-file React
/// Native app that reported it: `function* foo() {}` parses as its own distinct tree-sitter
/// node kind (`generator_function_declaration`), never `function_declaration`, so it was
/// completely invisible to this adapter — not indexed as a symbol, not tracked as a
/// function scope, calls inside its body never attributed to it, and nothing could be
/// reported as calling it either. Confirmed directly: indexing a file containing only one
/// generator function declaration reported `symbols_indexed: 0`. This is the entire
/// redux-saga convention (`function* mySaga() {}`), so every saga in that app was
/// completely blind to `impact_file`/`impact_diff` regardless of the array/call-argument
/// value-reference fix issue #5 asked for — that fix alone made no difference on the real
/// app until this was also fixed.
///
/// `worker.ts` default-exports a generator function that calls `helper`; `runner.ts` calls
/// the generator directly. Querying `helper.ts` should surface `worker.ts`'s generator as
/// a DIRECT dependent (proving a call *from inside* a generator's body is attributed to
/// it), and querying `worker.ts` should surface `runner.ts::runWorker` as a DIRECT
/// dependent (proving the generator itself is now a resolvable call target).
#[test]
fn generator_function_declarations_are_indexed_and_scoped() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 3);
    assert_eq!(stats["symbols_indexed"], 3);

    let helper_report = query(cache_dir.path(), "helper.ts");
    assert_eq!(
        helper_report["direct"],
        serde_json::json!([
            {"path": "worker::worker", "file": "worker.ts", "line": 3, "confidence": "Exact"},
        ])
    );

    let worker_report = query(cache_dir.path(), "worker.ts");
    assert_eq!(
        worker_report["direct"],
        serde_json::json!([
            {"path": "runner::runWorker", "file": "runner.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
