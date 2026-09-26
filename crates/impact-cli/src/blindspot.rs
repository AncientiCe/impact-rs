//! Blind-spot reporting: composes a GitHub issue draft describing a case where `impact`
//! missed or misreported something, for `impact report-blindspot` (and its MCP
//! equivalent). Composition here is pure and offline — no network, no `gh` invocation —
//! so a caller can always see the exact draft before anything is ever sent anywhere.
//! Submission is a separate, explicit step (see the CLI's `--submit` flag).

use anyhow::Context;
use clap::ValueEnum;

/// What kind of gap was found. Mirrors the "Blind spot:" callouts already baked into
/// every MCP tool description in `mcp.rs` (a missed edge through indirection, one that
/// shouldn't exist at all, an outright crash) plus a catch-all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BlindspotKind {
    /// `impact` failed to report a real caller/dependent that exists in the code.
    MissedEdge,
    /// `impact` reported a caller/dependent that doesn't actually exist.
    FalsePositive,
    /// `impact` panicked, hung, or otherwise failed to produce a report at all.
    Crash,
    /// Doesn't fit the above.
    Other,
}

impl BlindspotKind {
    fn as_str(self) -> &'static str {
        match self {
            BlindspotKind::MissedEdge => "missed-edge",
            BlindspotKind::FalsePositive => "false-positive",
            BlindspotKind::Crash => "crash",
            BlindspotKind::Other => "other",
        }
    }
}

/// A composed, ready-to-file (or already-filed) issue.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BlindspotDraft {
    pub repo: String,
    pub title: String,
    /// The caller-supplied description plus the trailing fingerprint comment.
    pub body: String,
    /// Hex-encoded, truncated to 16 characters — stable for identical
    /// `(kind, language, title)` inputs, so a re-run of the same report can find its own
    /// earlier issue via `gh issue list --search`.
    pub fingerprint: String,
}

const FINGERPRINT_LEN: usize = 16;

/// Builds the draft. `title` is normalized (trimmed, lowercased) before hashing so
/// cosmetic differences ("Fix It" vs "fix it") still collide onto the same fingerprint;
/// the title stored on the draft keeps the caller's original casing.
pub fn compose_draft(
    title: &str,
    body: &str,
    kind: BlindspotKind,
    language: Option<&str>,
    repo: &str,
) -> BlindspotDraft {
    let normalized_title = title.trim().to_lowercase();
    let language_key = language.unwrap_or("").trim().to_lowercase();
    let fingerprint_input = format!("{}|{language_key}|{normalized_title}", kind.as_str());
    let hash = blake3::hash(fingerprint_input.as_bytes());
    let fingerprint = hash.to_hex()[..FINGERPRINT_LEN].to_string();

    let language_line = language
        .map(|l| format!("**Language:** {l}\n"))
        .unwrap_or_default();
    // Stamped from the binary, not asked of the reporting agent: a gap is only
    // actionable against the release it was seen in, and it may already be fixed in a
    // later one. Deliberately left out of the fingerprint, so the same gap re-reported
    // from a newer version still finds its earlier issue instead of filing a duplicate.
    let body = format!(
        "**Kind:** {}\n{language_line}**impact version:** {}\n\n{}\n\n<!-- impact-blindspot-fp:{fingerprint} -->",
        kind.as_str(),
        env!("CARGO_PKG_VERSION"),
        body.trim(),
    );

    BlindspotDraft {
        repo: repo.to_string(),
        title: title.to_string(),
        body,
        fingerprint,
    }
}

/// An issue already on file, found via `find_existing`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ExistingIssue {
    pub number: u64,
    pub url: String,
    pub title: String,
}

/// The `gh` binary to shell out to — overridable via `IMPACT_GH_BIN` so tests can point
/// it at a fake script instead of touching the real `gh`/GitHub. Defaults to `"gh"`,
/// resolved from `PATH` like any other command.
fn gh_bin() -> String {
    std::env::var("IMPACT_GH_BIN").unwrap_or_else(|_| "gh".to_string())
}

/// Searches `repo` for an issue whose body already carries `fingerprint`, so `submit`
/// never files a duplicate of a gap that's already reported. Returns the first match, if
/// any; `Ok(None)` means the search ran cleanly and found nothing.
pub fn find_existing(repo: &str, fingerprint: &str) -> anyhow::Result<Option<ExistingIssue>> {
    let output = std::process::Command::new(gh_bin())
        .args(["issue", "list", "--repo", repo, "--state", "all"])
        .arg("--search")
        .arg(format!("{fingerprint} in:body"))
        .args(["--json", "number,url,title"])
        .output()
        .context("running `gh issue list` (is `gh` installed?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "`gh issue list` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let issues: Vec<ExistingIssue> =
        serde_json::from_slice(&output.stdout).context("parsing `gh issue list` output")?;
    Ok(issues.into_iter().next())
}

/// Files `draft` as a new issue via `gh issue create`, returning the created issue's
/// URL. Callers should run `find_existing` first — this never checks for duplicates
/// itself.
pub fn submit(draft: &BlindspotDraft) -> anyhow::Result<String> {
    let output = std::process::Command::new(gh_bin())
        .args(["issue", "create", "--repo", &draft.repo])
        .arg("--title")
        .arg(&draft.title)
        .arg("--body")
        .arg(&draft.body)
        .output()
        .context("running `gh issue create` (is `gh` installed and authenticated?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "`gh issue create` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        anyhow::bail!("`gh issue create` produced no output");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_produce_the_same_fingerprint() {
        let a = compose_draft(
            "misses an edge",
            "body text",
            BlindspotKind::MissedEdge,
            Some("swift"),
            "AncientiCe/impact-rs",
        );
        let b = compose_draft(
            "misses an edge",
            "different body text",
            BlindspotKind::MissedEdge,
            Some("swift"),
            "AncientiCe/impact-rs",
        );
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn title_casing_and_whitespace_dont_change_the_fingerprint() {
        let a = compose_draft("Misses An Edge", "b", BlindspotKind::MissedEdge, None, "r");
        let b = compose_draft(
            "  misses an edge  ",
            "b",
            BlindspotKind::MissedEdge,
            None,
            "r",
        );
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn different_kind_changes_the_fingerprint() {
        let a = compose_draft("same title", "b", BlindspotKind::MissedEdge, None, "r");
        let b = compose_draft("same title", "b", BlindspotKind::FalsePositive, None, "r");
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn different_language_changes_the_fingerprint() {
        let a = compose_draft(
            "same title",
            "b",
            BlindspotKind::MissedEdge,
            Some("swift"),
            "r",
        );
        let b = compose_draft(
            "same title",
            "b",
            BlindspotKind::MissedEdge,
            Some("go"),
            "r",
        );
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn body_embeds_the_fingerprint_comment() {
        let draft = compose_draft("t", "b", BlindspotKind::Other, None, "r");
        assert!(draft.body.contains(&format!(
            "<!-- impact-blindspot-fp:{} -->",
            draft.fingerprint
        )));
    }
}
