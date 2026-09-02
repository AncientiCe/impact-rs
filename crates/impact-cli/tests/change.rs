use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

fn enum_variants_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/enum_variants")
}

fn contracts_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contracts")
}

fn index(fixture: &Path, cache_dir: &Path) {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture.to_str().unwrap(), "--json"])
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

/// Builds the JSON shape a report's `direct`/`indirect` entries now carry: `{path, file,
/// line, confidence}` objects. Every entry here resolves unambiguously in its fixture, so
/// `Exact` is the right expected confidence throughout this file.
fn exact(entries: &[(&str, &str, u64)]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|(path, file, line)| {
                serde_json::json!({"path": path, "file": file, "line": line, "confidence": "Exact"})
            })
            .collect(),
    )
}

fn change(fixture: &Path, cache_dir: &Path, description: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["change", description, "--json"])
        .arg("--project")
        .arg(fixture)
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("change --json output should be valid JSON")
}

/// `enum_variants` wires (hand-traced): `status::PaymentStatus` has variants `Pending`
/// and `Failed`; `display::describe` matches on both (a DIRECT dependent of `Failed`);
/// `summary::summarize` calls `describe` *through* a `format!` macro argument — the same
/// macro-token-tree call scanner exercised by the `contracts` fixture's tests — making it
/// an INDIRECT (2-hop) dependent. Removing the variant should surface exactly that chain.
#[test]
fn remove_variant_finds_match_arms_across_files() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&enum_variants_fixture(), cache_dir.path());

    let report = change(
        &enum_variants_fixture(),
        cache_dir.path(),
        "remove variant status::PaymentStatus::Failed",
    );

    assert_eq!(
        report["direct"],
        exact(&[("display::describe", "src/display.rs", 3)])
    );
    assert_eq!(
        report["indirect"],
        exact(&[("summary::summarize", "src/summary.rs", 3)])
    );
}

/// `rename`, `remove`, and `change signature of` all reduce to the same "blast radius of
/// this one resolved symbol" computation (see `ChangeSpec::target_path`) — they differ
/// only in what a human/agent should read into the result, not in what gets computed. All
/// three should report the exact same dependents of `repo::save_payment`, which the
/// `contracts` fixture test already hand-verified: `create_payment_route` (direct),
/// `save_payment_persists` (direct, same-file caller — visible here because the seed is
/// just the one function, not "everything in repo.rs" the way file-mode's seed set is),
/// and the e2e test (indirect).
#[test]
fn rename_remove_and_signature_change_agree_on_the_same_symbol() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&contracts_fixture(), cache_dir.path());

    let expected_direct = exact(&[
        (
            "handlers::PaymentHandler::create_payment_route",
            "src/handlers.rs",
            6,
        ),
        ("repo::save_payment_persists", "src/repo.rs", 7),
    ]);
    let expected_indirect = exact(&[(
        "e2e_tests::creates_payment_route_end_to_end",
        "src/e2e_tests.rs",
        4,
    )]);

    for description in [
        "rename repo::save_payment",
        "rename repo::save_payment to repo::persist_payment",
        "remove repo::save_payment",
        "change signature of repo::save_payment",
    ] {
        let report = change(&contracts_fixture(), cache_dir.path(), description);
        assert_eq!(report["direct"], expected_direct, "for {description:?}");
        assert_eq!(report["indirect"], expected_indirect, "for {description:?}");
    }
}

/// Unparseable input is a hard error with a usage hint — never a best-effort guess at
/// what the user might have meant. Determinism is the whole pitch; free-text NLP would
/// make identical input produce different results across runs.
#[test]
fn unparseable_change_description_is_a_clear_error_not_a_guess() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&contracts_fixture(), cache_dir.path());

    Command::cargo_bin("impact")
        .unwrap()
        .args(["change", "please rewrite everything"])
        .arg("--project")
        .arg(contracts_fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .assert()
        .failure()
        .stderr(contains("could not parse change description"));
}

/// A syntactically valid but unresolvable path (a typo, or a symbol that was renamed
/// since the last index) is also a clear error, not a silently empty report — an empty
/// blast radius should mean "genuinely no dependents," not "we couldn't find it."
#[test]
fn unresolved_change_target_is_a_clear_error() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&contracts_fixture(), cache_dir.path());

    Command::cargo_bin("impact")
        .unwrap()
        .args(["change", "remove this_function_does_not_exist"])
        .arg("--project")
        .arg(contracts_fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .assert()
        .failure()
        .stderr(contains(
            "doesn't resolve to anything in the indexed project",
        ));
}
