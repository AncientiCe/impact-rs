use crate::format;

pub struct Json<T>(pub T);

pub async fn get_rows(ids: Vec<u32>) -> Json<Vec<String>> {
    Json(ids.into_iter().map(format::render_row).collect())
}

pub fn via_super(ids: &[u32]) -> Vec<String> {
    ids.iter().copied().map(super::super::format::render_row).collect()
}

pub fn in_macro(ids: &[u32]) -> String {
    format!("{:?}", ids.iter().copied().map(format::render_row).collect::<Vec<_>>())
}

pub fn filter_mapped(ids: &[u32]) -> Vec<String> {
    ids.iter().copied().filter_map(|id| Some(id)).map(format::render_row).collect()
}

pub struct Service;

impl Service {
    pub fn rows(&self, ids: &[u32]) -> Vec<String> {
        ids.iter().copied().map(format::render_row).collect()
    }
}
