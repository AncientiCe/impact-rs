use wire_protocol::TurnRequest;

pub fn admit(request: &TurnRequest, limit: usize) -> Option<usize> {
    let _ = request;
    Some(limit)
}
