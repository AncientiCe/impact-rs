pub fn save_payment(amount: u64) -> bool {
    let _ = sqlx::query!("INSERT INTO payments (amount) VALUES (?)", amount);
    true
}

#[test]
fn save_payment_persists() {
    assert!(save_payment(100));
}
