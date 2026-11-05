//! Pointer, keyboard and tablet settings (`input`).
//!
//! Every field here can also be the word `"inherit"`, which clears an override
//! so the system preference applies again. That is why each field is an
//! `Option<Inherit<T>>`: "not set in this document" and "set to follow the
//! system" are different instructions once documents are layered, and collapsing
//! them would make a lower layer's value reappear when a higher layer asked for
//! the system setting.
//!
//! Two things are deliberately *not* checked here: whether a keysym name exists
//! and whether a keybinding is a valid chord. Both need libxkbcommon and the
//! chord grammar, which belong to `gnoblin-input`; the document layer stores the
//! text and checks only that it is text.

use std::collections::BTreeMap;

use crate::enums::{
    AccelProfile, ClickMethod, DeviceAccelProfile, Handedness, PadButtonAction, ScrollMethod, StylusAction,
    StylusEraserMode, TabletMapping, TapButtonMap,
};
use crate::fields::{
    Parser, as_table, inherit_flag, inherit_integer, inherit_number, inherit_setting, integer, is_inherit, list,
    require, setting, strings, text, unknown_keys,
};
use crate::path::Path;
use crate::problem::Fault;
use crate::sections::Inherit;
use crate::value::Value;

/// The `input` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    /// Lock the screen orientation, or follow the system setting.
    pub orientation_lock: Option<Inherit<bool>>,
    /// Mouse settings.
    pub mouse: Option<Mouse>,
    /// Touchpad settings.
    pub touchpad: Option<Touchpad>,
    /// Trackball settings.
    pub trackball: Option<Trackball>,
    /// Pointing-stick settings.
    pub pointing_stick: Option<PointingStick>,
    /// Keyboard settings.
    pub keyboard: Option<Keyboard>,
    /// Per-device tablet overrides, keyed by `vvvv:pppp`.
    pub tablets: Option<BTreeMap<String, Tablet>>,
    /// Per-device stylus overrides, keyed by serial or `default-vvvv:pppp`.
    pub styluses: Option<BTreeMap<String, Stylus>>,
}

/// A pointer acceleration curve, in device units.
#[derive(Clone, Debug, PartialEq)]
pub struct AccelCurve {
    /// Device units per millisecond between points.
    pub step: f64,
    /// Output speed at each evenly spaced input speed.
    pub points: Vec<f64>,
}

/// Mouse settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mouse {
    /// Pointer speed, `-1` to `1`.
    pub speed: Option<Inherit<f64>>,
    /// Time between presses that counts as a double click, in milliseconds.
    pub double_click_time: Option<Inherit<u32>>,
    /// Distance before movement becomes a drag, in pixels.
    pub drag_threshold: Option<Inherit<u32>>,
    /// Swap the primary buttons.
    pub left_handed: Option<Inherit<bool>>,
    /// Reverse the scroll direction.
    pub natural_scroll: Option<Inherit<bool>>,
    /// Middle click when both buttons are pressed.
    pub middle_click_emulation: Option<Inherit<bool>>,
    /// The acceleration profile.
    pub accel_profile: Option<Inherit<AccelProfile>>,
    /// The curve `accel-profile = "custom"` uses.
    pub accel_curve: Option<Inherit<AccelCurve>>,
}

/// Touchpad settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Touchpad {
    /// Pointer speed, `-1` to `1`.
    pub speed: Option<Inherit<f64>>,
    /// Scroll speed, `0` to `2`.
    pub scroll_speed: Option<Inherit<f64>>,
    /// Which hand the touchpad is configured for.
    pub left_handed: Option<Inherit<Handedness>>,
    /// Reverse the scroll direction.
    pub natural_scroll: Option<Inherit<bool>>,
    /// Middle click when both buttons are pressed.
    pub middle_click_emulation: Option<Inherit<bool>>,
    /// The acceleration profile.
    pub accel_profile: Option<Inherit<AccelProfile>>,
    /// The curve `accel-profile = "custom"` uses.
    pub accel_curve: Option<Inherit<AccelCurve>>,
    /// Treat a tap as a click.
    pub tap_to_click: Option<Inherit<bool>>,
    /// Which button each tap area reports.
    pub tap_button_map: Option<Inherit<TapButtonMap>>,
    /// Drag by tapping and moving.
    pub tap_and_drag: Option<Inherit<bool>>,
    /// Keep dragging after the next tap.
    pub tap_and_drag_lock: Option<Inherit<bool>>,
    /// Ignore the touchpad while typing.
    pub disable_while_typing: Option<Inherit<bool>>,
    /// How long after a key press the touchpad stays disabled, in milliseconds.
    pub disable_while_typing_timeout: Option<Inherit<u16>>,
    /// Scroll along the touchpad edge.
    pub edge_scrolling_enabled: Option<Inherit<bool>>,
    /// Scroll with two fingers.
    pub two_finger_scrolling_enabled: Option<Inherit<bool>>,
    /// How a tap maps to a button.
    pub click_method: Option<Inherit<ClickMethod>>,
}

/// Trackball settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trackball {
    /// The acceleration profile.
    pub accel_profile: Option<Inherit<DeviceAccelProfile>>,
    /// Middle click when both buttons are pressed.
    pub middle_click_emulation: Option<Inherit<bool>>,
    /// The button that stands in for scroll events, `0` to `24`.
    pub scroll_wheel_emulation_button: Option<Inherit<u8>>,
    /// Hold that button to keep scrolling.
    pub scroll_wheel_emulation_button_lock: Option<Inherit<bool>>,
}

/// Pointing-stick settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointingStick {
    /// Pointer speed, `-1` to `1`.
    pub speed: Option<Inherit<f64>>,
    /// The acceleration profile.
    pub accel_profile: Option<Inherit<DeviceAccelProfile>>,
    /// How the stick scrolls.
    pub scroll_method: Option<Inherit<ScrollMethod>>,
}

/// Keyboard settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keyboard {
    /// Repeat a held key.
    pub repeat: Option<Inherit<bool>>,
    /// Time before repeat starts, in milliseconds.
    pub delay: Option<Inherit<u16>>,
    /// Time between repeated presses, in milliseconds.
    pub repeat_interval: Option<Inherit<u16>>,
    /// Remember the Num Lock state between sessions.
    pub remember_numlock_state: Option<Inherit<bool>>,
    /// Set the Num Lock state while this document is active.
    pub numlock_state: Option<Inherit<bool>>,
    /// The XKB option list, which replaces the active one.
    pub xkb_options: Option<Inherit<Vec<String>>>,
    /// Keyboard accessibility features.
    pub accessibility: Option<Inherit<Accessibility>>,
}

/// Keyboard accessibility features, as Mutter names them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Accessibility {
    /// Enable the keyboard shortcuts that toggle accessibility features.
    pub shortcuts_enabled: Option<Inherit<bool>>,
    /// Beep when a feature changes state.
    pub beep_on_feature_state_change: Option<Inherit<bool>>,
    /// Bounce Keys.
    pub bounce_keys: Option<Inherit<BounceKeys>>,
    /// Mouse Keys.
    pub mouse_keys: Option<Inherit<MouseKeys>>,
    /// Slow Keys.
    pub slow_keys: Option<Inherit<SlowKeys>>,
    /// Sticky Keys.
    pub sticky_keys: Option<Inherit<StickyKeys>>,
    /// Toggle Keys.
    pub toggle_keys: Option<Inherit<ToggleKeys>>,
}

/// Bounce Keys: ignore repeated presses of the same key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BounceKeys {
    /// Enable the feature.
    pub enabled: Option<Inherit<bool>>,
    /// Minimum interval between accepted presses, in milliseconds.
    pub delay_ms: Option<Inherit<u16>>,
    /// Beep when a repeated press is rejected.
    pub beep_on_reject: Option<Inherit<bool>>,
}

/// Mouse Keys: move the pointer with the keyboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MouseKeys {
    /// Enable the feature.
    pub enabled: Option<Inherit<bool>>,
    /// Maximum pointer speed, in pixels per second.
    pub max_speed: Option<Inherit<u16>>,
    /// Time to accelerate from rest to maximum, in milliseconds.
    pub acceleration_time_ms: Option<Inherit<u16>>,
    /// Delay before the pointer starts moving, in milliseconds.
    pub initial_delay_ms: Option<Inherit<u16>>,
}

/// Slow Keys: accept a press only after the key is held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlowKeys {
    /// Enable the feature.
    pub enabled: Option<Inherit<bool>>,
    /// How long the key must be held, in milliseconds.
    pub delay_ms: Option<Inherit<u16>>,
    /// Beep when a key is first pressed.
    pub beep_on_press: Option<Inherit<bool>>,
    /// Beep when a held press is accepted.
    pub beep_on_accept: Option<Inherit<bool>>,
    /// Beep when a press is rejected.
    pub beep_on_reject: Option<Inherit<bool>>,
}

/// Sticky Keys: modifiers apply to one press at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StickyKeys {
    /// Enable the feature.
    pub enabled: Option<Inherit<bool>>,
    /// Disable Sticky Keys when two keys are pressed together.
    pub two_key_off: Option<Inherit<bool>>,
    /// Beep when a modifier is pressed.
    pub beep_on_modifier: Option<Inherit<bool>>,
}

/// Toggle Keys: beep when a locking key toggles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToggleKeys {
    /// Enable the feature.
    pub enabled: Option<Inherit<bool>>,
}

/// One tablet-pad button action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadButton {
    /// The button number, `0` to `255`.
    pub button: u8,
    /// What the button does.
    pub action: PadButtonAction,
    /// The accelerator the `keybinding` action runs.
    pub keybinding: Option<String>,
}

/// One tablet's overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tablet {
    /// How pen movement maps to the pointer.
    pub mapping: Option<Inherit<TabletMapping>>,
    /// `auto`, or the connector id of the output to map to.
    pub output: Option<Inherit<String>>,
    /// Reverse the button orientation.
    pub left_handed: Option<Inherit<bool>>,
    /// Keep the tablet's proportions on a differently shaped display.
    pub keep_aspect: Option<Inherit<bool>>,
    /// The active area as left, right, top and bottom fractions.
    pub area: Option<Inherit<[f64; 4]>>,
    /// Actions for the tablet-pad buttons.
    pub pad_buttons: Option<Inherit<Vec<PadButton>>>,
}

/// One stylus's overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stylus {
    /// What the eraser button does.
    pub eraser_button_mode: Option<Inherit<StylusEraserMode>>,
    /// The eraser button's action, when the mode is `button`.
    pub eraser_button_action: Option<Inherit<StylusAction>>,
    /// The accelerator the eraser button runs.
    pub eraser_button_keybinding: Option<Inherit<String>>,
    /// The primary button's action.
    pub button_action: Option<Inherit<StylusAction>>,
    /// The accelerator the primary button runs.
    pub button_keybinding: Option<Inherit<String>>,
    /// The secondary button's action.
    pub secondary_button_action: Option<Inherit<StylusAction>>,
    /// The accelerator the secondary button runs.
    pub secondary_button_keybinding: Option<Inherit<String>>,
    /// The tertiary button's action.
    pub tertiary_button_action: Option<Inherit<StylusAction>>,
    /// The accelerator the tertiary button runs.
    pub tertiary_button_keybinding: Option<Inherit<String>>,
}

/// The keys `input` accepts at the top level.
const INPUT_KEYS: &[&str] = &[
    "keyboard",
    "mouse",
    "orientation-lock",
    "pointing-stick",
    "styluses",
    "tablets",
    "touchpad",
    "trackball",
];

/// The keys a mouse group accepts.
const MOUSE_KEYS: &[&str] = &[
    "accel-curve",
    "accel-profile",
    "double-click-time",
    "drag-threshold",
    "left-handed",
    "middle-click-emulation",
    "natural-scroll",
    "speed",
];

/// The keys a touchpad group accepts.
const TOUCHPAD_KEYS: &[&str] = &[
    "accel-curve",
    "accel-profile",
    "click-method",
    "disable-while-typing",
    "disable-while-typing-timeout",
    "edge-scrolling-enabled",
    "left-handed",
    "middle-click-emulation",
    "natural-scroll",
    "scroll-speed",
    "speed",
    "tap-and-drag",
    "tap-and-drag-lock",
    "tap-button-map",
    "tap-to-click",
    "two-finger-scrolling-enabled",
];

/// The keys a trackball group accepts.
const TRACKBALL_KEYS: &[&str] = &[
    "accel-profile",
    "middle-click-emulation",
    "scroll-wheel-emulation-button",
    "scroll-wheel-emulation-button-lock",
];

/// The keys a pointing-stick group accepts.
const POINTING_STICK_KEYS: &[&str] = &["accel-profile", "scroll-method", "speed"];

/// The keys a keyboard group accepts.
const KEYBOARD_KEYS: &[&str] = &[
    "accessibility",
    "delay",
    "numlock-state",
    "remember-numlock-state",
    "repeat",
    "repeat-interval",
    "xkb-options",
];

/// The keys a tablet group accepts, per device.
const TABLET_KEYS: &[&str] = &["area", "keep-aspect", "left-handed", "mapping", "output", "pad-buttons"];

/// The keys a stylus group accepts, per device.
const STYLUS_KEYS: &[&str] = &[
    "button-action",
    "button-keybinding",
    "eraser-button-action",
    "eraser-button-keybinding",
    "eraser-button-mode",
    "secondary-button-action",
    "secondary-button-keybinding",
    "tertiary-button-action",
    "tertiary-button-keybinding",
];

/// The keys `input.keyboard.accessibility` accepts.
const ACCESSIBILITY_KEYS: &[&str] = &[
    "beep-on-feature-state-change",
    "bounce-keys",
    "mouse-keys",
    "shortcuts-enabled",
    "slow-keys",
    "sticky-keys",
    "toggle-keys",
];

/// The keys `input.keyboard.accessibility.bounce-keys` accepts.
const BOUNCE_KEYS_KEYS: &[&str] = &["beep-on-reject", "delay-ms", "enabled"];

/// The keys `input.keyboard.accessibility.mouse-keys` accepts.
const MOUSE_KEYS_KEYS: &[&str] = &["acceleration-time-ms", "enabled", "initial-delay-ms", "max-speed"];

/// The keys `input.keyboard.accessibility.slow-keys` accepts.
const SLOW_KEYS_KEYS: &[&str] = &[
    "beep-on-accept",
    "beep-on-press",
    "beep-on-reject",
    "delay-ms",
    "enabled",
];

/// The keys `input.keyboard.accessibility.sticky-keys` accepts.
const STICKY_KEYS_KEYS: &[&str] = &["beep-on-modifier", "enabled", "two-key-off"];

/// The keys `input.keyboard.accessibility.toggle-keys` accepts.
const TOGGLE_KEYS_KEYS: &[&str] = &["enabled"];

/// The keys a tablet-pad button entry accepts.
const PAD_BUTTON_KEYS: &[&str] = &["action", "button", "keybinding"];

/// The keys an `accel-curve` table accepts.
const ACCEL_CURVE_KEYS: &[&str] = &["points", "step"];

/// The most tablet-pad buttons one tablet may remap.
const MAX_PAD_BUTTONS: usize = 256;

/// Parse the `input` table.
pub(crate) fn parse_input(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Input {
    unknown_keys(map, path, out, INPUT_KEYS);
    Input {
        orientation_lock: inherit_flag(map, "orientation-lock", path, out),
        mouse: group(map, "mouse", path, out, MOUSE_KEYS, parse_mouse),
        touchpad: group(map, "touchpad", path, out, TOUCHPAD_KEYS, parse_touchpad),
        trackball: group(map, "trackball", path, out, TRACKBALL_KEYS, parse_trackball),
        pointing_stick: group(
            map,
            "pointing-stick",
            path,
            out,
            POINTING_STICK_KEYS,
            parse_pointing_stick,
        ),
        keyboard: group(map, "keyboard", path, out, KEYBOARD_KEYS, parse_keyboard),
        tablets: devices(map, "tablets", path, out, TABLET_KEYS, parse_tablet, tablet_id_shaped),
        styluses: devices(map, "styluses", path, out, STYLUS_KEYS, parse_stylus, stylus_id_shaped),
    }
}

/// Parse one fixed-key group of `input`.
fn group<'m, T>(
    map: &'m mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    accepted: &'static [&'static str],
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> T,
) -> Option<T> {
    let fields = crate::fields::table(map, key, path, out, "a table of device settings")?;
    let group_path = path.key(key);
    unknown_keys(fields, &group_path, out, accepted);
    Some(parse(fields, &group_path, out))
}

/// Parse a group keyed by device id.
fn devices<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    accepted: &'static [&'static str],
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> T,
    id_shaped: fn(&str) -> bool,
) -> Option<BTreeMap<String, T>> {
    let entries = crate::fields::table(map, key, path, out, "a table keyed by device id")?;
    let group_path = path.key(key);
    let mut devices: BTreeMap<String, T> = BTreeMap::new();
    // Take the keys first: the device tables are borrowed mutably below.
    let ids: Vec<String> = entries.keys().cloned().collect();
    for id in ids {
        let device_path = group_path.key(&id);
        if !id_shaped(&id) {
            let expected = if key == "tablets" {
                "a device id of four hex digits, a colon, and four hex digits"
            } else {
                "a hex serial, or default- followed by four hex digits, a colon, and four hex digits"
            };
            out.problem(
                &device_path,
                Fault::UnsupportedValue {
                    expected: expected.to_owned(),
                },
            );
            continue;
        }
        let Some(fields) = crate::fields::table(&mut *entries, &id, &group_path, out, "a table of device settings")
        else {
            continue;
        };
        unknown_keys(fields, &device_path, out, accepted);
        devices.insert(id, parse(fields, &device_path, out));
    }
    Some(devices)
}

/// Whether a tablet id is `vvvv:pppp` in hex.
fn tablet_id_shaped(id: &str) -> bool {
    match id.split_once(':') {
        Some((vendor, product)) => hex_shaped(vendor, 4) && hex_shaped(product, 4),
        None => false,
    }
}

/// Whether a stylus id is a hex serial or `default-vvvv:pppp`.
fn stylus_id_shaped(id: &str) -> bool {
    if let Some(rest) = id.strip_prefix("default-") {
        return tablet_id_shaped(rest);
    }
    hex_shaped(id, id.len()) && !id.is_empty()
}

/// Whether `text` is exactly `length` hex digits.
fn hex_shaped(text: &str, length: usize) -> bool {
    text.len() == length && text.chars().all(|character| character.is_ascii_hexdigit())
}

/// Parse the mouse group.
fn parse_mouse(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Mouse {
    Mouse {
        speed: speed(map, "speed", path, out),
        double_click_time: inherit_integer(
            map,
            "double-click-time",
            path,
            out,
            0..=i64::from(i32::MAX),
            "an integer from 0 to 2147483647 milliseconds",
        ),
        drag_threshold: inherit_integer(
            map,
            "drag-threshold",
            path,
            out,
            1..=i64::from(i32::MAX),
            "an integer from 1 to 2147483647 pixels",
        ),
        left_handed: inherit_flag(map, "left-handed", path, out),
        natural_scroll: inherit_flag(map, "natural-scroll", path, out),
        middle_click_emulation: inherit_flag(map, "middle-click-emulation", path, out),
        accel_profile: inherit_setting(map, "accel-profile", "accel-profile", path, out),
        accel_curve: accel_curve(map, path, out),
    }
}

/// Parse the touchpad group.
fn parse_touchpad(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Touchpad {
    Touchpad {
        speed: speed(map, "speed", path, out),
        scroll_speed: inherit_number(map, "scroll-speed", path, out, 0.0..=2.0, "a number from 0 to 2"),
        left_handed: inherit_setting(map, "left-handed", "left-handed", path, out),
        natural_scroll: inherit_flag(map, "natural-scroll", path, out),
        middle_click_emulation: inherit_flag(map, "middle-click-emulation", path, out),
        accel_profile: inherit_setting(map, "accel-profile", "accel-profile", path, out),
        accel_curve: accel_curve(map, path, out),
        tap_to_click: inherit_flag(map, "tap-to-click", path, out),
        tap_button_map: inherit_setting(map, "tap-button-map", "tap-button-map", path, out),
        tap_and_drag: inherit_flag(map, "tap-and-drag", path, out),
        tap_and_drag_lock: inherit_flag(map, "tap-and-drag-lock", path, out),
        disable_while_typing: inherit_flag(map, "disable-while-typing", path, out),
        disable_while_typing_timeout: inherit_integer(
            map,
            "disable-while-typing-timeout",
            path,
            out,
            100..=5000,
            "an integer from 100 to 5000 milliseconds",
        ),
        edge_scrolling_enabled: inherit_flag(map, "edge-scrolling-enabled", path, out),
        two_finger_scrolling_enabled: inherit_flag(map, "two-finger-scrolling-enabled", path, out),
        click_method: inherit_setting(map, "click-method", "click-method", path, out),
    }
}

/// Parse the trackball group.
fn parse_trackball(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Trackball {
    Trackball {
        accel_profile: inherit_setting(map, "accel-profile", "device-accel-profile", path, out),
        middle_click_emulation: inherit_flag(map, "middle-click-emulation", path, out),
        scroll_wheel_emulation_button: inherit_integer(
            map,
            "scroll-wheel-emulation-button",
            path,
            out,
            0..=24,
            "an integer from 0 to 24",
        ),
        scroll_wheel_emulation_button_lock: inherit_flag(map, "scroll-wheel-emulation-button-lock", path, out),
    }
}

/// Parse the pointing-stick group.
fn parse_pointing_stick(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> PointingStick {
    PointingStick {
        speed: speed(map, "speed", path, out),
        accel_profile: inherit_setting(map, "accel-profile", "device-accel-profile", path, out),
        scroll_method: inherit_setting(map, "scroll-method", "scroll-method", path, out),
    }
}

/// Parse the keyboard group.
fn parse_keyboard(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Keyboard {
    let accessibility = match map.get_mut("accessibility") {
        None => None,
        Some(value) if is_inherit(value) => Some(Inherit::Inherit),
        Some(value) => {
            let accessibility_path = path.key("accessibility");
            match as_table(value, &accessibility_path, out, "a table of accessibility settings") {
                Some(fields) => {
                    unknown_keys(fields, &accessibility_path, out, ACCESSIBILITY_KEYS);
                    Some(Inherit::Value(parse_accessibility(fields, &accessibility_path, out)))
                }
                None => None,
            }
        }
    };
    Keyboard {
        repeat: inherit_flag(map, "repeat", path, out),
        delay: inherit_integer(
            map,
            "delay",
            path,
            out,
            1..=10_000,
            "an integer from 1 to 10000 milliseconds",
        ),
        repeat_interval: inherit_integer(
            map,
            "repeat-interval",
            path,
            out,
            1..=10_000,
            "an integer from 1 to 10000 milliseconds",
        ),
        remember_numlock_state: inherit_flag(map, "remember-numlock-state", path, out),
        numlock_state: inherit_flag(map, "numlock-state", path, out),
        xkb_options: inherit_strings(map, "xkb-options", path, out),
        accessibility,
    }
}

/// Parse `input.keyboard.accessibility`.
fn parse_accessibility(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Accessibility {
    Accessibility {
        shortcuts_enabled: inherit_flag(map, "shortcuts-enabled", path, out),
        beep_on_feature_state_change: inherit_flag(map, "beep-on-feature-state-change", path, out),
        bounce_keys: feature(map, "bounce-keys", path, out, BOUNCE_KEYS_KEYS, |f, p, o| BounceKeys {
            enabled: inherit_flag(f, "enabled", p, o),
            delay_ms: inherit_integer(
                f,
                "delay-ms",
                p,
                o,
                0..=10_000,
                "an integer from 0 to 10000 milliseconds",
            ),
            beep_on_reject: inherit_flag(f, "beep-on-reject", p, o),
        }),
        mouse_keys: feature(map, "mouse-keys", path, out, MOUSE_KEYS_KEYS, |f, p, o| MouseKeys {
            enabled: inherit_flag(f, "enabled", p, o),
            max_speed: inherit_integer(
                f,
                "max-speed",
                p,
                o,
                1..=10_000,
                "an integer from 1 to 10000 pixels per second",
            ),
            acceleration_time_ms: inherit_integer(
                f,
                "acceleration-time-ms",
                p,
                o,
                1..=10_000,
                "an integer from 1 to 10000 milliseconds",
            ),
            initial_delay_ms: inherit_integer(
                f,
                "initial-delay-ms",
                p,
                o,
                0..=10_000,
                "an integer from 0 to 10000 milliseconds",
            ),
        }),
        slow_keys: feature(map, "slow-keys", path, out, SLOW_KEYS_KEYS, |f, p, o| SlowKeys {
            enabled: inherit_flag(f, "enabled", p, o),
            delay_ms: inherit_integer(
                f,
                "delay-ms",
                p,
                o,
                0..=10_000,
                "an integer from 0 to 10000 milliseconds",
            ),
            beep_on_press: inherit_flag(f, "beep-on-press", p, o),
            beep_on_accept: inherit_flag(f, "beep-on-accept", p, o),
            beep_on_reject: inherit_flag(f, "beep-on-reject", p, o),
        }),
        sticky_keys: feature(map, "sticky-keys", path, out, STICKY_KEYS_KEYS, |f, p, o| StickyKeys {
            enabled: inherit_flag(f, "enabled", p, o),
            two_key_off: inherit_flag(f, "two-key-off", p, o),
            beep_on_modifier: inherit_flag(f, "beep-on-modifier", p, o),
        }),
        toggle_keys: feature(map, "toggle-keys", path, out, TOGGLE_KEYS_KEYS, |f, p, o| ToggleKeys {
            enabled: inherit_flag(f, "enabled", p, o),
        }),
    }
}

/// Parse one accessibility feature group, which is a table or `"inherit"`.
fn feature<T>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
    accepted: &'static [&'static str],
    parse: fn(&mut BTreeMap<String, Value>, &Path, &mut Parser) -> T,
) -> Option<Inherit<T>> {
    let value = map.get_mut(key)?;
    if is_inherit(value) {
        return Some(Inherit::Inherit);
    }
    let feature_path = path.key(key);
    let fields = as_table(value, &feature_path, out, "a table of feature settings")?;
    unknown_keys(fields, &feature_path, out, accepted);
    Some(Inherit::Value(parse(fields, &feature_path, out)))
}

/// Parse one device's tablet settings.
fn parse_tablet(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Tablet {
    Tablet {
        mapping: inherit_setting(map, "mapping", "tablet-mapping", path, out),
        output: inherit_output(map, path, out),
        left_handed: inherit_flag(map, "left-handed", path, out),
        keep_aspect: inherit_flag(map, "keep-aspect", path, out),
        area: tablet_area(map, path, out),
        pad_buttons: pad_buttons(map, path, out),
    }
}

/// Parse one device's stylus settings.
fn parse_stylus(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Stylus {
    Stylus {
        eraser_button_mode: inherit_setting(map, "eraser-button-mode", "eraser-button-mode", path, out),
        eraser_button_action: inherit_setting(map, "eraser-button-action", "stylus-action", path, out),
        eraser_button_keybinding: inherit_text(map, "eraser-button-keybinding", path, out),
        button_action: inherit_setting(map, "button-action", "stylus-action", path, out),
        button_keybinding: inherit_text(map, "button-keybinding", path, out),
        secondary_button_action: inherit_setting(map, "secondary-button-action", "stylus-action", path, out),
        secondary_button_keybinding: inherit_text(map, "secondary-button-keybinding", path, out),
        tertiary_button_action: inherit_setting(map, "tertiary-button-action", "stylus-action", path, out),
        tertiary_button_keybinding: inherit_text(map, "tertiary-button-keybinding", path, out),
    }
}

/// A pointer speed, `-1` to `1`.
fn speed(map: &mut BTreeMap<String, Value>, key: &str, path: &Path, out: &mut Parser) -> Option<Inherit<f64>> {
    inherit_number(map, key, path, out, -1.0..=1.0, "a number from -1 to 1")
}

/// A string setting that accepts `"inherit"`.
fn inherit_text(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<String>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    text(map, key, path, out).map(Inherit::Value)
}

/// A list of strings that accepts `"inherit"`.
fn inherit_strings(
    map: &mut BTreeMap<String, Value>,
    key: &str,
    path: &Path,
    out: &mut Parser,
) -> Option<Inherit<Vec<String>>> {
    if is_inherit(map.get(key)?) {
        return Some(Inherit::Inherit);
    }
    strings(map, key, path, out).map(Inherit::Value)
}

/// Parse `accel-curve`: a step and at least two non-negative points.
fn accel_curve(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<Inherit<AccelCurve>> {
    let value = map.get_mut("accel-curve")?;
    if is_inherit(value) {
        return Some(Inherit::Inherit);
    }
    let curve_path = path.key("accel-curve");
    let fields = as_table(value, &curve_path, out, "a table with step and points")?;
    unknown_keys(fields, &curve_path, out, ACCEL_CURVE_KEYS);
    if !require(fields, "step", &curve_path, out, "a step") {
        return None;
    }
    let step = crate::fields::number(
        fields,
        "step",
        &curve_path,
        out,
        0.0..=f64::MAX,
        "a number greater than 0",
    )?;
    if step <= 0.0 {
        out.problem(
            &curve_path.key("step"),
            Fault::UnsupportedValue {
                expected: "a number greater than 0".to_owned(),
            },
        );
        return None;
    }
    let points = list(fields, "points", &curve_path, out, "a list of at least two numbers")?;
    let points_path = curve_path.key("points");
    if points.len() < 2 {
        out.problem(
            &points_path,
            Fault::UnsupportedValue {
                expected: "at least two points".to_owned(),
            },
        );
        return None;
    }
    let mut speeds: Vec<f64> = Vec::with_capacity(points.len());
    let mut valid = true;
    for (index, point) in points.iter().enumerate() {
        match point.as_number() {
            Some(speed) if speed >= 0.0 => speeds.push(speed),
            Some(_) => {
                out.problem(
                    &points_path.entry(index),
                    Fault::UnsupportedValue {
                        expected: "a number of at least 0".to_owned(),
                    },
                );
                valid = false;
            }
            None => {
                out.problem(
                    &points_path.entry(index),
                    Fault::WrongType {
                        expected: "a number",
                        found: point.kind(),
                    },
                );
                valid = false;
            }
        }
    }
    valid.then_some(Inherit::Value(AccelCurve { step, points: speeds }))
}

/// Parse a tablet output: `"auto"` or a connector id.
fn inherit_output(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<Inherit<String>> {
    match inherit_text(map, "output", path, out)? {
        Inherit::Inherit => Some(Inherit::Inherit),
        Inherit::Value(name) => {
            if name == "auto" || connector_shaped(&name) {
                return Some(Inherit::Value(name));
            }
            out.problem(
                &path.key("output"),
                Fault::UnsupportedValue {
                    expected: "\"auto\" or a connector id of 1 to 64 letters, digits, dots, underscores or dashes"
                        .to_owned(),
                },
            );
            None
        }
    }
}

/// Whether `name` has the shape of an output connector id.
fn connector_shaped(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= 64
        && characters.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

/// Parse a tablet area: four fractions that leave room for each other.
fn tablet_area(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<Inherit<[f64; 4]>> {
    let value = map.get_mut("area")?;
    if is_inherit(value) {
        return Some(Inherit::Inherit);
    }
    let area_path = path.key("area");
    let items = match value {
        Value::List(items) => items,
        other => {
            out.problem(
                &area_path,
                Fault::WrongType {
                    expected: "four fractions or \"inherit\"",
                    found: other.kind(),
                },
            );
            return None;
        }
    };
    if items.len() != 4 {
        out.problem(
            &area_path,
            Fault::UnsupportedValue {
                expected: "four fractions in left, right, top, bottom order".to_owned(),
            },
        );
        return None;
    }
    let mut area = [0.0_f64; 4];
    for (index, item) in items.iter().enumerate() {
        match item.as_number() {
            Some(fraction) if (0.0..1.0).contains(&fraction) => area[index] = fraction,
            Some(_) => {
                out.problem(
                    &area_path.entry(index),
                    Fault::UnsupportedValue {
                        expected: "a fraction of at least 0 and below 1".to_owned(),
                    },
                );
                return None;
            }
            None => {
                out.problem(
                    &area_path.entry(index),
                    Fault::WrongType {
                        expected: "a number",
                        found: item.kind(),
                    },
                );
                return None;
            }
        }
    }
    if area[0] + area[1] >= 1.0 || area[2] + area[3] >= 1.0 {
        out.problem(
            &area_path,
            Fault::UnsupportedValue {
                expected: "opposing edges that add to less than 1".to_owned(),
            },
        );
        return None;
    }
    Some(Inherit::Value(area))
}

/// Parse tablet-pad button actions.
fn pad_buttons(map: &mut BTreeMap<String, Value>, path: &Path, out: &mut Parser) -> Option<Inherit<Vec<PadButton>>> {
    let value = map.get_mut("pad-buttons")?;
    if is_inherit(value) {
        return Some(Inherit::Inherit);
    }
    let buttons_path = path.key("pad-buttons");
    let items = match value {
        Value::List(items) => items,
        other => {
            out.problem(
                &buttons_path,
                Fault::WrongType {
                    expected: "a list of button actions or \"inherit\"",
                    found: other.kind(),
                },
            );
            return None;
        }
    };
    if items.len() > MAX_PAD_BUTTONS {
        out.problem(
            &buttons_path,
            Fault::UnsupportedValue {
                expected: format!("at most {MAX_PAD_BUTTONS} buttons"),
            },
        );
        return None;
    }
    let mut buttons: Vec<PadButton> = Vec::with_capacity(items.len());
    let mut valid = true;
    for (index, item) in items.iter_mut().enumerate() {
        let entry_path = buttons_path.entry(index);
        let Some(entry) = as_table(item, &entry_path, out, "a table with button and action") else {
            valid = false;
            continue;
        };
        unknown_keys(entry, &entry_path, out, PAD_BUTTON_KEYS);
        let number = integer::<u8>(
            entry,
            "button",
            &entry_path,
            out,
            0..=255,
            "a button number from 0 to 255",
        );
        let action = setting::<PadButtonAction>(entry, "action", "tablet-pad-button-action", &entry_path, out);
        let keybinding = inherit_text(entry, "keybinding", &entry_path, out);
        let keybinding = match keybinding {
            None => None,
            Some(Inherit::Value(text)) => Some(text),
            // A button entry has no system default of its own to inherit.
            Some(Inherit::Inherit) => {
                out.problem(
                    &entry_path.key("keybinding"),
                    Fault::UnsupportedValue {
                        expected: "an accelerator string".to_owned(),
                    },
                );
                valid = false;
                None
            }
        };
        match (number, action) {
            (Some(button), Some(action)) => {
                if buttons.iter().any(|existing| existing.button == button) {
                    out.problem(
                        &entry_path.key("button"),
                        Fault::UnsupportedValue {
                            expected: "a button number no other entry uses".to_owned(),
                        },
                    );
                    valid = false;
                }
                match (action, keybinding.as_deref()) {
                    (PadButtonAction::Keybinding, Some(accelerator)) if !accelerator.is_empty() => {}
                    (PadButtonAction::Keybinding, _) => {
                        out.problem(
                            &entry_path,
                            Fault::UnsupportedValue {
                                expected: "a keybinding for the keybinding action".to_owned(),
                            },
                        );
                        valid = false;
                    }
                    (_, Some(_)) => {
                        out.problem(
                            &entry_path.key("keybinding"),
                            Fault::UnsupportedValue {
                                expected: "a keybinding only with the keybinding action".to_owned(),
                            },
                        );
                        valid = false;
                    }
                    (_, None) => {}
                }
                buttons.push(PadButton {
                    button,
                    action,
                    keybinding,
                });
            }
            _ => valid = false,
        }
    }
    valid.then_some(Inherit::Value(buttons))
}
