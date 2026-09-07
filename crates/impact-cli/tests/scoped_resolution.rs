//! One test per remaining language adapter for the same question: does a dependent come
//! from evidence that the call lands here, or merely from a matching name?
//!
//! Each fixture is the same three-part shape — a module declaring a name, a real importer
//! of it, and an unrelated type with a same-named method of its own — because that shape
//! is what produced confidently wrong blast radii in every language.

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

fn direct(project: &Path, cache_dir: &Path, file: &str) -> Vec<(String, String)> {
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
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Python: `a.consumer.sync` imports the function by name, so it's `Exact`.
/// `b.client.Client.run` calls `self.camelize_order(...)` — its own method — and must not
/// appear at all. `b.legacy.run_legacy` calls a name it never imported, so it stays
/// visible but only as a guess.
#[test]
fn python_dependents_come_from_imports() {
    let project = fixture("python_imports");
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct(&project, cache_dir.path(), "a/util.py"),
        vec![
            ("a::consumer::sync".to_string(), "Exact".to_string()),
            ("b::legacy::run_legacy".to_string(), "Heuristic".to_string()),
        ]
    );
}

/// Go: `b.Consume` reaches `a.Prune` through the package selector on an import, so it's
/// `Exact`. `Printer.Run` calls `p.Prune(...)`, and `p`'s type is right there in the
/// method receiver — it resolves inside package `b` and never touches `a`.
#[test]
fn go_dependents_come_from_package_selectors() {
    let project = fixture("go_imports");
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct(&project, cache_dir.path(), "a/util.go"),
        vec![("b::consumer::Consume".to_string(), "Exact".to_string())]
    );
}

/// Kotlin: `b.consume` imports `a.prune` explicitly. `Printer.run` calls the `prune`
/// declared in its own class body, which is the nearer scope, so `a/Util.kt`'s blast
/// radius is the importer alone.
#[test]
fn kotlin_dependents_come_from_imports() {
    let project = fixture("kotlin_imports");
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct(&project, cache_dir.path(), "src/a/Util.kt"),
        vec![("b::Consumer::consume".to_string(), "Exact".to_string())]
    );
}

/// Swift is the exception, deliberately. It has no import that narrows a name — every
/// top-level symbol in a module is visible everywhere in it — so a bare `prune(x)` in
/// `Consumer.swift` really might be `Util.swift`'s, and structural resolution stays the
/// best evidence available rather than a guess. With two `prune`s in this fixture that
/// evidence is genuinely ambiguous, so `heuristic` is the honest answer, not a
/// regression.
///
/// What must not survive is `self.prune(...)` in `Printer`: `self` names the enclosing
/// type, whose own `prune` is right there, so `Printer.run` is not a dependent of
/// `Util.swift` at all.
#[test]
fn swift_keeps_module_wide_resolution_but_not_self_calls() {
    let project = fixture("swift_imports");
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    assert_eq!(
        direct(&project, cache_dir.path(), "Sources/A/Util.swift"),
        vec![(
            "Sources::B::Consumer::consume".to_string(),
            "Heuristic".to_string()
        )]
    );
}
