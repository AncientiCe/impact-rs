use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_jsx_tag_name_refs")
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

/// Regression fixture for a blind spot reported publicly as
/// github.com/AncientiCe/impact-rs#6 (2026-09-23) and independently hit internally on a
/// real React Native app's shared `Screen` component: `impact_file` on a component
/// exported as `export const Screen = Object.assign(Base, { Scrollable: ... })` and used
/// almost exclusively as a JSX tag (`<Screen>`, `<Screen.Scrollable>`) rather than passed
/// as a value, reported a drastically undersized blast radius — real importers were
/// missing entirely.
///
/// Root cause: `collect_refs` already turns a bare identifier handed somewhere as *data*
/// (a JSX attribute's value, an object-literal value, an array element, a call argument —
/// see `ts_jsx_registry_refs`) into a `References` edge, but never did the same for a JSX
/// tag name itself — the single most common way a React component is actually consumed.
/// `jsx_opening_element`/`jsx_self_closing_element` fell through to the generic recursion
/// arm, which walks children but never emits an edge for the tag name node.
///
/// `screen.tsx` exports `Screen` via the exact `Object.assign` shape from the real bug.
/// `plain_tag_consumer.tsx` uses it as a bare JSX tag (`<Screen>`); `member_tag_consumer.tsx`
/// uses the namespaced form (`<Screen.Scrollable>`), whose tag name parses as a
/// `member_expression` rather than a plain `identifier` — the base identifier (`Screen`,
/// not `Scrollable`) is the one that must resolve, since that's the imported binding.
#[test]
fn jsx_tag_name_references_are_resolved() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 3);

    let report = query(cache_dir.path(), "screen.tsx");

    assert_eq!(
        report["direct"],
        serde_json::json!([
            {"path": "member_tag_consumer::MemberTagConsumer", "file": "member_tag_consumer.tsx", "line": 3, "confidence": "Exact"},
            {"path": "plain_tag_consumer::PlainTagConsumer", "file": "plain_tag_consumer.tsx", "line": 3, "confidence": "Exact"},
        ])
    );
}
