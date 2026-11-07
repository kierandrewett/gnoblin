//! The whole-output view pass: the magnifier and the colour matrix, drawn over a copy of the
//! finished output.
//!
//! The pass has its own uniform block of 128 bytes, laid out as `GnoblinView` in
//! `wgsl/view.wgsl`: eight `vec4`s, so std140 and WGSL agree on the offsets without padding.
//! It is never the 96-byte user-shader block, so a user shader's declared fields cannot reach
//! it and this pass cannot change what a layer pass reads.
//!
//! The pass runs only when the frame has a view or a matrix that changes a pixel. An identity
//! view (zoom of one, no movement that matters) and an identity matrix are skipped, so a frame
//! with neither draws exactly as it did before this pass existed.

use gnoblin_render_frame::frame::{ColourMatrix, View};

/// The bytes of the view block: eight `vec4`s.
pub(crate) const VIEW_BYTES: usize = 128;

/// The uniform block of the view pass, packed as the shader reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewUniforms {
    /// The view's centre x, centre y, zoom and an unused slot.
    centre: [f32; 4],
    /// The output's width and height in physical pixels, then two unused slots.
    size: [f32; 4],
    /// The matrix's four rows, one per output channel, each with its four weights.
    rows: [[f32; 4]; 4],
    /// The matrix's offset for each output channel.
    offsets: [f32; 4],
    /// `x` is one when the matrix applies, zero otherwise.
    flags: [f32; 4],
}

impl ViewUniforms {
    /// The block for a view and matrix over an output of `width` by `height` physical pixels.
    ///
    /// A missing view centres on the output at zoom one, which maps every pixel to itself; a
    /// missing matrix sets the flag to zero, so the shader never runs the colour stage.
    pub(crate) fn new(view: Option<View>, matrix: Option<ColourMatrix>, width: u32, height: u32) -> Self {
        let (centre_x, centre_y, zoom) = match view {
            Some(view) => (view.centre_x, view.centre_y, view.zoom),
            None => (0.0, 0.0, 1.0),
        };
        let mut rows = [[0.0f32; 4]; 4];
        let mut offsets = [0.0f32; 4];
        let mut flag = 0.0f32;
        if let Some(matrix) = matrix {
            flag = 1.0;
            for (channel, row) in matrix.rows.iter().enumerate() {
                rows[channel] = [row[0], row[1], row[2], row[3]];
                offsets[channel] = row[4];
            }
        }
        Self {
            centre: [centre_x, centre_y, zoom, 0.0],
            size: [width as f32, height as f32, 0.0, 0.0],
            rows,
            offsets,
            flags: [flag, 0.0, 0.0, 0.0],
        }
    }

    /// The block's bytes, in the order the shader reads them.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(VIEW_BYTES);
        for vector in [
            self.centre,
            self.size,
            self.rows[0],
            self.rows[1],
            self.rows[2],
            self.rows[3],
            self.offsets,
            self.flags,
        ] {
            for value in vector {
                out.extend_from_slice(&value.to_ne_bytes());
            }
        }
        out
    }
}

/// Whether a frame's view or colour matrix changes any pixel, so the pass has work to do.
///
/// A view with a zoom of one maps every pixel to itself whatever its centre, so it is idle. A
/// matrix is idle when it equals the identity. The renderer draws the frame as it always did
/// when this returns false.
pub(crate) fn is_active(view: Option<View>, matrix: Option<ColourMatrix>) -> bool {
    let magnified = view.is_some_and(|view| view.zoom != 1.0);
    let tinted = matrix.is_some_and(|matrix| matrix.rows != ColourMatrix::IDENTITY.rows);
    magnified || tinted
}
