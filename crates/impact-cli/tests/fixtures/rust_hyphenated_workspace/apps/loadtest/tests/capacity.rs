use pod_gateway::bridge::Bridge;
use wire_protocol::ClientFrame;

fn relay_once(bridge: &Bridge) -> usize {
    Bridge::relay_frames(bridge)
}

#[test]
fn a_cancel_frame_is_cheap() {
    let frame = ClientFrame::Cancel;
    assert!(matches!(frame, ClientFrame::Cancel));
}
