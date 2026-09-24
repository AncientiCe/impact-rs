/// `pod-gateway` is only a dev-dependency of this crate, so its shipped code can't
/// reach `Bridge` — this `relay_frames` is some other type's method.
fn flush(pending: &Pending) -> usize {
    pending.relay_frames()
}

fn main() {}
