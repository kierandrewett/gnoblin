//! Who may create, move and destroy a scene node, and the id ranges and caps the
//! tree enforces (design/node-core.md §3.1, §3.2 and §3.5).
//!
//! Ownership governs lifecycle and parentage only. A node's creator decides who
//! may destroy it. A node's parent is set by whoever last gave it one: a node the
//! compositor parented is the compositor's to move, and the shell may move any
//! other node. Property writes are not governed by ownership at all; they are
//! governed by the schema's `readonly` flag, whoever owns the node.

/// Who is acting on the scene: the compositor, or the shell over its connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The compositor. It creates the windows, layer surfaces, cursor and lock
    /// nodes, and it parents the ones that must stay where it put them.
    Compositor,
    /// The shell. It places what the compositor left unparented, and it may
    /// destroy only the nodes it created.
    Shell,
}

/// The first compositor id: ids at or above this bit are compositor-allocated.
pub const COMPOSITOR_ID_BASE: u64 = 1 << 63;

/// Shell ids are strictly below this bound, so they cannot collide with
/// compositor ids. The shell's id is `(epoch << 32) | counter`, which stays
/// below it while the epoch is below `2^31`.
pub const SHELL_ID_LIMIT: u64 = 1 << 63;

/// The most nodes one scene holds. An insert beyond it is refused `too-many-nodes`.
pub const MAX_NODES: usize = 65536;

/// The deepest a node may sit, counting the root level as one. An insert or a move
/// that would place a node deeper is refused `too-deep`.
pub const MAX_DEPTH: usize = 256;
