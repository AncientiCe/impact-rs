use pod_gateway::bridge::Bridge;

fn run(bridge: &Bridge) -> usize {
    Bridge::relay_frames(bridge)
}

fn main() {}
