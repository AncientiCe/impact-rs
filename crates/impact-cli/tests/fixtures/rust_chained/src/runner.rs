use crate::factory::build_client;

pub fn run() -> bool {
    build_client().send()
}
