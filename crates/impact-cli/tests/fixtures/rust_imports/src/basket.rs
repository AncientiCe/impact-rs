pub struct Basket {
    items: Vec<u32>,
}

impl Basket {
    /// The only `len` declared anywhere in this fixture.
    pub fn len(&self) -> usize {
        self.items.len()
    }
}
