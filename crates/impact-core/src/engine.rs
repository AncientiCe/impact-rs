use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::change::ChangeSpec;
use crate::diff::DiffTouches;
use crate::graph::{Confidence, ContractKind, EdgeKind, Node, NodeId, NodeKind, SymbolGraph};
use crate::linker::Resolver;

/// One reverse-dependent found while walking the blast radius, tagged with how
/// confidently the linker resolved the call/reference chain connecting it to what was
/// queried. `Exact` means every hop along the way resolved unambiguously; `Heuristic`
/// means at least one hop matched a bare short name against more than one candidate. A
/// multi-hop chain's confidence is its weakest link, not its first or last hop — one
/// heuristic hop makes the whole chain only as trustworthy as that hop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dependent {
    pub path: String,
    /// File the dependent is declared in, relative to the project root — `""` if the
    /// graph has no location for it (shouldn't happen for a real symbol node).
    pub file: String,
    /// 1-indexed declaration line, matching `file` — `0` alongside an empty `file`.
    pub line: usize,
    pub confidence: Confidence,
    /// The intermediate dependents between the seed and this one, ordered nearest-seed
    /// first — the shortest BFS chain that connects them, not necessarily the only one.
    /// Always empty for a DIRECT entry (one hop from the seed, nothing intermediate to
    /// show) and, by default, for an INDIRECT entry too: only populated when `explain`
    /// is requested (see `apply_explain`), to keep a default report as compact as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
}

/// The blast radius of a change: who's affected directly, who's affected transitively
/// through them, which API routes / event types / database tables the affected symbols
/// touch, and which tests exercise any of it. Sorted, so the same graph always produces
/// the same report — determinism is the whole point of this tool.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ImpactReport {
    pub direct: Vec<Dependent>,
    pub indirect: Vec<Dependent>,
    pub api: Vec<String>,
    pub events: Vec<String>,
    pub database: Vec<String>,
    /// How many of the dependents found (`direct` + `indirect`) are test functions —
    /// redundant with `affected_tests.len()`, kept as its own field since a plain count
    /// is what most tree-text/agent consumers actually want to read at a glance.
    pub tests: usize,
    /// The test dependents themselves (a subset of `direct`/`indirect`, already carrying
    /// their own file/line), so an agent can run exactly these instead of the whole
    /// suite. Sorted the same way `direct`/`indirect` are.
    pub affected_tests: Vec<Dependent>,
}

/// Drops `direct`/`indirect`/`affected_tests` entries below `min` confidence (and
/// recomputes `tests` to match `affected_tests.len()` afterward), leaving `api`/`events`/
/// `database` untouched — those don't carry a confidence tier of their own.
pub fn filter_min_confidence(mut report: ImpactReport, min: Confidence) -> ImpactReport {
    report.direct.retain(|d| d.confidence.at_least(min));
    report.indirect.retain(|d| d.confidence.at_least(min));
    report.affected_tests.retain(|d| d.confidence.at_least(min));
    report.tests = report.affected_tests.len();
    report
}

/// `compute_impact` always computes each INDIRECT entry's `via` chain (cheap — just
/// following BFS parent pointers already built while walking the graph), but a default
/// report should stay as compact as before `via` existed. Call this with `explain: false`
/// (the default everywhere it's wired up) to clear it back out; `explain: true` leaves it
/// populated.
pub fn apply_explain(mut report: ImpactReport, explain: bool) -> ImpactReport {
    if !explain {
        for d in report
            .direct
            .iter_mut()
            .chain(report.indirect.iter_mut())
            .chain(report.affected_tests.iter_mut())
        {
            d.via.clear();
        }
    }
    report
}

/// Computes the full blast radius of `seeds`: DIRECT (1-hop reverse dependents) and
/// INDIRECT (2+-hop reverse dependents), then the API/EVENTS/DATABASE contracts reachable
/// from the seeds or any of their dependents, and finally which of the dependents found
/// are test functions (`affected_tests`, plus its own count as `tests`).
///
/// "Reverse dependent" walks backward over `Calls`/`References` edges (who calls this)
/// *and* `Produces`/`Consumes`/`Reads`/`Writes` edges (who touches this contract) as one
/// unified graph. That second half matters when a seed is (or belongs to) a file that
/// declares an event type or is the only place a table name is written, rather than the
/// one calling into it — without it, querying an event's own definition file would report
/// nothing, even though every producer and consumer of that event is exactly its blast
/// radius.
fn compute_impact(graph: &SymbolGraph, seeds: HashSet<NodeId>) -> ImpactReport {
    let mut callers: HashMap<NodeId, Vec<(NodeId, Confidence)>> = HashMap::new();
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
                .push((edge.from.clone(), edge.confidence));
        }
    }

    let mut direct: BTreeMap<String, (Confidence, NodeId)> = BTreeMap::new();
    let mut indirect: BTreeMap<String, (Confidence, NodeId)> = BTreeMap::new();
    let mut visited: HashSet<NodeId> = seeds.clone();
    // A seed is definitionally certain — the first hop off of it inherits the edge's own
    // confidence unmodified, which `weaker` below achieves by starting at `Exact`.
    let mut node_confidence: HashMap<NodeId, Confidence> = HashMap::new();
    // BFS parent pointers, for reconstructing an INDIRECT entry's `via` chain back to
    // (but not including) the nearest seed — see `via_chain`.
    let mut parent: HashMap<NodeId, NodeId> = HashMap::new();

    let mut frontier: Vec<NodeId> = seeds.iter().cloned().collect();
    let mut hop = 0;
    while !frontier.is_empty() {
        hop += 1;
        let mut next = Vec::new();
        for node_id in &frontier {
            let Some(node_callers) = callers.get(node_id) else {
                continue;
            };
            let incoming = node_confidence
                .get(node_id)
                .copied()
                .unwrap_or(Confidence::Exact);
            for (caller, edge_confidence) in node_callers {
                if !visited.insert(caller.clone()) {
                    continue;
                }
                let confidence = incoming.weaker(*edge_confidence);
                node_confidence.insert(caller.clone(), confidence);
                parent.insert(caller.clone(), node_id.clone());
                let name = graph
                    .node(caller)
                    .map(|n| n.qualified_path.clone())
                    .unwrap_or_else(|| caller.to_string());
                let bucket = if hop == 1 { &mut direct } else { &mut indirect };
                bucket
                    .entry(name)
                    .and_modify(|(c, _)| *c = (*c).weaker(confidence))
                    .or_insert((confidence, caller.clone()));
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

    let to_dependent = |path: String, confidence: Confidence, id: &NodeId| {
        let (file, line) = graph
            .node(id)
            .map(|n| (n.file.clone(), n.line))
            .unwrap_or_default();
        Dependent {
            path,
            file,
            line,
            confidence,
            via: via_chain(graph, &parent, &seeds, id),
        }
    };

    let mut affected_tests: BTreeMap<String, Dependent> = BTreeMap::new();
    for (path, (confidence, id)) in direct.iter().chain(indirect.iter()) {
        if graph.node(id).is_some_and(|n| n.is_test) {
            affected_tests.insert(path.clone(), to_dependent(path.clone(), *confidence, id));
        }
    }
    let tests = affected_tests.len();

    ImpactReport {
        direct: direct
            .into_iter()
            .map(|(path, (confidence, id))| to_dependent(path, confidence, &id))
            .collect(),
        indirect: indirect
            .into_iter()
            .map(|(path, (confidence, id))| to_dependent(path, confidence, &id))
            .collect(),
        api: api.into_iter().collect(),
        events: events.into_iter().collect(),
        database: database.into_iter().collect(),
        tests,
        affected_tests: affected_tests.into_values().collect(),
    }
}

/// Walks `parent` back from `id` toward the seed that reached it, collecting each
/// intermediate node's qualified path (nearest-seed first), stopping once the walk
/// reaches a seed without including the seed itself. For a DIRECT entry (`parent[id]` is
/// already a seed) this returns empty, matching `Dependent::via`'s documented behavior.
fn via_chain(
    graph: &SymbolGraph,
    parent: &HashMap<NodeId, NodeId>,
    seeds: &HashSet<NodeId>,
    id: &NodeId,
) -> Vec<String> {
    let mut chain = Vec::new();
    let mut current = id.clone();
    while let Some(p) = parent.get(&current) {
        if seeds.contains(p) {
            break;
        }
        chain.push(p.clone());
        current = p.clone();
    }
    chain.reverse();
    chain
        .into_iter()
        .filter_map(|id| graph.node(&id).map(|n| n.qualified_path.clone()))
        .collect()
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

/// Diff-mode query: the blast radius of every symbol a diff's touched lines fall inside,
/// across every file the diff mentions. Each symbol's own `[line, end_line]` span (see
/// `SymbolDecl::end_line`) is matched against the diff's touched ranges by overlap —
/// still a structural approximation, not precise AST containment (a symbol's span is its
/// outermost declaration node, e.g. a whole `impl` block or `fn`, not per-statement), but
/// no longer the coarser "nearest preceding declaration" guess: a touched range entirely
/// inside a function's body is now matched directly by its span, not inferred from
/// declaration order.
pub fn compute_diff_impact(graph: &SymbolGraph, touches: &DiffTouches) -> ImpactReport {
    let mut seeds: HashSet<NodeId> = HashSet::new();

    for (file, ranges) in &touches.files {
        let candidates: Vec<&Node> = graph
            .nodes()
            .filter(|n| &n.file == file && !matches!(n.kind, NodeKind::Module))
            .collect();

        for range in ranges {
            for candidate in &candidates {
                let span_end = candidate.end_line.max(candidate.line);
                let overlaps = candidate.line < range.end && range.start <= span_end;
                if overlaps {
                    seeds.insert(candidate.id.clone());
                }
            }
        }
    }

    compute_impact(graph, seeds)
}
