pub struct Cache {
    size: u32,
}

impl Cache {
    pub fn open(size: u32) -> Self {
        Cache { size }
    }

    pub fn size(&self) -> u32 {
        self.size
    }
}
