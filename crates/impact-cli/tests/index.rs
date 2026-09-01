use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy_crate")
}

fn run_index(cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).expect("index --json output should be valid JSON")
}

/// `toy_crate/src/lib.rs` has exactly 7 symbols, hand-counted from the source:
/// PaymentService (struct), PaymentService::charge (method), validate (fn),
/// PaymentStatus (enum) plus its two variants Pending and Failed (indexed individually
/// so `--change "remove variant PaymentStatus::Failed"` has something to resolve
/// against), Chargeable (trait).
#[test]
fn indexes_toy_crate_and_reports_symbol_count() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = run_index(cache_dir.path());

    assert_eq!(stats["files_indexed"], 1);
    assert_eq!(stats["files_skipped"], 0);
    assert_eq!(stats["symbols_indexed"], 7);
}

#[test]
fn second_run_against_unchanged_fixture_is_skipped() {
    let cache_dir = tempfile::tempdir().unwrap();

    let first = run_index(cache_dir.path());
    assert_eq!(first["files_indexed"], 1);

    let second = run_index(cache_dir.path());
    assert_eq!(second["files_indexed"], 0);
    assert_eq!(second["files_skipped"], 1);
    assert_eq!(second["symbols_indexed"], 0);
}
