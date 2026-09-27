//! Release check: tells the person using `impact` when a newer release is out, and how
//! to get it. It never downloads or installs anything. Upgrading stays a step the user
//! runs themselves.
//!
//! At most one request a day goes to GitHub's "latest release" endpoint, with a short
//! timeout. The answer is cached in `~/.impact/update-check.json`. An unreachable or
//! slow endpoint is silent: the command that triggered the check carries on as if it
//! never ran. Set `IMPACT_NO_UPDATE_CHECK` (any value) to turn it off. It is also off
//! whenever `CI` is set. `IMPACT_UPDATE_URL` and `IMPACT_UPDATE_STATE` relocate the
//! endpoint and the cache file, which is how the tests point it at a local server.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/AncientiCe/impact-rs/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/AncientiCe/impact-rs/releases/latest";
const INSTALL_SH: &str =
    "curl -fsSL https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.sh | sh";
const INSTALL_PS1: &str =
    "irm https://raw.githubusercontent.com/AncientiCe/impact-rs/master/scripts/install.ps1 | iex";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const TIMEOUT: Duration = Duration::from_secs(2);
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What `update-check.json` holds between runs.
#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Unix seconds of the last request to the release endpoint, successful or not, so an
    /// offline machine doesn't retry on every command.
    checked_at: u64,
    /// The newest release version seen, without the leading `v`.
    latest: Option<String>,
    /// Unix seconds the CLI last printed the notice.
    notified_at: u64,
}

fn disabled() -> bool {
    std::env::var_os("IMPACT_NO_UPDATE_CHECK").is_some() || std::env::var_os("CI").is_some()
}

fn state_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("IMPACT_UPDATE_STATE") {
        return Some(PathBuf::from(path));
    }
    let base = directories::BaseDirs::new()?;
    Some(base.home_dir().join(".impact").join("update-check.json"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load(path: &Path) -> State {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Best-effort: a cache that can't be written only means the next run checks again.
fn save(path: &Path, state: &State) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(state) {
        let _ = std::fs::write(path, bytes);
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

fn fetch_latest() -> Option<String> {
    let url = std::env::var("IMPACT_UPDATE_URL").unwrap_or_else(|_| LATEST_RELEASE_URL.to_string());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .header("User-Agent", concat!("impact/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let body = response.body_mut().read_to_string().ok()?;
    let release: Release = serde_json::from_str(&body).ok()?;
    Some(release.tag_name.trim_start_matches('v').to_string())
}

/// The cached state, first refreshed from the release endpoint if the last check is a
/// day old or more.
fn refreshed(path: &Path) -> State {
    let mut state = load(path);
    let now = now();
    if now.saturating_sub(state.checked_at) >= CHECK_INTERVAL_SECS {
        if let Some(latest) = fetch_latest() {
            state.latest = Some(latest);
        }
        state.checked_at = now;
        save(path, &state);
    }
    state
}

/// `major.minor.patch`, or `None` for anything else, pre-releases included, so a
/// release candidate is never announced as an upgrade.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// How to upgrade, judged from where the running binary lives: a Homebrew prefix, the
/// install scripts' default directory, or neither.
fn upgrade_hint() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok().or(Some(p)));
    let Some(exe) = exe else {
        return format!("download it from {RELEASES_PAGE}");
    };
    let lowered = exe.to_string_lossy().to_lowercase();
    if lowered.contains("cellar") || lowered.contains("homebrew") {
        return "brew upgrade impact".to_string();
    }
    let script_dir = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|base| {
            PathBuf::from(base)
                .join("Programs")
                .join("impact")
                .join("bin")
        })
    } else {
        directories::BaseDirs::new().map(|base| base.home_dir().join(".local").join("bin"))
    };
    let script_dir = script_dir.map(|dir| dir.canonicalize().unwrap_or(dir));
    if exe.parent().is_some() && exe.parent() == script_dir.as_deref() {
        let command = if cfg!(windows) {
            INSTALL_PS1
        } else {
            INSTALL_SH
        };
        return format!("re-run the installer: {command}");
    }
    format!("download it from {RELEASES_PAGE}")
}

fn message(latest: &str) -> String {
    format!(
        "impact {latest} is available (you have {CURRENT}). To upgrade, {}",
        upgrade_hint()
    )
}

/// The notice the CLI prints on stderr after a command, when a newer release is known
/// and the notice hasn't already been shown in the last day.
pub fn cli_notice() -> Option<String> {
    if disabled() {
        return None;
    }
    let path = state_path()?;
    let mut state = refreshed(&path);
    let latest = state.latest.clone()?;
    if !is_newer(&latest, CURRENT) {
        return None;
    }
    let now = now();
    if now.saturating_sub(state.notified_at) < CHECK_INTERVAL_SECS {
        return None;
    }
    state.notified_at = now;
    save(&path, &state);
    Some(message(&latest))
}

/// The notice the MCP server attaches to a tool result, when a newer release is known.
/// The server shows it once per session. It is addressed to the user, not the agent:
/// upgrading changes the user's installed toolchain, which is theirs to decide.
pub fn mcp_notice() -> Option<String> {
    if disabled() {
        return None;
    }
    let state = refreshed(&state_path()?);
    let latest = state.latest?;
    if !is_newer(&latest, CURRENT) {
        return None;
    }
    Some(format!(
        "{}. This notice is for the user: tell the user, and don't run the upgrade yourself.",
        message(&latest)
    ))
}

/// The newest release an earlier check already cached, if it is newer than this binary.
/// Reads the cache only and never the network, for callers that promise not to make a
/// request, like drafting a blind-spot report.
pub fn known_newer_release() -> Option<String> {
    if disabled() {
        return None;
    }
    let latest = load(&state_path()?).latest?;
    is_newer(&latest, CURRENT).then_some(latest)
}

/// What a blind-spot draft says when `known_newer_release` found one.
pub fn blindspot_note(latest: &str) -> String {
    format!(
        "impact {latest} is available (you have {CURRENT}). Check whether this is already fixed in {latest} before filing."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_component_is_newer() {
        assert!(is_newer("0.11.7", "0.11.6"));
        assert!(is_newer("0.12.0", "0.11.6"));
        assert!(is_newer("1.0.0", "0.11.6"));
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        assert!(is_newer("0.11.10", "0.11.9"));
        assert!(!is_newer("0.11.9", "0.11.10"));
    }

    #[test]
    fn the_same_or_an_older_version_is_not_newer() {
        assert!(!is_newer("0.11.6", "0.11.6"));
        assert!(!is_newer("0.10.0", "0.11.6"));
    }

    #[test]
    fn pre_releases_and_garbage_are_never_newer() {
        assert!(!is_newer("0.12.0-rc.1", "0.11.6"));
        assert!(!is_newer("latest", "0.11.6"));
        assert!(!is_newer("1.2", "0.11.6"));
    }
}
