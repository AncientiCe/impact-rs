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
use serde_json::{json, Value};
use toml_edit::{value, Array, DocumentMut, Item, Table};

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

// ---- JSON (Cursor mcp.json, Claude .claude.json, Claude Desktop config) ----

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

pub fn ensure_json_server(config: &mut Value, binary_path: &Path) -> Result<()> {
    if !config.is_object() {
        *config = json!({});
    }
    let object = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("JSON config root is not an object"))?;
    let servers = object
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("mcpServers must be a JSON object"))?;
    servers.insert(
        "impact".to_string(),
        json!({ "command": path_to_string(binary_path), "args": ["mcp"] }),
    );
    Ok(())
}

/// Returns `true` if an `impact` entry was actually present and removed.
pub fn remove_json_server(config: &mut Value) -> bool {
    let Some(servers) = config.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return false;
    };
    servers.remove("impact").is_some()
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
