use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kotlin_lang")
}

fn index(cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).unwrap()
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
    serde_json::from_slice(&output.stdout).unwrap()
}

/// `kotlin_lang` is a 4-file chain, hand-traced from the source: `Util.kt` declares
/// `helper`; `Service.kt` calls it from `process`; `Consumer.kt` declares class `Consumer`
/// with a `run()` method calling `process` (exercises `class_declaration` + nested
/// `function_declaration` extraction — Kotlin reuses one node kind for both free
/// functions and methods); `ProcessTest.kt` declares class `ProcessTest` with a
/// `@Test`-annotated `testProcess()` calling `process` — JUnit's annotation convention,
/// not a naming convention. Qualified paths for the class methods are self-referential
/// (`Consumer::Consumer::run`, `ProcessTest::ProcessTest::testProcess`) because each file
/// is its own module segment (this adapter's documented convention) and both files are
/// named after the class they declare — the same pattern already seen and documented for
/// `impact-lang-ts`'s `Widget.tsx`, not a bug. 6 symbols total: `helper`, `process`,
/// `Consumer`, `Consumer::run`, `ProcessTest`, `ProcessTest::testProcess`.
#[test]
fn kotlin_adapter_resolves_calls_across_files_and_detects_junit_tests() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);
    assert_eq!(stats["symbols_indexed"], 6);

    let report = query(cache_dir.path(), "Util.kt");
    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "Service::process", "file": "Service.kt", "line": 1, "confidence": "Exact"}])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "Consumer::Consumer::run", "file": "Consumer.kt", "line": 2, "confidence": "Exact"},
            {"path": "ProcessTest::ProcessTest::testProcess", "file": "ProcessTest.kt", "line": 4, "confidence": "Exact"},
        ])
    );
    assert_eq!(report["tests"], 1);
}
