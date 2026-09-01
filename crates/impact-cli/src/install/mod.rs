//! Registers (or removes) the `impact` MCP server, plus an agent rule telling the agent
//! to actually call it before/after editing code, in local AI coding tools — so
//! blast-radius verification works in every project automatically, not just one wired
//! up by hand.

mod clients;
mod config_io;
mod rule;

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use clients::{RuleKind, RuleTarget};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
    Cursor,
    Codex,
    Claude,
    ClaudeDesktop,
}

impl Client {
    pub fn name(self) -> &'static str {
        match self {
            Client::Cursor => "cursor",
            Client::Codex => "codex",
            Client::Claude => "claude",
            Client::ClaudeDesktop => "claude-desktop",
        }
    }
}

impl FromStr for Client {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "cursor" => Ok(Client::Cursor),
            "codex" => Ok(Client::Codex),
            "claude" | "claude-code" => Ok(Client::Claude),
            "claude-desktop" => Ok(Client::ClaudeDesktop),
            other => Err(anyhow!(
                "unknown MCP client '{other}' (expected cursor, codex, claude, claude-desktop, or all)"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    User,
    Project,
}

impl FromStr for Scope {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Scope::User),
            "project" => Ok(Scope::Project),
            other => Err(anyhow!(
                "unknown install scope '{other}' (expected user or project)"
            )),
        }
    }
}

/// Expands `--client`'s raw string into concrete clients — `"all"` becomes every
/// client supported on the current platform (Claude Desktop is Windows/macOS only).
fn parse_clients(value: &str) -> Result<Vec<Client>> {
    if value == "all" {
        let mut all = vec![Client::Cursor, Client::Codex, Client::Claude];
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            all.push(Client::ClaudeDesktop);
        }
        return Ok(all);
    }
    Ok(vec![value.parse()?])
}

#[derive(Clone, Debug)]
pub struct InstallOptions {
    pub clients: Vec<Client>,
    pub scope: Scope,
    pub project_dir: Option<PathBuf>,
    pub home_dir: PathBuf,
    pub binary_path: PathBuf,
    pub dry_run: bool,
    pub install_rule: bool,
}

impl InstallOptions {
    fn for_current_process(
        clients: Vec<Client>,
        scope: Scope,
        project_dir: Option<PathBuf>,
        home_dir_override: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            clients,
            scope,
            project_dir,
            home_dir: resolve_home_dir(home_dir_override)?,
            binary_path: std::env::current_exe().context("failed to resolve current executable")?,
            dry_run: false,
            install_rule: true,
        })
    }
}

/// Resolves the home directory `install`/`uninstall`/`doctor` operate relative to.
/// `--home-dir` (if passed) wins outright — it's a real, documented flag (not just a
/// test seam) for portable installs or unusual environments where the OS-native lookup
/// is wrong. Otherwise prefers the `directories` crate's native resolution (reliable on
/// Windows, where `$HOME` is often unset), falling back to `$HOME`/`%USERPROFILE%`.
fn resolve_home_dir(home_dir_override: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = home_dir_override {
        return Ok(dir);
    }
    if let Some(dirs) = directories::UserDirs::new() {
        return Ok(dirs.home_dir().to_path_buf());
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(profile));
    }
    Err(anyhow!(
        "could not determine home directory; pass --home-dir explicitly"
    ))
}

/// Builds `InstallOptions` from the raw CLI strings shared by `install`/`uninstall`/
/// `doctor`. `dry_run`/`install_rule` default to `false`/`true` here; callers override
/// them from their own flags.
pub fn build_options(
    client: &str,
    scope: &str,
    path: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Result<InstallOptions> {
    let clients = parse_clients(client)?;
    let scope: Scope = scope.parse()?;
    let project_dir = match (scope, path) {
        (Scope::Project, Some(path)) => Some(path),
        (Scope::Project, None) => Some(std::env::current_dir()?),
        (Scope::User, path) => path,
    };
    InstallOptions::for_current_process(clients, scope, project_dir, home_dir)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct InstallReport {
    pub changed: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub rule_changed: Vec<PathBuf>,
    pub rule_unchanged: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClientStatus {
    pub client: Client,
    pub path: PathBuf,
    pub configured: bool,
    pub points_to_expected_binary: bool,
    pub command: Option<String>,
    pub rule_path: PathBuf,
    pub rule_installed: bool,
    pub rule_current: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DoctorReport {
    pub clients: Vec<ClientStatus>,
}

fn install_config(options: &InstallOptions, client: Client) -> Result<(PathBuf, bool)> {
    let path = clients::config_path(options, client)?;
    let changed = match client {
        Client::Codex => {
            let existing = config_io::read_toml_document(&path)?;
            let mut next = existing.clone();
            config_io::ensure_toml_server(&mut next, &options.binary_path);
            config_io::write_toml_if_changed(&path, &existing, &next, options.dry_run)?
        }
        Client::Cursor | Client::Claude | Client::ClaudeDesktop => {
            let existing = config_io::read_json_config(&path)?;
            let mut next = existing.clone();
            config_io::ensure_json_server(&mut next, &options.binary_path)?;
            config_io::write_json_if_changed(&path, &existing, &next, options.dry_run)?
        }
    };
    Ok((path, changed))
}

fn uninstall_config(options: &InstallOptions, client: Client) -> Result<(PathBuf, bool)> {
    let path = clients::config_path(options, client)?;
    if !path.exists() {
        return Ok((path, false));
    }
    let changed = match client {
        Client::Codex => {
            let existing = config_io::read_toml_document(&path)?;
            let mut next = existing.clone();
            if !config_io::remove_toml_server(&mut next) {
                false
            } else {
                config_io::write_toml_if_changed(&path, &existing, &next, options.dry_run)?
            }
        }
        Client::Cursor | Client::Claude | Client::ClaudeDesktop => {
            let existing = config_io::read_json_config(&path)?;
            let mut next = existing.clone();
            if !config_io::remove_json_server(&mut next) {
                false
            } else {
                config_io::write_json_if_changed(&path, &existing, &next, options.dry_run)?
            }
        }
    };
    Ok((path, changed))
}

fn expected_rule_text(target: &RuleTarget, existing: &str) -> String {
    match target.kind {
        RuleKind::Standalone => rule::cursor_rule_text(),
        RuleKind::ManagedBlock => rule::upsert_managed_rule(existing),
    }
}

fn install_rule(target: &RuleTarget, dry_run: bool) -> Result<bool> {
    let existing = config_io::read_text_file(&target.path)?;
    let next = expected_rule_text(target, &existing);
    config_io::write_text_if_changed(&target.path, &existing, &next, dry_run)
}

fn uninstall_rule(target: &RuleTarget, dry_run: bool) -> Result<bool> {
    match target.kind {
        RuleKind::Standalone => config_io::remove_file_if_exists(&target.path, dry_run),
        RuleKind::ManagedBlock => {
            if !target.path.exists() {
                return Ok(false);
            }
            let existing = config_io::read_text_file(&target.path)?;
            let next = rule::remove_managed_rule(&existing);
            config_io::write_text_if_changed(&target.path, &existing, &next, dry_run)
        }
    }
}

pub fn install_clients(options: &InstallOptions) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    for &client in &options.clients {
        let (path, changed) = install_config(options, client)?;
        if changed {
            report.changed.push(path);
        } else {
            report.unchanged.push(path);
        }

        if options.install_rule {
            let target = clients::rule_target(options, client)?;
            let rule_changed = install_rule(&target, options.dry_run)?;
            if rule_changed {
                report.rule_changed.push(target.path);
            } else {
                report.rule_unchanged.push(target.path);
            }
        }
    }
    Ok(report)
}

pub fn uninstall_clients(options: &InstallOptions) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    for &client in &options.clients {
        let (path, changed) = uninstall_config(options, client)?;
        if changed {
            report.changed.push(path);
        } else {
            report.unchanged.push(path);
        }

        if options.install_rule {
            let target = clients::rule_target(options, client)?;
            let rule_changed = uninstall_rule(&target, options.dry_run)?;
            if rule_changed {
                report.rule_changed.push(target.path);
            } else {
                report.rule_unchanged.push(target.path);
            }
        }
    }
    Ok(report)
}

fn configured_command(client: Client, path: &std::path::Path) -> Result<Option<String>> {
    let command = match client {
        Client::Codex => {
            let doc = config_io::read_toml_document(path)?;
            doc.get("mcp_servers")
                .and_then(|servers| servers.get("impact"))
                .and_then(|server| server.get("command"))
                .and_then(|value| value.as_str())
                .map(String::from)
        }
        Client::Cursor | Client::Claude | Client::ClaudeDesktop => {
            let config = config_io::read_json_config(path)?;
            config
                .get("mcpServers")
                .and_then(|servers| servers.get("impact"))
                .and_then(|server| server.get("command"))
                .and_then(|value| value.as_str())
                .map(String::from)
        }
    };
    Ok(command)
}

pub fn doctor(options: &InstallOptions) -> Result<DoctorReport> {
    let mut clients_status = Vec::with_capacity(options.clients.len());
    for &client in &options.clients {
        let path = clients::config_path(options, client)?;
        let command = configured_command(client, &path)?;
        let points_to_expected_binary =
            command.as_deref() == Some(config_io::path_to_string(&options.binary_path).as_str());

        let target = clients::rule_target(options, client)?;
        let rule_installed = target.path.exists();
        let rule_current = if rule_installed {
            let existing = config_io::read_text_file(&target.path)?;
            existing == expected_rule_text(&target, &existing)
        } else {
            false
        };

        clients_status.push(ClientStatus {
            client,
            configured: command.is_some(),
            points_to_expected_binary,
            command,
            path,
            rule_path: target.path,
            rule_installed,
            rule_current,
        });
    }
    Ok(DoctorReport {
        clients: clients_status,
    })
}
