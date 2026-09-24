use std::collections::BTreeSet;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_hyphenated_workspace")
}

fn run_index(project: &Path, cache_dir: &Path) {
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

/// Every file that shows up among a report's direct or indirect dependents.
fn dependent_files(report: &Value, section: &str) -> BTreeSet<String> {
    report[section]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect()
}

fn indexed_report(file: &str) -> Value {
    let cache = tempfile::tempdir().unwrap();
    run_index(&fixture_path(), cache.path());
    query(&fixture_path(), cache.path(), file)
}

/// `rust_hyphenated_workspace` names its crates the way most Cargo workspaces do: with
/// hyphens (`wire-protocol`), which Rust code spells with underscores
/// (`use wire_protocol::ClientFrame`). Every file that imports `wire_protocol` is a
/// dependent of its root module: `edge-backend`'s lib (a grouped, multi-line `use`), its
/// `admission.rs` (which names `TurnRequest` only as a parameter type) and its integration
/// test, `pod-gateway`'s `bridge.rs`, and `loadtest`'s integration test, which reaches the
/// crate only through a dev-dependency.
#[test]
fn a_hyphenated_crate_is_found_through_its_underscored_import_name() {
    let report = indexed_report("crates/wire-protocol/src/lib.rs");
    let files = dependent_files(&report, "direct");

    for expected in [
        "apps/edge-backend/src/lib.rs",
        "apps/edge-backend/src/admission.rs",
        "apps/edge-backend/tests/routes.rs",
        "apps/pod-gateway/src/bridge.rs",
        "apps/loadtest/tests/capacity.rs",
    ] {
        assert!(
            files.contains(expected),
            "expected {expected} among direct dependents, got {files:?}"
        );
    }
}

/// `use wire_protocol::codec::encode_frame` names a function in the crate's `codec`
/// submodule, so `bridge.rs` depends on `codec.rs`.
#[test]
fn a_submodule_of_a_workspace_crate_is_found_through_its_import_path() {
    let report = indexed_report("crates/wire-protocol/src/codec.rs");
    let files = dependent_files(&report, "direct");
    assert!(
        files.contains("apps/pod-gateway/src/bridge.rs"),
        "expected bridge.rs to depend on codec.rs, got {files:?}"
    );
}

/// `pod-gateway` depends on `ticket-store` under a renamed key
/// (`store = { package = "ticket-store" }`), so `use store::TicketStore` means that
/// crate, and `bridge.rs`'s `self.tickets.list_open()` is one of its dependents.
#[test]
fn a_renamed_dependency_resolves_to_the_package_it_names() {
    let report = indexed_report("crates/ticket-store/src/lib.rs");
    let files = dependent_files(&report, "direct");
    assert!(
        files.contains("apps/pod-gateway/src/bridge.rs"),
        "expected bridge.rs to depend on ticket-store, got {files:?}"
    );
}

/// `bridge.rs` is reached from `pod-gateway`'s own binary (through its lib's import name,
/// `pod_gateway`) and from `loadtest`'s integration test (a dev-dependency) — and from
/// nothing else. `ticket-store` calls a method named `relay_frames` on a value of unknown
/// type, a name that only `Bridge` declares, but `ticket-store` doesn't depend on
/// `pod-gateway` (the dependency runs the other way), so that call can't be a caller.
/// Neither can `loadtest`'s own binary making the same kind of call: `pod-gateway` is only
/// a dev-dependency there, which Cargo keeps away from shipped code.
#[test]
fn a_crate_that_is_not_a_dependency_is_never_a_caller() {
    let report = indexed_report("apps/pod-gateway/src/bridge.rs");
    let direct = dependent_files(&report, "direct");
    let indirect = dependent_files(&report, "indirect");

    let expected: BTreeSet<String> = [
        "apps/pod-gateway/src/main.rs",
        "apps/loadtest/tests/capacity.rs",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(direct, expected, "direct dependents of bridge.rs");
    assert!(
        indirect.iter().all(|f| !f.starts_with("crates/")),
        "no upstream crate may depend on bridge.rs, got {indirect:?}"
    );
}
