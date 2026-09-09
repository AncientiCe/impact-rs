//! Behavior tests for the `--summary`/`summary: true` compact output mode (CLI `--json`
//! and MCP), across all three query types (`query`/`impact_file`, `change`/`impact_change`,
//! `diff`/`impact_diff`).
//!
//! `summary_mode` (see `tests/fixtures/summary_mode`) wires (hand-traced): `core::hub` has
//! two DIRECT callers (`direct::direct_one`, `direct::direct_two`), which together have
//! seven INDIRECT (2-hop) callers split across two files — `many_callers.rs` (5: `caller_1`
//! through `caller_5`) and `other_callers.rs` (2: `caller_a`, `caller_b`). With the default
//! per-file inline-show limit of 3, `many_callers.rs`'s group should report `count: 5` but
//! only 3 entries in `shown`; `other_callers.rs`'s group (`count: 2`) should show both.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/summary_mode")
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

/// Asserts the shape common to every query type's `--summary --json` output: exact
/// per-category counts, DIRECT listed in full, and INDIRECT grouped/counted by file with
/// the documented per-group truncation.
fn assert_summary_shape(report: &Value) {
    assert_eq!(
        report["counts"],
        serde_json::json!({
            "direct": 2,
            "indirect": 7,
            "api": 0,
            "events": 0,
            "database": 0,
            "tests": 0,
        }),
        "counts should reflect the true totals, not the truncated shown-entry counts: {report}"
    );

    let direct = report["direct"]
        .as_array()
        .expect("direct should be an array");
    assert_eq!(direct.len(), 2, "DIRECT should be listed in full: {report}");
    let direct_paths: Vec<&str> = direct.iter().map(|d| d["path"].as_str().unwrap()).collect();
    assert!(direct_paths.contains(&"direct::direct_one"));
    assert!(direct_paths.contains(&"direct::direct_two"));

    let groups = report["indirect_by_file"]
        .as_array()
        .expect("indirect_by_file should be an array");
    assert_eq!(
        groups.len(),
        2,
        "INDIRECT should be grouped by file: {report}"
    );

    let many = groups
        .iter()
        .find(|g| g["file"] == "src/many_callers.rs")
        .unwrap_or_else(|| panic!("no group for src/many_callers.rs in {report}"));
    assert_eq!(many["count"], 5, "many_callers.rs has 5 indirect callers");
    let many_shown = many["shown"].as_array().unwrap();
    assert_eq!(
        many_shown.len(),
        3,
        "a 5-entry group should be truncated to the default per-file limit of 3: {report}"
    );

    let other = groups
        .iter()
        .find(|g| g["file"] == "src/other_callers.rs")
        .unwrap_or_else(|| panic!("no group for src/other_callers.rs in {report}"));
    assert_eq!(other["count"], 2, "other_callers.rs has 2 indirect callers");
    let other_shown = other["shown"].as_array().unwrap();
    assert_eq!(
        other_shown.len(),
        2,
        "a 2-entry group is under the limit, so nothing should be truncated: {report}"
    );
}

#[test]
fn query_summary_groups_indirect_by_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&fixture(), cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/core.rs", "--summary", "--json"])
        .arg("--project")
        .arg(fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query --summary failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("query --summary --json output should be valid JSON");
    assert_summary_shape(&report);
}

#[test]
fn change_summary_groups_indirect_by_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&fixture(), cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args([
            "change",
            "change signature of core::hub",
            "--summary",
            "--json",
        ])
        .arg("--project")
        .arg(fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact change --summary failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("change --summary --json output should be valid JSON");
    assert_summary_shape(&report);
}

#[test]
fn diff_summary_groups_indirect_by_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&fixture(), cache_dir.path());

    // Touches `core::hub`'s own body — the diff-mode seed — so the resulting blast radius
    // is identical to querying `src/core.rs` directly.
    let diff = "--- a/src/core.rs\n\
                +++ b/src/core.rs\n\
                @@ -1,3 +1,3 @@\n\
                 pub fn hub() -> bool {\n\
                -    true\n\
                +    true // changed\n\
                 }\n";

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["diff", "--summary", "--json"])
        .arg("--project")
        .arg(fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .write_stdin(diff)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact diff --summary failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("diff --summary --json output should be valid JSON");
    assert_summary_shape(&report);
}

/// Without `--summary`, output is unchanged: the full report shape, no `counts` or
/// `indirect_by_file` keys — summary mode is opt-in, not a silent default change.
#[test]
fn without_summary_flag_output_is_the_full_report() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&fixture(), cache_dir.path());

    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "src/core.rs", "--json"])
        .arg("--project")
        .arg(fixture())
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.get("counts").is_none());
    assert!(report.get("indirect_by_file").is_none());
    assert_eq!(report["indirect"].as_array().unwrap().len(), 7);
}

/// Sends newline-delimited JSON-RPC requests to `impact mcp` over stdin and returns each
/// response line parsed as JSON — the same real stdio round-trip `tests/mcp.rs` uses,
/// duplicated here (rather than shared) since integration test binaries in this crate each
/// compile as their own standalone crate with no shared `tests/` module.
fn mcp_round_trip(requests: &[Value]) -> Vec<Value> {
    let input = requests
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let output = Command::cargo_bin("impact")
        .unwrap()
        .arg("mcp")
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact mcp exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("bad JSON line {line:?}: {e}"))
        })
        .collect()
}

fn tool_call(id: i64, name: &str, arguments: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
}

fn tool_result_json(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should have text content");
    serde_json::from_str(text).expect("tool result text should be JSON")
}

/// The `impact_file` MCP tool's `summary: true` argument reaches the same
/// `impact_core::summarize` path the CLI's `--summary` flag does — this only needs to prove
/// the MCP-specific plumbing (JSON arg -> `summarize`), since `assert_summary_shape` above
/// already hand-verified the shape itself against all three query types at the CLI level.
#[test]
fn impact_file_tool_summary_argument_groups_indirect_by_file() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(&fixture(), cache_dir.path());
    let project_str = fixture().to_str().unwrap().to_string();
    let cache_dir_str = cache_dir.path().to_str().unwrap().to_string();

    let responses = mcp_round_trip(&[
        tool_call(
            1,
            "impact_index",
            serde_json::json!({"project_path": project_str, "cache_dir": cache_dir_str}),
        ),
        tool_call(
            2,
            "impact_file",
            serde_json::json!({
                "path": "src/core.rs",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
                "summary": true,
            }),
        ),
    ]);

    let report = tool_result_json(&responses[1]);
    assert_summary_shape(&report);
}
