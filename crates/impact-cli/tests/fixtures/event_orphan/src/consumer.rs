use crate::events::OrderPlaced;

pub fn handle(event: OrderPlaced) -> bool {
    event.id > 0
}
