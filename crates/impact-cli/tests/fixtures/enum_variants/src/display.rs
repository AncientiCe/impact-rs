use crate::status::PaymentStatus;

pub fn describe(status: PaymentStatus) -> &'static str {
    match status {
        PaymentStatus::Pending => "pending",
        PaymentStatus::Failed => "failed",
    }
}
