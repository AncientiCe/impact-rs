pub struct Container<T> {
    pub value: T,
}

impl<T> Container<T> {
    pub fn get(&self) -> bool {
        crate::util::helper()
    }
}
