use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_named_import_alias")
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
/// Reported in [#11](https://github.com/AncientiCe/impact-rs/issues/11): a hook imported a
/// function under a local alias (`import { print as render } from './printer'`) and called
/// `render(...)`. The call was recorded under the local name, and the linker looked for a
/// symbol called `render` inside `printer.ts` — there is none — so the hook, the function's
/// main production caller, was missing while the namespace-import and same-name callers
/// resolved. The scope now remembers which exported name a local alias stands for.
#[test]
fn named_import_renamed_at_the_import_site_still_resolves() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);

    let report = query(cache_dir.path(), "printer.ts");
    let direct = report["direct"].as_array().unwrap();
    let paths: Vec<&str> = direct.iter().map(|d| d["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        ["hook::useReceipt", "plain::plain", "preview::preview"]
    );
    assert!(direct.iter().all(|d| d["confidence"] == "Exact"));
}
