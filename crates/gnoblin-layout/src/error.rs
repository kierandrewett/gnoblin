//! Reasons a layout reply is refused, and the failures a host reports.

use gnoblin_types::{ErrorCode, WindowId};

/// The wire code for a refused reply.
pub const REFUSED_CODE: ErrorCode = ErrorCode::from_static("refused");

/// What a transform or camera value belongs to, for error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    /// The placement of one tiled window.
    Window(WindowId),
    /// The camera of the reply.
    Camera,
}

/// Why one reply was refused. Every variant is a check the validator runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyError {
    /// A placement or float effect names a window that is not in the tiled set.
    UnknownWindow(WindowId),
    /// Two placements name the same window.
    DuplicatePlacement(WindowId),
    /// Two float effects name the same window.
    DuplicateEffect(WindowId),
    /// A tiled window has neither a placement nor a float effect.
    MissingWindow(WindowId),
    /// A float effect names a window that also has a placement.
    FloatedWindowPlaced(WindowId),
    /// A value that must be finite is NaN or infinite.
    NonFinite {
        /// Which placement or camera carries the value.
        subject: Subject,
        /// Name of the field in the reply.
        field: &'static str,
    },
    /// A translation or size lies outside the accepted bound.
    OutOfBounds {
        /// Which placement or camera carries the value.
        subject: Subject,
        /// Name of the field in the reply.
        field: &'static str,
    },
    /// A width or height is negative.
    NegativeSize(WindowId),
    /// A transform cannot be inverted, so the window has no defined inverse.
    NotInvertible(Subject),
    /// An opacity is outside `0.0..=1.0`.
    OpacityOutOfRange(WindowId),
    /// A perspective camera has a field of view outside `(0, 180)` degrees.
    FieldOfViewOutOfRange,
    /// A perspective camera has a near plane that is not positive.
    NearPlaneInvalid,
    /// A perspective camera has a far plane that is not beyond its near plane.
    FarPlaneInvalid,
}

impl ReplyError {
    /// The code reported to the caller of the layout.
    pub const fn code(&self) -> ErrorCode {
        REFUSED_CODE
    }
}

/// Why a layout run produced no usable layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutFailure {
    /// The layout did not answer before its deadline.
    Deadline,
    /// The layout ran out of memory.
    Memory,
    /// The layout raised an error.
    Error,
    /// The layout stopped making progress.
    Stalled,
    /// The Lua virtual machine is gone.
    VmDown,
    /// The module that defines the layout is not loaded.
    MissingModule,
    /// The layout answered, but the reply failed validation.
    RefusedReply(ReplyError),
}

/// Why a set of layout parameters cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamsError {
    /// An output size or window size is NaN or infinite.
    NonFinite(&'static str),
    /// An output size or window size is outside the accepted bound.
    OutOfBounds(&'static str),
    /// An output size is zero or negative.
    EmptyOutput,
    /// Two tiled windows share an id.
    DuplicateWindow(WindowId),
}
