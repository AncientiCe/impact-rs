pub enum ClientMessage {
    Join { room: String },
    Ping(u32),
    Leave,
}
