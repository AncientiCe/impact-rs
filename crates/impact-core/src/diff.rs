//! Parses a unified diff (`git diff` output, or any tool producing the same format) into
//! per-file touched line ranges on the *new* side of the diff — the side that matches
//! what's on disk (and therefore what was indexed) when the diff represents uncommitted
//! working-tree changes, which is the expected way this gets used: `git diff | impact
//! diff`, or an agent handing its about-to-apply patch straight to `impact_diff`.

use std::collections::HashMap;
use std::ops::Range;

/// Per-file touched line ranges, 1-indexed and half-open (`start..end`), one entry per
/// hunk — taken directly from each hunk's new-file range (`@@ -.. +start,count @@`), not
/// merged or deduplicated, since `compute_diff_impact` only cares about coverage, not
/// hunk boundaries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffTouches {
    pub files: HashMap<String, Vec<Range<usize>>>,
}

/// Parses unified diff text into per-file touched line ranges. Recognizes standard `git
/// diff` headers (`+++ b/path`, stripping the leading `a/`/`b/` git always adds) and `@@
/// -oldStart,oldCount +newStart,newCount @@` hunk headers. Everything else (context lines,
/// `-`/`+` content itself, a binary-file notice, `diff --git` lines) is silently skipped —
/// a real diff contains plenty of lines this doesn't need to understand, and an unparsed
/// line is not an error, just not a hunk header.
pub fn parse_unified_diff(diff: &str) -> DiffTouches {
    let mut touches = DiffTouches::default();
    let mut current_file: Option<String> = None;

    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            current_file = normalize_diff_path(path);
            continue;
        }
        if let Some(hunk) = line.strip_prefix("@@ ") {
            let Some(file) = current_file.as_ref() else {
                continue;
            };
            if let Some(range) = parse_hunk_new_range(hunk) {
                touches.files.entry(file.clone()).or_default().push(range);
            }
        }
    }

    touches
}

/// Strips a leading `a/`/`b/` (git's default prefixes) and the `/dev/null` sentinel for a
/// deleted file (nothing to map lines against there — a deleted file's blast radius is
/// everything that used to call into it, which needs the file's *old* content indexed,
/// not something a diff alone can recover).
fn normalize_diff_path(raw: &str) -> Option<String> {
    let path = raw.split('\t').next().unwrap_or(raw).trim();
    if path.is_empty() || path == "/dev/null" {
        return None;
    }
    let path = path
        .strip_prefix("b/")
        .or_else(|| path.strip_prefix("a/"))
        .unwrap_or(path);
    Some(path.to_string())
}

/// Parses the new-file half of a hunk header (`-oldStart,oldCount +newStart,newCount @@`,
/// with the leading `@@ ` already stripped by the caller) into a 1-indexed, half-open line
/// range. `count` defaults to 1 when a hunk header omits it — standard unified-diff
/// shorthand for a single-line hunk. A hunk with `newCount` 0 (a pure deletion — nothing
/// added on the new side) still touches the insertion point itself, since whatever
/// function used to contain the deleted lines still surrounds that point in the new file.
fn parse_hunk_new_range(rest: &str) -> Option<Range<usize>> {
    let new_part = rest.split_whitespace().find(|s| s.starts_with('+'))?;
    let new_part = new_part.trim_start_matches('+');
    let mut pieces = new_part.splitn(2, ',');
    let start: usize = pieces.next()?.parse().ok()?;
    let count: usize = match pieces.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    let start = start.max(1);
    let count = count.max(1);
    Some(start..(start + count))
}
