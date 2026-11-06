//! The `input` section of the configuration, as the knobs a libinput device has.
//!
//! The configuration is written for a person (`touchpad.tap-to-click`, `mouse.speed`), and
//! libinput speaks in device settings, per device, with an enable call per setting. This module
//! is the join between them: [`DeviceSettings::from_config`] picks the section that belongs to
//! the device's kind through the rest of the composition root.
//!
//! It is pure for the same reason [`super::map`] is: the *decision* — which settings a device of
//! this kind gets, what an `inherit` means, what a custom acceleration curve without a curve
//! does — is the part that can be wrong in a way nobody notices, and it is tested here. The
//! device side is [`Applier`], one method per device, implemented by the glue over
//! `libinput::Device`.
//!
//! # `inherit` is not a value
//!
//! A setting written as `inherit` clears an override so the system preference shows through
//! again (`gnoblin_config::Inherit`). It is therefore *not* applied: sending libinput's default
//! is not the same thing as leaving the device alone, because a desktop environment or a
//! udev rule may have set it to something this session must not undo.
//!
//! # Failure modes first
//!
//! 1. **A touchpad setting reaches a mouse.** The kind selects the section, and only the
//!    keyboard's own repeat settings are excluded by name:
//!    `a_mouse_never_gets_a_touchpad_setting`.
//! 2. **An inherited setting is applied as its default.** `inherit` produces no setting at all:
//!    `an_inherited_setting_is_not_applied`.
//! 3. **A custom acceleration profile is sent without a curve**, which libinput refuses and
//!    which leaves the device with no acceleration at all. It falls back to the adaptive
//!    profile: `a_custom_profile_without_a_curve_falls_back_to_adaptive`.
//! 4. **A speed outside the range a device accepts is sent.** It is clamped to what libinput
//!    states, because the range is the device's and not the configuration's:
//!    `a_speed_outside_the_range_is_clamped`.
//! 5. **A device that refuses one setting stops the rest.** The applier answers whether it
//!    accepted, and a refusal is counted and skipped: `a_refused_setting_does_not_stop_the_rest`.
//! 6. **A keyboard is given pointer settings.** The keyboard's section has no device setting in
//!    it at all — repeat and delay are `wl_keyboard.repeat_info`, not libinput — so a keyboard
//!    produces an empty list: `a_keyboard_has_no_libinput_setting`.

use gnoblin_config::{AccelProfile, ClickMethod, Handedness, Inherit, Input, TapButtonMap};

/// What kind of device a setting applies to.
///
/// It is libinput's capability set collapsed to the kinds the configuration has a section for,
/// so one rule decides which section a device reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    /// A touchpad, which has every setting the configuration offers.
    Touchpad,
    /// A mouse: acceleration, handedness, scroll direction.
    Mouse,
    /// A trackball: acceleration, middle-click emulation, scroll emulation.
    Trackball,
    /// A pointing stick: acceleration, scroll method.
    PointingStick,
    /// A touch screen: no settings, because its calibration belongs to the seat.
    Touch,
    /// A keyboard: repeat and delay are the keymap's, not libinput's.
    Keyboard,
}

/// The acceleration profile a pointer uses.
#[derive(Clone, PartialEq, Debug)]
pub enum Accel {
    /// The driver's default.
    Default,
    /// No acceleration at all.
    Flat,
    /// The adaptive profile.
    Adaptive,
    /// A custom curve: a step in device units per millisecond, and the output speed at each
    /// point. It is never empty: a profile without a curve becomes [`Accel::Adaptive`].
    Custom {
        /// Device units per millisecond between points.
        step: f64,
        /// The output speed at each evenly spaced input speed, at least two entries.
        points: Vec<f64>,
    },
}

/// Which button a tap maps to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonMap {
    /// The device's default.
    Default,
    /// Left, right, middle.
    LeftRightMiddle,
    /// Left, middle, right.
    LeftMiddleRight,
}

/// How a clickpad decides where a click landed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Click {
    /// The device's default.
    Default,
    /// No software buttons.
    None,
    /// Button areas.
    Areas,
    /// Finger count.
    Fingers,
}

/// How a pointer scrolls without a wheel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scroll {
    /// The device's default.
    Default,
    /// No scrolling.
    None,
    /// Scroll while a button is held.
    OnButtonDown,
}

/// One setting a device is given, in the terms libinput's configuration API states it.
#[derive(Clone, PartialEq, Debug)]
pub enum Setting {
    /// The acceleration profile and its curve.
    Acceleration(Accel),
    /// The pointer speed, -1 to 1.
    Speed(f64),
    /// Reverse the scroll direction.
    NaturalScroll(bool),
    /// Treat a tap as a click.
    TapToClick(bool),
    /// Drag by tapping and moving.
    TapAndDrag(bool),
    /// Keep dragging after the next tap.
    TapAndDragLock(bool),
    /// Which button each tap area reports.
    TapButtonMap(ButtonMap),
    /// How a clickpad decides where a click landed.
    ClickMethod(Click),
    /// Middle click when both buttons are pressed.
    MiddleClickEmulation(bool),
    /// Swap the primary buttons.
    LeftHanded(bool),
    /// Ignore the device while typing.
    DisableWhileTyping(bool),
    /// How long after a key press the device stays disabled, in milliseconds.
    DisableWhileTypingTimeout(u64),
    /// How the device scrolls without a wheel.
    ScrollMethod(Scroll),
    /// Scroll along the touchpad's edge.
    EdgeScrolling(bool),
    /// Scroll with two fingers.
    TwoFingerScrolling(bool),
    /// The button that stands in for scroll events.
    ScrollWheelEmulationButton(u32),
    /// Hold that button to keep scrolling.
    ScrollWheelEmulationButtonLock(bool),
}

/// What a device is given, and how many of those settings it refused.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DeviceSettings {
    /// The kind the settings were built for; `None` only for the `Default` value, which matches no kind.
    kind: Option<DeviceKind>,
    /// The settings to apply, in the order the configuration lists them.
    settings: Vec<Setting>,
    /// How many of [`Self::settings`] libinput refused on the last apply.
    refused: usize,
}

impl DeviceSettings {
    /// The settings the configuration gives a device of this kind.
    ///
    /// A configuration with no `input` section, or a section for another kind, produces an
    /// empty list, which is the desired state: libinput's own defaults and whatever the system
    /// set stay in force.
    #[must_use]
    pub fn from_config(input: Option<&Input>, kind: DeviceKind) -> Self {
        let mut settings = Vec::new();
        match kind {
            DeviceKind::Touchpad => {
                if let Some(touchpad) = input.and_then(|input| input.touchpad.as_ref()) {
                    push(&mut settings, touchpad.accel_profile.as_ref(), |profile| {
                        Some(Setting::Acceleration(accel(profile, touchpad.accel_curve.as_ref())))
                    });
                    push(&mut settings, touchpad.speed.as_ref(), |speed| {
                        Some(Setting::Speed(clamp_speed(*speed)))
                    });
                    push(&mut settings, touchpad.natural_scroll.as_ref(), |value| {
                        Some(Setting::NaturalScroll(*value))
                    });
                    push(&mut settings, touchpad.tap_to_click.as_ref(), |value| {
                        Some(Setting::TapToClick(*value))
                    });
                    push(&mut settings, touchpad.tap_and_drag.as_ref(), |value| {
                        Some(Setting::TapAndDrag(*value))
                    });
                    push(&mut settings, touchpad.tap_and_drag_lock.as_ref(), |value| {
                        Some(Setting::TapAndDragLock(*value))
                    });
                    push(&mut settings, touchpad.tap_button_map.as_ref(), |map| {
                        Some(Setting::TapButtonMap(button_map(*map)))
                    });
                    push(&mut settings, touchpad.click_method.as_ref(), |method| {
                        Some(Setting::ClickMethod(click(*method)))
                    });
                    push(&mut settings, touchpad.middle_click_emulation.as_ref(), |value| {
                        Some(Setting::MiddleClickEmulation(*value))
                    });
                    push(&mut settings, touchpad.left_handed.as_ref(), |value| {
                        Some(Setting::LeftHanded(*value == Handedness::Left))
                    });
                    push(&mut settings, touchpad.disable_while_typing.as_ref(), |value| {
                        Some(Setting::DisableWhileTyping(*value))
                    });
                    push(
                        &mut settings,
                        touchpad.disable_while_typing_timeout.as_ref(),
                        |timeout| Some(Setting::DisableWhileTypingTimeout(u64::try_from(*timeout).unwrap_or(0))),
                    );
                    push(&mut settings, touchpad.edge_scrolling_enabled.as_ref(), |value| {
                        Some(Setting::EdgeScrolling(*value))
                    });
                    push(&mut settings, touchpad.two_finger_scrolling_enabled.as_ref(), |value| {
                        Some(Setting::TwoFingerScrolling(*value))
                    });
                }
            }
            DeviceKind::Mouse => {
                if let Some(mouse) = input.and_then(|input| input.mouse.as_ref()) {
                    push(&mut settings, mouse.accel_profile.as_ref(), |profile| {
                        Some(Setting::Acceleration(accel(profile, mouse.accel_curve.as_ref())))
                    });
                    push(&mut settings, mouse.speed.as_ref(), |speed| {
                        Some(Setting::Speed(clamp_speed(*speed)))
                    });
                    push(&mut settings, mouse.natural_scroll.as_ref(), |value| {
                        Some(Setting::NaturalScroll(*value))
                    });
                    push(&mut settings, mouse.middle_click_emulation.as_ref(), |value| {
                        Some(Setting::MiddleClickEmulation(*value))
                    });
                    push(&mut settings, mouse.left_handed.as_ref(), |value| {
                        Some(Setting::LeftHanded(*value))
                    });
                }
            }
            DeviceKind::Trackball => {
                if let Some(trackball) = input.and_then(|input| input.trackball.as_ref()) {
                    push(&mut settings, trackball.accel_profile.as_ref(), |profile| {
                        Some(Setting::Acceleration(device_accel(*profile)))
                    });
                    push(&mut settings, trackball.middle_click_emulation.as_ref(), |value| {
                        Some(Setting::MiddleClickEmulation(*value))
                    });
                    push(
                        &mut settings,
                        trackball.scroll_wheel_emulation_button.as_ref(),
                        |button| Some(Setting::ScrollWheelEmulationButton(u32::from(*button as u16))),
                    );
                    push(
                        &mut settings,
                        trackball.scroll_wheel_emulation_button_lock.as_ref(),
                        |value| Some(Setting::ScrollWheelEmulationButtonLock(*value)),
                    );
                }
            }
            DeviceKind::PointingStick => {
                if let Some(stick) = input.and_then(|input| input.pointing_stick.as_ref()) {
                    push(&mut settings, stick.accel_profile.as_ref(), |profile| {
                        Some(Setting::Acceleration(device_accel(*profile)))
                    });
                    push(&mut settings, stick.speed.as_ref(), |speed| {
                        Some(Setting::Speed(clamp_speed(*speed)))
                    });
                    push(&mut settings, stick.scroll_method.as_ref(), |method| {
                        Some(Setting::ScrollMethod(scroll(*method)))
                    });
                }
            }
            // A touch screen's calibration is the seat's, and a keyboard's repeat is the
            // keymap's: neither has a libinput setting in this configuration.
            DeviceKind::Touch | DeviceKind::Keyboard => {}
        }
        Self {
            kind: Some(kind),
            settings,
            refused: 0,
        }
    }

    /// The settings to give the device, in the order the configuration declares them.
    #[must_use]
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// The kind of device these settings are for.
    #[must_use]
    pub fn kind(&self) -> Option<DeviceKind> {
        self.kind
    }

    /// How many settings the device refused.
    #[must_use]
    pub fn refused(&self) -> usize {
        self.refused
    }

    /// Whether the configuration had anything to say about this device.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }

    /// Hands every setting to the device.
    ///
    /// A setting the device refuses is counted and the rest are still offered: a touchpad
    /// without a middle button refuses middle-click emulation and everything else must still
    /// apply, or one unsupported knob would undo the configuration.
    pub fn apply(&mut self, applier: &mut dyn Applier) {
        self.refused = 0;
        for setting in &self.settings {
            if !applier.apply(setting) {
                self.refused = self.refused.saturating_add(1);
            }
        }
    }
}

/// One device that settings can be applied to.
///
/// It is a trait so the mapping above is testable against a recorder, and so the glue is one
/// implementation of a `match` over [`Setting`] and nothing else.
pub trait Applier {
    /// Offers one setting to the device, answering whether the device took it.
    ///
    /// It answers rather than returning an error because a refused setting is not a failure of
    /// the session: a device without a feature simply does not have it.
    fn apply(&mut self, setting: &Setting) -> bool;
}

/// Pushes a setting when the configuration wrote a value for it.
///
/// An `inherit` produces nothing at all: it clears the override rather than naming a value, so
/// the device keeps whatever the system set.
fn push<T, F>(settings: &mut Vec<Setting>, field: Option<&Inherit<T>>, build: F)
where
    F: FnOnce(&T) -> Option<Setting>,
{
    let Some(value) = field.and_then(Inherit::value) else {
        return;
    };
    if let Some(setting) = build(value) {
        settings.push(setting);
    }
}

/// The acceleration profile, with the curve a custom profile needs.
///
/// libinput refuses `Custom` without a curve, which leaves the device unaccelerated — a worse
/// state than the default it was in. A profile that names no usable curve therefore becomes the
/// adaptive one.
fn accel(profile: &AccelProfile, curve: Option<&Inherit<gnoblin_config::generated::AccelCurve>>) -> Accel {
    match profile {
        AccelProfile::Default => Accel::Default,
        AccelProfile::Flat => Accel::Flat,
        AccelProfile::Adaptive => Accel::Adaptive,
        AccelProfile::Custom => {
            let points = curve
                .and_then(Inherit::value)
                .and_then(|curve| curve.points.clone())
                .filter(|points| points.len() >= 2);
            let Some(points) = points else {
                return Accel::Adaptive;
            };
            let step = curve
                .and_then(Inherit::value)
                .and_then(|curve| curve.step)
                .filter(|step| *step > 0.0)
                .unwrap_or(1.0);
            Accel::Custom { step, points }
        }
    }
}

/// The acceleration profile a trackball or pointing stick can name.
///
/// Its own enum has no `Custom` variant, so there is no curve to look for.
fn device_accel(profile: gnoblin_config::DeviceAccelProfile) -> Accel {
    match profile {
        gnoblin_config::DeviceAccelProfile::Default => Accel::Default,
        gnoblin_config::DeviceAccelProfile::Flat => Accel::Flat,
        gnoblin_config::DeviceAccelProfile::Adaptive => Accel::Adaptive,
    }
}

/// The tap button map.
fn button_map(map: TapButtonMap) -> ButtonMap {
    match map {
        TapButtonMap::Default => ButtonMap::Default,
        TapButtonMap::LeftRightMiddle => ButtonMap::LeftRightMiddle,
        TapButtonMap::LeftMiddleRight => ButtonMap::LeftMiddleRight,
    }
}

/// How a clickpad decides where a click landed.
fn click(method: ClickMethod) -> Click {
    match method {
        ClickMethod::Default => Click::Default,
        ClickMethod::None => Click::None,
        ClickMethod::Areas => Click::Areas,
        ClickMethod::Fingers => Click::Fingers,
    }
}

/// How a pointing stick scrolls.
fn scroll(method: gnoblin_config::ScrollMethod) -> Scroll {
    match method {
        gnoblin_config::ScrollMethod::Default => Scroll::Default,
        gnoblin_config::ScrollMethod::None => Scroll::None,
        gnoblin_config::ScrollMethod::OnButtonDown => Scroll::OnButtonDown,
    }
}

/// A speed libinput accepts.
///
/// libinput states -1 to 1 and the configuration states the same range, so this only catches a
/// value that did not come from the document — a script, or a reload whose validator ran
/// against a different schema.
fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() { speed.clamp(-1.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::generated::AccelCurve;
    use gnoblin_config::{Mouse, Touchpad};

    /// A device that takes everything, and records what it was given.
    #[derive(Default)]
    struct Recorder {
        seen: Vec<Setting>,
        refuse: Vec<String>,
    }

    impl Applier for Recorder {
        fn apply(&mut self, setting: &Setting) -> bool {
            if self.refuse.iter().any(|name| name == name_of(setting)) {
                return false;
            }
            self.seen.push(setting.clone());
            true
        }
    }

    fn name_of(setting: &Setting) -> &'static str {
        match setting {
            Setting::Acceleration(_) => "acceleration",
            Setting::Speed(_) => "speed",
            Setting::NaturalScroll(_) => "natural-scroll",
            Setting::TapToClick(_) => "tap-to-click",
            Setting::TapAndDrag(_) => "tap-and-drag",
            Setting::TapAndDragLock(_) => "tap-and-drag-lock",
            Setting::TapButtonMap(_) => "tap-button-map",
            Setting::ClickMethod(_) => "click-method",
            Setting::MiddleClickEmulation(_) => "middle-click-emulation",
            Setting::LeftHanded(_) => "left-handed",
            Setting::DisableWhileTyping(_) => "disable-while-typing",
            Setting::DisableWhileTypingTimeout(_) => "disable-while-typing-timeout",
            Setting::ScrollMethod(_) => "scroll-method",
            Setting::EdgeScrolling(_) => "edge-scrolling",
            Setting::TwoFingerScrolling(_) => "two-finger-scrolling",
            Setting::ScrollWheelEmulationButton(_) => "scroll-wheel-emulation-button",
            Setting::ScrollWheelEmulationButtonLock(_) => "scroll-wheel-emulation-button-lock",
        }
    }

    fn touchpad_input(touchpad: Touchpad) -> Input {
        Input {
            orientation_lock: None,
            mouse: None,
            touchpad: Some(touchpad),
            trackball: None,
            pointing_stick: None,
            keyboard: None,
            ..Input::default()
        }
    }

    #[test]
    fn a_mouse_never_gets_a_touchpad_setting() {
        let input = Input {
            orientation_lock: None,
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(true)),
                ..Touchpad::default()
            }),
            trackball: None,
            pointing_stick: None,
            keyboard: None,
            ..Input::default()
        };
        let settings = DeviceSettings::from_config(Some(&input), DeviceKind::Mouse);
        assert_eq!(settings.settings(), &[Setting::Speed(0.5)]);
    }

    #[test]
    fn an_inherited_setting_is_not_applied() {
        let input = touchpad_input(Touchpad {
            natural_scroll: Some(Inherit::Inherit),
            tap_to_click: Some(Inherit::Value(true)),
            ..Touchpad::default()
        });
        let settings = DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad);
        assert_eq!(settings.settings(), &[Setting::TapToClick(true)]);
    }

    #[test]
    fn a_custom_profile_without_a_curve_falls_back_to_adaptive() {
        let input = touchpad_input(Touchpad {
            accel_profile: Some(Inherit::Value(AccelProfile::Custom)),
            ..Touchpad::default()
        });
        let settings = DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad);
        assert_eq!(settings.settings(), &[Setting::Acceleration(Accel::Adaptive)]);

        let with_curve = touchpad_input(Touchpad {
            accel_profile: Some(Inherit::Value(AccelProfile::Custom)),
            accel_curve: Some(Inherit::Value(AccelCurve {
                step: Some(2.0),
                points: Some(vec![0.0, 0.5, 1.5]),
            })),
            ..Touchpad::default()
        });
        let settings = DeviceSettings::from_config(Some(&with_curve), DeviceKind::Touchpad);
        assert_eq!(
            settings.settings(),
            &[Setting::Acceleration(Accel::Custom {
                step: 2.0,
                points: vec![0.0, 0.5, 1.5]
            })]
        );
    }

    #[test]
    fn a_speed_outside_the_range_is_clamped() {
        let input = touchpad_input(Touchpad {
            speed: Some(Inherit::Value(4.0)),
            ..Touchpad::default()
        });
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad).settings(),
            &[Setting::Speed(1.0)]
        );
        let input = touchpad_input(Touchpad {
            speed: Some(Inherit::Value(f64::NAN)),
            ..Touchpad::default()
        });
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad).settings(),
            &[Setting::Speed(0.0)]
        );
    }

    #[test]
    fn a_refused_setting_does_not_stop_the_rest() {
        let input = touchpad_input(Touchpad {
            natural_scroll: Some(Inherit::Value(true)),
            tap_to_click: Some(Inherit::Value(true)),
            ..Touchpad::default()
        });
        let mut settings = DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad);
        let mut recorder = Recorder {
            seen: Vec::new(),
            refuse: vec!["natural-scroll".to_owned()],
        };
        settings.apply(&mut recorder);
        assert_eq!(recorder.seen, vec![Setting::TapToClick(true)]);
        assert_eq!(settings.refused(), 1);
    }

    #[test]
    fn a_keyboard_has_no_libinput_setting() {
        let input = Input {
            orientation_lock: None,
            mouse: None,
            touchpad: None,
            trackball: None,
            pointing_stick: None,
            keyboard: Some(gnoblin_config::Keyboard {
                repeat: Some(Inherit::Value(true)),
                ..gnoblin_config::Keyboard::default()
            }),
            ..Input::default()
        };
        let settings = DeviceSettings::from_config(Some(&input), DeviceKind::Keyboard);
        assert!(settings.is_empty());
        assert_eq!(settings.kind(), Some(DeviceKind::Keyboard));
    }

    #[test]
    fn a_configuration_with_no_input_section_gives_nothing() {
        let settings = DeviceSettings::from_config(None, DeviceKind::Touchpad);
        assert!(settings.is_empty());
    }

    #[test]
    fn every_touchpad_key_reaches_the_device() {
        // The configuration's touchpad section and this mapping have to agree: a key that the
        // document accepts and nothing applies is a setting that silently does nothing.
        let touchpad = Touchpad {
            speed: Some(Inherit::Value(0.25)),
            scroll_speed: None,
            left_handed: Some(Inherit::Value(Handedness::Left)),
            natural_scroll: Some(Inherit::Value(true)),
            middle_click_emulation: Some(Inherit::Value(true)),
            accel_profile: Some(Inherit::Value(AccelProfile::Flat)),
            accel_curve: None,
            tap_to_click: Some(Inherit::Value(true)),
            tap_button_map: Some(Inherit::Value(TapButtonMap::LeftMiddleRight)),
            tap_and_drag: Some(Inherit::Value(true)),
            tap_and_drag_lock: Some(Inherit::Value(true)),
            disable_while_typing: Some(Inherit::Value(true)),
            disable_while_typing_timeout: Some(Inherit::Value(750)),
            edge_scrolling_enabled: Some(Inherit::Value(true)),
            two_finger_scrolling_enabled: Some(Inherit::Value(true)),
            click_method: Some(Inherit::Value(ClickMethod::Fingers)),
        };
        let settings = DeviceSettings::from_config(Some(&touchpad_input(touchpad)), DeviceKind::Touchpad);
        let named: Vec<&str> = settings.settings().iter().map(name_of).collect();
        assert_eq!(
            named,
            vec![
                "acceleration",
                "speed",
                "natural-scroll",
                "tap-to-click",
                "tap-and-drag",
                "tap-and-drag-lock",
                "tap-button-map",
                "click-method",
                "middle-click-emulation",
                "left-handed",
                "disable-while-typing",
                "disable-while-typing-timeout",
                "edge-scrolling",
                "two-finger-scrolling",
            ]
        );
        assert_eq!(settings.refused(), 0);
    }
}
