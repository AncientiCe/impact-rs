use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_route_name_static_config")
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

/// Issue #16, static-config form. React Navigation 7 apps register screens in a
/// `createStackNavigator({ screens: { Home: { screen: HomeScreen } } })` object rather than
/// `<Stack.Screen>` elements, and navigate to them by the key: `navigation.navigate('Home')`.
/// That is the shape of the bug where `navigate` pushed a second copy of a screen already in
/// the stack: the routes involved were registered this way and reached by string.
///
/// `stack.tsx` registers `HomeScreen` (object form) and `DetailsScreen` (bare form).
/// `notroutes.ts` has a lookalike `{ Details: { screen: DetailsScreen } }` outside any
/// `screens` object: it is not a registration, so `navigate('Details')` still resolves to
/// the one real route with full `Probable` confidence rather than an ambiguous pair.
#[test]
fn screen_registered_in_static_config_is_reached_from_navigate_calls() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 5);

    let home = query(cache_dir.path(), "home.tsx");
    assert_eq!(
        home["direct"],
        serde_json::json!([
            {"path": "stack::AppStack", "file": "stack.tsx", "line": 4, "confidence": "Exact"},
            {"path": "stack::route:Home", "file": "stack.tsx", "line": 6, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        home["indirect"],
        serde_json::json!([
            {"path": "nav::goHome", "file": "nav.ts", "line": 1, "confidence": "Probable"},
        ])
    );

    let details = query(cache_dir.path(), "details.tsx");
    assert_eq!(
        details["direct"],
        serde_json::json!([
            {"path": "notroutes::buildRegistry", "file": "notroutes.ts", "line": 3, "confidence": "Exact"},
            {"path": "stack::AppStack", "file": "stack.tsx", "line": 4, "confidence": "Exact"},
            {"path": "stack::route:Details", "file": "stack.tsx", "line": 7, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        details["indirect"],
        serde_json::json!([
            {"path": "nav::goDetails", "file": "nav.ts", "line": 5, "confidence": "Probable"},
        ])
    );
}
