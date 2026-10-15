use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_local_renamed_export")
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

fn change(cache_dir: &Path, description: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["change", description, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

fn callers(report: &Value) -> Vec<(String, String)> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| !d["path"].as_str().unwrap().ends_with("::<module>"))
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// `export { scheduled as inner }` binds the public name `inner` to `scheduled`, even
/// though the module also declares an unexported local called `inner`. An importer's
/// `inner()` runs `scheduled`, and so does a barrel that passes `inner` on under
/// another name.
#[test]
fn importers_of_a_renamed_local_export_reach_the_aliased_function() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = change(cache_dir.path(), "change signature of fetcher::scheduled");
    assert_eq!(
        callers(&report),
        [
            ("a::load".to_string(), "Exact".to_string()),
            ("b::viaBarrel".to_string(), "Exact".to_string()),
        ]
    );
}

/// The local `inner` is only called by `scheduled` in its own file; no importer reaches it.
#[test]
fn the_shadowed_local_keeps_only_its_in_file_caller() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = change(cache_dir.path(), "change signature of fetcher::inner");
    assert_eq!(
        callers(&report),
        [("lib::fetcher::scheduled".to_string(), "Exact".to_string())]
    );
}
