//! The engine contract: a slot settles, keeps its last good answer while a run is
//! pending, and discards any result for a generation it no longer wants.
//!
//! The fake engine answers synchronously. That is a test double only. Production
//! backends are workers, and the slot must behave the same for both.

use std::collections::VecDeque;
use std::time::Instant;

use gnoblin_types::{Rect, WindowId};
use gnoblin_wm::layout::{
    Camera, Collected, Generation, Handled, Job, LayoutEngine, LayoutFailure, LayoutOutcome, LayoutWindow, Origin,
    Params, Placement, Reply, ReplyError, Request, RequestKind, Slot, Ticket,
};

fn win(id: u64) -> WindowId {
    WindowId::new(id)
}

fn params(ids: &[u64]) -> Params {
    let windows = ids
        .iter()
        .map(|&id| LayoutWindow {
            id: win(id),
            width: 100.0,
            height: 50.0,
        })
        .collect();
    Params::new(800.0, 600.0, windows).expect("valid params")
}

fn handled(placements: Vec<Placement>) -> Reply {
    Reply::Handled(Handled {
        placements,
        camera: Camera::Orthographic,
        effects: vec![],
    })
}

/// What the fake does on its next `collect`.
#[derive(Clone, Debug)]
enum Answer {
    /// Answer the generation asked for, with this result.
    Now(Result<Reply, LayoutFailure>),
    /// Answer an older generation, which the slot must discard.
    Stale(Result<Reply, LayoutFailure>),
    /// Nothing is ready before the deadline.
    Hang,
}

/// A synchronous, scripted engine. Each `collect` consumes one answer; when the
/// script is empty the fake hangs.
struct FakeEngine {
    script: VecDeque<Answer>,
}

impl LayoutEngine for FakeEngine {
    fn submit(&mut self, _job: Job, generation: Generation) -> Ticket {
        Ticket::new(generation)
    }

    fn collect(&mut self, generation: Generation, _deadline: Instant) -> LayoutOutcome {
        match self.script.pop_front().unwrap_or(Answer::Hang) {
            Answer::Now(result) => LayoutOutcome::Ready { generation, result },
            Answer::Stale(result) => LayoutOutcome::Ready {
                generation: Generation::FIRST,
                result,
            },
            Answer::Hang => LayoutOutcome::Pending,
        }
    }
}

fn slot(script: Vec<Answer>) -> Slot {
    Slot::new(Box::new(FakeEngine { script: script.into() }))
}

#[test]
fn an_idle_slot_reports_idle() {
    let mut slot = slot(vec![]);
    assert_eq!(slot.collect(Instant::now()), Collected::Idle);
}

#[test]
fn a_handled_reply_settles() {
    let mut slot = slot(vec![Answer::Now(Ok(handled(vec![Placement::rect(
        win(1),
        0.0,
        0.0,
        100.0,
        50.0,
    )])))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Settled(layout) = slot.collect(Instant::now()) else {
        panic!("expected a settled layout");
    };
    assert_eq!(layout.origin, Origin::Reply);
    assert_eq!(layout.placements.len(), 1);
}

#[test]
fn a_declined_reply_settles_on_the_fallback() {
    let mut slot = slot(vec![Answer::Now(Ok(Reply::Declined))]);
    slot.submit(Job::Arrange(params(&[1, 2])));
    let Collected::Settled(layout) = slot.collect(Instant::now()) else {
        panic!("expected the fallback");
    };
    assert_eq!(layout.origin, Origin::Fallback);
    assert_eq!(layout.placements.len(), 2);
}

#[test]
fn a_layout_error_is_reported_with_its_reason() {
    let mut slot = slot(vec![Answer::Now(Err(LayoutFailure::Error))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a failure");
    };
    assert_eq!(failure, LayoutFailure::Error);
    assert_eq!(failure.reason(), "error");
}

#[test]
fn a_timeout_is_reported_with_its_reason() {
    let mut slot = slot(vec![Answer::Now(Err(LayoutFailure::Timeout))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a failure");
    };
    assert_eq!(failure.reason(), "timeout");
}

#[test]
fn a_disabled_layout_is_reported_with_its_reason() {
    let mut slot = slot(vec![Answer::Now(Err(LayoutFailure::Disabled))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a failure");
    };
    assert_eq!(failure.reason(), "disabled");
}

#[test]
fn a_wedged_vm_is_reported_with_its_reason() {
    let mut slot = slot(vec![Answer::Now(Err(LayoutFailure::VmWedged))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a failure");
    };
    assert_eq!(failure.reason(), "vm-wedged");
}

#[test]
fn a_reply_for_an_unknown_window_is_refused() {
    let reply = handled(vec![Placement::rect(win(9), 0.0, 0.0, 1.0, 1.0)]);
    let mut slot = slot(vec![Answer::Now(Ok(reply))]);
    slot.submit(Job::Arrange(params(&[1])));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a refused reply");
    };
    assert_eq!(failure, LayoutFailure::InvalidReply(ReplyError::UnknownWindow(win(9))));
    assert_eq!(failure.reason(), "invalid-reply");
}

#[test]
fn a_hanging_engine_keeps_the_run_pending_then_settles_when_ready() {
    let mut slot = slot(vec![Answer::Hang, Answer::Now(Ok(Reply::Declined))]);
    slot.submit(Job::Arrange(params(&[1])));
    assert_eq!(slot.collect(Instant::now()), Collected::Pending);
    assert!(matches!(slot.collect(Instant::now()), Collected::Settled(_)));
    assert_eq!(slot.collect(Instant::now()), Collected::Idle);
}

#[test]
fn a_stale_generation_is_discarded_and_the_run_stays_pending() {
    let mut slot = slot(vec![
        Answer::Stale(Ok(Reply::Declined)),
        Answer::Now(Ok(Reply::Declined)),
    ]);
    slot.submit(Job::Arrange(params(&[1])));
    slot.submit(Job::Arrange(params(&[1, 2])));
    assert_eq!(slot.collect(Instant::now()), Collected::Pending);
    let Collected::Settled(layout) = slot.collect(Instant::now()) else {
        panic!("the current generation should settle");
    };
    assert_eq!(layout.placements.len(), 2, "the newer params were used");
}

#[test]
fn a_newer_submit_supersedes_the_run_in_flight() {
    let mut slot = slot(vec![Answer::Now(Ok(Reply::Declined))]);
    slot.submit(Job::Arrange(params(&[1])));
    slot.submit(Job::Arrange(params(&[1, 2])));
    let Collected::Settled(layout) = slot.collect(Instant::now()) else {
        panic!("expected a settled layout");
    };
    assert_eq!(layout.placements.len(), 2);
}

fn request(ids: &[u64], kind: RequestKind) -> Request {
    Request {
        params: params(ids),
        kind,
    }
}

#[test]
fn a_handled_request_settles_with_its_placements() {
    let reply = handled(vec![Placement::rect(win(1), 0.0, 0.0, 100.0, 50.0)]);
    let mut slot = slot(vec![Answer::Now(Ok(reply))]);
    slot.submit_request(request(&[1], RequestKind::Place { window: win(1) }));
    let Collected::Settled(layout) = slot.collect(Instant::now()) else {
        panic!("expected a settled request");
    };
    assert_eq!(layout.origin, Origin::Reply);
    assert_eq!(layout.placements.len(), 1);
}

#[test]
fn a_declined_request_is_reported_not_replaced_by_the_fallback() {
    let mut slot = slot(vec![Answer::Now(Ok(Reply::Declined))]);
    slot.submit_request(request(&[1, 2], RequestKind::Place { window: win(2) }));
    assert_eq!(slot.collect(Instant::now()), Collected::Declined);
}

#[test]
fn a_request_error_is_reported_with_its_reason() {
    let mut slot = slot(vec![Answer::Now(Err(LayoutFailure::Error))]);
    slot.submit_request(request(&[1], RequestKind::Place { window: win(1) }));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a failure");
    };
    assert_eq!(failure.reason(), "error");
}

#[test]
fn a_pending_request_waits_for_its_answer() {
    let mut slot = slot(vec![Answer::Hang, Answer::Now(Ok(Reply::Declined))]);
    slot.submit_request(request(&[1], RequestKind::Place { window: win(1) }));
    assert_eq!(slot.collect(Instant::now()), Collected::Pending);
    assert_eq!(slot.collect(Instant::now()), Collected::Declined);
}

#[test]
fn a_stale_request_answer_is_discarded() {
    let stale = handled(vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)]);
    let mut slot = slot(vec![Answer::Stale(Ok(stale)), Answer::Now(Ok(Reply::Declined))]);
    slot.submit_request(request(&[1], RequestKind::Place { window: win(1) }));
    slot.submit_request(request(
        &[1],
        RequestKind::Move {
            window: win(1),
            geometry: Rect::new(0, 0, 100, 50),
        },
    ));
    assert_eq!(slot.collect(Instant::now()), Collected::Pending);
    assert_eq!(slot.collect(Instant::now()), Collected::Declined);
}

#[test]
fn a_request_reply_for_an_unknown_window_is_refused() {
    let reply = handled(vec![Placement::rect(win(9), 0.0, 0.0, 1.0, 1.0)]);
    let mut slot = slot(vec![Answer::Now(Ok(reply))]);
    slot.submit_request(request(&[1], RequestKind::Place { window: win(1) }));
    let Collected::Failed(failure) = slot.collect(Instant::now()) else {
        panic!("expected a refused request reply");
    };
    assert_eq!(failure, LayoutFailure::InvalidReply(ReplyError::UnknownWindow(win(9))));
}
