//! The OpenXR pointer as seat input.
//!
//! Each frame the bridge casts the runtime's pointing ray against the planned layers.
//! [`Pointer`] turns the result into owner `input` ops: a motion over the window under the
//! ray, and a leave when the ray stops hitting it. Button and scroll input are not wired:
//! the runtime exposes no button source yet.
use xr_scene::{Layer, NodeId, Ray, pick};
use xr_tree::{PointerInput, TreeClient, TreeError};

/// The seat the OpenXR pointer uses. The owner creates it on first use, unless the snapshot
/// already holds a seat named `xr`, which the pointer reuses.
pub const SEAT_NAME: &str = "xr";

/// What the pointer last told the owner. Motion is sent only when the hit point changes.
#[derive(Debug, Default)]
pub struct Pointer {
    seat: Option<NodeId>,
    hovered: Option<NodeId>,
    last: Option<(f64, f64)>,
}

impl Pointer {
    /// Casts `ray` against `layers` and sends the input it produces.
    ///
    /// A missing ray or a miss sends a leave for the window last hovered. A hit over a
    /// different window leaves the old one and moves onto the new one. A hit over the same
    /// window sends motion only when the pixel changes.
    ///
    /// # Errors
    ///
    /// The owner refuses a seat or input transaction, or the socket fails.
    pub fn update(&mut self, tree: &mut TreeClient, layers: &[Layer], ray: Option<Ray>) -> Result<(), TreeError> {
        let hit = ray.and_then(|ray| pick(layers, &ray));
        let Some(hit) = hit else {
            return self.leave(tree);
        };

        let seat = self.ensure_seat(tree)?;
        if self.hovered != Some(hit.source) {
            self.leave(tree)?;
            self.hovered = Some(hit.source);
            self.last = None;
        }

        let point = (hit.u, hit.v);
        if self.last != Some(point) {
            tree.send_input(&PointerInput::Motion {
                seat,
                node: hit.source,
                x: point.0,
                y: point.1,
            })?;
            self.last = Some(point);
        }
        Ok(())
    }

    /// Sends a leave for the hovered window, if there is one, and forgets it.
    fn leave(&mut self, tree: &mut TreeClient) -> Result<(), TreeError> {
        if let (Some(seat), Some(node)) = (self.seat, self.hovered.take()) {
            tree.send_input(&PointerInput::Leave { seat, node })?;
        }
        self.last = None;
        Ok(())
    }

    /// The pointer seat, found by name or created on first use.
    fn ensure_seat(&mut self, tree: &mut TreeClient) -> Result<NodeId, TreeError> {
        if let Some(seat) = self.seat {
            return Ok(seat);
        }
        let seat = match tree.seat_named(SEAT_NAME) {
            Some(seat) => seat,
            None => tree.create_seat(SEAT_NAME)?,
        };
        self.seat = Some(seat);
        Ok(seat)
    }
}
