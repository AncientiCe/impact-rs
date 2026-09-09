//! Idiomatic Go dependency injection — a struct field typed as an interface, called
//! through a one-hop selector chain (`uc.ChangesRepository.AddOperation(...)`) rather than
//! a plain `receiver.Method()` call. Confirmed via a real dogfooding session (grep found 7
//! real call sites for this exact pattern across 6 files in a checkout service; `impact
//! change` at `--min-confidence exact` found only 1, the same-file one) that this shape
//! was essentially invisible to the linker: the callee's selector operand
//! (`uc.ChangesRepository`) is itself a nested selector expression, not a plain
//! identifier, and the code only handled the latter.
//!
//! `go_di_fields` (see `tests/fixtures/go_di_fields`) reproduces the shape: `changes`
//! declares the `Repository` interface and its only real implementation,
//! `repositoryImpl.AddOperation`; `usecase.UseCase` holds a `ChangesRepository
//! changes.Repository` field and calls `uc.ChangesRepository.AddOperation(...)` from
//! `Do`.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_di_fields")
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

/// The regression this fixture exists to catch: at `--min-confidence exact`, a plain
/// `receiver.Method()` call resolves through the receiver's own declared type and is
/// `Exact` — but before this fix, a one-hop `receiver.field.Method()` call fell all the
/// way through to a bare, `Opaque`-capped short-name match (`Probable` at best), which
/// `--min-confidence exact` filters out entirely. So the pre-fix report's `direct` is
/// empty here; the fix's job is for `usecase::usecase::UseCase::Do` to show up, and at
/// `Exact`.
#[test]
fn field_selector_call_resolves_at_exact_confidence() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "change",
            "change signature of changes::repository::repositoryImpl::AddOperation",
            "--min-confidence",
            "exact",
            "--json",
        ])
        .arg("--project")
        .arg(&project)
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("change --json output should be valid JSON");

    assert_eq!(
        report["direct"],
        serde_json::json!([{
            "path": "usecase::usecase::UseCase::Do",
            "file": "usecase/usecase.go",
            "line": 13,
            "confidence": "Exact",
        }]),
        "uc.ChangesRepository.AddOperation(...) should resolve to repositoryImpl::AddOperation \
         at Exact confidence, via the field's declared interface type's package: {report}"
    );
}

/// Querying the interface's own defining file (`impact query`/`impact_file`, file-mode
/// seeding every symbol declared there) should likewise surface the use-case as a
/// dependent — the same call site, reached the other direction.
#[test]
fn querying_the_repository_file_finds_the_use_case() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "query",
            "changes/repository.go",
            "--min-confidence",
            "exact",
            "--json",
        ])
        .arg("--project")
        .arg(&project)
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON");

    let direct_paths: Vec<&str> = report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();
    assert!(
        direct_paths.contains(&"usecase::usecase::UseCase::Do"),
        "expected usecase::usecase::UseCase::Do among DIRECT dependents of changes/repository.go: {report}"
    );
}
