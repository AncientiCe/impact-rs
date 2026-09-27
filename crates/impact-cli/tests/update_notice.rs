use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use assert_cmd::Command;

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toy_crate")
}

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A real HTTP server on a loopback port standing in for GitHub's "latest release"
/// endpoint: every request gets `{"tag_name": <tag>}` back. Returns the URL to point
/// `IMPACT_UPDATE_URL` at, plus a counter of requests served so a test can assert how
/// often `impact` actually asked.
fn fake_release_server(tag: &str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/repos/AncientiCe/impact-rs/releases/latest",
        listener.local_addr().unwrap()
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let served = Arc::clone(&hits);
    let body = serde_json::json!({ "tag_name": tag }).to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                line.clear();
            }
            served.fetch_add(1, Ordering::SeqCst);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, hits)
}

/// A loopback URL nothing listens on, so a connection attempt is refused immediately —
/// the offline case.
fn unreachable_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/releases/latest")
}

/// `impact` with the update check switched back on (`.cargo/config.toml` turns it off
/// for every other test) and pointed at `url`, keeping its state in `state`.
fn impact_with_update_check(url: &str, state: &Path) -> Command {
    let mut cmd = Command::cargo_bin("impact").unwrap();
    cmd.env_remove("IMPACT_NO_UPDATE_CHECK")
        .env_remove("CI")
        .env("IMPACT_UPDATE_URL", url)
        .env("IMPACT_UPDATE_STATE", state);
    cmd
}

fn run_index(mut cmd: Command, cache_dir: &Path) -> String {
    let output = cmd
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
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("the notice must never leak into stdout");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn a_newer_release_is_announced_on_stderr() {
    let (url, _) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();

    let stderr = run_index(
        impact_with_update_check(&url, &dir.path().join("update.json")),
        &dir.path().join("cache"),
    );

    assert!(
        stderr.contains(&format!("impact 99.0.0 is available (you have {CURRENT})")),
        "got:\n{stderr}"
    );
}

/// The notice names how to upgrade. A test binary lives in `target/`, which is neither
/// a Homebrew nor an install-script location, so it gets the releases page.
#[test]
fn the_notice_says_how_to_upgrade() {
    let (url, _) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();

    let stderr = run_index(
        impact_with_update_check(&url, &dir.path().join("update.json")),
        &dir.path().join("cache"),
    );

    assert!(
        stderr.contains("https://github.com/AncientiCe/impact-rs/releases/latest"),
        "got:\n{stderr}"
    );
}

/// "From time to time", not on every command: a second run the same day neither asks
/// GitHub again nor repeats the notice.
#[test]
fn checks_and_announces_at_most_once_a_day() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("update.json");

    let first = run_index(
        impact_with_update_check(&url, &state),
        &dir.path().join("cache"),
    );
    let second = run_index(
        impact_with_update_check(&url, &state),
        &dir.path().join("cache"),
    );

    assert!(first.contains("is available"), "got:\n{first}");
    assert!(!second.contains("is available"), "got:\n{second}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn the_current_release_is_not_announced() {
    let (url, _) = fake_release_server(&format!("v{CURRENT}"));
    let dir = tempfile::tempdir().unwrap();

    let stderr = run_index(
        impact_with_update_check(&url, &dir.path().join("update.json")),
        &dir.path().join("cache"),
    );

    assert!(!stderr.contains("is available"), "got:\n{stderr}");
}

#[test]
fn opting_out_skips_the_check_entirely() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = impact_with_update_check(&url, &dir.path().join("update.json"));
    cmd.env("IMPACT_NO_UPDATE_CHECK", "1");

    let stderr = run_index(cmd, &dir.path().join("cache"));

    assert!(!stderr.contains("is available"), "got:\n{stderr}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[test]
fn ci_runs_skip_the_check_entirely() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = impact_with_update_check(&url, &dir.path().join("update.json"));
    cmd.env("CI", "true");

    let stderr = run_index(cmd, &dir.path().join("cache"));

    assert!(!stderr.contains("is available"), "got:\n{stderr}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

/// Offline, the command still does its job and says nothing about updates.
#[test]
fn an_unreachable_release_server_is_silent() {
    let dir = tempfile::tempdir().unwrap();

    let stderr = run_index(
        impact_with_update_check(&unreachable_url(), &dir.path().join("update.json")),
        &dir.path().join("cache"),
    );

    assert!(!stderr.contains("is available"), "got:\n{stderr}");
    assert!(!stderr.contains("update"), "got:\n{stderr}");
}

/// Runs `impact mcp` with the update check pointed at `url`: `initialize`, then
/// `impact_index` twice. Returns the text of every content item of each `tools/call`
/// response, in order.
fn mcp_two_tool_calls(url: &str, state: &Path, cache_dir: &Path) -> Vec<Vec<String>> {
    let index_call = |id: i64| {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "impact_index",
                "arguments": {
                    "project_path": fixture_path().to_str().unwrap(),
                    "cache_dir": cache_dir.to_str().unwrap(),
                }
            }
        })
    };
    let requests = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        index_call(2),
        index_call(3),
    ];
    let input = requests
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let output = impact_with_update_check(url, state)
        .arg("mcp")
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            let response: serde_json::Value = serde_json::from_str(line).unwrap();
            response["result"]["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["text"].as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

/// Agents are where impact mostly runs, so the notice reaches them too — once per
/// session, as its own content item, addressed to the user rather than to the agent.
#[test]
fn mcp_announces_a_newer_release_once_per_session_for_the_user() {
    let (url, _) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();

    let calls = mcp_two_tool_calls(
        &url,
        &dir.path().join("update.json"),
        &dir.path().join("cache"),
    );

    assert_eq!(calls.len(), 2);
    let first_notice = calls[0].iter().find(|text| text.contains("is available"));
    let Some(notice) = first_notice else {
        panic!("first tool call carries no notice: {:?}", calls[0]);
    };
    assert!(
        notice.contains(&format!("impact 99.0.0 is available (you have {CURRENT})")),
        "got: {notice}"
    );
    assert!(notice.contains("tell the user"), "got: {notice}");
    assert!(
        notice.contains("don't run the upgrade yourself"),
        "got: {notice}"
    );
    serde_json::from_str::<serde_json::Value>(&calls[0][0])
        .expect("the tool's own result stays the first content item, unchanged");
    assert!(
        !calls[1].iter().any(|text| text.contains("is available")),
        "got: {:?}",
        calls[1]
    );
}

#[test]
fn mcp_is_silent_about_the_current_release() {
    let (url, _) = fake_release_server(&format!("v{CURRENT}"));
    let dir = tempfile::tempdir().unwrap();

    let calls = mcp_two_tool_calls(
        &url,
        &dir.path().join("update.json"),
        &dir.path().join("cache"),
    );

    assert!(calls.iter().all(|items| items.len() == 1), "got: {calls:?}");
}

/// Runs `impact index` against the fake server so the state file learns about the
/// release, the way any earlier real command would have.
fn learn_latest_release(url: &str, state: &Path, cache_dir: &Path) {
    run_index(impact_with_update_check(url, state), cache_dir);
}

fn report_blindspot(url: &str, state: &Path, json: bool) -> String {
    let mut cmd = impact_with_update_check(url, state);
    cmd.args([
        "report-blindspot",
        "misses an indirect call",
        "--body",
        "impact_file reported no callers, but grep found one.",
    ]);
    if json {
        cmd.arg("--json");
    }
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

/// A gap seen on an old release may already be fixed, so a draft says when a newer
/// release is known.
#[test]
fn a_blindspot_draft_warns_when_a_newer_release_is_known() {
    let (url, _) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("update.json");
    learn_latest_release(&url, &state, &dir.path().join("cache"));

    let stdout = report_blindspot(&url, &state, false);

    assert!(
        stdout.contains(&format!("impact 99.0.0 is available (you have {CURRENT})")),
        "got:\n{stdout}"
    );
    assert!(stdout.contains("already fixed"), "got:\n{stdout}");
}

#[test]
fn a_blindspot_json_draft_names_the_newer_release() {
    let (url, _) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("update.json");
    learn_latest_release(&url, &state, &dir.path().join("cache"));

    let draft: serde_json::Value =
        serde_json::from_str(&report_blindspot(&url, &state, true)).unwrap();

    assert_eq!(draft["newer_release"], "99.0.0");
}

/// Drafting a report never touches the network, so the draft only knows what an earlier
/// command already cached, and drafting doesn't run the check itself.
#[test]
fn drafting_a_blindspot_never_asks_for_the_latest_release() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();

    let stdout = report_blindspot(&url, &dir.path().join("update.json"), false);

    assert!(!stdout.contains("is available"), "got:\n{stdout}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

fn mcp_report_blindspot(url: &str, state: &Path) -> serde_json::Value {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "impact_report_blindspot",
            "arguments": {
                "title": "misses an indirect call",
                "body": "impact_file reported no callers, but grep found one.",
            }
        }
    });
    let output = impact_with_update_check(url, state)
        .arg("mcp")
        .write_stdin(request.to_string())
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn an_mcp_blindspot_draft_names_the_newer_release() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("update.json");
    learn_latest_release(&url, &state, &dir.path().join("cache"));

    let response = mcp_report_blindspot(&url, &state);

    let draft: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(draft["newer_release"], "99.0.0");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

/// Over MCP too: a session whose first call drafts a report doesn't run the check.
#[test]
fn an_mcp_blindspot_draft_never_asks_for_the_latest_release() {
    let (url, hits) = fake_release_server("v99.0.0");
    let dir = tempfile::tempdir().unwrap();

    let response = mcp_report_blindspot(&url, &dir.path().join("update.json"));

    let content = response["result"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "got: {response}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}
