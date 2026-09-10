//! `tsconfig.json`/`jsconfig.json` `compilerOptions.paths` — resolves a project's own
//! module-alias convention (`@scope/*` -> `./src/packages/*`, or a bare catch-all
//! `"*": ["./src/*"]`) the same way `resolve_specifier` already resolves a relative
//! import. Deliberately narrow: only `baseUrl` and `paths` are read, `"extends"` (a base
//! config elsewhere, often inside `node_modules`) is not followed — good enough for
//! structural blast-radius, not a claim of running `tsc`'s real module resolver.

use std::fs;
use std::path::Path;

/// One `paths` entry, already split around its pattern's single `*` (tsconfig allows at
/// most one per pattern/target). `has_wildcard` distinguishes an exact pattern (`"utils"`)
/// from a wildcard one whose capture happens to be empty.
#[derive(Debug, Clone)]
pub struct PathAlias {
    pattern_prefix: String,
    pattern_suffix: String,
    has_wildcard: bool,
    /// The first configured target for this pattern, relative to `baseUrl`, with the
    /// same `*` split applied — a pattern can list multiple fallback targets, but this
    /// adapter (like the rest of it) resolves module identity, not "the real file on
    /// disk", so the first one is the only one worth trying.
    target_prefix: String,
    target_suffix: String,
    target_has_wildcard: bool,
}

fn split_wildcard(pattern: &str) -> (String, String, bool) {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => (prefix.to_string(), suffix.to_string(), true),
        None => (pattern.to_string(), String::new(), false),
    }
}

/// Matches `specifier` against one pattern, returning the substring the pattern's `*`
/// captured (empty for a non-wildcard exact match).
fn match_pattern(
    specifier: &str,
    prefix: &str,
    suffix: &str,
    has_wildcard: bool,
) -> Option<String> {
    if !has_wildcard {
        return (specifier == prefix).then(String::new);
    }
    let rest = specifier.strip_prefix(prefix)?;
    let capture = rest.strip_suffix(suffix)?;
    Some(capture.to_string())
}

/// Reads `tsconfig.json` (falling back to `jsconfig.json`) directly under `project_root`.
/// Anything short of "found a well-formed `compilerOptions.paths` object" — no file, a
/// parse error, `paths` absent or the wrong shape — quietly yields no aliases; a project
/// with no config file is the common case, not an error.
pub fn load_path_aliases(project_root: &Path) -> Vec<PathAlias> {
    let raw = ["tsconfig.json", "jsconfig.json"]
        .iter()
        .find_map(|name| fs::read_to_string(project_root.join(name)).ok());
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&strip_jsonc_comments(&raw)) else {
        return Vec::new();
    };

    let compiler_options = &value["compilerOptions"];
    let base_url = compiler_options["baseUrl"].as_str().unwrap_or(".");
    // `join_relative` already treats a bare "." as a no-op, same as a real `cd .` would —
    // reusing it here (against an empty starting point) avoids re-deriving that rule and
    // avoids a literal "." leaking into every resolved alias when baseUrl is unset.
    let normalized_base = super::join_relative(&[], base_url);
    let base_segments: Vec<&str> = normalized_base
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    let Some(paths) = compiler_options["paths"].as_object() else {
        return Vec::new();
    };

    let mut aliases: Vec<PathAlias> = paths
        .iter()
        .filter_map(|(pattern, targets)| {
            let target = targets.as_array()?.first()?.as_str()?;
            let (pattern_prefix, pattern_suffix, has_wildcard) = split_wildcard(pattern);
            let resolved_target = super::join_relative(&base_segments, target);
            let (target_prefix, target_suffix, target_has_wildcard) =
                split_wildcard(&resolved_target);
            Some(PathAlias {
                pattern_prefix,
                pattern_suffix,
                has_wildcard,
                target_prefix,
                target_suffix,
                target_has_wildcard,
            })
        })
        .collect();

    // The most specific (longest literal prefix) pattern should win when more than one
    // matches — real `tsc` behavior — so a scoped alias like `@newstore/*` is tried
    // before a catch-all `"*"`. A stable sort keeps `tsconfig.json`'s own ordering as the
    // tiebreak for equal-length prefixes.
    aliases.sort_by_key(|a| std::cmp::Reverse(a.pattern_prefix.len()));
    aliases
}

/// Tries every configured alias against a bare specifier, in most-specific-first order,
/// returning the resolved module path (still needing `module_prefix`) of the first match.
pub fn resolve_alias(specifier: &str, aliases: &[PathAlias]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        let capture = match_pattern(
            specifier,
            &alias.pattern_prefix,
            &alias.pattern_suffix,
            alias.has_wildcard,
        )?;
        let target = if alias.target_has_wildcard {
            format!("{}{capture}{}", alias.target_prefix, alias.target_suffix)
        } else {
            alias.target_prefix.clone()
        };
        Some(target)
    })
}

/// Strips `//` and `/* */` comments from JSONC (what `tsconfig.json`/`jsconfig.json`
/// actually are — TypeScript's own parser accepts comments and a trailing comma) so
/// `serde_json`, which doesn't, can read it. String contents are left untouched: a `//`
/// or `/*` inside a quoted string is real data, not a comment, so this tracks string
/// state (including `\"` escapes) rather than scanning blindly.
fn strip_jsonc_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_jsonc_comments;

    /// Pure, stateless, non-domain helper (no `paths`/`tsconfig` semantics involved) —
    /// the kind AGENTS.md's TDD rule exempts from the fixture-level-only requirement,
    /// same as the project's `blake3` `NodeId` hash. The real behavior (aliases actually
    /// resolving imports) is covered end-to-end by `tests/ts_path_aliases.rs`.
    #[test]
    fn strips_line_and_block_comments_but_not_string_contents() {
        let input = "{\n  // a comment\n  \"a\": \"http://not-a-comment\", /* block */ \"b\": 1\n}";
        let stripped = strip_jsonc_comments(input);
        let value: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["a"], "http://not-a-comment");
        assert_eq!(value["b"], 1);
    }
}
