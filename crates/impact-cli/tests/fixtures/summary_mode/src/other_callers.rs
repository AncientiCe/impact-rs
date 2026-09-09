// A second file with INDIRECT dependents of `core::hub`, fewer than the summary mode's
// per-file inline-show limit — its group should show every entry, with no truncation.

pub fn caller_a() -> bool {
    crate::direct::direct_one()
}

pub fn caller_b() -> bool {
    crate::direct::direct_two()
}
