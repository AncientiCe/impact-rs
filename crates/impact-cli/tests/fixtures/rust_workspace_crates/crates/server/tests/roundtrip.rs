use proto::ClientMessage;

#[test]
fn leave_is_constructible() {
    let msg = ClientMessage::Leave;
    assert!(matches!(msg, ClientMessage::Leave));
}
