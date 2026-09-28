use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python_module_scope_calls")
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

/// The Python side of github.com/AncientiCe/impact-rs#9 (see `ts_module_scope_calls`): a
/// bare `init()` at a module's top level, run on every import, left no edge at all. It's
/// now attributed to `boot::<module>`, and every module that imports `boot` — by name
/// (`settings.py`), only for its side effects (`app.py`), or transitively (`root.py`
/// imports `app`) — is a dependent of it. `test_settings.py` follows pytest's file naming,
/// so its module scope, which runs `turn_on()` on import, counts as a test.
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
            {"path": "boot::<module>", "file": "boot.py", "line": 1, "confidence": "Exact"},
            {"path": "boot::enable_option", "file": "boot.py", "line": 5, "confidence": "Exact"},
        ])
    );
    assert_eq!(
        paths(&report["indirect"]),
        vec![
            "app::<module>",
            "root::<module>",
            "settings::<module>",
            "settings::turn_on",
            "test_settings::<module>",
        ]
    );
    assert_eq!(report["tests"], 1);
    assert_eq!(
        paths(&report["affected_tests"]),
        vec!["test_settings::<module>"]
    );
}

/// A module with no top-level calls runs nothing on import, so importing it is not a
/// dependency on anything it declares.
#[test]
fn importing_a_module_without_top_level_calls_adds_no_module_dependents() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let report = run(cache_dir.path(), &["query", "format.py"], "");

    assert_eq!(paths(&report["direct"]), vec!["report::render"]);
    assert_eq!(paths(&report["indirect"]), Vec::<String>::new());
}

/// `register("button", get_base_styles)(button)` — a decorator factory applied by hand —
/// calls whatever `register(...)` returns. It used to be recorded as a call to
/// `get_base_styles`, the last identifier inside its callee, matching every same-named
/// function in the project (here `card.py`'s), while the `register(...)` call itself was
/// never recorded.
#[test]
fn curried_call_is_a_call_to_the_inner_callee_only() {
    let cache_dir = tempfile::tempdir().unwrap();
    index(cache_dir.path());

    let unrelated = run(cache_dir.path(), &["query", "card.py"], "");
    assert_eq!(paths(&unrelated["direct"]), Vec::<String>::new());

    let factory = run(cache_dir.path(), &["query", "registry.py"], "");
    assert_eq!(paths(&factory["direct"]), vec!["button::<module>"]);
}
