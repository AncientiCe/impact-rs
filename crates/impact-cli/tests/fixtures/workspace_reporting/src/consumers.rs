use crate::events::UnrelatedEvent;

pub fn on_unrelated(event: UnrelatedEvent) -> bool {
    !event.note.is_empty()
}
