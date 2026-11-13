use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_split_impl")
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

fn run(cache_dir: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(args)
        .arg("--json")
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact {args:?} failed: {}",
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

/// `self.authenticate()` in one module's `impl Session` reaches the method another
/// module's `impl Session` defines: same type, same crate.
#[test]
fn self_call_reaches_a_method_in_another_modules_impl_block() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/session_auth.rs"]);
    let direct = direct_paths(&report);
    assert!(
        direct.iter().any(|p| p.ends_with("session::Session::open")),
        "expected Session::open among {direct:?}"
    );
}

/// The cross-module lookup is narrowed by the impl's type: `Device::authenticate` shares
/// the name but not the type, so no `Session` method calls it.
#[test]
fn self_call_does_not_reach_a_same_named_method_on_another_type() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/device.rs"]);
    let direct = direct_paths(&report);
    assert!(
        !direct.iter().any(|p| p.ends_with("Session::open")),
        "Session::open should not call Device::authenticate: {direct:?}"
    );
}

/// A same-file `self.flush()` still resolves the way it always has.
#[test]
fn same_file_self_call_still_resolves() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(
        cache_dir.path(),
        &["change", "change signature of Session::flush"],
    );
    let direct = direct_paths(&report);
    assert!(
        direct
            .iter()
            .any(|p| p.ends_with("session::Session::close")),
        "expected Session::close among {direct:?}"
    );
}
