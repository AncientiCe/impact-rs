use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming_events")
}

/// This fixture's `impact.toml` sets `events.strategy = "naming_convention"` with
/// `naming_suffix = "Event"` and defines no marker trait at all — `PaymentCreatedEvent`
/// is recognized as an event purely by its name ending in "Event", proving the config
/// file is actually read and the naming-convention strategy (as opposed to the
/// marker-trait default exercised by the `contracts` fixture) actually runs.
#[test]
fn naming_convention_strategy_detects_event_from_impact_toml() {
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
        .args(["query", "src/events.rs", "--json"])
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
        serde_json::json!(["handler::on_created", "handler::publish"])
    );
    assert_eq!(report["events"], serde_json::json!(["PaymentCreatedEvent"]));
}
