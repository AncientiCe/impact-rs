pub struct PaymentService;

impl PaymentService {
    pub fn charge(&self) -> bool {
        validate()
    }
}

fn validate() -> bool {
    true
}

pub enum PaymentStatus {
    Pending,
    Failed,
}

pub trait Chargeable {
    fn charge(&self) -> bool;
}
