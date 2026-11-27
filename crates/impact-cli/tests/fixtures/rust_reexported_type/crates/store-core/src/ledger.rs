pub struct Ledger {
    entries: Vec<u32>,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger { entries: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
