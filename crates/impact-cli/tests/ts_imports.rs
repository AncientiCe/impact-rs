use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_imports")
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

fn query(cache_dir: &Path, file: &str, min_confidence: Option<&str>) -> Value {
    let mut cmd = Command::cargo_bin("impact").unwrap();
    cmd.args(["query", file, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir);
    if let Some(min) = min_confidence {
        cmd.args(["--min-confidence", min]);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

fn direct(report: &Value) -> Vec<(String, String)> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// The defect this fixture is built around, reported from a real React Native repo: a
/// query for `store/sync/utils.js` came back with dozens of `Exact` dependents from
/// unrelated corners of the codebase, none of which imported it — every module named
/// `utils.js` had been folded together, and every same-named method counted as a call.
///
/// `Fiscal::send` calls `this.prune(...)`, its own method, and must not appear at all.
/// `airPrint` calls a `prune` it never imported, so it stays visible (missing a real
/// caller is the worse failure) but only as `heuristic`.
#[test]
fn dependents_come_from_imports_not_from_matching_names() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/store/sync/utils.js", None);

    assert_eq!(
        direct(&report),
        vec![
            ("printer::airPrint".to_string(), "Heuristic".to_string()),
            (
                "store::sync::__tests__::utils.spec::sync utils".to_string(),
                "Exact".to_string()
            ),
            (
                "store::sync::__tests__::utils.spec::sync utils::camelizes an order".to_string(),
                "Exact".to_string()
            ),
            (
                "store::sync::actions::syncCart".to_string(),
                "Exact".to_string()
            ),
            ("widget::Widget".to_string(), "Exact".to_string()),
        ]
    );
}

/// `min_confidence: exact` was no protection at all before this: the bogus matches were
/// tagged `Exact` too. Now it leaves exactly the callers that really do import the
/// module — a namespace import (`import * as helpers`) counts as much as a named one.
#[test]
fn min_confidence_exact_leaves_only_real_importers() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/store/sync/utils.js", Some("exact"));

    assert_eq!(
        direct(&report),
        vec![
            (
                "store::sync::__tests__::utils.spec::sync utils".to_string(),
                "Exact".to_string()
            ),
            (
                "store::sync::__tests__::utils.spec::sync utils::camelizes an order".to_string(),
                "Exact".to_string()
            ),
            (
                "store::sync::actions::syncCart".to_string(),
                "Exact".to_string()
            ),
            ("widget::Widget".to_string(), "Exact".to_string()),
        ]
    );
}

/// The same fix seen from the other end: a class calling its own method resolves to that
/// method, so querying the file that declares it finds the caller inside it — and
/// querying an unrelated module with the same method name finds nothing.
#[test]
fn a_class_method_call_resolves_within_its_own_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/printer.ts", None);
    assert!(
        report["direct"].as_array().unwrap().is_empty(),
        "nothing calls airPrint, got: {:?}",
        report["direct"]
    );
}

/// Every assertion in a Jest/Vitest/Mocha suite lives inside an anonymous callback passed
/// to `it()`, and anonymous callbacks introduced no scope — so the calls inside them were
/// dropped outright. A JS/TS project's tests were therefore invisible to the whole tool:
/// `tests: 0` and an empty `affected_tests` on every query, no matter how many spec files
/// imported the module.
///
/// `describe`/`it` blocks now name their own scope, so a test shows up as a dependent —
/// by its own title, which is what a developer needs to run it — and a `beforeEach` body
/// counts toward the block that encloses it.
#[test]
fn test_blocks_are_dependents_of_what_they_import() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/store/sync/utils.js", Some("exact"));

    let names: Vec<String> = report["affected_tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "store::sync::__tests__::utils.spec::sync utils".to_string(),
            "store::sync::__tests__::utils.spec::sync utils::camelizes an order".to_string(),
        ]
    );
    assert_eq!(report["tests"], 2);
}
