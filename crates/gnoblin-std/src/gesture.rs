//! Touchpad gestures: the [`GestureBinding`] trait, and [`Std::bind_gesture`] that runs one.
//!
//! A port of the lifecycle of `data/js/gnoblin-core/input/gesture.ts` and the `GestureBinding` class of
//! `data/js/gnoblin-core/bindings/binding.ts`, over the core's `bind_gesture` claim. The compositor claims every swipe or pinch of a
//! finger count for the shell; the SDK turns its steps into what a binding reads:
//!
//! - A gesture **begins** when the fingers touch. A binding with a direction (`Swipe3Left`, `Pinch2In`) waits until the fingers have
//!   gone far enough to have one ([`SWIPE_DECIDE`] touchpad units, or a scale that has moved [`PINCH_DECIDE`] from 1) and is asked to
//!   [start](GestureBinding::on_start) only if the gesture went its way; a binding with no direction is asked at once. A binding that
//!   declines is not asked again for this gesture, and the gesture stays the compositor's.
//! - Every later step is an [update](GestureBinding::on_update) with how far the fingers have gone and `progress`, 0 to 1, for a
//!   directed gesture (400 units of swipe, a scale change of 0.5 of a pinch, in its own direction only).
//! - The lift ends it ([`on_end`](GestureBinding::on_end)); the compositor cancelling it ends it with
//!   [`on_cancel`](GestureBinding::on_cancel).
//!
//! ```ignore
//! struct Slide;
//! impl GestureBinding for Slide {
//!     fn trigger(&self) -> String { "Swipe3".to_owned() }
//!     fn on_update(&mut self, event: &GestureInfo) { /* follow the fingers */ }
//!     fn on_end(&mut self, event: &GestureInfo) { if event.progress > SWIPE_COMMIT { /* commit */ } }
//! }
//! std.bind_gesture(Slide)?;
//! ```
//!
//! Two bindings of one finger count and kind take one claim each; the core refuses the second claim while another holds it, so a shell
//! that wants a swipe left and a swipe right on three fingers binds one binding with no direction and reads
//! [`GestureInfo::direction`].

use gnoblin_core::{DragPhase, GestureEvent as RawGesture, GestureKind, Subscription};

use crate::api::Std;
use crate::bindings::Point;
use crate::client::Client;
use crate::error::{Error, ErrorKind, Result};

/// Touchpad units of centroid movement before a swipe has a direction.
pub const SWIPE_DECIDE: f64 = 12.0;
/// How far a scale must move from 1 before a pinch has a direction.
pub const PINCH_DECIDE: f64 = 0.1;
/// Touchpad units for a whole swipe (`progress` 1).
pub const SWIPE_TRAVEL: f64 = 400.0;
/// The scale distance for a whole pinch.
pub const PINCH_TRAVEL: f64 = 0.5;
/// Minimum progress for a swipe action to run when the gesture ends.
pub const SWIPE_COMMIT: f64 = 48.0 / SWIPE_TRAVEL;
/// Minimum progress for a pinch action to run when the gesture ends.
pub const PINCH_COMMIT: f64 = 0.12 / PINCH_TRAVEL;

/// The way a gesture went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureDirection {
    /// A swipe to the left.
    Left,
    /// A swipe to the right.
    Right,
    /// A swipe up.
    Up,
    /// A swipe down.
    Down,
    /// A pinch that closes.
    In,
    /// A pinch that opens.
    Out,
}

impl GestureDirection {
    fn name(self) -> &'static str {
        match self {
            GestureDirection::Left => "Left",
            GestureDirection::Right => "Right",
            GestureDirection::Up => "Up",
            GestureDirection::Down => "Down",
            GestureDirection::In => "In",
            GestureDirection::Out => "Out",
        }
    }
}

/// What a gesture binding listens for: a swipe or a pinch of 2 to 5 fingers, with or without a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GestureTrigger {
    /// A swipe or a pinch.
    pub kind: GestureKind,
    /// How many fingers, 2 to 5.
    pub fingers: u8,
    /// The way it must go, or `None` for any way.
    pub direction: Option<GestureDirection>,
}

impl GestureTrigger {
    /// Reads the spelling of the TypeScript library's triggers: `Swipe3`, `Swipe4Left`, `Pinch2In` (any case).
    ///
    /// # Errors
    ///
    /// The text is not a swipe or a pinch of 2 to 5 fingers, or its direction is not one that kind goes.
    pub fn parse(text: &str) -> Result<GestureTrigger> {
        let bad = |why: &str| Error::new(ErrorKind::Type, format!("`{text}` is not a gesture trigger: {why}"));
        let lower = text.trim().to_ascii_lowercase();
        let (kind, rest) = if let Some(rest) = lower.strip_prefix("swipe") {
            (GestureKind::Swipe, rest)
        } else if let Some(rest) = lower.strip_prefix("pinch") {
            (GestureKind::Pinch, rest)
        } else {
            return Err(bad("expected Swipe or Pinch with a finger count"));
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let (count, way) = rest.split_at(digits);
        let fingers: u8 = count.parse().map_err(|_| bad("expected a finger count"))?;
        if !(2..=5).contains(&fingers) {
            return Err(bad("a gesture has 2 to 5 fingers"));
        }
        let direction = match (kind, way) {
            (_, "") => None,
            (GestureKind::Swipe, "left") => Some(GestureDirection::Left),
            (GestureKind::Swipe, "right") => Some(GestureDirection::Right),
            (GestureKind::Swipe, "up") => Some(GestureDirection::Up),
            (GestureKind::Swipe, "down") => Some(GestureDirection::Down),
            (GestureKind::Pinch, "in") => Some(GestureDirection::In),
            (GestureKind::Pinch, "out") => Some(GestureDirection::Out),
            _ => return Err(bad("a swipe goes left, right, up or down and a pinch in or out")),
        };
        Ok(GestureTrigger {
            kind,
            fingers,
            direction,
        })
    }

    /// The canonical spelling: `Swipe3`, `Pinch2In`.
    pub fn text(&self) -> String {
        let kind = match self.kind {
            GestureKind::Swipe => "Swipe",
            GestureKind::Pinch => "Pinch",
        };
        format!(
            "{kind}{}{}",
            self.fingers,
            self.direction.map_or("", GestureDirection::name)
        )
    }
}

/// Where in a gesture an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureStage {
    /// The binding is being asked to start.
    Begin,
    /// The fingers moved.
    Update,
    /// The fingers lifted.
    End,
    /// The compositor ended the gesture.
    Cancel,
}

/// What a gesture binding is told at each step.
#[derive(Clone, Debug)]
pub struct GestureInfo {
    /// The window under the pointer when the gesture began, if one was.
    pub client: Option<Client>,
    /// Where in the gesture this is.
    pub phase: GestureStage,
    /// What the binding listens for.
    pub trigger: GestureTrigger,
    /// How many fingers.
    pub fingers: u8,
    /// Horizontal travel of the fingers' centre since the begin, in touchpad units.
    pub dx: f64,
    /// Vertical travel.
    pub dy: f64,
    /// A pinch's scale: 1 at the begin.
    pub scale: f64,
    /// A pinch's rotation in degrees.
    pub rotation: f64,
    /// How far a directed gesture has gone, 0 to 1: 400 units of swipe, or a scale change of 0.5, in the trigger's own direction.
    /// For a trigger with no direction, how far the fingers have gone in any way.
    pub progress: f64,
    /// Where the pointer was when the gesture began, when the seat reports it.
    pub position: Option<Point>,
}

impl GestureInfo {
    /// The way the fingers have gone so far (the longer axis of a swipe; a pinch's scale from 1), or `None` while they have gone less
    /// than [`SWIPE_DECIDE`] units or [`PINCH_DECIDE`] of scale.
    pub fn direction(&self) -> Option<GestureDirection> {
        direction_of(self.trigger.kind, self.dx, self.dy, self.scale)
    }
}

/// A gesture of the shell's own, claimed with [`Std::bind_gesture`]. Only [`trigger`](GestureBinding::trigger) is required.
///
/// The binding is one value that lives as long as its claim, so it keeps the state of a gesture in `self` between `on_start` and
/// `on_end`. The SDK runs one gesture of a binding at a time.
pub trait GestureBinding: 'static {
    /// What the binding listens for: `Swipe3`, `Swipe4Left`, `Pinch2In`.
    fn trigger(&self) -> String;

    /// The gesture began (or, for a directed binding, went the right way). Return `false` to decline: the binding is not asked again
    /// for this gesture.
    fn on_start(&mut self, _event: &GestureInfo) -> bool {
        true
    }

    /// The fingers moved.
    fn on_update(&mut self, _event: &GestureInfo) {}

    /// The fingers lifted.
    fn on_end(&mut self, _event: &GestureInfo) {}

    /// The compositor ended the gesture. The binding undoes what it did.
    fn on_cancel(&mut self, _event: &GestureInfo) {}
}

fn direction_of(kind: GestureKind, dx: f64, dy: f64, scale: f64) -> Option<GestureDirection> {
    match kind {
        GestureKind::Swipe => {
            if dx.abs().max(dy.abs()) < SWIPE_DECIDE {
                None
            } else if dx.abs() >= dy.abs() {
                Some(if dx > 0.0 {
                    GestureDirection::Right
                } else {
                    GestureDirection::Left
                })
            } else {
                Some(if dy > 0.0 {
                    GestureDirection::Down
                } else {
                    GestureDirection::Up
                })
            }
        }
        GestureKind::Pinch => {
            if (scale - 1.0).abs() < PINCH_DECIDE {
                None
            } else {
                Some(if scale < 1.0 {
                    GestureDirection::In
                } else {
                    GestureDirection::Out
                })
            }
        }
    }
}

/// How far a gesture has gone for `trigger`, from 0 to 1. A directed gesture counts only its own way.
fn progress_of(trigger: &GestureTrigger, dx: f64, dy: f64, scale: f64) -> f64 {
    let unit = |value: f64| value.clamp(0.0, 1.0);
    match trigger.kind {
        GestureKind::Pinch => {
            let spread = match trigger.direction {
                Some(GestureDirection::In) => 1.0 - scale,
                Some(GestureDirection::Out) => scale - 1.0,
                _ => (scale - 1.0).abs(),
            };
            unit(spread / PINCH_TRAVEL)
        }
        GestureKind::Swipe => match trigger.direction {
            Some(GestureDirection::Left) => unit(-dx / SWIPE_TRAVEL),
            Some(GestureDirection::Right) => unit(dx / SWIPE_TRAVEL),
            Some(GestureDirection::Up) => unit(-dy / SWIPE_TRAVEL),
            Some(GestureDirection::Down) => unit(dy / SWIPE_TRAVEL),
            _ => unit(dx.hypot(dy) / SWIPE_TRAVEL),
        },
    }
}

/// One gesture, from its begin to its end or cancel.
struct GestureRun {
    client: Option<Client>,
    position: Option<Point>,
    latest: RawGesture,
    started: bool,
    /// No start can happen for this gesture any more.
    dead: bool,
}

impl Std {
    /// The pointer's place, as the seat reports it (`pointer_x` and `pointer_y`).
    pub(crate) fn pointer(&self) -> Option<Point> {
        let g = self.gnoblin();
        let seat = g.class("seat").ok()?;
        let node = g
            .root()
            .ok()?
            .children()
            .ok()?
            .into_iter()
            .find(|node| node.class().ok() == Some(seat))?;
        Some(Point {
            x: node.get_f64("pointer_x").ok()?,
            y: node.get_f64("pointer_y").ok()?,
        })
    }

    /// Claims the touchpad gesture the binding names and runs its gestures, as the module documentation describes. The claim lasts for
    /// the life of the generation, or until the returned subscription is removed.
    ///
    /// # Errors
    ///
    /// The binding's trigger is not a gesture, the compositor refused the claim (another shell holds it), or the generation retired.
    pub fn bind_gesture<B: GestureBinding>(&self, mut binding: B) -> Result<Subscription> {
        let trigger = GestureTrigger::parse(&binding.trigger())?;
        let std = self.clone();
        let mut run: Option<GestureRun> = None;
        let event = move |std: &Std, run: &GestureRun, phase: GestureStage| {
            let sample = &run.latest;
            GestureInfo {
                client: run.client.clone(),
                phase,
                trigger,
                fingers: trigger.fingers,
                dx: sample.dx,
                dy: sample.dy,
                scale: sample.scale,
                rotation: sample.rotation,
                progress: progress_of(&trigger, sample.dx, sample.dy, sample.scale),
                position: run.position.or_else(|| std.pointer()),
            }
        };
        // Starts the gesture on the binding once its direction has come true.
        let decide = move |binding: &mut B, std: &Std, run: &mut GestureRun| {
            if run.started || run.dead {
                return;
            }
            if let Some(wanted) = trigger.direction {
                let way = direction_of(trigger.kind, run.latest.dx, run.latest.dy, run.latest.scale);
                match way {
                    // Not decided yet: wait for the fingers to go further.
                    None => return,
                    Some(way) if way != wanted => {
                        run.dead = true;
                        return;
                    }
                    Some(_) => {}
                }
            }
            if binding.on_start(&event(std, run, GestureStage::Begin)) {
                run.started = true;
            } else {
                run.dead = true;
            }
        };
        self.gnoblin()
            .bind_gesture(trigger.kind, trigger.fingers, move |raw: &RawGesture| match raw.phase {
                DragPhase::Start => {
                    if let Some(previous) = run.take()
                        && previous.started
                    {
                        binding.on_cancel(&event(&std, &previous, GestureStage::Cancel));
                    }
                    let position = std.pointer();
                    let client = position.and_then(|at| std.window_at(at)).map(|(client, _)| client);
                    let mut fresh = GestureRun {
                        client,
                        position,
                        latest: raw.clone(),
                        started: false,
                        dead: false,
                    };
                    decide(&mut binding, &std, &mut fresh);
                    run = Some(fresh);
                }
                DragPhase::Update => {
                    let Some(held) = run.as_mut() else {
                        return;
                    };
                    held.latest = raw.clone();
                    decide(&mut binding, &std, held);
                    if held.started {
                        binding.on_update(&event(&std, held, GestureStage::Update));
                    }
                }
                DragPhase::End => {
                    let Some(mut held) = run.take() else {
                        return;
                    };
                    held.latest = raw.clone();
                    decide(&mut binding, &std, &mut held);
                    if held.started {
                        binding.on_end(&event(&std, &held, GestureStage::End));
                    }
                }
                DragPhase::Cancel => {
                    if let Some(held) = run.take()
                        && held.started
                    {
                        binding.on_cancel(&event(&std, &held, GestureStage::Cancel));
                    }
                }
            })
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_read_the_spelling_of_the_typescript_library() {
        let swipe = GestureTrigger::parse("Swipe3").expect("a swipe");
        assert_eq!(
            (swipe.kind, swipe.fingers, swipe.direction),
            (GestureKind::Swipe, 3, None)
        );
        let pinch = GestureTrigger::parse("pinch2in").expect("a pinch");
        assert_eq!(pinch.direction, Some(GestureDirection::In));
        assert_eq!(pinch.text(), "Pinch2In");
        assert_eq!(
            GestureTrigger::parse("Swipe4Left").map(|t| t.text()),
            Ok("Swipe4Left".to_owned())
        );
        for bad in ["Swipe", "Swipe1", "Swipe6", "Swipe3In", "Pinch2Left", "Tap3"] {
            assert!(GestureTrigger::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_swipe_has_a_direction_once_it_has_gone_twelve_units_along_its_longer_axis() {
        assert_eq!(direction_of(GestureKind::Swipe, 11.0, 5.0, 1.0), None);
        assert_eq!(
            direction_of(GestureKind::Swipe, -12.0, 5.0, 1.0),
            Some(GestureDirection::Left)
        );
        assert_eq!(
            direction_of(GestureKind::Swipe, 3.0, 40.0, 1.0),
            Some(GestureDirection::Down)
        );
        assert_eq!(direction_of(GestureKind::Pinch, 0.0, 0.0, 1.05), None);
        assert_eq!(
            direction_of(GestureKind::Pinch, 0.0, 0.0, 0.8),
            Some(GestureDirection::In)
        );
    }

    #[test]
    fn progress_counts_only_the_triggers_own_way() {
        let left = GestureTrigger::parse("Swipe3Left").expect("a swipe");
        assert_eq!(progress_of(&left, -200.0, 0.0, 1.0), 0.5);
        assert_eq!(progress_of(&left, 200.0, 0.0, 1.0), 0.0);
        assert_eq!(progress_of(&left, -800.0, 0.0, 1.0), 1.0);
        let any = GestureTrigger::parse("Swipe3").expect("a swipe");
        assert_eq!(progress_of(&any, 120.0, 160.0, 1.0), 0.5);
        let out = GestureTrigger::parse("Pinch2Out").expect("a pinch");
        assert_eq!(progress_of(&out, 0.0, 0.0, 1.25), 0.5);
    }
}
