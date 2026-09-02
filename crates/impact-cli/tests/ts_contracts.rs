use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_contracts")
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

/// `ts_contracts` wires (hand-traced from the source): `routes.ts` registers
/// `app.get("/payments", createPaymentRoute)` — a named-handler-reference registration,
/// the one shape `impact-lang-ts`'s Express/Fastify detector recognizes — and separately
/// `app.get("/anonymous", (req, res) => {...})`, an inline arrow-function handler with no
/// name to report, which the detector deliberately does not recognize.
/// `handlers.ts`'s `createPaymentRoute` calls `repo.ts`'s `savePayment`.
///
/// Querying `repo.ts` should surface `createPaymentRoute` as its one DIRECT caller, and
/// — because that caller also produces `GET /payments` — the API route in the report
/// too. The anonymous handler's route never appears anywhere: no named handler means no
/// `ContractRef` was ever emitted for it.
#[test]
fn express_named_handler_route_is_detected_inline_handler_is_not() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "repo.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "handlers::createPaymentRoute", "file": "handlers.ts", "line": 3, "confidence": "Exact"}])
    );
    assert_eq!(report["api"], serde_json::json!(["GET /payments"]));
}
