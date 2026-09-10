use std::collections::HashMap;

use crate::adapter::{ContractRef, ContractRole, RefDecl, RefTarget};
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
///
/// These three tiers answer the question a *user* asks (`--change "remove
/// PaymentService::charge"`), where the name typed is the whole of the evidence. A *call
/// site* carries more than its name — which module an import bound it to, whether the
/// receiver's type was knowable — so references resolve through `resolve_ref` instead,
/// which uses that evidence first and only falls back to these tiers.
pub struct Resolver<'g> {
    by_qualified_path: HashMap<&'g str, NodeId>,
    by_last_two_segments: HashMap<String, Vec<NodeId>>,
    by_short_name: HashMap<&'g str, Vec<(&'g str, NodeId)>>,
    /// Nodes an adapter marked `is_generated` (see `SymbolDecl::is_generated`) — consulted
    /// only by `in_module`, to keep a generated mock sitting beside its real
    /// implementation from diluting the real one's confidence on a package-scoped call.
    /// The plain short-name/last-two-segment tiers above are deliberately left alone:
    /// they're already the weakest evidence this resolver produces, and a `--change`
    /// target or a same-file call precisely naming a generated symbol should still find
    /// it.
    generated: std::collections::HashSet<NodeId>,
}

impl<'g> Resolver<'g> {
    pub fn build(graph: &'g SymbolGraph) -> Self {
        let mut by_qualified_path = HashMap::new();
        let mut by_last_two_segments: HashMap<String, Vec<NodeId>> = HashMap::new();
        let mut by_short_name: HashMap<&str, Vec<(&str, NodeId)>> = HashMap::new();
        let mut generated = std::collections::HashSet::new();

        for node in graph.nodes() {
            if node.is_generated {
                generated.insert(node.id.clone());
            }
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
                .push((node.qualified_path.as_str(), node.id.clone()));
        }

        Self {
            by_qualified_path,
            by_last_two_segments,
            by_short_name,
            generated,
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
        self.by_short_name.get(name).map(|matches| {
            let confidence = if matches.len() == 1 {
                Confidence::Exact
            } else {
                Confidence::Heuristic
            };
            (
                matches.iter().map(|(_, id)| id.clone()).collect(),
                confidence,
            )
        })
    }

    /// Resolves one call site, using whatever the adapter could work out about it (see
    /// `RefTarget`) before falling back to the structural tiers above.
    ///
    /// The rule that matters: a bare name is never enough for `Exact`. Matching exactly
    /// one symbol project-wide says the *name* is unique, not that the call goes there —
    /// `words.len()` matches a lone `Counter::len` while actually calling `Vec::len` from
    /// the standard library, and `x.prune()` matches an unrelated module's `prune`. Only
    /// an import (or a same-file declaration) is real evidence of where a name points, so
    /// only `Module` can produce `Exact`.
    pub fn resolve_ref(&self, r: &RefDecl) -> Option<(Vec<NodeId>, Confidence)> {
        match &r.to_target {
            // An empty module is the project root as its own scope, which carries no more
            // information than the name itself — resolve it structurally.
            RefTarget::Module(module) if module.is_empty() => self.resolve(&r.to_name),
            RefTarget::Module(module) => {
                let ids = self.in_module(module, &r.to_name);
                if ids.is_empty() {
                    // The import resolved somewhere outside this project (a dependency, a
                    // standard-library module). Dropping the edge is right: inventing a
                    // same-named local match is exactly the false positive this fixes.
                    return None;
                }
                let confidence = if ids.len() == 1 {
                    Confidence::Exact
                } else {
                    Confidence::Probable
                };
                Some((ids, confidence))
            }
            RefTarget::Opaque => {
                let (ids, confidence) = self.resolve(&r.to_name)?;
                Some((ids, confidence.weaker(Confidence::Probable)))
            }
            RefTarget::Unscoped => self.resolve(&r.to_name),
        }
    }

    /// Every symbol named `name` that lives under `module`, where "under" means the
    /// module's `::`-separated segments appear as a contiguous run in the symbol's own
    /// qualified path.
    ///
    /// Segment containment rather than a plain prefix match is what lets one rule serve
    /// languages that scope by file and languages that scope by directory. A TypeScript
    /// import of `./utils` from `a/sync/actions.js` gives the module `a::sync::utils`,
    /// which prefixes `a::sync::utils::camelizeOrder` exactly; a Go import of
    /// `example.com/x/internal/svc` gives the package directory segment `svc`, which
    /// appears mid-path in `internal::svc::handler::Handle`. Both are the same question:
    /// is this symbol inside that module?
    fn in_module(&self, module: &str, name: &str) -> Vec<NodeId> {
        let Some(candidates) = self.by_short_name.get(name) else {
            return Vec::new();
        };
        let wanted: Vec<&str> = module.split("::").filter(|s| !s.is_empty()).collect();
        let matched: Vec<NodeId> = candidates
            .iter()
            .filter(|(path, _)| {
                let segments: Vec<&str> = path.split("::").collect();
                // The last segment is `name` itself; the module has to sit in front of it.
                segments
                    .len()
                    .checked_sub(1)
                    .is_some_and(|end| contains_run(&segments[..end], &wanted))
            })
            .map(|(_, id)| id.clone())
            .collect();

        // A generated mock living in the same package as the real implementation it
        // mocks (Go's own `mockgen` convention: `interface_mock.go` beside
        // `interface.go`) matches this same module+name lookup and, left in, downgrades
        // the real implementation's own confidence from Exact to Probable on every call
        // reached through its interface — see `SymbolDecl::is_generated`. Preferring the
        // non-generated subset (when there is one) fixes that without touching a query
        // whose only candidates happen to be generated.
        // A generated mock living in the same package as the real implementation it
        // mocks (Go's own `mockgen` convention: `interface_mock.go` beside
        // `interface.go`) matches this same module+name lookup and, left in, downgrades
        // the real implementation's own confidence from Exact to Probable on every call
        // reached through its interface — see `SymbolDecl::is_generated`. Preferring the
        // non-generated subset (when there is one) fixes that without touching a query
        // whose only candidates happen to be generated.
        let non_generated: Vec<NodeId> = matched
            .iter()
            .filter(|id| !self.generated.contains(id))
            .cloned()
            .collect();
        if non_generated.is_empty() {
            matched
        } else {
            non_generated
        }
    }
}

/// Whether `needle`'s segments appear consecutively, in order, anywhere in `haystack`.
fn contains_run(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
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
        let Some((to_ids, confidence)) = resolver.resolve_ref(r) else {
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
