//! A Go interface's own method spec as a blast-radius target. Idiomatic Go declares an
//! interface in its own file (`interface.go`) and implements it in sibling files, and callers
//! reach the method only through a value typed as the interface
//! (`func Load(s store.Store) { s.Fetch(...) }`). The interface method spec is where a
//! signature change starts — it breaks every implementation and every caller — yet it had
//! no symbol of its own: querying the interface's file reported no dependents at all, and
//! `impact change` on the method failed to resolve, while the same call edge hung off the
//! concrete implementation only.
//!
//! `go_interface_methods` (see `tests/fixtures/go_interface_methods`) reproduces the
//! shape: `store` declares `Store` in `interface.go`, its only real implementation `DB` in
//! `db.go`, and a mockgen mock in `interface_mock.go`; `service.Load` calls `s.Fetch` through
//! a `store.Store` parameter; `api.Handle` calls `service.Load`.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_interface_methods")
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

fn run(project: &Path, cache_dir: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(args)
        .args(["--min-confidence", "exact", "--json"])
        .arg("--project")
        .arg(project)
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("--json output should be valid JSON")
}

fn paths(entries: &Value) -> Vec<&str> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect()
}

/// The interface method resolves as a target, and the call through the interface-typed
/// parameter is its caller at `Exact` — the declared type names the interface — with the
/// chain up to the handler behind it.
#[test]
fn interface_method_reports_callers_through_the_interface() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = run(
        &project,
        cache_dir.path(),
        &[
            "change",
            "change signature of store::interface::Store::Fetch",
        ],
    );

    assert_eq!(
        report["direct"],
        serde_json::json!([{
            "path": "service::service::Load",
            "file": "service/service.go",
            "line": 6,
            "confidence": "Exact",
        }]),
        "s.Fetch(...) through a store.Store parameter should be a caller of Store::Fetch: {report}"
    );
    assert!(
        paths(&report["indirect"]).contains(&"api::handler::Handle"),
        "api::handler::Handle calls Load, so it should be INDIRECT: {report}"
    );
}

/// Querying the interface's own file — the usual "what does editing this file break"
/// question — finds the caller instead of an empty, safe-looking report.
#[test]
fn querying_the_interface_file_finds_its_callers() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = run(&project, cache_dir.path(), &["query", "store/interface.go"]);

    assert!(
        paths(&report["direct"]).contains(&"service::service::Load"),
        "expected service::service::Load among DIRECT dependents of store/interface.go: {report}"
    );
}

/// Indexing the interface method must not cost the real implementation its own `Exact`
/// caller: the same package-scoped call now also matches the interface's spec, which,
/// left to dilute, would drop `Load` to `Probable` and out of this report.
#[test]
fn implementation_keeps_its_exact_caller() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = run(
        &project,
        cache_dir.path(),
        &["change", "change signature of store::db::DB::Fetch"],
    );

    assert_eq!(
        report["direct"],
        serde_json::json!([{
            "path": "service::service::Load",
            "file": "service/service.go",
            "line": 6,
            "confidence": "Exact",
        }]),
        "the call through store.Store should still reach DB::Fetch at Exact: {report}"
    );
}
