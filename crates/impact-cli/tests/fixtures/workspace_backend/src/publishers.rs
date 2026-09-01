use crate::events::{OrderPlaced, PaymentCreated, UnrelatedEvent};

pub fn publish_payment() -> bool {
    let _ = PaymentCreated { amount: 10 };
    true
}

pub fn publish_order() -> bool {
    let _ = OrderPlaced { id: 1 };
    true
}

pub fn publish_unrelated() -> bool {
    let _ = UnrelatedEvent {
        note: "x".to_string(),
    };
    true
}
