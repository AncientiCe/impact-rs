pub mod codec;

pub enum ClientFrame {
    Start { id: u32 },
    Cancel,
}

pub enum ServerEvent {
    Done,
    Token(String),
}

pub struct TurnRequest {
    pub text: String,
}
