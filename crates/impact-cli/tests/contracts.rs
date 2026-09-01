use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contracts")
}

fn index(cache_dir: &Path) {
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

/// The `contracts` fixture wires, hand-traced from its source (not from the
/// implementation):
///   - `routes::app` registers `POST /payments` -> `handlers::PaymentHandler::create_payment_route`
///     (an axum `.route(path, verb(handler))` call)
///   - `create_payment_route` constructs a `PaymentCreated` event and calls
///     `repo::save_payment`, which runs `sqlx::query!("INSERT INTO payments ...")`
///   - `handlers::PaymentHandler::on_payment_created` takes `PaymentCreated` as a parameter
///     (a Consumes edge, not exercised by this query but present in the graph)
///   - `e2e_tests::creates_payment_route_end_to_end` is a `#[test]` in a different file
///     that calls `create_payment_route` (wrapped in `assert!`, exercising the
///     macro-argument call scanner) — an indirect (2-hop) caller of `save_payment`
///
/// Querying `repo.rs` (which declares `save_payment` and owns the `payments` table
/// reference) should surface the whole chain: DIRECT/INDIRECT callers, the API route and
/// event reachable through them, the table itself, and the one test that exercises it.
#[test]
fn reports_api_events_database_and_tests_for_repo_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/repo.rs");

    assert_eq!(
        report["direct"],
        serde_json::json!(["handlers::PaymentHandler::create_payment_route"])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!(["e2e_tests::creates_payment_route_end_to_end"])
    );
    assert_eq!(report["api"], serde_json::json!(["POST /payments"]));
    assert_eq!(report["events"], serde_json::json!(["PaymentCreated"]));
    assert_eq!(report["database"], serde_json::json!(["payments"]));
    assert_eq!(report["tests"], 1);
}

/// `events.rs` declares `PaymentCreated` but calls nothing itself — its blast radius is
/// entirely "who produces/consumes this event", which only exists because the engine
/// treats `Produces`/`Consumes`/`Reads`/`Writes` edges as reverse-dependency edges too
/// (not just `Calls`/`References`): `create_payment_route` constructs the event
/// (Produces) and `on_payment_created` takes it as a parameter (Consumes), so both are
/// DIRECT; whatever calls a DIRECT symbol is INDIRECT in turn (the e2e test, which calls
/// `create_payment_route`). `create_payment_route` also happens to produce `POST
/// /payments`, so that surfaces as API too — genuinely part of the blast radius: renaming
/// `PaymentCreated` touches the same function that registers that route.
///
/// `database` stays empty and `save_payment` (which does the actual DB write) never
/// appears: `save_payment` is something `create_payment_route` *calls*, not something
/// that calls `create_payment_route` — the reverse-BFS walks dependents backward, not
/// forward through what a dependent happens to call.
#[test]
fn event_declaration_file_reports_producers_and_consumers() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/events.rs");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            "handlers::PaymentHandler::create_payment_route",
            "handlers::PaymentHandler::on_payment_created",
        ])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!(["e2e_tests::creates_payment_route_end_to_end"])
    );
    assert_eq!(report["api"], serde_json::json!(["POST /payments"]));
    assert_eq!(report["events"], serde_json::json!(["PaymentCreated"]));
    assert_eq!(report["database"], serde_json::json!([]));
    assert_eq!(report["tests"], 1);
}
