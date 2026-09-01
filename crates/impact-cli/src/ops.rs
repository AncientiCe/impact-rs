//! Shared computation behind every subcommand and MCP tool — `main.rs` and `mcp.rs` each
//! wrap these in their own presentation (tree-text/JSON on stdout, or an MCP tool result)
//! but neither reimplements the logic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use impact_core::{
    Cache, ChangeSpec, DetectorConfig, ImpactReport, IndexStats, Indexer, LanguageAdapter,
    Workspace, WorkspaceImpactReport,
};
use impact_lang_rust::RustAdapter;
use impact_lang_ts::TsAdapter;

fn cache_path(project_root: &Path, cache_dir: Option<&Path>) -> PathBuf {
    cache_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_root.join(".impact"))
        .join("cache.sqlite")
}

pub fn index_project(
    path: &Path,
    cache_dir: Option<&Path>,
    force: bool,
) -> anyhow::Result<IndexStats> {
    let project_root = path.canonicalize()?;
    let project_id = project_root.to_string_lossy().to_string();
    let mut cache = Cache::open(&cache_path(&project_root, cache_dir))?;
    if force {
        cache.clear()?;
    }

    let config = DetectorConfig::load(&project_root)?;
    let rust_adapter = RustAdapter::new(config);
    let ts_adapter = TsAdapter;
    let adapters: Vec<&dyn LanguageAdapter> = vec![&rust_adapter, &ts_adapter];
    let indexer = Indexer::new(project_id, adapters);

    indexer.index(&project_root, &mut cache)
}

/// Resolves `--project`/`--cache-dir` the same way for every command that reads an
/// existing index (`query`, `change`) and opens the cache at that location.
fn open_project_cache(
    project: Option<&Path>,
    cache_dir: Option<&Path>,
) -> anyhow::Result<(PathBuf, Cache)> {
    let project_root = project.unwrap_or_else(|| Path::new(".")).canonicalize()?;
    let cache = Cache::open(&cache_path(&project_root, cache_dir))?;
    Ok((project_root, cache))
}

pub fn query_file(
    path: &Path,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
) -> anyhow::Result<ImpactReport> {
    let (project_root, cache) = open_project_cache(project, cache_dir)?;
    let graph = cache.load_graph()?;

    let file_abs = if path.is_absolute() {
        path.canonicalize()?
    } else {
        project_root.join(path).canonicalize()?
    };
    let rel_file = file_abs
        .strip_prefix(&project_root)?
        .to_string_lossy()
        .replace('\\', "/");

    Ok(impact_core::compute_file_impact(&graph, &rel_file))
}

pub fn apply_change(
    description: &str,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
) -> anyhow::Result<ImpactReport> {
    let spec = impact_core::parse_change(description)?;
    let (_project_root, cache) = open_project_cache(project, cache_dir)?;
    let graph = cache.load_graph()?;

    change_report(&graph, &spec)
}

fn change_report(
    graph: &impact_core::SymbolGraph,
    spec: &ChangeSpec,
) -> anyhow::Result<ImpactReport> {
    impact_core::compute_change_impact(graph, spec).ok_or_else(|| {
        anyhow::anyhow!(
            "\"{}\" doesn't resolve to anything in the indexed project — check the path, \
             or run `impact index` again if the project has changed since the last index",
            spec.target_path()
        )
    })
}

/// Extends `local` with what it touches in other projects registered in the
/// `workspace.toml` at `workspace_path` — see `impact_core::workspace` for the matching
/// and confidence-tiering rules. `project` is resolved the same way every other command
/// resolves it, and must match one of the workspace's registered project paths.
///
/// A sibling project that hasn't been indexed yet (no `.impact/cache.sqlite`) is silently
/// skipped rather than an error: cross-project matching is opportunistic over whatever
/// has actually been indexed, not a requirement that the whole workspace be indexed first.
pub fn cross_project_report(
    local: ImpactReport,
    project: Option<&Path>,
    workspace_path: &Path,
) -> anyhow::Result<WorkspaceImpactReport> {
    let project_root = project.unwrap_or_else(|| Path::new(".")).canonicalize()?;
    let workspace = Workspace::load(workspace_path)?;

    let source_id = workspace
        .projects
        .iter()
        .find(|p| {
            p.path
                .canonicalize()
                .map(|c| c == project_root)
                .unwrap_or(false)
        })
        .map(|p| p.id.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not registered in workspace {}",
                project_root.display(),
                workspace_path.display()
            )
        })?;

    let mut other_graphs = HashMap::new();
    for other in &workspace.projects {
        if other.id == source_id {
            continue;
        }
        let cache_file = workspace.cache_dir_for(other).join("cache.sqlite");
        if !cache_file.exists() {
            continue;
        }
        if let Ok(cache) = Cache::open(&cache_file) {
            if let Ok(graph) = cache.load_graph() {
                other_graphs.insert(other.id.clone(), graph);
            }
        }
    }

    let cross_project =
        impact_core::cross_project_matches(&workspace, &source_id, &local, &other_graphs);
    Ok(WorkspaceImpactReport {
        local,
        cross_project,
    })
}
