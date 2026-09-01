use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn index(project: &Path, cache_dir: &Path) {
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
}

/// Writes a `workspace.toml` registering the three fixture projects at `dir`, with
/// per-project `cache_dir` overrides pointing at `cache_b`/`cache_w`/`cache_r` — real
/// temp directories, so the test never writes a `.impact/` folder into the checked-in
/// fixture trees. Mirrors the two links hand-traced in the module doc below.
fn write_workspace_toml(
    dir: &Path,
    cache_b: &Path,
    cache_w: &Path,
    cache_r: &Path,
) -> std::path::PathBuf {
    let contents = format!(
        r#"
[[projects]]
id = "backend"
path = {backend:?}
cache_dir = {cache_b:?}

[[projects]]
id = "web"
path = {web:?}
cache_dir = {cache_w:?}

[[projects]]
id = "reporting"
path = {reporting:?}
cache_dir = {cache_r:?}

[[links]]
produces = "backend:PaymentCreated"
consumes = "web"

[[links]]
produces = "backend"
consumes = "web"
"#,
        backend = fixture("workspace_backend").to_str().unwrap(),
        web = fixture("workspace_web").to_str().unwrap(),
        reporting = fixture("workspace_reporting").to_str().unwrap(),
        cache_b = cache_b.to_str().unwrap(),
        cache_w = cache_w.to_str().unwrap(),
        cache_r = cache_r.to_str().unwrap(),
    );
    let path = dir.join("workspace.toml");
    std::fs::write(&path, contents).unwrap();
    path
}

/// `workspace_backend` publishes three events (constructs `PaymentCreated`, `OrderPlaced`,
/// `UnrelatedEvent` — each in its own function, so each shows as a DIRECT dependent of
/// `events.rs`); `workspace_web` independently declares and consumes `PaymentCreated` and
/// `OrderPlaced` (a typed parameter each); `workspace_reporting` independently declares
/// and consumes only `UnrelatedEvent`. The workspace links backend:PaymentCreated
/// specifically to web (Declared), and backend to web generally (Strong — catches
/// OrderPlaced, which no link names specifically) — reporting has no link to backend at
/// all, so its shared `UnrelatedEvent` identity match is Weak. All three tiers, from one
/// workspace, hand-traced from the fixtures' source, not the implementation.
#[test]
fn cross_project_matches_all_three_confidence_tiers() {
    let cache_b = tempfile::tempdir().unwrap();
    let cache_w = tempfile::tempdir().unwrap();
    let cache_r = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    index(&fixture("workspace_backend"), cache_b.path());
    index(&fixture("workspace_web"), cache_w.path());
    index(&fixture("workspace_reporting"), cache_r.path());
    let workspace_toml = write_workspace_toml(
        ws_dir.path(),
        cache_b.path(),
        cache_w.path(),
        cache_r.path(),
    );

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/events.rs", "--json"])
        .arg("--project")
        .arg(fixture("workspace_backend"))
        .arg("--cache-dir")
        .arg(cache_b.path())
        .arg("--workspace")
        .arg(&workspace_toml)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(
        report["cross_project"],
        serde_json::json!([
            {"project_id": "reporting", "contract_kind": "Event", "contract_id": "UnrelatedEvent", "confidence": "weak"},
            {"project_id": "web", "contract_kind": "Event", "contract_id": "OrderPlaced", "confidence": "strong"},
            {"project_id": "web", "contract_kind": "Event", "contract_id": "PaymentCreated", "confidence": "declared"},
        ])
    );
}

/// A workspace member that hasn't been indexed yet (no cache at its `cache_dir`) is
/// silently absent from `cross_project` results, not an error — cross-project matching
/// is opportunistic over whatever's actually been indexed.
#[test]
fn unindexed_sibling_project_is_skipped_not_an_error() {
    let cache_b = tempfile::tempdir().unwrap();
    let cache_w = tempfile::tempdir().unwrap();
    let cache_r = tempfile::tempdir().unwrap(); // never indexed
    let ws_dir = tempfile::tempdir().unwrap();

    index(&fixture("workspace_backend"), cache_b.path());
    index(&fixture("workspace_web"), cache_w.path());
    let workspace_toml = write_workspace_toml(
        ws_dir.path(),
        cache_b.path(),
        cache_w.path(),
        cache_r.path(),
    );

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/events.rs", "--json"])
        .arg("--project")
        .arg(fixture("workspace_backend"))
        .arg("--cache-dir")
        .arg(cache_b.path())
        .arg("--workspace")
        .arg(&workspace_toml)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    let projects: Vec<&str> = report["cross_project"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["project_id"].as_str().unwrap())
        .collect();
    assert!(
        !projects.contains(&"reporting"),
        "unindexed project should not appear: {report}"
    );
    assert!(
        projects.contains(&"web"),
        "indexed sibling should still be matched: {report}"
    );
}
