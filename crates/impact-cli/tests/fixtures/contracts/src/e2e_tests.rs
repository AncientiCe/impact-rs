use crate::handlers::PaymentHandler;

#[test]
fn creates_payment_route_end_to_end() {
    let handler = PaymentHandler;
    assert!(handler.create_payment_route());
}
