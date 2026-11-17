use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cpp_preprocessor_in_expr")
}

fn run_impact(args: &[&str], cache_dir: &Path) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(args)
        .arg("--json")
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// `loop.cpp` holds `read_datagram`, whose `if (...)` condition is assembled from
/// preprocessor-guarded pieces (`#ifdef NET_EINTR  net_errno()!=NET_EINTR &&  #endif ...`).
/// tree-sitter has no preprocessor and can't parse a directive in the middle of an
/// expression, so the function swallowed the closing braces of everything after it and
/// `run_loop` — a later function in the same file, which calls `process_pending_jobs`
/// behind `#ifdef WITH_API` — was never indexed as a symbol. The call site then had no
/// enclosing function and the edge vanished, leaving a report that looked complete
/// (github.com/AncientiCe/impact-rs#23). Reading every branch of a conditional, the way
/// a build with that macro defined would, must keep `run_loop` indexed and linked.
#[test]
fn call_after_a_directive_guarded_condition_is_reported_as_a_dependent() {
    let cache_dir = tempfile::tempdir().unwrap();
    let project = fixture_path();
    let project = project.to_str().unwrap();

    let stats = run_impact(&["index", project], cache_dir.path());
    assert_eq!(stats["files_indexed"], 2);

    let report = run_impact(
        &["query", "jobs.cpp", "--project", project],
        cache_dir.path(),
    );
    let direct = report["direct"].as_array().unwrap();
    assert_eq!(direct.len(), 1, "unexpected dependents: {direct:?}");
    assert_eq!(direct[0]["file"], "loop.cpp");
    assert!(
        direct[0]["path"].as_str().unwrap().ends_with("run_loop"),
        "unexpected dependent: {}",
        direct[0]
    );
}
