use std::sync::Arc;

fn main() {
    let store = Arc::new(store_core::Store::open(64));
    let ledger = store_core::Ledger::new();
    println!("{} {}", store.capacity(), ledger.len());
}

fn spare_stores() -> Vec<store_core::Store> {
    [8u32, 16].into_iter().map(store_core::Store::open).collect()
}
