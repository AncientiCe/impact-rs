use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::engine::ImpactReport;
use crate::graph::{ContractKind, NodeKind, SymbolGraph};

/// One project registered in a `workspace.toml`. `path` and `cache_dir` (when set) are
/// resolved (at `load` time) relative to the workspace file's own directory, so they're
/// always ready to `join` without the caller re-resolving them.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceProject {
    pub id: String,
    pub path: PathBuf,
    /// Where this project's index cache lives, if not the `<path>/.impact` default —
    /// matches every other command's `--cache-dir` override, for the same reasons (a
    /// project indexed somewhere non-default, or a hermetic test needing an isolated
    /// cache without touching the checked-in project directory).
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
}

/// An explicit produce/consume relationship a human declared, because identity matching
/// alone can't tell two unrelated `POST /health` routes in different repos apart from a
/// real dependency. Each side is either `"<project_id>:<contract_id>"` (names one exact
/// contract) or bare `"<project_id>"` (declares the two *projects* related, without
/// committing to which contract) — see `link_confidence`.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceLink {
    pub produces: String,
    pub consumes: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub projects: Vec<WorkspaceProject>,
    #[serde(default)]
    pub links: Vec<WorkspaceLink>,
}

impl Workspace {
    /// Loads and parses `path`, resolving every project's `path` field relative to
    /// `path`'s own parent directory (so a `workspace.toml` written with `../web` keeps
    /// working regardless of the caller's current directory).
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut workspace: Workspace =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

        let base = path.parent().unwrap_or_else(|| Path::new("."));
        for project in &mut workspace.projects {
            if !project.path.is_absolute() {
                project.path = base.join(&project.path);
            }
            if let Some(cache_dir) = &mut project.cache_dir {
                if !cache_dir.is_absolute() {
                    *cache_dir = base.join(&cache_dir);
                }
            }
        }
        Ok(workspace)
    }

    /// Where `project`'s index cache lives: its explicit `cache_dir`, or the
    /// `<path>/.impact` default every other command uses.
    pub fn cache_dir_for(&self, project: &WorkspaceProject) -> PathBuf {
        project
            .cache_dir
            .clone()
            .unwrap_or_else(|| project.path.join(".impact"))
    }
}

/// How much a cross-project contract match should be trusted — see `WorkspaceLink`'s doc
/// for what "declared" means. Deliberately not a confidence *score*: the same workspace
/// and the same indexed graphs always produce the same tier for the same match, which is
/// the whole point — cross-project impact stays as deterministic as everything else this
/// tool reports, never a fuzzy/probabilistic guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkConfidence {
    /// A `[[links]]` entry names this exact contract on at least one side.
    Declared,
    /// No entry names this contract, but one relates these two projects generally.
    Strong,
    /// Identity match only — nothing in `workspace.toml` relates these two projects at
    /// all. The likeliest tier to be a false positive (two repos that both happen to
    /// expose `POST /health`, unrelated) — always shown, but clearly labeled so a caller
    /// can filter it out.
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossProjectMatch {
    pub project_id: String,
    pub contract_kind: ContractKind,
    pub contract_id: String,
    pub confidence: LinkConfidence,
}

/// A local `ImpactReport` extended with what it touches in other workspace projects.
/// Kept separate from `ImpactReport` itself rather than adding an always-empty field to
/// it — cross-project matching is an opt-in, `--workspace`-gated concern, not part of
/// what a plain single-project query means.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceImpactReport {
    #[serde(flatten)]
    pub local: ImpactReport,
    pub cross_project: Vec<CrossProjectMatch>,
}

/// Every contract this project's local `report` touches (its `api`/`events`/`database`
/// sections), matched by exact `(kind, id)` identity against `other_graphs` — the
/// already-indexed graphs of every *other* workspace project. A project with no indexed
/// cache simply isn't in `other_graphs` and is silently skipped, not an error: cross-
/// project matching is opportunistic over whatever has actually been indexed.
pub fn cross_project_matches(
    workspace: &Workspace,
    source_project_id: &str,
    report: &ImpactReport,
    other_graphs: &HashMap<String, SymbolGraph>,
) -> Vec<CrossProjectMatch> {
    let contracts: Vec<(ContractKind, &str)> = report
        .api
        .iter()
        .map(|c| (ContractKind::ApiRoute, c.as_str()))
        .chain(
            report
                .events
                .iter()
                .map(|c| (ContractKind::Event, c.as_str())),
        )
        .chain(
            report
                .database
                .iter()
                .map(|c| (ContractKind::Table, c.as_str())),
        )
        .collect();

    let mut out = Vec::new();
    for (other_id, graph) in other_graphs {
        for (kind, contract_id) in &contracts {
            let found = graph.nodes().any(|n| {
                matches!(n.kind, NodeKind::Contract(k) if k == *kind)
                    && n.qualified_path == *contract_id
            });
            if !found {
                continue;
            }
            let confidence = link_confidence(workspace, source_project_id, other_id, contract_id);
            out.push(CrossProjectMatch {
                project_id: other_id.clone(),
                contract_kind: *kind,
                contract_id: contract_id.to_string(),
                confidence,
            });
        }
    }
    // Deterministic output order: `other_graphs` is a HashMap, so iteration order alone
    // isn't stable across runs.
    out.sort_by(|a, b| {
        (
            &a.project_id,
            format!("{:?}", a.contract_kind),
            &a.contract_id,
        )
            .cmp(&(
                &b.project_id,
                format!("{:?}", b.contract_kind),
                &b.contract_id,
            ))
    });
    out
}

/// Whether `link`, read in either direction, pairs a side naming `exact_contract` with a
/// side naming project `project` (bare, or also exactly qualified to `project_contract`).
fn link_pairs_contract_with_project(
    link: &WorkspaceLink,
    exact_contract: &str,
    project: &str,
    project_contract: &str,
) -> bool {
    let names_project = |s: &str| s == project || s == project_contract;
    (link.produces == exact_contract && names_project(&link.consumes))
        || (link.consumes == exact_contract && names_project(&link.produces))
}

fn link_confidence(
    workspace: &Workspace,
    source: &str,
    other: &str,
    contract_id: &str,
) -> LinkConfidence {
    let source_qualified = format!("{source}:{contract_id}");
    let other_qualified = format!("{other}:{contract_id}");

    // Declared: some link names this exact contract on (at least) one side — in either
    // project — with the other side naming the other project, bare or also exact. This
    // is deliberately symmetric in both which side is qualified and which project is
    // named first: `produces = "backend:POST /payments"` / `consumes = "web"` (the
    // canonical example) and its mirror image should both count.
    let declared = workspace.links.iter().any(|link| {
        link_pairs_contract_with_project(link, &source_qualified, other, &other_qualified)
            || link_pairs_contract_with_project(link, &other_qualified, source, &source_qualified)
    });
    if declared {
        return LinkConfidence::Declared;
    }

    // Strong: no link names this specific contract, but some link relates these two
    // projects generally (both sides bare).
    let strong = workspace.links.iter().any(|link| {
        (link.produces == source && link.consumes == other)
            || (link.consumes == source && link.produces == other)
    });
    if strong {
        return LinkConfidence::Strong;
    }

    LinkConfidence::Weak
}
