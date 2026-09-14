use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ts_default_import_alias")
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

/// Regression fixture for a second, independent blind spot found 2026-09-14 while
/// verifying the JSX-registry `References` fix against a real repo: a screen component
/// exported as `export default XScreen` and imported under a *different* local alias
/// (`import Foo from './XScreen'`, a common shape — the file/alias name matches a
/// project's own naming convention, not the internal symbol's own name) still resolved to
/// zero consumers, with or without that fix. A default import's local name is chosen by
/// the importer, not by whatever the target module calls its own export, so matching by
/// name (`Resolver::in_module`, what every other import kind uses) is structurally the
/// wrong tool: `in_module` looked for a symbol literally named "Foo" inside the target
/// module and never found one, since the real symbol is named "XScreen".
///
/// Confirmed this wasn't specific to JSX at all: `service.ts`/`caller.ts` cover a bare
/// function *call* through a renamed default import; `screen.tsx`/`navigator.tsx` cover
/// the JSX-attribute-value case the earlier fix introduced. `impact-lang-ts` now marks
/// which symbol is a file's own default export (`SymbolDecl::is_default_export`) and
/// binds a default import via `FileScope::add_default_import`, which resolves through the
/// new `RefTarget::ModuleDefault` (`Resolver::in_module_default`) — matched by which
/// symbol is flagged as the module's default export, never by name.
#[test]
fn default_export_renamed_at_the_import_site_still_resolves() {
    let cache_dir = tempfile::tempdir().unwrap();
    let stats = index(cache_dir.path());
    assert_eq!(stats["files_indexed"], 4);

    let service_report = query(cache_dir.path(), "service.ts");
    assert_eq!(
        service_report["direct"],
        serde_json::json!([
            {"path": "caller::run", "file": "caller.ts", "line": 3, "confidence": "Exact"},
        ])
    );

    let screen_report = query(cache_dir.path(), "screen.tsx");
    assert_eq!(
        screen_report["direct"],
        serde_json::json!([
            {"path": "navigator::AppNavigator", "file": "navigator.tsx", "line": 3, "confidence": "Exact"},
        ])
    );
}
