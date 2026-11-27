use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_reexported_type")
}

fn index(cache_dir: &Path) {
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
}

fn change(description: &str) -> Value {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["change", description, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("--json output should be valid JSON")
}

fn direct_paths(report: &Value) -> Vec<String> {
    let mut paths: Vec<String> = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .filter(|path| !path.ends_with("::<module>"))
        .collect();
    paths.sort();
    paths
}

/// `store_core::Store::open(..)` from another crate, with no `use`, names `Store` through
/// the crate root's `pub use store::*;` — the type itself lives in `store.rs`, one module
/// below the root the path spells out (GitHub issue #25).
#[test]
fn fully_qualified_call_through_a_glob_reexport_is_a_direct_caller() {
    let report = change("change signature of Store::open");
    let direct = direct_paths(&report);
    assert!(
        direct.iter().any(|p| p == "apps::server::src::main"),
        "expected the server's main among {direct:?}"
    );
}

/// The same call shape through a named re-export (`pub use ledger::Ledger;`).
#[test]
fn fully_qualified_call_through_a_named_reexport_is_a_direct_caller() {
    let report = change("change signature of Ledger::new");
    let direct = direct_paths(&report);
    assert!(
        direct.iter().any(|p| p == "apps::server::src::main"),
        "expected the server's main among {direct:?}"
    );
}

/// An associated function handed on as a value (`.map(store_core::Store::open)`) is
/// resolved the same way as a call to it.
#[test]
fn fully_qualified_path_passed_as_a_value_through_a_reexport_is_a_direct_caller() {
    let report = change("change signature of Store::open");
    let direct = direct_paths(&report);
    assert!(
        direct
            .iter()
            .any(|p| p == "apps::server::src::spare_stores"),
        "expected spare_stores among {direct:?}"
    );
}

/// The type in the path still narrows the match: `Cache::open` shares the method name
/// and the crate, but nothing in the server names `Cache`.
#[test]
fn reexported_type_path_does_not_reach_a_same_named_method_on_another_type() {
    let report = change("change signature of Cache::open");
    let direct = direct_paths(&report);
    assert!(
        !direct.iter().any(|p| p.starts_with("apps::server")),
        "no server function calls Cache::open, got {direct:?}"
    );
}
