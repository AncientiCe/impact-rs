use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::Value;

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
