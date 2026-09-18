//! Blind-spot reporting: composes a GitHub issue draft describing a case where `impact`
//! missed or misreported something, for `impact report-blindspot` (and its MCP
//! equivalent). Composition here is pure and offline — no network, no `gh` invocation —
//! so a caller can always see the exact draft before anything is ever sent anywhere.
//! Submission is a separate, explicit step (see the CLI's `--submit` flag).

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
    let body = format!(
        "**Kind:** {}\n{language_line}\n{}\n\n<!-- impact-blindspot-fp:{fingerprint} -->",
        kind.as_str(),
        body.trim(),
    );

    BlindspotDraft {
        repo: repo.to_string(),
        title: title.to_string(),
        body,
        fingerprint,
    }
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
