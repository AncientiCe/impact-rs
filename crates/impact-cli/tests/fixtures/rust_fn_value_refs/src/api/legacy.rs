use super::super::format::render_row;

pub fn legacy_rows(ids: &[u32]) -> Vec<String> {
    ids.iter().copied().map(render_row).collect()
}

pub fn bare_in_macro(ids: &[u32]) -> String {
    format!("{:?}", ids.iter().copied().map(render_row).collect::<Vec<_>>())
}
