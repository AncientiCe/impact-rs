// Five INDIRECT (2-hop) dependents of `core::hub` in one file — enough to exceed the
// summary mode's default per-file inline-show limit (3), so this file's group should
// report `count: 5` while only showing the first 3 entries.

pub fn caller_1() -> bool {
    crate::direct::direct_one()
}

pub fn caller_2() -> bool {
    crate::direct::direct_one()
}

pub fn caller_3() -> bool {
    crate::direct::direct_one()
}

pub fn caller_4() -> bool {
    crate::direct::direct_two()
}

pub fn caller_5() -> bool {
    crate::direct::direct_two()
}
