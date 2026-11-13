use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_builtin_globals")
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

/// `Date.now()` is the runtime's clock. A file whose own `const now = Date.now()` is
/// indexed was reported as the callee of every other `Date.now()` in the project, so
/// unrelated scripts became its dependents.
#[test]
fn date_now_calls_do_not_link_unrelated_modules() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/Clock.tsx"]);
    assert_eq!(direct_paths(&report), Vec::<String>::new());
}
