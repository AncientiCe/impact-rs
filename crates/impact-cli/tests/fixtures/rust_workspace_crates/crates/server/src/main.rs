use proto::ClientMessage;

fn route(msg: ClientMessage) -> u32 {
    match msg {
        ClientMessage::Join { room } => room.len() as u32,
        _ => 0,
    }
}

fn is_leave(msg: &ClientMessage) -> bool {
    if let ClientMessage::Leave = msg {
        return true;
    }
    false
}

fn join(room: String) -> ClientMessage {
    ClientMessage::Join { room }
}

fn leave() -> ClientMessage {
    ClientMessage::Leave
}

fn ping() -> ClientMessage {
    ClientMessage::Ping(1)
}

fn main() {
    let _ = route(join(String::new()));
    let _ = is_leave(&leave());
    let _ = ping();
}
