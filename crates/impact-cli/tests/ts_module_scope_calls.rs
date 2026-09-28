use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_module_scope_calls")
}

fn index(cache_dir: &Path) {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", fixture_path().to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(cache_dir: &Path, args: &[&str], stdin: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(args)
        .arg("--json")
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .write_stdin(stdin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("--json output should be valid JSON")
}

fn paths(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap().to_string())
        .collect()
}

/// Regression fixture for github.com/AncientiCe/impact-rs#9 (2026-09-28): a bare call
/// statement at a module's top level (`init();` in `boot.ts`, run on every import) left
/// no edge at all, because a call was only ever attributed to the function enclosing it
/// and module scope has none. On the real React Native app it was reported against, that
/// call was the always-executed path, so the report understated the blast radius.
///
/// The call is now attributed to the file's own `boot::<module>` scope, and every file
/// that imports `boot` — by name (`settings.ts`) or purely for its side effects
/// (`app.ts`), and transitively (`root.ts` imports `app.ts`) — is a dependent of that
/// scope, since loading any of them runs `init()`. `types.ts` only has an `import type`
/// of `boot`, which is erased at compile time, so it loads nothing.
#[test]
fn top_level_call_is_attributed_to_the_module_and_reaches_its_importers() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(
        cache_dir.path(),
        &["change", "change signature of boot::init"],
        "",
    );

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "boot::<module>", "file": "src/boot.ts", "line": 1, "confidence": "Exact"},
            {"path": "boot::enableOption", "file": "src/boot.ts", "line": 5, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        paths(&report["indirect"]),
        vec![
            "app::<module>",
            "root::<module>",
            "settings.test::<module>",
            "settings::<module>",
            "settings::turnOn",
        ]
    );
    // A test file's module scope is test code: its top-level setup runs when the test
    // file loads, so a change reaching it affects that test file.
    assert_eq!(report["tests"], 1);
    assert_eq!(
        paths(&report["affected_tests"]),
        vec!["settings.test::<module>"]
    );
}

/// The shape the issue was actually reported through: `impact diff --min-confidence
/// exact` on a hunk inside `init`'s body.
#[test]
fn diff_inside_a_function_called_at_top_level_reports_the_module() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let diff = "diff --git a/src/boot.ts b/src/boot.ts\n\
--- a/src/boot.ts\n\
+++ b/src/boot.ts\n\
@@ -2 +2 @@\n\
-  return withOption;\n\
+  return !withOption;\n";
    let report = run(
        cache_dir.path(),
        &["diff", "--min-confidence", "exact"],
        diff,
    );

    assert_eq!(
        paths(&report["direct"]),
        vec!["boot::<module>", "boot::enableOption"]
    );
}

/// The other side of the same rule: a module with no top-level calls has no load-time
/// behavior of its own, so importing it is not a dependency on anything it declares.
/// `report.ts` imports `format.ts` and calls `format` from inside `render` — that call is
/// the whole blast radius, with no module scope dragged in alongside it.
#[test]
fn importing_a_module_without_top_level_calls_adds_no_module_dependents() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "src/format.ts"], "");

    assert_eq!(paths(&report["direct"]), vec!["report::render"]);
    assert_eq!(paths(&report["indirect"]), Vec::<String>::new());
}

/// A curried call — `withTheme('button', getBaseStyles)(Button)`, the React HOC export
/// shape — used to be recorded as a call to the last identifier anywhere inside its
/// callee: `getBaseStyles`, an argument of the inner call. With no scope evidence for
/// that name it matched every same-named function in the project (here `card.ts`'s,
/// which nothing calls). Harmless while it only happened inside function bodies; once
/// top-level calls were attributed to the module scope, every themed component file
/// became a heuristic caller of every `getBaseStyles` in a real app. The outer call has
/// no name of its own — only the inner `withTheme(...)` call is a call site.
#[test]
fn curried_call_is_a_call_to_the_inner_callee_only() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let unrelated = run(cache_dir.path(), &["query", "src/card.ts"], "");
    assert_eq!(paths(&unrelated["direct"]), Vec::<String>::new());

    let hoc = run(cache_dir.path(), &["query", "src/theme.ts"], "");
    assert_eq!(paths(&hoc["direct"]), vec!["button::<module>"]);
}
