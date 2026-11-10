use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/go_contracts")
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

/// `go_contracts` wires (hand-traced from the source): `routes.go`'s `registerRoutes`
/// calls `mux.HandleFunc("POST /payments", createPaymentRoute)` — Go 1.22+'s
/// method-prefixed pattern shape, the one form `impact-lang-go`'s `net/http` detector
/// recognizes — and separately `mux.HandleFunc("/legacy", legacyHandler)`, a pattern with
/// no method prefix, which the detector deliberately does not recognize (no verb to
/// report). `handlers.go`'s `createPaymentRoute` calls `repo.go`'s `savePayment`.
///
/// Querying `repo.go` should surface `createPaymentRoute` as its one DIRECT caller, and —
/// because that caller also produces `POST /payments` — the API route in the report too.
/// `legacyHandler`'s unprefixed route never appears anywhere: no verb prefix means no
/// `ContractRef` was ever emitted for it.
#[test]
fn net_http_method_prefixed_route_is_detected_unprefixed_is_not() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "repo.go");

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "handlers::createPaymentRoute", "file": "handlers.go", "line": 3, "confidence": "Exact"}])
    );
    assert_eq!(report["api"], serde_json::json!(["POST /payments"]));
}

/// Hand-verified SQL literals in sql.go: Save writes audit_entries; load reads it
/// and accounts; update writes accounts; remove deletes expired_entries. QueryRow
/// and Context variants read the same tables. Comments, SQL string values, bind
/// arguments, dynamic query variables, and logging are not table references.
#[test]
fn go_sql_literals_surface_database_tables_in_file_and_change_reports() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());
    let report = query(cache_dir.path(), "sql.go");
    assert_eq!(
        report["database"],
        serde_json::json!(["accounts", "audit_entries", "expired_entries"])
    );

    for (symbol, tables) in [
        ("Repository::Save", vec!["audit_entries"]),
        ("load", vec!["accounts", "audit_entries"]),
        ("update", vec!["accounts"]),
        ("remove", vec!["expired_entries"]),
        ("count", vec!["accounts"]),
        ("first", vec!["audit_entries"]),
        ("contextual", vec!["accounts"]),
        ("ignored", vec![]),
    ] {
        let output = Command::cargo_bin("impact")
            .unwrap()
            .args([
                "change",
                &format!("change signature of sql::{symbol}"),
                "--json",
            ])
            .arg("--project")
            .arg(fixture_path())
            .arg("--cache-dir")
            .arg(cache_dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["database"], serde_json::json!(tables), "{symbol}");
    }
}

#[test]
fn quoted_sql_identifiers_are_tables_but_never_keywords() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());
    assert_eq!(
        query(cache_dir.path(), "quoted_sql.go")["database"],
        serde_json::json!(["public.quoted_entries"])
    );
}
