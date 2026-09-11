use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_nested_helper_functions")
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

/// Regression fixture for a mismatch between `impact-lang-ts`'s two AST passes:
/// `collect_refs` (call extraction) fully recurses into every function body and, for a
/// named local helper (`const helper = () => {...}` or `function helper() {...}` declared
/// *inside* another function — a common pattern for a scoped async helper closure),
/// attributes calls made from within it to a flat `file_prefix::helper` qualified path.
/// But `walk` (symbol extraction) never descended into a function body at all, so that
/// same `helper` was never registered as a real symbol — the linker's `from` lookup for
/// the ref then failed and the whole call was silently dropped, regardless of whether the
/// callee itself resolved. This was invisible until it hid a real blast-radius miss: a
/// call made only from inside such a nested helper had zero chance of ever showing up as
/// a dependent, no matter how correctly the callee resolved.
///
/// `target.ts` exports `doWork`; `consumer.ts`'s top-level `outer` declares a nested
/// `const helper = () => { doWork() }` and calls it. Querying `target.ts` should surface
/// `consumer::helper` (not `consumer::outer`) as the DIRECT caller, since that's where the
/// call site textually lives — and `consumer::outer` as an INDIRECT caller (`Probable`,
/// since `helper()` is a bare call with no import/type evidence tying it — this project's
/// resolver never marks a bare name `Exact`), since it calls `helper`.
#[test]
fn nested_named_helper_functions_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 2);

    let report = query(cache_dir.path(), "target.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "consumer::helper", "file": "consumer.ts", "line": 4, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "consumer::outer", "file": "consumer.ts", "line": 3, "confidence": "Probable"},
        ])
    );
}
