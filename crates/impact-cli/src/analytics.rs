//! Local, best-effort usage analytics: one global SQLite DB (`~/.impact/analytics.sqlite`
//! by default) recording every real CLI/MCP analysis call, and the rollup query behind
//! `impact gain`.
//!
//! Purely local — nothing here ever makes a network call. Recording never fails a real
//! command: a DB that can't be opened or written just gets a stderr note and is skipped.
//! Set `IMPACT_NO_ANALYTICS` (any value) to disable recording outright; set
//! `IMPACT_ANALYTICS_DB` to a file path to relocate (or, in tests, isolate) the DB.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

/// One recorded `index`/`file`/`change`/`diff` call.
pub struct UsageEvent {
    pub command: &'static str,
    pub source: &'static str,
    pub client: String,
    pub client_version: Option<String>,
    pub duration_ms: u64,
    pub success: bool,
}

pub fn db_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("IMPACT_ANALYTICS_DB") {
        return Ok(PathBuf::from(path));
    }
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?;
    Ok(base.home_dir().join(".impact").join("analytics.sqlite"))
}

pub fn open() -> Result<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating analytics directory {}", parent.display()))?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("opening analytics database {}", path.display()))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS usage_events (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            ts             INTEGER NOT NULL,
            command        TEXT NOT NULL,
            source         TEXT NOT NULL,
            client         TEXT NOT NULL,
            client_version TEXT,
            duration_ms    INTEGER NOT NULL,
            success        INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS usage_events_ts_idx ON usage_events(ts);
        ",
    )?;
    Ok(())
}

/// Records `event` against `conn`, or logs a one-line warning and does nothing on
/// failure — analytics must never be able to fail the real command that triggered it.
/// A no-op (silently) when `IMPACT_NO_ANALYTICS` is set.
pub fn record(conn: &Connection, event: UsageEvent) {
    if std::env::var_os("IMPACT_NO_ANALYTICS").is_some() {
        return;
    }
    if let Err(e) = try_record(conn, &event) {
        eprintln!("impact: analytics write failed (ignored): {e}");
    }
}

fn try_record(conn: &Connection, event: &UsageEvent) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO usage_events (ts, command, source, client, client_version, duration_ms, success)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            ts,
            event.command,
            event.source,
            event.client,
            event.client_version,
            event.duration_ms as i64,
            event.success as i64,
        ],
    )?;
    Ok(())
}

/// Opens the analytics DB and records one event in it, for one-shot CLI commands that
/// don't otherwise need a live connection. Same failure handling as [`record`]: never
/// fails the caller, just a stderr note.
pub fn record_new(event: UsageEvent) {
    if std::env::var_os("IMPACT_NO_ANALYTICS").is_some() {
        return;
    }
    match open() {
        Ok(conn) => record(&conn, event),
        Err(e) => eprintln!("impact: analytics write failed (ignored): {e}"),
    }
}

#[derive(Clone, Copy)]
pub enum Period {
    Daily,
    Weekly,
    Monthly,
}

impl Period {
    /// A SQLite expression (over the `ts` column, unix seconds) producing this period's
    /// bucket label: `YYYY-MM-DD` daily, `YYYY-Wnn` weekly (ISO-ish: Monday-first week
    /// of year, per `strftime('%W', ...)`), `YYYY-MM` monthly.
    fn bucket_expr(self) -> &'static str {
        match self {
            Period::Daily => "strftime('%Y-%m-%d', ts, 'unixepoch')",
            Period::Weekly => {
                "strftime('%Y', ts, 'unixepoch') || '-W' || strftime('%W', ts, 'unixepoch')"
            }
            Period::Monthly => "strftime('%Y-%m', ts, 'unixepoch')",
        }
    }
}

#[derive(Serialize)]
pub struct Bucket {
    pub label: String,
    pub total: u64,
    pub success: u64,
    pub by_client: Vec<(String, u64)>,
    pub by_command: Vec<(String, u64)>,
}

/// The most recent `limit` buckets (most recent first), each broken down by client and
/// by command — the data behind `impact gain`.
pub fn rollup(conn: &Connection, period: Period, limit: u32) -> Result<Vec<Bucket>> {
    let bucket_expr = period.bucket_expr();

    let mut totals_stmt = conn.prepare(&format!(
        "SELECT {bucket_expr} AS bucket, COUNT(*), SUM(success)
         FROM usage_events
         GROUP BY bucket
         ORDER BY bucket DESC
         LIMIT ?1"
    ))?;
    let mut buckets: Vec<Bucket> = totals_stmt
        .query_map(rusqlite::params![limit], |row| {
            Ok(Bucket {
                label: row.get(0)?,
                total: row.get::<_, i64>(1)? as u64,
                success: row.get::<_, i64>(2)? as u64,
                by_client: Vec::new(),
                by_command: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    for bucket in &mut buckets {
        bucket.by_client = grouped_counts(conn, bucket_expr, &bucket.label, "client")?;
        bucket.by_command = grouped_counts(conn, bucket_expr, &bucket.label, "command")?;
    }

    Ok(buckets)
}

fn grouped_counts(
    conn: &Connection,
    bucket_expr: &str,
    label: &str,
    column: &str,
) -> Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {column}, COUNT(*)
         FROM usage_events
         WHERE {bucket_expr} = ?1
         GROUP BY {column}
         ORDER BY COUNT(*) DESC"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params![label], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
