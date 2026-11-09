//! Turns libinput's touchpad gesture stream into the router's [`GestureEvent`]s.
//!
//! libinput reports a gesture already recognised: a begin with a finger count, updates
//! carrying the movement since the last one, and an end that says whether it was
//! cancelled. This adapter keeps the one gesture in flight, accumulates its movement into
//! the path the router matches, and emits the same begin, update, end and cancel events
//! the contact recogniser does. A hold has no [`GestureKind`], so it is not routed.
//!
//! Nothing here reads a device or a clock: the caller passes the tick and the sample.

use gnoblin_types::Tick;

use crate::gesture::{CancelReason, GestureEvent, GestureKind, GesturePath, MAX_PATH_POINTS};

/// Where a libinput gesture sample sits in its gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchpadPhase {
    /// The gesture was recognised; `fingers` is set.
    Begin,
    /// Movement since the previous sample.
    Update,
    /// The gesture ended normally.
    End,
    /// The gesture ended and libinput marked it cancelled.
    Cancel,
}

/// The kind of a libinput gesture sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchpadKind {
    /// Fingers moving together.
    Swipe,
    /// Fingers spreading or gathering.
    Pinch,
    /// Fingers held still. Not routed: there is no binding kind for it.
    Hold,
}

/// One libinput gesture sample, in the caller's own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchpadSample {
    /// Finger count, as stated by the begin.
    pub fingers: u8,
    /// Where the sample sits in its gesture.
    pub phase: TouchpadPhase,
    /// Swipe, pinch or hold.
    pub kind: TouchpadKind,
    /// Horizontal movement since the previous sample.
    pub dx: f64,
    /// Vertical movement since the previous sample.
    pub dy: f64,
    /// Cumulative scale since the begin; one for a swipe.
    pub scale: f64,
}

/// The gesture in flight, if any.
#[derive(Debug)]
struct ActiveTouchpad {
    kind: GestureKind,
    fingers: u8,
    begin: Tick,
    x: f64,
    y: f64,
    path: Vec<(f64, f64)>,
    travel: f64,
    scale: f64,
}

/// Keeps the touchpad's one gesture in flight and turns its samples into events.
#[derive(Debug, Default)]
pub struct TouchpadStream {
    active: Option<ActiveTouchpad>,
}

impl TouchpadStream {
    /// Feed one sample at `tick`, and return the router event it produces, if any.
    ///
    /// An update or end with no begin before it produces nothing: the begin was lost, so
    /// there is no gesture to continue.
    pub fn feed(&mut self, tick: Tick, sample: TouchpadSample) -> Option<GestureEvent> {
        let kind = match sample.kind {
            TouchpadKind::Hold => return None,
            TouchpadKind::Swipe => GestureKind::Swipe,
            TouchpadKind::Pinch => GestureKind::Pinch,
        };
        match sample.phase {
            TouchpadPhase::Begin => {
                self.active = Some(ActiveTouchpad {
                    kind,
                    fingers: sample.fingers,
                    begin: tick,
                    x: 0.0,
                    y: 0.0,
                    path: vec![(0.0, 0.0)],
                    travel: 0.0,
                    scale: 1.0,
                });
                Some(GestureEvent::Begin {
                    tick,
                    kind,
                    fingers: sample.fingers,
                    edge: None,
                })
            }
            TouchpadPhase::Update => {
                let active = self.active.as_mut()?;
                active.x += sample.dx;
                active.y += sample.dy;
                active.travel += sample.dx.hypot(sample.dy);
                active.scale = sample.scale;
                if active.path.len() < MAX_PATH_POINTS {
                    active.path.push((active.x, active.y));
                }
                Some(GestureEvent::Update {
                    tick,
                    kind: active.kind,
                    fingers: active.fingers,
                    edge: None,
                    dx: sample.dx,
                    dy: sample.dy,
                    scale: sample.scale,
                    angle_delta: 0.0,
                })
            }
            TouchpadPhase::End => {
                let active = self.active.take()?;
                Some(GestureEvent::End {
                    tick,
                    kind: active.kind,
                    fingers: active.fingers,
                    edge: None,
                    path: GesturePath::from_points(active.path),
                    travel: active.travel,
                    scale: active.scale,
                    duration: tick.since(active.begin),
                })
            }
            TouchpadPhase::Cancel => {
                let active = self.active.take()?;
                Some(GestureEvent::Cancel {
                    tick,
                    kind: active.kind,
                    fingers: active.fingers,
                    reason: CancelReason::Cancelled,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(phase: TouchpadPhase, kind: TouchpadKind, dx: f64, dy: f64, scale: f64) -> TouchpadSample {
        TouchpadSample {
            fingers: 3,
            phase,
            kind,
            dx,
            dy,
            scale,
        }
    }

    #[test]
    fn a_three_finger_swipe_reports_begin_updates_and_its_path_on_end() {
        let mut stream = TouchpadStream::default();
        let begin = stream.feed(
            Tick::new(10),
            sample(TouchpadPhase::Begin, TouchpadKind::Swipe, 0.0, 0.0, 1.0),
        );
        assert!(matches!(
            begin,
            Some(GestureEvent::Begin {
                kind: GestureKind::Swipe,
                fingers: 3,
                ..
            })
        ));
        stream.feed(
            Tick::new(11),
            sample(TouchpadPhase::Update, TouchpadKind::Swipe, 30.0, 0.0, 1.0),
        );
        stream.feed(
            Tick::new(12),
            sample(TouchpadPhase::Update, TouchpadKind::Swipe, 0.0, -40.0, 1.0),
        );
        let end = stream.feed(
            Tick::new(20),
            sample(TouchpadPhase::End, TouchpadKind::Swipe, 0.0, 0.0, 1.0),
        );
        match end {
            Some(GestureEvent::End {
                path,
                travel,
                duration,
                fingers,
                ..
            }) => {
                assert_eq!(path.points(), &[(0.0, 0.0), (30.0, 0.0), (30.0, -40.0)]);
                assert!((travel - 70.0).abs() < 1e-9);
                assert_eq!(duration, 10);
                assert_eq!(fingers, 3);
            }
            other => panic!("expected an end, got {other:?}"),
        }
    }

    #[test]
    fn a_cancelled_swipe_reports_a_cancel_and_no_end() {
        let mut stream = TouchpadStream::default();
        stream.feed(
            Tick::new(1),
            sample(TouchpadPhase::Begin, TouchpadKind::Swipe, 0.0, 0.0, 1.0),
        );
        let cancel = stream.feed(
            Tick::new(2),
            sample(TouchpadPhase::Cancel, TouchpadKind::Swipe, 0.0, 0.0, 1.0),
        );
        assert!(matches!(cancel, Some(GestureEvent::Cancel { .. })));
        // The gesture is gone: a late end has nothing to finish.
        assert!(
            stream
                .feed(
                    Tick::new(3),
                    sample(TouchpadPhase::End, TouchpadKind::Swipe, 0.0, 0.0, 1.0)
                )
                .is_none()
        );
    }

    #[test]
    fn a_hold_is_not_routed() {
        let mut stream = TouchpadStream::default();
        assert!(
            stream
                .feed(
                    Tick::new(1),
                    sample(TouchpadPhase::Begin, TouchpadKind::Hold, 0.0, 0.0, 1.0)
                )
                .is_none()
        );
    }

    #[test]
    fn an_update_without_a_begin_produces_nothing() {
        let mut stream = TouchpadStream::default();
        assert!(
            stream
                .feed(
                    Tick::new(1),
                    sample(TouchpadPhase::Update, TouchpadKind::Swipe, 5.0, 5.0, 1.0)
                )
                .is_none()
        );
    }

    #[test]
    fn a_pinch_end_carries_the_last_scale() {
        let mut stream = TouchpadStream::default();
        stream.feed(
            Tick::new(1),
            sample(TouchpadPhase::Begin, TouchpadKind::Pinch, 0.0, 0.0, 1.0),
        );
        stream.feed(
            Tick::new(2),
            sample(TouchpadPhase::Update, TouchpadKind::Pinch, 0.0, 0.0, 0.6),
        );
        match stream.feed(
            Tick::new(3),
            sample(TouchpadPhase::End, TouchpadKind::Pinch, 0.0, 0.0, 0.6),
        ) {
            Some(GestureEvent::End { kind, scale, .. }) => {
                assert_eq!(kind, GestureKind::Pinch);
                assert!((scale - 0.6).abs() < 1e-9);
            }
            other => panic!("expected a pinch end, got {other:?}"),
        }
    }
}
