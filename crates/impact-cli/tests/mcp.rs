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

/// Same round trip as `mcp_round_trip`, but with extra environment variables set on the
/// `impact mcp` child process — used to point `IMPACT_GH_BIN` at a fake `gh` script
/// without touching this test process's own environment.
fn mcp_round_trip_env(requests: &[Value], envs: &[(&str, &std::path::Path)]) -> Vec<Value> {
    let input = requests
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let mut cmd = Command::cargo_bin("impact").unwrap();
    cmd.arg("mcp");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let output = cmd.write_stdin(input).output().unwrap();
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
        vec![
            "impact_index",
            "impact_file",
            "impact_change",
            "impact_diff",
            "impact_report_blindspot"
        ]
    );
}

/// impact records the calls it can see syntactically, so a call reached through a
/// registry/selector indirection, or a function handed over as a value instead of called,
/// leaves no edge — twice observed producing an empty radius for a symbol that had a real
/// production consumer. Every query tool must say so, or an agent reads "no dependents" as
/// "safe to change".
#[test]
fn query_tool_descriptions_disclose_the_dynamic_dispatch_blind_spot() {
    let responses = mcp_round_trip(&[serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/list"
    })]);

    let tools = responses[0]["result"]["tools"].as_array().unwrap();
    for name in ["impact_file", "impact_change", "impact_diff"] {
        let description = tools
            .iter()
            .find(|t| t["name"] == name)
            .and_then(|t| t["description"].as_str())
            .unwrap_or_else(|| panic!("{name} should be listed with a description"));
        assert!(
            description.contains("grep"),
            "{name} should tell the agent to cross-check by grep: {description}"
        );
        assert!(
            description.contains("as a value"),
            "{name} should disclose that a function passed as a value leaves no edge: {description}"
        );
    }
}

/// The server's `initialize` instructions must tell agents to run impact analysis not
/// just before editing but also when proposing a fix concrete enough to state in
/// `impact_change`'s grammar (rename/remove/signature change), before any code is
/// written — see the impact_rs `todo` Palace memory recorded 2026-09-03.
#[test]
fn initialize_instructions_cover_proposing_a_concrete_fix() {
    let responses = mcp_round_trip(&[serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05"}
    })]);

    let instructions = responses[0]["result"]["instructions"]
        .as_str()
        .expect("initialize result should have instructions");
    assert!(
        instructions.contains("propos"),
        "instructions should mention proposing a fix: {instructions}"
    );
    assert!(
        instructions.contains("vague") || instructions.contains("exploratory"),
        "instructions should still exclude vague/exploratory discussion: {instructions}"
    );
}

/// Same reason the installed rule carries a `## SESSION START` trigger: clients that
/// defer MCP tools behind a tool search never load impact's tools if every trigger is
/// conditional, so the server's own instructions must ask for an index up front rather
/// than only once an edit is already in flight.
#[test]
fn initialize_instructions_ask_for_an_index_at_session_start() {
    let responses = mcp_round_trip(&[serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05"}
    })]);

    let instructions = responses[0]["result"]["instructions"]
        .as_str()
        .expect("initialize result should have instructions");
    assert!(
        instructions.contains("session"),
        "instructions should ask for an index at the start of a session: {instructions}"
    );
}

/// The server's instructions carry the same mechanical checkpoints as the installed rule:
/// an agent should not have to classify its own change correctly before the protocol can
/// fire on it.
#[test]
fn initialize_instructions_state_mechanical_checkpoints() {
    let responses = mcp_round_trip(&[serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05"}
    })]);

    let instructions = responses[0]["result"]["instructions"]
        .as_str()
        .expect("initialize result should have instructions");
    assert!(
        instructions.contains("commit"),
        "instructions should ask for analysis before any commit: {instructions}"
    );
    assert!(
        instructions.contains("first edit"),
        "instructions should ask for analysis before the session's first edit: {instructions}"
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
        serde_json::json!([{"path": "handlers::PaymentHandler::create_payment_route", "file": "src/handlers.rs", "line": 6, "confidence": "Exact"}])
    );
    assert_eq!(file_result["api"], serde_json::json!(["POST /payments"]));
    assert_eq!(file_result["events"], serde_json::json!(["PaymentCreated"]));
    assert_eq!(file_result["database"], serde_json::json!(["payments"]));
    assert_eq!(file_result["tests"], 1);

    let change_result = tool_result_json(&responses[2]);
    assert_eq!(
        change_result["direct"],
        serde_json::json!([
            {"path": "handlers::PaymentHandler::create_payment_route", "file": "src/handlers.rs", "line": 6, "confidence": "Exact"},
            {"path": "repo::save_payment_persists", "file": "src/repo.rs", "line": 7, "confidence": "Exact"},
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
        serde_json::json!([{"path": "caller::call_precise", "file": "src/caller.rs", "line": 12, "confidence": "Exact"}])
    );
}

/// `impact_diff`, called over the real stdio protocol: a diff touching only
/// `PaymentService::charge`'s body (not its declaration line) should resolve to the same
/// DIRECT/INDIRECT chain the CLI's `diff.rs` tests already verified for this exact diff
/// against `multi_file` — this only needs to prove the MCP tool reaches
/// `ops::diff_impact`, not re-verify the diff-to-symbol mapping itself.
#[test]
fn impact_diff_reaches_the_same_diff_to_symbol_mapping_as_the_cli() {
    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    let cache_dir = tempfile::tempdir().unwrap();
    let project = fixture("multi_file");
    let project_str = project.to_str().unwrap();
    let cache_dir_str = cache_dir.path().to_str().unwrap();
    let diff = "diff --git a/src/payment/service.rs b/src/payment/service.rs\n\
--- a/src/payment/service.rs\n\
+++ b/src/payment/service.rs\n\
@@ -5 +5 @@\n\
-        true\n\
+        false\n";

    let responses = mcp_round_trip(&[
        tool_call(
            1,
            "impact_index",
            serde_json::json!({"project_path": project_str, "cache_dir": cache_dir_str}),
        ),
        tool_call(
            2,
            "impact_diff",
            serde_json::json!({
                "diff": diff,
                "project_path": project_str,
                "cache_dir": cache_dir_str,
            }),
        ),
    ]);

    let result = tool_result_json(&responses[1]);
    assert_eq!(
        result["direct"],
        serde_json::json!([{"path": "payment::controller::PaymentController::handle", "file": "src/payment/controller.rs", "line": 8, "confidence": "Exact"}])
    );
    assert_eq!(
        result["indirect"],
        serde_json::json!([{"path": "order::OrderService::checkout", "file": "src/order.rs", "line": 8, "confidence": "Exact"}])
    );
}

/// `impact_file`'s `explain` argument reaches the same `apply_explain` the CLI's
/// `query.rs` tests already verified — without it, an INDIRECT entry's `via` field is
/// absent entirely; with it, `checkout`'s `via` chain names its one DIRECT dependent.
#[test]
fn impact_file_explain_populates_indirect_via_chain() {
    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    let cache_dir = tempfile::tempdir().unwrap();
    let project = fixture("multi_file");
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
                "path": "src/payment/service.rs",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
            }),
        ),
        tool_call(
            3,
            "impact_file",
            serde_json::json!({
                "path": "src/payment/service.rs",
                "project_path": project_str,
                "cache_dir": cache_dir_str,
                "explain": true,
            }),
        ),
    ]);

    let without_explain = tool_result_json(&responses[1]);
    assert_eq!(without_explain["indirect"][0].get("via"), None);

    let with_explain = tool_result_json(&responses[2]);
    assert_eq!(
        with_explain["indirect"][0]["via"],
        serde_json::json!(["payment::controller::PaymentController::handle"])
    );
}

/// What the fake `gh issue create` invocation should do — `Some(url)` "succeeds" and
/// prints it, `None` means creation must never be invoked in this test.
type FakeGhCreate<'a> = Option<&'a str>;

/// Points `impact` at `crates/impact-cli/src/bin/fake_gh.rs` (a real compiled process,
/// not a `/bin/sh`/`cmd.exe` script — see that file's module doc for why a script can't
/// do this job on Windows) and writes the JSON config file it reads its behavior from —
/// same technique as `tests/blindspot.rs`'s CLI-level version, duplicated here because
/// each integration test file is its own crate. `issue list` answers with `list_json`;
/// `issue create` behaves per `create`. Returns `(gh_bin, config_path)` — the caller sets
/// both as env vars (`IMPACT_GH_BIN`, `FAKE_GH_CONFIG`) on the `impact mcp` invocation.
fn write_fake_gh(
    dir: &Path,
    list_json: &str,
    create: FakeGhCreate,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let gh_bin = std::path::PathBuf::from(std::env::var("CARGO_BIN_EXE_fake_gh").unwrap());
    let config_path = dir.join("fake-gh-config.json");
    let config = serde_json::json!({"list_json": list_json, "create_url": create});
    std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    (gh_bin, config_path)
}

/// `impact_report_blindspot` files a public GitHub issue on the user's behalf, but the
/// tool is run against private repos — an agent that pastes real local paths, repo
/// names, or internal symbol names into the draft leaks workspace details onto a public
/// tracker. The tool's own description and its `body` field must tell the agent to
/// rewrite evidence generically before drafting, not just describe what the field holds.
#[test]
fn report_blindspot_tool_requires_redaction() {
    let responses = mcp_round_trip(&[serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/list"
    })]);

    let tools = responses[0]["result"]["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|t| t["name"] == "impact_report_blindspot")
        .expect("impact_report_blindspot should be listed");

    let description = tool["description"].as_str().unwrap();
    assert!(
        description.contains("public") && description.contains("redact"),
        "description should warn this files a public issue and demand redaction: {description}"
    );

    let body_description = tool["inputSchema"]["properties"]["body"]["description"]
        .as_str()
        .unwrap();
    assert!(
        body_description.contains("redact") || body_description.contains("generic"),
        "body field should require generic, redacted evidence: {body_description}"
    );
    assert!(
        body_description.contains("path") || body_description.contains("repo name"),
        "body field should call out what to strip (local paths, private repo/symbol names): {body_description}"
    );
}

/// `impact_report_blindspot` without `submit` never touches the network — it only
/// returns the composed draft, always with `submitted: false`.
#[test]
fn report_blindspot_dry_run_returns_a_draft() {
    let responses = mcp_round_trip(&[tool_call(
        1,
        "impact_report_blindspot",
        serde_json::json!({
            "title": "misses an indirect call",
            "body": "impact_file reported no callers, but grep found one.",
            "kind": "missed-edge",
            "language": "swift",
        }),
    )]);

    let result = tool_result_json(&responses[0]);
    assert_eq!(result["title"], "misses an indirect call");
    assert_eq!(result["repo"], "AncientiCe/impact-rs");
    assert_eq!(result["submitted"], false);
    assert!(result["body"]
        .as_str()
        .unwrap()
        .contains("impact_file reported no callers"));
    assert_eq!(result["fingerprint"].as_str().unwrap().len(), 16);
}

/// `submit: true` with no existing match files a new issue via the (faked) `gh`.
#[test]
fn report_blindspot_submit_files_a_new_issue() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, gh_config) = write_fake_gh(
        dir.path(),
        "[]",
        Some("https://github.com/AncientiCe/impact-rs/issues/999"),
    );

    let responses = mcp_round_trip_env(
        &[tool_call(
            1,
            "impact_report_blindspot",
            serde_json::json!({"title": "t", "body": "b", "submit": true}),
        )],
        &[
            ("IMPACT_GH_BIN", gh.as_path()),
            ("FAKE_GH_CONFIG", gh_config.as_path()),
        ],
    );

    let result = tool_result_json(&responses[0]);
    assert_eq!(result["submitted"], true);
    assert_eq!(
        result["url"],
        "https://github.com/AncientiCe/impact-rs/issues/999"
    );
}

/// `submit: true` with a matching fingerprint already on file reports that issue's URL
/// and never calls `gh issue create` (the fake `gh` fails loudly if it does).
#[test]
fn report_blindspot_submit_skips_a_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let (gh, gh_config) = write_fake_gh(
        dir.path(),
        r#"[{"number":7,"url":"https://github.com/AncientiCe/impact-rs/issues/7","title":"existing"}]"#,
        None,
    );

    let responses = mcp_round_trip_env(
        &[tool_call(
            1,
            "impact_report_blindspot",
            serde_json::json!({"title": "t", "body": "b", "submit": true}),
        )],
        &[
            ("IMPACT_GH_BIN", gh.as_path()),
            ("FAKE_GH_CONFIG", gh_config.as_path()),
        ],
    );

    let result = tool_result_json(&responses[0]);
    assert_eq!(result["submitted"], false);
    assert_eq!(
        result["existing_url"],
        "https://github.com/AncientiCe/impact-rs/issues/7"
    );
}
