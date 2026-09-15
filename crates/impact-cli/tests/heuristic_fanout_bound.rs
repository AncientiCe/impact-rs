use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/heuristic_fanout_bound")
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

fn query(cache_dir: &Path) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/target.rs", "--json"])
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

/// Regression fixture for a blind spot found 2026-09-15 while dogfooding `impact` against
/// a real, large (~30k-symbol, 7-language) monorepo: querying one Swift file returned 61
/// direct + 7066 indirect dependents, all `Confidence::Heuristic`, pointing at completely
/// unrelated JS/TS files — a result so large (34k+ lines) it exceeded the MCP response
/// limit and had to be read from a saved file. Root cause traced to `compute_impact`'s BFS
/// (`engine.rs`): once a hop resolves at `Confidence::Heuristic` (a bare short name
/// matching more than one candidate — structurally the *weakest*, least trustworthy
/// evidence this tool produces), the BFS kept right on treating that node as a trusted
/// stepping stone and chasing *its* callers for further indirect hops — so one ambiguous
/// short-name collision (common in a codebase this size, doubly so for Swift, whose own
/// scope resolution is module-wide by name rather than import-scoped) could fan out
/// through an unbounded number of further hops, compounding an already-weak signal rather
/// than just reporting it and stopping.
///
/// `target.rs` declares `shared`; `decoy.rs` declares an unrelated same-named `shared`
/// purely to make the short name ambiguous; `caller::call_ambiguous` calls it by that bare
/// short name, resolving `Heuristic` (two candidates) — a real, if weak, DIRECT dependent
/// of `target.rs`, and correctly still reported as one. `far_caller::call_two_hops_away`
/// then calls `caller::call_ambiguous` through an ordinary, unambiguous import — an edge
/// that would resolve `Exact` on its own — but is reachable from `target.rs` only by
/// continuing the BFS *through* the already-`Heuristic` `call_ambiguous` node. It must not
/// appear as an INDIRECT dependent: the engine should surface a `Heuristic` hop (over-
/// report rather than silently miss a caller) without treating it as reliable enough to
/// keep chaining through.
#[test]
fn bfs_does_not_continue_past_a_heuristic_confidence_hop() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path());

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "caller::call_ambiguous", "file": "src/caller.rs", "line": 6, "confidence": "Heuristic"},
        ])
    );
    assert_eq!(report["indirect"], serde_json::json!([]));
}
