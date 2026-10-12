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
                    entry["hooks"].as_array().is_some_and(|hooks| {
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

/// The blast radius a reviewer can't see on their own belongs in the description of the
/// pull/merge request, so the rule has to ask for it when one is opened.
#[test]
fn installed_rule_asks_for_the_blast_radius_in_pull_request_descriptions() {
    let home = tempfile::tempdir().unwrap();

    install(home.path(), &[]);

    let rule = fs::read_to_string(rule_path(home.path())).unwrap();
    let section = rule
        .split("## OPENING A PULL REQUEST")
        .nth(1)
        .unwrap_or_else(|| panic!("rule should carry a pull-request trigger: {rule}"))
        .split("## BLIND SPOT FOUND")
        .next()
        .unwrap_or_default();
    assert!(
        section.contains("impact_diff") && section.contains("description"),
        "the pull-request trigger should put the diff's blast radius in the description: {rule}"
    );
    assert!(
        section.contains("merge request"),
        "the pull-request trigger should cover merge requests too: {rule}"
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
    assert_eq!(
        pre.len(),
        2,
        "impact's hook should be added, not swapped in"
    );
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

// ---- Gemini CLI ----

#[test]
fn install_writes_gemini_settings_and_gemini_md_block() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "gemini", &[]);

    let config = read_json(&home.path().join(".gemini/settings.json"));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
    assert!(Path::new(config["mcpServers"]["impact"]["command"].as_str().unwrap()).is_absolute());

    let gemini_md = fs::read_to_string(home.path().join(".gemini/GEMINI.md")).unwrap();
    assert!(gemini_md.contains("<!-- BEGIN IMPACT -->"));
    assert!(gemini_md.contains("impact_index"));
}

#[test]
fn gemini_install_preserves_unrelated_settings() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".gemini")).unwrap();
    fs::write(
        home.path().join(".gemini/settings.json"),
        serde_json::to_string_pretty(&json!({
            "theme": "GitHub",
            "mcpServers": {"other-tool": {"command": "other"}}
        }))
        .unwrap(),
    )
    .unwrap();

    install_client(home.path(), "gemini", &[]);
    uninstall_client(home.path(), "gemini", &[]);

    let config = read_json(&home.path().join(".gemini/settings.json"));
    assert_eq!(config["theme"], "GitHub");
    assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");
    assert!(config["mcpServers"].get("impact").is_none());
}

#[test]
fn gemini_project_scope_writes_under_project_dir_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "gemini",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(project.path().join(".gemini/settings.json").exists());
    assert!(project.path().join("GEMINI.md").exists());
    assert!(!home.path().join(".gemini").exists());
}

// ---- --client all only reaches the opt-in agents that are actually present ----

#[test]
fn client_all_skips_agents_with_no_home_directory() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "all", &[]);
    let report = doctor(home.path(), &[]);

    assert!(!home.path().join(".gemini").exists());
    assert!(report["clients"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["client"] != "gemini"));
}

#[test]
fn client_all_installs_agents_whose_home_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".gemini")).unwrap();

    install_client(home.path(), "all", &[]);

    let config = read_json(&home.path().join(".gemini/settings.json"));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));
    let report = doctor(home.path(), &[]);
    let gemini = client_status(&report, "gemini");
    assert_eq!(gemini["configured"], true);
    assert_eq!(gemini["rule_current"], true);
}

// ---- Kiro ----

#[test]
fn install_writes_kiro_mcp_entry_and_always_on_steering_file() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "kiro", &[]);

    let config = read_json(&home.path().join(".kiro/settings/mcp.json"));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));

    let steering = fs::read_to_string(home.path().join(".kiro/steering/impact.md")).unwrap();
    assert!(steering.starts_with("---\ninclusion: always\n---\n"));
    assert!(steering.contains("impact_change"));
    assert!(!steering.contains("<!-- BEGIN IMPACT -->"));
}

#[test]
fn kiro_project_scope_writes_under_project_dir_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "kiro",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(project.path().join(".kiro/settings/mcp.json").exists());
    assert!(project.path().join(".kiro/steering/impact.md").exists());
    assert!(!home.path().join(".kiro").exists());
}

#[test]
fn kiro_uninstall_removes_steering_file_and_only_the_impact_entry() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".kiro/settings")).unwrap();
    fs::write(
        home.path().join(".kiro/settings/mcp.json"),
        serde_json::to_string_pretty(&json!({
            "mcpServers": {"other-tool": {"command": "other"}}
        }))
        .unwrap(),
    )
    .unwrap();
    install_client(home.path(), "kiro", &[]);

    uninstall_client(home.path(), "kiro", &[]);

    let config = read_json(&home.path().join(".kiro/settings/mcp.json"));
    assert_eq!(config["mcpServers"]["other-tool"]["command"], "other");
    assert!(config["mcpServers"].get("impact").is_none());
    assert!(!home.path().join(".kiro/steering/impact.md").exists());
}

#[test]
fn client_all_installs_kiro_when_its_home_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".kiro")).unwrap();

    install_client(home.path(), "all", &[]);

    assert!(home.path().join(".kiro/settings/mcp.json").exists());
    let report = doctor(home.path(), &[]);
    assert_eq!(client_status(&report, "kiro")["rule_current"], true);
}

// ---- Windsurf ----

#[test]
fn install_writes_windsurf_mcp_config_and_global_rules_block() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "windsurf", &[]);

    let config = read_json(&home.path().join(".codeium/windsurf/mcp_config.json"));
    assert_eq!(config["mcpServers"]["impact"]["args"], json!(["mcp"]));

    let rules = fs::read_to_string(
        home.path()
            .join(".codeium/windsurf/memories/global_rules.md"),
    )
    .unwrap();
    assert!(rules.contains("<!-- BEGIN IMPACT -->"));
    assert!(rules.contains("impact_index"));
}

#[test]
fn windsurf_scope_project_moves_only_the_rule_into_an_always_on_rule_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "windsurf",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    assert!(home
        .path()
        .join(".codeium/windsurf/mcp_config.json")
        .exists());
    let rule = fs::read_to_string(project.path().join(".windsurf/rules/impact.md")).unwrap();
    assert!(rule.starts_with("---\ntrigger: always_on\n"));
    assert!(rule.contains("impact_change"));
    assert!(!home
        .path()
        .join(".codeium/windsurf/memories/global_rules.md")
        .exists());
}

#[test]
fn windsurf_uninstall_keeps_the_users_own_global_rules() {
    let home = tempfile::tempdir().unwrap();
    let memories = home.path().join(".codeium/windsurf/memories");
    fs::create_dir_all(&memories).unwrap();
    fs::write(
        memories.join("global_rules.md"),
        "Always answer in English.\n",
    )
    .unwrap();
    install_client(home.path(), "windsurf", &[]);

    uninstall_client(home.path(), "windsurf", &[]);

    let rules = fs::read_to_string(memories.join("global_rules.md")).unwrap();
    assert!(rules.contains("Always answer in English."));
    assert!(!rules.contains("<!-- BEGIN IMPACT -->"));
    let config = read_json(&home.path().join(".codeium/windsurf/mcp_config.json"));
    assert!(config["mcpServers"].get("impact").is_none());
}

#[test]
fn client_all_installs_windsurf_when_its_home_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".codeium/windsurf")).unwrap();

    install_client(home.path(), "all", &[]);

    assert!(home
        .path()
        .join(".codeium/windsurf/mcp_config.json")
        .exists());
    let report = doctor(home.path(), &[]);
    assert_eq!(client_status(&report, "windsurf")["rule_current"], true);
}

// ---- OpenCode ----

#[test]
fn install_writes_opencode_local_mcp_entry_and_agents_md_block() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "opencode", &[]);

    let config = read_json(&home.path().join(".config/opencode/opencode.json"));
    let server = &config["mcp"]["impact"];
    assert_eq!(server["type"], "local");
    assert_eq!(server["enabled"], true);
    let command = server["command"].as_array().unwrap();
    assert_eq!(command.len(), 2);
    assert!(Path::new(command[0].as_str().unwrap()).is_absolute());
    assert_eq!(command[1], "mcp");

    let agents_md = fs::read_to_string(home.path().join(".config/opencode/AGENTS.md")).unwrap();
    assert!(agents_md.contains("<!-- BEGIN IMPACT -->"));
}

#[test]
fn opencode_install_and_uninstall_preserve_unrelated_config() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".config/opencode");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("opencode.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet",
            "mcp": {"other-tool": {"type": "remote", "url": "https://example.test/mcp"}}
        }))
        .unwrap(),
    )
    .unwrap();

    install_client(home.path(), "opencode", &[]);
    let report = doctor(home.path(), &["--client", "opencode"]);
    let status = client_status(&report, "opencode");
    assert_eq!(status["configured"], true);
    assert_eq!(status["points_to_expected_binary"], true);

    uninstall_client(home.path(), "opencode", &[]);

    let config = read_json(&dir.join("opencode.json"));
    assert_eq!(config["model"], "anthropic/claude-sonnet");
    assert_eq!(config["mcp"]["other-tool"]["type"], "remote");
    assert!(config["mcp"].get("impact").is_none());
}

#[test]
fn opencode_project_scope_writes_under_project_dir_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "opencode",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    let config = read_json(&project.path().join("opencode.json"));
    assert_eq!(config["mcp"]["impact"]["type"], "local");
    assert!(project.path().join("AGENTS.md").exists());
    assert!(!home.path().join(".config/opencode").exists());
}

#[test]
fn client_all_installs_opencode_when_its_config_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".config/opencode")).unwrap();

    install_client(home.path(), "all", &[]);

    assert!(home.path().join(".config/opencode/opencode.json").exists());
    let report = doctor(home.path(), &[]);
    assert_eq!(client_status(&report, "opencode")["rule_current"], true);
}

// ---- VS Code (GitHub Copilot) ----

fn vscode_user_dir(home: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "windows") {
        home.join("AppData/Roaming/Code/User")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Code/User")
    } else {
        home.join(".config/Code/User")
    }
}

#[test]
fn install_writes_vscode_user_mcp_json_and_instructions_file() {
    let home = tempfile::tempdir().unwrap();

    install_client(home.path(), "vscode", &[]);

    let config = read_json(&vscode_user_dir(home.path()).join("mcp.json"));
    let server = &config["servers"]["impact"];
    assert_eq!(server["type"], "stdio");
    assert_eq!(server["args"], json!(["mcp"]));
    assert!(Path::new(server["command"].as_str().unwrap()).is_absolute());

    let instructions =
        fs::read_to_string(vscode_user_dir(home.path()).join("prompts/impact.instructions.md"))
            .unwrap();
    assert!(instructions.starts_with("---\n"));
    assert!(instructions.contains("applyTo: \"**\""));
    assert!(instructions.contains("impact_index"));
}

#[test]
fn vscode_project_scope_writes_under_project_dir_only() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    run_ok_json(&[
        "install",
        "--client",
        "vscode",
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
        "--home-dir",
        home.path().to_str().unwrap(),
        "--json",
    ]);

    let config = read_json(&project.path().join(".vscode/mcp.json"));
    assert_eq!(config["servers"]["impact"]["type"], "stdio");
    assert!(project
        .path()
        .join(".github/instructions/impact.instructions.md")
        .exists());
    assert!(!vscode_user_dir(home.path()).exists());
}

/// VS Code's `mcp.json` is JSONC: comments in it are the user's, and must survive both
/// an install and an uninstall.
#[test]
fn vscode_install_and_uninstall_keep_comments_and_other_servers() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::create_dir_all(project.path().join(".vscode")).unwrap();
    let original = "{\n  // servers I added by hand\n  \"servers\": {\n    \"other-tool\": { \"type\": \"stdio\", \"command\": \"other\" }, // keep me\n  },\n}\n";
    fs::write(project.path().join(".vscode/mcp.json"), original).unwrap();
    let scope = [
        "--scope",
        "project",
        "--path",
        project.path().to_str().unwrap(),
    ];

    install_client(home.path(), "vscode", &scope);
    let installed = fs::read_to_string(project.path().join(".vscode/mcp.json")).unwrap();
    assert!(installed.contains("// servers I added by hand"));
    assert!(installed.contains("// keep me"));
    assert!(installed.contains("\"impact\""));
    let second = install_client(home.path(), "vscode", &scope);
    assert_eq!(second["changed"], json!([]));
    let mut doctor_args = vec!["--client", "vscode"];
    doctor_args.extend_from_slice(&scope);
    let report = doctor(home.path(), &doctor_args);
    let status = client_status(&report, "vscode");
    assert_eq!(status["configured"], true);
    assert_eq!(status["points_to_expected_binary"], true);

    uninstall_client(home.path(), "vscode", &scope);

    let uninstalled = fs::read_to_string(project.path().join(".vscode/mcp.json")).unwrap();
    assert!(uninstalled.contains("// servers I added by hand"));
    assert!(uninstalled.contains("\"other-tool\""));
    assert!(!uninstalled.contains("\"impact\""));
}

#[test]
fn client_all_installs_vscode_when_its_user_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(vscode_user_dir(home.path())).unwrap();

    install_client(home.path(), "all", &[]);

    assert!(vscode_user_dir(home.path()).join("mcp.json").exists());
    let report = doctor(home.path(), &[]);
    assert_eq!(client_status(&report, "vscode")["rule_current"], true);
}

// ---- Zed ----

fn zed_settings_path(home: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "windows") {
        home.join("AppData/Roaming/Zed/settings.json")
    } else {
        home.join(".config/zed/settings.json")
    }
}

fn install_zed_for_project(home: &Path, project: &Path) -> Value {
    install_client(
        home,
        "zed",
        &["--scope", "project", "--path", project.to_str().unwrap()],
    )
}

/// Zed keeps user-level rules in its own database, not a file, so a user-scope install
/// registers the server and reports no rule at all rather than inventing a file.
#[test]
fn install_writes_zed_context_server_and_no_user_rule_file() {
    let home = tempfile::tempdir().unwrap();

    let report = install_client(home.path(), "zed", &[]);

    let config = read_json(&zed_settings_path(home.path()));
    let server = &config["context_servers"]["impact"];
    assert_eq!(server["args"], json!(["mcp"]));
    assert!(Path::new(server["command"].as_str().unwrap()).is_absolute());
    assert_eq!(report["rule_changed"], json!([]));
    assert_eq!(report["rule_unchanged"], json!([]));

    let doctor_report = doctor(home.path(), &["--client", "zed"]);
    let status = client_status(&doctor_report, "zed");
    assert_eq!(status["configured"], true);
    assert_eq!(status["rule_path"], Value::Null);
}

#[test]
fn zed_install_keeps_comments_in_settings_json() {
    let home = tempfile::tempdir().unwrap();
    let settings = zed_settings_path(home.path());
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        "// Zed settings\n{\n  \"theme\": \"One Dark\", // my theme\n}\n",
    )
    .unwrap();

    install_client(home.path(), "zed", &[]);
    let installed = fs::read_to_string(&settings).unwrap();
    assert!(installed.contains("// my theme"));
    assert!(installed.contains("\"context_servers\""));

    uninstall_client(home.path(), "zed", &[]);
    let uninstalled = fs::read_to_string(&settings).unwrap();
    assert!(uninstalled.contains("// Zed settings"));
    assert!(uninstalled.contains("\"theme\": \"One Dark\""));
    assert!(!uninstalled.contains("\"impact\""));
}

#[test]
fn zed_project_scope_creates_agents_md_when_the_project_has_no_rules_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    install_zed_for_project(home.path(), project.path());

    let agents_md = fs::read_to_string(project.path().join("AGENTS.md")).unwrap();
    assert!(agents_md.contains("<!-- BEGIN IMPACT -->"));
    assert!(zed_settings_path(home.path()).exists());
}

/// Zed reads only the first rules file it finds (`.rules` before `AGENTS.md`), so the
/// block has to go into the one Zed will actually read.
#[test]
fn zed_project_scope_writes_into_the_rules_file_zed_reads_first() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join(".rules"), "Use tabs.\n").unwrap();
    fs::write(project.path().join("AGENTS.md"), "# Agents\n").unwrap();

    install_zed_for_project(home.path(), project.path());

    let rules = fs::read_to_string(project.path().join(".rules")).unwrap();
    assert!(rules.contains("Use tabs."));
    assert!(rules.contains("<!-- BEGIN IMPACT -->"));
    assert_eq!(
        fs::read_to_string(project.path().join("AGENTS.md")).unwrap(),
        "# Agents\n"
    );
}

#[test]
fn client_all_installs_zed_when_its_config_directory_exists() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(zed_settings_path(home.path()).parent().unwrap()).unwrap();

    install_client(home.path(), "all", &[]);

    assert!(zed_settings_path(home.path()).exists());
    let report = doctor(home.path(), &[]);
    assert_eq!(client_status(&report, "zed")["configured"], true);
}
