use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contracts")
}

/// Sends newline-delimited JSON-RPC requests to `impact mcp` over stdin and returns each
/// response line parsed as JSON, in order — a real stdio round-trip against the compiled
/// binary, not a call into the dispatch function directly.
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

/// Extracts the tool result's JSON payload from an MCP `tools/call` response envelope
/// (`result.content[0].text`, itself a JSON string — this project's tool results are
/// always JSON text, not free-form prose).
fn tool_result_json(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result should have text content");
    serde_json::from_str(text).expect("tool result text should be JSON")
}

/// `initialize` and `tools/list` should describe a working server without needing any
/// project indexed first.
#[test]
fn initialize_and_tools_list_describe_the_server() {
    let responses = mcp_round_trip(&[
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05"}
        }),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ]);

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "impact");
    let tool_names: Vec<&str> = responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        tool_names,
        vec!["impact_index", "impact_file", "impact_change"]
    );
}

/// An unrecognized JSON-RPC method is a proper JSON-RPC error, not a crash or a silently
/// dropped request.
#[test]
fn unknown_method_is_a_json_rpc_error() {
    let responses = mcp_round_trip(&[
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "not/a/real/method"}),
    ]);

    assert_eq!(responses[0]["error"]["code"], -32601);
}

/// The three tools, called over the real stdio protocol in sequence (index, then query,
/// then change — exactly the workflow an agent would follow), should produce the same
/// results the CLI's behavior tests already hand-verified for this fixture: `contracts.rs`
/// verified `impact query src/repo.rs` gives this exact DIRECT/API/EVENTS/DATABASE/TESTS
/// shape, and `change.rs` verified `remove repo::save_payment` additionally surfaces
/// `repo::save_payment_persists` (a same-file caller, visible at symbol granularity).
#[test]
fn index_file_and_change_tools_work_over_the_real_protocol() {
    let cache_dir = tempfile::tempdir().unwrap();
    let cache_dir_str = cache_dir.path().to_str().unwrap();
    let project = fixture_path();
    let project_str = project.to_str().unwrap();

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
                "path": "src/repo.rs",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
            }),
        ),
        tool_call(
            3,
            "impact_change",
            serde_json::json!({
                "description": "remove repo::save_payment",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
            }),
        ),
    ]);

    let index_result = tool_result_json(&responses[0]);
    assert_eq!(index_result["files_indexed"], 6);
    assert_eq!(index_result["symbols_indexed"], 10);

    let file_result = tool_result_json(&responses[1]);
    assert_eq!(
        file_result["direct"],
        serde_json::json!([{"path": "handlers::PaymentHandler::create_payment_route", "confidence": "Exact"}])
    );
    assert_eq!(file_result["api"], serde_json::json!(["POST /payments"]));
    assert_eq!(file_result["events"], serde_json::json!(["PaymentCreated"]));
    assert_eq!(file_result["database"], serde_json::json!(["payments"]));
    assert_eq!(file_result["tests"], 1);

    let change_result = tool_result_json(&responses[2]);
    assert_eq!(
        change_result["direct"],
        serde_json::json!([
            {"path": "handlers::PaymentHandler::create_payment_route", "confidence": "Exact"},
            {"path": "repo::save_payment_persists", "confidence": "Exact"},
        ])
    );
    assert_eq!(change_result["tests"], 2);
}

/// A tool-level failure (bad change grammar) comes back as a normal tool result carrying
/// an `error` field, not a JSON-RPC protocol error — the request itself was valid, the
/// requested operation failed, which is what MCP tool errors are for.
#[test]
fn tool_level_error_is_reported_in_the_result_not_the_protocol() {
    let cache_dir = tempfile::tempdir().unwrap();
    let project = fixture_path();

    let responses = mcp_round_trip(&[
        tool_call(
            1,
            "impact_index",
            serde_json::json!({
                "project_path": project.to_str().unwrap(),
                "cache_dir": cache_dir.path().to_str().unwrap(),
            }),
        ),
        tool_call(
            2,
            "impact_change",
            serde_json::json!({
                "description": "please rewrite everything",
                "project_path": project.to_str().unwrap(),
                "cache_dir": cache_dir.path().to_str().unwrap(),
            }),
        ),
    ]);

    assert!(responses[1].get("error").is_none());
    let result = tool_result_json(&responses[1]);
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("could not parse change description"),
        "unexpected result: {result}"
    );
}

/// `impact_file`'s `workspace_path` argument reaches the same cross-project matching the
/// CLI's `workspace.rs` tests already verified in full (all three confidence tiers) —
/// this only needs to prove the MCP-specific plumbing (JSON arg -> `ops::cross_project_report`)
/// isn't dropped or mis-wired, not re-verify the matching logic itself.
#[test]
fn impact_file_workspace_path_reaches_cross_project_matching() {
    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    let cache_b = tempfile::tempdir().unwrap();
    let cache_w = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    mcp_round_trip(&[
        tool_call(
            1,
            "impact_index",
            serde_json::json!({
                "project_path": fixture("workspace_backend").to_str().unwrap(),
                "cache_dir": cache_b.path().to_str().unwrap(),
            }),
        ),
        tool_call(
            2,
            "impact_index",
            serde_json::json!({
                "project_path": fixture("workspace_web").to_str().unwrap(),
                "cache_dir": cache_w.path().to_str().unwrap(),
            }),
        ),
    ]);

    let workspace_toml = ws_dir.path().join("workspace.toml");
    std::fs::write(
        &workspace_toml,
        format!(
            "[[projects]]\nid = \"backend\"\npath = {:?}\ncache_dir = {:?}\n\n[[projects]]\nid = \"web\"\npath = {:?}\ncache_dir = {:?}\n\n[[links]]\nproduces = \"backend:PaymentCreated\"\nconsumes = \"web\"\n",
            fixture("workspace_backend").to_str().unwrap(),
            cache_b.path().to_str().unwrap(),
            fixture("workspace_web").to_str().unwrap(),
            cache_w.path().to_str().unwrap(),
        ),
    )
    .unwrap();

    let responses = mcp_round_trip(&[tool_call(
        1,
        "impact_file",
        serde_json::json!({
            "path": "src/events.rs",
            "project_path": fixture("workspace_backend").to_str().unwrap(),
            "cache_dir": cache_b.path().to_str().unwrap(),
            "workspace_path": workspace_toml.to_str().unwrap(),
        }),
    )]);

    let result = tool_result_json(&responses[0]);
    assert_eq!(
        result["cross_project"],
        serde_json::json!([
            {"project_id": "web", "contract_kind": "Event", "contract_id": "OrderPlaced", "confidence": "weak"},
            {"project_id": "web", "contract_kind": "Event", "contract_id": "PaymentCreated", "confidence": "declared"},
        ])
    );
}

/// `impact_file`'s `min_confidence` argument reaches the same filter the CLI's
/// `confidence.rs` tests already verified (`--min-confidence exact` hides heuristic
/// entries) — this only needs to prove the MCP-specific plumbing isn't dropped or
/// mis-wired, not re-verify the filtering logic itself.
#[test]
fn impact_file_min_confidence_filters_heuristic_entries() {
    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    let cache_dir = tempfile::tempdir().unwrap();
    let project = fixture("confidence");
    let project_str = project.to_str().unwrap();
    let cache_dir_str = cache_dir.path().to_str().unwrap();

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
                "path": "src/target.rs",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
                "min_confidence": "exact",
            }),
        ),
    ]);

    let result = tool_result_json(&responses[1]);
    assert_eq!(
        result["direct"],
        serde_json::json!([{"path": "caller::call_precise", "confidence": "Exact"}])
    );
}
