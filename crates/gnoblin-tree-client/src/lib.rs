//! The client side of the tree protocol, with no script runtime involved.
//!
//! [`tree_mirror`] holds the shell's copy of the object graph and the transaction builder
//! that turns script edits into a batch the compositor applies. [`tree_link`] carries those
//! batches over the owner socket. A runtime (retired today, JavaScript next) sits on top of these
//! types and owns only the translation between its values and [`tree_mirror::MirrorValue`].
//!
//! [`relay`] is the byte form of the link for a runtime library that receives it across the C ABI.
//!
//! [`gate`] is the transaction gate (one in flight, refusals sent again) and [`session`] the whole connection built on the mirror,
//! the builder and the gate: the one place the shell side of the tree protocol is implemented, for every language.

pub mod gate;
pub mod relay;
pub mod session;
pub mod slot_link;
pub mod tree_link;
pub mod tree_mirror;
