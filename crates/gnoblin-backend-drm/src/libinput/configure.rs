//! The configuration calls libinput takes, and the one place a [`Setting`] becomes them.
//!
//! [`gnoblin_backend::input_settings`] decides *what* a device is given. This module is *how*: it turns each
//! [`Setting`] into the calls libinput's configuration API has ([`DeviceCall`]), and runs them through [`Execute`].
//! The real device is one implementation of [`Execute`], and a recorder in the tests is the other, so the
//! sequence of calls a document produces, per device and per reload, is proved on a machine with no
//! input device.
//!
//! # What this build does not ask a device for
//!
//! The libinput binding is built for libinput 1.19 and up (`design/rust-compositor.md`, "Distribution and
//! toolchain"), and some settings need newer calls. They are refused with [`SettingRefusal::Unavailable`] and a
//! reason, which is not the device's refusal: the same device would take them on a build with a newer floor.
//!
//! - A **custom acceleration curve** is libinput 1.23.
//! - **Software buttons off** (`click-method = none`) has no value in the binding's click method.
//! - The **disable-while-typing timeout** is libinput 1.31.
//! - A **scroll button** the document numbers as X's scroll wheel (4 to 7), or past 9, has no evdev button.
//!
//! # Failure modes first
//!
//! 1. **A document value reaches libinput unmapped.** The scroll button is numbered as the toolkits number
//!    it (2 is the middle button) and libinput wants an evdev code: `a_scroll_button_is_sent_as_its_evdev_code`.
//! 2. **A device that cannot scroll by button is told its method before its button.** The button comes
//!    first, because the method is what turns the device on: `a_scroll_button_is_set_before_the_method_that_uses_it`.
//! 3. **A custom curve is sent where it cannot be made.** It is refused with its reason and the device keeps
//!    the profile it had: `a_custom_curve_is_refused_with_its_reason`.
//! 4. **A removed setting stays in the device.** It is restored to the device's own default:
//!    `a_removed_setting_is_restored_to_the_devices_default`.
//! 5. **The touchpad scroll speed reaches libinput.** It is the seat's multiplier and no libinput call:
//!    `the_scroll_speed_is_the_seats_and_no_call`.

use smithay::reexports::input as libinput;

use gnoblin_backend::input_settings::{
    Accel, Applier, ButtonMap, Click, Scroll, Setting, SettingKind, SettingRefusal, evdev_button,
};

/// The profile of a [`DeviceCall::AccelProfile`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
    /// No acceleration.
    Flat,
    /// The adaptive profile.
    Adaptive,
}

/// The click method of a [`DeviceCall::ClickMethod`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClickKind {
    /// Software button areas.
    Areas,
    /// The finger count decides the button.
    Fingers,
}

/// The scroll method of a [`DeviceCall::ScrollMethod`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollKind {
    /// No scroll events in place of motion.
    Off,
    /// Scroll while a button is held.
    OnButtonDown,
    /// Scroll with two fingers.
    TwoFinger,
    /// Scroll along the edge.
    Edge,
}

/// One call of libinput's device configuration API.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DeviceCall {
    /// `libinput_device_config_accel_set_profile`.
    AccelProfile(Profile),
    /// `libinput_device_config_accel_set_speed`.
    AccelSpeed(f64),
    /// `libinput_device_config_scroll_set_natural_scroll_enabled`.
    NaturalScroll(bool),
    /// `libinput_device_config_tap_set_enabled`.
    Tap(bool),
    /// `libinput_device_config_tap_set_drag_enabled`.
    TapDrag(bool),
    /// `libinput_device_config_tap_set_drag_lock_enabled`.
    TapDragLock(bool),
    /// `libinput_device_config_tap_set_button_map`.
    TapButtonMap(ButtonMap),
    /// `libinput_device_config_click_set_method`.
    ClickMethod(ClickKind),
    /// `libinput_device_config_middle_emulation_set_enabled`.
    MiddleEmulation(bool),
    /// `libinput_device_config_left_handed_set`.
    LeftHanded(bool),
    /// `libinput_device_config_dwt_set_enabled`.
    DisableWhileTyping(bool),
    /// `libinput_device_config_scroll_set_method`.
    ScrollMethod(ScrollKind),
    /// `libinput_device_config_scroll_set_button`, with an evdev code.
    ScrollButton(u32),
    /// `libinput_device_config_scroll_set_button_lock`.
    ScrollButtonLock(bool),
    /// The setting's `libinput_device_config_*_set_*` call with the value of the matching `*_get_default_*`
    /// call, which is what puts a device back as it started.
    Restore(SettingKind),
}

/// A device libinput's configuration calls can be made on.
pub trait Execute {
    /// Makes one call on the device.
    ///
    /// # Errors
    ///
    /// [`SettingRefusal`] when libinput answers unsupported or invalid.
    fn call(&mut self, call: DeviceCall) -> Result<(), SettingRefusal>;
}

/// A libinput device that is given the document's settings.
///
/// It owns the device handle and the one setting that is not libinput's: the scroll speed, which the seat
/// applies to the touchpad's scroll distance ([`Configurator::scroll_speed`]).
pub struct Configurator<E> {
    device: E,
    /// The scroll multiplier the code that translates the device's scroll events applies.
    scroll_speed: f64,
}

impl<E: Execute> Configurator<E> {
    /// A device with no setting applied: its scroll is not scaled.
    #[must_use]
    pub fn new(device: E) -> Self {
        Self {
            device,
            scroll_speed: 1.0,
        }
    }

    /// The device the calls are made on.
    #[must_use]
    pub fn device(&self) -> &E {
        &self.device
    }

    /// How much the device's scroll distance is multiplied by: 1 unless `touchpad.scroll-speed` says otherwise.
    #[must_use]
    pub fn scroll_speed(&self) -> f64 {
        self.scroll_speed
    }

    fn call(&mut self, call: DeviceCall) -> Result<(), SettingRefusal> {
        self.device.call(call)
    }
}

impl<E: Execute> Applier for Configurator<E> {
    fn apply(&mut self, setting: &Setting) -> Result<(), SettingRefusal> {
        match setting {
            Setting::Acceleration(Accel::Default) => self.call(DeviceCall::Restore(SettingKind::Acceleration)),
            Setting::Acceleration(Accel::Flat) => self.call(DeviceCall::AccelProfile(Profile::Flat)),
            Setting::Acceleration(Accel::Adaptive) => self.call(DeviceCall::AccelProfile(Profile::Adaptive)),
            Setting::Acceleration(Accel::Custom { .. }) => Err(SettingRefusal::Unavailable(
                "a custom acceleration curve needs libinput 1.23, and this build is made for 1.19",
            )),
            Setting::Speed(speed) => self.call(DeviceCall::AccelSpeed(*speed)),
            Setting::ScrollSpeed(factor) => {
                self.scroll_speed = *factor;
                Ok(())
            }
            Setting::NaturalScroll(on) => self.call(DeviceCall::NaturalScroll(*on)),
            Setting::TapToClick(on) => self.call(DeviceCall::Tap(*on)),
            Setting::TapAndDrag(on) => self.call(DeviceCall::TapDrag(*on)),
            Setting::TapAndDragLock(on) => self.call(DeviceCall::TapDragLock(*on)),
            Setting::TapButtonMap(ButtonMap::Default) => self.call(DeviceCall::Restore(SettingKind::TapButtonMap)),
            Setting::TapButtonMap(map) => self.call(DeviceCall::TapButtonMap(*map)),
            Setting::ClickMethod(Click::Default) => self.call(DeviceCall::Restore(SettingKind::ClickMethod)),
            Setting::ClickMethod(Click::None) => Err(SettingRefusal::Unavailable(
                "the libinput binding has no way to turn software buttons off",
            )),
            Setting::ClickMethod(Click::Areas) => self.call(DeviceCall::ClickMethod(ClickKind::Areas)),
            Setting::ClickMethod(Click::Fingers) => self.call(DeviceCall::ClickMethod(ClickKind::Fingers)),
            Setting::MiddleClickEmulation(on) => self.call(DeviceCall::MiddleEmulation(*on)),
            Setting::LeftHanded(on) => self.call(DeviceCall::LeftHanded(*on)),
            Setting::DisableWhileTyping(on) => self.call(DeviceCall::DisableWhileTyping(*on)),
            Setting::DisableWhileTypingTimeout(_) => Err(SettingRefusal::Unavailable(
                "the disable-while-typing timeout needs libinput 1.31, and this build is made for 1.19",
            )),
            Setting::ScrollMethod(Scroll::Default) => self.call(DeviceCall::Restore(SettingKind::ScrollMethod)),
            Setting::ScrollMethod(Scroll::None) => self.call(DeviceCall::ScrollMethod(ScrollKind::Off)),
            Setting::ScrollMethod(Scroll::OnButtonDown) => {
                self.call(DeviceCall::ScrollMethod(ScrollKind::OnButtonDown))
            }
            Setting::ScrollMethod(Scroll::TwoFinger) => self.call(DeviceCall::ScrollMethod(ScrollKind::TwoFinger)),
            Setting::ScrollMethod(Scroll::Edge) => self.call(DeviceCall::ScrollMethod(ScrollKind::Edge)),
            // No button is no button scrolling. A button is set before the method that uses it.
            Setting::ScrollWheelEmulationButton(0) => self.call(DeviceCall::ScrollMethod(ScrollKind::Off)),
            Setting::ScrollWheelEmulationButton(number) => {
                let code = evdev_button(*number).ok_or(SettingRefusal::Unavailable(
                    "the document's button number has no evdev button",
                ))?;
                self.call(DeviceCall::ScrollButton(code))?;
                self.call(DeviceCall::ScrollMethod(ScrollKind::OnButtonDown))
            }
            Setting::ScrollWheelEmulationButtonLock(on) => self.call(DeviceCall::ScrollButtonLock(*on)),
        }
    }

    fn reset(&mut self, kind: SettingKind) -> Result<(), SettingRefusal> {
        match kind {
            SettingKind::ScrollSpeed => {
                self.scroll_speed = 1.0;
                Ok(())
            }
            // These two were refused when they were applied, so there is nothing of them to put back.
            SettingKind::DisableWhileTypingTimeout => Ok(()),
            other => self.call(DeviceCall::Restore(other)),
        }
    }
}

impl Execute for libinput::Device {
    fn call(&mut self, call: DeviceCall) -> Result<(), SettingRefusal> {
        let result = match call {
            DeviceCall::AccelProfile(Profile::Flat) => self.config_accel_set_profile(libinput::AccelProfile::Flat),
            DeviceCall::AccelProfile(Profile::Adaptive) => {
                self.config_accel_set_profile(libinput::AccelProfile::Adaptive)
            }
            DeviceCall::AccelSpeed(speed) => self.config_accel_set_speed(speed),
            DeviceCall::NaturalScroll(on) => self.config_scroll_set_natural_scroll_enabled(on),
            DeviceCall::Tap(on) => self.config_tap_set_enabled(on),
            DeviceCall::TapDrag(on) => self.config_tap_set_drag_enabled(on),
            DeviceCall::TapDragLock(on) => self.config_tap_set_drag_lock_enabled(drag_lock(on)),
            DeviceCall::TapButtonMap(map) => self.config_tap_set_button_map(match map {
                ButtonMap::LeftMiddleRight => libinput::TapButtonMap::LeftMiddleRight,
                ButtonMap::LeftRightMiddle | ButtonMap::Default => libinput::TapButtonMap::LeftRightMiddle,
            }),
            DeviceCall::ClickMethod(ClickKind::Areas) => {
                self.config_click_set_method(libinput::ClickMethod::ButtonAreas)
            }
            DeviceCall::ClickMethod(ClickKind::Fingers) => {
                self.config_click_set_method(libinput::ClickMethod::Clickfinger)
            }
            DeviceCall::MiddleEmulation(on) => self.config_middle_emulation_set_enabled(on),
            DeviceCall::LeftHanded(on) => self.config_left_handed_set(on),
            DeviceCall::DisableWhileTyping(on) => self.config_dwt_set_enabled(on),
            DeviceCall::ScrollMethod(kind) => self.config_scroll_set_method(match kind {
                ScrollKind::Off => libinput::ScrollMethod::NoScroll,
                ScrollKind::OnButtonDown => libinput::ScrollMethod::OnButtonDown,
                ScrollKind::TwoFinger => libinput::ScrollMethod::TwoFinger,
                ScrollKind::Edge => libinput::ScrollMethod::Edge,
            }),
            DeviceCall::ScrollButton(code) => self.config_scroll_set_button(code),
            DeviceCall::ScrollButtonLock(on) => self.config_scroll_set_button_lock(if on {
                libinput::ScrollButtonLockState::Enabled
            } else {
                libinput::ScrollButtonLockState::Disabled
            }),
            DeviceCall::Restore(kind) => return restore(self, kind),
        };
        result.map_err(refusal_of)
    }
}

/// The drag lock state for a boolean: the timeout mode, which is what the older boolean call meant.
fn drag_lock(on: bool) -> libinput::DragLockState {
    if on {
        libinput::DragLockState::EnabledTimeout
    } else {
        libinput::DragLockState::Disabled
    }
}

/// What libinput's refusal means for a setting.
fn refusal_of(error: libinput::DeviceConfigError) -> SettingRefusal {
    match error {
        libinput::DeviceConfigError::Unsupported => SettingRefusal::Unsupported,
        libinput::DeviceConfigError::Invalid => SettingRefusal::Invalid,
    }
}

/// Puts one setting back to the value libinput says the device starts with.
fn restore(device: &mut libinput::Device, kind: SettingKind) -> Result<(), SettingRefusal> {
    let result = match kind {
        SettingKind::Acceleration => match device.config_accel_default_profile() {
            Some(profile) => device.config_accel_set_profile(profile),
            None => return Err(SettingRefusal::Unsupported),
        },
        SettingKind::Speed => {
            let speed = device.config_accel_default_speed();
            device.config_accel_set_speed(speed)
        }
        SettingKind::NaturalScroll => {
            let on = device.config_scroll_default_natural_scroll_enabled();
            device.config_scroll_set_natural_scroll_enabled(on)
        }
        SettingKind::TapToClick => {
            let on = device.config_tap_default_enabled();
            device.config_tap_set_enabled(on)
        }
        SettingKind::TapAndDrag => {
            let on = device.config_tap_default_drag_enabled();
            device.config_tap_set_drag_enabled(on)
        }
        SettingKind::TapAndDragLock => {
            let state = device.config_tap_default_drag_lock_enabled();
            device.config_tap_set_drag_lock_enabled(state)
        }
        SettingKind::TapButtonMap => match device.config_tap_default_button_map() {
            Some(map) => device.config_tap_set_button_map(map),
            None => return Err(SettingRefusal::Unsupported),
        },
        SettingKind::ClickMethod => match device.config_click_default_method() {
            Some(method) => device.config_click_set_method(method),
            // The device starts with no software buttons, which the binding cannot say.
            None => {
                return Err(SettingRefusal::Unavailable(
                    "the libinput binding has no way to turn software buttons off",
                ));
            }
        },
        SettingKind::MiddleClickEmulation => {
            let on = device.config_middle_emulation_default_enabled();
            device.config_middle_emulation_set_enabled(on)
        }
        SettingKind::LeftHanded => {
            let on = device.config_left_handed_default();
            device.config_left_handed_set(on)
        }
        SettingKind::DisableWhileTyping => {
            let on = device.config_dwt_default_enabled();
            device.config_dwt_set_enabled(on)
        }
        SettingKind::ScrollMethod => match device.config_scroll_default_method() {
            Some(method) => device.config_scroll_set_method(method),
            None => return Err(SettingRefusal::Unsupported),
        },
        // A scroll button is two settings of libinput's: the button, and the method that uses it.
        SettingKind::ScrollWheelEmulationButton => {
            let button = device.config_scroll_default_button();
            device
                .config_scroll_set_button(button)
                .and_then(|()| match device.config_scroll_default_method() {
                    Some(method) => device.config_scroll_set_method(method),
                    None => Err(libinput::DeviceConfigError::Unsupported),
                })
        }
        SettingKind::ScrollWheelEmulationButtonLock => {
            let state = device.config_scroll_default_button_lock();
            device.config_scroll_set_button_lock(state)
        }
        // Not libinput's: the scroll speed is the seat's, and the timeout was never applied.
        SettingKind::ScrollSpeed | SettingKind::DisableWhileTypingTimeout => return Ok(()),
    };
    result.map_err(refusal_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_backend::input_settings::{DeviceBook, DeviceSettings};
    use gnoblin_backend::{DeviceCaps, DeviceInfo, DeviceKind};
    use gnoblin_config::{AccelProfile, Handedness, Inherit, Input, Mouse, Touchpad, Trackball};
    use gnoblin_types::DeviceId;

    /// A device that records the calls it was given and refuses the ones it is told to.
    #[derive(Default)]
    struct Recording {
        calls: Vec<DeviceCall>,
        refuse: Vec<&'static str>,
    }

    fn call_name(call: &DeviceCall) -> &'static str {
        match call {
            DeviceCall::AccelProfile(_) => "accel-profile",
            DeviceCall::AccelSpeed(_) => "accel-speed",
            DeviceCall::NaturalScroll(_) => "natural-scroll",
            DeviceCall::Tap(_) => "tap",
            DeviceCall::TapDrag(_) => "tap-drag",
            DeviceCall::TapDragLock(_) => "tap-drag-lock",
            DeviceCall::TapButtonMap(_) => "tap-button-map",
            DeviceCall::ClickMethod(_) => "click-method",
            DeviceCall::MiddleEmulation(_) => "middle-emulation",
            DeviceCall::LeftHanded(_) => "left-handed",
            DeviceCall::DisableWhileTyping(_) => "dwt",
            DeviceCall::ScrollMethod(_) => "scroll-method",
            DeviceCall::ScrollButton(_) => "scroll-button",
            DeviceCall::ScrollButtonLock(_) => "scroll-button-lock",
            DeviceCall::Restore(_) => "restore",
        }
    }

    impl Execute for Recording {
        fn call(&mut self, call: DeviceCall) -> Result<(), SettingRefusal> {
            if self.refuse.contains(&call_name(&call)) {
                return Err(SettingRefusal::Unsupported);
            }
            self.calls.push(call);
            Ok(())
        }
    }

    fn input_with_touchpad(touchpad: Touchpad) -> Input {
        Input {
            touchpad: Some(touchpad),
            ..Input::default()
        }
    }

    fn touchpad_device(id: u64) -> DeviceInfo {
        DeviceInfo::new(DeviceId::new(id), "touchpad", DeviceKind::Touchpad)
    }

    fn calls_of(book: &DeviceBook<Configurator<Recording>>, id: u64) -> Vec<DeviceCall> {
        book.device(DeviceId::new(id)).expect("held").device().calls.clone()
    }

    #[test]
    fn a_document_becomes_the_calls_libinput_takes_in_the_documents_order() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(input_with_touchpad(Touchpad {
            accel_profile: Some(Inherit::Value(AccelProfile::Flat)),
            speed: Some(Inherit::Value(0.25)),
            natural_scroll: Some(Inherit::Value(true)),
            tap_to_click: Some(Inherit::Value(true)),
            tap_and_drag: Some(Inherit::Value(false)),
            tap_and_drag_lock: Some(Inherit::Value(true)),
            tap_button_map: Some(Inherit::Value(gnoblin_config::TapButtonMap::LeftMiddleRight)),
            click_method: Some(Inherit::Value(gnoblin_config::ClickMethod::Fingers)),
            middle_click_emulation: Some(Inherit::Value(true)),
            left_handed: Some(Inherit::Value(Handedness::Left)),
            disable_while_typing: Some(Inherit::Value(false)),
            two_finger_scrolling_enabled: Some(Inherit::Value(true)),
            ..Touchpad::default()
        }));
        book.add(&touchpad_device(1), Configurator::new(Recording::default()));
        assert_eq!(
            calls_of(&book, 1),
            vec![
                DeviceCall::AccelProfile(Profile::Flat),
                DeviceCall::AccelSpeed(0.25),
                DeviceCall::NaturalScroll(true),
                DeviceCall::Tap(true),
                DeviceCall::TapDrag(false),
                DeviceCall::TapDragLock(true),
                DeviceCall::TapButtonMap(ButtonMap::LeftMiddleRight),
                DeviceCall::ClickMethod(ClickKind::Fingers),
                DeviceCall::MiddleEmulation(true),
                DeviceCall::LeftHanded(true),
                DeviceCall::DisableWhileTyping(false),
                DeviceCall::ScrollMethod(ScrollKind::TwoFinger),
            ]
        );
    }

    #[test]
    fn a_reload_reaches_every_device_with_its_own_calls() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.add(&touchpad_device(1), Configurator::new(Recording::default()));
        book.add(&touchpad_device(2), Configurator::new(Recording::default()));
        let mouse = DeviceInfo::new(DeviceId::new(3), "mouse", DeviceKind::Mouse);
        book.add(&mouse, Configurator::new(Recording::default()));
        book.set_input(Input {
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(true)),
                ..Touchpad::default()
            }),
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        assert_eq!(calls_of(&book, 1), vec![DeviceCall::Tap(true)]);
        assert_eq!(calls_of(&book, 2), vec![DeviceCall::Tap(true)]);
        assert_eq!(calls_of(&book, 3), vec![DeviceCall::AccelSpeed(0.5)]);
        // The next reload changes the touchpads' tap and leaves the mouse alone.
        book.set_input(Input {
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(false)),
                ..Touchpad::default()
            }),
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        assert_eq!(calls_of(&book, 1), vec![DeviceCall::Tap(true), DeviceCall::Tap(false)]);
        assert_eq!(calls_of(&book, 2), vec![DeviceCall::Tap(true), DeviceCall::Tap(false)]);
        assert_eq!(
            calls_of(&book, 3),
            vec![DeviceCall::AccelSpeed(0.5)],
            "the mouse's calls did not change"
        );
    }

    #[test]
    fn a_removed_setting_is_restored_to_the_devices_default() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(input_with_touchpad(Touchpad {
            tap_to_click: Some(Inherit::Value(true)),
            speed: Some(Inherit::Value(0.5)),
            ..Touchpad::default()
        }));
        book.add(&touchpad_device(1), Configurator::new(Recording::default()));
        // The tap line leaves the document; the speed stays.
        book.set_input(input_with_touchpad(Touchpad {
            speed: Some(Inherit::Value(0.5)),
            ..Touchpad::default()
        }));
        assert_eq!(
            calls_of(&book, 1),
            vec![
                DeviceCall::AccelSpeed(0.5),
                DeviceCall::Tap(true),
                DeviceCall::Restore(SettingKind::TapToClick),
                DeviceCall::AccelSpeed(0.5)
            ]
        );
        // The whole section leaves: what is left is put back.
        book.set_input(Input::default());
        let calls = calls_of(&book, 1);
        assert_eq!(calls[4..], [DeviceCall::Restore(SettingKind::Speed)]);
    }

    #[test]
    fn a_device_plugged_in_later_is_configured_when_it_arrives() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(input_with_touchpad(Touchpad {
            natural_scroll: Some(Inherit::Value(true)),
            ..Touchpad::default()
        }));
        assert!(book.is_empty());
        book.add(&touchpad_device(7), Configurator::new(Recording::default()));
        assert_eq!(calls_of(&book, 7), vec![DeviceCall::NaturalScroll(true)]);
        let _ = book.remove(DeviceId::new(7));
        assert!(book.device(DeviceId::new(7)).is_none(), "a removed device is forgotten");
    }

    #[test]
    fn an_unsupported_option_is_refused_once_and_the_rest_still_apply() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(input_with_touchpad(Touchpad {
            middle_click_emulation: Some(Inherit::Value(true)),
            tap_to_click: Some(Inherit::Value(true)),
            ..Touchpad::default()
        }));
        let news = book.add(
            &touchpad_device(1),
            Configurator::new(Recording {
                refuse: vec!["middle-emulation"],
                ..Recording::default()
            }),
        );
        assert_eq!(news.len(), 1, "the device said no to one setting");
        assert_eq!(news[0].setting, SettingKind::MiddleClickEmulation);
        assert_eq!(news[0].refusal, SettingRefusal::Unsupported);
        assert_eq!(calls_of(&book, 1), vec![DeviceCall::Tap(true)], "the tap still applied");
        // The same refusal on a later reload is not said again.
        let news = book.set_input(input_with_touchpad(Touchpad {
            middle_click_emulation: Some(Inherit::Value(true)),
            tap_to_click: Some(Inherit::Value(false)),
            ..Touchpad::default()
        }));
        assert!(news.is_empty(), "{news:?}");
    }

    #[test]
    fn a_custom_curve_is_refused_with_its_reason() {
        let mut device = Configurator::new(Recording::default());
        let refusal = device
            .apply(&Setting::Acceleration(Accel::Custom {
                step: 1.0,
                points: vec![0.0, 1.0],
            }))
            .expect_err("a custom curve needs a newer libinput");
        assert!(
            matches!(refusal, SettingRefusal::Unavailable(reason) if reason.contains("1.23")),
            "{refusal}"
        );
        assert!(device.device().calls.is_empty(), "the device keeps the profile it had");
    }

    #[test]
    fn options_this_build_cannot_ask_for_are_refused_and_say_why() {
        let mut device = Configurator::new(Recording::default());
        for setting in [
            Setting::ClickMethod(Click::None),
            Setting::DisableWhileTypingTimeout(500),
            Setting::ScrollWheelEmulationButton(5),
            Setting::ScrollWheelEmulationButton(10),
        ] {
            assert!(
                matches!(device.apply(&setting), Err(SettingRefusal::Unavailable(_))),
                "{setting:?}"
            );
        }
        assert!(device.device().calls.is_empty());
    }

    #[test]
    fn a_scroll_button_is_sent_as_its_evdev_code() {
        for (number, code) in [(1, 0x110), (2, 0x112), (3, 0x111), (8, 0x113), (9, 0x114)] {
            let mut device = Configurator::new(Recording::default());
            device
                .apply(&Setting::ScrollWheelEmulationButton(number))
                .expect("a button libinput has");
            assert_eq!(
                device.device().calls[0],
                DeviceCall::ScrollButton(code),
                "button {number}"
            );
        }
    }

    #[test]
    fn a_scroll_button_is_set_before_the_method_that_uses_it() {
        let mut device = Configurator::new(Recording::default());
        device.apply(&Setting::ScrollWheelEmulationButton(2)).expect("takes it");
        assert_eq!(
            device.device().calls,
            vec![
                DeviceCall::ScrollButton(0x112),
                DeviceCall::ScrollMethod(ScrollKind::OnButtonDown)
            ]
        );
        let mut device = Configurator::new(Recording::default());
        device.apply(&Setting::ScrollWheelEmulationButton(0)).expect("takes it");
        assert_eq!(
            device.device().calls,
            vec![DeviceCall::ScrollMethod(ScrollKind::Off)],
            "no button is no button scrolling"
        );
    }

    #[test]
    fn a_trackball_is_configured_from_its_own_section() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(Input {
            trackball: Some(Trackball {
                scroll_wheel_emulation_button: Some(Inherit::Value(2)),
                scroll_wheel_emulation_button_lock: Some(Inherit::Value(true)),
                ..Trackball::default()
            }),
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        let ball = DeviceInfo::new(DeviceId::new(1), "trackball", DeviceKind::Trackball);
        book.add(&ball, Configurator::new(Recording::default()));
        assert_eq!(
            calls_of(&book, 1),
            vec![
                DeviceCall::ScrollButton(0x112),
                DeviceCall::ScrollMethod(ScrollKind::OnButtonDown),
                DeviceCall::ScrollButtonLock(true)
            ],
            "the mouse section's speed does not reach a trackball"
        );
    }

    #[test]
    fn a_tablet_is_not_given_the_mouse_section() {
        let mut book: DeviceBook<Configurator<Recording>> = DeviceBook::new();
        book.set_input(Input {
            mouse: Some(Mouse {
                left_handed: Some(Inherit::Value(true)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        let tablet = DeviceInfo::new(DeviceId::new(1), "tablet", DeviceKind::Mouse).with_caps(DeviceCaps::TABLET_TOOL);
        book.add(&tablet, Configurator::new(Recording::default()));
        assert!(
            calls_of(&book, 1).is_empty(),
            "a mouse's left-handed would turn the tablet around"
        );
    }

    #[test]
    fn the_scroll_speed_is_the_seats_and_no_call() {
        let mut device = Configurator::new(Recording::default());
        assert!((device.scroll_speed() - 1.0).abs() < f64::EPSILON);
        device.apply(&Setting::ScrollSpeed(1.5)).expect("the seat takes it");
        assert!((device.scroll_speed() - 1.5).abs() < f64::EPSILON);
        assert!(device.device().calls.is_empty(), "libinput was not asked");
        device.reset(SettingKind::ScrollSpeed).expect("the seat takes it back");
        assert!((device.scroll_speed() - 1.0).abs() < f64::EPSILON);
        assert!(device.device().calls.is_empty());
    }

    #[test]
    fn every_setting_the_document_can_produce_has_a_call_or_a_reason() {
        let mut settings = DeviceSettings::from_config(
            Some(&Input {
                touchpad: Some(Touchpad {
                    accel_profile: Some(Inherit::Value(AccelProfile::Default)),
                    ..Touchpad::default()
                }),
                ..Input::default()
            }),
            DeviceKind::Touchpad,
        );
        let mut device = Configurator::new(Recording::default());
        settings.apply(None, &mut device);
        assert_eq!(
            device.device().calls,
            vec![DeviceCall::Restore(SettingKind::Acceleration)],
            "the default profile asks libinput for its own default and names no profile"
        );
        assert!(settings.refused().is_empty());
    }
}
