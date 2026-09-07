use crate::service::PaymentService;

pub fn build() -> bool {
    let _ = PaymentService;
    PaymentService::charge_static()
}
