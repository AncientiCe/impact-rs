use std::collections::{HashMap, HashSet};

use crate::adapter::{ContractRef, ContractRole, PackageDecl, RefDecl, RefTarget};
use crate::graph::{Confidence, Edge, EdgeKind, Node, NodeId, NodeKind, SymbolGraph};

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
    /// `(qualified_path, id)` for every node an adapter marked `is_default_export` (see
    /// `SymbolDecl::is_default_export`) — consulted only by `in_module_default`. Kept as
    /// a plain list rather than keyed by name: there's normally exactly one per module,
    /// so a name-keyed map would buy nothing a linear segment-containment scan doesn't
    /// already give `in_module` itself.
    default_exports: Vec<(&'g str, NodeId)>,
}

impl<'g> Resolver<'g> {
    pub fn build(graph: &'g SymbolGraph) -> Self {
        Self::build_where(graph, |_| true)
    }

    /// A resolver over only the nodes `keep` admits — see `link`, which uses this to keep
    /// a call site from resolving into a different language than the one it was written in.
    fn build_where(graph: &'g SymbolGraph, keep: impl Fn(&Node) -> bool) -> Self {
        let mut by_qualified_path = HashMap::new();
        let mut by_last_two_segments: HashMap<String, Vec<NodeId>> = HashMap::new();
        let mut by_short_name: HashMap<&str, Vec<(&str, NodeId)>> = HashMap::new();
        let mut generated = std::collections::HashSet::new();
        let mut default_exports = Vec::new();

        for node in graph.nodes().filter(|n| keep(n)) {
            if node.is_generated {
                generated.insert(node.id.clone());
            }
            if node.is_default_export {
                default_exports.push((node.qualified_path.as_str(), node.id.clone()));
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
            default_exports,
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
            // A default import's local name carries no information about what the target
            // is actually called there (unlike `Module`, where the name is an import or
            // same-file declaration the file itself wrote) — so this ignores `r.to_name`
            // entirely and matches on `is_default_export` instead.
            RefTarget::ModuleDefault(module) => {
                let ids = self.in_module_default(module);
                if ids.is_empty() {
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

    /// Every node marked `is_default_export` that lives under `module` — the
    /// `ModuleDefault` counterpart to `in_module`, using the same segment-containment
    /// test but never matching on name, since a default import's local alias isn't
    /// evidence of the target's own name the way a named import's is.
    fn in_module_default(&self, module: &str) -> Vec<NodeId> {
        let wanted: Vec<&str> = module.split("::").filter(|s| !s.is_empty()).collect();
        self.default_exports
            .iter()
            .filter(|(path, _)| {
                let segments: Vec<&str> = path.split("::").collect();
                segments
                    .len()
                    .checked_sub(1)
                    .is_some_and(|end| contains_run(&segments[..end], &wanted))
            })
            .map(|(_, id)| id.clone())
            .collect()
    }
}

/// Whether `needle`'s segments appear consecutively, in order, anywhere in `haystack`.
fn contains_run(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Which package (see `PackageDecl`) each file belongs to, and what each package may
/// reach — the part of resolution no single file's imports can answer.
struct Packages<'p> {
    packages: Vec<(&'p str, &'p PackageDecl)>,
    /// For each package, the indices of the packages its shipped code can reach: itself
    /// and everything it depends on, directly or not. Transitive, because a value of a
    /// type from a dependency's dependency can still have its methods called.
    visible: Vec<HashSet<usize>>,
    /// The same, plus its dev-only dependencies (and theirs) — what its test code reaches.
    visible_to_tests: Vec<HashSet<usize>>,
}

impl<'p> Packages<'p> {
    fn new(packages: &'p [(String, PackageDecl)]) -> Self {
        let packages: Vec<(&str, &PackageDecl)> =
            packages.iter().map(|(l, p)| (l.as_str(), p)).collect();
        let find = |language: &str, name: &str| {
            packages
                .iter()
                .position(|(l, p)| *l == language && p.name == name)
        };
        let direct: Vec<Vec<(usize, bool)>> = packages
            .iter()
            .map(|(language, package)| {
                package
                    .dependencies
                    .iter()
                    .filter_map(|d| find(language, &d.package).map(|i| (i, d.dev_only)))
                    .collect()
            })
            .collect();
        // Everything reachable from `start` through non-dev dependencies — a dependency's
        // own dev-dependencies are never part of what depends on it.
        let closure = |start: &[usize]| {
            let mut seen: HashSet<usize> = HashSet::new();
            let mut stack = start.to_vec();
            while let Some(i) = stack.pop() {
                if seen.insert(i) {
                    stack.extend(direct[i].iter().filter(|(_, dev)| !dev).map(|(d, _)| *d));
                }
            }
            seen
        };
        let visible: Vec<HashSet<usize>> = (0..packages.len()).map(|i| closure(&[i])).collect();
        let visible_to_tests = (0..packages.len())
            .map(|i| {
                let mut start = vec![i];
                start.extend(direct[i].iter().filter(|(_, dev)| *dev).map(|(d, _)| *d));
                closure(&start)
            })
            .collect();
        Self {
            packages,
            visible,
            visible_to_tests,
        }
    }

    /// The package `file` belongs to: the one with the deepest root containing it, so a
    /// crate nested inside another crate's directory owns its own files.
    fn owner(&self, language: &str, file: &str) -> Option<usize> {
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, (l, p))| *l == language && is_under(file, &p.root))
            .max_by_key(|(_, (_, p))| p.root.len())
            .map(|(i, _)| i)
    }

    /// `r` with an import of a package rewritten to where that package's symbols are
    /// indexed, when `r` names one this package can see by its import name — or `r`
    /// unchanged. `use wire_protocol::codec::X` gives `Module("wire_protocol::codec")`,
    /// which says nothing about the `crates::wire-protocol::src::codec` its symbols
    /// actually live under; the manifest does.
    fn rewrite(&self, package: usize, r: &RefDecl) -> Option<RefDecl> {
        let module = match &r.to_target {
            RefTarget::Module(m) | RefTarget::ModuleDefault(m) => m,
            RefTarget::Opaque | RefTarget::Unscoped => return None,
        };
        let (first, rest) = match module.split_once("::") {
            Some((first, rest)) => (first, Some(rest)),
            None => (module.as_str(), None),
        };
        let (language, own) = self.packages[package];
        let target = if own.import_name == first {
            own
        } else {
            let dependency = own.dependencies.iter().find(|d| d.import_name == first)?;
            self.packages
                .iter()
                .find(|(l, p)| *l == language && p.name == dependency.package)
                .map(|(_, p)| *p)?
        };
        let rewritten = match rest {
            Some(rest) if !target.module_root.is_empty() => {
                format!("{}::{rest}", target.module_root)
            }
            Some(rest) => rest.to_string(),
            None => target.module_root.clone(),
        };
        let to_target = match r.to_target {
            RefTarget::ModuleDefault(_) => RefTarget::ModuleDefault(rewritten),
            _ => RefTarget::Module(rewritten),
        };
        Some(RefDecl {
            to_target,
            ..r.clone()
        })
    }
}

/// Whether `file` sits under the directory `root` (`""` being the project root).
fn is_under(file: &str, root: &str) -> bool {
    root.is_empty()
        || file
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Resolves adapter-emitted `RefDecl`s and `ContractRef`s into graph `Edge`s. This is
/// structural (name + qualified-path matching), not semantic — it doesn't know about
/// types, traits, or scope, so a name that could plausibly resolve to several candidates
/// resolves to *all* of them, tagged `Confidence::Heuristic`. A blast-radius tool should
/// over-report rather than silently miss a caller: false positives are visible and
/// filterable, false negatives are not.
///
/// A call site only ever resolves into its own language: no adapter parses a
/// cross-language call, so a bare `Leave()` in a JavaScript file that matches a Rust enum
/// variant by name is always a coincidence, never a caller. Each language gets its own
/// resolver (built lazily, on the first call site from that language), which also keeps
/// the other languages' same-named symbols from diluting a match's confidence. Contract
/// nodes stay visible to every language, since a contract is shared by definition.
///
/// `packages` (see `PackageDecl`) narrows that further for a call site inside a known
/// package: it resolves only into its own package and the packages it depends on, and an
/// import of one of those by its import name is rewritten to where its symbols live. A
/// symbol outside every package stays visible to everything, as before.
pub fn link(
    graph: &SymbolGraph,
    refs: &[RefDecl],
    contract_refs: &[ContractRef],
    packages: &[(String, PackageDecl)],
) -> Vec<Edge> {
    let resolver = Resolver::build(graph);
    let packages = Packages::new(packages);
    let owners: HashMap<&NodeId, (&str, Option<usize>)> = graph
        .nodes()
        .map(|n| {
            let language = n.language.as_str();
            (&n.id, (language, packages.owner(language, &n.file)))
        })
        .collect();
    let mut by_scope: HashMap<(&str, Option<usize>, bool), Resolver> = HashMap::new();
    let mut edges = Vec::new();

    for r in refs {
        let Some((from_ids, _)) = resolver.resolve(&r.from_qualified_path) else {
            continue;
        };
        for from_id in &from_ids {
            let (language, package) = owners.get(from_id).copied().unwrap_or_default();
            let rewritten = package.and_then(|p| packages.rewrite(p, r));
            let resolved_ref = rewritten.as_ref().unwrap_or(r);
            // Test code may also reach dev-only dependencies: a test function, a file
            // outside the package's shipped source (an integration test, a bench), or a
            // reference an import stands behind — which covers a test-only helper module
            // inside the shipped source too, since it has to import what it uses.
            let as_test = package.is_some_and(|p| {
                graph.node(from_id).is_some_and(|n| {
                    n.is_test || !is_under(&n.file, &packages.packages[p].1.source_root)
                }) || matches!(
                    resolved_ref.to_target,
                    RefTarget::Module(_) | RefTarget::ModuleDefault(_)
                )
            });
            let targets = by_scope
                .entry((language, package, as_test))
                .or_insert_with(|| {
                    let visible = package.map(|p| {
                        if as_test {
                            &packages.visible_to_tests[p]
                        } else {
                            &packages.visible[p]
                        }
                    });
                    Resolver::build_where(graph, |n| {
                        if matches!(n.kind, NodeKind::Contract(_)) {
                            return true;
                        }
                        if n.language != language {
                            return false;
                        }
                        match (visible, owners.get(&n.id).and_then(|(_, owner)| *owner)) {
                            (Some(visible), Some(owner)) => visible.contains(&owner),
                            _ => true,
                        }
                    })
                });
            let Some((to_ids, confidence)) = targets.resolve_ref(resolved_ref) else {
                continue;
            };
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
