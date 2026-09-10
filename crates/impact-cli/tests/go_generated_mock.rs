//! A generated gomock/mockgen-style mock living beside the real implementation it mocks
//! — reproduced from a real checkout service repo dogfooded against this release: an
//! interface (`interface.go`), its real implementation (`http.go`), and a generated mock
//! (`interface_mock.go`, `mockgen`'s exact `// Code generated ... DO NOT EDIT.` header)
//! all share one package. A caller in a different package reaches the real
//! implementation only through the interface-typed field (`repository.Repository`), which
//! the Go adapter's cross-package resolution can only narrow down to "this package, this
//! method name" — before this fix, that matched the mock too, downgrading the real
//! implementation's own confidence from `Exact` to `Probable` and dropping it from a
//! `--min-confidence exact` report entirely, exactly the report this project's own
//! Impact protocol tells an agent to trust before editing.
//!
//! `go_generated_mock` (see `tests/fixtures/go_generated_mock`) reproduces the shape.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_generated_mock")
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

fn change(project: &Path, cache_dir: &Path, description: &str, min_confidence: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "change",
            description,
            "--min-confidence",
            min_confidence,
            "--json",
        ])
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

/// The regression this fixture exists to catch: renaming/changing the *real*
/// implementation's method used to report an empty blast radius at `--min-confidence
/// exact` — the generated mock's same-named method sitting in the same package diluted
/// the only real caller down to `Probable`, which `exact` filters out. A thin or empty
/// radius here is exactly the silent-safety-looking failure this tool exists to avoid.
#[test]
fn real_implementation_resolves_at_exact_despite_generated_mock() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = change(
        &project,
        cache_dir.path(),
        "change signature of repository::http::HTTPRepository::Notify",
        "exact",
    );

    assert_eq!(
        report["direct"],
        serde_json::json!([{
            "path": "usecase::usecase::UseCase::Do",
            "file": "usecase/usecase.go",
            "line": 11,
            "confidence": "Exact",
        }]),
        "the interface-typed call through repository.Repository should resolve to the \
         real HTTPRepository, not be diluted by the generated MockRepository sharing its \
         package and method name: {report}"
    );
}

/// The generated mock's own method is still a real, queryable symbol — this fix narrows
/// *disambiguation*, it doesn't hide generated code from the graph.
#[test]
fn generated_mock_itself_still_resolves_when_named_directly() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "change",
            "change signature of repository::interface_mock::MockRepository::Notify",
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
        "impact change on the generated mock's own method should still resolve: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
