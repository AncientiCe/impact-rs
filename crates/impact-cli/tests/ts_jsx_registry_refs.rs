use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_jsx_registry_refs")
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

/// Regression fixture for a blind spot found 2026-09-14 while dogfooding on a real React
/// Native app: `impact_file` on a screen component reached only through a React
/// Navigation-style route registry (`<Stack.Screen component={PrinterEditor} />`) or an
/// object-literal registry (`{ PrinterEditor }`) reported zero consumers, even though the
/// component is real and used in production. `collect_refs` only ever walked
/// `call_expression` nodes — a bare identifier handed somewhere as *data* rather than
/// called produced no edge at all.
///
/// `screen.tsx` exports `PrinterEditor`; `navigator.tsx` imports it and passes it as a
/// JSX attribute's expression value (`component={PrinterEditor}`); `registry.ts` imports
/// it and places it in an object literal both explicitly (`screen: PrinterEditor`) and as
/// shorthand (`PrinterEditor`). Querying `screen.tsx` should surface both enclosing
/// functions as DIRECT dependents via the existing `References` edge kind (already used by
/// `impact-lang-rust`'s enum-variant references, and already unioned into blast-radius
/// traversal) — the same module+name resolution a call already gets, just for a value
/// reference instead of a call.
#[test]
fn jsx_attribute_and_object_literal_value_references_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 3);

    let report = query(cache_dir.path(), "screen.tsx");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "navigator::AppNavigator", "file": "navigator.tsx", "line": 3, "confidence": "Exact"},
            {"path": "registry::buildRegistry", "file": "registry.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
