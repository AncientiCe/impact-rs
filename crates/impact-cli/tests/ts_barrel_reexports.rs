use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_barrel_reexports")
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
/// A barrel re-exports another module's symbols (`export { print } from '../lib/printer'`,
/// `export * from ...`, `export * as printing from ...`). The linker matched an import to a
/// symbol only when the imported module's path ran through the symbol's own, which a barrel
/// beside its origin satisfies by accident and any other one doesn't: a caller importing
/// through a barrel in another directory, through `export *`, or through a chain of
/// barrels was missing from the origin's blast radius. `e.ts` imports through two barrels
/// that re-export each other and reach nothing: resolving it has to stop, not loop.
#[test]
fn callers_importing_through_barrels_reach_the_origin() {
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
            ("a::viaNamed", "Exact"),
            ("b::viaStar", "Exact"),
            ("c::viaNamespace", "Exact"),
            ("d::viaChain", "Exact"),
        ]
    );
}
