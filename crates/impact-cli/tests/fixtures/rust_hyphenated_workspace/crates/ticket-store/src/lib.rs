pub struct TicketStore {
    open: Vec<u32>,
}

impl TicketStore {
    pub fn list_open(&self) -> Vec<u32> {
        self.open.clone()
    }
}

/// `queue` is a parameter, so its type is unknown here. The method name happens to be
/// unique project-wide, but this crate depends on nothing in the project, so the call
/// can never reach pod-gateway's `relay_frames`.
pub fn drain(queue: &mut Vec<u8>) {
    queue.relay_frames();
}
