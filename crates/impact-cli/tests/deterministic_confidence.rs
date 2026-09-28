use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed_confidence_paths")
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

/// The same query against the same index used to give different reports from run to run
/// (seen on a real 2,700-file app as 1,115 vs 1,116 dependents for one file).
///
/// `both` in `caller.ts` calls both of `target.ts`'s functions: `exact` through an
/// import (`Exact`), and `ambiguous` through an untyped receiver that could as well be
/// `other.ts`'s `ambiguous` (`Heuristic`). Querying `target.ts` seeds both functions, and
/// the walk took `both`'s confidence from whichever seed it happened to visit first —
/// seeds come out of a hash set, in a different order every process. Visited from
/// `ambiguous` first, `both` was `Heuristic`, which is never walked further, so `entry`
/// vanished from the report.
///
/// A dependent reached through several edges in the same hop now takes the strongest
/// one, whatever order they're visited in. Each run is a fresh process with its own hash
/// seed, so twenty of them hit both orders.
#[test]
fn a_dependent_reached_two_ways_takes_the_strongest_regardless_of_visit_order() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    for _ in 0..20 {
        let report = query(cache_dir.path(), "src/target.ts");
        assert_eq!(
            report["direct"],
            serde_json::json!([
                {"path": "caller::both", "file": "src/caller.ts", "line": 3, "confidence": "Exact"},
            ])
        );
        assert_eq!(
            report["indirect"],
            serde_json::json!([
                {"path": "top::entry", "file": "src/top.ts", "line": 3, "confidence": "Exact"},
            ])
        );
    }
}
