use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_anonymous_default_export")
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

/// A default-exported function with no name (`export default (options) => async (dispatch)
/// => { ... }`, the shape of a redux thunk factory) was not a symbol, so the calls inside it
/// were attributed to the file's module scope. A module scope is "run" by every file that
/// loads the module, so one call there made the whole chain of importers a dependent: in a
/// real app a single such file put about nine hundred entries into one report.
///
/// `thunk.ts` default-exports the anonymous function and calls `helper` from `util.ts`.
/// `consumer.ts` imports it by default and calls it; `app.ts` only imports `consumer.ts`.
/// The anonymous function is a symbol of its own (`default`), the caller of `helper`, and
/// reached by the default import. Neither the module scopes of `thunk.ts` and `consumer.ts`
/// nor the file that merely imports `consumer.ts` depend on `helper`.
#[test]
fn calls_in_an_anonymous_default_export_belong_to_it_not_to_the_module() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);

    let report = query(cache_dir.path(), "util.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "thunk::default", "file": "thunk.ts", "line": 3, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "consumer::run", "file": "consumer.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
