//! Read-modify-write helpers for the config/rule files `impact install` touches.
//!
//! Every writer follows the same shape: read the existing file (or an empty default if
//! it doesn't exist yet), clone it, mutate only the `impact` entry/block, compare
//! against the original, and write only if something actually changed — after backing
//! up the original once (never overwriting an existing `.bak`). This is what lets
//! `install` merge into a config file a user or another tool already wrote to without
//! clobbering unrelated content.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::{json, Value};
use toml_edit::{value, Array, DocumentMut, Item, Table};

use super::clients::JsonLayout;

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn backup_existing(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid config filename: {}", path.display()))?;
    let backup = path.with_file_name(format!("{file_name}.bak"));
    if !backup.exists() {
        fs::copy(path, &backup).with_context(|| {
            format!(
                "failed to back up {} to {}",
                path.display(),
                backup.display()
            )
        })?;
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))
}

// ---- JSON (Cursor mcp.json, Claude .claude.json, OpenCode opencode.json, ...) ----

pub fn read_json_config(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn json_servers_key(layout: JsonLayout) -> &'static str {
    match layout {
        JsonLayout::McpServers => "mcpServers",
        JsonLayout::OpenCode => "mcp",
        JsonLayout::VsCode => "servers",
        JsonLayout::Zed => "context_servers",
    }
}

fn json_server_entry(layout: JsonLayout, binary_path: &Path) -> Value {
    let binary = path_to_string(binary_path);
    match layout {
        JsonLayout::McpServers | JsonLayout::Zed => {
            json!({ "command": binary, "args": ["mcp"] })
        }
        // OpenCode puts the arguments in `command` itself and needs the server type.
        JsonLayout::OpenCode => {
            json!({ "type": "local", "command": [binary, "mcp"], "enabled": true })
        }
        JsonLayout::VsCode => json!({ "type": "stdio", "command": binary, "args": ["mcp"] }),
    }
}

pub fn ensure_json_server(
    config: &mut Value,
    layout: JsonLayout,
    binary_path: &Path,
) -> Result<()> {
    if !config.is_object() {
        *config = json!({});
    }
    let object = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("JSON config root is not an object"))?;
    let key = json_servers_key(layout);
    let servers = object
        .entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{key} must be a JSON object"))?;
    servers.insert("impact".to_string(), json_server_entry(layout, binary_path));
    Ok(())
}

/// Returns `true` if an `impact` entry was actually present and removed.
pub fn remove_json_server(config: &mut Value, layout: JsonLayout) -> bool {
    let Some(servers) = config
        .get_mut(json_servers_key(layout))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    servers.remove("impact").is_some()
}

/// The binary the `impact` entry launches, if there is one.
pub fn json_server_command(config: &Value, layout: JsonLayout) -> Option<String> {
    let command = config
        .get(json_servers_key(layout))
        .and_then(|servers| servers.get("impact"))
        .and_then(|server| server.get("command"))?;
    let binary = match layout {
        JsonLayout::McpServers | JsonLayout::VsCode | JsonLayout::Zed => command,
        JsonLayout::OpenCode => command.get(0)?,
    };
    binary.as_str().map(String::from)
}

// ---- JSONC (VS Code mcp.json, Zed settings.json) — edited as a syntax tree so comments survive ----

fn parse_jsonc(text: &str) -> Result<CstRootNode> {
    CstRootNode::parse(text, &ParseOptions::default()).map_err(|error| anyhow!("{error}"))
}

fn to_cst_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_cst_input).collect()),
        Value::Object(entries) => CstInputValue::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), to_cst_input(entry)))
                .collect(),
        ),
    }
}

/// `text` with the `impact` entry added or replaced, everything else left as written.
pub fn ensure_jsonc_server(text: &str, layout: JsonLayout, binary_path: &Path) -> Result<String> {
    let root = parse_jsonc(text)?;
    let key = json_servers_key(layout);
    let servers = root
        .object_value_or_set()
        .object_value_or_create(key)
        .ok_or_else(|| anyhow!("{key} must be a JSON object"))?;
    let entry = json_server_entry(layout, binary_path);
    match servers.get("impact") {
        Some(existing)
            if existing.value().and_then(|value| value.to_serde_value()) == Some(entry.clone()) =>
        {
            return Ok(text.to_string())
        }
        Some(existing) => existing.set_value(to_cst_input(&entry)),
        None => {
            servers.append("impact", to_cst_input(&entry));
        }
    }
    let next = root.to_string();
    Ok(if next.ends_with('\n') {
        next
    } else {
        format!("{next}\n")
    })
}

/// `text` without the `impact` entry — unchanged if there was none.
pub fn remove_jsonc_server(text: &str, layout: JsonLayout) -> Result<String> {
    let root = parse_jsonc(text)?;
    let Some(entry) = root
        .object_value()
        .and_then(|object| object.object_value(json_servers_key(layout)))
        .and_then(|servers| servers.get("impact"))
    else {
        return Ok(text.to_string());
    };
    entry.remove();
    Ok(root.to_string())
}

pub fn read_jsonc_config(path: &Path) -> Result<Value> {
    let root = parse_jsonc(&read_text_file(path)?)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(root.to_serde_value().unwrap_or_else(|| json!({})))
}

/// Writes `next` to `path` if it differs from `existing`, backing up first. Returns
/// whether a write happened (or, under `dry_run`, would have happened).
pub fn write_json_if_changed(
    path: &Path,
    existing: &Value,
    next: &Value,
    dry_run: bool,
) -> Result<bool> {
    if existing == next {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    backup_existing(path)?;
    ensure_parent_dir(path)?;
    let text = serde_json::to_string_pretty(next)?;
    fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

// ---- TOML (Codex config.toml) — toml_edit preserves comments/formatting on round-trip ----

pub fn read_toml_document(path: &Path) -> Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

pub fn ensure_toml_server(document: &mut DocumentMut, binary_path: &Path) {
    if !document.contains_key("mcp_servers") || !document["mcp_servers"].is_table_like() {
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let mut server = Table::new();
    server["command"] = value(path_to_string(binary_path));
    let mut args = Array::new();
    args.push("mcp");
    server["args"] = value(args);
    document["mcp_servers"]["impact"] = Item::Table(server);
}

/// Returns `true` if an `impact` entry was actually present and removed.
pub fn remove_toml_server(document: &mut DocumentMut) -> bool {
    let Some(table) = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
    else {
        return false;
    };
    table.remove("impact").is_some()
}

pub fn write_toml_if_changed(
    path: &Path,
    existing: &DocumentMut,
    next: &DocumentMut,
    dry_run: bool,
) -> Result<bool> {
    if existing.to_string() == next.to_string() {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    backup_existing(path)?;
    ensure_parent_dir(path)?;
    fs::write(path, next.to_string())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

// ---- Text (rule files: managed-block AGENTS.md/CLAUDE.md, standalone Cursor .mdc) ----

pub fn read_text_file(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub fn write_text_if_changed(
    path: &Path,
    existing: &str,
    next: &str,
    dry_run: bool,
) -> Result<bool> {
    if existing == next {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    backup_existing(path)?;
    ensure_parent_dir(path)?;
    fs::write(path, next).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

/// Deletes `path` (backing it up first) if it exists. Returns whether a deletion
/// happened (or, under `dry_run`, would have happened).
pub fn remove_file_if_exists(path: &Path, dry_run: bool) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    backup_existing(path)?;
    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(true)
}
