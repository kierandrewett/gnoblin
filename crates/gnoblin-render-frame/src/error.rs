//! Why a renderer could not draw.

use std::fmt;

/// Why a renderer could not draw.
///
/// It is deliberately small and `Copy`: a frame failure must never be the reason a
/// session stops, so the compositor logs it and keeps the previous pixels. The
/// detail of a pixman failure is in the log line where it happened, not in the
/// value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderError {
    /// The renderer or its buffer could not be created.
    Unavailable,
    /// The requested buffer or output covers no pixels.
    Degenerate {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
    /// The renderer refused a drawing operation.
    Failed,
    /// The device behind the renderer is gone, so the frame was dropped.
    ///
    /// Nothing the renderer created can be used again: the caller builds a new renderer
    /// and damages the whole output. Only a GPU renderer reports it.
    Lost,
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(formatter, "the renderer could not be created"),
            Self::Degenerate { width, height } => write!(formatter, "a {width}x{height} target covers no pixels"),
            Self::Failed => write!(formatter, "the renderer refused a drawing operation"),
            Self::Lost => write!(formatter, "the renderer's device was lost"),
        }
    }
}

impl std::error::Error for RenderError {}
