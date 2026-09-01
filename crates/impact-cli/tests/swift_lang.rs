use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/swift_lang")
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

/// `swift_lang` is a 4-file chain, hand-traced from the source: `Util.swift` declares
/// `helper`; `Service.swift` calls it from `process`; `Consumer.swift` declares class
/// `Consumer` with a `run()` method calling `process` (exercises `class_declaration` +
/// nested `function_declaration` extraction — Swift reuses one node kind for both free
/// functions and methods); `ProcessTests.swift` declares `ProcessTests: XCTestCase` with
/// `testProcess()` calling `process` *through* `XCTAssertTrue(...)` — proving nested
/// calls resolve (the outer `XCTAssertTrue` call itself doesn't match anything in the
/// project and is silently unresolved, exactly as the linker is documented to handle an
/// external call, while the call it wraps still resolves normally) and that XCTestCase
/// inheritance-based test detection works, not just a bare `test`-prefix naming
/// convention. Qualified paths for the class methods are self-referential
/// (`Consumer::Consumer::run`, `ProcessTests::ProcessTests::testProcess`) for the same
/// reason already documented for `impact-lang-ts`'s `Widget.tsx` and
/// `impact-lang-kotlin`'s `Consumer.kt`/`ProcessTest.kt`: each file is its own module
/// segment, and these files are named after what they declare. 6 symbols total: `helper`,
/// `process`, `Consumer`, `Consumer::run`, `ProcessTests`, `ProcessTests::testProcess`.
#[test]
fn swift_adapter_resolves_calls_across_files_and_detects_xctest_tests() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);
    assert_eq!(stats["symbols_indexed"], 6);

    let report = query(cache_dir.path(), "Util.swift");
    assert_eq!(report["direct"], serde_json::json!(["Service::process"]));
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            "Consumer::Consumer::run",
            "ProcessTests::ProcessTests::testProcess",
        ])
    );
    assert_eq!(report["tests"], 1);
}
