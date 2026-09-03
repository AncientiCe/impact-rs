//! MCP server — stdio JSON-RPC tool loop for AI coding agents.
//!
//! Install: `impact mcp`
//!
//! Hand-rolled, not built on an MCP SDK crate — matches this project's sibling tool
//! `palace-rs` (`mempalace-rs/src/mcp_server.rs`): line-delimited JSON-RPC 2.0 over
//! stdin/stdout, dispatched by hand. Minimal dependency surface, full control over the
//! protocol loop.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};

use crate::ops;

pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {e}")}
                });
                let mut out = stdout.lock();
                writeln!(out, "{err}")?;
                out.flush()?;
                continue;
            }
        };

        if let Some(response) = handle_request(&request) {
            let mut out = stdout.lock();
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }

    Ok(())
}

fn initialize_result(protocol_version: &str) -> Value {
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "impact", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Before modifying code, call impact_index once per project, then \
            impact_file, impact_change, or impact_diff (given a unified diff) to see what \
            depends on what you're about to change — direct/indirect callers, API routes, \
            event types, database tables, and affected tests. This also applies before \
            proposing a fix: once the proposed fix is concrete enough to state as a \
            rename/remove/signature-change target, run impact analysis before presenting \
            the proposal, even if no code has been written yet — vague, exploratory \
            discussion that hasn't settled on a concrete target doesn't need it. Re-run \
            impact_index after the project changes; results are only as fresh as the \
            last index.",
    })
}

fn handle_request(req: &Value) -> Option<String> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_default();
    let req_id = req.get("id").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            let protocol_version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            Some(initialize_result(protocol_version))
        }
        "notifications/initialized" => return None,
        "tools/list" => Some(json!({"tools": tool_list()})),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();
            let result = dispatch_tool(tool_name, &args);
            Some(json!({
                "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default()}]
            }))
        }
        _ => {
            return Some(
                json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32601, "message": format!("Unknown method: {method}")}
                })
                .to_string(),
            )
        }
    };

    result.map(|r| {
        json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": r,
        })
        .to_string()
    })
}

fn tool_list() -> Value {
    json!([
        {
            "name": "impact_index",
            "description": "Index a project (or re-index it) so impact_file and impact_change have something to query. Call this once per project before the first query, and again whenever the project has changed since the last index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": {"type": "string", "description": "Project root to index"},
                    "cache_dir": {"type": "string", "description": "Where to store the index cache (defaults to <project_path>/.impact)"},
                    "force": {"type": "boolean", "description": "Wipe the existing cache and fully re-index, ignoring content-hash skips (default: false)"}
                },
                "required": ["project_path"]
            }
        },
        {
            "name": "impact_file",
            "description": "Report the blast radius of changing a file: direct callers, indirect (transitive) callers, API routes, event types, and database tables the affected code touches, plus a count of affected tests. Each caller is tagged with a confidence tier (exact or heuristic) reflecting how unambiguously the linker resolved it — use min_confidence to hide heuristic (short-name-ambiguous) matches. With workspace_path, also reports which sibling projects registered there are touched by the same API routes/events/tables. Requires impact_index to have run first (and, for cross-project results, the sibling projects to have been indexed too).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File to compute the blast radius for, relative to project_path or absolute"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything)"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"}
                },
                "required": ["path"]
            }
        },
        {
            "name": "impact_change",
            "description": "Report the deterministic blast radius of a specific change, described in impact's small fixed grammar: \"rename <path>\", \"rename <path> to <path>\", \"remove <path>\", \"remove variant <Enum>::<Variant>\", \"remove field <Type>.<field>\", or \"change signature of <path>\". Not natural language — an unrecognized description is a hard error, never a best-effort guess. Each caller in the result is tagged with a confidence tier (exact or heuristic); use min_confidence to hide heuristic matches. With workspace_path, also reports cross-project impact like impact_file does. Requires impact_index to have run first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "The change, in impact's deterministic grammar"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything)"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"}
                },
                "required": ["description"]
            }
        },
        {
            "name": "impact_diff",
            "description": "Report the combined blast radius of a unified diff (e.g. `git diff` output) — every symbol the diff's touched lines fall inside, across every file it mentions. Useful for checking the blast radius of a change you're about to apply (or already have, uncommitted) in one call instead of one impact_file call per touched file. Requires the project to be indexed against the diff's new side — i.e. the working tree as it currently stands. Each caller in the result is tagged with a confidence tier (exact or heuristic); use min_confidence to hide heuristic matches. With workspace_path, also reports cross-project impact like impact_file does.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "diff": {"type": "string", "description": "Unified diff text, e.g. the output of `git diff`"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything)"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"}
                },
                "required": ["diff"]
            }
        }
    ])
}

fn dispatch_tool(name: &str, args: &Value) -> Value {
    match name {
        "impact_index" => tool_index(args),
        "impact_file" => tool_file(args),
        "impact_change" => tool_change(args),
        "impact_diff" => tool_diff(args),
        other => json!({"error": format!("Unknown tool: {other}")}),
    }
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn path_arg(args: &Value, key: &str) -> Option<PathBuf> {
    str_arg(args, key).map(PathBuf::from)
}

/// Parses the optional `min_confidence` tool argument, rejecting anything other than the
/// two documented values with a clear error rather than silently ignoring a typo.
fn min_confidence_arg(args: &Value) -> Result<Option<impact_core::Confidence>, String> {
    match args.get("min_confidence").and_then(|v| v.as_str()) {
        None => Ok(None),
        Some("exact") => Ok(Some(impact_core::Confidence::Exact)),
        Some("heuristic") => Ok(Some(impact_core::Confidence::Heuristic)),
        Some(other) => Err(format!(
            "min_confidence must be \"exact\" or \"heuristic\", got {other:?}"
        )),
    }
}

fn apply_min_confidence(
    report: impact_core::ImpactReport,
    min: Option<impact_core::Confidence>,
) -> impact_core::ImpactReport {
    match min {
        Some(min) => impact_core::filter_min_confidence(report, min),
        None => report,
    }
}

fn explain_arg(args: &Value) -> bool {
    args.get("explain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn ok_or_error<T: serde::Serialize>(result: anyhow::Result<T>) -> Value {
    match result {
        Ok(value) => {
            serde_json::to_value(value).unwrap_or_else(|e| json!({"error": e.to_string()}))
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn tool_index(args: &Value) -> Value {
    let Some(project_path) = path_arg(args, "project_path") else {
        return json!({"error": "project_path is required"});
    };
    let cache_dir = path_arg(args, "cache_dir");
    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

    ok_or_error(ops::index_project(
        &project_path,
        cache_dir.as_deref(),
        force,
    ))
}

fn tool_file(args: &Value) -> Value {
    let Some(path) = path_arg(args, "path") else {
        return json!({"error": "path is required"});
    };
    let project_path = path_arg(args, "project_path");
    let cache_dir = path_arg(args, "cache_dir");
    let workspace_path = path_arg(args, "workspace_path");
    let min_confidence = match min_confidence_arg(args) {
        Ok(v) => v,
        Err(e) => return json!({"error": e}),
    };
    let explain = explain_arg(args);

    let result = ops::query_file(&path, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| {
            with_workspace(local, project_path.as_deref(), workspace_path.as_deref())
        });
    ok_or_error(result)
}

fn tool_change(args: &Value) -> Value {
    let Some(description) = str_arg(args, "description") else {
        return json!({"error": "description is required"});
    };
    let project_path = path_arg(args, "project_path");
    let cache_dir = path_arg(args, "cache_dir");
    let workspace_path = path_arg(args, "workspace_path");
    let min_confidence = match min_confidence_arg(args) {
        Ok(v) => v,
        Err(e) => return json!({"error": e}),
    };
    let explain = explain_arg(args);

    let result = ops::apply_change(&description, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| {
            with_workspace(local, project_path.as_deref(), workspace_path.as_deref())
        });
    ok_or_error(result)
}

fn tool_diff(args: &Value) -> Value {
    let Some(diff) = str_arg(args, "diff") else {
        return json!({"error": "diff is required"});
    };
    let project_path = path_arg(args, "project_path");
    let cache_dir = path_arg(args, "cache_dir");
    let workspace_path = path_arg(args, "workspace_path");
    let min_confidence = match min_confidence_arg(args) {
        Ok(v) => v,
        Err(e) => return json!({"error": e}),
    };
    let explain = explain_arg(args);

    let result = ops::diff_impact(&diff, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| {
            with_workspace(local, project_path.as_deref(), workspace_path.as_deref())
        });
    ok_or_error(result)
}

/// Extends a local report with cross-project matches when `workspace_path` was given, or
/// returns it as-is — both serialized to `Value` here so the two branches unify into one
/// return type without needing a trait object.
fn with_workspace(
    local: impact_core::ImpactReport,
    project_path: Option<&std::path::Path>,
    workspace_path: Option<&std::path::Path>,
) -> anyhow::Result<Value> {
    match workspace_path {
        Some(ws) => {
            let report = ops::cross_project_report(local, project_path, ws)?;
            Ok(serde_json::to_value(report)?)
        }
        None => Ok(serde_json::to_value(local)?),
    }
}
