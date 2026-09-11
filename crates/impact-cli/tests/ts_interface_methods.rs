use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_interface_methods")
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

/// Regression fixture for a blind spot in `impact-lang-ts`'s symbol extraction: a TS
/// `interface`'s members (`method_signature`) were never indexed as symbols — only
/// `class_declaration` methods, `function_declaration`s, and function-valued variables
/// were. Any value whose shape is described by an `interface` rather than implemented as
/// a class (a common shape for hand-typed service objects, ports, or bindings to code
/// this adapter can't see into) was therefore invisible on the receiving end of a method
/// call: the callee name never existed in the graph under that module, so the resolver's
/// module+name lookup came up empty and the edge was silently dropped — indistinguishable
/// from an import that resolved outside the project entirely.
///
/// `service.ts` declares `interface Greeter { greet(...) }` and default-exports a value
/// typed by it; `consumer.ts` imports that value and calls `.greet(...)` on it. Querying
/// `service.ts` should surface `consumer.ts` as a DIRECT caller through the interface
/// method, the same way a class method already would.
#[test]
fn interface_method_callers_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 2);

    let report = query(cache_dir.path(), "service.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "consumer::run", "file": "consumer.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
