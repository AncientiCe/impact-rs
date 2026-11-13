use crate::session::Session;

impl Session {
    pub fn authenticate(&self) -> bool {
        self.token.len() > 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_long_tokens() {
        assert!(Session::new("abcd").authenticate());
    }
}
