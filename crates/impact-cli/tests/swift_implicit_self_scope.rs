use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/swift_implicit_self_scope")
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

/// Regression fixture for a blind spot found 2026-09-15 while dogfooding `impact` against
/// a real ~30k-symbol, 7-language monorepo: `impact-lang-swift`'s `build_scope` only ever
/// declared a file's *top-level* function/class/protocol names as locals — never a
/// class's own methods — so an *implicit-self* call (`helper()` from inside another
/// method of the same type, valid Swift with no `self.` prefix required, arguably Swift's
/// single most common calling convention) fell through to `RefTarget::Unscoped` exactly
/// as if it named something in a completely different file. Combined with `Unscoped`'s
/// project-wide short-name fallback and a large project having many short, ordinary
/// method names, this was the dominant source of the 7066-indirect-dependent explosion
/// documented in `heuristic_fanout_bound` (the general BFS-side half of that fix) —
/// `self.helper()` already resolved correctly via `FileScope::own()`; only the bare,
/// implicit-self spelling was affected.
///
/// `Widget.swift` declares `Widget` with `run()` calling `helper()` by its bare,
/// implicit-self name — a real, same-type reference. `Decoy.swift` declares an unrelated
/// top-level `helper()` purely to give the *old* behavior (project-wide bare-name
/// resolution) a second, wrong candidate to match. Querying `Decoy.swift` is the signal
/// to check, not `Widget.swift`: `run` and `helper` are declared in the same file being
/// queried there, so `compute_file_impact` correctly treats `run` as part of that file's
/// own blast radius rather than an external dependent of it, regardless of this fix —
/// only a query for the *decoy* isolates whether `run`'s call was ever misattributed to
/// it. Before this fix, `run`'s bare call resolved `Unscoped`, matching both `helper`
/// candidates project-wide: `Decoy.swift` incorrectly showed `run` as a `Heuristic`
/// dependent even though it never calls into `Decoy.swift` at all. After this fix, the
/// call resolves only to the sibling method declared two lines away, so `Decoy.swift`
/// has no dependents.
#[test]
fn implicit_self_call_does_not_falsely_resolve_to_a_same_named_decoy_elsewhere() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let decoy_report = query(cache_dir.path(), "Decoy.swift");
    assert_eq!(decoy_report["direct"], serde_json::json!([]));
    assert_eq!(decoy_report["indirect"], serde_json::json!([]));
}
