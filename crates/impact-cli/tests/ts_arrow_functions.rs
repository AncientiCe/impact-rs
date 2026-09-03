use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_arrow_functions")
}

fn index(cache_dir: &Path) -> Value {
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
    serde_json::from_slice(&output.stdout).unwrap()
}

fn query(cache_dir: &Path, file: &str) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["query", file, "--json"])
        .arg("--project")
        .arg(fixture_path())
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("query --json output should be valid JSON")
}

/// Regression fixture for the arrow-function blind spot: `impact-lang-ts`'s `walk`
/// (extract_symbols) and `collect_refs` (extract_references) used to only recognize
/// `function_declaration`/`method_definition` as function-scope-introducing constructs,
/// so `const Foo = () => {...}` — the dominant style for React/React Native components
/// and hooks — was invisible to both: never registered as a symbol, and calls inside its
/// body silently dropped (never attributed to any caller). `util.ts` exports `helper`;
/// four call sites exercise the shapes that were previously all blind spots:
/// `component.tsx`'s unexported `const Foo = () => {...}` (block body), `hook.ts`'s
/// `export const useThing = () => {...}` (block body), `expr.ts`'s
/// `const Bar = function () {...}` (function expression), and `concise.ts`'s
/// `export const useConcise = () => helper()` (concise/expression arrow body, no braces
/// — a distinct tree-sitter shape where the callee sits directly in the `body` field
/// rather than inside a `statement_block`).
///
/// Querying `util.ts` should surface all four as DIRECT callers.
#[test]
fn arrow_function_and_function_expression_callers_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 5);
    assert_eq!(stats["symbols_indexed"], 5);

    let report = query(cache_dir.path(), "util.ts");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "component::Foo", "file": "component.tsx", "line": 3, "confidence": "Exact"},
            {"path": "concise::useConcise", "file": "concise.ts", "line": 3, "confidence": "Exact"},
            {"path": "expr::Bar", "file": "expr.ts", "line": 3, "confidence": "Exact"},
            {"path": "hook::useThing", "file": "hook.ts", "line": 3, "confidence": "Exact"},
        ])
    );
}
