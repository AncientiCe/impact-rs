use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_workspace_crates")
}

fn run_index(project: &Path, cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).expect("index --json output should be valid JSON")
}

fn query(project: &Path, cache_dir: &Path, file: &str) -> Value {
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
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

fn direct_paths(report: &Value) -> Vec<String> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect()
}

fn direct_files(report: &Value) -> Vec<String> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// `rust_workspace_crates` is a two-crate Cargo workspace: `proto` declares
/// `ClientMessage` (a struct variant, a tuple variant and a unit variant) and `server`
/// imports it with `use proto::ClientMessage`. Every way `server` touches the enum is a
/// direct dependent of `proto/src/lib.rs`: a match whose only named arm is a struct
/// variant (`route`), an `if let` on a unit variant (`is_leave`), a struct-variant
/// construction (`join`), a unit-variant value (`leave`) and a tuple-variant call
/// (`ping`). The crate's integration test builds a unit variant, so it is affected too.
#[test]
fn enum_uses_in_another_workspace_crate_are_direct_dependents() {
    let cache = tempfile::tempdir().unwrap();
    run_index(&fixture_path(), cache.path());
    let report = query(&fixture_path(), cache.path(), "crates/proto/src/lib.rs");
    let direct = direct_paths(&report);

    for name in ["route", "is_leave", "join", "leave", "ping"] {
        assert!(
            direct
                .iter()
                .any(|p| p.ends_with(&format!("::{name}")) && p.contains("server")),
            "expected server's `{name}` among direct dependents, got {direct:?}"
        );
    }
    assert!(
        report["tests"].as_u64().unwrap() >= 1,
        "expected the server integration test to be affected, got {report}"
    );
}

/// `web/client.js` calls bare `Leave()` and `Join()`, names it never declares. The only
/// symbols with those names are `ClientMessage`'s Rust variants, and a JavaScript call
/// cannot reach a Rust enum variant, so the JS functions must not show up as dependents
/// of the Rust file.
#[test]
fn a_javascript_call_never_resolves_to_a_rust_symbol() {
    let cache = tempfile::tempdir().unwrap();
    run_index(&fixture_path(), cache.path());
    let report = query(&fixture_path(), cache.path(), "crates/proto/src/lib.rs");

    let files = direct_files(&report);
    assert!(
        files.iter().all(|f| !f.starts_with("web/")),
        "JavaScript callers must not depend on a Rust file, got {files:?}"
    );
}

/// An untracked `node_modules/` (not listed in any `.gitignore`, as in a project that
/// isn't a git repository at all) holds third-party code, not the project's own. A copy
/// of the fixture with a minified bundle dropped into `node_modules/` must index exactly
/// the same files as the fixture itself, and nothing in the bundle may appear in a report.
#[test]
fn node_modules_is_never_indexed() {
    let plain_cache = tempfile::tempdir().unwrap();
    let plain = run_index(&fixture_path(), plain_cache.path());

    let project = tempfile::tempdir().unwrap();
    copy_dir(&fixture_path(), project.path());
    let bundle_dir = project.path().join("node_modules/axios/dist");
    std::fs::create_dir_all(&bundle_dir).unwrap();
    std::fs::write(
        bundle_dir.join("axios.min.js"),
        "function freezeMethods(e){return e.Leave(e)}function t(n){return Join(n)}\n",
    )
    .unwrap();

    let cache = tempfile::tempdir().unwrap();
    let with_bundle = run_index(project.path(), cache.path());
    assert_eq!(with_bundle["files_indexed"], plain["files_indexed"]);
    assert_eq!(with_bundle["symbols_indexed"], plain["symbols_indexed"]);

    let report = query(project.path(), cache.path(), "crates/proto/src/lib.rs");
    let files = direct_files(&report);
    assert!(
        files.iter().all(|f| !f.starts_with("node_modules/")),
        "node_modules must not contribute dependents, got {files:?}"
    );
}
