use std::collections::HashMap;

use crate::adapter::{ContractRef, ContractRole, RefDecl};
use crate::graph::{Confidence, Edge, EdgeKind, NodeId, NodeKind, SymbolGraph};

/// Resolves a name (however precisely an adapter or a user could state it) against a
/// graph's `Function`, `Field` (used for enum variants — see `impact-lang-rust`), and
/// `Contract` nodes, in three tiers of decreasing precision:
///
/// 1. **Exact qualified path** — always unambiguous, `Confidence::Exact`.
/// 2. **Last two path segments** (`"PaymentStatus::Failed"`) — for references that name
///    an enclosing scope but not the full module path, like a match arm's pattern or a
///    `--change` argument typed by a user who doesn't know (or care about) the module.
///    Unambiguous unless two different types in the project share both a name and a
///    same-named member.
/// 3. **Short name only** (the last segment) — a bare call-site name (`validate()`,
///    `self.charge()`) with no scope information at all. The least precise tier: any
///    number of same-named functions across the project all match, `Confidence::Heuristic`
///    when there's more than one.
///
/// This structural resolution is deliberately not semantic (no type-checking), so it can
/// over-match — see the module doc on `link` for why that's the right tradeoff here.
pub struct Resolver<'g> {
    by_qualified_path: HashMap<&'g str, NodeId>,
    by_last_two_segments: HashMap<String, Vec<NodeId>>,
    by_short_name: HashMap<&'g str, Vec<NodeId>>,
}

impl<'g> Resolver<'g> {
    pub fn build(graph: &'g SymbolGraph) -> Self {
        let mut by_qualified_path = HashMap::new();
        let mut by_last_two_segments: HashMap<String, Vec<NodeId>> = HashMap::new();
        let mut by_short_name: HashMap<&str, Vec<NodeId>> = HashMap::new();

        for node in graph.nodes() {
            // `Module` is deliberately excluded: this project doesn't emit any today
            // (see `impact-lang-rust`'s module-prefix comment), and admitting it would
            // let a bare crate-root reference resolve to noise. Every other kind is a
            // legitimate `--change` target — including `Type`, so "remove PaymentService"
            // and `RemoveField`'s type-path fallback (see `ChangeSpec`) have something to
            // resolve against, not just functions.
            if matches!(node.kind, NodeKind::Module) {
                continue;
            }
            by_qualified_path.insert(node.qualified_path.as_str(), node.id.clone());

            let segments: Vec<&str> = node.qualified_path.split("::").collect();
            if segments.len() >= 2 {
                let last_two = segments[segments.len() - 2..].join("::");
                by_last_two_segments
                    .entry(last_two)
                    .or_default()
                    .push(node.id.clone());
            }

            let short_name = segments.last().copied().unwrap_or(&node.qualified_path);
            by_short_name
                .entry(short_name)
                .or_default()
                .push(node.id.clone());
        }

        Self {
            by_qualified_path,
            by_last_two_segments,
            by_short_name,
        }
    }

    pub fn resolve(&self, name: &str) -> Option<(Vec<NodeId>, Confidence)> {
        if let Some(id) = self.by_qualified_path.get(name) {
            return Some((vec![id.clone()], Confidence::Exact));
        }
        if let Some(ids) = self.by_last_two_segments.get(name) {
            let confidence = if ids.len() == 1 {
                Confidence::Exact
            } else {
                Confidence::Heuristic
            };
            return Some((ids.clone(), confidence));
        }
        self.by_short_name.get(name).map(|ids| {
            let confidence = if ids.len() == 1 {
                Confidence::Exact
            } else {
                Confidence::Heuristic
            };
            (ids.clone(), confidence)
        })
    }
}

/// Resolves adapter-emitted `RefDecl`s and `ContractRef`s into graph `Edge`s. This is
/// structural (name + qualified-path matching), not semantic — it doesn't know about
/// types, traits, or scope, so a name that could plausibly resolve to several candidates
/// resolves to *all* of them, tagged `Confidence::Heuristic`. A blast-radius tool should
/// over-report rather than silently miss a caller: false positives are visible and
/// filterable, false negatives are not.
pub fn link(graph: &SymbolGraph, refs: &[RefDecl], contract_refs: &[ContractRef]) -> Vec<Edge> {
    let resolver = Resolver::build(graph);
    let mut edges = Vec::new();

    for r in refs {
        let Some((from_ids, _)) = resolver.resolve(&r.from_qualified_path) else {
            continue;
        };
        let Some((to_ids, confidence)) = resolver.resolve(&r.to_name) else {
            continue;
        };
        for from_id in &from_ids {
            for to_id in &to_ids {
                edges.push(Edge {
                    from: from_id.clone(),
                    to: to_id.clone(),
                    kind: r.kind,
                    confidence,
                });
            }
        }
    }

    for cr in contract_refs {
        let Some((symbol_ids, confidence)) = resolver.resolve(&cr.symbol_name) else {
            continue;
        };
        // Contract identity is always an exact match — never the fuzzier tiers, since a
        // bare contract id (a table name, an event type name) could otherwise
        // coincidentally collide with an unrelated function's short name.
        let Some(contract_id) = resolver.by_qualified_path.get(cr.contract_id.as_str()) else {
            continue;
        };
        let kind = match cr.role {
            ContractRole::Produces => EdgeKind::Produces,
            ContractRole::Consumes => EdgeKind::Consumes,
            ContractRole::Reads => EdgeKind::Reads,
            ContractRole::Writes => EdgeKind::Writes,
        };
        for symbol_id in &symbol_ids {
            edges.push(Edge {
                from: symbol_id.clone(),
                to: contract_id.clone(),
                kind,
                confidence,
            });
        }
    }

    edges
}
