//! XML protocol `wlr-virtual-pointer-unstable-v1`: the interfaces `zwlr_virtual_pointer_manager_v1` and
//! `zwlr_virtual_pointer_v1`.
//!
//! A virtual pointer is a pointer a client makes: a remote-desktop server, a switch-access tool or an
//! automation harness moves it, presses its buttons and scrolls it, and the session treats those events as it
//! treats a mouse's. Smithay serves the protocol objects and turns every request into an input event; this
//! module is the compositor's side of that: it turns the events into the events the seat routes
//! (`gnoblin_backend::InputEvent`), so a virtual pointer's motion goes through [`Host::input`] and meets the
//! lock card, a modal, a shell's pointer chords and grabs, the confinement a client holds and the focus rules
//! exactly as a real device's does.
//!
//! ## What each request becomes
//!
//! - `motion` is a relative move. The protocol's deltas are fractional; the seat's relative event is whole
//!   logical pixels, so each pointer keeps the fraction it has not yet moved by and adds it to the next delta.
//! - `motion_absolute` is a position as a fraction of a client-chosen extent. It maps onto the layout's
//!   bounding box, or onto the output a `create_virtual_pointer_with_output` pointer named, and lands on the
//!   output that holds the point (the nearest output when the point falls in a gap of the layout).
//! - `button` is a button edge. A pointer destroyed with a button down releases it, which Smithay does before
//!   it removes the device.
//! - `axis`, `axis_source`, `axis_discrete`, `axis_stop` and `frame` make one scroll frame
//!   ([`crate::scroll::Scroll`]): a wheel step counts its 1/120 units, a finger or continuous source carries
//!   pixels only, and a stop ends a kinetic scroll. A frame is sent whole, so a client sees one `wl_pointer`
//!   axis group per `frame` request.
//!
//! ## Who may bind it
//!
//! A virtual pointer clicks anything on the screen, including another client's consent buttons, so the manager
//! is a privileged global (`Global::VirtualPointer`): the session's own processes see it on the privileged
//! socket and an application on the ordinary socket does not. A sandbox is refused it whatever else it earned.
//! The decision is [`crate::access::decide`]'s, as it is for every other privileged global.
//!
//! ## Failure modes
//!
//! Each is a test in `tests/virtual_pointer.rs`:
//!
//! 1. An application on the ordinary socket never sees the manager:
//!    `an_unprivileged_client_does_not_see_the_virtual_pointer_manager`, and a sandbox is refused it too:
//!    `a_sandboxed_client_does_not_see_the_virtual_pointer_manager`.
//! 2. A move and a click reach the client under the pointer: `a_virtual_pointer_moves_and_clicks_a_window`.
//! 3. An absolute move lands where its fraction says, on the whole layout and on a named output:
//!    `an_absolute_motion_lands_at_its_fraction_of_the_layout`.
//! 4. Scroll frames keep their source, steps and stop:
//!    `a_wheel_frame_reaches_the_client_with_its_steps`, `a_finger_frame_reaches_the_client_and_stops`.
//! 5. A pointer destroyed with a button down releases it: `a_destroyed_pointer_releases_its_held_button`.
//! 6. A modal that is up takes the click and the client sees none: `a_virtual_click_under_a_modal_goes_to_the_modal`.
//! 7. A zero extent or an invalid axis cannot move the pointer or leave a half frame: Smithay drops the first
//!    and answers the second with the protocol's `invalid_axis` error.

use std::collections::HashMap;
use std::sync::Arc;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, InputEvent as DeviceEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::virtual_pointer::{
    VirtualPointerBackend, VirtualPointerDevice, VirtualPointerHandler, VirtualPointerManagerState,
};

use gnoblin_backend::{Host, InputEvent};
use gnoblin_types::{DeviceId, OutputId, Point, Rect};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use crate::log;
use crate::scroll::{Scroll, ScrollAxis};

/// The global and what the session keeps for the pointers clients made.
pub(crate) struct VirtualPointerServing {
    /// Kept alive so `zwlr_virtual_pointer_manager_v1` stays registered; the bind is gated by the roster.
    #[allow(dead_code)]
    manager: VirtualPointerManagerState,
    /// The fraction of a pixel each pointer has moved by and not yet reported.
    remainder: HashMap<VirtualPointerDevice, (f64, f64)>,
}

/// Creates the `zwlr_virtual_pointer_manager_v1` global behind the access roster.
pub(crate) fn global(display: &DisplayHandle, access: &Arc<Access>) -> VirtualPointerServing {
    let access = Arc::clone(access);
    VirtualPointerServing {
        manager: VirtualPointerManagerState::new::<State, _>(display, move |client: &Client| {
            AccessPolicy::allows(&*access, &client.id(), Global::VirtualPointer)
        }),
        remainder: HashMap::new(),
    }
}

/// What this module asks of the compositor: the seat's input path and the layout. The whole of the module's
/// reach into [`State`] is these four methods, so the protocol can live outside this crate's state.
pub(crate) trait PointerSeat {
    /// Delivers one event the way a backend's device would.
    fn deliver(&mut self, event: InputEvent);
    /// Delivers one scroll frame the way a backend's device would.
    fn deliver_scroll(&mut self, scroll: Scroll);
    /// The outputs and the logical rectangle each occupies, in the layout's order.
    fn layout(&self) -> Vec<(OutputId, Rect)>;
    /// The output a smithay `Output` is, when the session created it.
    fn output_named(&self, output: &smithay::output::Output) -> Option<OutputId>;
    /// The pointers' bookkeeping.
    fn serving(&mut self) -> &mut VirtualPointerServing;
}

impl PointerSeat for State {
    fn deliver(&mut self, event: InputEvent) {
        self.input(event);
    }

    fn deliver_scroll(&mut self, scroll: Scroll) {
        self.input_scroll(scroll);
    }

    fn layout(&self) -> Vec<(OutputId, Rect)> {
        self.outputs.iter().map(|entry| (entry.backend, entry.rect)).collect()
    }

    fn output_named(&self, output: &smithay::output::Output) -> Option<OutputId> {
        self.output_id_of(output)
    }

    fn serving(&mut self) -> &mut VirtualPointerServing {
        &mut self.virtual_pointer
    }
}

impl VirtualPointerHandler for State {
    fn process_virtual_pointer_event(&mut self, event: DeviceEvent<VirtualPointerBackend>) {
        process(self, event);
    }
}

/// Turns one virtual pointer event into what the seat routes.
fn process<S: PointerSeat>(seat: &mut S, event: DeviceEvent<VirtualPointerBackend>) {
    match event {
        DeviceEvent::DeviceAdded { device } => {
            log::debug!(area = "virtual-pointer", device = ?device_name(&device), "a client made a virtual pointer");
            seat.serving().remainder.insert(device, (0.0, 0.0));
        }
        DeviceEvent::DeviceRemoved { device } => {
            seat.serving().remainder.remove(&device);
            log::debug!(area = "virtual-pointer", "a virtual pointer was destroyed");
        }
        DeviceEvent::PointerMotion { event } => {
            let device = event.device();
            let remainder = seat.serving().remainder.entry(device).or_insert((0.0, 0.0));
            let (dx, dy, kept) = whole_pixels(*remainder, (event.delta_x(), event.delta_y()));
            *remainder = kept;
            if dx != 0 || dy != 0 {
                seat.deliver(InputEvent::PointerMotionRelative {
                    device: DeviceId::VIRTUAL,
                    dx,
                    dy,
                });
            }
        }
        DeviceEvent::PointerMotionAbsolute { event } => {
            let device = event.device();
            let named = device.output().and_then(|output| seat.output_named(&output));
            let layout = seat.layout();
            if let Some((output, position)) = place(&layout, named, event.x(), event.y()) {
                seat.deliver(InputEvent::PointerMotion {
                    device: DeviceId::VIRTUAL,
                    output,
                    position,
                });
            }
        }
        DeviceEvent::PointerButton { event } => {
            seat.deliver(InputEvent::PointerButton {
                device: DeviceId::VIRTUAL,
                button: event.button_code(),
                pressed: event.state() == ButtonState::Pressed,
            });
        }
        DeviceEvent::PointerAxis { event } => seat.deliver_scroll(scroll_of(&event)),
        // The other kinds of event are keyboards, touch and tablets, which this protocol never makes.
        _ => {}
    }
}

/// The device's name for a log line.
fn device_name(device: &VirtualPointerDevice) -> String {
    smithay::backend::input::Device::id(device)
}

/// The whole pixels a delta moves by, given what an earlier delta left over, and what is left over now.
///
/// The remainder keeps its sign, so a pointer nudged by `0.4` three times moves one pixel on the third, and a
/// pointer nudged back by the same distance returns to where it was instead of drifting.
fn whole_pixels(remainder: (f64, f64), delta: (f64, f64)) -> (i32, i32, (f64, f64)) {
    let x = remainder.0 + delta.0;
    let y = remainder.1 + delta.1;
    // `trunc` rounds toward zero, which keeps the remainder's sign with the delta's.
    let (whole_x, whole_y) = (x.trunc().clamp(-1.0e9, 1.0e9), y.trunc().clamp(-1.0e9, 1.0e9));
    #[allow(clippy::cast_possible_truncation)]
    (whole_x as i32, whole_y as i32, (x - whole_x, y - whole_y))
}

/// The output and output-local position a fraction of the layout (or of one output) is.
///
/// `x` and `y` are in `0.0..=1.0`. With `only` set the fraction is of that output's rectangle; otherwise it is of
/// the bounding box of every output. A point of the box that no output holds (the corner of an L-shaped layout)
/// is taken to the nearest output, so a client that addresses the whole screen never loses the pointer in a gap.
fn place(layout: &[(OutputId, Rect)], only: Option<OutputId>, x: f64, y: f64) -> Option<(OutputId, Point)> {
    let region = match only.and_then(|id| layout.iter().find(|(output, _)| *output == id)) {
        Some((_, rect)) => *rect,
        None => bounds(layout)?,
    };
    let target_x = f64::from(region.x) + x.clamp(0.0, 1.0) * f64::from(region.width.saturating_sub(1));
    let target_y = f64::from(region.y) + y.clamp(0.0, 1.0) * f64::from(region.height.saturating_sub(1));
    let candidates: Vec<&(OutputId, Rect)> = match only {
        Some(id) if layout.iter().any(|(output, _)| *output == id) => {
            layout.iter().filter(|(output, _)| *output == id).collect()
        }
        _ => layout.iter().collect(),
    };
    let mut best: Option<(f64, OutputId, Rect, f64, f64)> = None;
    for (output, rect) in candidates {
        let clamped_x = target_x.clamp(
            f64::from(rect.x),
            f64::from(rect.x) + f64::from(rect.width.saturating_sub(1)),
        );
        let clamped_y = target_y.clamp(
            f64::from(rect.y),
            f64::from(rect.y) + f64::from(rect.height.saturating_sub(1)),
        );
        let distance = (clamped_x - target_x).powi(2) + (clamped_y - target_y).powi(2);
        if best.is_none_or(|(least, ..)| distance < least) {
            best = Some((distance, *output, *rect, clamped_x, clamped_y));
        }
    }
    let (_, output, rect, at_x, at_y) = best?;
    #[allow(clippy::cast_possible_truncation)]
    let position = Point {
        x: at_x.round() as i32 - rect.x,
        y: at_y.round() as i32 - rect.y,
    };
    Some((output, position))
}

/// The bounding box of every output in the layout.
fn bounds(layout: &[(OutputId, Rect)]) -> Option<Rect> {
    let mut boxes = layout.iter().map(|(_, rect)| *rect);
    let first = boxes.next()?;
    let (mut left, mut top) = (first.x, first.y);
    let (mut right, mut bottom) = (
        first.x.saturating_add_unsigned(first.width),
        first.y.saturating_add_unsigned(first.height),
    );
    for rect in boxes {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.x.saturating_add_unsigned(rect.width));
        bottom = bottom.max(rect.y.saturating_add_unsigned(rect.height));
    }
    Some(Rect {
        x: left,
        y: top,
        width: right.abs_diff(left),
        height: bottom.abs_diff(top),
    })
}

/// One `frame` of axis requests as a scroll frame.
fn scroll_of(event: &impl PointerAxisEvent<VirtualPointerBackend>) -> Scroll {
    let axis = |axis: Axis| {
        let pixels = event.amount(axis)?;
        // A discrete wheel step counts 120 units a step; the value is a whole number of them.
        #[allow(clippy::cast_possible_truncation)]
        let steps = event.amount_v120(axis).map(|steps| steps.round() as i32);
        // Smithay states a stop as an axis that was named with no value and no steps.
        let stop = pixels == 0.0 && steps.is_none();
        Some(ScrollAxis { pixels, steps, stop })
    };
    Scroll {
        source: event.source(),
        horizontal: axis(Axis::Horizontal),
        vertical: axis(Axis::Vertical),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: OutputId = OutputId::new(0);
    const RIGHT: OutputId = OutputId::new(1);

    fn two_outputs() -> Vec<(OutputId, Rect)> {
        vec![
            (
                LEFT,
                Rect {
                    x: 0,
                    y: 0,
                    width: 1000,
                    height: 500,
                },
            ),
            (
                RIGHT,
                Rect {
                    x: 1000,
                    y: 0,
                    width: 1000,
                    height: 500,
                },
            ),
        ]
    }

    #[test]
    fn a_fraction_nudge_adds_up_to_a_pixel() {
        let (dx, _, kept) = whole_pixels((0.0, 0.0), (0.4, 0.0));
        assert_eq!(dx, 0);
        let (dx, _, kept) = whole_pixels(kept, (0.4, 0.0));
        assert_eq!(dx, 0);
        let (dx, _, kept) = whole_pixels(kept, (0.4, 0.0));
        assert_eq!(dx, 1);
        assert!((kept.0 - 0.2).abs() < 1e-9);
    }

    #[test]
    fn a_negative_nudge_rounds_toward_zero() {
        let (dx, dy, kept) = whole_pixels((0.0, 0.0), (-1.5, -0.5));
        assert_eq!((dx, dy), (-1, 0));
        assert!((kept.0 + 0.5).abs() < 1e-9 && (kept.1 + 0.5).abs() < 1e-9);
    }

    #[test]
    fn the_far_corner_of_the_layout_is_its_last_pixel() {
        let (output, at) = place(&two_outputs(), None, 1.0, 1.0).expect("a layout");
        assert_eq!(output, RIGHT);
        assert_eq!(at, Point { x: 999, y: 499 });
    }

    #[test]
    fn the_middle_of_two_outputs_is_on_the_seam() {
        let (output, _) = place(&two_outputs(), None, 0.25, 0.0).expect("a layout");
        assert_eq!(output, LEFT);
        let (output, at) = place(&two_outputs(), None, 0.75, 0.0).expect("a layout");
        assert_eq!(output, RIGHT);
        assert_eq!(at.x, 500);
    }

    #[test]
    fn a_named_output_takes_the_whole_fraction() {
        let (output, at) = place(&two_outputs(), Some(RIGHT), 0.0, 1.0).expect("a layout");
        assert_eq!(output, RIGHT);
        assert_eq!(at, Point { x: 0, y: 499 });
    }

    #[test]
    fn a_point_in_a_gap_goes_to_the_nearest_output() {
        let layout = vec![
            (
                LEFT,
                Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
            ),
            (
                RIGHT,
                Rect {
                    x: 100,
                    y: 100,
                    width: 100,
                    height: 100,
                },
            ),
        ];
        // The top right corner of the box holds no output.
        let (output, at) = place(&layout, None, 1.0, 0.4).expect("a layout");
        assert_eq!(output, RIGHT);
        assert_eq!(at, Point { x: 99, y: 0 });
    }

    #[test]
    fn no_outputs_place_nothing() {
        assert_eq!(place(&[], None, 0.5, 0.5), None);
    }
}
