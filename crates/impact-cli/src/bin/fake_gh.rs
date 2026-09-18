//! Test-only double for `gh`, used by `tests/blindspot.rs` and `tests/mcp.rs` (via
//! `write_fake_gh` in each). A real spawned process, not a `/bin/sh`/`cmd.exe` script:
//! `impact report-blindspot --submit` passes `gh issue create` a `--body` argument that's
//! multi-line and carries backticks/asterisks straight from the drafted markdown, and
//! Windows refuses outright to spawn a `.bat`/`.cmd` file with such an argument (a
//! deliberate safety check in the standard library, since there's no way to escape it
//! safely for `cmd.exe`'s batch-parameter substitution). A real executable has no such
//! restriction — argv is argv, on every platform.
//!
//! Never shipped: `.github/workflows/release.yml` packages only the `impact` binary.
//!
//! Reads its behavior from the JSON file named by `FAKE_GH_CONFIG` (written by
//! `write_fake_gh`) rather than baking it into a generated script, since a real binary
//! can't be generated per test the way a script's text can.

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(serde::Deserialize)]
struct FakeGhConfig {
    /// Printed verbatim on `gh issue list`.
    list_json: String,
    /// `gh issue create`'s behavior: `Some(url)` prints it (a "successful" filing);
    /// `None` means creation must never be invoked in this test.
    create_url: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let Ok(config_path) = std::env::var("FAKE_GH_CONFIG") else {
        eprintln!("fake_gh: FAKE_GH_CONFIG is not set");
        return ExitCode::FAILURE;
    };
    let config_text = match std::fs::read_to_string(PathBuf::from(&config_path)) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("fake_gh: failed to read {config_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let config: FakeGhConfig = match serde_json::from_str(&config_text) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("fake_gh: failed to parse {config_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("issue"), Some("list")) => {
            println!("{}", config.list_json);
            ExitCode::SUCCESS
        }
        (Some("issue"), Some("create")) => match config.create_url {
            Some(url) => {
                println!("{url}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("fake_gh: issue create should not have been called");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("fake_gh: unexpected gh invocation: {}", args.join(" "));
            ExitCode::FAILURE
        }
    }
}
