//! The frame-callback and presentation-feedback clients the frame tests share: a callback answered adds one to the count its
//! user data holds, and feedback that says a frame was presented adds the time to the list its user data holds.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use wayland_client::protocol::wl_callback;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};

use super::App;

/// How many frame callbacks a client has been answered.
pub type Answers = Arc<AtomicU32>;

/// The times, in milliseconds on the compositor's monotonic clock, a client's feedback said its frames were presented.
pub type Presented = Arc<Mutex<Vec<u64>>>;

impl Dispatch<wl_callback::WlCallback, Answers> for App {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        answers: &Answers,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            answers.fetch_add(1, Ordering::SeqCst);
        }
    }
}

wayland_client::delegate_noop!(App: ignore wp_presentation::WpPresentation);

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, Presented> for App {
    fn event(
        _: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        presented: &Presented,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation_feedback::Event::Presented {
            tv_sec_hi,
            tv_sec_lo,
            tv_nsec,
            ..
        } = event
        {
            let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
            presented
                .lock()
                .expect("the lock is not poisoned")
                .push(seconds * 1000 + u64::from(tv_nsec) / 1_000_000);
        }
    }
}
