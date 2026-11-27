pub struct Store {
    capacity: u32,
}

impl Store {
    pub fn open(capacity: u32) -> Self {
        Store { capacity }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}
