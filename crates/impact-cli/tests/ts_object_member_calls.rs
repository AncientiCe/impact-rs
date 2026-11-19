use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_object_member_calls")
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

/// `store.get()` on an imported object constant calls a property of `store`, and the
/// object's properties aren't indexed symbols of their own — only `store` is. Resolving
/// the call as `get` inside `store`'s module found nothing, so a module whose dependents
/// all reached it through member calls reported none at all.
#[test]
fn member_calls_on_an_imported_object_constant_are_its_dependents() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/store.ts"]);
    let direct = direct_paths(&report);
    assert!(
        direct.iter().any(|p| p.ends_with("reader::readValue")),
        "a `store.get()` caller should be a dependent: {direct:?}"
    );
    assert!(
        direct.iter().any(|p| p.ends_with("Bumper::bump")),
        "a `store.update()` caller should be a dependent: {direct:?}"
    );
    assert!(
        direct.iter().any(|p| p.contains("store.test")),
        "a spec calling `store.get()` should be a dependent: {direct:?}"
    );
    assert!(
        !direct.iter().any(|p| p.contains("unrelated")),
        "an unrelated `cache.get()` must not link to the store: {direct:?}"
    );
}
