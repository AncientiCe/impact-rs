use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/confidence")
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

fn query(cache_dir: &Path, extra_args: &[&str]) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/target.rs", "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .args(extra_args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

/// `confidence` wires (hand-traced from the source): `target.rs` declares `shared` and
/// `unique_target`; `decoy.rs` declares a second, unrelated `shared` purely to give the
/// linker's short-name tier two candidates for that name; `caller.rs` calls
/// `target::shared()` from `call_ambiguous` and `target::unique_target()` from
/// `call_precise`. Both calls use the same qualified-call syntax, but only `to_name`'s
/// *last* identifier is ever tracked (`impact-lang-rust`'s call extraction), so
/// `call_ambiguous` resolves against both `target::shared` and `decoy::shared` —
/// `Confidence::Heuristic` — while `call_precise` resolves against the one and only
/// `unique_target` in the project — `Confidence::Exact`.
#[test]
fn direct_dependents_carry_per_entry_confidence() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), &[]);

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "caller::call_ambiguous", "confidence": "Heuristic"},
            {"path": "caller::call_precise", "confidence": "Exact"},
        ])
    );
}

/// `--min-confidence exact` should hide the heuristic entry entirely, leaving only the
/// unambiguous one — the whole point of surfacing confidence per entry is being able to
/// filter down to just what's trustworthy.
#[test]
fn min_confidence_exact_filters_out_heuristic_entries() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), &["--min-confidence", "exact"]);

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "caller::call_precise", "confidence": "Exact"}])
    );
}
