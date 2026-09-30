use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_barrel_default")
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
/// A barrel passes a default export through with `export { default } from './printer'` (or
/// `export { print as default } from ...`, which makes a named export the barrel's default),
/// and a caller reaches it with `import Render from './lib'`. The caller's name is its own
/// choice, so the link has to go through the barrel's default to whichever symbol that is.
/// `export *` never re-exports a default, so `c.ts` reaches nothing.
#[test]
fn default_imports_through_a_barrel_reach_the_origin() {
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
        [
            ("a::viaDefault", "Exact"),
            ("b::viaNamedAsDefault", "Exact"),
            ("d::viaCrossDir", "Exact"),
        ]
    );
}
