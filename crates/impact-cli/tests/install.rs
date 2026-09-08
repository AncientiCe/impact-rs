use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::{json, Value};

fn run(args: &[&str]) -> std::process::Output {
    Command::cargo_bin("impact")
        .unwrap()
        .args(args)
        .output()
        .unwrap()
}

fn run_ok_json(args: &[&str]) -> Value {
    let output = run(args);
    assert!(
        output.status.success(),
        "impact {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("expected JSON stdout, got error {e}: {:?}", output.stdout))
}

fn install_client(home: &Path, client: &str, extra: &[&str]) -> Value {
    let mut args = vec!["install", "--client", client, "--home-dir"];
    let home_str = home.to_str().unwrap();
    args.push(home_str);
    args.push("--json");
    args.extend_from_slice(extra);
    run_ok_json(&args)
}

fn uninstall_client(home: &Path, client: &str, extra: &[&str]) -> Value {
    let mut args = vec!["uninstall", "--client", client, "--home-dir"];
    let home_str = home.to_str().unwrap();
    args.push(home_str);
    args.push("--json");
    args.extend_from_slice(extra);
    run_ok_json(&args)
}

fn doctor(home: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["doctor", "--home-dir"];
    let home_str = home.to_str().unwrap();
    args.push(home_str);
    args.push("--json");
    args.extend_from_slice(extra);
    run_ok_json(&args)
}

fn install(home: &Path, extra: &[&str]) -> Value {
    install_client(home, "cursor", extra)
}

fn uninstall(home: &Path, extra: &[&str]) -> Value {
    uninstall_client(home, "cursor", extra)
}

fn mcp_json_path(home: &Path) -> std::path::PathBuf {
    home.join(".cursor").join("mcp.json")
}

fn rule_path(home: &Path) -> std::path::PathBuf {
    home.join(".cursor").join("rules").join("impact.mdc")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn claude_settings_path(home: &Path) -> std::path::PathBuf {
    home.join(".claude").join("settings.json")
}

/// The `PreToolUse` entries `impact` owns — matched on the command's tail so a reinstall
/// from a different binary path still finds the one it wrote last time.
fn impact_hook_entries(settings: &Value) -> Vec<&Value> {
    settings["hooks"]["PreToolUse"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry["hooks"]
                        .as_array()
                        .is_some_and(|hooks| {
                            hooks.iter().any(|h| {
                                h["command"]
                                    .as_str()
                                    .is_some_and(|c| c.trim_end().ends_with("hook pre-tool-use"))
                            })
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn client_status<'a>(report: &'a Value, client: &str) -> &'a Value {
    report["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["client"] == client)
        .unwrap_or_else(|| panic!("no doctor status for client {client} in {report}"))
}

#[test]
fn install_writes_cursor_mcp_entry_and_rule_file() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);

    let config = read_json(&mcp_json_path(home.path()));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
    let command = config["mcpServers"]["impact"]["command"]
        .as_str()
        .expect("command should be a string");
    assert!(
        Path::new(command).is_absolute(),
        "command should be an absolute binary path, got {command}"
    );

    let rule = fs::read_to_string(rule_path(home.path())).unwrap();
    assert!(rule.contains("alwaysApply: true"));
    assert!(rule.contains("impact_index"));
    assert!(rule.contains("impact_file"));
    assert!(rule.contains("impact_change"));
}

/// The installed rule must widen the "before editing" trigger to also cover proposing a
/// concrete fix (a rename/remove/signature-change target stated before any code is
/// written) while still excluding vague, exploratory discussion that hasn't settled on a
/// concrete target — see the impact_rs `todo` Palace memory recorded 2026-09-03.
#[test]
fn installed_rule_covers_proposing_a_concrete_fix() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);

    let rule = fs::read_to_string(rule_path(home.path())).unwrap();
    assert!(
        rule.contains("propos"),
        "rule should mention proposing a fix: {rule}"
    );
    assert!(
        rule.contains("vague") || rule.contains("exploratory"),
        "rule should still exclude vague/exploratory discussion: {rule}"
    );
}

/// Impact's MCP tools are deferred behind a tool search in some clients, so a protocol
/// whose every trigger is conditional never gets them loaded at all — when a trigger
/// finally fires there is nothing in the agent's tool set to reach for. The rule
/// therefore needs one unconditional trigger that loads the tools and indexes the
/// project up front.
#[test]
fn installed_rule_has_an_unconditional_session_start_trigger() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);

    let rule = fs::read_to_string(rule_path(home.path())).unwrap();
    let session_start = rule
        .split("## SESSION START")
        .nth(1)
        .unwrap_or_else(|| panic!("rule should carry a session-start trigger: {rule}"));
    let section = session_start
        .split("## BEFORE EDITING")
        .next()
        .unwrap_or_default();
    assert!(
        section.contains("impact_index"),
        "the session-start trigger should index the project: {rule}"
    );
}

/// "Before renaming, removing, or changing a signature" makes the agent classify its own
/// change before the trigger can fire — a step it skips exactly when it is moving fast and
/// the change is riskiest. The rule must also state checkpoints that need no judgment
/// call: the first edit of a session, and any commit.
#[test]
fn installed_rule_states_mechanical_checkpoints() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);

    let rule = fs::read_to_string(rule_path(home.path())).unwrap();
    let before_editing = rule
        .split("## BEFORE EDITING")
        .nth(1)
        .unwrap_or_else(|| panic!("rule should carry a before-editing trigger: {rule}"));
    let section = before_editing
        .split("## AFTER EDITING")
        .next()
        .unwrap_or_default();
    assert!(
        section.contains("commit"),
        "the before-editing trigger should fire before any commit: {rule}"
    );
    assert!(
        section.contains("first Edit/Write"),
        "the before-editing trigger should fire before the session's first edit: {rule}"
    );
}

/// The README publishes the rule text for agents `impact install` has no installer for,
/// claiming it is "exactly what `impact install` generates". It silently drifted out of
/// date once already; keep the claim true.
#[test]
fn readme_publishes_the_installed_rule_text_verbatim() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "codex", &[]);

    let agents_md = fs::read_to_string(home.path().join(".codex").join("AGENTS.md")).unwrap();
    let body = agents_md
        .split_once("<!-- BEGIN IMPACT -->\n")
        .and_then(|(_, rest)| rest.split_once("\n<!-- END IMPACT -->"))
        .map(|(body, _)| body)
        .expect("installed AGENTS.md should carry a managed block");

    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md")).unwrap();
    assert!(
        readme.contains(body),
        "README's quoted rule block is stale; it must match what install writes:\n{body}"
    );
}

/// The rule tells the agent when to run impact; the hook makes the client run it whether
/// or not the agent remembered. Claude Code is the one supported client with a hook
/// mechanism, so it is the one that gets it.
#[test]
fn install_writes_the_claude_pre_tool_use_hook() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "claude", &[]);

    let settings = read_json(&claude_settings_path(home.path()));
    let entries = impact_hook_entries(&settings);
    assert_eq!(entries.len(), 1, "expected one impact hook: {settings}");
    let matcher = entries[0]["matcher"].as_str().unwrap_or_default();
    for tool in ["Edit", "Write", "Bash"] {
        assert!(
            matcher.contains(tool),
            "hook should match {tool}: {matcher}"
        );
    }
    let command = entries[0]["hooks"][0]["command"].as_str().unwrap();
    assert!(
        command.contains("hook pre-tool-use"),
        "hook should run impact's own hook subcommand: {command}"
    );
}

/// Cursor and Codex have no hook mechanism to install into, so installing for them must
/// not leave a Claude settings file behind.
#[test]
fn clients_without_hooks_get_no_settings_file() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "cursor", &[]);
    install_client(home.path(), "codex", &[]);

    assert!(!claude_settings_path(home.path()).exists());
}

#[test]
fn no_hook_skips_the_hook() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "claude", &["--no-hook"]);

    assert!(!claude_settings_path(home.path()).exists());
}

#[test]
fn claude_hook_install_preserves_unrelated_hooks_and_settings() {
    let home = tempfile::tempdir().unwrap();
    let settings_path = claude_settings_path(home.path());
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool hook"}]}
                ],
                "Stop": [{"hooks": [{"type": "command", "command": "other-tool stop"}]}]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    install_client(home.path(), "claude", &[]);

    let settings = read_json(&settings_path);
    assert_eq!(settings["theme"], "dark");
    assert_eq!(
        settings["hooks"]["Stop"][0]["hooks"][0]["command"],
        "other-tool stop"
    );
    let pre = settings["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2, "impact's hook should be added, not swapped in");
    assert_eq!(pre[0]["hooks"][0]["command"], "other-tool hook");
    assert_eq!(impact_hook_entries(&settings).len(), 1);
}

#[test]
fn second_claude_install_leaves_the_hook_untouched() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "claude", &[]);
    let report = install_client(home.path(), "claude", &[]);

    assert_eq!(report["hook_changed"], json!([]));
    assert_eq!(
        report["hook_unchanged"],
        json!([claude_settings_path(home.path())])
    );
    assert_eq!(
        impact_hook_entries(&read_json(&claude_settings_path(home.path()))).len(),
        1
    );
}

#[test]
fn uninstall_removes_only_the_impact_hook() {
    let home = tempfile::tempdir().unwrap();
    let settings_path = claude_settings_path(home.path());
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool hook"}]}
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    install_client(home.path(), "claude", &[]);

    uninstall_client(home.path(), "claude", &[]);

    let settings = read_json(&settings_path);
    assert!(impact_hook_entries(&settings).is_empty());
    assert_eq!(
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "other-tool hook"
    );
}

#[test]
fn doctor_reports_the_claude_hook() {
    let home = tempfile::tempdir().unwrap();

    let before = doctor(home.path(), &["--client", "claude"]);
    assert_eq!(client_status(&before, "claude")["hook_installed"], false);

    install_client(home.path(), "claude", &[]);

    let after = doctor(home.path(), &["--client", "claude"]);
    assert_eq!(client_status(&after, "claude")["hook_installed"], true);
    assert_eq!(client_status(&after, "claude")["hook_current"], true);
}

#[test]
fn install_merges_into_existing_cursor_config_without_clobbering() {
    let home = tempfile::tempdir().unwrap();
    let cursor_dir = home.path().join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {"other-tool": {"command": "other", "args": []}},
            "unrelatedKey": true
        }))
        .unwrap(),
    )
    .unwrap();

    install(home.path(), &[]);

    let config = read_json(&mcp_json_path(home.path()));
    assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");
    assert_eq!(config["unrelatedKey"], true);
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
    assert!(cursor_dir.join("mcp.json.bak").exists());
}

#[test]
fn second_install_is_idempotent() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);
    let mcp_before = fs::read_to_string(mcp_json_path(home.path())).unwrap();
    let rule_before = fs::read_to_string(rule_path(home.path())).unwrap();

    let report = install(home.path(), &[]);

    assert_eq!(report["changed"], json!([]));
    assert_eq!(
        mcp_before,
        fs::read_to_string(mcp_json_path(home.path())).unwrap()
    );
    assert_eq!(
        rule_before,
        fs::read_to_string(rule_path(home.path())).unwrap()
    );
}

#[test]
fn dry_run_reports_changes_but_writes_nothing() {
    let home = tempfile::tempdir().unwrap();

    let report = install(home.path(), &["--dry-run"]);

    assert_ne!(report["changed"], json!([]));
    assert!(!mcp_json_path(home.path()).exists());
    assert!(!rule_path(home.path()).exists());
}

#[test]
fn no_rule_skips_rule_file() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &["--no-rule"]);

    assert!(mcp_json_path(home.path()).exists());
    assert!(!rule_path(home.path()).exists());
}

#[test]
fn uninstall_removes_only_impact_entry_and_deletes_standalone_rule() {
    let home = tempfile::tempdir().unwrap();
    install(home.path(), &[]);
    let cursor_dir = home.path().join(".cursor");
    let mut config: Value = read_json(&mcp_json_path(home.path()));
    config["mcpServers"]["other-tool"] = json!({"command": "other", "args": []});
    fs::write(
        cursor_dir.join("mcp.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();

    uninstall(home.path(), &[]);

    let config = read_json(&mcp_json_path(home.path()));
    assert!(config["mcpServers"].get("impact").is_none());
    assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");
    assert!(!rule_path(home.path()).exists());
}

// ---- Phase 2: Cursor, project scope ----

#[test]
fn cursor_project_scope_writes_under_project_dir_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "cursor",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(project.path().join(".cursor/mcp.json").exists());
    assert!(project.path().join(".cursor/rules/impact.mdc").exists());
    assert!(!mcp_json_path(home.path()).exists());
    assert!(!rule_path(home.path()).exists());
}

// ---- Phase 3: Codex (TOML) ----

#[test]
fn install_writes_codex_toml_entry_and_agents_md_block() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "codex", &[]);

    let toml_text = fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    let doc: toml_edit::DocumentMut = toml_text.parse().unwrap();
    assert_eq!(
        doc["mcp_servers"]["impact"]["args"][0].as_str(),
        Some("mcp")
    );
    assert!(!doc["mcp_servers"]["impact"]["command"]
        .as_str()
        .unwrap()
        .is_empty());

    let agents_md = fs::read_to_string(home.path().join(".codex/AGENTS.md")).unwrap();
    assert!(agents_md.contains("<!-- BEGIN IMPACT -->"));
    assert!(agents_md.contains("impact_change"));
    assert!(agents_md.contains("<!-- END IMPACT -->"));
}

#[test]
fn codex_install_preserves_comments_and_unrelated_table() {
    let home = tempfile::tempdir().unwrap();
    let codex_dir = home.path().join(".codex");
    fs::create_dir_all(&codex_dir).unwrap();
    fs::write(
        codex_dir.join("config.toml"),
        "# keep this comment\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other\"\n",
    )
    .unwrap();

    install_client(home.path(), "codex", &[]);

    let toml_text = fs::read_to_string(codex_dir.join("config.toml")).unwrap();
    assert!(toml_text.contains("# keep this comment"));
    assert!(toml_text.contains("model = \"gpt-5\""));
    let doc: toml_edit::DocumentMut = toml_text.parse().unwrap();
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
    assert!(!doc["mcp_servers"]["impact"]["command"]
        .as_str()
        .unwrap()
        .is_empty());
}

#[test]
fn codex_scope_project_moves_only_the_rule_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "codex",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(home.path().join(".codex/config.toml").exists());
    assert!(project.path().join("AGENTS.md").exists());
    assert!(!home.path().join(".codex/AGENTS.md").exists());
    assert!(!project.path().join(".codex/config.toml").exists());
}

#[test]
fn codex_second_install_is_idempotent() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "codex", &[]);
    let toml_before = fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();

    let report = install_client(home.path(), "codex", &[]);

    assert_eq!(report["changed"], json!([]));
    assert_eq!(
        toml_before,
        fs::read_to_string(home.path().join(".codex/config.toml")).unwrap()
    );
}

#[test]
fn codex_uninstall_preserves_unrelated_table() {
    let home = tempfile::tempdir().unwrap();
    install_client(home.path(), "codex", &[]);
    let codex_dir = home.path().join(".codex");
    let mut doc: toml_edit::DocumentMut = fs::read_to_string(codex_dir.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    doc["mcp_servers"]["other"] = toml_edit::Item::Table(toml_edit::Table::new());
    doc["mcp_servers"]["other"]["command"] = toml_edit::value("other");
    fs::write(codex_dir.join("config.toml"), doc.to_string()).unwrap();

    uninstall_client(home.path(), "codex", &[]);

    let doc: toml_edit::DocumentMut = fs::read_to_string(codex_dir.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(doc["mcp_servers"].get("impact").is_none());
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
}

// ---- Phase 4: Claude Code ----

#[test]
fn install_writes_claude_json_file_and_claude_md_block() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "claude", &[]);

    let config = read_json(&home.path().join(".claude.json"));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
    assert!(!home
        .path()
        .join(".claude")
        .join("mcp_servers.json")
        .exists());

    let claude_md = fs::read_to_string(home.path().join(".claude/CLAUDE.md")).unwrap();
    assert!(claude_md.contains("<!-- BEGIN IMPACT -->"));
    assert!(claude_md.contains("impact_index"));
}

#[test]
fn claude_install_preserves_unrelated_json_content() {
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join(".claude.json"),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {"other-tool": {"command": "other", "args": []}},
            "theme": "dark"
        }))
        .unwrap(),
    )
    .unwrap();

    install_client(home.path(), "claude", &[]);

    let config = read_json(&home.path().join(".claude.json"));
    assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");
    assert_eq!(config["theme"], "dark");
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
}

#[test]
fn claude_scope_project_moves_only_claude_md() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "claude",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(home.path().join(".claude.json").exists());
    assert!(project.path().join("CLAUDE.md").exists());
    assert!(!home.path().join(".claude/CLAUDE.md").exists());
}

// ---- Phase 5: Claude Desktop ----

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[test]
fn install_writes_claude_desktop_config_under_home_dir() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "claude-desktop", &[]);

    let path = if cfg!(target_os = "windows") {
        home.path()
            .join("AppData/Roaming/Claude/claude_desktop_config.json")
    } else {
        home.path()
            .join("Library/Application Support/Claude/claude_desktop_config.json")
    };
    let config = read_json(&path);
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[test]
fn claude_desktop_is_rejected_on_unsupported_platforms() {
    let home = tempfile::tempdir().unwrap();

    let output = run(&[
        "install",
        "--client",
        "claude-desktop",
        "--home-dir",
        home.path().to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported"));
}

// ---- Phase 6: --client all ----

#[test]
fn client_all_installs_every_supported_client() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "all", &[]);

    assert!(mcp_json_path(home.path()).exists());
    assert!(home.path().join(".codex/config.toml").exists());
    assert!(home.path().join(".claude.json").exists());
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        let path = if cfg!(target_os = "windows") {
            home.path()
                .join("AppData/Roaming/Claude/claude_desktop_config.json")
        } else {
            home.path()
                .join("Library/Application Support/Claude/claude_desktop_config.json")
        };
        assert!(path.exists());
    }
}

// ---- Phase 7: doctor ----

#[test]
fn doctor_reports_nothing_configured_on_empty_home() {
    let home = tempfile::tempdir().unwrap();

    let report = doctor(home.path(), &[]);

    let cursor = client_status(&report, "cursor");
    assert_eq!(cursor["configured"], false);
    assert_eq!(cursor["rule_installed"], false);
}

#[test]
fn doctor_reports_configured_and_current_after_install() {
    let home = tempfile::tempdir().unwrap();
    install_client(home.path(), "cursor", &[]);

    let report = doctor(home.path(), &[]);

    let cursor = client_status(&report, "cursor");
    assert_eq!(cursor["configured"], true);
    assert_eq!(cursor["points_to_expected_binary"], true);
    assert_eq!(cursor["rule_installed"], true);
    assert_eq!(cursor["rule_current"], true);

    let codex = client_status(&report, "codex");
    assert_eq!(codex["configured"], false);
}

#[test]
fn doctor_flags_config_pointing_at_a_different_binary() {
    let home = tempfile::tempdir().unwrap();
    let cursor_dir = home.path().join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {"impact": {"command": "/some/other/impact", "args": ["mcp"]}}
        }))
        .unwrap(),
    )
    .unwrap();

    let report = doctor(home.path(), &[]);

    let cursor = client_status(&report, "cursor");
    assert_eq!(cursor["configured"], true);
    assert_eq!(cursor["points_to_expected_binary"], false);
}

#[test]
fn doctor_flags_missing_rule_after_manual_deletion() {
    let home = tempfile::tempdir().unwrap();
    install_client(home.path(), "cursor", &[]);
    fs::remove_file(rule_path(home.path())).unwrap();

    let report = doctor(home.path(), &[]);

    let cursor = client_status(&report, "cursor");
    assert_eq!(cursor["configured"], true);
    assert_eq!(cursor["rule_installed"], false);
}

#[test]
fn doctor_flags_stale_rule_after_manual_edit() {
    let home = tempfile::tempdir().unwrap();
    install_client(home.path(), "cursor", &[]);
    fs::write(
        rule_path(home.path()),
        "edited by hand, no longer matches\n",
    )
    .unwrap();

    let report = doctor(home.path(), &[]);

    let cursor = client_status(&report, "cursor");
    assert_eq!(cursor["rule_installed"], true);
    assert_eq!(cursor["rule_current"], false);
}
