//! A query that can't find its index should say so, not fail somewhere else
//! ([#12](https://github.com/AncientiCe/impact-rs/issues/12)). An MCP client often starts
//! `impact mcp` with `/` as its working directory, so `impact_file` without `project_path`
//! used to resolve the project to `/` and fail with "creating cache directory /.impact",
//! which reads like a disk permission problem rather than a missing argument.

use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;

/// Runs `impact mcp` with `cwd` as its working directory, so a test can check that a failed
/// call left nothing behind there.
fn mcp_in(cwd: &Path, requests: &[Value]) -> Vec<Value> {
    let input = requests
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let output = Command::cargo_bin("impact")
        .unwrap()
        .arg("mcp")
        .current_dir(cwd)
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

fn tool_error(response: &Value) -> String {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should have text content");
    let payload: Value = serde_json::from_str(text).expect("tool result text should be JSON");
    payload["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error, got {payload}"))
        .to_string()
}

/// A project directory with one source file and no index.
fn unindexed_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "pub fn run() {}\n").unwrap();
    dir
}

/// `impact_file`, `impact_change` and `impact_diff` all reject a missing `project_path`
/// the way `impact_index` does, instead of falling back to the server's working directory.
#[test]
fn query_tools_require_project_path() {
    let cwd = tempfile::tempdir().unwrap();
    let responses = mcp_in(
        cwd.path(),
        &[
            tool_call(1, "impact_file", serde_json::json!({"path": "lib.rs"})),
            tool_call(
                2,
                "impact_change",
                serde_json::json!({"description": "rename run"}),
            ),
            tool_call(3, "impact_diff", serde_json::json!({"diff": ""})),
        ],
    );

    for response in &responses {
        assert_eq!(tool_error(response), "project_path is required");
    }
    assert!(
        !cwd.path().join(".impact").exists(),
        "a rejected call must not create a cache in the server's working directory"
    );
}

/// The query tools' schemas list `project_path` as required, so a client can validate the
/// call before sending it.
#[test]
fn query_tool_schemas_mark_project_path_required() {
    let cwd = tempfile::tempdir().unwrap();
    let responses = mcp_in(
        cwd.path(),
        &[serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})],
    );

    for tool in responses[0]["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if name == "impact_report_blindspot" {
            continue;
        }
        let required: Vec<&str> = tool["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            required.contains(&"project_path"),
            "{name} should require project_path, requires {required:?}"
        );
    }
}

/// Querying a project that was never indexed says to index it first, rather than creating
/// an empty cache and reporting no dependents.
#[test]
fn query_tools_on_an_unindexed_project_say_to_index_first() {
    let project = unindexed_project();
    let project_str = project.path().to_str().unwrap();
    let responses = mcp_in(
        project.path(),
        &[
            tool_call(
                1,
                "impact_file",
                serde_json::json!({"path": "lib.rs", "project_path": project_str}),
            ),
            tool_call(
                2,
                "impact_change",
                serde_json::json!({"description": "rename run", "project_path": project_str}),
            ),
            tool_call(
                3,
                "impact_diff",
                serde_json::json!({"diff": "", "project_path": project_str}),
            ),
        ],
    );

    for response in &responses {
        let error = tool_error(response);
        assert!(
            error.contains("has not been indexed") && error.contains("impact_index"),
            "unexpected error: {error}"
        );
    }
    assert!(
        !project.path().join(".impact").exists(),
        "a query must not create a cache for a project that was never indexed"
    );
}

/// The CLI keeps defaulting `--project` to the current directory, but gets the same
/// "index it first" error for an unindexed project.
#[test]
fn cli_query_on_an_unindexed_project_says_to_index_first() {
    let project = unindexed_project();
    Command::cargo_bin("impact")
        .unwrap()
        .args(["query", "lib.rs"])
        .current_dir(project.path())
        .assert()
        .failure()
        .stderr(contains("has not been indexed"))
        .stderr(contains("impact index"));
    assert!(!project.path().join(".impact").exists());
}
