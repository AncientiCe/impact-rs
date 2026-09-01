use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::change::ChangeSpec;
use crate::graph::{ContractKind, EdgeKind, NodeId, NodeKind, SymbolGraph};
use crate::linker::Resolver;

/// The blast radius of a change: who's affected directly, who's affected transitively
/// through them, which API routes / event types / database tables the affected symbols
/// touch, and how many tests exercise any of it. Sorted, so the same graph always
/// produces the same report — determinism is the whole point of this tool.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ImpactReport {
    pub direct: Vec<String>,
    pub indirect: Vec<String>,
    pub api: Vec<String>,
    pub events: Vec<String>,
    pub database: Vec<String>,
    pub tests: usize,
}

/// Computes the full blast radius of `seeds`: DIRECT (1-hop reverse dependents) and
/// INDIRECT (2+-hop reverse dependents), then the API/EVENTS/DATABASE contracts reachable
/// from the seeds or any of their dependents, and finally a count of how many of the
/// dependents found are test functions.
///
/// "Reverse dependent" walks backward over `Calls`/`References` edges (who calls this)
/// *and* `Produces`/`Consumes`/`Reads`/`Writes` edges (who touches this contract) as one
/// unified graph. That second half matters when a seed is (or belongs to) a file that
/// declares an event type or is the only place a table name is written, rather than the
/// one calling into it — without it, querying an event's own definition file would report
/// nothing, even though every producer and consumer of that event is exactly its blast
/// radius.
fn compute_impact(graph: &SymbolGraph, seeds: HashSet<NodeId>) -> ImpactReport {
    let mut callers: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in graph.edges() {
        if matches!(
            edge.kind,
            EdgeKind::Calls
                | EdgeKind::References
                | EdgeKind::Produces
                | EdgeKind::Consumes
                | EdgeKind::Reads
                | EdgeKind::Writes
        ) {
            callers
                .entry(edge.to.clone())
                .or_default()
                .push(edge.from.clone());
        }
    }

    let mut direct: BTreeSet<String> = BTreeSet::new();
    let mut indirect: BTreeSet<String> = BTreeSet::new();
    let mut visited: HashSet<NodeId> = seeds.clone();

    let mut frontier: Vec<NodeId> = seeds.iter().cloned().collect();
    let mut hop = 0;
    while !frontier.is_empty() {
        hop += 1;
        let mut next = Vec::new();
        for node_id in &frontier {
            let Some(node_callers) = callers.get(node_id) else {
                continue;
            };
            for caller in node_callers {
                if !visited.insert(caller.clone()) {
                    continue;
                }
                let name = graph
                    .node(caller)
                    .map(|n| n.qualified_path.clone())
                    .unwrap_or_else(|| caller.to_string());
                if hop == 1 {
                    direct.insert(name);
                } else {
                    indirect.insert(name);
                }
                next.push(caller.clone());
            }
        }
        frontier = next;
    }

    let mut api: BTreeSet<String> = BTreeSet::new();
    let mut events: BTreeSet<String> = BTreeSet::new();
    let mut database: BTreeSet<String> = BTreeSet::new();
    for edge in graph.edges() {
        if !matches!(
            edge.kind,
            EdgeKind::Produces | EdgeKind::Consumes | EdgeKind::Reads | EdgeKind::Writes
        ) {
            continue;
        }
        if !visited.contains(&edge.from) {
            continue;
        }
        let Some(contract) = graph.node(&edge.to) else {
            continue;
        };
        let NodeKind::Contract(kind) = contract.kind else {
            continue;
        };
        match kind {
            ContractKind::ApiRoute => {
                api.insert(contract.qualified_path.clone());
            }
            ContractKind::Event => {
                events.insert(contract.qualified_path.clone());
            }
            ContractKind::Table => {
                database.insert(contract.qualified_path.clone());
            }
            ContractKind::Test => {}
        }
    }

    let tests = visited
        .iter()
        .filter(|id| !seeds.contains(*id))
        .filter_map(|id| graph.node(id))
        .filter(|n| n.is_test)
        .count();

    ImpactReport {
        direct: direct.into_iter().collect(),
        indirect: indirect.into_iter().collect(),
        api: api.into_iter().collect(),
        events: events.into_iter().collect(),
        database: database.into_iter().collect(),
        tests,
    }
}

/// File-mode query: the blast radius of every symbol declared in `file`.
pub fn compute_file_impact(graph: &SymbolGraph, file: &str) -> ImpactReport {
    let seeds: HashSet<NodeId> = graph
        .nodes()
        .filter(|n| n.file == file)
        .map(|n| n.id.clone())
        .collect();
    compute_impact(graph, seeds)
}

/// Symbol-mode query: the blast radius of one resolved path, seeded via the same tiered
/// resolution as call-site linking (see `Resolver`). Returns `None` if `path` doesn't
/// resolve to anything in the graph — a `--change` target the indexed project has never
/// heard of, most likely a typo.
pub fn compute_symbol_impact(graph: &SymbolGraph, path: &str) -> Option<ImpactReport> {
    let resolver = Resolver::build(graph);
    let (ids, _confidence) = resolver.resolve(path)?;
    let seeds: HashSet<NodeId> = ids.into_iter().collect();
    Some(compute_impact(graph, seeds))
}

/// Change-mode query: resolves `spec`'s target path and computes its blast radius — see
/// `ChangeSpec::target_path` for how each change kind maps to a resolvable path.
pub fn compute_change_impact(graph: &SymbolGraph, spec: &ChangeSpec) -> Option<ImpactReport> {
    compute_symbol_impact(graph, &spec.target_path())
}
