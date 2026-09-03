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
