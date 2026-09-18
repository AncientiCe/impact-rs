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
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::analytics;
use crate::blindspot::{self, BlindspotKind};
use crate::ops;

/// The MCP client's `clientInfo`, learned from `initialize` and reused to attribute
/// every subsequent `tools/call` in this stdio session.
#[derive(Default)]
struct ClientIdentity {
    name: Option<String>,
    version: Option<String>,
}

pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let analytics_conn = analytics::open().ok();
    let mut client = ClientIdentity::default();

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

        if let Some(response) = handle_request(&request, &mut client, analytics_conn.as_ref()) {
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
        "instructions": "At the start of every session, unconditionally, call impact_index \
            once with the project root — before you know whether the task will touch code \
            at all; the other tools are useless against an unindexed project. Then, before \
            modifying code, call \
            impact_file, impact_change, or impact_diff (given a unified diff) to see what \
            depends on what you're about to change — direct/indirect callers, API routes, \
            event types, database tables, and affected tests. Do that at two checkpoints \
            that need no judgment call — before the session's first edit, and before any \
            commit — rather than first classifying the change as a rename, a removal or a \
            signature change. This also applies before \
            proposing a fix: once the proposed fix is concrete enough to state as a \
            rename/remove/signature-change target, run impact analysis before presenting \
            the proposal, even if no code has been written yet — vague, exploratory \
            discussion that hasn't settled on a concrete target doesn't need it. Re-run \
            impact_index after the project changes; results are only as fresh as the \
            last index. If you manually confirm — by reading the code or grepping, never \
            by assumption — that one of these tools missed or misreported something, call \
            impact_report_blindspot (without submit) to draft a report, show it to the \
            user, and only pass submit after their explicit go-ahead; it files a public \
            GitHub issue and is never yours to send unilaterally.",
    })
}

fn handle_request(
    req: &Value,
    client: &mut ClientIdentity,
    analytics_conn: Option<&Connection>,
) -> Option<String> {
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_default();
    let req_id = req.get("id").cloned().unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            let protocol_version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            if let Some(info) = params.get("clientInfo") {
                client.name = info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                client.version = info
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            Some(initialize_result(protocol_version))
        }
        "notifications/initialized" => return None,
        "tools/list" => Some(json!({"tools": tool_list()})),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();
            let start = std::time::Instant::now();
            let result = dispatch_tool(tool_name, &args);
            if let (Some(command), Some(conn)) = (usage_command(tool_name), analytics_conn) {
                analytics::record(
                    conn,
                    analytics::UsageEvent {
                        command,
                        source: "mcp",
                        client: client.name.clone().unwrap_or_else(|| "unknown".to_string()),
                        client_version: client.version.clone(),
                        duration_ms: start.elapsed().as_millis() as u64,
                        success: result.get("error").is_none(),
                    },
                );
            }
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
            "description": "Index a project (or re-index it) so impact_file and impact_change have something to query. Call this once per project before the first query, and again whenever the project has changed since the last index. The result's orphaned_events lists any event contract that had both a producer and a consumer on the previous index and lost one of them on this one — worth a second look (not proof of a mistake) since it's the shape a producer-to-direct-call migration takes when the replacement forgets behavior the old consumer had.",
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
            "description": "Report the blast radius of changing a file: direct callers, indirect (transitive) callers, API routes, event types, and database tables the affected code touches, plus a count of affected tests. Each caller is tagged with a confidence tier (exact, probable, or heuristic) reflecting what evidence tied the call to this symbol: exact means an import or a declared type did, probable means only a project-unique name did, heuristic means an ambiguous name did — use min_confidence to hide the weaker tiers. With workspace_path, also reports which sibling projects registered there are touched by the same API routes/events/tables. Requires impact_index to have run first (and, for cross-project results, the sibling projects to have been indexed too). Blind spot: only calls impact can see syntactically become edges, so a call reached through a registry/selector indirection, or a function handed to something else as a value (transform: camelizeOrder) rather than called, leaves none — an empty or thin result is not proof there are no consumers, so cross-check the symbol name with grep.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File to compute the blast radius for, relative to project_path or absolute"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "probable", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything). exact = an import or declared type ties the call to this symbol; probable = a unique name with no scope evidence; heuristic = an ambiguous name"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"},
                    "summary": {"type": "boolean", "description": "Compact response: exact per-category counts, DIRECT listed in full, INDIRECT grouped and counted by file with only the first few entries shown per group. Use this for a file with many transitive dependents, where the full result risks exceeding the response size limit (default: false)"}
                },
                "required": ["path"]
            }
        },
        {
            "name": "impact_change",
            "description": "Report the deterministic blast radius of a specific change, described in impact's small fixed grammar: \"rename <path>\", \"rename <path> to <path>\", \"remove <path>\", \"remove variant <Enum>::<Variant>\", \"remove field <Type>.<field>\", or \"change signature of <path>\". Not natural language — an unrecognized description is a hard error, never a best-effort guess. <path> is a \"::\"-joined qualified symbol path from the language's own indexed structure, not a filesystem path (\"some/file.go::Symbol\" does not resolve) — e.g. Rust's \"some::Type::variant\", or a Go method reached through nested packages: \"service::repositories::changes::repository::Repository::AddOperation\" (exactly what impact_file prints in a report's direct/indirect entries — copy a path from there when unsure). Shorter forms (last two segments, or the bare symbol name) also resolve, at lower confidence when the name is ambiguous project-wide. Each caller in the result is tagged with a confidence tier (exact, probable, or heuristic); use min_confidence to hide the weaker tiers. With workspace_path, also reports cross-project impact like impact_file does. Requires impact_index to have run first. Blind spot: only calls impact can see syntactically become edges, so a call reached through a registry/selector indirection, or a function handed to something else as a value (transform: camelizeOrder) rather than called, leaves none — an empty or thin result is not proof there are no consumers, so cross-check the symbol name with grep.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "description": {"type": "string", "description": "The change, in impact's deterministic grammar"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "probable", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything). exact = an import or declared type ties the call to this symbol; probable = a unique name with no scope evidence; heuristic = an ambiguous name"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"},
                    "summary": {"type": "boolean", "description": "Compact response: exact per-category counts, DIRECT listed in full, INDIRECT grouped and counted by file with only the first few entries shown per group. Use this for a change with many transitive dependents, where the full result risks exceeding the response size limit (default: false)"}
                },
                "required": ["description"]
            }
        },
        {
            "name": "impact_diff",
            "description": "Report the combined blast radius of a unified diff (e.g. `git diff` output) — every symbol the diff's touched lines fall inside, across every file it mentions. Useful for checking the blast radius of a change you're about to apply (or already have, uncommitted) in one call instead of one impact_file call per touched file. Requires the project to be indexed against the diff's new side — i.e. the working tree as it currently stands. Each caller in the result is tagged with a confidence tier (exact, probable, or heuristic); use min_confidence to hide the weaker tiers. With workspace_path, also reports cross-project impact like impact_file does. Blind spot: only calls impact can see syntactically become edges, so a call reached through a registry/selector indirection, or a function handed to something else as a value (transform: camelizeOrder) rather than called, leaves none — an empty or thin result is not proof there are no consumers, so cross-check the symbol name with grep.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "diff": {"type": "string", "description": "Unified diff text, e.g. the output of `git diff`"},
                    "project_path": {"type": "string", "description": "Project root the cache was built against (defaults to the current directory)"},
                    "cache_dir": {"type": "string", "description": "Where the index cache lives (defaults to <project_path>/.impact)"},
                    "workspace_path": {"type": "string", "description": "Path to a workspace.toml registering sibling projects, to also compute cross-project impact"},
                    "min_confidence": {"type": "string", "enum": ["exact", "probable", "heuristic"], "description": "Only include DIRECT/INDIRECT dependents resolved with at least this confidence (default: heuristic, i.e. show everything). exact = an import or declared type ties the call to this symbol; probable = a unique name with no scope evidence; heuristic = an ambiguous name"},
                    "explain": {"type": "boolean", "description": "Include each INDIRECT entry's chain back to its nearest DIRECT dependent (default: false)"},
                    "summary": {"type": "boolean", "description": "Compact response: exact per-category counts, DIRECT listed in full, INDIRECT grouped and counted by file with only the first few entries shown per group. Use this for a diff with many transitive dependents, where the full result risks exceeding the response size limit (default: false)"}
                },
                "required": ["diff"]
            }
        },
        {
            "name": "impact_report_blindspot",
            "description": "Draft a GitHub issue reporting a case where impact missed or misreported something — only after manually confirming the gap by reading code or grepping, never speculatively. Composing the draft never touches the network; it only returns what would be filed, including a stable fingerprint used to avoid duplicates. This files against a public repo, and impact runs on private codebases — before drafting, redact every detail specific to the workspace under analysis: no local paths, no private repo names, no internal file/symbol names, no product-specific architecture. State the gap generically instead (language, rough file/symbol counts, the shape of the miss) — e.g. \"a 400-file C++ tree indexed 1 file\" rather than \"src/engine/render_pipeline.cpp in ProjectX\". Set submit=true to actually file it via `gh issue create` (after first checking `gh issue list` for an existing report with the same fingerprint) — get the user's explicit go-ahead before ever doing that, since it posts something public on their behalf.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Short issue title — generic, no private repo or project names"},
                    "body": {"type": "string", "description": "What was expected vs. what impact actually reported, rewritten to be generic and redacted: strip local paths, private repo names, internal file/symbol names, and product-specific architecture before filling this in. Describe the shape of the miss instead (language, counts, kind of construct) so a reader can't identify the private tree it came from."},
                    "kind": {"type": "string", "enum": ["missed-edge", "false-positive", "crash", "other"], "description": "What kind of gap this is (default: other)"},
                    "language": {"type": "string", "description": "The language involved, if relevant (e.g. \"swift\", \"go\")"},
                    "repo": {"type": "string", "description": "The owner/repo this would be filed against (default: AncientiCe/impact-rs)"},
                    "submit": {"type": "boolean", "description": "Actually file the issue via gh, after checking for a duplicate (default: false — draft only, no network)"}
                },
                "required": ["title", "body"]
            }
        }
    ])
}

/// Maps an MCP tool name to the canonical usage-analytics command name shared with the
/// CLI's equivalent subcommand (`impact query` and `impact_file` are both `"file"`).
fn usage_command(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "impact_index" => Some("index"),
        "impact_file" => Some("file"),
        "impact_change" => Some("change"),
        "impact_diff" => Some("diff"),
        "impact_report_blindspot" => Some("report-blindspot"),
        _ => None,
    }
}

fn dispatch_tool(name: &str, args: &Value) -> Value {
    match name {
        "impact_index" => tool_index(args),
        "impact_file" => tool_file(args),
        "impact_change" => tool_change(args),
        "impact_diff" => tool_diff(args),
        "impact_report_blindspot" => tool_report_blindspot(args),
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
/// three documented values with a clear error rather than silently ignoring a typo.
fn min_confidence_arg(args: &Value) -> Result<Option<impact_core::Confidence>, String> {
    match args.get("min_confidence").and_then(|v| v.as_str()) {
        None => Ok(None),
        Some("exact") => Ok(Some(impact_core::Confidence::Exact)),
        Some("probable") => Ok(Some(impact_core::Confidence::Probable)),
        Some("heuristic") => Ok(Some(impact_core::Confidence::Heuristic)),
        Some(other) => Err(format!(
            "min_confidence must be \"exact\", \"probable\" or \"heuristic\", got {other:?}"
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

fn summary_arg(args: &Value) -> bool {
    args.get("summary")
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

/// Parses the optional `kind` tool argument for `impact_report_blindspot`, defaulting to
/// `Other` when absent — mirrors `min_confidence_arg`'s "reject an unrecognized value,
/// don't silently ignore a typo" behavior.
fn blindspot_kind_arg(args: &Value) -> Result<BlindspotKind, String> {
    match args.get("kind").and_then(|v| v.as_str()) {
        None => Ok(BlindspotKind::Other),
        Some("missed-edge") => Ok(BlindspotKind::MissedEdge),
        Some("false-positive") => Ok(BlindspotKind::FalsePositive),
        Some("crash") => Ok(BlindspotKind::Crash),
        Some("other") => Ok(BlindspotKind::Other),
        Some(other) => Err(format!(
            "kind must be \"missed-edge\", \"false-positive\", \"crash\" or \"other\", got {other:?}"
        )),
    }
}

/// Composes the draft and, when `submit` is set, checks for an existing report before
/// filing a new one — the same two-step de-duplication as the CLI's `run_report_blindspot`.
fn tool_report_blindspot(args: &Value) -> Value {
    let Some(title) = str_arg(args, "title") else {
        return json!({"error": "title is required"});
    };
    let Some(body) = str_arg(args, "body") else {
        return json!({"error": "body is required"});
    };
    let kind = match blindspot_kind_arg(args) {
        Ok(k) => k,
        Err(e) => return json!({"error": e}),
    };
    let language = str_arg(args, "language");
    let repo = str_arg(args, "repo").unwrap_or_else(|| "AncientiCe/impact-rs".to_string());
    let submit = args
        .get("submit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let draft = blindspot::compose_draft(&title, &body, kind, language.as_deref(), &repo);

    if !submit {
        return match serde_json::to_value(&draft) {
            Ok(mut value) => {
                value["submitted"] = Value::Bool(false);
                value
            }
            Err(e) => json!({"error": e.to_string()}),
        };
    }

    match blindspot::find_existing(&draft.repo, &draft.fingerprint) {
        Ok(Some(existing)) => match serde_json::to_value(&draft) {
            Ok(mut value) => {
                value["submitted"] = Value::Bool(false);
                value["existing_url"] = Value::String(existing.url);
                value
            }
            Err(e) => json!({"error": e.to_string()}),
        },
        Ok(None) => match blindspot::submit(&draft) {
            Ok(url) => match serde_json::to_value(&draft) {
                Ok(mut value) => {
                    value["submitted"] = Value::Bool(true);
                    value["url"] = Value::String(url);
                    value
                }
                Err(e) => json!({"error": e.to_string()}),
            },
            Err(e) => json!({"error": e.to_string()}),
        },
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
    let summary = summary_arg(args);

    let result = ops::query_file(&path, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| with_workspace(local, project_path.as_deref(), workspace_path.as_deref()))
        .and_then(|(local, cross_project)| finalize_report(local, cross_project, summary));
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
    let summary = summary_arg(args);

    let result = ops::apply_change(&description, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| with_workspace(local, project_path.as_deref(), workspace_path.as_deref()))
        .and_then(|(local, cross_project)| finalize_report(local, cross_project, summary));
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
    let summary = summary_arg(args);

    let result = ops::diff_impact(&diff, project_path.as_deref(), cache_dir.as_deref())
        .map(|local| apply_min_confidence(local, min_confidence))
        .map(|local| impact_core::apply_explain(local, explain))
        .and_then(|local| with_workspace(local, project_path.as_deref(), workspace_path.as_deref()))
        .and_then(|(local, cross_project)| finalize_report(local, cross_project, summary));
    ok_or_error(result)
}

/// Extends a local report with cross-project matches when `workspace_path` was given —
/// returns the `ImpactReport` alongside the cross-project matches (if any) rather than a
/// serialized `Value`, so `finalize_report` can still choose between the full report and
/// `summarize`'s compact form after this runs.
fn with_workspace(
    local: impact_core::ImpactReport,
    project_path: Option<&std::path::Path>,
    workspace_path: Option<&std::path::Path>,
) -> anyhow::Result<(
    impact_core::ImpactReport,
    Option<Vec<impact_core::CrossProjectMatch>>,
)> {
    match workspace_path {
        Some(ws) => {
            let report = ops::cross_project_report(local, project_path, ws)?;
            Ok((report.local, Some(report.cross_project)))
        }
        None => Ok((local, None)),
    }
}

/// Renders a report (plus any cross-project matches) to the final `Value` a tool call
/// returns — either the full `ImpactReport`/`WorkspaceImpactReport` shape, or, when
/// `summary` is set, `impact_core::summarize`'s compact grouped-by-file form with
/// `cross_project` merged back in alongside it.
fn finalize_report(
    local: impact_core::ImpactReport,
    cross_project: Option<Vec<impact_core::CrossProjectMatch>>,
    summary: bool,
) -> anyhow::Result<Value> {
    if summary {
        let summary_report =
            impact_core::summarize(&local, impact_core::DEFAULT_SUMMARY_GROUP_LIMIT);
        let mut value = serde_json::to_value(&summary_report)?;
        if let Some(cross_project) = &cross_project {
            value["cross_project"] = serde_json::to_value(cross_project)?;
        }
        return Ok(value);
    }
    match cross_project {
        Some(cross_project) => {
            let report = impact_core::WorkspaceImpactReport {
                local,
                cross_project,
            };
            Ok(serde_json::to_value(report)?)
        }
        None => Ok(serde_json::to_value(local)?),
    }
}
