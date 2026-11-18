//! The rules of a touchpad gesture binding: the trigger's spelling, when a gesture has a direction, and how far it has gone.
//!
//! Ports of `data/js/gnoblin-core/input/gesture.ts` and `trigger.ts`, which are the reference. The numbers are the TypeScript ones.

use gnoblin_core::GestureKind;
use gnoblin_std_abi::{STD_WAY_DOWN, STD_WAY_IN, STD_WAY_LEFT, STD_WAY_NONE, STD_WAY_OUT, STD_WAY_RIGHT, STD_WAY_UP};

use crate::error::{Fail, Res};

/// Touchpad units of centroid movement before a swipe has a direction.
pub(super) const SWIPE_DECIDE: f64 = 12.0;
/// How far a scale must move from 1 before a pinch has a direction.
pub(super) const PINCH_DECIDE: f64 = 0.1;
/// Touchpad units for a whole swipe (`progress` 1).
pub(super) const SWIPE_TRAVEL: f64 = 400.0;
/// The scale distance for a whole pinch.
pub(super) const PINCH_TRAVEL: f64 = 0.5;

/// What a gesture binding listens for: a swipe or a pinch of 2 to 5 fingers, with or without a direction (`STD_WAY_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GestureTrigger {
    pub(super) kind: GestureKind,
    pub(super) fingers: u8,
    pub(super) direction: u32,
}

impl GestureTrigger {
    /// Reads the spelling of the TypeScript library's triggers: `Swipe3`, `Swipe4Left`, `Pinch2In` (any case).
    pub(super) fn parse(text: &str) -> Res<GestureTrigger> {
        let bad = |why: &str| Fail::type_error(format!("`{text}` is not a gesture trigger: {why}"));
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
            (_, "") => STD_WAY_NONE,
            (GestureKind::Swipe, "left") => STD_WAY_LEFT,
            (GestureKind::Swipe, "right") => STD_WAY_RIGHT,
            (GestureKind::Swipe, "up") => STD_WAY_UP,
            (GestureKind::Swipe, "down") => STD_WAY_DOWN,
            (GestureKind::Pinch, "in") => STD_WAY_IN,
            (GestureKind::Pinch, "out") => STD_WAY_OUT,
            _ => return Err(bad("a swipe goes left, right, up or down and a pinch in or out")),
        };
        Ok(GestureTrigger {
            kind,
            fingers,
            direction,
        })
    }

    /// The canonical spelling: `Swipe3`, `Pinch2In`. Two spellings of one trigger have one text.
    pub(super) fn text(&self) -> String {
        let kind = match self.kind {
            GestureKind::Swipe => "Swipe",
            GestureKind::Pinch => "Pinch",
        };
        let way = match self.direction {
            STD_WAY_LEFT => "Left",
            STD_WAY_RIGHT => "Right",
            STD_WAY_UP => "Up",
            STD_WAY_DOWN => "Down",
            STD_WAY_IN => "In",
            STD_WAY_OUT => "Out",
            _ => "",
        };
        format!("{kind}{}{way}", self.fingers)
    }
}

/// The way the fingers have gone so far, or `STD_WAY_NONE` while they have gone less than the decision distance.
pub(super) fn way_of(kind: GestureKind, dx: f64, dy: f64, scale: f64) -> u32 {
    let decide = match kind {
        GestureKind::Swipe => SWIPE_DECIDE,
        GestureKind::Pinch => PINCH_DECIDE,
    };
    way_past(kind, dx, dy, scale, decide)
}

/// [`way_of`] with the distance a gesture must have gone to have a way: `threshold` touchpad units for a swipe, as far from 1 for a
/// pinch's scale. A binding of the library's own decides at `SWIPE_DECIDE` and `PINCH_DECIDE`.
pub(crate) fn way_past(kind: GestureKind, dx: f64, dy: f64, scale: f64, threshold: f64) -> u32 {
    match kind {
        GestureKind::Swipe => {
            if dx.abs().max(dy.abs()) < threshold {
                STD_WAY_NONE
            } else if dx.abs() >= dy.abs() {
                if dx > 0.0 { STD_WAY_RIGHT } else { STD_WAY_LEFT }
            } else if dy > 0.0 {
                STD_WAY_DOWN
            } else {
                STD_WAY_UP
            }
        }
        GestureKind::Pinch => {
            if (scale - 1.0).abs() < threshold {
                STD_WAY_NONE
            } else if scale < 1.0 {
                STD_WAY_IN
            } else {
                STD_WAY_OUT
            }
        }
    }
}

/// How far a gesture has gone for `trigger`, from 0 to 1. A directed gesture counts only its own way.
pub(super) fn progress_of(trigger: &GestureTrigger, dx: f64, dy: f64, scale: f64) -> f64 {
    let unit = |value: f64| value.clamp(0.0, 1.0);
    match trigger.kind {
        GestureKind::Pinch => {
            let spread = match trigger.direction {
                STD_WAY_IN => 1.0 - scale,
                STD_WAY_OUT => scale - 1.0,
                _ => (scale - 1.0).abs(),
            };
            unit(spread / PINCH_TRAVEL)
        }
        GestureKind::Swipe => match trigger.direction {
            STD_WAY_LEFT => unit(-dx / SWIPE_TRAVEL),
            STD_WAY_RIGHT => unit(dx / SWIPE_TRAVEL),
            STD_WAY_UP => unit(-dy / SWIPE_TRAVEL),
            STD_WAY_DOWN => unit(dy / SWIPE_TRAVEL),
            _ => unit(dx.hypot(dy) / SWIPE_TRAVEL),
        },
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
            (GestureKind::Swipe, 3, STD_WAY_NONE)
        );
        let pinch = GestureTrigger::parse("pinch2in").expect("a pinch");
        assert_eq!(pinch.direction, STD_WAY_IN);
        assert_eq!(pinch.text(), "Pinch2In");
        assert_eq!(
            GestureTrigger::parse("Swipe4Left").map(|t| t.text()).ok(),
            Some("Swipe4Left".to_owned())
        );
        for bad in ["Swipe", "Swipe1", "Swipe6", "Swipe3In", "Pinch2Left", "Tap3"] {
            assert!(GestureTrigger::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_swipe_has_a_direction_once_it_has_gone_twelve_units_along_its_longer_axis() {
        assert_eq!(way_of(GestureKind::Swipe, 11.0, 5.0, 1.0), STD_WAY_NONE);
        assert_eq!(way_of(GestureKind::Swipe, -12.0, 5.0, 1.0), STD_WAY_LEFT);
        assert_eq!(way_of(GestureKind::Swipe, 3.0, 40.0, 1.0), STD_WAY_DOWN);
        assert_eq!(way_of(GestureKind::Pinch, 0.0, 0.0, 1.05), STD_WAY_NONE);
        assert_eq!(way_of(GestureKind::Pinch, 0.0, 0.0, 0.8), STD_WAY_IN);
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
