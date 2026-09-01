use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed_lang")
}

fn index(cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).unwrap()
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
    serde_json::from_slice(&output.stdout).unwrap()
}

/// This is `impact-lang-ts`'s only integration point with anything outside its own crate:
/// `TsAdapter` registered in the same `Vec<&dyn LanguageAdapter>` as `RustAdapter` (see
/// `ops::index_project`). Nothing in `impact-core` changed to support it — the whole
/// point of this fixture is proving that a second language adapter plugs into indexing,
/// linking, and the blast-radius engine unmodified.
///
/// `mixed_lang` has one Rust file (`lib.rs`, `rust_side`, unrelated) plus a 3-file
/// TypeScript chain, hand-traced from the source: `util.ts` exports `helper`; `service.ts`
/// imports and calls it from `process`; `caller.ts` imports `process` and calls it from
/// `Runner.run` (a class method — also exercises `class_declaration`/`method_definition`
/// extraction). 5 symbols total: `rust_side`, `helper`, `process`, `Runner`, `Runner::run`.
#[test]
fn typescript_adapter_resolves_calls_across_files() {
    let cache_dir = tempfile::tempdir().unwrap();

    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);
    assert_eq!(stats["symbols_indexed"], 5);

    let report = query(cache_dir.path(), "src/util.ts");
    assert_eq!(report["direct"], serde_json::json!(["service::process"]));
    assert_eq!(
        report["indirect"],
        serde_json::json!(["caller::Runner::run"])
    );
}

/// The Rust file in the same project is indexed by `RustAdapter` as normal, entirely
/// unaffected by `TsAdapter` also being registered — each adapter only ever sees the
/// files its own `file_globs` claims.
#[test]
fn rust_file_in_the_same_project_is_unaffected() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/lib.rs");
    assert_eq!(report["direct"], serde_json::json!([]));
    assert_eq!(report["indirect"], serde_json::json!([]));
}
