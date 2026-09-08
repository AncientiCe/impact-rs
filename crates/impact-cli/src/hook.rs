//! Hook entry points for AI coding tools that can run a command around a tool call.
//!
//! The agent rule (`install::rule`) states the protocol, but a rule is read once and
//! competes for attention with everything else in the session. A hook is mechanical: the
//! client runs it whether or not the agent remembered, at exactly the two checkpoints the
//! rule names — the session's first edit, and any commit.
//!
//! Currently Claude Code's `PreToolUse` shape: the hook payload arrives as JSON on stdin,
//! and a JSON response on stdout feeds `additionalContext` back to the agent. Anything it
//! doesn't recognize produces no output and a clean exit, so attaching it to a broader
//! matcher (or running it by hand) never fails the tool call it wraps.

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::PathBuf;

use anyhow::Result;
use serde_json::{json, Value};

/// Tools whose use means a file is about to change.
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

const FIRST_EDIT_REMINDER: &str = "Impact protocol — this is the first file edit of this \
    session. Before it lands: call impact_index on the project root, then impact_file on \
    the file you're changing (or impact_change on the rename/removal/signature change), \
    and treat the report as a checklist of callers and tests to update. A thin or empty \
    radius is not proof of safety — a call reached through a registry indirection, or a \
    function passed as a value rather than called, leaves no edge, so cross-check the \
    symbol name with grep.";

const COMMIT_REMINDER: &str = "Impact protocol — this commit is about to leave the \
    working tree. Re-run impact_index (results are only as fresh as the last index), then \
    impact_diff on the `git diff` you're committing, and confirm the blast radius you \
    addressed is what's reported now and nothing new appeared.";

pub fn pre_tool_use() -> Result<()> {
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload)?;

    let Some(reminder) = reminder_for(&payload) else {
        return Ok(());
    };

    let response = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": reminder,
        }
    });
    let mut out = io::stdout();
    writeln!(out, "{response}")?;
    out.flush()?;
    Ok(())
}

/// The reminder this tool call has earned, if any. Claiming the session's first edit is a
/// side effect of asking — the marker has to be written before the edit happens, and the
/// hook has no later moment to write it in.
fn reminder_for(payload: &str) -> Option<&'static str> {
    let payload: Value = serde_json::from_str(payload).ok()?;
    let tool = payload.get("tool_name").and_then(Value::as_str)?;
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    if EDIT_TOOLS.contains(&tool) {
        return claim_first_edit(session).then_some(FIRST_EDIT_REMINDER);
    }
    if tool == "Bash" {
        let command = payload
            .pointer("/tool_input/command")
            .and_then(Value::as_str)?;
        if is_git_commit(command) {
            return Some(COMMIT_REMINDER);
        }
    }
    None
}

/// Whether `command` runs `git commit` — matched per shell segment on the tokens
/// themselves, so `git -C repo commit --amend` counts and `echo "git commit"` doesn't.
fn is_git_commit(command: &str) -> bool {
    command.split(['\n', ';', '&', '|']).any(|segment| {
        let mut tokens = segment.split_whitespace();
        let runs_git = tokens
            .next()
            .is_some_and(|first| first == "git" || first.ends_with("/git"));
        runs_git && tokens.any(|token| token == "commit")
    })
}

/// Marks `session` as having been reminded, returning whether this call is the one that
/// claimed it. `create_new` makes the claim atomic, so two edits racing each other still
/// produce one reminder. Any other failure (an unwritable state directory) reminds again
/// rather than going quiet — a repeated reminder is the cheaper mistake.
fn claim_first_edit(session: &str) -> bool {
    let dir = state_dir();
    if fs::create_dir_all(&dir).is_err() {
        return true;
    }
    let marker = dir.join(format!("{}.first-edit", sanitized(session)));
    match OpenOptions::new().write(true).create_new(true).open(&marker) {
        Ok(_) => true,
        Err(e) => e.kind() != ErrorKind::AlreadyExists,
    }
}

/// Session markers live in the temp directory on purpose: they are meaningless once the
/// session that wrote them ends, and the OS clears them out on its own schedule.
fn state_dir() -> PathBuf {
    std::env::temp_dir().join("impact-hook")
}

/// A session id is client-supplied, so it never reaches the filesystem as-is.
fn sanitized(session: &str) -> String {
    session
        .chars()
        .take(64)
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
