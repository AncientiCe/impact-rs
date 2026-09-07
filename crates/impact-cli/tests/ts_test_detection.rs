use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_test_detection")
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

/// `ts_test_detection` wires four callers of `service.ts`'s `process`, one per naming
/// convention this adapter's file-based test detection recognizes (or deliberately
/// doesn't): `service.test.ts` (`.test.` in the filename), `service.spec.ts` (`.spec.`),
/// `__tests__/other.ts` (a `__tests__/` directory, filename otherwise unremarkable), and
/// `not_a_test.ts` (none of the above — a same-shaped caller that should NOT be marked a
/// test, proving this isn't just "every caller gets marked").
///
/// The three test files are written the way real Jest/Vitest/Mocha suites are — assertions
/// inside anonymous callbacks passed to `describe`/`it` — and that matters more than it
/// looks. An earlier version of this fixture used `export function testProcess()` in each
/// of them, a shape no real test file has, and that is precisely why this test stayed
/// green while every JS/TS project's tests were invisible to the tool: an anonymous
/// callback introduced no scope, so every call inside one was dropped. A fixture that
/// doesn't look like the real thing can only prove the tool works on things that aren't.
///
/// Each block is reported by its own title, which is what a developer needs to run it.
/// `directory suite` appears in its own right because its `beforeEach` calls `process` —
/// a lifecycle hook isn't a test and gets no scope of its own, so its body belongs to the
/// block enclosing it.
#[test]
fn test_ts_file_naming_conventions_mark_functions_as_tests() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/service.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "__tests__::other::directory suite", "file": "src/__tests__/other.ts", "line": 3, "confidence": "Exact"},
            {"path": "__tests__::other::directory suite::works", "file": "src/__tests__/other.ts", "line": 8, "confidence": "Exact"},
            {"path": "not_a_test::notATest", "file": "src/not_a_test.ts", "line": 3, "confidence": "Exact"},
            {"path": "service.spec::processes in a spec file", "file": "src/service.spec.ts", "line": 4, "confidence": "Exact"},
            {"path": "service.test::service::processes", "file": "src/service.test.ts", "line": 4, "confidence": "Exact"},
        ])
    );
    assert_eq!(report["tests"], 4);

    let affected_test_paths: Vec<&str> = report["affected_tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        affected_test_paths,
        vec![
            "__tests__::other::directory suite",
            "__tests__::other::directory suite::works",
            "service.spec::processes in a spec file",
            "service.test::service::processes",
        ],
        "the non-test caller must stay out of affected_tests"
    );
}
