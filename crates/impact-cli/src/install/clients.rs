//! Per-client config-file and rule-file path resolution.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::rule::{
    CURSOR_FRONT_MATTER, KIRO_FRONT_MATTER, VSCODE_FRONT_MATTER, WINDSURF_FRONT_MATTER,
};
use super::{Client, InstallOptions, Scope};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleKind {
    /// A file entirely owned by `impact` (written/deleted whole) — Cursor's `.mdc`, Kiro's
    /// steering file — opening with the client's own front matter.
    Standalone { front_matter: &'static str },
    /// A delimited block inserted into a file the user may also edit.
    ManagedBlock,
}

/// How a client's MCP config file is laid out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigFormat {
    /// Codex's `config.toml`: an `[mcp_servers.impact]` table.
    CodexToml,
    /// Plain JSON, parsed and rewritten whole.
    Json(JsonLayout),
    /// JSON with comments, edited in place so the user's comments and formatting survive.
    Jsonc(JsonLayout),
}

/// Where a JSON config keeps its MCP servers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonLayout {
    /// An `mcpServers.impact` object with `command`/`args` — what most clients share.
    McpServers,
    /// OpenCode's `mcp.impact` object, with the arguments inside `command`.
    OpenCode,
    /// VS Code's `servers.impact` object, which also names the transport (`stdio`).
    VsCode,
}

pub fn config_format(client: Client) -> ConfigFormat {
    match client {
        Client::Codex => ConfigFormat::CodexToml,
        Client::Cursor
        | Client::Claude
        | Client::ClaudeDesktop
        | Client::Gemini
        | Client::Kiro
        | Client::Windsurf => ConfigFormat::Json(JsonLayout::McpServers),
        Client::OpenCode => ConfigFormat::Json(JsonLayout::OpenCode),
        Client::VsCode => ConfigFormat::Jsonc(JsonLayout::VsCode),
    }
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

/// A client's dot-directory (`.gemini`, `.kiro`) — in the home directory for user scope,
/// in the project for project scope.
fn scoped_dir(options: &InstallOptions, name: &str) -> Result<PathBuf> {
    match options.scope {
        Scope::User => Ok(options.home_dir.join(name)),
        Scope::Project => Ok(require_project_dir(options)?.join(name)),
    }
}

/// Resolves where the MCP server registration lives for `client`.
///
/// Codex, Claude and Windsurf have no project-scoped MCP registration mechanism in this tool's
/// v1 design — the whole point of `impact install` is to make the server available in
/// *every* project automatically, so their MCP entry is always written to the user's
/// home config regardless of `--scope`. Only their rule file honors `--scope project`
/// (see `rule_target`). Cursor's, Gemini CLI's and Kiro's configs are genuinely
/// scope-sensitive.
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
        Client::Gemini => Ok(scoped_dir(options, ".gemini")?.join("settings.json")),
        Client::Kiro => Ok(scoped_dir(options, ".kiro")?
            .join("settings")
            .join("mcp.json")),
        Client::Windsurf => Ok(windsurf_home(&options.home_dir).join("mcp_config.json")),
        Client::OpenCode => match options.scope {
            Scope::User => Ok(opencode_home(&options.home_dir).join("opencode.json")),
            Scope::Project => Ok(require_project_dir(options)?.join("opencode.json")),
        },
        Client::VsCode => match options.scope {
            Scope::User => Ok(vscode_user_dir(&options.home_dir).join("mcp.json")),
            Scope::Project => Ok(require_project_dir(options)?
                .join(".vscode")
                .join("mcp.json")),
        },
    }
}

/// VS Code's user-profile directory, derived from `home_dir` like Claude Desktop's config
/// so `--home-dir` redirects it too.
fn vscode_user_dir(home_dir: &Path) -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        home_dir.join("AppData").join("Roaming")
    } else if cfg!(target_os = "macos") {
        home_dir.join("Library").join("Application Support")
    } else {
        home_dir.join(".config")
    };
    base.join("Code").join("User")
}

/// OpenCode reads its global config from `~/.config/opencode` on every platform,
/// Windows included.
fn opencode_home(home_dir: &Path) -> PathBuf {
    home_dir.join(".config").join("opencode")
}

fn windsurf_home(home_dir: &Path) -> PathBuf {
    home_dir.join(".codeium").join("windsurf")
}

/// Whether `client`'s own home directory exists — the signal that the user actually
/// runs it, used to decide whether `--client all` should reach an opt-in client.
pub fn is_present(home_dir: &Path, client: Client) -> bool {
    match client {
        Client::Gemini => home_dir.join(".gemini").is_dir(),
        Client::Kiro => home_dir.join(".kiro").is_dir(),
        Client::Windsurf => windsurf_home(home_dir).is_dir(),
        Client::OpenCode => opencode_home(home_dir).is_dir(),
        Client::VsCode => vscode_user_dir(home_dir).is_dir(),
        Client::Cursor | Client::Codex | Client::Claude | Client::ClaudeDesktop => true,
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
        (Client::Gemini, Scope::User) => options.home_dir.join(".gemini").join("GEMINI.md"),
        (Client::Gemini, Scope::Project) => require_project_dir(options)?.join("GEMINI.md"),
        (Client::Kiro, _) => scoped_dir(options, ".kiro")?
            .join("steering")
            .join("impact.md"),
        // Windsurf has one global rules file shared with the user's own rules, but a
        // directory of separate rule files per workspace.
        (Client::Windsurf, Scope::User) => windsurf_home(&options.home_dir)
            .join("memories")
            .join("global_rules.md"),
        (Client::Windsurf, Scope::Project) => require_project_dir(options)?
            .join(".windsurf")
            .join("rules")
            .join("impact.md"),
        (Client::OpenCode, Scope::User) => opencode_home(&options.home_dir).join("AGENTS.md"),
        (Client::OpenCode, Scope::Project) => require_project_dir(options)?.join("AGENTS.md"),
        // An instructions file of its own rather than a block in
        // `.github/copilot-instructions.md`, which VS Code only reads behind a setting.
        (Client::VsCode, Scope::User) => vscode_user_dir(&options.home_dir)
            .join("prompts")
            .join("impact.instructions.md"),
        (Client::VsCode, Scope::Project) => require_project_dir(options)?
            .join(".github")
            .join("instructions")
            .join("impact.instructions.md"),
    };
    let kind = match (client, options.scope) {
        (Client::Cursor, _) => RuleKind::Standalone {
            front_matter: CURSOR_FRONT_MATTER,
        },
        (Client::Kiro, _) => RuleKind::Standalone {
            front_matter: KIRO_FRONT_MATTER,
        },
        (Client::Windsurf, Scope::Project) => RuleKind::Standalone {
            front_matter: WINDSURF_FRONT_MATTER,
        },
        (Client::VsCode, _) => RuleKind::Standalone {
            front_matter: VSCODE_FRONT_MATTER,
        },
        (
            Client::Codex
            | Client::Claude
            | Client::ClaudeDesktop
            | Client::Gemini
            | Client::Windsurf
            | Client::OpenCode,
            _,
        ) => RuleKind::ManagedBlock,
    };
    Ok(RuleTarget { path, kind })
}

/// Where `client`'s tool-call hooks live, if `install` wires one in at all. Claude Code
/// is the only client that gets a hook; the others get the MCP server and rule only.
pub fn hook_target(options: &InstallOptions, client: Client) -> Result<Option<PathBuf>> {
    let path = match (client, options.scope) {
        (Client::Claude, Scope::User) => options.home_dir.join(".claude").join("settings.json"),
        (Client::Claude, Scope::Project) => require_project_dir(options)?
            .join(".claude")
            .join("settings.json"),
        _ => return Ok(None),
    };
    Ok(Some(path))
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
