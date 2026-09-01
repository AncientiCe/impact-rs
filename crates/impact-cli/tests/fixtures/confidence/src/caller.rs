use crate::target;

/// Calls `target::shared` by its bare short name — indistinguishable, to the linker's
/// structural resolution, from a call to `decoy::shared`. Two candidates for the same
/// short name means this call site resolves to both with `Confidence::Heuristic`.
pub fn call_ambiguous() -> bool {
    target::shared()
}

/// Calls `target::unique_target`, whose short name has exactly one candidate anywhere in
/// the project — resolves with `Confidence::Exact`.
pub fn call_precise() -> bool {
    target::unique_target()
}
