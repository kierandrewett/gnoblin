//! The messages a layout exchanges with the compositor.

use std::collections::BTreeSet;

use gnoblin_types::WindowId;

use crate::error::ParamsError;
use crate::transform::{BOUND, Transform3};

/// A window the layout must place or float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutWindow {
    /// The window's identity.
    pub id: WindowId,
    /// Current width in logical units, before the layout runs.
    pub width: f64,
    /// Current height in logical units, before the layout runs.
    pub height: f64,
}

/// Validated input to a layout run: the output size and the tiled windows.
///
/// Fields are private so a `Params` always has a finite, positive output, finite
/// bounded window sizes and no duplicate windows. Build one with [`Params::new`].
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    output_width: f64,
    output_height: f64,
    windows: Vec<LayoutWindow>,
}

impl Params {
    /// Checks the output size and every window, and keeps the window order.
    pub fn new(output_width: f64, output_height: f64, windows: Vec<LayoutWindow>) -> Result<Self, ParamsError> {
        check_size("output width", output_width)?;
        check_size("output height", output_height)?;
        if output_width <= 0.0 || output_height <= 0.0 {
            return Err(ParamsError::EmptyOutput);
        }
        let mut seen = BTreeSet::new();
        for window in &windows {
            check_size("window width", window.width)?;
            check_size("window height", window.height)?;
            if !seen.insert(window.id) {
                return Err(ParamsError::DuplicateWindow(window.id));
            }
        }
        Ok(Self {
            output_width,
            output_height,
            windows,
        })
    }

    /// Output width in logical units.
    pub fn output_width(&self) -> f64 {
        self.output_width
    }

    /// Output height in logical units.
    pub fn output_height(&self) -> f64 {
        self.output_height
    }

    /// The tiled windows, in the order the compositor supplied them.
    pub fn windows(&self) -> &[LayoutWindow] {
        &self.windows
    }
}

fn check_size(field: &'static str, value: f64) -> Result<(), ParamsError> {
    if !value.is_finite() {
        Err(ParamsError::NonFinite(field))
    } else if value.abs() > BOUND {
        Err(ParamsError::OutOfBounds(field))
    } else {
        Ok(())
    }
}

/// Where one window goes and how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The tiled window this placement moves.
    pub window: WindowId,
    /// Width in logical units, before the transform.
    pub width: f64,
    /// Height in logical units, before the transform.
    pub height: f64,
    /// The 3D transform applied to the window.
    pub transform: Transform3,
    /// Whether the window is drawn at all.
    pub visible: bool,
    /// Opacity from 0.0 (invisible) to 1.0 (opaque).
    pub opacity: f64,
    /// Whether server-side decorations are drawn around the window.
    pub decorated: bool,
}

impl Placement {
    /// A visible, opaque, decorated flat rectangle at `(x, y)`.
    pub fn rect(window: WindowId, x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            window,
            width,
            height,
            transform: Transform3::at(x, y),
            visible: true,
            opacity: 1.0,
            decorated: true,
        }
    }
}

/// The projection a layout uses for its placements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Camera {
    /// Flat projection: depth is ignored. This is the default.
    Orthographic,
    /// Perspective projection with a pose for the eye.
    Perspective {
        /// Vertical field of view in degrees, strictly between 0 and 180.
        fov_degrees: f64,
        /// Near clip distance, strictly positive.
        near: f64,
        /// Far clip distance, strictly beyond `near`.
        far: f64,
        /// The camera's pose in the layout's space.
        pose: Transform3,
    },
}

/// A side effect a layout asks the compositor to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEffect {
    /// Take the window out of tiling and let it keep its own geometry.
    Float {
        /// The tiled window to float.
        window: WindowId,
    },
}

impl LayoutEffect {
    /// The window this effect names.
    pub fn window(&self) -> WindowId {
        match *self {
            Self::Float { window } => window,
        }
    }
}

/// A layout that answered and accepted the request.
#[derive(Clone, Debug, PartialEq)]
pub struct Handled {
    /// One placement per placed window, in any order until validated.
    pub placements: Vec<Placement>,
    /// The camera the placements are projected through.
    pub camera: Camera,
    /// Side effects, in any order until validated.
    pub effects: Vec<LayoutEffect>,
}

/// What a layout returns for one request.
#[derive(Clone, Debug, PartialEq)]
// A reply is built once per layout run, so the size gap between variants is accepted.
#[allow(clippy::large_enum_variant)]
pub enum Reply {
    /// The layout placed or floated every tiled window.
    Handled(Handled),
    /// The layout chose not to lay out; the built-in fallback applies.
    Declined,
}

/// One request to a layout: the validated parameters for a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The output and tiled windows the layout must answer for.
    pub params: Params,
}

/// Which source produced a [`crate::Layout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The layout returned a valid [`Handled`] reply.
    Reply,
    /// The layout declined, and the built-in fallback was used.
    Fallback,
}
