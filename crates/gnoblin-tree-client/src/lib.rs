//! The client side of the tree protocol, with no script runtime involved.
//!
//! [`tree_mirror`] holds the shell's copy of the object graph and the transaction builder
//! that turns script edits into a batch the compositor applies. [`tree_link`] carries those
//! batches over the owner socket. A runtime (retired today, JavaScript next) sits on top of these
//! types and owns only the translation between its values and [`tree_mirror::MirrorValue`].

pub mod slot_link;
pub mod tree_link;
pub mod tree_mirror;
