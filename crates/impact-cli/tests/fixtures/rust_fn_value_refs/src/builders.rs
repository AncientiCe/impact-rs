use crate::format;

pub fn build_one(id: u32) -> String {
    format::render_row(id)
}

/// Not a caller: `render_row` here is a parameter, and this file never imports the
/// function of that name.
pub fn shadowed(render_row: u32) -> u32 {
    std::convert::identity(render_row)
}
