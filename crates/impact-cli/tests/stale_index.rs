use std::path::Path;

use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_file")
}

/// Copies a checked-in fixture into a temp directory so a test can delete files from it.
/// The fixtures themselves stay read-only source of truth; only the copy is mutated.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn index(project: &Path, cache_dir: &Path) -> Value {
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

fn query(project: &Path, cache_dir: &Path, file: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", file, "--json"])
        .arg("--project")
        .arg(project)
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

fn dependent_paths(report: &Value, bucket: &str) -> Vec<String> {
    report[bucket]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect()
}

/// A file deleted from disk must disappear from the index on the next run, without
/// `--force`. Before this was fixed, its nodes, refs, and edges survived indefinitely
/// (nothing ever removed a `file_hashes` row), so a blast radius kept naming a caller
/// that no longer existed — pointing at a file the user could no longer even open.
#[test]
fn deleted_file_is_pruned_from_the_index_on_the_next_run() {
    let project = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture_path(), project.path());

    index(project.path(), cache_dir.path());
    let before = query(project.path(), cache_dir.path(), "src/payment/service.rs");
    assert_eq!(
        dependent_paths(&before, "direct"),
        vec!["payment::controller::PaymentController::handle"],
        "fixture precondition: controller.rs is the one direct caller of service.rs"
    );

    std::fs::remove_file(project.path().join("src/payment/controller.rs")).unwrap();
    let stats = index(project.path(), cache_dir.path());
    assert_eq!(
        stats["files_pruned"], 1,
        "the deleted file should be reported as pruned, got: {stats}"
    );

    let after = query(project.path(), cache_dir.path(), "src/payment/service.rs");
    assert!(
        dependent_paths(&after, "direct").is_empty(),
        "a deleted file must not stay in the blast radius, got: {:?}",
        after["direct"]
    );
}

/// Indexing a project that hasn't lost any files must not report phantom prunes — the
/// prune pass has to key off what's actually on disk, not off which files were re-parsed
/// on this run (every unchanged file is skipped, and skipping is not deletion).
#[test]
fn unchanged_project_prunes_nothing() {
    let project = tempfile::tempdir().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture_path(), project.path());

    index(project.path(), cache_dir.path());
    let second = index(project.path(), cache_dir.path());

    assert_eq!(second["files_indexed"], 0);
    assert_eq!(second["files_pruned"], 0);
    assert!(second["files_skipped"].as_u64().unwrap() > 0);
}

/// A cache written by a *different build of impact* is stale even when every file's
/// content hash still matches: the symbols in it were produced by the old extractor, so
/// skipping unchanged files preserves whatever that build failed to see. This is the
/// defect behind the real-world report of `files_indexed: 11, files_skipped: 2859` right
/// after an upgrade — the content hashes matched, so the old (under-extracted) symbols
/// survived, and only `--force` rebuilt them.
#[test]
fn cache_from_a_different_extractor_version_is_rebuilt_without_force() {
    let cache_dir = tempfile::tempdir().unwrap();

    let first = index(&fixture_path(), cache_dir.path());
    let indexed_first_time = first["files_indexed"].as_u64().unwrap();
    assert!(indexed_first_time > 0);

    // Stamp the cache as having been written by an older build, leaving every file hash
    // (and every extracted symbol) untouched — exactly the on-disk state an upgrade
    // leaves behind.
    let conn = Connection::open(cache_dir.path().join("cache.sqlite")).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT OR REPLACE INTO meta (key, value) VALUES ('extractor_version', '0.0.0-old');
        ",
    )
    .unwrap();
    drop(conn);

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("built by a different impact version"),
        "expected a version-mismatch notice on stderr, got: {stderr}"
    );

    let stats: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        stats["files_indexed"], indexed_first_time,
        "every file should be re-parsed by the new extractor, not skipped: {stats}"
    );
    assert_eq!(stats["files_skipped"], 0);
}

/// The counterpart to the test above: a cache written by *this* build is reused, so the
/// content-hash skip that makes re-indexing cheap still works. Rebuilding on every run
/// would be correct but unusably slow on a real repo.
#[test]
fn cache_from_the_same_extractor_version_still_skips_unchanged_files() {
    let cache_dir = tempfile::tempdir().unwrap();

    index(&fixture_path(), cache_dir.path());
    let second = index(&fixture_path(), cache_dir.path());

    assert_eq!(second["files_indexed"], 0);
    assert!(second["files_skipped"].as_u64().unwrap() > 0);
}
