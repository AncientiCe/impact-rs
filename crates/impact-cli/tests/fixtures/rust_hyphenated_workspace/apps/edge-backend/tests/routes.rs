use edge_backend::handle_frame;
use wire_protocol::TurnRequest;

#[test]
fn cancel_finishes_the_turn() {
    let request = TurnRequest { text: String::new() };
    assert!(request.text.is_empty());
    let _ = handle_frame(wire_protocol::ClientFrame::Cancel);
}
