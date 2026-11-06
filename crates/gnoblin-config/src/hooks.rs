//! The cross-field rules, which the schema cannot declare.
//!
//! A generated validator reads one setting: type, range, accepted values. Some
//! rules are about a whole table or about the relationship between two settings,
//! and those stay here as named functions. The schema names them (`hook = "..."`)
//! and the generator calls them, so the hand-written part is a closed, listed set
//! rather than a second undocumented validator:
//!
//! * the shape of a free-text value whose grammar belongs to another crate
//!   (a keysym name, an output connector, a chord);
//! * rules between settings of one table (a swipe path belongs to a swipe, an
//!   action and a command are exclusive, a spacer does not open or close a
//!   side);
//! * rules over a list (workspace ids are unique, two gestures do not claim the
//!   same movement, a pad button appears once).
//!
//! Every hook reports through the same [`Parser`] as the generated code, so a
//! problem found here has the same path and the same accepted-values shape.

use std::collections::BTreeMap;

use crate::appearance::Accent;
use crate::engine::{Parser, is_inherit};
use crate::generated::{
    AccelCurve, Appearance, ButtonLayout, Compositor, FrameRenderers, Gesture, GesturePoint, GestureWhen, PadButton,
    PadButtonAction, Tablet, TouchpadGestures, WindowManagement, Workspace, Workspaces, Xwayland,
};
use crate::model::Inherit;
use crate::path::Path;
use crate::problem::{Fault, Problem};
use crate::value::Value;

/// Every hook this module implements, checked against the schema by test.
pub const IMPLEMENTED_HOOKS: &[&str] = crate::generated::DECLARED_HOOKS;

/// `compositor`: `locate-pointer-key` must be one key name, not a chord.
///
/// Whether the name is a keysym this build knows is a question for
/// `gnoblin-input`, which owns libxkbcommon; asking it here would make the
/// document layer depend on a keyboard.
pub(crate) fn compositor(
    section: &mut Compositor,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    if let Some(key) = &section.locate_pointer_key
        && !(key.is_empty()
            || key == "disabled"
            || !(key.contains('<') || key.contains('>') || key.chars().any(char::is_whitespace)))
    {
        out.problem(
            &path.key("locate-pointer-key"),
            Fault::UnsupportedValue {
                expected: "one XKB keysym name, \"disabled\", or an empty string".to_owned(),
            },
        );
        section.locate_pointer_key = None;
        return false;
    }
    true
}

/// `xwayland`: a scale is `0` or at least `0.5`.
///
/// Mutter rounds the value and converts it to `int`; anything between 0 and 0.5
/// would round to zero, which is not a scale.
pub(crate) fn xwayland(section: &mut Xwayland, _map: &BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> bool {
    if let Some(scale) = section.scaling_factor
        && scale != 0.0
        && scale < 0.5
    {
        out.problem(
            &path.key("scaling-factor"),
            Fault::UnsupportedValue {
                expected: "0, or a finite number from 0.5 through 2147483520".to_owned(),
            },
        );
        section.scaling_factor = None;
        return false;
    }
    true
}

/// `window-management`: settings that were removed rather than renamed.
///
/// A removed key is refused with what replaced it. Reporting it here, before the
/// generated unknown-key pass, is what makes the explanation the only problem: a
/// key with an explanation is not also a typo.
pub(crate) fn window_management(
    _section: &mut WindowManagement,
    map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let detail = "declare named workspaces in the top-level workspaces field";
    let mut valid = true;
    for key in ["workspace-ids", "workspace-names", "num-workspaces"] {
        if map.contains_key(key) {
            out.problem(
                &path.key(key),
                Fault::Removed {
                    detail: detail.to_owned(),
                },
            );
            valid = false;
        }
    }
    if map.contains_key("center-new-windows") {
        out.problem(
            &path.key("center-new-windows"),
            Fault::Removed {
                detail: "use new-window-placement, which takes free_space, center or cascade".to_owned(),
            },
        );
        valid = false;
    }
    valid
}

/// `workspaces`: ids are unique, and the list is not empty.
pub(crate) fn workspaces(section: &mut Workspaces, _value: &Value, path: &Path, out: &mut Parser) -> bool {
    // The section path *is* the list path for a section whose value is a list.
    let list_path = path.clone();
    if section.workspaces.is_empty() {
        out.problem(
            &list_path,
            Fault::UnsupportedValue {
                expected: "at least one workspace; omit the list to start with four".to_owned(),
            },
        );
        return false;
    }
    let mut seen: Vec<String> = Vec::new();
    for (index, workspace) in section.workspaces.iter().enumerate() {
        let Some(id) = workspace.id.as_deref() else {
            continue;
        };
        if seen.iter().any(|previous| previous == id) {
            out.problem(
                &list_path.entry(index).key("id"),
                Fault::UnsupportedValue {
                    expected: "an id no other workspace uses".to_owned(),
                },
            );
        }
        seen.push(id.to_owned());
    }
    true
}

/// `workspace`: the id and name rules.
pub(crate) fn workspace(
    section: &mut Workspace,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let mut valid = true;
    let id_shaped = {
        let id = section.id.as_deref().unwrap_or_default();
        let mut characters = id.chars();
        characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
            && id.len() <= 64
            && characters.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    };
    if !id_shaped {
        out.problem(
            &path.key("id"),
            Fault::UnsupportedValue {
                expected:
                    "an id of 1 to 64 letters, digits, dots, underscores or dashes, starting with a letter or digit"
                        .to_owned(),
            },
        );
        valid = false;
    }
    let name = section.name.as_deref().unwrap_or_default();
    let length = name.chars().count();
    if name.is_empty() || length > 80 {
        out.problem(
            &path.key("name"),
            Fault::UnsupportedValue {
                expected: "a nonempty name of at most 80 characters".to_owned(),
            },
        );
        valid = false;
    }
    valid
}

/// `button-layout`: the rules Mutter's own parser enforces.
///
/// A spacer cannot open or close a side, two spacers cannot sit next to each
/// other, a button appears at most once per side and a side holds at most seven
/// entries. A layout Mutter silently ignores is worse than a diagnostic.
pub(crate) fn button_layout(
    section: &mut ButtonLayout,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let mut valid = true;
    for (key, side) in [("left", &section.left), ("right", &section.right)] {
        let Some(buttons) = side else {
            continue;
        };
        let side_path = path.key(key);
        if buttons.len() > 7 {
            out.problem(
                &side_path,
                Fault::UnsupportedValue {
                    expected: "at most 7 entries".to_owned(),
                },
            );
            valid = false;
            continue;
        }
        let mut seen: Vec<&crate::generated::TitlebarButton> = Vec::new();
        for (index, button) in buttons.iter().enumerate() {
            let spacer = format!("{button}") == "spacer";
            if spacer {
                let first = index == 0;
                let last = index + 1 == buttons.len();
                let adjacent = index > 0 && format!("{}", buttons[index - 1]) == "spacer";
                if first || last || adjacent {
                    out.problem(
                        &side_path.entry(index),
                        Fault::UnsupportedValue {
                            expected: "a spacer between two buttons".to_owned(),
                        },
                    );
                    valid = false;
                }
                continue;
            }
            if seen.contains(&button) {
                out.problem(
                    &side_path.entry(index),
                    Fault::UnsupportedValue {
                        expected: "each button at most once per side".to_owned(),
                    },
                );
                valid = false;
            }
            seen.push(button);
        }
    }
    valid
}

/// `touchpad-gestures`: no two gestures claim the same movement.
///
/// Both would reserve input at the start of the same gesture, so only one can
/// run; refusing the document is clearer than picking one.
pub(crate) fn gestures(section: &mut TouchpadGestures, _value: &Value, path: &Path, out: &mut Parser) -> bool {
    let list_path = path.clone();
    let mut valid = true;
    for (index, gesture) in section.gestures.iter().enumerate() {
        for previous in 0..index {
            if claims_same_movement(&section.gestures[previous], gesture) {
                out.problem(
                    &list_path.entry(index),
                    Fault::Conflict {
                        other: list_path.entry(previous),
                    },
                );
                valid = false;
                break;
            }
        }
    }
    valid
}

/// Whether two gestures reserve the same movement in an overlapping context.
fn claims_same_movement(first: &Gesture, second: &Gesture) -> bool {
    let same_motion = first.gesture == second.gesture
        && first.fingers == second.fingers
        && match (&first.path, &second.path) {
            (Some(path), Some(other)) => path == other,
            (None, None) => first.direction == second.direction,
            _ => false,
        };
    let overlapping = first.when.unwrap_or(GestureWhen::Unlocked) == GestureWhen::Any
        || second.when.unwrap_or(GestureWhen::Unlocked) == GestureWhen::Any
        || first.when == second.when;
    same_motion && overlapping
}

/// `gesture`: the rules that depend on which kind of gesture this is.
pub(crate) fn gesture(section: &mut Gesture, map: &BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> bool {
    let mut valid = true;
    let Some(kind) = section.gesture else {
        return false;
    };
    let swipe = format!("{kind}") == "swipe";
    if swipe {
        if map.contains_key("direction") {
            out.problem(
                &path.key("direction"),
                Fault::UnsupportedValue {
                    expected: "a swipe takes path and no direction".to_owned(),
                },
            );
            valid = false;
        }
        match &section.path {
            Some(points) => {
                if !path_shaped(points) {
                    out.problem(
                        &path.key("path"),
                        Fault::UnsupportedValue {
                            expected: "a path that starts at x = 0, y = 0 and moves".to_owned(),
                        },
                    );
                    valid = false;
                }
            }
            None => {
                out.problem(
                    &path.key("path"),
                    Fault::Missing {
                        expected: "a path of 2 to 16 points",
                    },
                );
                valid = false;
            }
        }
        if map.contains_key("tolerance") {
            // The band itself is checked by the generated reader; the rule that
            // tolerance applies only to swipes is here.
        } else {
            section.tolerance = Some(0.22);
        }
        if section.threshold.is_none() {
            section.threshold = Some(48.0);
        }
    } else {
        if map.contains_key("path") {
            out.problem(
                &path.key("path"),
                Fault::UnsupportedValue {
                    expected: "a pinch takes direction and no path".to_owned(),
                },
            );
            valid = false;
        }
        if section.direction.is_none() {
            out.problem(&path.key("direction"), Fault::Missing { expected: "in or out" });
            valid = false;
        }
        if map.contains_key("tolerance") {
            out.problem(
                &path.key("tolerance"),
                Fault::UnsupportedValue {
                    expected: "tolerance applies only to swipe paths".to_owned(),
                },
            );
            valid = false;
        }
        if section.threshold.is_none() {
            section.threshold = Some(0.12);
        }
    }
    if section.when.is_none() {
        section.when = Some(GestureWhen::Unlocked);
    }
    let has_action = map.contains_key("action");
    let has_command = map.contains_key("command");
    if has_action == has_command {
        out.problem(
            path,
            Fault::UnsupportedValue {
                expected: "exactly one of action or command".to_owned(),
            },
        );
        valid = false;
    } else if (has_action && section.action.is_none()) || (has_command && section.command.is_none()) {
        valid = false;
    }
    if let Some(command) = &section.command
        && (command.is_empty() || command.first().is_some_and(|first| first.is_empty()))
    {
        out.problem(
            &path.key("command"),
            Fault::UnsupportedValue {
                expected: "a nonempty argv array whose first entry names the program".to_owned(),
            },
        );
        valid = false;
    }
    valid
}

/// Whether a swipe path starts at the origin, moves, and ends away from it.
fn path_shaped(points: &[GesturePoint]) -> bool {
    if points.len() < 2 || points.len() > 16 {
        return false;
    }
    let coordinate = |point: &GesturePoint| (point.x.unwrap_or(f64::NAN), point.y.unwrap_or(f64::NAN));
    let starts_at_origin = coordinate(&points[0]) == (0.0, 0.0);
    let moves = points
        .windows(2)
        .all(|pair| coordinate(&pair[0]) != coordinate(&pair[1]));
    let last = coordinate(&points[points.len() - 1]);
    let ends_away = last != (0.0, 0.0);
    starts_at_origin && moves && ends_away
}

/// `accel-curve`: a positive step and at least two non-negative points.
pub(crate) fn accel_curve(
    section: &mut AccelCurve,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let mut valid = true;
    if section.step.is_some_and(|step| step <= 0.0) {
        out.problem(
            &path.key("step"),
            Fault::UnsupportedValue {
                expected: "a number greater than 0".to_owned(),
            },
        );
        valid = false;
    }
    match &section.points {
        Some(points) => {
            if points.len() < 2 {
                out.problem(
                    &path.key("points"),
                    Fault::UnsupportedValue {
                        expected: "at least two points".to_owned(),
                    },
                );
                valid = false;
            }
            for (index, point) in points.iter().enumerate() {
                if *point < 0.0 {
                    out.problem(
                        &path.key("points").entry(index),
                        Fault::UnsupportedValue {
                            expected: "a number of at least 0".to_owned(),
                        },
                    );
                    valid = false;
                }
            }
        }
        None => {
            out.problem(
                &path.key("points"),
                Fault::Missing {
                    expected: "at least two points",
                },
            );
            valid = false;
        }
    }
    valid
}

/// `appearance`: the accent is a named accent or an opaque `#rrggbb`.
///
/// The accent is the one string the schema cannot shape on its own: a name is one of
/// nine, a hex is exactly three or six digits. The message from [`Accent::parse`] is
/// not reused, so the fault names the accepted forms the way the other hooks do.
pub(crate) fn appearance(
    section: &mut Appearance,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let Some(accent) = &section.accent else {
        return true;
    };
    match Accent::parse(accent) {
        Ok(_) => true,
        Err(_) => {
            out.problem(
                &path.key("accent"),
                Fault::UnsupportedValue {
                    expected: "a named accent or #rrggbb without alpha".to_owned(),
                },
            );
            false
        }
    }
}

/// `tablets.<id>`: an output name, the active-area rule, and unique pad buttons.
pub(crate) fn tablet(section: &mut Tablet, _map: &BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> bool {
    let mut valid = true;
    if let Some(Inherit::Value(output)) = &section.output
        && !(output == "auto" || connector_shaped(output))
    {
        out.problem(
            &path.key("output"),
            Fault::UnsupportedValue {
                expected: "\"auto\" or a connector id of 1 to 64 letters, digits, dots, underscores or dashes"
                    .to_owned(),
            },
        );
        section.output = None;
        valid = false;
    }
    if let Some(Inherit::Value(area)) = &section.area
        && area.len() == 4
        && (area[0] + area[1] >= 1.0 || area[2] + area[3] >= 1.0)
    {
        out.problem(
            &path.key("area"),
            Fault::UnsupportedValue {
                expected: "opposing edges that add to less than 1".to_owned(),
            },
        );
        valid = false;
    }
    if let Some(Inherit::Value(buttons)) = &section.pad_buttons {
        let mut seen: Vec<i64> = Vec::new();
        for (index, button) in buttons.iter().enumerate() {
            if let Some(number) = button.button
                && seen.contains(&number)
            {
                out.problem(
                    &path.key("pad-buttons").entry(index).key("button"),
                    Fault::UnsupportedValue {
                        expected: "a button number no other entry uses".to_owned(),
                    },
                );
                valid = false;
            }
            if let Some(number) = button.button {
                seen.push(number);
            }
        }
    }
    valid
}

/// `pad-buttons[]`: a keybinding goes with the keybinding action and nothing else.
pub(crate) fn pad_button(
    section: &mut PadButton,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let keybinding_action = section.action == Some(PadButtonAction::Keybinding);
    match (keybinding_action, &section.keybinding) {
        (true, Some(accelerator)) if !accelerator.is_empty() => true,
        (true, _) => {
            out.problem(
                path,
                Fault::UnsupportedValue {
                    expected: "a keybinding for the keybinding action".to_owned(),
                },
            );
            false
        }
        (false, Some(_)) => {
            out.problem(
                &path.key("keybinding"),
                Fault::UnsupportedValue {
                    expected: "a keybinding only with the keybinding action".to_owned(),
                },
            );
            section.keybinding = None;
            false
        }
        (false, None) => true,
    }
}

/// `frame-renderers`: an argv whose first entry names the program.
pub(crate) fn frame_renderers(
    section: &mut FrameRenderers,
    _map: &BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> bool {
    let mut valid = true;
    let names: Vec<String> = section.renderers.keys().cloned().collect();
    for name in names {
        let Some(argv) = section.renderers.get(&name) else {
            continue;
        };
        let present = argv.first().is_some_and(|first| !first.is_empty());
        if argv.is_empty() || argv.len() > 32 || !present {
            out.problem(
                &path.key(&name),
                Fault::UnsupportedValue {
                    expected: "an argv of 1 to 32 strings whose first entry names the program".to_owned(),
                },
            );
            section.renderers.remove(&name);
            valid = false;
        }
    }
    valid
}

/// Whether `name` has the shape of an output connector id.
fn connector_shaped(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= 64
        && characters.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

/// A problem list entry, so a hook can build a report without importing
/// [`Problem`] twice.
#[allow(dead_code)]
pub(crate) fn problem(path: &Path, fault: Fault) -> Problem {
    Problem::new(path.clone(), fault)
}

/// Whether a value is the `"inherit"` marker, re-exported for hooks that read the
/// map directly.
#[allow(dead_code)]
pub(crate) fn inherited(value: &Value) -> bool {
    is_inherit(value)
}
