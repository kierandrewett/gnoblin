//! Helpers shared by the scene integration tests.

use gnoblin_render::Scene;
use gnoblin_types::NodeId;

/// FNV-1a over the whole tree: every id, its parent, its children in paint order and
/// its visual transform (the `Debug` form prints every field of it).
pub fn tree_hash(scene: &Scene) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut ids: Vec<NodeId> = scene.node_ids().collect();
    ids.sort();
    for node in ids {
        feed(&node.get().to_le_bytes());
        feed(format!("{:?}", scene.parent(node)).as_bytes());
        feed(format!("{:?}", scene.children(node)).as_bytes());
        feed(format!("{:?}", scene.transform(node)).as_bytes());
    }
    hash
}
