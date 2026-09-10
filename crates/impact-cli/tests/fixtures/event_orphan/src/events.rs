pub trait Event {}

pub struct OrderPlaced {
    pub id: u64,
}

impl Event for OrderPlaced {}
