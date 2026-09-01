pub trait Event {}

pub struct PaymentCreated {
    pub amount: u64,
}
impl Event for PaymentCreated {}

pub struct OrderPlaced {
    pub id: u64,
}
impl Event for OrderPlaced {}
