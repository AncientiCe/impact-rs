use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generic_impl")
}

/// A generic `impl<T> Container<T>` block's method should be reported by its plain type
/// name (`container::Container::get`), not with the generic parameter list leaked into
/// the qualified path (`container::Container<T>::get`) — found by dogfooding `impact`
/// against its own `impl<'a> Indexer<'a>`, which produced exactly that noise.
#[test]
fn generic_impl_method_qualified_path_has_no_type_parameters() {
    let cache_dir = tempfile::tempdir().unwrap();

    let index_output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        index_output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&index_output.stderr)
    );

    let query_output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/util.rs", "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        query_output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&query_output.stderr)
    );
    let report: Value = serde_json::from_slice(&query_output.stdout)
        .expect("query --json output should be valid JSON");

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "container::Container::get", "confidence": "Exact"}])
    );
}
