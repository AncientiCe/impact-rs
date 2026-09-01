use crate::events::{OrderPlaced, PaymentCreated};

pub fn on_payment_created(event: PaymentCreated) -> bool {
    event.amount > 0
}

pub fn on_order_placed(event: OrderPlaced) -> bool {
    event.id > 0
}
