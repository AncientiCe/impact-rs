use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_fn_value_refs")
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
    serde_json::from_slice(&output.stdout).expect("change --json output should be valid JSON")
}

fn direct_paths(report: &Value) -> Vec<String> {
    let mut paths: Vec<String> = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect();
    paths.sort();
    paths
}

/// A function handed to `map` by name rather than called is still a caller for a
/// signature change: adding a parameter breaks `.map(render_row)` just as surely as it
/// breaks `render_row(id)`. Every shape here passes `format::render_row` as a value —
/// by imported bare name, by `let` binding, through `super::super::`, and inside a
/// macro's arguments — and each has to show up next to the ordinary direct callers.
#[test]
fn functions_passed_as_values_are_signature_change_callers() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = change(cache_dir.path(), "change signature of format::render_row");
    assert_eq!(
        direct_paths(&report),
        vec![
            "api::legacy::bare_in_macro",
            "api::legacy::legacy_rows",
            "api::rows::Service::rows",
            "api::rows::filter_mapped",
            "api::rows::get_rows",
            "api::rows::in_macro",
            "api::rows::via_super",
            "builders::build_one",
            "format::tests::renders_a_row",
            "handlers::bound_then_used",
            "handlers::list_rows",
            "handlers::list_rows_imported",
            "handlers::print_rows",
        ]
    );
}

/// A parameter that merely shares the function's name, in a file that never imports
/// it, is not a reference to it.
#[test]
fn a_same_named_local_is_not_a_value_reference() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = change(cache_dir.path(), "change signature of format::render_row");
    assert!(
        !direct_paths(&report).contains(&"builders::shadowed".to_string()),
        "a parameter named render_row must not be reported, got: {:?}",
        direct_paths(&report)
    );

    let unused = change(
        cache_dir.path(),
        "change signature of format::unused_helper",
    );
    assert!(
        direct_paths(&unused).is_empty(),
        "nothing references unused_helper, got: {:?}",
        direct_paths(&unused)
    );
}
