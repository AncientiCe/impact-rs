use crate::target;

/// Calls `target::shared` by its bare short name — indistinguishable, to the linker's
/// structural resolution, from a call to `decoy::shared`. Resolves to both with
/// `Confidence::Heuristic`, making this the fixture's one weak (ambiguous) hop.
pub fn call_ambiguous() -> bool {
    target::shared()
}
