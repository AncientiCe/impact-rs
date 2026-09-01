use crate::events::PaymentCreated;

pub struct PaymentHandler;

impl PaymentHandler {
    pub fn create_payment_route(&self) -> bool {
        let event = PaymentCreated { amount: 100 };
        crate::repo::save_payment(event.amount)
    }

    pub fn on_payment_created(&self, event: PaymentCreated) -> bool {
        event.amount > 0
    }
}
