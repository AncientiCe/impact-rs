//! Claude Code's `PreToolUse` hook entry, as it lives in `settings.json`.
//!
//! Read-modify-write over a file the user owns, same as the rest of `install`: impact's
//! own entry is added, replaced or removed, and every other hook in the file is left
//! exactly as it was.

use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::config_io::path_to_string;

/// Wide enough to reach both checkpoints: a file about to change, and a shell command
/// that might be a commit. `impact hook pre-tool-use` decides which of the calls that
/// reach it actually earn a reminder.
const MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";

fn hook_command(binary_path: &Path) -> String {
    format!("\"{}\" hook pre-tool-use", path_to_string(binary_path))
}

fn impact_entry(binary_path: &Path) -> Value {
    json!({
        "matcher": MATCHER,
        "hooks": [{"type": "command", "command": hook_command(binary_path)}],
    })
}

/// Matched on the command's tail rather than the whole string, so a reinstall from a
/// different binary path updates the entry it wrote last time instead of adding a second
/// one that fires alongside it.
fn is_impact_command(command: &str) -> bool {
    command.trim_end().ends_with("hook pre-tool-use")
}

fn is_impact_entry(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_impact_command)
            })
        })
}

fn pre_tool_use_entries(settings: &Value) -> Option<&Vec<Value>> {
    settings.pointer("/hooks/PreToolUse").and_then(Value::as_array)
}

pub fn ensure_hook(settings: &mut Value, binary_path: &Path) -> Result<()> {
    if !settings.is_object() {
        *settings = json!({});
    }
    let root = settings
        .as_object_mut()
        .ok_or_else(|| anyhow!("settings root is not a JSON object"))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("hooks must be a JSON object"))?;
    let entries = hooks
        .entry("PreToolUse")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("hooks.PreToolUse must be a JSON array"))?;

    let entry = impact_entry(binary_path);
    match entries.iter_mut().find(|existing| is_impact_entry(existing)) {
        Some(existing) => *existing = entry,
        None => entries.push(entry),
    }
    Ok(())
}

/// Returns `true` if an impact hook was actually present and removed. An entry that also
/// carries someone else's hook keeps that one; containers left empty are pruned, so
/// uninstalling doesn't leave `"hooks": {"PreToolUse": []}` behind.
pub fn remove_hook(settings: &mut Value) -> bool {
    let Some(entries) = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = entries.clone();
    for entry in entries.iter_mut() {
        if let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|hook| {
                !hook
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_impact_command)
            });
        }
    }
    entries.retain(|entry| match entry.get("hooks").and_then(Value::as_array) {
        Some(hooks) => !hooks.is_empty(),
        None => true,
    });
    let removed = *entries != before;
    if entries.is_empty() {
        prune_empty_containers(settings);
    }
    removed
}

fn prune_empty_containers(settings: &mut Value) {
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        hooks.remove("PreToolUse");
        if hooks.is_empty() {
            if let Some(root) = settings.as_object_mut() {
                root.remove("hooks");
            }
        }
    }
}

pub fn has_hook(settings: &Value) -> bool {
    pre_tool_use_entries(settings).is_some_and(|entries| entries.iter().any(is_impact_entry))
}

/// Whether the installed entry is exactly what this binary would write now — a hook
/// pointing at an older path or an older matcher is installed but not current.
pub fn hook_is_current(settings: &Value, binary_path: &Path) -> bool {
    let expected = impact_entry(binary_path);
    pre_tool_use_entries(settings)
        .is_some_and(|entries| entries.iter().any(|entry| *entry == expected))
}
