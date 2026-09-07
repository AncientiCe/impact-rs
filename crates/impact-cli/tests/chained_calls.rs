//! A chained call keeps its receiver in the callee, not in the arguments:
//! `build_client().send()` calls `build_client` too. Every adapter walked only the
//! arguments, so that inner call was dropped and the function providing the receiver
//! vanished from its own blast radius.
//!
//! Each fixture puts the receiver-providing function in a file of its own, so a dropped
//! inner call shows up as an empty report rather than being masked by the method call
//! that follows it. The TypeScript case is covered by `ts_imports`, whose spec fixture
//! only resolves because `expect(value)` in `expect(value).toEqual(x)` is now walked.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn index(project: &Path, cache_dir: &Path) {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", project.to_str().unwrap(), "--json"])
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

fn direct_paths(project: &Path, cache_dir: &Path, file: &str) -> Vec<String> {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", file, "--json"])
        .arg("--project")
        .arg(project)
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect()
}

fn assert_receiver_call_is_found(fixture_name: &str, queried: &str, expected: &str) {
    let project = fixture(fixture_name);
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct_paths(&project, cache_dir.path(), queried),
        vec![expected.to_string()],
        "{fixture_name}: the call providing a chained call's receiver must not be dropped"
    );
}

#[test]
fn rust_chained_call_receiver_is_a_dependency() {
    assert_receiver_call_is_found("rust_chained", "src/factory.rs", "runner::run");
}

#[test]
fn python_chained_call_receiver_is_a_dependency() {
    assert_receiver_call_is_found("python_chained", "factory.py", "runner::run");
}

#[test]
fn go_chained_call_receiver_is_a_dependency() {
    assert_receiver_call_is_found("go_chained", "a/factory.go", "b::runner::Run");
}
