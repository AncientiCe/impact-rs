use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::Value;

/// What the fake `gh issue create` invocation should do — `Some(url)` "succeeds" and
/// prints it, `None` means creation must never be invoked in this test.
type FakeGhCreate<'a> = Option<&'a str>;

/// Points `impact` at `crates/impact-cli/src/bin/fake_gh.rs` (a real compiled process,
/// not a `/bin/sh`/`cmd.exe` script — see that file's module doc for why a script can't
/// do this job on Windows) and writes the JSON config file it reads its behavior from:
/// `issue list` answers with `list_json`; `issue create` behaves per `create`. Returns
/// `(gh_bin, config_path)` — the caller sets both as env vars (`IMPACT_GH_BIN`,
/// `FAKE_GH_CONFIG`) on the `impact` invocation under test.
fn write_fake_gh(dir: &Path, list_json: &str, create: FakeGhCreate) -> (PathBuf, PathBuf) {
    let gh_bin = PathBuf::from(std::env::var("CARGO_BIN_EXE_fake_gh").unwrap());
    let config_path = dir.join("fake-gh-config.json");
    let config = serde_json::json!({"list_json": list_json, "create_url": create});
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    (gh_bin, config_path)
}

/// `impact report-blindspot` never touches the network by default — it only composes
/// and prints the draft. This asserts the human-readable rendering carries every field
/// a caller supplied, plus the "dry run" notice telling them nothing was sent.
#[test]
fn dry_run_prints_a_draft_with_no_network_notice() {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "report-blindspot",
            "misses an indirect call",
            "--body",
            "impact_file reported no callers, but grep found one via a registry.",
            "--kind",
            "missed-edge",
            "--language",
            "swift",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact report-blindspot failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(contains("dry run").eval(&stdout), "got:\n{stdout}");
    assert!(
        contains("title: misses an indirect call").eval(&stdout),
        "got:\n{stdout}"
    );
    assert!(
        contains("repo:  AncientiCe/impact-rs").eval(&stdout),
        "got:\n{stdout}"
    );
    assert!(
        contains("**Kind:** missed-edge").eval(&stdout),
        "got:\n{stdout}"
    );
    assert!(
        contains("**Language:** swift").eval(&stdout),
        "got:\n{stdout}"
    );
    assert!(
        contains("impact_file reported no callers").eval(&stdout),
        "got:\n{stdout}"
    );
    assert!(
        contains("<!-- impact-blindspot-fp:").eval(&stdout),
        "got:\n{stdout}"
    );
}

/// `--json` gives back a machine-readable draft, always with `submitted: false` — this
/// subcommand alone never files anything.
#[test]
fn json_output_has_the_expected_shape() {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "report-blindspot",
            "reports a caller that doesn't exist",
            "--body",
            "body text",
            "--kind",
            "false-positive",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value["title"], "reports a caller that doesn't exist");
    assert_eq!(value["repo"], "AncientiCe/impact-rs");
    assert_eq!(value["submitted"], false);
    assert!(value["body"].as_str().unwrap().contains("body text"));
    assert_eq!(value["fingerprint"].as_str().unwrap().len(), 16);
}

/// Without `--body`, the description is read from stdin — same fallback shape as
/// `impact diff` reading a diff from stdin when `--file` is omitted.
#[test]
fn body_falls_back_to_stdin_when_omitted() {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["report-blindspot", "crashes on empty file", "--json"])
        .write_stdin("panics in the tree-sitter adapter on a zero-byte file")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(value["body"]
        .as_str()
        .unwrap()
        .contains("panics in the tree-sitter adapter"));
}

/// `--kind` and `--repo` both have sane defaults so a minimal report still works.
#[test]
fn kind_and_repo_default_when_omitted() {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["report-blindspot", "something odd", "--body", "b", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(value["body"].as_str().unwrap().contains("**Kind:** other"));
    assert_eq!(value["repo"], "AncientiCe/impact-rs");
}

/// `--submit` with no matching existing issue runs `gh issue create` and reports the
/// new issue's URL.
#[test]
fn submit_files_a_new_issue_when_none_exists_yet() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, gh_config) = write_fake_gh(
        dir.path(),
        "[]",
        Some("https://github.com/AncientiCe/impact-rs/issues/999"),
    );

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "report-blindspot",
            "crashes on empty file",
            "--body",
            "b",
            "--submit",
            "--json",
        ])
        .env("IMPACT_GH_BIN", &gh)
        .env("FAKE_GH_CONFIG", &gh_config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact report-blindspot --submit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value["submitted"], true);
    assert_eq!(
        value["url"],
        "https://github.com/AncientiCe/impact-rs/issues/999"
    );
}

/// `--submit` checks for an existing issue with the same fingerprint first — a match
/// means it reports that issue's URL and never calls `gh issue create` at all (the fake
/// `gh` fails loudly if `create` is invoked, so a passing test proves it wasn't).
#[test]
fn submit_finds_an_existing_report_and_skips_creating_a_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, gh_config) = write_fake_gh(
        dir.path(),
        r#"[{"number":42,"url":"https://github.com/AncientiCe/impact-rs/issues/42","title":"existing"}]"#,
        None,
    );

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "report-blindspot",
            "crashes on empty file",
            "--body",
            "b",
            "--submit",
            "--json",
        ])
        .env("IMPACT_GH_BIN", &gh)
        .env("FAKE_GH_CONFIG", &gh_config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact report-blindspot --submit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value["submitted"], false);
    assert_eq!(
        value["existing_url"],
        "https://github.com/AncientiCe/impact-rs/issues/42"
    );
}

/// A missing/broken `gh` binary is a clean, contextualized error — not a panic.
#[test]
fn submit_fails_cleanly_when_gh_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let missing_gh = dir.path().join("does-not-exist");

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["report-blindspot", "title", "--body", "b", "--submit"])
        .env("IMPACT_GH_BIN", &missing_gh)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        contains("gh issue list").eval(&stderr) || contains("gh` installed").eval(&stderr),
        "got:\n{stderr}"
    );
}
