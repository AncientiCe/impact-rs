use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_lang")
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
    serde_json::from_slice(&output.stdout).unwrap()
}

/// `python_lang` is a 4-file chain, hand-traced from the source: `util.py` declares
/// `helper`; `service.py` imports and calls it from `process`; `caller.py` imports
/// `process` and calls it from `Consumer.run` (a class method — exercises
/// `class_definition` + nested `function_definition` extraction); `test_service.py`
/// imports `process` and calls it from `test_process`, a pytest-style test function
/// (name starts with `test`) in a different file from what it tests. 5 symbols total:
/// `helper`, `process`, `Consumer`, `Consumer::run`, `test_process`.
#[test]
fn python_adapter_resolves_calls_across_files_and_detects_tests() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);
    assert_eq!(stats["symbols_indexed"], 5);

    let report = query(cache_dir.path(), "util.py");
    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "service::process", "confidence": "Exact"}])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "caller::Consumer::run", "confidence": "Exact"},
            {"path": "test_service::test_process", "confidence": "Exact"},
        ])
    );
    assert_eq!(report["tests"], 1);
}
