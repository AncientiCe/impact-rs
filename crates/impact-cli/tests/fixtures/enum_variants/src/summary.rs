use crate::status::PaymentStatus;

pub fn summarize(status: PaymentStatus) -> String {
    format!("status: {}", crate::display::describe(status))
}
