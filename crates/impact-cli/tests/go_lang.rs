use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_lang")
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

/// `go_lang` is a 4-file, same-package chain, hand-traced from the source: `util.go`
/// declares `Helper`; `service.go` calls it (unqualified — same-package calls in real Go
/// need no qualification, and it resolves here via the linker's short-name fallback tier)
/// from `Process`; `caller.go` calls `Process` from `Consumer.Run` (a receiver method —
/// exercises `method_declaration` extraction, which is structurally different from every
/// other adapter: Go methods are top-level declarations carrying a receiver, not nested
/// inside a class/impl body); `service_test.go` calls `Process` from `TestProcess`, the
/// standard-library `go test` naming convention, in its own `_test.go` file. 5 symbols
/// total: `Helper`, `Process`, `Consumer`, `Consumer::Run`, `TestProcess`.
#[test]
fn go_adapter_resolves_calls_across_files_and_detects_tests() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);
    assert_eq!(stats["symbols_indexed"], 5);

    let report = query(cache_dir.path(), "util.go");
    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "service::Process", "file": "service.go", "line": 3, "confidence": "Exact"}])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "caller::Consumer::Run", "file": "caller.go", "line": 5, "confidence": "Exact"},
            {"path": "service_test::TestProcess", "file": "service_test.go", "line": 5, "confidence": "Exact"},
        ])
    );
    assert_eq!(report["tests"], 1);
}
