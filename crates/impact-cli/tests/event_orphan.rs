//! `impact index` now flags an event contract that had both a producer and a consumer on
//! the previous run and lost one of them on this run — the shape a producer-to-direct-
//! call migration takes when the replacement forgets behavior the old consumer had (the
//! motivating real-world case: an event-based document-notification path replaced by a
//! direct HTTP call whose new handler dropped a field the old consumer always set).
//! Deliberately narrow: this can't know the replacement is related to the orphaned event
//! at all, only that the event itself just lost a side it used to have — an early warning
//! to look closer, not a claim of what broke.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/event_orphan")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn index(project: &Path, cache_dir: &Path) -> Value {
    let output = Command::cargo_bin("impact")
        .unwrap()
        .args(["index", project.to_str().unwrap(), "--json"])
        .arg("--cache-dir")
        .arg(cache_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "impact index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("index --json output should be valid JSON")
}

#[test]
fn losing_an_events_last_producer_is_flagged_once() {
    let project_dir = tempfile::tempdir().unwrap();
    let project = project_dir.path();
    copy_dir(&fixture_path(), project);
    let cache_dir = tempfile::tempdir().unwrap();

    // First index: nothing to compare against yet, so nothing can be "orphaned".
    let first = index(project, cache_dir.path());
    assert_eq!(first["orphaned_events"], serde_json::json!([]));

    // Remove producer.rs's only OrderPlaced construction — the event still exists
    // (events.rs untouched) and is still consumed (consumer.rs untouched), but it just
    // lost its last producer.
    std::fs::write(
        project.join("src/producer.rs"),
        "pub fn dispatch(id: u64) {\n    let _ = id;\n}\n",
    )
    .unwrap();

    let second = index(project, cache_dir.path());
    assert_eq!(
        second["orphaned_events"],
        serde_json::json!([{
            "event": "OrderPlaced",
            "lost_producer": true,
            "lost_consumer": false,
        }]),
        "OrderPlaced lost its only producer between the first and second index: {second}"
    );

    // Indexing again with no further change shouldn't re-report the same loss forever —
    // there's nothing new to compare against, since the event already had no producer
    // going into this run.
    let third = index(project, cache_dir.path());
    assert_eq!(third["orphaned_events"], serde_json::json!([]));
}
