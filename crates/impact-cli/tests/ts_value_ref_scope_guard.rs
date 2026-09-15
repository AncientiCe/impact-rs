use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_value_ref_scope_guard")
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
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

/// Regression fixture for a blind spot found 2026-09-14 while dogfooding the JSX/object-
/// literal `References` fix (commit 4044b36) on a real React Native app: the new edge kind
/// resolves a bare value-reference identifier (a JSX attribute's `{expr}` value) via
/// `FileScope::bare`, but when that lookup finds no import and no same-file top-level
/// declaration it falls through to `RefTarget::Opaque`, which the linker then resolves by
/// matching the bare name against *any* same-named symbol project-wide. React's own
/// callback-prop convention (`onDone`, `onSave`, `onSuccess`, ...) means a local parameter
/// forwarded as a JSX attribute value collides with unrelated same-named functions
/// elsewhere constantly — `AddAttribution.tsx`'s own local `onDone` callback parameter
/// registered as a "dependent" of `PrinterEditor.tsx` purely because both files happen to
/// use the word `onDone` somewhere.
///
/// `consumer.tsx::renderItem` takes a local parameter named `onDone` (not imported, not a
/// top-level declaration in this file) and forwards it as a JSX attribute value
/// (`<Row onDone={onDone} />`). `target.tsx` separately, and completely unrelated,
/// declares its own top-level function also named `onDone`. Querying `target.tsx` must not
/// report `consumer.tsx::renderItem` as a dependent: a bare identifier with no import or
/// same-file declaration backing it is not evidence of a real reference, so the
/// `References` edge should never be emitted for it in the first place — falling back to a
/// project-wide name match (as a resolved `call_expression` target already legitimately
/// does) manufactures a false positive here instead.
#[test]
fn jsx_attribute_value_with_no_scope_evidence_is_not_reported_as_a_dependent() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 2);

    let report = query(cache_dir.path(), "target.tsx");

    assert_eq!(report["direct"], serde_json::json!([]));
    assert_eq!(report["indirect"], serde_json::json!([]));
}
