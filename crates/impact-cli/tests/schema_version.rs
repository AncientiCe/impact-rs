use std::path::Path;

use assert_cmd::Command;
use rusqlite::Connection;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy_crate")
}

/// Hand-builds a cache.sqlite in the pre-versioning shape (same tables `Cache::migrate`
/// creates, but no `PRAGMA user_version` ever set — exactly what a cache written by any
/// `impact` build before schema versioning existed looks like on disk today). Seeds it
/// with a node under a file the real fixture doesn't contain, so its survival (or not)
/// after `impact index` proves whether the whole cache was wiped rather than just the
/// fixture's one real file being incrementally replaced.
fn write_stale_cache(cache_dir: &Path) {
    std::fs::create_dir_all(cache_dir).unwrap();
    let conn = Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE file_hashes (file TEXT PRIMARY KEY, content_hash TEXT NOT NULL);
        CREATE TABLE nodes (
            id TEXT PRIMARY KEY, kind TEXT NOT NULL, qualified_path TEXT NOT NULL,
            file TEXT NOT NULL, line INTEGER NOT NULL, language TEXT NOT NULL,
            is_test INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE edges (from_id TEXT NOT NULL, to_id TEXT NOT NULL, kind TEXT NOT NULL, confidence TEXT NOT NULL);
        CREATE TABLE refs (file TEXT NOT NULL, from_qualified_path TEXT NOT NULL, to_name TEXT NOT NULL, kind TEXT NOT NULL);
        CREATE TABLE contract_refs (file TEXT NOT NULL, contract_kind TEXT NOT NULL, contract_id TEXT NOT NULL, symbol_name TEXT NOT NULL, role TEXT NOT NULL);
        INSERT INTO nodes (id, kind, qualified_path, file, line, language, is_test)
            VALUES ('ghost-id', '\"Function\"', 'ghost::STALE_MARKER_SYMBOL', 'src/ghost.rs', 1, 'rust', 0);
        INSERT INTO file_hashes (file, content_hash) VALUES ('src/ghost.rs', 'bogus');
        ",
    )
    .unwrap();
    // Deliberately no `PRAGMA user_version` write — defaults to 0, matching a cache
    // written before this feature existed.
}

/// `impact index` against a cache last written before schema versioning existed (tables
/// present, `user_version` at its SQLite default of 0) should notice the mismatch, wipe
/// the stale cache instead of trying to reuse it, and re-stamp it with the current
/// version — rather than silently mixing old-shaped rows into a query result.
#[test]
fn stale_schema_version_triggers_full_wipe_and_reindex() {
    let cache_dir = tempfile::tempdir().unwrap();
    write_stale_cache(cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap()])
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cache schema changed") && stderr.contains("wiping"),
        "expected a schema-wipe notice on stderr, got: {stderr}"
    );

    let conn = Connection::open(cache_dir.path().join("cache.sqlite")).unwrap();
    let ghost_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE qualified_path = 'ghost::STALE_MARKER_SYMBOL'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        ghost_count, 0,
        "stale row from before the schema bump should not survive a wipe"
    );

    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert!(
        version > 0,
        "cache should be re-stamped with a real schema version, got {version}"
    );
}

/// A brand-new cache (nothing to wipe) should index normally with no wipe notice on
/// stderr — the message is specifically for an existing, stale cache being discarded.
#[test]
fn fresh_cache_does_not_print_a_wipe_notice() {
    let cache_dir = tempfile::tempdir().unwrap();

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap()])
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cache schema changed"),
        "a first-time index of a fresh cache should not print a schema-wipe notice, got: {stderr}"
    );
}
