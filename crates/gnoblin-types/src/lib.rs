//! The primitives every Gnoblin crate passes around.
//!
//! This crate is the single home of the values that cross a crate boundary as a
//! plain datum: a rectangle, a compositing scale, a window transform, a
//! direction, an identity, a tick, a revision, a version and an error code.
//! Everything here is a `Copy` value type over `std` and nothing else, so the
//! policy core, the engine crates, the client, the compat adapters and the
//! scripts all name the same types instead of each declaring its own `Rect` and
//! converting at the seam.
//!
//! # Coordinates
//!
//! Two coordinate spaces exist on purpose, and the names say which is which:
//!
//! - [`Point`], [`Size`] and [`Rect`] are **logical pixels**. A logical pixel is
//!   what layout, window management, the output layout and the accessibility
//!   machines reason about, and it is a whole number: an origin is an [`i32`]
//!   because an output or a window can sit left of or above the origin, and an
//!   extent is a [`u32`] because a negative extent cannot mean anything and an
//!   unsigned extent makes "logical sizes are integral and non-negative" a
//!   property of the type instead of a rule every caller has to remember.
//! - [`PointF`], [`RectF`] and [`Vec3`] are **continuous** values, [`f64`].
//!   They are what a transform, a camera and a projected bound carry: a
//!   rotation or a fractional scale produces a non-integral position, and
//!   rounding belongs at the far end where a value becomes a pixel again, not
//!   at every step of a composition.
//!
//! The two spaces do not convert implicitly. [`Point::to_f64`] and
//! [`PointF::to_i32`] are the explicit crossings, which is what keeps the
//! projection step visible in a hit-test path.
//!
//! # Identities and monotonic values
//!
//! [`Orientation`] is how an output presents its mode (a panel can be mounted
//! sideways), which the display configuration, the mode resolution and the IPC
//! all name, so it is one type instead of three.
//!
//! [`WindowId`], [`OutputId`], [`WorkspaceId`], [`ObjectId`], [`OperationId`],
//! [`RequestId`], [`SubscriptionId`], [`OwnerId`] and [`ClientId`] are opaque
//! [`u64`] tokens.
//! They are all [`u64`] because the object graph allocates unbounded counters
//! and the engine must be able to hold one without narrowing it; a backend that
//! numbers outputs from zero widens trivially. [`Tick`] is a caller-supplied
//! monotonic reading and [`Revision`] a graph history stamp: neither crate reads
//! a clock, so the same event sequence replays to the same decisions.

mod color;
mod direction;
mod error_code;
mod geometry;
mod ids;
mod keyboard;
mod orientation;
mod parent_window;
mod scale;
mod scene_ids;
mod tick;
mod transform;
mod version;

pub use color::Color;
pub use direction::{Axis, Corner, Direction, Edge};
pub use error_code::{ErrorCode, InvalidErrorCode, is_error_code};
pub use geometry::{Insets, Point, PointF, Rect, RectF, Size, Vec3};
pub use ids::{ClientId, ObjectId, OperationId, OutputId, OwnerId, RequestId, SubscriptionId, WindowId, WorkspaceId};
pub use keyboard::{ButtonState, KeyCode, KeyState, Keysym, Modifiers};
pub use orientation::Orientation;
pub use parent_window::{InvalidParentWindow, MAX_HANDLE_BYTES, MAX_XID_DIGITS, ParentWindow};
pub use scale::Scale;
pub use scene_ids::{NodeId, TrackId};
pub use tick::{Revision, Tick};
pub use transform::{Transform, Unsupported3d, Unsupported3dField};
pub use version::{InvalidVersion, Version};
