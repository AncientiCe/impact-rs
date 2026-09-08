//! `impact hook pre-tool-use` — what Claude Code runs before an edit or a commit.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

/// Runs the hook with `payload` on stdin, isolating its per-session state in `state_dir`
/// so tests can't see each other's markers.
fn run_hook(payload: &str, state_dir: &Path) -> std::process::Output {
    let dir = state_dir.to_str().unwrap();
    Command::cargo_bin("impact")
        .unwrap()
        .args(["hook", "pre-tool-use"])
        .env("TMPDIR", dir)
        .env("TMP", dir)
        .env("TEMP", dir)
        .write_stdin(payload.to_string())
        .output()
        .unwrap()
}

fn additional_context(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "expected a JSON hook response, got error {e}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(response["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("hook response should carry additionalContext: {response}"))
        .to_string()
}

fn assert_silent(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "",
        "hook should have said nothing"
    );
}

fn edit(session: &str) -> String {
    format!(r#"{{"session_id": "{session}", "tool_name": "Edit"}}"#)
}

fn bash(session: &str, command: &str) -> String {
    format!(r#"{{"session_id": "{session}", "tool_name": "Bash", "tool_input": {{"command": "{command}"}}}}"#)
}

/// The checkpoint the rule names first: the session's first edit, whether or not the
/// agent has classified it as a rename/removal/signature change.
#[test]
fn the_first_edit_of_a_session_gets_the_before_editing_reminder() {
    let state = tempfile::tempdir().unwrap();

    let context = additional_context(&run_hook(&edit("session-a"), state.path()));

    assert!(
        context.contains("impact_index"),
        "reminder should ask for an index: {context}"
    );
    assert!(
        context.contains("impact_file") || context.contains("impact_change"),
        "reminder should ask for a blast radius: {context}"
    );
}

/// One reminder per session, not one per edit — a hook that fires on every Edit is noise
/// the agent learns to skip past.
#[test]
fn later_edits_in_the_same_session_stay_quiet() {
    let state = tempfile::tempdir().unwrap();

    additional_context(&run_hook(&edit("session-a"), state.path()));

    assert_silent(&run_hook(&edit("session-a"), state.path()));
    assert_silent(&run_hook(
        &format!(r#"{{"session_id": "session-a", "tool_name": "Write"}}"#),
        state.path(),
    ));
}

#[test]
fn a_new_session_gets_its_own_reminder() {
    let state = tempfile::tempdir().unwrap();

    additional_context(&run_hook(&edit("session-a"), state.path()));

    let context = additional_context(&run_hook(&edit("session-b"), state.path()));
    assert!(context.contains("impact_index"), "{context}");
}

/// The rule's second mechanical checkpoint. Commits are rare enough to remind on every
/// one, and it is the last moment before the change leaves the working tree.
#[test]
fn a_commit_gets_the_after_editing_reminder() {
    let state = tempfile::tempdir().unwrap();

    let context = additional_context(&run_hook(
        &bash("session-a", "git commit -m 'wip'"),
        state.path(),
    ));

    assert!(
        context.contains("impact_index"),
        "reminder should ask for a re-index: {context}"
    );
    assert!(
        context.contains("impact_diff"),
        "reminder should point at the diff's blast radius: {context}"
    );

    let context = additional_context(&run_hook(
        &bash("session-a", "git -C /tmp/repo commit --amend"),
        state.path(),
    ));
    assert!(context.contains("impact_index"), "{context}");
}

#[test]
fn other_bash_commands_stay_quiet() {
    let state = tempfile::tempdir().unwrap();

    assert_silent(&run_hook(&bash("session-a", "ls -la"), state.path()));
    assert_silent(&run_hook(
        &bash("session-a", "echo 'git commit'"),
        state.path(),
    ));
}

/// Run outside a hook (or by a client whose payload it doesn't recognize), it says
/// nothing and exits clean rather than failing the tool call it was attached to.
#[test]
fn an_unrecognized_payload_produces_no_output() {
    let state = tempfile::tempdir().unwrap();

    assert_silent(&run_hook("", state.path()));
    assert_silent(&run_hook("not json at all", state.path()));
    assert_silent(&run_hook(r#"{"tool_name": "Read"}"#, state.path()));
}
