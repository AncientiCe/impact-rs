use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/index_exclude")
}

fn run_index(project: &Path, cache_dir: &Path) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", project.to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("index --json output should be valid JSON")
}

/// `index_exclude` has two files (`src/main.rs`'s `app`, `src/vendor/thirdparty.rs`'s
/// `vendored`) and an `impact.toml` excluding `src/vendor/**`. Indexing the fixture as-is
/// should see only `main.rs`; indexing an identical copy with the `impact.toml` removed
/// (everything else byte-for-byte the same) should see both — proving the exclude glob,
/// not something else about the fixture, is what makes the difference.
#[test]
fn index_toml_exclude_glob_skips_matching_files() {
    let with_exclude_cache = tempfile::tempdir().unwrap();
    let with_exclude = run_index(&fixture_path(), with_exclude_cache.path());
    assert_eq!(with_exclude["files_indexed"], 1);
    assert_eq!(with_exclude["symbols_indexed"], 1);

    let without_toml_dir = tempfile::tempdir().unwrap();
    let src = without_toml_dir.path().join("src");
    std::fs::create_dir_all(src.join("vendor")).unwrap();
    std::fs::copy(fixture_path().join("src/main.rs"), src.join("main.rs")).unwrap();
    std::fs::copy(
        fixture_path().join("src/vendor/thirdparty.rs"),
        src.join("vendor/thirdparty.rs"),
    )
    .unwrap();

    let without_exclude_cache = tempfile::tempdir().unwrap();
    let without_exclude = run_index(without_toml_dir.path(), without_exclude_cache.path());
    assert_eq!(without_exclude["files_indexed"], 2);
    assert_eq!(without_exclude["symbols_indexed"], 2);
}
