use crate::caller;

/// Calls `caller::call_ambiguous` — whose short name has exactly one candidate anywhere
/// in the project, so *this* edge on its own would resolve `Confidence::Exact`. It only
/// exists two hops away from `target::shared`, reachable solely by continuing the BFS
/// through `call_ambiguous`'s own `Confidence::Heuristic` incoming edge — the fixture's
/// probe for whether the engine keeps chaining through an already-unreliable hop.
pub fn call_two_hops_away() -> bool {
    caller::call_ambiguous()
}
