use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi_file")
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

fn diff_via_stdin(cache_dir: &Path, diff_text: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["diff", "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .write_stdin(diff_text)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("diff --json output should be valid JSON")
}

/// A single-line hunk touching only the *body* of `PaymentService::charge` (line 5 —
/// `true`/`false`, not `charge`'s own declaration line 4) — this only resolves to a seed
/// via `compute_diff_impact`'s nearest-preceding-declaration fallback, not the "range
/// contains the declaration line" direct case, so it's the case that actually exercises
/// the fallback path.
const CHARGE_BODY_DIFF: &str = "diff --git a/src/payment/service.rs b/src/payment/service.rs\n\
--- a/src/payment/service.rs\n\
+++ b/src/payment/service.rs\n\
@@ -5 +5 @@\n\
-        true\n\
+        false\n";

/// `multi_file` wires `OrderService::checkout` -> `PaymentController::handle` ->
/// `PaymentService::charge` (see `query.rs`'s fixture doc). A diff touching only
/// `charge`'s body (not its declaration line) should still resolve to `charge` as the
/// touched symbol, and report the exact same DIRECT/INDIRECT chain `impact query
/// src/payment/service.rs` reports for the whole file.
#[test]
fn diff_touching_a_function_body_resolves_to_the_enclosing_function() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = diff_via_stdin(cache_dir.path(), CHARGE_BODY_DIFF);

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "payment::controller::PaymentController::handle", "confidence": "Exact"}])
    );
    assert_eq!(
        report["indirect"],
        serde_json::json!([{"path": "order::OrderService::checkout", "confidence": "Exact"}])
    );
}

/// A diff touching `order.rs` (the deepest caller in the chain, nothing calls into it)
/// should report an empty blast radius, same as querying that file directly.
#[test]
fn diff_touching_a_leaf_file_has_no_impact() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let diff = "diff --git a/src/order.rs b/src/order.rs\n\
--- a/src/order.rs\n\
+++ b/src/order.rs\n\
@@ -1,3 +1,3 @@\n\
 use crate::payment::controller::PaymentController;\n\
 \n\
-pub struct OrderService {\n\
+pub struct OrderServiceX {\n";

    let report = diff_via_stdin(cache_dir.path(), diff);

    assert_eq!(report["direct"], serde_json::json!([]));
    assert_eq!(report["indirect"], serde_json::json!([]));
}

/// `--file` should behave identically to piping the same diff over stdin.
#[test]
fn diff_file_flag_reads_from_a_file_instead_of_stdin() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let diff_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(diff_file.path(), CHARGE_BODY_DIFF).unwrap();

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["diff", "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .arg("--file")
        .arg(diff_file.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(
        report["direct"],
        serde_json::json!([{"path": "payment::controller::PaymentController::handle", "confidence": "Exact"}])
    );
}
