use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::adapter::LanguageAdapter;
use crate::cache::Cache;
use crate::graph::{ContractKind, Node, NodeId, NodeKind};

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub symbols_indexed: usize,
    pub duration_ms: u128,
}

/// Walks a project, routes each file to whichever registered adapter claims it, and
/// persists extracted symbols into the cache — skipping files whose content hash hasn't
/// changed since the last run.
pub struct Indexer<'a> {
    project_id: String,
    adapters: Vec<&'a dyn LanguageAdapter>,
}

impl<'a> Indexer<'a> {
    pub fn new(project_id: impl Into<String>, adapters: Vec<&'a dyn LanguageAdapter>) -> Self {
        Self {
            project_id: project_id.into(),
            adapters,
        }
    }

    pub fn index(&self, project_root: &Path, cache: &mut Cache) -> Result<IndexStats> {
        let start = Instant::now();
        let mut stats = IndexStats::default();

        let routed = self.build_routes()?;

        for entry in WalkBuilder::new(project_root).build() {
            let entry = entry?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(project_root)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            let Some(adapter) = routed
                .iter()
                .find(|(_, globs)| globs.is_match(&rel_str))
                .map(|(a, _)| *a)
            else {
                continue;
            };

            let content = std::fs::read_to_string(path)?;
            let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

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
                })
                .collect();

            stats.symbols_indexed += nodes.len();
            cache.replace_file(&rel_str, &content_hash, &nodes, &refs, &contract_refs)?;
            stats.files_indexed += 1;
        }

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
            };
            graph.insert_node(node.clone());
            synthesized.push(node);
        }
        if !synthesized.is_empty() {
            cache.upsert_nodes(&synthesized)?;
        }

        let edges = crate::linker::link(&graph, &all_refs, &all_contract_refs);
        cache.replace_edges(&edges)?;

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
}
