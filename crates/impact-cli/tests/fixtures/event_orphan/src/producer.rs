use crate::events::OrderPlaced;

pub fn dispatch(id: u64) -> OrderPlaced {
    OrderPlaced { id }
}
