/// Same short name as `target::shared` — the ambiguity this fixture exists to create.
/// Nothing calls this one; it only needs to exist so the linker's short-name tier finds
/// two candidates for `shared` instead of one.
pub fn shared() -> bool {
    false
}
