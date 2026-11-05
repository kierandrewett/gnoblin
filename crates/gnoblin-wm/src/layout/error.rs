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

/// Why a layout run produced no usable layout. Each reason names one fallback
/// the wm takes for the workspace on that frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutFailure {
    /// The layout raised an error, or ran out of memory.
    Error,
    /// The layout did not answer before its deadline.
    Timeout,
    /// Lua layouts are switched off for this session, or the module is not loaded.
    Disabled,
    /// The layout's worker stopped answering and was abandoned.
    VmWedged,
    /// The layout answered, but the reply failed validation.
    InvalidReply(ReplyError),
}

impl LayoutFailure {
    /// The name reported in the `layout-failed` effect.
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Disabled => "disabled",
            Self::VmWedged => "vm-wedged",
            Self::InvalidReply(_) => "invalid-reply",
        }
    }
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
