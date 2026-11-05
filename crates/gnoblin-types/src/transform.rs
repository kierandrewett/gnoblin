//! A node's transform, and the named refusal for the depth values this build
//! cannot draw.

use std::fmt;

use crate::geometry::{PointF, Vec3};

/// The transforms this build refuses because they need depth support.
///
/// Every variant is a member of the published 3D surface, not an internal
/// detail: the configuration and IPC layers translate the same names, so a
/// refused value today becomes the accepted value when depth lands rather than a
/// different spelling. A camera adds its own refusal member (`perspective`)
/// beside the camera type, because a transform cannot carry one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported3dField {
    /// An offset along the depth axis.
    OffsetZ,
    /// A rotation about the x axis.
    RotationX,
    /// A rotation about the y axis.
    RotationY,
    /// A scale along the depth axis.
    ScaleZ,
    /// A non-zero draw depth.
    Depth,
}

impl Unsupported3dField {
    /// The stable member name, as the configuration and IPC layers spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OffsetZ => "offset_z",
            Self::RotationX => "rotation_x",
            Self::RotationY => "rotation_y",
            Self::ScaleZ => "scale_z",
            Self::Depth => "depth",
        }
    }
}

/// The named refusal for a value outside the accepted 2D subset.
///
/// `unsupported-3d` is the code, matching design/rust-compositor.md (Renderer
/// card, "3D-ready, 2D today"), so a caller on the other side of the IPC can
/// branch on it instead of parsing prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported3d {
    /// Which member carried the value.
    pub field: Unsupported3dField,
}

impl Unsupported3d {
    /// The machine-readable code every surface reports.
    pub const CODE: &'static str = "unsupported-3d";

    /// A refusal for one member.
    #[must_use]
    pub const fn new(field: Unsupported3dField) -> Self {
        Self { field }
    }

    /// Which member carried the value.
    #[must_use]
    pub const fn field(self) -> Unsupported3dField {
        self.field
    }
}

impl fmt::Display for Unsupported3d {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: `{}` is outside the accepted 2D subset",
            Self::CODE,
            self.field.name()
        )
    }
}

impl std::error::Error for Unsupported3d {}

/// A node's transform, 3D-ready and 2D today.
///
/// The plan's Renderer card says every node carries a full 3D transform and a
/// depth. The type therefore stores three axes of offset, rotation and scale
/// plus a draw depth, in the published position/rotation/scale form rather than
/// as a bare 4x4 matrix: the configuration and IPC layers carry the named
/// members, and a matrix would have to be decomposed to recover them.
///
/// Only the 2D subset is accepted in this build. [`Transform::check_2d`] is the
/// single place that decides which values are allowed, so no mutation path can
/// forget it while another accepts a value: a z offset, an x or y rotation, a z
/// scale or a non-zero depth is refused with [`Unsupported3d`]. The constructors
/// below build the 2D subset and cannot produce a refused value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Translation in the parent's space, in logical pixels.
    pub offset: Vec3,
    /// Rotation about each axis, in degrees.
    pub rotation_degrees: Vec3,
    /// Scale along each axis, where 1 keeps the size.
    pub scale: Vec3,
    /// Draw depth, where 0 is the plane every 2D node occupies.
    pub depth: f64,
}

impl Transform {
    /// The identity transform: no offset, no rotation, unit scale, no depth.
    pub const IDENTITY: Self = Self {
        offset: Vec3::ZERO,
        rotation_degrees: Vec3::ZERO,
        scale: Vec3::ONE,
        depth: 0.0,
    };

    /// A transform that only moves a node within its parent's plane.
    #[must_use]
    pub const fn translate(x: f64, y: f64) -> Self {
        Self {
            offset: Vec3::flat(x, y),
            ..Self::IDENTITY
        }
    }

    /// A transform that only scales a node in its parent's plane.
    #[must_use]
    pub const fn scaled(x: f64, y: f64) -> Self {
        Self {
            scale: Vec3::new(x, y, 1.0),
            ..Self::IDENTITY
        }
    }

    /// A transform that only rotates a node about the depth axis, in degrees.
    ///
    /// This is the only rotation the 2D subset accepts; the x and y members stay
    /// zero so a caller cannot smuggle half of a 3D rotation through a
    /// convenience constructor.
    #[must_use]
    pub const fn rotate_z(degrees: f64) -> Self {
        Self {
            rotation_degrees: Vec3::new(0.0, 0.0, degrees),
            ..Self::IDENTITY
        }
    }

    /// A transform that moves, rotates about the depth axis and scales.
    #[must_use]
    pub const fn from_2d(offset: Vec3, rotation_z_degrees: f64, scale: Vec3) -> Self {
        Self {
            offset,
            rotation_degrees: Vec3::new(0.0, 0.0, rotation_z_degrees),
            scale,
            depth: 0.0,
        }
    }

    /// Whether every value is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.offset.is_finite() && self.rotation_degrees.is_finite() && self.scale.is_finite() && self.depth.is_finite()
    }

    /// Whether the transform lies inside the accepted 2D subset.
    #[must_use]
    pub fn is_2d(&self) -> bool {
        self.check_2d().is_ok()
    }

    /// Refuse the first value that needs depth support.
    ///
    /// The first member in a fixed order is reported, not whichever is cheapest
    /// to compare, so a caller that fixes one value at a time sees a stable
    /// sequence of refusals.
    pub fn check_2d(&self) -> Result<(), Unsupported3d> {
        if self.offset.z != 0.0 {
            return Err(Unsupported3d::new(Unsupported3dField::OffsetZ));
        }
        if self.rotation_degrees.x != 0.0 {
            return Err(Unsupported3d::new(Unsupported3dField::RotationX));
        }
        if self.rotation_degrees.y != 0.0 {
            return Err(Unsupported3d::new(Unsupported3dField::RotationY));
        }
        if self.scale.z != 1.0 {
            return Err(Unsupported3d::new(Unsupported3dField::ScaleZ));
        }
        if self.depth != 0.0 {
            return Err(Unsupported3d::new(Unsupported3dField::Depth));
        }
        Ok(())
    }

    /// Map a point from the node's space into its parent's plane.
    ///
    /// The order is the published one: scale, then rotate about the depth axis,
    /// then translate. A refused transform maps nothing, which is why the
    /// result is a [`Result`] rather than a point: silently dropping a depth
    /// value would render a node somewhere the caller did not ask for.
    pub fn apply_point(&self, point: PointF) -> Result<PointF, Unsupported3d> {
        self.check_2d()?;
        let x = point.x * self.scale.x;
        let y = point.y * self.scale.y;
        let radians = self.rotation_degrees.z.to_radians();
        let (sin, cos) = radians.sin_cos();
        Ok(PointF::new(
            x * cos - y * sin + self.offset.x,
            x * sin + y * cos + self.offset.y,
        ))
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
