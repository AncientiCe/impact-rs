//! Shared computation behind every subcommand and MCP tool — `main.rs` and `mcp.rs` each
//! wrap these in their own presentation (tree-text/JSON on stdout, or an MCP tool result)
//! but neither reimplements the logic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use impact_core::{
    Cache, ChangeSpec, DetectorConfig, ImpactReport, IndexConfig, IndexStats, Indexer,
    LanguageAdapter, Workspace, WorkspaceImpactReport,
};
use impact_lang_cpp::CppAdapter;
use impact_lang_go::GoAdapter;
use impact_lang_kotlin::KotlinAdapter;
use impact_lang_python::PythonAdapter;
use impact_lang_rust::RustAdapter;
use impact_lang_swift::SwiftAdapter;
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
    } else if cache.ensure_extractor_version()? {
        // Content hashes only tell us whether a file changed, not whether the extractor
        // did — so a cache from another build has to be rebuilt even though every hash
        // still matches. Say so, rather than silently re-parsing everything.
        eprintln!(
            "impact: cache was built by a different impact version, re-indexing from scratch"
        );
    }

    let config = DetectorConfig::load(&project_root)?;
    let rust_adapter = RustAdapter::new(config.clone());
    let ts_adapter = TsAdapter::new(config.clone(), &project_root);
    let python_adapter = PythonAdapter::new(config.clone());
    let go_adapter = GoAdapter::new(config);
    let kotlin_adapter = KotlinAdapter;
    let swift_adapter = SwiftAdapter;
    let cpp_adapter = CppAdapter;
    let adapters: Vec<&dyn LanguageAdapter> = vec![
        &rust_adapter,
        &ts_adapter,
        &python_adapter,
        &go_adapter,
        &kotlin_adapter,
        &swift_adapter,
        &cpp_adapter,
    ];
    let index_config = IndexConfig::load(&project_root)?;
    let indexer = Indexer::new(project_id, adapters).with_exclude(index_config.exclude);

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

/// Diff-mode query: parses a unified diff (e.g. `git diff` output) and computes the blast
/// radius of every symbol its touched lines fall inside — see
/// `impact_core::compute_diff_impact` for how "falls inside" is resolved. File paths in
/// the diff are matched against the index as-is (after the parser's `a/`/`b/` strip), so
/// the diff needs paths relative to `project` — exactly what `git diff` run inside the
/// project produces.
pub fn diff_impact(
    diff_text: &str,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
) -> anyhow::Result<ImpactReport> {
    let (_project_root, cache) = open_project_cache(project, cache_dir)?;
    let graph = cache.load_graph()?;
    let touches = impact_core::parse_unified_diff(diff_text);

    Ok(impact_core::compute_diff_impact(&graph, &touches))
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
        let target = spec.target_path();
        anyhow::anyhow!(
            "\"{target}\" doesn't resolve to anything in the indexed project — check the \
             path, or run `impact index` again if the project has changed since the last \
             index{}",
            bare_name_hint(graph, &target)
        )
    })
}

/// When a full path fails every tier of `Resolver::resolve` (see its doc comment — exact
/// qualified path, then last-two-segments, then bare short name), the most common real
/// cause is a wrong module/package qualifier on an otherwise-correct symbol name: someone
/// guessed the qualified-path syntax and got the prefix wrong. The resolver's own weakest
/// tier already has the answer to "is there a symbol with this bare name at all" — reusing
/// it here turns a dead-end error into a concrete suggestion instead of leaving the caller
/// to guess a second time.
///
/// Returns `""` when `target` has no `::` (the bare-name tier was already tried as part of
/// the failed resolution itself, so there's nothing more precise left to suggest) or when
/// even the bare trailing segment doesn't match anything.
fn bare_name_hint(graph: &impact_core::SymbolGraph, target: &str) -> String {
    let trailing = target.rsplit("::").next().unwrap_or(target);
    if trailing == target {
        return String::new();
    }
    let resolver = impact_core::Resolver::build(graph);
    let Some((ids, _confidence)) = resolver.resolve(trailing) else {
        return String::new();
    };
    let mut names: Vec<&str> = ids
        .iter()
        .filter_map(|id| graph.node(id).map(|n| n.qualified_path.as_str()))
        .collect();
    names.sort_unstable();
    names.dedup();
    format!(
        " — but the bare name {trailing:?} matches: {}",
        names.join(", ")
    )
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
