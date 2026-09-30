use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_barrel_renames")
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
/// A barrel can rename what it passes through: `export { print as emit } from './printer'`
/// and `export { default as renderer } from './printer'`. A caller writes the barrel's name,
/// which the origin has no symbol for, so the link has to map it back to `print` and to the
/// origin's default export.
#[test]
fn renamed_reexports_reach_the_origin() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "lib/printer.ts");
    let callers: Vec<(&str, &str)> = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| !d["path"].as_str().unwrap().ends_with("::<module>"))
        .map(|d| {
            (
                d["path"].as_str().unwrap(),
                d["confidence"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        callers,
        [("a::viaRename", "Exact"), ("b::viaDefault", "Exact")]
    );
}
