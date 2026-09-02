use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::adapter::{ContractRef, ContractRole, RefDecl};
use crate::graph::{ContractKind, Edge, EdgeKind, Node, SymbolGraph};

/// Bumped whenever the table shapes below change in a way an already-cached database
/// can't transparently keep using (a new column, a changed meaning for an existing one).
/// `migrate` compares this against the database's own `PRAGMA user_version` and wipes
/// every table before recreating them on a mismatch — simpler and safer than writing a
/// column-by-column migration for a local, fully-rebuildable index cache.
const SCHEMA_VERSION: i32 = 1;

/// Per-project SQLite-backed cache of the last-indexed graph, keyed by file content hash
/// so unchanged files can skip re-parsing on the next index run.
pub struct Cache {
    conn: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating cache directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening cache database {}", path.display()))?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> Result<()> {
        let stored_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        // A brand-new cache (no `nodes` table yet) is never "stale" — it just hasn't been
        // stamped with a version yet, so skip the wipe (a no-op on empty tables anyway)
        // and the message that would otherwise print on every first-time `impact index`.
        let has_existing_tables: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'nodes'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if has_existing_tables && stored_version < SCHEMA_VERSION {
            eprintln!(
                "impact: cache schema changed (v{stored_version} -> v{SCHEMA_VERSION}), wiping and re-indexing from scratch"
            );
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS file_hashes;
                DROP TABLE IF EXISTS nodes;
                DROP TABLE IF EXISTS edges;
                DROP TABLE IF EXISTS refs;
                DROP TABLE IF EXISTS contract_refs;
                ",
            )?;
        }
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS file_hashes (
                file TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                qualified_path TEXT NOT NULL,
                file TEXT NOT NULL,
                line INTEGER NOT NULL,
                language TEXT NOT NULL,
                is_test INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS nodes_file_idx ON nodes(file);
            CREATE TABLE IF NOT EXISTS edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                confidence TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS refs (
                file TEXT NOT NULL,
                from_qualified_path TEXT NOT NULL,
                to_name TEXT NOT NULL,
                kind TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS refs_file_idx ON refs(file);
            CREATE TABLE IF NOT EXISTS contract_refs (
                file TEXT NOT NULL,
                contract_kind TEXT NOT NULL,
                contract_id TEXT NOT NULL,
                symbol_name TEXT NOT NULL,
                role TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS contract_refs_file_idx ON contract_refs(file);
            ",
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    pub fn file_hash(&self, file: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT content_hash FROM file_hashes WHERE file = ?1",
                params![file],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    /// Wipes every table — for a forced full re-index that shouldn't trust any previous
    /// content-hash skip, e.g. after upgrading `impact` itself changes how symbols are
    /// extracted.
    pub fn clear(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM file_hashes", [])?;
        tx.execute("DELETE FROM nodes", [])?;
        tx.execute("DELETE FROM edges", [])?;
        tx.execute("DELETE FROM refs", [])?;
        tx.execute("DELETE FROM contract_refs", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Replaces every node, reference, and contract reference previously indexed from
    /// `file`, and records `content_hash` as the file's current state, all in one
    /// transaction. Edges aren't touched here — they depend on the whole project's
    /// symbol table, not one file, so the indexer recomputes them separately via
    /// `all_refs`/`all_contract_refs` + `replace_edges` after every file has been
    /// (re)indexed.
    pub fn replace_file(
        &mut self,
        file: &str,
        content_hash: &str,
        nodes: &[Node],
        refs: &[RefDecl],
        contract_refs: &[ContractRef],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM nodes WHERE file = ?1", params![file])?;
        for node in nodes {
            let kind_json = serde_json::to_string(&node.kind)?;
            tx.execute(
                "INSERT OR REPLACE INTO nodes (id, kind, qualified_path, file, line, language, is_test)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    node.id.0,
                    kind_json,
                    node.qualified_path,
                    node.file,
                    node.line as i64,
                    node.language,
                    node.is_test as i64,
                ],
            )?;
        }
        tx.execute("DELETE FROM refs WHERE file = ?1", params![file])?;
        for r in refs {
            let kind_json = serde_json::to_string(&r.kind)?;
            tx.execute(
                "INSERT INTO refs (file, from_qualified_path, to_name, kind) VALUES (?1, ?2, ?3, ?4)",
                params![file, r.from_qualified_path, r.to_name, kind_json],
            )?;
        }
        tx.execute("DELETE FROM contract_refs WHERE file = ?1", params![file])?;
        for cr in contract_refs {
            let kind_json = serde_json::to_string(&cr.contract_kind)?;
            let role_str = role_to_str(cr.role);
            tx.execute(
                "INSERT INTO contract_refs (file, contract_kind, contract_id, symbol_name, role)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![file, kind_json, cr.contract_id, cr.symbol_name, role_str],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO file_hashes (file, content_hash) VALUES (?1, ?2)",
            params![file, content_hash],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Every reference indexed across the whole project, regardless of which files were
    /// (re)parsed on this run — the linker needs the full set to resolve cross-file calls.
    pub fn all_refs(&self) -> Result<Vec<RefDecl>> {
        let mut stmt = self
            .conn
            .prepare("SELECT from_qualified_path, to_name, kind FROM refs")?;
        let rows = stmt.query_map([], |row| {
            let from_qualified_path: String = row.get(0)?;
            let to_name: String = row.get(1)?;
            let kind_json: String = row.get(2)?;
            Ok((from_qualified_path, to_name, kind_json))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (from_qualified_path, to_name, kind_json) = row?;
            let kind: EdgeKind = serde_json::from_str(&kind_json)?;
            out.push(RefDecl {
                from_qualified_path,
                to_name,
                kind,
            });
        }
        Ok(out)
    }

    /// Every contract reference indexed across the whole project — same rationale as
    /// `all_refs`.
    pub fn all_contract_refs(&self) -> Result<Vec<ContractRef>> {
        let mut stmt = self
            .conn
            .prepare("SELECT contract_kind, contract_id, symbol_name, role FROM contract_refs")?;
        let rows = stmt.query_map([], |row| {
            let contract_kind_json: String = row.get(0)?;
            let contract_id: String = row.get(1)?;
            let symbol_name: String = row.get(2)?;
            let role_str: String = row.get(3)?;
            Ok((contract_kind_json, contract_id, symbol_name, role_str))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (contract_kind_json, contract_id, symbol_name, role_str) = row?;
            let contract_kind: ContractKind = serde_json::from_str(&contract_kind_json)?;
            let Some(role) = role_from_str(&role_str) else {
                continue;
            };
            out.push(ContractRef {
                contract_kind,
                contract_id,
                symbol_name,
                role,
            });
        }
        Ok(out)
    }

    /// Inserts or replaces a set of nodes without deleting anything file-scoped first —
    /// for nodes that don't belong to one file the way a parsed symbol does, namely the
    /// `Contract` nodes the indexer synthesizes for API routes and database tables (an
    /// identity that's referenced into existence — a `.route()` call or a `FROM` clause
    /// — rather than declared the way an event's marker-trait `impl` declares it).
    pub fn upsert_nodes(&mut self, nodes: &[Node]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for node in nodes {
            let kind_json = serde_json::to_string(&node.kind)?;
            tx.execute(
                "INSERT OR REPLACE INTO nodes (id, kind, qualified_path, file, line, language, is_test)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    node.id.0,
                    kind_json,
                    node.qualified_path,
                    node.file,
                    node.line as i64,
                    node.language,
                    node.is_test as i64,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Wholesale-replaces the edges table. Edges are cheap to fully recompute from
    /// `all_refs`/`all_contract_refs` on every index run, so there's no incremental edge
    /// cache to keep in sync — simpler, and correctness doesn't depend on tracking which
    /// edges a changed file's refs used to produce.
    pub fn replace_edges(&mut self, edges: &[Edge]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM edges", [])?;
        for edge in edges {
            let kind_json = serde_json::to_string(&edge.kind)?;
            let confidence_json = serde_json::to_string(&edge.confidence)?;
            tx.execute(
                "INSERT INTO edges (from_id, to_id, kind, confidence) VALUES (?1, ?2, ?3, ?4)",
                params![edge.from.0, edge.to.0, kind_json, confidence_json],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Loads the entire cached graph into memory for querying. Fine at this project's
    /// scale (an agent-session workload); revisit if it ever needs to avoid a full load.
    pub fn load_graph(&self) -> Result<SymbolGraph> {
        let mut graph = SymbolGraph::new();

        let mut node_stmt = self
            .conn
            .prepare("SELECT id, kind, qualified_path, file, line, language, is_test FROM nodes")?;
        let node_rows = node_stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let kind_json: String = row.get(1)?;
            let qualified_path: String = row.get(2)?;
            let file: String = row.get(3)?;
            let line: i64 = row.get(4)?;
            let language: String = row.get(5)?;
            let is_test: i64 = row.get(6)?;
            Ok((id, kind_json, qualified_path, file, line, language, is_test))
        })?;
        for row in node_rows {
            let (id, kind_json, qualified_path, file, line, language, is_test) = row?;
            let kind = serde_json::from_str(&kind_json)?;
            graph.insert_node(Node {
                id: crate::graph::NodeId(id),
                kind,
                qualified_path,
                file,
                line: line as usize,
                language,
                is_test: is_test != 0,
            });
        }

        let mut edge_stmt = self
            .conn
            .prepare("SELECT from_id, to_id, kind, confidence FROM edges")?;
        let edge_rows = edge_stmt.query_map([], |row| {
            let from_id: String = row.get(0)?;
            let to_id: String = row.get(1)?;
            let kind_json: String = row.get(2)?;
            let confidence_json: String = row.get(3)?;
            Ok((from_id, to_id, kind_json, confidence_json))
        })?;
        for row in edge_rows {
            let (from_id, to_id, kind_json, confidence_json) = row?;
            let kind = serde_json::from_str(&kind_json)?;
            let confidence = serde_json::from_str(&confidence_json)?;
            graph.insert_edge(Edge {
                from: crate::graph::NodeId(from_id),
                to: crate::graph::NodeId(to_id),
                kind,
                confidence,
            });
        }

        Ok(graph)
    }
}

fn role_to_str(role: ContractRole) -> &'static str {
    match role {
        ContractRole::Produces => "produces",
        ContractRole::Consumes => "consumes",
        ContractRole::Reads => "reads",
        ContractRole::Writes => "writes",
    }
}

fn role_from_str(s: &str) -> Option<ContractRole> {
    match s {
        "produces" => Some(ContractRole::Produces),
        "consumes" => Some(ContractRole::Consumes),
        "reads" => Some(ContractRole::Reads),
        "writes" => Some(ContractRole::Writes),
        _ => None,
    }
}
