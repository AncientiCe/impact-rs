use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust_imports")
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

fn query(cache_dir: &Path, file: &str, min_confidence: Option<&str>) -> Value {
    let mut cmd = Command::cargo_bin("impact").unwrap();
    cmd.args(["query", file, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir);
    if let Some(min) = min_confidence {
        cmd.args(["--min-confidence", min]);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

/// `(path, confidence)` for each direct dependent, which is what these tests are about.
fn direct(report: &Value) -> Vec<(String, String)> {
    report["direct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_string(),
                d["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// `words::count_words` calls `Vec::len` from the standard library. `Basket::len` is the
/// only `len` declared in this fixture, and that used to be enough to report the call as
/// an `Exact` dependency — a confident, completely wrong answer that `--min-confidence
/// exact` did nothing to hide. A method call on a receiver whose type the adapter can't
/// determine is now `probable` at best: still reported (a missed caller is worse than a
/// noisy one), but no longer dressed up as certain.
#[test]
fn method_call_on_an_untyped_receiver_is_never_exact() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/basket.rs", None);
    assert_eq!(
        direct(&report),
        vec![("words::count_words".to_string(), "Probable".to_string())]
    );

    let filtered = query(cache_dir.path(), "src/basket.rs", Some("exact"));
    assert!(
        filtered["direct"].as_array().unwrap().is_empty(),
        "min-confidence exact must not surface a std-library call, got: {:?}",
        filtered["direct"]
    );
}

/// The other half of the same change: a call the adapter *can* tie to a `use` stays
/// `Exact`. `runner::run` imports `validate` by name, so there is real evidence — not
/// just a matching name — that the call lands there.
#[test]
fn imported_free_function_call_stays_exact() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/validate.rs", None);
    assert_eq!(
        direct(&report),
        vec![("runner::run".to_string(), "Exact".to_string())]
    );
}

/// Two more shapes that carry real evidence and must not be downgraded along with the
/// untyped ones: `self.service.charge()`, where the receiver is a struct field whose
/// declared type is imported in the same file, and `PaymentService::charge_static()`,
/// where the type is named at the call site.
#[test]
fn field_typed_and_type_qualified_calls_stay_exact() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = query(cache_dir.path(), "src/service.rs", None);
    assert_eq!(
        direct(&report),
        vec![
            (
                "controller::Controller::handle".to_string(),
                "Exact".to_string()
            ),
            ("factory::build".to_string(), "Exact".to_string()),
        ]
    );
}
