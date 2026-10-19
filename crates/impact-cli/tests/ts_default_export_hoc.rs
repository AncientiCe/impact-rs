use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_default_export_hoc")
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

/// Regression fixture for [#14](https://github.com/AncientiCe/impact-rs/issues/14): a
/// component whose default export is wrapped in a higher-order component
/// (`export default withTheme('card', {})(Card)`, `export default memo(List)`) and is
/// rendered through a default import (`import Card from './card'` + `<Card />`) had no
/// dependents at all. The default-export marker only recognized `export default Name` and
/// named declarations, so no symbol in the module was flagged as its default and the
/// default import resolved to nothing. The wrapped component is the argument of the
/// outermost call, unwrapped through nested wrappers.
#[test]
fn default_export_wrapped_in_a_higher_order_component_resolves_to_the_wrapped_component() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);

    let card_report = query(cache_dir.path(), "card.tsx");
    assert_eq!(
        card_report["direct"],
        serde_json::json!([
            {"path": "screen::Screen", "file": "screen.tsx", "line": 4, "confidence": "Exact"},
        ])
    );

    let list_report = query(cache_dir.path(), "list.tsx");
    assert_eq!(
        list_report["direct"],
        serde_json::json!([
            {"path": "screen::Screen", "file": "screen.tsx", "line": 4, "confidence": "Exact"},
        ])
    );
}
