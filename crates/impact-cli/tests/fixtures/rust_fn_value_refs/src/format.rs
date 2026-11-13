pub fn render_row(id: u32) -> String {
    format!("row-{id}")
}

pub fn unused_helper(id: u32) -> u32 {
    id + 1
}

#[cfg(test)]
mod tests {
    use super::render_row;

    #[test]
    fn renders_a_row() {
        assert_eq!(render_row(1), "row-1");
    }
}
