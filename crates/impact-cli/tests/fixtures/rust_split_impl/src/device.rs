// Another type with a method of the same name: no `Session` call reaches it.
pub struct Device;

impl Device {
    pub fn authenticate(&self) -> bool {
        true
    }
}
