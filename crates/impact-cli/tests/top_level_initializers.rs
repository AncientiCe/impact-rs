//! A call in a top-level binding's initializer — `var Startup = compute()` and its
//! equivalents — ran at module load and was still a real dependency, but had no enclosing
//! function to be attributed to, so every adapter dropped it. The binding itself is the
//! call site, so it's now indexed as one.
//!
//! Rust is deliberately absent: a `const`/`static` initializer there has to be
//! const-evaluable, so it can't call ordinary functions in the first place.

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

fn assert_binding_is_a_dependent(fixture_name: &str, queried: &str, expected: &str) {
    let project = fixture(fixture_name);
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct_paths(&project, cache_dir.path(), queried),
        vec![expected.to_string()],
        "{fixture_name}: the top-level binding should be reported as the call site"
    );
}

#[test]
fn typescript_top_level_binding_is_a_call_site() {
    assert_binding_is_a_dependent("ts_top_level", "src/util.ts", "startup::STARTUP");
}

#[test]
fn python_module_level_binding_is_a_call_site() {
    assert_binding_is_a_dependent("python_top_level", "util.py", "startup::STARTUP");
}

#[test]
fn go_package_level_var_is_a_call_site() {
    assert_binding_is_a_dependent("go_top_level", "a/util.go", "b::startup::Startup");
}

#[test]
fn kotlin_top_level_property_is_a_call_site() {
    assert_binding_is_a_dependent("kotlin_top_level", "src/a/Util.kt", "b::Startup::STARTUP");
}

#[test]
fn swift_top_level_property_is_a_call_site() {
    assert_binding_is_a_dependent(
        "swift_top_level",
        "Sources/A/Util.swift",
        "Sources::B::Startup::startup",
    );
}
