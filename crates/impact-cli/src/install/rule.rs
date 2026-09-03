//! Agent-rule content: the text telling an AI coding agent to call `impact`'s MCP tools
//! before/after editing code, plus the pure string operations that install it either as
//! a standalone file (Cursor) or as a managed block inside a shared file the user may
//! also edit (Codex's `AGENTS.md`, Claude's `CLAUDE.md`).

pub const RULE_BEGIN: &str = "<!-- BEGIN IMPACT -->";
pub const RULE_END: &str = "<!-- END IMPACT -->";

pub const RULE_BODY: &str = r#"# Impact Blast-Radius Protocol — MANDATORY

**MANDATORY — two hard triggers, every task, no exceptions.**

## BEFORE EDITING
*Before renaming, removing, or changing the signature of any function, type, enum
variant, or field — or touching code behind an API route, event, or database table. This
also covers proposing such a change: once your proposed fix is concrete enough to state
as a rename/remove/signature-change target, run this before presenting the proposal,
even if you haven't written any code yet. Vague, exploratory "here's roughly how I'd
approach it" discussion that hasn't settled on a concrete target doesn't need it.*
→ If this project hasn't been indexed yet this session (or has changed since), call
  `impact_index` once with the project root.
→ Then call `impact_file` (blast radius of a file) or `impact_change` (blast radius of a
  specific rename/remove/signature change — e.g. `"rename PaymentStatus::Failed"`,
  `"remove field User.email"`, `"change signature of PaymentService::charge"`) to see
  direct/indirect callers, API routes, event types, database tables, and affected tests
  before writing (or proposing) the change.
→ Treat a nonzero result as a checklist: update every caller and affected test the
  report names, not just the file you were asked to change.

## AFTER EDITING
*After the change is made, before considering the task done.*
→ Re-run `impact_index` (results are only as fresh as the last index), then re-run
  `impact_file`/`impact_change` against the same target to confirm the blast radius you
  addressed matches what's reported now, and nothing new appeared.

`impact_change` grammar: `rename <path>`, `rename <path> to <path>`, `remove <path>`,
`remove variant <Enum>::<Variant>`, `remove field <Type>.<field>`, `change signature of
<path>`. Not natural language — an unrecognized description is a hard error."#;

/// The full managed block, including its delimiter markers.
pub fn managed_rule_block() -> String {
    format!("{RULE_BEGIN}\n{RULE_BODY}\n{RULE_END}")
}

/// Cursor's standalone rule file: frontmatter plus the same body, no HTML markers
/// (Cursor owns the whole file, so there is nothing else in it to delimit around).
pub fn cursor_rule_text() -> String {
    format!(
        "---\ndescription: Verify blast radius with impact before/after editing code\nalwaysApply: true\n---\n\n{RULE_BODY}\n"
    )
}

/// Locate the managed block's byte range `[start, end)`, markers included.
pub fn find_managed_block(text: &str) -> Option<(usize, usize)> {
    let start = text.find(RULE_BEGIN)?;
    let search_from = start + RULE_BEGIN.len();
    let end_relative = text[search_from..].find(RULE_END)?;
    let end = search_from + end_relative + RULE_END.len();
    Some((start, end))
}

/// Insert or replace the managed block in `existing`, leaving everything else intact.
pub fn upsert_managed_rule(existing: &str) -> String {
    let block = managed_rule_block();
    if let Some((start, end)) = find_managed_block(existing) {
        let mut next = String::with_capacity(existing.len() + block.len());
        next.push_str(&existing[..start]);
        next.push_str(&block);
        next.push_str(&existing[end..]);
        return next;
    }
    if existing.is_empty() {
        return format!("{block}\n");
    }
    let separator = if existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{existing}{separator}{block}\n")
}

/// Remove the managed block from `existing`, if present, collapsing any resulting
/// run of blank lines left behind.
pub fn remove_managed_rule(existing: &str) -> String {
    let Some((start, end)) = find_managed_block(existing) else {
        return existing.to_string();
    };
    let mut next = String::with_capacity(existing.len());
    next.push_str(&existing[..start]);
    next.push_str(&existing[end..]);
    while next.contains("\n\n\n") {
        next = next.replace("\n\n\n", "\n\n");
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_inserts_block_into_empty_text() {
        let result = upsert_managed_rule("");
        assert!(result.starts_with(RULE_BEGIN));
        assert!(result.trim_end().ends_with(RULE_END));
    }

    #[test]
    fn upsert_appends_block_after_existing_content() {
        let result = upsert_managed_rule("# Existing guidance\n\nKeep this line.\n");
        assert!(result.contains("# Existing guidance"));
        assert!(result.contains("Keep this line."));
        assert!(result.contains(RULE_BEGIN));
    }

    #[test]
    fn upsert_replaces_previous_block_in_place() {
        let first = upsert_managed_rule("# Before\n");
        let second = upsert_managed_rule(&first);
        assert_eq!(first, second);
        assert_eq!(second.matches(RULE_BEGIN).count(), 1);
    }

    #[test]
    fn remove_strips_block_and_collapses_blank_lines() {
        let with_block = upsert_managed_rule("# Before\n\nAfter.\n");
        let removed = remove_managed_rule(&with_block);
        assert!(!removed.contains(RULE_BEGIN));
        assert!(removed.contains("# Before"));
        assert!(removed.contains("After."));
        assert!(!removed.contains("\n\n\n"));
    }

    #[test]
    fn remove_is_noop_when_block_absent() {
        let text = "# Nothing to remove\n";
        assert_eq!(remove_managed_rule(text), text);
    }
}
