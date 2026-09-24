pub mod admission;

use wire_protocol::{
    ClientFrame, ServerEvent,
};

pub fn handle_frame(frame: ClientFrame) -> ServerEvent {
    match frame {
        ClientFrame::Start { id } => ServerEvent::Token(id.to_string()),
        ClientFrame::Cancel => ServerEvent::Done,
    }
}
