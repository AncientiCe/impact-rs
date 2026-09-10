//! `tsconfig.json` path-alias resolution. Before this, `impact-lang-ts` only followed
//! relative (`./`/`../`) import specifiers — a bare one (`'@newstore/foo'`, `'widget'`)
//! was left unresolved on principle, rather than guessed at. That's the right call for a
//! real package name, but real repos also write internal module boundaries through
//! `tsconfig.json`'s `compilerOptions.paths` — a real React Native monorepo's own
//! tsconfig.json maps `@newstore/*` to `./src/packages/*`, `@testing/*` to a fixed
//! `./src/testing`, and even a catch-all `"*": ["./src/*"]` — so every import written in
//! that style produced a confidently *missing* edge, not just an unresolved one: the
//! same module, imported via `./utils` vs `@newstore/.../utils`, gets a real dependent in
//! one case and none in the other.
//!
//! `ts_path_aliases` (see `tests/fixtures/ts_path_aliases`) reproduces all three alias
//! shapes from a real tsconfig.json, including its inline `//` comment (tsconfig.json is
//! JSONC, not strict JSON) which a naive `serde_json::from_str` would fail to parse.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_path_aliases")
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

fn query(project: &Path, cache_dir: &Path, file: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", file, "--min-confidence", "exact", "--json"])
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

fn direct_paths(report: &Value) -> Vec<String> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect()
}

/// `@newstore/*` -> `./src/packages/*`: a wildcard alias whose target is itself a
/// wildcard, substituted from the matched capture.
#[test]
fn wildcard_to_wildcard_alias_resolves_at_exact_confidence() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = query(
        &project,
        cache_dir.path(),
        "src/packages/aisles/cart/sync/utils.js",
    );

    assert_eq!(
        direct_paths(&report),
        vec!["packages::aisles::cart::sync::actions::syncCart".to_string()],
        "actions.js imports via '@newstore/aisles/cart/sync/utils', which should resolve \
         through the @newstore/* -> ./src/packages/* alias: {report}"
    );
}

/// `@testing/*` -> `./src/testing` (a fixed target, no wildcard in it — the whole
/// specifier's suffix after the matched prefix is discarded, not appended).
#[test]
fn wildcard_alias_with_fixed_target_resolves() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = query(&project, cache_dir.path(), "src/testing.js");

    assert_eq!(
        direct_paths(&report),
        vec!["consumer::runAll".to_string()],
        "consumer.js imports '@testing/setup', which should resolve through the \
         @testing/* -> ./src/testing alias regardless of the 'setup' suffix: {report}"
    );
}

/// `"*": ["./src/*"]` — the catch-all every other pattern is tried before, since it
/// matches literally anything.
#[test]
fn catch_all_alias_resolves_a_bare_specifier() {
    let project = fixture();
    let cache_dir = tempfile::tempdir().unwrap();
    index(&project, cache_dir.path());

    let report = query(&project, cache_dir.path(), "src/widget.js");

    assert_eq!(
        direct_paths(&report),
        vec!["consumer::runAll".to_string()],
        "consumer.js imports the bare specifier 'widget', which should resolve through \
         the catch-all \"*\" -> \"./src/*\" alias: {report}"
    );
}
