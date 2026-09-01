//! Per-client config-file and rule-file path resolution.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::{Client, InstallOptions, Scope};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleKind {
    /// Cursor's `.mdc` file: entirely owned by `impact` (written/deleted whole).
    Standalone,
    /// A delimited block inserted into a file the user may also edit.
    ManagedBlock,
}

pub struct RuleTarget {
    pub path: PathBuf,
    pub kind: RuleKind,
}

fn require_project_dir(options: &InstallOptions) -> Result<&Path> {
    options
        .project_dir
        .as_deref()
        .ok_or_else(|| anyhow!("--path is required for project-scope installs"))
}

/// Resolves where the MCP server registration lives for `client`.
///
/// Codex and Claude have no project-scoped MCP registration mechanism in this tool's
/// v1 design — the whole point of `impact install` is to make the server available in
/// *every* project automatically, so their MCP entry is always written to the user's
/// home config regardless of `--scope`. Only their rule file honors `--scope project`
/// (see `rule_target`). Cursor's `mcp.json` is genuinely scope-sensitive.
pub fn config_path(options: &InstallOptions, client: Client) -> Result<PathBuf> {
    match client {
        Client::Cursor => match options.scope {
            Scope::User => Ok(options.home_dir.join(".cursor").join("mcp.json")),
            Scope::Project => Ok(require_project_dir(options)?
                .join(".cursor")
                .join("mcp.json")),
        },
        Client::Codex => Ok(options.home_dir.join(".codex").join("config.toml")),
        // Claude Code CLI reads `~/.claude.json` — a file, not a `~/.claude/` directory.
        Client::Claude => Ok(options.home_dir.join(".claude.json")),
        Client::ClaudeDesktop => claude_desktop_config_path(&options.home_dir),
    }
}

pub fn rule_target(options: &InstallOptions, client: Client) -> Result<RuleTarget> {
    let path = match (client, options.scope) {
        (Client::Cursor, Scope::User) => options
            .home_dir
            .join(".cursor")
            .join("rules")
            .join("impact.mdc"),
        (Client::Cursor, Scope::Project) => require_project_dir(options)?
            .join(".cursor")
            .join("rules")
            .join("impact.mdc"),
        (Client::Codex, Scope::User) => options.home_dir.join(".codex").join("AGENTS.md"),
        (Client::Codex, Scope::Project) => require_project_dir(options)?.join("AGENTS.md"),
        (Client::Claude, Scope::User) => options.home_dir.join(".claude").join("CLAUDE.md"),
        (Client::Claude, Scope::Project) => require_project_dir(options)?.join("CLAUDE.md"),
        (Client::ClaudeDesktop, _) => options.home_dir.join(".claude").join("CLAUDE.md"),
    };
    let kind = match client {
        Client::Cursor => RuleKind::Standalone,
        Client::Codex | Client::Claude | Client::ClaudeDesktop => RuleKind::ManagedBlock,
    };
    Ok(RuleTarget { path, kind })
}

/// Derived entirely from `home_dir` (rather than reading `%APPDATA%`/native APIs
/// directly) so that `--home-dir` reliably redirects it in tests and non-standard
/// profile setups alike.
pub fn claude_desktop_config_path(home_dir: &Path) -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        Ok(home_dir
            .join("AppData")
            .join("Roaming")
            .join("Claude")
            .join("claude_desktop_config.json"))
    } else if cfg!(target_os = "macos") {
        Ok(home_dir
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude_desktop_config.json"))
    } else {
        Err(anyhow!("claude-desktop is not supported on this platform"))
    }
}
