use crate::events::PaymentCreatedEvent;

pub fn publish() -> bool {
    let e = PaymentCreatedEvent { amount: 5 };
    on_created(e)
}

pub fn on_created(event: PaymentCreatedEvent) -> bool {
    event.amount > 0
}
