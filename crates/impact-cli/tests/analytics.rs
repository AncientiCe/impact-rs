use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy_crate")
}

/// Every CLI invocation in these tests is pointed at its own scratch analytics DB via
/// `IMPACT_ANALYTICS_DB`, so runs never touch the real `~/.impact/analytics.sqlite` and
/// tests don't interfere with each other.
fn cmd(analytics_db: &Path) -> Command {
    let mut cmd = Command::cargo_bin("impact").unwrap();
    cmd.env("IMPACT_ANALYTICS_DB", analytics_db);
    cmd.env_remove("IMPACT_NO_ANALYTICS");
    cmd
}

fn run_index(analytics_db: &Path, cache_dir: &Path) {
    let output = cmd(analytics_db)
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

fn run_query(analytics_db: &Path, cache_dir: &Path) {
    let output = cmd(analytics_db)
        .args(["query", "src/lib.rs", "--json"])
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
}

fn run_gain(analytics_db: &Path, extra_args: &[&str]) -> Value {
    let output = cmd(analytics_db)
        .arg("gain")
        .arg("--json")
        .args(extra_args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact gain failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("gain --json output should be valid JSON")
}

fn by_command_count(bucket: &Value, command: &str) -> u64 {
    bucket["by_command"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[0] == command)
        .map(|entry| entry[1].as_u64().unwrap())
        .unwrap_or(0)
}

fn by_client_count(bucket: &Value, client: &str) -> u64 {
    bucket["by_client"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[0] == client)
        .map(|entry| entry[1].as_u64().unwrap())
        .unwrap_or(0)
}

/// `index` + `query` runs against the real CLI show up in the current month's bucket,
/// broken down by command and by client ("cli", since these are direct CLI invocations).
#[test]
fn cli_index_and_query_calls_are_recorded_and_rolled_up_monthly() {
    let analytics_db = tempfile::tempdir().unwrap().path().join("analytics.sqlite");
    let cache_dir = tempfile::tempdir().unwrap();

    run_index(&analytics_db, cache_dir.path());
    run_query(&analytics_db, cache_dir.path());
    run_query(&analytics_db, cache_dir.path());

    let buckets = run_gain(&analytics_db, &["--monthly"]);
    let buckets = buckets.as_array().unwrap();
    assert_eq!(buckets.len(), 1, "expected exactly one monthly bucket");

    let bucket = &buckets[0];
    assert_eq!(bucket["total"], 3);
    assert_eq!(by_command_count(bucket, "index"), 1);
    assert_eq!(by_command_count(bucket, "file"), 2);
    assert_eq!(by_client_count(bucket, "cli"), 3);
}

/// `--daily`/`--weekly`/`--monthly` change the bucket label's shape; the default (no
/// flag) matches `--monthly`.
#[test]
fn daily_weekly_monthly_produce_differently_shaped_labels() {
    let analytics_db = tempfile::tempdir().unwrap().path().join("analytics.sqlite");
    let cache_dir = tempfile::tempdir().unwrap();
    run_index(&analytics_db, cache_dir.path());

    let daily = run_gain(&analytics_db, &["--daily"]);
    let weekly = run_gain(&analytics_db, &["--weekly"]);
    let monthly = run_gain(&analytics_db, &["--monthly"]);
    let default = run_gain(&analytics_db, &[]);

    let daily_label = daily[0]["label"].as_str().unwrap();
    let weekly_label = weekly[0]["label"].as_str().unwrap();
    let monthly_label = monthly[0]["label"].as_str().unwrap();

    assert_eq!(
        daily_label.matches('-').count(),
        2,
        "YYYY-MM-DD: {daily_label}"
    );
    assert!(weekly_label.contains("-W"), "YYYY-Wnn: {weekly_label}");
    assert_eq!(
        monthly_label.matches('-').count(),
        1,
        "YYYY-MM: {monthly_label}"
    );
    assert_eq!(default, monthly);
}

/// `IMPACT_NO_ANALYTICS=1` disables recording entirely — commands still work, but leave
/// no trace for `impact gain` to report.
#[test]
fn no_analytics_env_var_disables_recording() {
    let analytics_db = tempfile::tempdir().unwrap().path().join("analytics.sqlite");
    let cache_dir = tempfile::tempdir().unwrap();

    let output = cmd(&analytics_db)
        .env("IMPACT_NO_ANALYTICS", "1")
        .args(["index", fixture_path().to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let buckets = run_gain(&analytics_db, &[]);
    assert_eq!(buckets.as_array().unwrap().len(), 0);
}

/// The human (non-`--json`) rendering is a bar chart per breakdown, with no ANSI escapes
/// when stdout isn't a terminal — exactly what `assert_cmd` gives it, so this doubles as
/// proof color is properly gated on a real TTY check, not always-on.
#[test]
fn gain_text_output_is_a_colorless_bar_chart_when_piped() {
    let analytics_db = tempfile::tempdir().unwrap().path().join("analytics.sqlite");
    let cache_dir = tempfile::tempdir().unwrap();

    run_index(&analytics_db, cache_dir.path());
    run_query(&analytics_db, cache_dir.path());

    let output = cmd(&analytics_db).arg("gain").output().unwrap();
    assert!(
        output.status.success(),
        "impact gain failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();

    assert!(
        !text.contains('\x1b'),
        "piped output should have no ANSI escapes: {text}"
    );
    assert!(text.contains("BY CLIENT"), "{text}");
    assert!(text.contains("BY COMMAND"), "{text}");
    assert!(
        text.contains('█') || text.contains('░'),
        "expected a bar chart: {text}"
    );
    assert!(text.contains('%'), "expected a percentage column: {text}");
    assert!(text.contains("2 calls"), "{text}");
}

/// Calls made over the real MCP stdio protocol are recorded with the client name the
/// session reported in `initialize`'s `clientInfo`.
#[test]
fn mcp_tool_calls_are_recorded_under_the_reported_client_name() {
    let analytics_db = tempfile::tempdir().unwrap().path().join("analytics.sqlite");
    let cache_dir = tempfile::tempdir().unwrap();

    let requests = [
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "clientInfo": {"name": "test-agent", "version": "9.9"}
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {
                "name": "impact_index",
                "arguments": {
                    "project_path": fixture_path().to_str().unwrap(),
                    "cache_dir": cache_dir.path().to_str().unwrap(),
                }
            }
        }),
    ];
    let input = requests
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let output = cmd(&analytics_db)
        .arg("mcp")
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact mcp exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let buckets = run_gain(&analytics_db, &[]);
    let bucket = &buckets[0];
    assert_eq!(by_client_count(bucket, "test-agent"), 1);
    assert_eq!(by_command_count(bucket, "index"), 1);
}
