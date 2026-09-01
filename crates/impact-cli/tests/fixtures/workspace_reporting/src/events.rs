pub trait Event {}

pub struct UnrelatedEvent {
    pub note: String,
}
impl Event for UnrelatedEvent {}
