use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cpp_lang")
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

/// `cpp_lang` is a 6-file project (2 header-only prototypes, 4 definitions), hand-traced
/// from the source: `util.h` only declares `helper()` (a prototype, never indexed —
/// nothing else about `impact` treats a signature with no body as a symbol);
/// `util.cpp` defines it. `service.cpp` defines a free function `process()` that calls
/// `helper()` by its bare name — proving a name resolves project-wide even though C++
/// has no file-level import an adapter could use to narrow it (see the adapter's module
/// doc). `widget.h` declares `class Widget { void render(); };` (again a prototype, not
/// indexed); `widget.cpp` *defines* `Widget::render()` out of line — the dominant
/// real-world C++ pattern this adapter exists to handle — and it also calls `helper()`
/// bare. `consumer.cpp` declares `class Consumer` with an inline method `run()` that
/// calls `process()` bare, one call removed from `helper()`. 6 symbols total: `helper`,
/// `process`, `Consumer` (the type), `Consumer::run`, `Widget` (the type),
/// `Widget::render`.
#[test]
fn cpp_adapter_resolves_calls_across_files_including_out_of_line_methods() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 6);
    assert_eq!(stats["symbols_indexed"], 6);

    let report = query(cache_dir.path(), "util.cpp");
    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "service::process", "file": "service.cpp", "line": 3, "confidence": "Exact"},
            {"path": "widget::Widget::render", "file": "widget.cpp", "line": 4, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([
            {"path": "consumer::Consumer::run", "file": "consumer.cpp", "line": 3, "confidence": "Exact"},
        ])
    );
    assert_eq!(report["tests"], 0);
}
