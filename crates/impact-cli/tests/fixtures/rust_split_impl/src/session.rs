pub struct Session {
    pub(crate) token: String,
}

impl Session {
    pub fn new(token: &str) -> Self {
        Session {
            token: token.to_string(),
        }
    }

    // `authenticate` lives in another module's `impl Session` block.
    pub fn open(&self) -> bool {
        self.authenticate()
    }

    pub fn close(&self) -> bool {
        self.flush()
    }

    fn flush(&self) -> bool {
        !self.token.is_empty()
    }
}
