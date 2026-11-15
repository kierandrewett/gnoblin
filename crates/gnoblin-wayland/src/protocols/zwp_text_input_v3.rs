//! XML protocol `text_input_unstable_v3` (`<protocol name="text_input_unstable_v3">`); interfaces served: `zwp_text_input_manager_v3`.
//!
//! `zwp_text_input_manager_v3` and `zwp_text_input_v3` (XML `text-input-unstable-v3`).
//!
//! A client that wants to type in a language an input method serves creates a
//! `zwp_text_input_v3` and enables it while its surface has focus. Smithay carries the
//! conversation through `TextInputHandle`; this module owns the two things a compositor
//! must decide for it:
//!
//! - **Focus.** `enter` and `leave` are sent when the keyboard focus moves, because a text
//!   input that is not told it has focus never asks to be enabled.
//! - **The initial `enter`.** A text input created on a surface that already has focus is
//!   announced by [`announce_new_text_inputs`].
//! - **An input method that arrives late.** Smithay discards a text input's requests while no
//!   input method is bound, so an input method that starts (or restarts) after the application
//!   enabled its text field would never hear of it. [`hold`] keeps those requests and
//!   [`replay`] gives them to Smithay once an input method binds.
//!
//! # Failure modes
//!
//! 1. **A text input is never told it has focus.** Focus moves set the handle and send
//!    `enter`, so a client's `enable` is accepted rather than discarded:
//!    `text_input_enter_and_commit_round_trip` in `tests/protocols.rs`.
//! 2. **A committed string never reaches the client.** An input method's `commit_string`
//!    goes to the active text input of the focused surface, and only there: the same test
//!    proves the round trip end to end.
//! 3. **An enabled text field is forgotten.** An application's `enable` and `commit` can reach
//!    the compositor before the input method's `get_input_method`, because the two connections
//!    are served as they arrive: `a_text_field_enabled_before_the_input_method_binds_is_activated`.
//!    The input method is activated with the state the application committed, and the
//!    application's serial still counts every commit it made, so the input method's `commit`
//!    is accepted.
//! 4. **A text field outlives its focus.** `leave` resets a text input, so what was held for it
//!    is dropped when the keyboard focus moves (`set_keyboard_focus`), and a field enabled on a
//!    window that has since lost the keyboard is not activated for the input method:
//!    `a_text_field_enabled_before_the_input_method_binds_is_not_activated_after_it_lost_focus`.
//! 5. **The held state grows without bound.** With no input method an application can send its
//!    surrounding text at every keystroke, so only the newest request of each kind is kept.

use std::collections::HashSet;

use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    ContentHint, ContentPurpose, Request, ZwpTextInputV3,
};
use smithay::reexports::wayland_server::{DataInit, DisplayHandle, Resource};
use smithay::wayland::Dispatch2;
use smithay::wayland::input_method::InputMethodSeat;
use smithay::wayland::text_input::{TextInputActivation, TextInputManagerState, TextInputUserData};

use crate::State;

use crate::log;

/// Creates the `zwp_text_input_manager_v3` global.
pub(crate) fn text_input_global(display: &DisplayHandle) -> TextInputManagerState {
    TextInputManagerState::new::<State>(display)
}

/// Tells a `zwp_text_input_v3` it has focus when it was created on a surface that already
/// had it.
///
/// `enter` normally follows the keyboard focus, and Smithay sends it for a newly created
/// text input only when an input method already exists. An ordinary session starts its
/// input method after the first window is focused, so a text input created then would wait
/// for a focus change that never comes and its `enable` would never be answered. This is
/// run after every dispatch, which is the only place a new text input can appear, and it
/// remembers what it told so an object hears `enter` once.
pub(crate) fn announce_new_text_inputs(state: &mut State) {
    use smithay::wayland::text_input::{TextInputHandle, TextInputSeat};
    let handle: TextInputHandle = state.seat.text_input().clone();
    let announced = std::mem::take(&mut state.announced_text_inputs);
    let mut fresh = Vec::new();
    let mut current = HashSet::new();
    handle.with_focused_text_input(|input, surface| {
        let id = input.id();
        if !announced.contains(&id) {
            fresh.push((input.clone(), surface.clone()));
        }
        current.insert(id);
    });
    // Outside the handle's lock: writing to a client is the client's own lock, and taking
    // it while holding this one is the shape of a dispatch deadlock.
    for (input, surface) in fresh {
        input.enter(&surface);
    }
    state.announced_text_inputs = current;
}

impl TextInputActivation for State {
    fn activated(&mut self, content_type: Option<(ContentHint, ContentPurpose)>) {
        log::debug!(area = "text-input", hint = ?content_type.map(|(hint, _)| hint),
            "a client enabled its text input"
        );
        // A client enables a text input only after it was told it has focus, and the sweep
        // after the dispatch that created it is what told it.
        announce_new_text_inputs(self);
    }

    fn deactivated(&mut self) {
        log::debug!(area = "text-input", "the active text input was disabled");
    }
}

/// What a text input asked while no input method was bound, as far as an input method needs it.
pub(crate) struct HeldTextInput {
    resource: ZwpTextInputV3,
    /// How many commits the client made. Smithay numbers the `done` events it answers a text input with by the
    /// commits it saw, so a replay makes as many.
    commits: u32,
    /// The newest request of each kind as of the last commit.
    committed: RequestSlots,
    /// The newest request of each kind since the last commit, which no commit has applied yet.
    pending: RequestSlots,
}

/// The newest request of each kind that a commit applies, in the order Smithay forwards them to an input method:
/// enable or disable, surrounding text, change cause, content type and cursor rectangle.
#[derive(Default)]
struct RequestSlots([Option<Request>; 5]);

impl RequestSlots {
    /// The slot a request belongs to, or `None` for one that is not state (`commit`, `destroy`).
    fn index(request: &Request) -> Option<usize> {
        Some(match request {
            Request::Enable | Request::Disable => 0,
            Request::SetSurroundingText { .. } => 1,
            Request::SetTextChangeCause { .. } => 2,
            Request::SetContentType { .. } => 3,
            Request::SetCursorRectangle { .. } => 4,
            _ => return None,
        })
    }

    /// Whether the text input is enabled as of these requests.
    fn enabled(&self) -> bool {
        matches!(self.0[0], Some(Request::Enable))
    }

    /// Takes `newer`'s request wherever it has one: the later request of a kind wins.
    fn overlay(&mut self, newer: RequestSlots) {
        for (old, new) in self.0.iter_mut().zip(newer.0) {
            if new.is_some() {
                *old = new;
            }
        }
    }

    fn into_requests(self) -> impl Iterator<Item = Request> {
        self.0.into_iter().flatten()
    }
}

/// Keeps a text input's request that Smithay would discard because no input method is bound.
///
/// The request comes back when it is not kept, so the caller passes it on to Smithay as before, and `None` means it was
/// kept for [`replay`]. A request is kept only when Smithay would have discarded it for want of an input method and not
/// for any other reason: its client has the text-input focus. A `destroy` is never kept, and drops what was.
pub(crate) fn hold(state: &mut State, input: &ZwpTextInputV3, request: Request) -> Option<Request> {
    use smithay::wayland::text_input::TextInputSeat;
    // These are the two conditions under which Smithay discards a text input's request for want of an input method.
    if state.seat.input_method().has_instance() || state.seat.text_input().compositor_input_method() {
        return Some(request);
    }
    if matches!(request, Request::Destroy) {
        state.held_text_inputs.remove(&input.id());
        return Some(request);
    }
    let focused = state
        .seat
        .text_input()
        .focus()
        .is_some_and(|focus| focus.id().same_client_as(&input.id()));
    if !focused {
        return Some(request);
    }
    state.held_text_inputs.retain(|_, held| held.resource.is_alive());
    let held = state
        .held_text_inputs
        .entry(input.id())
        .or_insert_with(|| HeldTextInput {
            resource: input.clone(),
            commits: 0,
            committed: RequestSlots::default(),
            pending: RequestSlots::default(),
        });
    if matches!(request, Request::Commit) {
        held.commits = held.commits.saturating_add(1);
        let newer = std::mem::take(&mut held.pending);
        held.committed.overlay(newer);
        return None;
    }
    match RequestSlots::index(&request) {
        Some(slot) => {
            held.pending.0[slot] = Some(request);
            None
        }
        None => Some(request),
    }
}

/// Ends the conversation of a text input that is destroyed while it is the active one.
///
/// Smithay deactivates the input method when a destroyed text input was the focused client's last, so a client with a
/// second text input that destroys the enabled one leaves its input method activated for a field that is gone. A
/// `disable` and a `commit` through Smithay's own path make the same deactivation the client would have asked for.
pub(crate) fn end_if_active(
    state: &mut State,
    input: &ZwpTextInputV3,
    dhandle: &DisplayHandle,
    data_init: &mut DataInit<'_, State>,
) {
    use smithay::wayland::text_input::TextInputSeat;
    let mut active = false;
    state
        .seat
        .text_input()
        .with_active_text_input(|held, _| active |= held == input);
    if !active {
        return;
    }
    let (Some(client), Some(data)) = (input.client(), input.data::<TextInputUserData>()) else {
        return;
    };
    data.request(state, &client, input, Request::Disable, dhandle, data_init);
    data.request(state, &client, input, Request::Commit, dhandle, data_init);
}

/// Gives what [`hold`] kept to Smithay once an input method is bound, so the input method is activated and is sent
/// the committed state as if it had been bound all along.
///
/// Smithay never saw the commits, so it makes them now: all but the last carry nothing and only number the `done`
/// events, then the last carries the state. A text input that was not enabled as of its last commit has nothing to
/// tell an input method, and only has its serial brought up to its commit count. What was asked since the last
/// commit follows as pending state.
pub(crate) fn replay(state: &mut State, dhandle: &DisplayHandle, data_init: &mut DataInit<'_, State>) {
    if state.held_text_inputs.is_empty() || !state.seat.input_method().has_instance() {
        return;
    }
    for (_, held) in std::mem::take(&mut state.held_text_inputs) {
        let HeldTextInput {
            resource,
            commits,
            committed,
            pending,
        } = held;
        let (Some(client), Some(data)) = (resource.client(), resource.data::<TextInputUserData>()) else {
            continue;
        };
        let mut feed = |state: &mut State, request: Request| {
            data.request(state, &client, &resource, request, dhandle, data_init);
        };
        if committed.enabled() {
            for _ in 1..commits {
                feed(state, Request::Commit);
            }
            for request in committed.into_requests() {
                feed(state, request);
            }
            feed(state, Request::Commit);
        } else {
            for _ in 0..commits {
                feed(state, Request::Commit);
            }
        }
        for request in pending.into_requests() {
            feed(state, request);
        }
    }
}
