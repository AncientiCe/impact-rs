use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::adapter::LanguageAdapter;
use crate::cache::Cache;
use crate::graph::{ContractKind, Edge, EdgeKind, Node, NodeId, NodeKind, SymbolGraph};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    /// Files dropped from the cache because they're no longer on disk — see
    /// `Cache::prune_missing`. Counted separately from `files_indexed`/`files_skipped`,
    /// which only describe files this run actually found.
    pub files_pruned: usize,
    pub symbols_indexed: usize,
    /// An event contract that had at least one producer *and* one consumer before this
    /// run and lost one of those sides on this run — see `event_wiring`. Not proof of a
    /// mistake (removing an event's last producer is often exactly the point of a
    /// change), but worth a second look: this is the shape a producer-to-direct-call
    /// migration takes when the replacement forgets behavior the old consumer had.
    pub orphaned_events: Vec<OrphanedEvent>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OrphanedEvent {
    pub event: String,
    pub lost_producer: bool,
    pub lost_consumer: bool,
}

/// Whether each `Event` contract declared in `graph` currently has at least one
/// `Produces` edge and at least one `Consumes` edge pointing at it, among `edges`.
/// `edges` is taken separately rather than read off `graph.edges()` because the graph
/// loaded fresh from the cache still carries the *previous* run's persisted edges until
/// `Cache::replace_edges` overwrites them — using the freshly linked set here keeps this
/// answering "right now", not "as of the last completed run". Contracts with neither
/// reachable (declared but never produced or consumed) are included too, as
/// `(false, false)` — a baseline `event_diff` can still detect against.
fn event_wiring(
    graph: &SymbolGraph,
    edges: &[Edge],
) -> std::collections::HashMap<String, (bool, bool)> {
    let mut wiring: std::collections::HashMap<String, (bool, bool)> = graph
        .nodes()
        .filter(|n| matches!(n.kind, NodeKind::Contract(ContractKind::Event)))
        .map(|n| (n.qualified_path.clone(), (false, false)))
        .collect();
    for edge in edges {
        if !matches!(edge.kind, EdgeKind::Produces | EdgeKind::Consumes) {
            continue;
        }
        let Some(target) = graph.node(&edge.to) else {
            continue;
        };
        if !matches!(target.kind, NodeKind::Contract(ContractKind::Event)) {
            continue;
        }
        let entry = wiring.entry(target.qualified_path.clone()).or_default();
        match edge.kind {
            EdgeKind::Produces => entry.0 = true,
            EdgeKind::Consumes => entry.1 = true,
            _ => unreachable!(),
        }
    }
    wiring
}

/// Diffs two `event_wiring` snapshots: an event that had both a producer and a consumer
/// `before` and is missing one of those `after` is reported, naming which side it lost.
/// An event absent from `after` entirely (its declaration itself was removed) counts as
/// having lost both.
fn event_diff(
    before: &std::collections::HashMap<String, (bool, bool)>,
    after: &std::collections::HashMap<String, (bool, bool)>,
) -> Vec<OrphanedEvent> {
    let mut out: Vec<OrphanedEvent> = before
        .iter()
        .filter(|(_, (had_producer, had_consumer))| *had_producer && *had_consumer)
        .filter_map(|(event, _)| {
            let (has_producer, has_consumer) = after.get(event).copied().unwrap_or((false, false));
            if has_producer && has_consumer {
                return None;
            }
            Some(OrphanedEvent {
                event: event.clone(),
                lost_producer: !has_producer,
                lost_consumer: !has_consumer,
            })
        })
        .collect();
    out.sort_by(|a, b| a.event.cmp(&b.event));
    out
}

/// Walks a project, routes each file to whichever registered adapter claims it, and
/// persists extracted symbols into the cache — skipping files whose content hash hasn't
/// changed since the last run.
pub struct Indexer<'a> {
    project_id: String,
    adapters: Vec<&'a dyn LanguageAdapter>,
    exclude: Vec<String>,
}

impl<'a> Indexer<'a> {
    pub fn new(project_id: impl Into<String>, adapters: Vec<&'a dyn LanguageAdapter>) -> Self {
        Self {
            project_id: project_id.into(),
            adapters,
            exclude: Vec::new(),
        }
    }

    /// Extra glob patterns (see `IndexConfig::exclude`) that skip a file even though some
    /// adapter's globs would otherwise claim it.
    pub fn with_exclude(mut self, exclude: Vec<String>) -> Self {
        self.exclude = exclude;
        self
    }

    pub fn index(&self, project_root: &Path, cache: &mut Cache) -> Result<IndexStats> {
        let start = Instant::now();
        let mut stats = IndexStats::default();

        // Snapshotted before any file in this run is touched, so a file this run
        // re-indexes can't already reflect its own change by the time it's compared
        // against — see `event_diff` at the end of this function. This is the one place
        // `graph.edges()` is the right source: it's read before this run has changed
        // anything, so it genuinely is the previous run's fully-committed state.
        let before_graph = cache.load_graph()?;
        let events_before = event_wiring(&before_graph, before_graph.edges());

        let routed = self.build_routes()?;
        let exclude = self.build_exclude()?;
        // Every file this run found and claimed for an adapter, whether it was re-parsed
        // or skipped as unchanged — the set `prune_missing` diffs the cache against.
        let mut seen: HashSet<String> = HashSet::new();

        for entry in WalkBuilder::new(project_root).build() {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(project_root)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if exclude.is_match(&rel_str) {
                continue;
            }

            let Some(adapter) = routed
                .iter()
                .find(|(_, globs)| globs.is_match(&rel_str))
                .map(|(a, _)| *a)
            else {
                continue;
            };

            let content = std::fs::read_to_string(path)?;
            let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

            seen.insert(rel_str.clone());

            if cache.file_hash(&rel_str)?.as_deref() == Some(content_hash.as_str()) {
                stats.files_skipped += 1;
                continue;
            }

            let ast = adapter.parse_file(rel, &content)?;
            let decls = adapter.extract_symbols(&ast);
            let refs = adapter.extract_references(&ast);
            let contract_refs = adapter.extract_contract_refs(&ast);

            let nodes: Vec<Node> = decls
                .into_iter()
                .map(|decl| Node {
                    id: NodeId::new(&self.project_id, &decl.qualified_path, decl.kind),
                    kind: decl.kind,
                    qualified_path: decl.qualified_path,
                    file: rel_str.clone(),
                    line: decl.line,
                    end_line: decl.end_line,
                    language: adapter.language_id().to_string(),
                    is_test: decl.is_test,
                    is_generated: decl.is_generated,
                    is_default_export: decl.is_default_export,
                })
                .collect();

            stats.symbols_indexed += nodes.len();
            cache.replace_file(&rel_str, &content_hash, &nodes, &refs, &contract_refs)?;
            stats.files_indexed += 1;
        }

        // A file that vanished from disk has to leave the cache before the graph is
        // rebuilt, or its symbols keep resolving as live callers of everything they used
        // to call.
        stats.files_pruned = cache.prune_missing(&seen)?;

        // Edges depend on the whole project's symbol table, not just the files touched
        // on this run, so they're always fully recomputed from every cached ref — see
        // `Cache::replace_edges`.
        let mut graph = cache.load_graph()?;
        let all_refs = cache.all_refs()?;
        let all_contract_refs = cache.all_contract_refs()?;

        // Unlike an event (declared by its marker-trait `impl`), an API route or a
        // database table has no separate declaration syntax — the `.route()` call or the
        // `FROM`/`INTO` clause that references it *is* its only declaration, so the
        // indexer synthesizes a Contract node for any such id that doesn't already exist.
        // Events are deliberately excluded: their Produces/Consumes refs are emitted
        // over-broadly (every constructed type, every typed parameter) specifically so
        // the *absence* of a declared contract node filters out the non-events.
        let mut known_contracts: HashSet<(ContractKind, String)> = graph
            .nodes()
            .filter_map(|n| match n.kind {
                NodeKind::Contract(kind) => Some((kind, n.qualified_path.clone())),
                _ => None,
            })
            .collect();
        let mut synthesized = Vec::new();
        for cr in &all_contract_refs {
            if !matches!(
                cr.contract_kind,
                ContractKind::ApiRoute | ContractKind::Table
            ) {
                continue;
            }
            let key = (cr.contract_kind, cr.contract_id.clone());
            if !known_contracts.insert(key) {
                continue;
            }
            let node = Node {
                id: NodeId::new(
                    &self.project_id,
                    &cr.contract_id,
                    NodeKind::Contract(cr.contract_kind),
                ),
                kind: NodeKind::Contract(cr.contract_kind),
                qualified_path: cr.contract_id.clone(),
                file: String::new(),
                line: 0,
                end_line: 0,
                language: String::new(),
                is_test: false,
                is_generated: false,
                is_default_export: false,
            };
            graph.insert_node(node.clone());
            synthesized.push(node);
        }
        if !synthesized.is_empty() {
            cache.upsert_nodes(&synthesized)?;
        }

        let edges = crate::linker::link(&graph, &all_refs, &all_contract_refs);
        cache.replace_edges(&edges)?;

        stats.orphaned_events = event_diff(&events_before, &event_wiring(&graph, &edges));

        stats.duration_ms = start.elapsed().as_millis();
        Ok(stats)
    }

    fn build_routes(&self) -> Result<Vec<(&'a dyn LanguageAdapter, GlobSet)>> {
        self.adapters
            .iter()
            .map(|adapter| {
                let mut builder = GlobSetBuilder::new();
                for pattern in adapter.file_globs() {
                    builder.add(Glob::new(pattern)?);
                }
                Ok((*adapter, builder.build()?))
            })
            .collect()
    }

    fn build_exclude(&self) -> Result<GlobSet> {
        let mut builder = GlobSetBuilder::new();
        for pattern in &self.exclude {
            builder.add(Glob::new(pattern)?);
        }
        Ok(builder.build()?)
    }
}
