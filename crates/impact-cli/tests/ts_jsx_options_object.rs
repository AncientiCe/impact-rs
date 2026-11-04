use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_jsx_options_object")
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
/// Regression fixture for issue #16: a shared options object (`export default
/// defaultOptions`, a plain `const` object literal) handed to a navigator as
/// `screenOptions={defaultOptions}` reported no dependents, because a top-level binding
/// whose initializer is an object literal was never indexed — the module's default export
/// had no symbol to flag, so nothing the importing files referenced could resolve to it.
///
/// `options.ts` default-exports the object; `bare.tsx` passes it as a JSX attribute value.
/// `unrelated.tsx` declares its own local `defaultOptions` and must not be reported.
#[test]
fn object_literal_default_export_passed_as_jsx_attribute_is_a_dependency() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 3);

    let report = query(cache_dir.path(), "options.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "bare::BareStack", "file": "bare.tsx", "line": 3, "confidence": "Exact"},
        ])
    );
}
