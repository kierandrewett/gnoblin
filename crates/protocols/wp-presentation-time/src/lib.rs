//! XML protocol `presentation_time` (`<protocol name="presentation_time">`); interfaces served: `wp_presentation`.
//!
//! `wp_presentation`: a client is told when its frame actually reached the screen.
//!
//! The protocol is a measurement, not a request: a client that wants to pace itself asks
//! for feedback on a surface, and the compositor answers with `presented` **after** the
//! frame it asked about was shown, carrying the time the frame reached the output and the
//! output's refresh rate. A client that instead used its own timer would drift against a
//! refresh rate it cannot see.
//!
//! ## What makes the timestamp real
//!
//! The reply is sent from the frame hook, after the backend drew the output and presented
//! it, and the timestamp is read at that moment from the same clock the protocol names:
//! `CLOCK_MONOTONIC` is the id sent in `wp_presentation.clock_id`, and Smithay's
//! [`Clock<Monotonic>`] reads exactly that clock. So a feedback's time is a reading of
//! the compositor's monotonic clock taken once the frame was drawn, which is what a
//! client needs to compare with its own `clock_gettime(CLOCK_MONOTONIC)`.
//!
//! ## What this slice does not do
//!
//! A frame is presented synchronously on a headless output, so the frame the client asked
//! about and the frame that was drawn are the same one. On a real output the reply belongs
//! after the page flip, and the place for it is the backend's present path — the same call
//! site, with `Present::Complete` instead of `Present::Nothing` as the trigger. The
//! feedback's timestamp is already taken from the present, so that change moves the call
//! later, not the meaning of the number.
//!
//! # Failure modes
//!
//! 1. **A feedback is never answered.** Every request whose surface was in the frame just
//!    presented is answered in that iteration, so a client waiting for its first
//!    `presented` gets one: `presentation_feedback_reports_a_present_timestamp`.
//! 2. **A feedback for a surface nobody drew is answered.** Only the surfaces the frame
//!    drew are answered, so a feedback asked for by a hidden window stays pending until it
//!    is shown, which is the protocol's rule (a discarded frame is not presented).
//! 3. **A dead client's feedback is answered.** Smithay's `presented` looks the client up
//!    from the surface and does nothing when it is gone, so a closed connection cannot be
//!    written to.

use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::{wp_presentation, wp_presentation_feedback};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch, Resource};
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::GlobalData;
use smithay::wayland::compositor::with_states;
use smithay::wayland::presentation::{PresentationData, PresentationFeedbackCachedState, PresentationState, Refresh};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-presentation-time",
    interfaces: &[ServedInterface::new("wp_presentation", 2)],
    privilege: Privilege::Core,
};

/// The clock every timestamp this module sends is read from: `CLOCK_MONOTONIC`.
const CLOCK_MONOTONIC: u32 = 1;

/// The feedback bookkeeping the session carries between frames.
#[derive(Debug, Default)]
pub struct WpPresentationTimeTables {
    /// The sequence number of the last frame presented, which feedback reports.
    sequence: u64,
}

impl WpPresentationTimeTables {
    /// The sequence number the next frame will carry.
    pub fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }
}

/// What the protocol needs of the compositor: the tables it keeps between frames.
pub trait Host {
    /// The session's feedback tables.
    fn presentation_tables(&mut self) -> &mut WpPresentationTimeTables;
}

/// Creates the `wp_presentation` global.
pub fn register<D>(registrar: &Registrar<'_>) -> PresentationState
where
    D: GlobalDispatch<wp_presentation::WpPresentation, PresentationData>
        + Dispatch<wp_presentation::WpPresentation, PresentationData>
        + Dispatch<wp_presentation_feedback::WpPresentationFeedback, GlobalData>
        + 'static,
{
    PresentationState::new::<D>(registrar.display(), CLOCK_MONOTONIC)
}

/// Answers the feedback of the surfaces a frame drew.
pub trait PresentationExt: Host {
    /// Answers every feedback a client asked for on a surface this frame drew, and returns the present time it
    /// answered with, or `None` when no surface was drawn.
    ///
    /// The time is read after the frame reached the output; `output` is the one it reached, whose refresh rate the
    /// feedback carries. The same reading is what a window's `presented` publishes to the shell
    /// (`present_frame`), so the client and the shell are told the same instant.
    fn present_feedback(&mut self, output: &Output, surfaces: &[WlSurface], refresh: Refresh) -> Option<Duration> {
        if surfaces.is_empty() {
            return None;
        }
        let time: Duration = Clock::<Monotonic>::new().now().into();
        let sequence = self.presentation_tables().next_sequence();
        for surface in surfaces {
            let callbacks = with_states(surface, |states| {
                let mut cached = states.cached_state.get::<PresentationFeedbackCachedState>();
                cached.current().callbacks.split_off(0)
            });
            for callback in callbacks {
                log::debug!(area = "presentation", surface = ?surface.id(),
                    milliseconds = time.as_millis(),
                    sequence,
                    "a frame was presented"
                );
                callback.presented(output, time, refresh, sequence, wp_presentation_feedback::Kind::Vsync);
            }
        }
        Some(time)
    }
}

impl<T: Host> PresentationExt for T {}

/// The refresh rate of an output, as a presentation feedback states it.
#[must_use]
pub fn refresh_of(output: &Output) -> Refresh {
    match output.current_mode() {
        Some(mode) if mode.refresh > 0 => {
            // The mode states the rate in millihertz, so one period is a milliard
            // nanoseconds over the rate.
            Refresh::Fixed(Duration::from_nanos(
                1_000_000_000_000 / u64::try_from(mode.refresh).unwrap_or(1),
            ))
        }
        _ => Refresh::Unknown,
    }
}
