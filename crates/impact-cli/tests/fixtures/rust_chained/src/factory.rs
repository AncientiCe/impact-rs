use crate::client::Client;

/// The only thing this file declares. It is called *as the receiver* of a chained call,
/// which is the position that used to be skipped entirely.
pub fn build_client() -> Client {
    Client
}
