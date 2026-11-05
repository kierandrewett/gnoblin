//! Layout value types, reply validation and the built-in floating fallback for
//! Lua layouts (design/rust-compositor.md, D17).
//!
//! This module is pure: it holds no compositor state, reads no clock and does
//! no IO. The Lua host builds a [`Params`], asks the layout for a [`Reply`], and
//! passes both to [`settle`], which either accepts the reply in canonical form
//! or refuses it with a [`ReplyError`]. A declined reply falls back to
//! [`fallback`], a cascade of floating windows.

mod engine;
mod error;
mod fallback;
mod placement;
mod settle;
mod transform;

pub use engine::{Collected, Generation, Job, LayoutEngine, LayoutOutcome, Slot, Ticket};

pub use error::{LayoutFailure, ParamsError, REFUSED_CODE, ReplyError, Subject};
pub use fallback::fallback;
pub use placement::{
    Camera, Handled, LayoutEffect, LayoutOutput, LayoutWindow, Origin, Params, Placement, Reply, Request, RequestKind,
};
pub use settle::{Layout, settle};
pub use transform::{BOUND, Matrix4, Transform3};
