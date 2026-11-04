use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_route_name_navigation")
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

/// Issue #16: `navigation.navigate(Screens.Detail)`, `popTo(...)` and `navigate('Settings')`
/// name a screen only by the route name it is registered under, so changing the screen
/// reported none of the code that navigates to it. In React Navigation 7 that matters more:
/// `navigate` no longer returns to a screen already in the stack, so each call site is a
/// behavior to re-check when the screen or its registration changes.
///
/// `router.tsx` registers `DetailScreen` under `Screens.Detail` and `SettingsScreen` under
/// the string `"Settings"`. `list.tsx` navigates by enum member, `back.ts` pops to it, and
/// `open.ts` navigates by string. `tags.ts` pushes the string `"Settings"` onto an array,
/// which is not navigation and must not be reported. `harness.spec.tsx` registers the same
/// screen under the same name to render it in isolation: that is not a route of the app, so
/// it must not make `Screens.Detail` ambiguous (which would demote every navigate call to
/// it from `Probable` to `Heuristic`).
#[test]
fn screen_is_reached_from_code_navigating_to_its_route_name() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 9);

    let detail = query(cache_dir.path(), "detail.tsx");
    assert_eq!(
        detail["direct"],
        serde_json::json!([
            {"path": "harness.spec::renderDetail", "file": "harness.spec.tsx", "line": 4, "confidence": "Exact"},
            {"path": "router::AppRouter", "file": "router.tsx", "line": 5, "confidence": "Exact"},
            {"path": "router::route:Detail", "file": "router.tsx", "line": 8, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        detail["indirect"],
        serde_json::json!([
            {"path": "back::backToDetail", "file": "back.ts", "line": 3, "confidence": "Probable"},
            {"path": "list::ListScreen", "file": "list.tsx", "line": 3, "confidence": "Probable"},
        ])
    );

    let settings = query(cache_dir.path(), "settings.tsx");
    assert_eq!(
        settings["indirect"],
        serde_json::json!([
            {"path": "open::openSettings", "file": "open.ts", "line": 1, "confidence": "Probable"},
        ])
    );
}
