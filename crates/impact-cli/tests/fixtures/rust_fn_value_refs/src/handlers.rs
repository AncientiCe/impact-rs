use crate::format;
use crate::format::render_row;

pub fn list_rows(ids: &[u32]) -> Vec<String> {
    ids.iter().copied().map(format::render_row).collect()
}

pub fn list_rows_imported(ids: &[u32]) -> Vec<String> {
    ids.iter().copied().map(render_row).collect()
}

pub fn bound_then_used(ids: &[u32]) -> Vec<String> {
    let render = render_row;
    ids.iter().copied().map(render).collect()
}

pub fn print_rows(ids: &[u32]) {
    ids.iter().copied().map(crate::format::render_row).for_each(drop);
}
