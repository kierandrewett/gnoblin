//! Window management, workspaces and touchpad gestures.
//!
//! These three sections describe how the session arranges windows, so they share
//! a module: a workspace declaration and a gesture that switches to it are read
//! together, and the conflict between a declared workspace list and
//! `dynamic-workspaces` is reported once, at the workspace list.
//!
//! Validation of list entries is per entry and never stops the list: two bad
//! gestures are two problems, and the entries around them are still read.

use std::collections::BTreeMap;

use crate::enums::{
    FocusMode, FocusNewWindows, GestureAction, GestureDirection, GestureKind, GestureWhen, NewWindowPlacement,
    TitlebarAction, TitlebarButton,
};
use crate::fields::{Parser, as_table, flag, integer, list, removed, require, setting, strings, text, unknown_keys};
use crate::path::Path;
use crate::problem::Fault;
use crate::value::Value;

/// `window-management`: stacking, focus and placement policy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowManagement {
    /// When pointer movement changes focus.
    pub focus_mode: Option<FocusMode>,
    /// When a new window may take focus.
    pub focus_new_windows: Option<FocusNewWindows>,
    /// Maximize a window that nearly fills its monitor.
    pub auto_maximize: Option<bool>,
    /// Raise a window when it is clicked.
    pub raise_on_click: Option<bool>,
    /// Raise the focused window after a delay.
    pub auto_raise: Option<bool>,
    /// The delay before auto-raise, in milliseconds.
    pub auto_raise_delay: Option<u16>,
    /// Change focus only after the pointer rests over another window.
    pub focus_change_on_pointer_rest: Option<bool>,
    /// How long a client may take to answer a ping, in milliseconds.
    pub check_alive_timeout: Option<u32>,
    /// Keep workspaces on the primary monitor.
    pub workspaces_only_on_primary: Option<bool>,
    /// Drag a window to a screen edge to tile it.
    pub edge_tiling: Option<bool>,
    /// Where a new application window opens.
    pub new_window_placement: Option<NewWindowPlacement>,
    /// Place modal dialogs with their parent.
    pub attach_modal_dialogs: Option<bool>,
    /// Turn off Mutter's compatibility workarounds for X11 clients.
    pub disable_workarounds: Option<bool>,
    /// Keep a dragged window inside the work area.
    pub constrain_drag_to_work_area: Option<bool>,
    /// Let the workspace count follow use instead of a declaration.
    ///
    /// Refused together with a top-level `workspaces` list: the declaration
    /// fixes the count, so a document that asks for both is contradictory.
    pub dynamic_workspaces: Option<bool>,
    /// What a titlebar double-click does.
    pub action_double_click_titlebar: Option<TitlebarAction>,
    /// What a titlebar middle-click does.
    pub action_middle_click_titlebar: Option<TitlebarAction>,
    /// What a titlebar right-click does.
    pub action_right_click_titlebar: Option<TitlebarAction>,
    /// The button layout preference stored for compatibility with Mutter.
    pub button_layout: Option<ButtonLayout>,
}

/// `window-management.button-layout`: the ordered buttons per side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ButtonLayout {
    /// The buttons drawn on the left, in order.
    pub left: Option<Vec<TitlebarButton>>,
    /// The buttons drawn on the right, in order.
    pub right: Option<Vec<TitlebarButton>>,
}

/// The keys `window-management` accepts.
const WINDOW_MANAGEMENT_KEYS: &[&str] = &[
    "action-double-click-titlebar",
    "action-middle-click-titlebar",
    "action-right-click-titlebar",
    "attach-modal-dialogs",
    "auto-maximize",
    "auto-raise",
    "auto-raise-delay",
    "button-layout",
    "check-alive-timeout",
    "constrain-drag-to-work-area",
    "disable-workarounds",
    "dynamic-workspaces",
    "edge-tiling",
    "focus-change-on-pointer-rest",
    "focus-mode",
    "focus-new-windows",
    "new-window-placement",
    "raise-on-click",
    "workspaces-only-on-primary",
];

/// The keys `window-management.button-layout` accepts.
const BUTTON_LAYOUT_KEYS: &[&str] = &["left", "right"];

/// The most buttons one side of a titlebar may hold, spacers included.
const MAX_BUTTONS_PER_SIDE: usize = 7;

/// Parse the `window-management` table.
pub(crate) fn parse_window_management(
    map: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> WindowManagement {
    // The removed settings are reported first so the unknown-key hint does not
    // fire for them: one mistake earns one problem, and the explanation is more
    // useful than a list of accepted keys.
    let removed_detail = "declare named workspaces in the top-level workspaces field";
    for key in ["workspace-ids", "workspace-names", "num-workspaces"] {
        removed(map, key, path, out, removed_detail);
    }
    removed(
        map,
        "center-new-windows",
        path,
        out,
        "use new-window-placement, which takes free_space, center or cascade",
    );
    unknown_keys(map, path, out, WINDOW_MANAGEMENT_KEYS);
    let button_layout = if map.contains_key("button-layout") {
        crate::fields::table(map, "button-layout", path, out, "a table")
            .map(|layout| parse_button_layout(layout, &path.key("button-layout"), out))
    } else {
        None
    };
    WindowManagement {
        focus_mode: setting(map, "focus-mode", "focus-mode", path, out),
        focus_new_windows: setting(map, "focus-new-windows", "focus-new-windows", path, out),
        auto_maximize: flag(map, "auto-maximize", path, out),
        raise_on_click: flag(map, "raise-on-click", path, out),
        auto_raise: flag(map, "auto-raise", path, out),
        auto_raise_delay: integer(
            map,
            "auto-raise-delay",
            path,
            out,
            0..=10_000,
            "an integer from 0 to 10000",
        ),
        focus_change_on_pointer_rest: flag(map, "focus-change-on-pointer-rest", path, out),
        check_alive_timeout: integer(
            map,
            "check-alive-timeout",
            path,
            out,
            0..=i64::from(u32::MAX),
            "an integer from 0 to 4294967295",
        ),
        workspaces_only_on_primary: flag(map, "workspaces-only-on-primary", path, out),
        edge_tiling: flag(map, "edge-tiling", path, out),
        new_window_placement: setting(map, "new-window-placement", "new-window-placement", path, out),
        attach_modal_dialogs: flag(map, "attach-modal-dialogs", path, out),
        disable_workarounds: flag(map, "disable-workarounds", path, out),
        constrain_drag_to_work_area: flag(map, "constrain-drag-to-work-area", path, out),
        dynamic_workspaces: flag(map, "dynamic-workspaces", path, out),
        action_double_click_titlebar: setting(map, "action-double-click-titlebar", "titlebar-action", path, out),
        action_middle_click_titlebar: setting(map, "action-middle-click-titlebar", "titlebar-action", path, out),
        action_right_click_titlebar: setting(map, "action-right-click-titlebar", "titlebar-action", path, out),
        button_layout,
    }
}

/// Parse `window-management.button-layout`.
fn parse_button_layout(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> ButtonLayout {
    unknown_keys(map, path, out, BUTTON_LAYOUT_KEYS);
    ButtonLayout {
        left: side(map, "left", path, out),
        right: side(map, "right", path, out),
    }
}

/// One side of the titlebar layout.
///
/// A side is a gap-separated run of buttons: a spacer cannot open or close the
/// run, two spacers cannot sit next to each other, and a button appears at most
/// once. All three rules exist because Mutter's own parser rejects the layout
/// otherwise, and a layout Mutter silently ignores is worse than a diagnostic.
fn side(map: &mut BTreeMap<String, Value>, key: &str, path: &Path, out: &mut Parser) -> Option<Vec<TitlebarButton>> {
    let entry_path = path.key(key);
    let buttons = crate::fields::settings::<TitlebarButton>(map, key, "button-layout", path, out)?;
    if buttons.len() > MAX_BUTTONS_PER_SIDE {
        out.problem(
            &entry_path,
            Fault::UnsupportedValue {
                expected: format!("at most {MAX_BUTTONS_PER_SIDE} entries"),
            },
        );
        return None;
    }
    let mut seen: Vec<TitlebarButton> = Vec::new();
    let mut valid = true;
    for (index, button) in buttons.iter().enumerate() {
        if *button == TitlebarButton::Spacer {
            let first = index == 0;
            let last = index + 1 == buttons.len();
            let adjacent = index > 0 && buttons[index - 1] == TitlebarButton::Spacer;
            if first || last || adjacent {
                out.problem(
                    &entry_path.entry(index),
                    Fault::UnsupportedValue {
                        expected: "a spacer between two buttons".to_owned(),
                    },
                );
                valid = false;
            }
            continue;
        }
        if seen.contains(button) {
            out.problem(
                &entry_path.entry(index),
                Fault::UnsupportedValue {
                    expected: "each button at most once per side".to_owned(),
                },
            );
            valid = false;
        }
        seen.push(*button);
    }
    valid.then_some(buttons)
}

/// One named workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    /// The stable identifier, which rules and scripts use.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// The keys a workspace entry accepts.
const WORKSPACE_KEYS: &[&str] = &["id", "name"];

/// The longest workspace name, in characters.
const MAX_WORKSPACE_NAME_LENGTH: usize = 80;

/// The longest workspace id, in bytes, including the leading character.
const MAX_WORKSPACE_ID_LENGTH: usize = 64;

/// Parse the top-level `workspaces` list.
pub(crate) fn parse_workspaces(
    document: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> Vec<Workspace> {
    let Some(items) = list(document, "workspaces", path, out, "a list of workspace tables") else {
        return Vec::new();
    };
    let list_path = path.key("workspaces");
    if items.is_empty() {
        out.problem(
            &list_path,
            Fault::UnsupportedValue {
                expected: "at least one workspace; omit the list to start with four".to_owned(),
            },
        );
        return Vec::new();
    }
    let mut workspaces: Vec<Workspace> = Vec::with_capacity(items.len());
    for (index, item) in items.iter_mut().enumerate() {
        let entry_path = list_path.entry(index);
        let Some(entry) = as_table(item, &entry_path, out, "a workspace table") else {
            continue;
        };
        unknown_keys(entry, &entry_path, out, WORKSPACE_KEYS);
        let id = if require(entry, "id", &entry_path, out, "a workspace id") {
            text(entry, "id", &entry_path, out)
        } else {
            None
        };
        let id = id.and_then(|id| check_workspace_id(id, &entry_path, out));
        if let Some(id) = id.as_ref()
            && workspaces.iter().any(|workspace| &workspace.id == id)
        {
            out.problem(
                &entry_path.key("id"),
                Fault::UnsupportedValue {
                    expected: "an id no other workspace uses".to_owned(),
                },
            );
        }
        let name = if require(entry, "name", &entry_path, out, "a workspace name") {
            text(entry, "name", &entry_path, out)
        } else {
            None
        };
        let name = name.and_then(|name| {
            let length = name.chars().count();
            if name.is_empty() || length > MAX_WORKSPACE_NAME_LENGTH {
                out.problem(
                    &entry_path.key("name"),
                    Fault::UnsupportedValue {
                        expected: format!("a nonempty name of at most {MAX_WORKSPACE_NAME_LENGTH} characters"),
                    },
                );
                return None;
            }
            Some(name)
        });
        if let (Some(id), Some(name)) = (id, name) {
            workspaces.push(Workspace { id, name });
        }
    }
    workspaces
}

/// Check the shape of a workspace id.
fn check_workspace_id(id: String, path: &Path, out: &mut Parser) -> Option<String> {
    let mut characters = id.chars();
    let leading = characters.next();
    let shaped = leading.is_some_and(|first| first.is_ascii_alphanumeric())
        && id.len() <= MAX_WORKSPACE_ID_LENGTH
        && characters.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character));
    if shaped {
        return Some(id);
    }
    out.problem(
        &path.key("id"),
        Fault::UnsupportedValue {
            expected: "an id of 1 to 64 letters, digits, dots, underscores or dashes, starting with a letter or digit"
                .to_owned(),
        },
    );
    None
}

/// One point of a swipe path, normalised to the range -1 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GesturePoint {
    /// Horizontal position, increasing to the right.
    pub x: f64,
    /// Vertical position, increasing downwards.
    pub y: f64,
}

/// One touchpad gesture binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    /// The unique name diagnostics use.
    pub name: String,
    /// Whether this is a swipe or a pinch.
    pub gesture: GestureKind,
    /// How many fingers the gesture needs.
    pub fingers: u8,
    /// The pinch direction; only for a pinch.
    pub direction: Option<GestureDirection>,
    /// The swipe path; only for a swipe.
    pub path: Option<Vec<GesturePoint>>,
    /// The built-in action, or `None` when this runs a command.
    pub action: Option<GestureAction>,
    /// The command to start, or `None` when this runs an action.
    pub command: Option<Vec<String>>,
    /// The lock context the gesture applies in.
    pub when: GestureWhen,
    /// The minimum movement before the path can match.
    pub threshold: f64,
    /// How closely the movement must match the path.
    pub tolerance: Option<f64>,
}

/// The keys a gesture entry accepts.
const GESTURE_KEYS: &[&str] = &[
    "action",
    "command",
    "direction",
    "fingers",
    "gesture",
    "name",
    "path",
    "threshold",
    "tolerance",
    "when",
];

/// The most gestures one document may declare.
const MAX_GESTURES: usize = 64;

/// The most points a swipe path may hold.
const MAX_PATH_POINTS: usize = 16;

/// The greatest horizontal or vertical offset a path point may hold.
const MAX_PATH_COORDINATE: f64 = 1.0;

/// Parse the `touchpad-gestures` list.
pub(crate) fn parse_touchpad_gestures(
    document: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
) -> Vec<Gesture> {
    let Some(items) = list(document, "touchpad-gestures", path, out, "a list of gesture tables") else {
        return Vec::new();
    };
    let list_path = path.key("touchpad-gestures");
    if items.len() > MAX_GESTURES {
        out.problem(
            &list_path,
            Fault::UnsupportedValue {
                expected: format!("at most {MAX_GESTURES} gestures"),
            },
        );
    }
    let mut gestures: Vec<Gesture> = Vec::new();
    let mut inputs: Vec<GestureInput> = Vec::new();
    for (index, item) in items.iter_mut().enumerate() {
        let entry_path = list_path.entry(index);
        let Some(entry) = as_table(item, &entry_path, out, "a gesture table") else {
            continue;
        };
        unknown_keys(entry, &entry_path, out, GESTURE_KEYS);
        let before = out.problem_count();
        let gesture = parse_gesture(entry, &entry_path, out, &gestures);
        if let Some(gesture) = gesture {
            if let Some(previous) = inputs
                .iter()
                .position(|input| conflicts(input, &GestureInput::of(&gesture)))
            {
                out.problem(
                    &entry_path,
                    Fault::Conflict {
                        other: list_path.entry(previous),
                    },
                );
            }
            inputs.push(GestureInput::of(&gesture));
            gestures.push(gesture);
        } else {
            debug_assert!(out.problem_count() > before, "a dropped gesture must be reported");
        }
    }
    gestures
}

/// Parse one gesture entry.
fn parse_gesture(
    entry: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
    previous: &[Gesture],
) -> Option<Gesture> {
    let name = if require(entry, "name", path, out, "a gesture name") {
        text(entry, "name", path, out)
    } else {
        None
    };
    let name = name?;
    let unique = !previous.iter().any(|gesture| gesture.name == name);
    if !gesture_name_shaped(&name) || !unique {
        out.problem(
            &path.key("name"),
            Fault::UnsupportedValue {
                expected:
                    "a unique name of 1 to 64 letters, digits, underscores or dashes, starting with a letter or digit"
                        .to_owned(),
            },
        );
        return None;
    }
    if !require(entry, "gesture", path, out, "swipe or pinch") {
        return None;
    }
    let gesture = setting::<GestureKind>(entry, "gesture", "gesture", path, out)?;
    if !require(entry, "fingers", path, out, "a finger count from 2 to 5") {
        return None;
    }
    let fingers = integer::<u8>(entry, "fingers", path, out, 2..=5, "an integer from 2 to 5 fingers")?;
    let mut valid = true;
    let (direction, path_points, threshold) = match gesture {
        GestureKind::Swipe => {
            if !require(entry, "path", path, out, "a path of 2 to 16 points") {
                return None;
            }
            let points = swipe_path(entry, path, out)?;
            let threshold = threshold(entry, path, out, 16.0..=240.0, 48.0, "a swipe threshold from 16 to 240");
            if entry.contains_key("direction") {
                out.problem(
                    &path.key("direction"),
                    Fault::UnsupportedValue {
                        expected: "a swipe takes path and no direction".to_owned(),
                    },
                );
                valid = false;
            }
            (None, Some(points), threshold)
        }
        GestureKind::Pinch => {
            if !require(entry, "direction", path, out, "in or out") {
                return None;
            }
            let direction = setting::<GestureDirection>(entry, "direction", "direction", path, out);
            let threshold = threshold(entry, path, out, 0.05..=0.5, 0.12, "a pinch threshold from 0.05 to 0.5");
            if entry.contains_key("path") {
                out.problem(
                    &path.key("path"),
                    Fault::UnsupportedValue {
                        expected: "a pinch takes direction and no path".to_owned(),
                    },
                );
                valid = false;
            }
            if direction.is_none() {
                valid = false;
            }
            (direction, None, threshold)
        }
    };
    let has_action = entry.contains_key("action");
    let has_command = entry.contains_key("command");
    let action = setting::<GestureAction>(entry, "action", "touchpad-gesture-action", path, out);
    let command = if has_command {
        strings(entry, "command", path, out).and_then(|command| check_command(command, path, out))
    } else {
        None
    };
    if has_action == has_command {
        out.problem(
            path,
            Fault::UnsupportedValue {
                expected: "exactly one of action or command".to_owned(),
            },
        );
        valid = false;
    } else if (has_action && action.is_none()) || (has_command && command.is_none()) {
        // The value itself was already reported; the gesture cannot be built.
        valid = false;
    }
    let when = setting::<GestureWhen>(entry, "when", "when", path, out).unwrap_or(GestureWhen::Unlocked);
    let tolerance = match gesture {
        GestureKind::Swipe => tolerance(entry, path, out),
        GestureKind::Pinch => {
            if entry.contains_key("tolerance") {
                out.problem(
                    &path.key("tolerance"),
                    Fault::UnsupportedValue {
                        expected: "tolerance applies only to swipe paths".to_owned(),
                    },
                );
                valid = false;
            }
            None
        }
    };
    valid.then_some(Gesture {
        name,
        gesture,
        fingers,
        direction,
        path: path_points,
        action,
        command,
        when,
        threshold,
        tolerance,
    })
}

/// Whether a gesture name is 1 to 64 letters, digits, `_` or `-`, starting with a
/// letter or digit.
fn gesture_name_shaped(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= 64
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

/// Read and check a swipe path.
fn swipe_path(entry: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<Vec<GesturePoint>> {
    let entry_path = path.key("path");
    let points = crate::fields::list(entry, "path", path, out, "a list of x and y points")?;
    if points.len() < 2 || points.len() > MAX_PATH_POINTS {
        out.problem(
            &entry_path,
            Fault::UnsupportedValue {
                expected: format!("2 to {MAX_PATH_POINTS} points"),
            },
        );
        return None;
    }
    let mut parsed: Vec<GesturePoint> = Vec::with_capacity(points.len());
    for (index, point) in points.iter_mut().enumerate() {
        let point_path = entry_path.entry(index);
        let table = as_table(point, &point_path, out, "a point table with x and y")?;
        unknown_keys(table, &point_path, out, &["x", "y"]);
        let x = crate::fields::number(
            table,
            "x",
            &point_path,
            out,
            -1.0..=MAX_PATH_COORDINATE,
            "a number from -1 to 1",
        );
        let y = crate::fields::number(
            table,
            "y",
            &point_path,
            out,
            -1.0..=MAX_PATH_COORDINATE,
            "a number from -1 to 1",
        );
        let (Some(x), Some(y)) = (x, y) else {
            return None;
        };
        parsed.push(GesturePoint { x, y });
    }
    let starts_at_origin = parsed[0].x == 0.0 && parsed[0].y == 0.0;
    let moves = parsed.windows(2).all(|pair| pair[0] != pair[1]);
    let ends_away = parsed[parsed.len() - 1] != (GesturePoint { x: 0.0, y: 0.0 });
    if !starts_at_origin || !moves || !ends_away {
        out.problem(
            &entry_path,
            Fault::UnsupportedValue {
                expected: "a path that starts at x = 0, y = 0 and moves".to_owned(),
            },
        );
        return None;
    }
    Some(parsed)
}

/// The movement threshold, defaulted per gesture kind.
fn threshold(
    entry: &mut BTreeMap<String, Value>,
    path: &Path,
    out: &mut Parser,
    range: std::ops::RangeInclusive<f64>,
    default: f64,
    expected: &str,
) -> f64 {
    crate::fields::number(entry, "threshold", path, out, range, expected).unwrap_or(default)
}

/// How closely the movement must match the path, for a swipe.
fn tolerance(entry: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<f64> {
    crate::fields::number(
        entry,
        "tolerance",
        path,
        out,
        0.05..=0.5,
        "a tolerance from 0.05 to 0.5",
    )
}

/// Check a gesture command: a nonempty argv whose executable is nonempty.
fn check_command(command: Vec<String>, path: &Path, out: &mut Parser) -> Option<Vec<String>> {
    let executable_present = command.first().is_some_and(|first| !first.is_empty());
    if command.is_empty() || !executable_present {
        out.problem(
            &path.key("command"),
            Fault::UnsupportedValue {
                expected: "a nonempty argv array whose first entry names the program".to_owned(),
            },
        );
        return None;
    }
    Some(command)
}

/// The part of a gesture that decides whether two of them can fire at once.
#[derive(Debug)]
struct GestureInput {
    kind: GestureKind,
    fingers: u8,
    direction: Option<GestureDirection>,
    path: Option<Vec<GesturePoint>>,
    when: GestureWhen,
}

impl GestureInput {
    fn of(gesture: &Gesture) -> GestureInput {
        GestureInput {
            kind: gesture.gesture,
            fingers: gesture.fingers,
            direction: gesture.direction,
            path: gesture.path.clone(),
            when: gesture.when,
        }
    }
}

/// Whether two gestures claim the same movement in an overlapping context.
///
/// Both would reserve input at the start of the same gesture, so only one can
/// run; refusing the document is clearer than picking one.
fn conflicts(first: &GestureInput, second: &GestureInput) -> bool {
    let same_motion = first.kind == second.kind
        && first.fingers == second.fingers
        && match (&first.path, &second.path) {
            (Some(path), Some(other)) => path == other,
            (None, None) => first.direction == second.direction,
            _ => false,
        };
    same_motion && (first.when == GestureWhen::Any || second.when == GestureWhen::Any || first.when == second.when)
}
