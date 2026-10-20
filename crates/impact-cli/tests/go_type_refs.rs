//! A Go type that is only ever *named*, never called. Renaming or moving an imported
//! struct has to update every place another package names it — a dependency-struct field
//! (`Svc source.Service`), a parameter, a result, a composite literal, an interface method
//! signature — yet none of those is a call, and before this the Go adapter only recorded
//! calls, so `rename` of such a type reported no dependents at all (GitHub issue #15).
//!
//! `go_type_refs` (see `tests/fixtures/go_type_refs`) declares `source.Service` and names
//! it once per shape from the `consumer` package, plus one function that doesn't name it.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_type_refs")
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

fn change(project: &Path, cache_dir: &Path, description: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["change", description, "--json"])
        .arg("--project")
        .arg(project)
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("change --json output should be valid JSON")
}

/// Every consumer that names the imported type is a direct dependent, at `Exact` — the
/// import of `source` is the evidence tying `source.Service` to that package's `Service`.
#[test]
fn renaming_an_imported_struct_reaches_every_place_that_names_it() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = change(
        &project,
        cache_dir.path(),
        "rename source::service::Service",
    );

    let mut direct: Vec<(String, String)> = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    direct.sort();

    let expected: Vec<(String, String)> = [
        "consumer::deps::Dependencies",
        "consumer::handler::Build",
        "consumer::handler::Default",
        "consumer::handler::Handle",
        "consumer::handler::Store",
    ]
    .iter()
    .map(|path| (path.to_string(), "Exact".to_string()))
    .collect();

    assert_eq!(
        direct, expected,
        "each consumer naming source.Service — field, interface signature, result, \
         composite literal, parameter — should be an Exact direct dependent, and \
         Unrelated should not: {report}"
    );
}
