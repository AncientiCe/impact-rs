use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_proxy_trap_builtins")
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

/// A `Proxy` handler's `get`/`apply` methods are traps the runtime invokes, not functions
/// anything calls by name. Indexed as project symbols, every untyped `index.get(name)` or
/// `handler.apply(...)` in the project resolved to them, so tooling scripts that never
/// import the helper showed up as its callers.
#[test]
fn proxy_trap_methods_do_not_catch_unrelated_member_calls() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/bindAll.ts"]);
    assert_eq!(
        direct_paths(&report),
        [
            "bindAll.test::bindAll::keeps methods bound".to_string(),
            "panel::renderPanel".to_string(),
        ]
    );
}

/// `fn.apply(...)`/`fn.call(...)` is `Function.prototype`, and `Reflect.ownKeys(...)` is a
/// builtin namespace: neither reaches a project function that happens to share the name.
#[test]
fn builtin_apply_call_and_reflect_do_not_link_to_same_named_project_functions() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let apply = run(
        cache_dir.path(),
        &["change", "change signature of patch::apply"],
    );
    assert_eq!(direct_paths(&apply), ["editor::save".to_string()]);

    let own_keys = run(
        cache_dir.path(),
        &["change", "change signature of patch::ownKeys"],
    );
    assert_eq!(direct_paths(&own_keys), ["editor::save".to_string()]);
}
