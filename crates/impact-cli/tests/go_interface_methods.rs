//! A Go interface's own method spec as a blast-radius target, and the methods satisfying
//! it. Idiomatic Go declares an interface in its own file (`interface.go`), implements it
//! elsewhere — in the same package, or in an adapter package that never names it — and
//! callers reach the method only through a value typed as the interface
//! (`func Load(s store.Store) { s.Fetch(...) }`). The interface method spec is where a
//! signature change starts — it breaks every implementation and every caller — yet it had
//! no symbol of its own: querying the interface's file reported no dependents at all, and
//! `impact change` on the method failed to resolve, while the same call edge hung off the
//! concrete implementation only.
//!
//! `go_interface_methods` (see `tests/fixtures/go_interface_methods`) reproduces the
//! shape: `store` declares `Store` (`Fetch`, `Save`) and `ReadStore` (embedding `Store`)
//! in `interface.go`; `DB` implements `Store` across `db.go` and `db_write.go`, and a
//! mockgen mock in `interface_mock.go`; `memory.Cache` implements it from another package,
//! while `memory.Lookup` has same-named methods with other signatures and `memory.Closer`
//! has only `ReadStore`'s own `Close`; `service.Load` calls `s.Fetch` through a
//! `store.Store` parameter; `api.Handle` calls `service.Load`.

use std::collections::BTreeSet;
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

fn change(description: &str) -> Value {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());
    run(&project, cache_dir.path(), &["change", description])
}

fn paths(entries: &Value) -> BTreeSet<&str> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect()
}

/// The interface method resolves as a target. Its dependents are the call through the
/// interface-typed parameter — `Exact`, since the declared type names the interface —
/// and every method satisfying it, whichever package or file it lives in, including the
/// generated mock that has to be regenerated; the handler is behind the call.
#[test]
fn interface_method_reports_callers_and_implementations() {
    let report = change("change signature of store::interface::Store::Fetch");

    assert_eq!(
        paths(&report["direct"]),
        BTreeSet::from([
            "memory::memory::Cache::Fetch",
            "service::service::Load",
            "store::db::DB::Fetch",
            "store::interface_mock::MockStore::Fetch",
        ]),
        "the caller through store.Store and every Store implementation should be DIRECT: {report}"
    );
    assert!(
        paths(&report["indirect"]).contains("api::handler::Handle"),
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
        paths(&report["direct"]).contains("service::service::Load"),
        "expected service::service::Load among DIRECT dependents of store/interface.go: {report}"
    );
}

/// Indexing the interface method must not cost the real implementation its own `Exact`
/// caller: the same package-scoped call now also matches the interface's spec, which,
/// left to dilute, would drop `Load` to `Probable` and out of this report. The spec it
/// satisfies is a dependent too: changing the method stops `DB` being a `Store`.
#[test]
fn implementation_keeps_its_exact_caller() {
    let report = change("change signature of store::db::DB::Fetch");

    let direct = paths(&report["direct"]);
    assert!(
        direct.contains("service::service::Load")
            && direct.contains("store::interface::Store::Fetch"),
        "the call through store.Store and the spec DB satisfies should be DIRECT: {report}"
    );
    let load = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "service::service::Load")
        .unwrap();
    assert_eq!(load["confidence"], "Exact", "{report}");
}

/// An implementation in another package — an adapter implementing a port — is reached
/// by nothing but the interface. Changing it reports the spec it satisfies and, through
/// it, the callers that would hit it at runtime.
#[test]
fn implementation_in_another_package_reaches_callers_through_the_interface() {
    let report = change("change signature of memory::memory::Cache::Fetch");

    assert!(
        paths(&report["direct"]).contains("store::interface::Store::Fetch"),
        "Cache satisfies store.Store, so Store::Fetch should be DIRECT: {report}"
    );
    assert!(
        paths(&report["indirect"]).contains("service::service::Load"),
        "Load calls Fetch through store.Store, so it should be INDIRECT: {report}"
    );
}

/// Same method names are not an implementation: `Lookup`'s signatures differ from
/// `Store`'s, so neither side reports the other.
#[test]
fn same_names_with_other_signatures_are_not_an_implementation() {
    let spec = change("change signature of store::interface::Store::Fetch");
    assert!(
        !paths(&spec["direct"]).contains("memory::memory::Lookup::Fetch"),
        "Lookup.Fetch(int) string doesn't satisfy Store: {spec}"
    );

    let lookup = change("change signature of memory::memory::Lookup::Fetch");
    assert!(
        paths(&lookup["direct"]).is_empty(),
        "nothing depends on Lookup.Fetch: {lookup}"
    );
}

/// An interface embedding another has methods declared elsewhere, so its own specs are
/// not its whole method set: `Closer`, with only `ReadStore`'s own `Close`, isn't a
/// `ReadStore`.
#[test]
fn embedded_interface_is_not_satisfied_by_its_own_methods_alone() {
    let report = change("change signature of store::interface::ReadStore::Close");

    assert!(
        !paths(&report["direct"]).contains("memory::memory::Closer::Close"),
        "Closer lacks the Store methods ReadStore embeds: {report}"
    );
}
