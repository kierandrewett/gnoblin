//! Which holder has claimed a touchpad gesture.
//!
//! A shell claims one kind and finger count, such as a three-finger swipe. From the begin of a
//! matching gesture to its end, every event of it goes to that holder and to nothing else: the
//! configured bindings of [`GestureRouter`](crate::GestureRouter) never see it, and no client does.
//! A gesture nobody claims is [`ClaimRoute::Unclaimed`], and the caller routes it as it always did.
//!
//! The claim is decided at the begin and never re-decided, so a gesture cannot move to another
//! holder half way through, and a claim made while a gesture is in flight applies to the next one.
//! Three things end delivery early, and each leaves the rest of the gesture withheld rather than
//! handed to a binding: the holder unclaiming or going away, and the session locking. A lock that
//! comes up mid-gesture delivers one cancel to the holder and nothing after it, and a gesture that
//! begins while the session is locked is never claimed.
//!
//! Nothing here reads a clock or a device: the caller passes the recogniser's events and the lock
//! state, and a [`Holder`] is the caller's own number for whoever claimed.

use std::fmt;

use crate::gesture::{GestureEvent, GestureKind, GesturePhase};
use crate::gesture_router::GestureLock;

/// The fewest fingers a claim names, as the configured bindings.
pub const MIN_CLAIM_FINGERS: u8 = 2;

/// The most fingers a claim names, as the configured bindings.
pub const MAX_CLAIM_FINGERS: u8 = 5;

/// The caller's number for whoever holds a claim. It is only compared, never interpreted.
pub type Holder = u64;

/// One claim: the holder, the id it allocated, and the gesture it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GestureClaim {
    /// Who holds it.
    pub holder: Holder,
    /// The id the holder allocated, which names the claim in every delivery.
    pub id: i64,
    /// Swipe or pinch.
    pub kind: GestureKind,
    /// How many contacts the gesture has.
    pub fingers: u8,
}

/// One event of a claimed gesture, for its holder.
///
/// The movement is the gesture's so far, not the step since the last event, so a holder that
/// follows the fingers reads its position straight off the latest delivery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureDelivery {
    /// Who the event is for.
    pub holder: Holder,
    /// The claim's id.
    pub id: i64,
    /// Swipe or pinch.
    pub kind: GestureKind,
    /// How many contacts the gesture has.
    pub fingers: u8,
    /// Begin, update, end or cancel.
    pub phase: GesturePhase,
    /// Horizontal centroid movement since the begin, to the right positive.
    pub dx: f64,
    /// Vertical centroid movement since the begin, downward positive.
    pub dy: f64,
    /// Mean distance of the contacts from their centroid relative to the begin; one for a swipe.
    pub scale: f64,
    /// Rotation of the contacts since the begin in radians, clockwise positive; zero for a swipe.
    pub rotation: f64,
}

/// What the claim table did with a gesture event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClaimRoute {
    /// No claim takes this gesture: route it as an unclaimed gesture.
    Unclaimed,
    /// A claim takes it: this event is for its holder.
    Deliver(GestureDelivery),
    /// A claim took it, but its delivery has ended: the event goes nowhere, and not to a binding.
    Withheld,
}

/// Why a claim was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// The finger count is outside [`MIN_CLAIM_FINGERS`] to [`MAX_CLAIM_FINGERS`].
    Fingers {
        /// The refused count.
        fingers: i64,
    },
    /// Another claim already takes this kind and finger count.
    Taken {
        /// Who holds it.
        holder: Holder,
        /// Its id.
        id: i64,
    },
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fingers { fingers } => write!(
                formatter,
                "a gesture has {MIN_CLAIM_FINGERS} to {MAX_CLAIM_FINGERS} fingers, not {fingers}"
            ),
            Self::Taken { holder, id } => {
                write!(formatter, "claim {id} of holder {holder} already takes this gesture")
            }
        }
    }
}

impl std::error::Error for ClaimError {}

/// The gesture in flight, and whom its events still go to.
#[derive(Clone, Copy, Debug)]
struct Flight {
    /// The claim delivering it, or `None` once delivery has ended and the rest is withheld.
    live: Option<(Holder, i64)>,
    kind: GestureKind,
    fingers: u8,
    dx: f64,
    dy: f64,
    scale: f64,
    rotation: f64,
}

impl Flight {
    /// The event for the live holder, if delivery has not ended.
    fn delivery(&self, phase: GesturePhase) -> Option<GestureDelivery> {
        let (holder, id) = self.live?;
        Some(GestureDelivery {
            holder,
            id,
            kind: self.kind,
            fingers: self.fingers,
            phase,
            dx: self.dx,
            dy: self.dy,
            scale: self.scale,
            rotation: self.rotation,
        })
    }

    /// Delivers `phase`, or says the gesture's events are withheld.
    fn route(&self, phase: GesturePhase) -> ClaimRoute {
        self.delivery(phase).map_or(ClaimRoute::Withheld, ClaimRoute::Deliver)
    }
}

/// The claims, and the one gesture in flight.
#[derive(Clone, Debug, Default)]
pub struct GestureClaims {
    claims: Vec<GestureClaim>,
    flight: Option<Flight>,
}

impl GestureClaims {
    /// An empty table: every gesture is unclaimed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            claims: Vec::new(),
            flight: None,
        }
    }

    /// Takes a gesture for `holder` under `id`.
    ///
    /// A claim of an id the holder already has replaces it. A gesture another claim takes is
    /// refused, so two holders never compete for one movement.
    ///
    /// # Errors
    ///
    /// [`ClaimError::Fingers`] for a count outside the accepted range, and [`ClaimError::Taken`]
    /// when another claim takes the same kind and count.
    pub fn claim(&mut self, holder: Holder, id: i64, kind: GestureKind, fingers: i64) -> Result<(), ClaimError> {
        let fingers = u8::try_from(fingers)
            .ok()
            .filter(|count| (MIN_CLAIM_FINGERS..=MAX_CLAIM_FINGERS).contains(count))
            .ok_or(ClaimError::Fingers { fingers })?;
        if let Some(taken) = self
            .claims
            .iter()
            .find(|claim| claim.kind == kind && claim.fingers == fingers && (claim.holder, claim.id) != (holder, id))
        {
            return Err(ClaimError::Taken {
                holder: taken.holder,
                id: taken.id,
            });
        }
        self.unclaim(holder, id);
        self.claims.push(GestureClaim {
            holder,
            id,
            kind,
            fingers,
        });
        Ok(())
    }

    /// Gives a claim up. The gesture it was delivering, if any, is withheld to its end.
    /// Returns whether the holder had such a claim.
    pub fn unclaim(&mut self, holder: Holder, id: i64) -> bool {
        let before = self.claims.len();
        self.claims.retain(|claim| (claim.holder, claim.id) != (holder, id));
        self.withhold(|live| live == (holder, id));
        self.claims.len() != before
    }

    /// Drops every claim of the holders that went away. The gesture one of them was delivering is
    /// withheld to its end: its holder is gone, so nobody is left to cancel it for.
    /// Returns how many claims were dropped.
    pub fn release(&mut self, gone: &[Holder]) -> usize {
        let before = self.claims.len();
        self.claims.retain(|claim| !gone.contains(&claim.holder));
        self.withhold(|(holder, _)| gone.contains(&holder));
        before - self.claims.len()
    }

    /// The claims, in the order they were made.
    #[must_use]
    pub fn claims(&self) -> &[GestureClaim] {
        &self.claims
    }

    /// Whether a claimed gesture has begun and not ended.
    #[must_use]
    pub const fn in_flight(&self) -> bool {
        self.flight.is_some()
    }

    /// Whether the gesture in flight is still delivering to a holder.
    #[must_use]
    pub fn delivering(&self) -> bool {
        self.flight.is_some_and(|flight| flight.live.is_some())
    }

    /// Decides what a recognised gesture event means to the claims.
    pub fn observe(&mut self, event: &GestureEvent, context: GestureLock) -> ClaimRoute {
        match *event {
            GestureEvent::Begin { kind, fingers, .. } => {
                // A begin starts afresh: a gesture whose end was lost does not leak into it.
                self.flight = None;
                if context == GestureLock::Locked {
                    return ClaimRoute::Unclaimed;
                }
                let Some(claim) = self
                    .claims
                    .iter()
                    .find(|claim| claim.kind == kind && claim.fingers == fingers)
                else {
                    return ClaimRoute::Unclaimed;
                };
                let flight = Flight {
                    live: Some((claim.holder, claim.id)),
                    kind,
                    fingers,
                    dx: 0.0,
                    dy: 0.0,
                    scale: 1.0,
                    rotation: 0.0,
                };
                self.flight = Some(flight);
                flight.route(GesturePhase::Begin)
            }
            GestureEvent::Update {
                dx,
                dy,
                scale,
                angle_delta,
                ..
            } => {
                let Some(flight) = self.flight.as_mut() else {
                    return ClaimRoute::Unclaimed;
                };
                flight.dx += dx;
                flight.dy += dy;
                flight.scale = scale;
                flight.rotation += angle_delta;
                if context == GestureLock::Locked {
                    // The lock came up under the fingers: the holder is told once, then left alone.
                    let route = flight.route(GesturePhase::Cancel);
                    flight.live = None;
                    return route;
                }
                flight.route(GesturePhase::Update)
            }
            GestureEvent::End { scale, .. } => {
                let Some(mut flight) = self.flight.take() else {
                    return ClaimRoute::Unclaimed;
                };
                flight.scale = scale;
                if context == GestureLock::Locked {
                    return flight.route(GesturePhase::Cancel);
                }
                flight.route(GesturePhase::End)
            }
            GestureEvent::Cancel { .. } => self
                .flight
                .take()
                .map_or(ClaimRoute::Unclaimed, |flight| flight.route(GesturePhase::Cancel)),
        }
    }

    /// Ends delivery of the gesture in flight when `matches` says its holder is the one affected.
    fn withhold(&mut self, matches: impl Fn((Holder, i64)) -> bool) {
        if let Some(flight) = self.flight.as_mut()
            && flight.live.is_some_and(matches)
        {
            flight.live = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::{CancelReason, GesturePath};
    use gnoblin_types::Tick;

    const SHELL: Holder = 7;

    fn begin(kind: GestureKind, fingers: u8) -> GestureEvent {
        GestureEvent::Begin {
            tick: Tick::new(1),
            kind,
            fingers,
            edge: None,
        }
    }

    fn update(kind: GestureKind, fingers: u8, dx: f64, dy: f64, scale: f64, angle: f64) -> GestureEvent {
        GestureEvent::Update {
            tick: Tick::new(2),
            kind,
            fingers,
            edge: None,
            dx,
            dy,
            scale,
            angle_delta: angle,
        }
    }

    fn end(kind: GestureKind, fingers: u8, scale: f64) -> GestureEvent {
        GestureEvent::End {
            tick: Tick::new(3),
            kind,
            fingers,
            edge: None,
            path: GesturePath::default(),
            travel: 0.0,
            scale,
            duration: 2,
        }
    }

    fn cancel(kind: GestureKind, fingers: u8) -> GestureEvent {
        GestureEvent::Cancel {
            tick: Tick::new(3),
            kind,
            fingers,
            reason: CancelReason::Cancelled,
        }
    }

    fn claims_swipe3() -> GestureClaims {
        let mut claims = GestureClaims::new();
        claims
            .claim(SHELL, 1, GestureKind::Swipe, 3)
            .expect("the claim is accepted");
        claims
    }

    fn delivered(route: ClaimRoute) -> GestureDelivery {
        match route {
            ClaimRoute::Deliver(delivery) => delivery,
            other => panic!("expected a delivery, got {other:?}"),
        }
    }

    #[test]
    fn a_claimed_swipe_is_delivered_from_begin_to_end_with_its_movement_so_far() {
        let mut claims = claims_swipe3();
        let unlocked = GestureLock::Unlocked;
        let begun = delivered(claims.observe(&begin(GestureKind::Swipe, 3), unlocked));
        assert_eq!((begun.holder, begun.id, begun.phase), (SHELL, 1, GesturePhase::Begin));
        assert_eq!((begun.dx, begun.dy, begun.scale, begun.rotation), (0.0, 0.0, 1.0, 0.0));

        let first = delivered(claims.observe(&update(GestureKind::Swipe, 3, 10.0, -4.0, 1.0, 0.0), unlocked));
        let second = delivered(claims.observe(&update(GestureKind::Swipe, 3, 5.0, -1.0, 1.0, 0.0), unlocked));
        assert_eq!((first.phase, first.dx, first.dy), (GesturePhase::Update, 10.0, -4.0));
        assert_eq!(
            (second.dx, second.dy),
            (15.0, -5.0),
            "the movement is the total, not the step"
        );

        let ended = delivered(claims.observe(&end(GestureKind::Swipe, 3, 1.0), unlocked));
        assert_eq!((ended.phase, ended.dx, ended.dy), (GesturePhase::End, 15.0, -5.0));
        assert!(!claims.in_flight(), "the end finishes the gesture");
    }

    #[test]
    fn a_pinch_reports_its_scale_and_accumulates_its_rotation() {
        let mut claims = GestureClaims::new();
        claims.claim(SHELL, 2, GestureKind::Pinch, 4).expect("claim");
        let unlocked = GestureLock::Unlocked;
        claims.observe(&begin(GestureKind::Pinch, 4), unlocked);
        claims.observe(&update(GestureKind::Pinch, 4, 0.0, 0.0, 1.2, 0.1), unlocked);
        let latest = delivered(claims.observe(&update(GestureKind::Pinch, 4, 0.0, 0.0, 1.5, 0.25), unlocked));
        assert_eq!(latest.kind, GestureKind::Pinch);
        assert!((latest.scale - 1.5).abs() < 1e-9);
        assert!((latest.rotation - 0.35).abs() < 1e-9);
        let ended = delivered(claims.observe(&end(GestureKind::Pinch, 4, 1.6), unlocked));
        assert!((ended.scale - 1.6).abs() < 1e-9, "the end carries the final scale");
    }

    #[test]
    fn a_gesture_of_another_kind_or_finger_count_is_unclaimed() {
        let mut claims = claims_swipe3();
        let unlocked = GestureLock::Unlocked;
        assert_eq!(
            claims.observe(&begin(GestureKind::Swipe, 4), unlocked),
            ClaimRoute::Unclaimed
        );
        assert_eq!(
            claims.observe(&update(GestureKind::Swipe, 4, 9.0, 0.0, 1.0, 0.0), unlocked),
            ClaimRoute::Unclaimed,
            "the rest of an unclaimed gesture stays unclaimed"
        );
        assert_eq!(
            claims.observe(&begin(GestureKind::Pinch, 3), unlocked),
            ClaimRoute::Unclaimed
        );
        assert_eq!(
            claims.observe(&end(GestureKind::Pinch, 3, 1.0), unlocked),
            ClaimRoute::Unclaimed
        );
    }

    #[test]
    fn a_cancel_is_delivered_once_and_a_late_end_is_not() {
        let mut claims = claims_swipe3();
        let unlocked = GestureLock::Unlocked;
        claims.observe(&begin(GestureKind::Swipe, 3), unlocked);
        claims.observe(&update(GestureKind::Swipe, 3, 8.0, 0.0, 1.0, 0.0), unlocked);
        let cancelled = delivered(claims.observe(&cancel(GestureKind::Swipe, 3), unlocked));
        assert_eq!((cancelled.phase, cancelled.dx), (GesturePhase::Cancel, 8.0));
        assert_eq!(
            claims.observe(&end(GestureKind::Swipe, 3, 1.0), unlocked),
            ClaimRoute::Unclaimed,
            "a cancelled gesture has nothing left to end"
        );
    }

    #[test]
    fn a_gesture_that_begins_while_locked_is_never_claimed() {
        let mut claims = claims_swipe3();
        assert_eq!(
            claims.observe(&begin(GestureKind::Swipe, 3), GestureLock::Locked),
            ClaimRoute::Unclaimed
        );
        // The lock coming down does not hand the rest of it to the holder.
        assert_eq!(
            claims.observe(
                &update(GestureKind::Swipe, 3, 9.0, 0.0, 1.0, 0.0),
                GestureLock::Unlocked
            ),
            ClaimRoute::Unclaimed
        );
        assert!(!claims.in_flight());
    }

    #[test]
    fn a_lock_mid_gesture_cancels_it_once_and_withholds_the_rest() {
        let mut claims = claims_swipe3();
        claims.observe(&begin(GestureKind::Swipe, 3), GestureLock::Unlocked);
        claims.observe(
            &update(GestureKind::Swipe, 3, 6.0, 0.0, 1.0, 0.0),
            GestureLock::Unlocked,
        );
        let cancelled =
            delivered(claims.observe(&update(GestureKind::Swipe, 3, 6.0, 0.0, 1.0, 0.0), GestureLock::Locked));
        assert_eq!(cancelled.phase, GesturePhase::Cancel);
        assert_eq!(
            claims.observe(&update(GestureKind::Swipe, 3, 6.0, 0.0, 1.0, 0.0), GestureLock::Locked),
            ClaimRoute::Withheld
        );
        assert_eq!(
            claims.observe(&end(GestureKind::Swipe, 3, 1.0), GestureLock::Locked),
            ClaimRoute::Withheld,
            "an end under the lock reaches nobody, and not a binding either"
        );
        assert!(!claims.in_flight(), "the gesture is over");
    }

    #[test]
    fn a_lock_at_the_end_cancels_instead_of_ending() {
        let mut claims = claims_swipe3();
        claims.observe(&begin(GestureKind::Swipe, 3), GestureLock::Unlocked);
        let route = claims.observe(&end(GestureKind::Swipe, 3, 1.0), GestureLock::Locked);
        assert_eq!(delivered(route).phase, GesturePhase::Cancel);
    }

    #[test]
    fn unclaiming_mid_gesture_withholds_the_rest_and_frees_the_gesture() {
        let mut claims = claims_swipe3();
        let unlocked = GestureLock::Unlocked;
        claims.observe(&begin(GestureKind::Swipe, 3), unlocked);
        assert!(claims.unclaim(SHELL, 1));
        assert_eq!(
            claims.observe(&update(GestureKind::Swipe, 3, 3.0, 0.0, 1.0, 0.0), unlocked),
            ClaimRoute::Withheld
        );
        assert_eq!(
            claims.observe(&end(GestureKind::Swipe, 3, 1.0), unlocked),
            ClaimRoute::Withheld
        );
        assert_eq!(
            claims.observe(&begin(GestureKind::Swipe, 3), unlocked),
            ClaimRoute::Unclaimed,
            "the next gesture is nobody's again"
        );
        assert!(!claims.unclaim(SHELL, 1), "a claim is gone once, not twice");
    }

    #[test]
    fn a_holder_that_goes_away_takes_its_claims_and_its_gesture_with_it() {
        let mut claims = claims_swipe3();
        claims
            .claim(8, 1, GestureKind::Pinch, 4)
            .expect("another holder's claim");
        let unlocked = GestureLock::Unlocked;
        claims.observe(&begin(GestureKind::Swipe, 3), unlocked);
        assert_eq!(claims.release(&[SHELL]), 1);
        assert_eq!(
            claims.observe(&update(GestureKind::Swipe, 3, 3.0, 0.0, 1.0, 0.0), unlocked),
            ClaimRoute::Withheld,
            "no event of the abandoned gesture is delivered or routed to a binding"
        );
        assert_eq!(
            claims.observe(&cancel(GestureKind::Swipe, 3), unlocked),
            ClaimRoute::Withheld
        );
        assert!(!claims.in_flight());
        assert_eq!(claims.claims().len(), 1, "the other holder's claim stays");
    }

    #[test]
    fn a_release_of_another_holder_leaves_the_gesture_delivering() {
        let mut claims = claims_swipe3();
        claims.observe(&begin(GestureKind::Swipe, 3), GestureLock::Unlocked);
        assert_eq!(claims.release(&[99]), 0);
        assert!(claims.delivering());
    }

    #[test]
    fn a_second_claim_of_the_same_gesture_is_refused_and_names_the_holder() {
        let mut claims = claims_swipe3();
        assert_eq!(
            claims.claim(8, 1, GestureKind::Swipe, 3),
            Err(ClaimError::Taken { holder: SHELL, id: 1 })
        );
        assert_eq!(
            claims.claim(SHELL, 2, GestureKind::Swipe, 3),
            Err(ClaimError::Taken { holder: SHELL, id: 1 })
        );
        claims
            .claim(8, 1, GestureKind::Swipe, 4)
            .expect("another finger count is free");
    }

    #[test]
    fn a_claim_of_a_held_id_replaces_it() {
        let mut claims = claims_swipe3();
        claims
            .claim(SHELL, 1, GestureKind::Swipe, 4)
            .expect("the same id moves");
        assert_eq!(
            claims.claims(),
            &[GestureClaim {
                holder: SHELL,
                id: 1,
                kind: GestureKind::Swipe,
                fingers: 4,
            }]
        );
        assert_eq!(
            claims.observe(&begin(GestureKind::Swipe, 3), GestureLock::Unlocked),
            ClaimRoute::Unclaimed
        );
    }

    #[test]
    fn a_finger_count_outside_two_to_five_is_refused() {
        let mut claims = GestureClaims::new();
        for fingers in [-1, 0, 1, 6, 300] {
            assert_eq!(
                claims.claim(SHELL, 1, GestureKind::Swipe, fingers),
                Err(ClaimError::Fingers { fingers })
            );
        }
        assert!(claims.claims().is_empty());
    }

    #[test]
    fn a_begin_replaces_a_gesture_whose_end_was_lost() {
        let mut claims = claims_swipe3();
        let unlocked = GestureLock::Unlocked;
        claims.observe(&begin(GestureKind::Swipe, 3), unlocked);
        claims.observe(&update(GestureKind::Swipe, 3, 50.0, 0.0, 1.0, 0.0), unlocked);
        let fresh = delivered(claims.observe(&begin(GestureKind::Swipe, 3), unlocked));
        assert_eq!((fresh.dx, fresh.dy), (0.0, 0.0), "the new gesture starts from nothing");
    }
}
