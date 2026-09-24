use store::TicketStore;
use wire_protocol::codec::encode_frame;
use wire_protocol::ServerEvent;

pub struct Bridge {
    tickets: TicketStore,
}

impl Bridge {
    pub fn relay_frames(&self) -> usize {
        let open = self.tickets.list_open();
        encode_frame(&[]).len() + open.len()
    }

    pub fn finish(&self) -> ServerEvent {
        ServerEvent::Done
    }
}
