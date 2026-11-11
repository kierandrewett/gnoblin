//! The OpenXR controller as seat input.
//!
//! Each frame the bridge casts the runtime's pointing ray against the planned layers and reads the controller's
//! buttons and stick. [`Pointer`] turns that into owner `input` ops on the `xr` seat:
//!
//! - **Motion** over the window under the ray, and a **leave** when the ray stops hitting it.
//! - **Buttons**: select is `BTN_LEFT` and secondary is `BTN_RIGHT` (the thresholds are in `xr_input`). A press goes to
//!   the window under the ray.
//! - **Scroll**: the stick, as axis amounts over time, to the window under the ray.
//!
//! A press is an implicit grab, as on a Wayland pointer. While a button is held, every op goes to the window that
//! was under the ray at the press, however the ray moves: no leave, no motion into another window. The coordinates
//! keep following the ray, projected onto the grabbed window's plane, so a drag that leaves the window's rectangle
//! reports pixels outside it. The grab ends on the release of the last held button, and the pointer then enters
//! whatever the ray is over.
//!
//! A press that lands on no window is not delivered, and neither is its release, so a window never sees a release it
//! has no press for.

use xr_input::{Controller, ControllerState, PointerEvent};
use xr_scene::{Layer, NodeId, Ray, pick, project};
use xr_tree::{PointerInput, TreeClient, TreeError};

/// The seat the OpenXR pointer uses. The extension creates it on the first frame of the session, unless the snapshot
/// already holds a seat named `xr`, which the pointer reuses.
pub const SEAT_NAME: &str = "xr";

/// The window a held button grabbed.
#[derive(Debug)]
struct Grab {
    /// The View node whose layer's plane the ray is projected onto.
    view: NodeId,
    /// The window the View shows: where every op goes until the grab ends.
    window: NodeId,
    /// The buttons whose press the owner accepted and whose release has not come, as evdev codes.
    buttons: Vec<u32>,
}

/// Where the ray points this frame.
#[derive(Debug, Clone, Copy)]
struct Target {
    view: NodeId,
    window: NodeId,
    x: f64,
    y: f64,
}

/// What the pointer last told the owner. Motion is sent only when the point changes.
#[derive(Debug, Default)]
pub struct Pointer {
    seat: Option<NodeId>,
    controller: Controller,
    /// The display time of the previous frame, for the scroll step.
    time: Option<i64>,
    /// The window the owner last saw the pointer over.
    hovered: Option<NodeId>,
    /// The last point sent for `hovered`.
    last: Option<(f64, f64)>,
    /// The point the ray is at in the window it targets, as of the latest frame that located one.
    at: (f64, f64),
    grab: Option<Grab>,
    /// Set once the owner's refusal of the seat has been logged, so a refusal that repeats every frame logs once.
    refusal_logged: bool,
}

impl Pointer {
    /// Applies one frame of the controller: the ray casts against `layers`, then the buttons and scroll of `state`
    /// are delivered. `time` is the frame's display time in nanoseconds; the scroll step is the time since the
    /// previous call.
    ///
    /// Without a grab, a missing ray or a miss sends a leave for the window last hovered, a hit over a different
    /// window leaves the old one and moves onto the new one, and a hit over the same window sends motion only when the
    /// pixel changes. With a grab, see the module documentation.
    ///
    /// An op the owner refuses is logged and left undelivered, and the rest of the frame carries on: a refused press
    /// starts no grab, a refused release still ends one, and a refused motion is sent again at the next change.
    ///
    /// # Errors
    ///
    /// The socket fails or closes. A refusal is not an error.
    pub fn update(
        &mut self,
        tree: &mut TreeClient,
        layers: &[Layer],
        state: &ControllerState,
        time: i64,
    ) -> Result<(), TreeError> {
        // The seat comes with the session, not with the first hit: the stock shell builds the room, and so the Views
        // a ray could hit, only once the seat exists.
        self.ensure_seat(tree)?;
        let dt = self.time.map_or(0.0, |previous| seconds(time.saturating_sub(previous)));
        self.time = Some(time);
        let events = self.controller.update(state, dt);

        let target = self.locate(layers, state.ray);
        if let Some(target) = target {
            self.at = (target.x, target.y);
        }
        self.move_to(tree, target)?;

        let was_grabbed = self.grab.is_some();
        for event in events {
            match event {
                PointerEvent::Button { button, pressed: true } => self.press(tree, target, button)?,
                PointerEvent::Button { button, pressed: false } => self.release(tree, button)?,
                PointerEvent::Scroll { dx, dy } => self.scroll(tree, target, dx, dy)?,
            }
        }
        if was_grabbed && self.grab.is_none() {
            // The last button came up: the pointer is now free to enter whatever the ray is over.
            let target = self.locate(layers, state.ray);
            self.move_to(tree, target)?;
        }
        Ok(())
    }

    /// Where `ray` points: on the grabbed window's plane while a button is held, else on the layer it hits.
    fn locate(&self, layers: &[Layer], ray: Option<Ray>) -> Option<Target> {
        let ray = ray?;
        let hit = match &self.grab {
            Some(grab) => project(layers, grab.view, &ray)?,
            None => pick(layers, &ray)?,
        };
        Some(Target {
            view: hit.id,
            window: hit.source,
            x: hit.u,
            y: hit.v,
        })
    }

    /// Moves the pointer to `target`: motion over the window, a leave for the window left. While a grab holds the
    /// pointer on its window, only motion within it is sent, and a frame that locates nothing sends nothing.
    fn move_to(&mut self, tree: &mut TreeClient, target: Option<Target>) -> Result<(), TreeError> {
        if let Some(grab) = &self.grab {
            let window = grab.window;
            if let (Some(target), Some(seat)) = (target, self.seat) {
                self.motion(tree, seat, window, (target.x, target.y))?;
            }
            return Ok(());
        }
        let Some(target) = target else {
            return self.leave(tree);
        };
        let Some(seat) = self.ensure_seat(tree)? else {
            return Ok(());
        };
        if self.hovered != Some(target.window) {
            self.leave(tree)?;
            self.hovered = Some(target.window);
            self.last = None;
        }
        self.motion(tree, seat, target.window, (target.x, target.y))
    }

    /// Sends motion to `window` when `point` is not the one last sent.
    fn motion(
        &mut self,
        tree: &mut TreeClient,
        seat: NodeId,
        window: NodeId,
        point: (f64, f64),
    ) -> Result<(), TreeError> {
        if self.last == Some(point) {
            return Ok(());
        }
        let input = PointerInput::Motion {
            seat,
            node: window,
            x: point.0,
            y: point.1,
        };
        if send(tree, &input)? {
            self.last = Some(point);
        }
        Ok(())
    }

    /// A press: starts a grab on the window under the ray, or joins the grab already held. A press over no window is
    /// dropped.
    fn press(&mut self, tree: &mut TreeClient, target: Option<Target>, button: u32) -> Result<(), TreeError> {
        let Some(seat) = self.seat else {
            return Ok(());
        };
        if let Some(grab) = &self.grab {
            let window = grab.window;
            let input = self.button(seat, window, button, true);
            if send(tree, &input)?
                && let Some(grab) = &mut self.grab
            {
                grab.buttons.push(button);
            }
            return Ok(());
        }
        let Some(target) = target.filter(|target| self.hovered == Some(target.window)) else {
            return Ok(());
        };
        let input = self.button(seat, target.window, button, true);
        if send(tree, &input)? {
            self.grab = Some(Grab {
                view: target.view,
                window: target.window,
                buttons: vec![button],
            });
        }
        Ok(())
    }

    /// A release: goes to the grabbed window if the owner took the press, and ends the grab with its last button.
    fn release(&mut self, tree: &mut TreeClient, button: u32) -> Result<(), TreeError> {
        let Some(seat) = self.seat else {
            return Ok(());
        };
        let Some(grab) = &mut self.grab else {
            return Ok(());
        };
        let Some(index) = grab.buttons.iter().position(|held| *held == button) else {
            return Ok(());
        };
        grab.buttons.remove(index);
        let window = grab.window;
        if grab.buttons.is_empty() {
            self.grab = None;
        }
        let input = self.button(seat, window, button, false);
        send(tree, &input).map(|_| ())
    }

    /// A scroll: to the grabbed window, else the window under the ray. Over no window it is dropped.
    fn scroll(&mut self, tree: &mut TreeClient, target: Option<Target>, dx: f64, dy: f64) -> Result<(), TreeError> {
        let Some(seat) = self.seat else {
            return Ok(());
        };
        let window = match (&self.grab, target) {
            (Some(grab), _) => grab.window,
            (None, Some(target)) if self.hovered == Some(target.window) => target.window,
            (None, _) => return Ok(()),
        };
        let input = PointerInput::Scroll {
            seat,
            node: window,
            x: self.at.0,
            y: self.at.1,
            dx,
            dy,
        };
        send(tree, &input).map(|_| ())
    }

    fn button(&self, seat: NodeId, window: NodeId, button: u32, pressed: bool) -> PointerInput {
        PointerInput::Button {
            seat,
            node: window,
            x: self.at.0,
            y: self.at.1,
            button: u64::from(button),
            pressed,
        }
    }

    /// Sends a leave for the hovered window, if there is one, and forgets it.
    fn leave(&mut self, tree: &mut TreeClient) -> Result<(), TreeError> {
        if let (Some(seat), Some(node)) = (self.seat, self.hovered.take()) {
            send(tree, &PointerInput::Leave { seat, node })?;
        }
        self.last = None;
        Ok(())
    }

    /// The pointer seat, found by name or created on first use. `None` when the owner refused to create it: the frame
    /// has no input, and the next one tries again.
    fn ensure_seat(&mut self, tree: &mut TreeClient) -> Result<Option<NodeId>, TreeError> {
        if let Some(seat) = self.seat {
            return Ok(Some(seat));
        }
        let seat = match tree.seat_named(SEAT_NAME) {
            Some(seat) => seat,
            None => match tree.create_seat(SEAT_NAME) {
                Ok(seat) => seat,
                Err(TreeError::Refused { member, code }) => {
                    if !self.refusal_logged {
                        eprintln!("gnoblin-xr: the owner refused the {SEAT_NAME} seat ({member}: {code})");
                        self.refusal_logged = true;
                    }
                    return Ok(None);
                }
                Err(error) => return Err(error),
            },
        };
        self.seat = Some(seat);
        Ok(Some(seat))
    }
}

/// Sends one input. `Ok(true)` when the owner applied it; `Ok(false)` when it refused, after logging the refusal.
fn send(tree: &mut TreeClient, input: &PointerInput) -> Result<bool, TreeError> {
    match tree.send_input(input) {
        Ok(()) => Ok(true),
        Err(TreeError::Refused { member, code }) => {
            eprintln!("gnoblin-xr: the owner refused pointer input {input:?} ({member}: {code})");
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Nanoseconds as seconds, as a scroll step.
fn seconds(nanoseconds: i64) -> f32 {
    (nanoseconds as f64 * 1e-9) as f32
}
