//! The `input` section of the configuration, as the knobs an input device has.
//!
//! The configuration is written for a person (`touchpad.tap-to-click`, `mouse.speed`), and a device
//! speaks in settings, per device, with one call per setting. This module is the join between the two,
//! and it is shared by every backend: [`DeviceSettings::for_device`] picks the section that belongs to a
//! device, [`DeviceBook`] keeps the accepted section and the devices it was applied to, and the device
//! side is [`Applier`], implemented by the hardware backend over libinput and by the headless backend
//! as a recorder.
//!
//! It is pure for the same reason the libinput mapper is: the *decision* (which settings a device of
//! this kind gets, what an `inherit` means, what a custom acceleration curve without a curve does, what
//! happens to a device when its setting leaves the document) is the part that can be wrong in a way
//! nobody notices, and it is tested here without a device.
//!
//! # `inherit` is not a value
//!
//! A setting written as `inherit` clears an override so the system preference shows through again
//! (`gnoblin_config::Inherit`). It is therefore *not* applied: sending the driver's default is not the
//! same thing as leaving the device alone, because a udev rule or a hardware quirk may have set it to
//! something this session must not undo.
//!
//! # What leaves the document
//!
//! A setting that was applied and is no longer in the document (the key was removed, or became
//! `inherit`, or the whole section went) is put back to the device's own default
//! ([`Applier::reset`]). Without that, deleting a line from `shell.ts` would leave the old value in the
//! device until the next login.
//!
//! # Failure modes first
//!
//! 1. **A touchpad setting reaches a mouse.** The kind selects the section, and only the keyboard's own
//!    repeat settings are excluded by name: `a_mouse_never_gets_a_touchpad_setting`.
//! 2. **An inherited setting is applied as its default.** `inherit` produces no setting at all:
//!    `an_inherited_setting_is_not_applied`.
//! 3. **A custom acceleration profile is sent without a curve**, which a device refuses and which
//!    leaves it with no acceleration at all. It falls back to the adaptive profile:
//!    `a_custom_profile_without_a_curve_falls_back_to_adaptive`.
//! 4. **A speed outside the range a device accepts is sent.** It is clamped to what the device states,
//!    because the range is the device's and not the configuration's: `a_speed_outside_the_range_is_clamped`.
//! 5. **A device that refuses one setting stops the rest.** A refusal is recorded and skipped:
//!    `a_refused_setting_does_not_stop_the_rest`.
//! 6. **A keyboard, a tablet or a lid switch is given pointer settings.** Only a device with a pointer
//!    capability reads a pointer section: `a_device_without_a_pointer_gets_no_pointer_setting`.
//! 7. **Two scroll switches fight.** A touchpad has one scroll method, and the document has two booleans
//!    for it, so they are resolved to one: `the_two_touchpad_scroll_switches_become_one_method`.
//! 8. **A reload leaves a removed setting in the device.** What was applied and is gone is reset:
//!    `a_setting_that_leaves_the_document_is_reset`.
//! 9. **A removed device keeps a book entry.** Its handle is dropped with it: `a_removed_device_is_forgotten`.
//! 10. **One unsupported option is reported on every reload.** A device is told about a refused setting
//!     once: `a_refusal_is_reported_once_per_device`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gnoblin_config::{AccelProfile, ClickMethod, Handedness, Inherit, Input, TapButtonMap, Touchpad};
use gnoblin_types::DeviceId;

use crate::log;
use crate::{DeviceCaps, DeviceInfo, DeviceKind};

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
    /// Scroll with two fingers on a touchpad.
    TwoFinger,
    /// Scroll along a touchpad's edge.
    Edge,
}

/// One setting a device is given, in the terms a device's configuration API states it.
#[derive(Clone, PartialEq, Debug)]
pub enum Setting {
    /// The acceleration profile and its curve.
    Acceleration(Accel),
    /// The pointer speed, -1 to 1.
    Speed(f64),
    /// How much a touchpad's scrolling is multiplied by, 0 to 2. It is the compositor's, not the device's:
    /// the device reports its own scroll distance and the seat scales it.
    ScrollSpeed(f64),
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
    /// The button that stands in for scroll events, as the document numbers it: 1 left, 2 middle, 3 right,
    /// 8 back, 9 forward (X's numbering, the one the toolkits use), and 0 for none.
    ScrollWheelEmulationButton(u32),
    /// Hold that button to keep scrolling.
    ScrollWheelEmulationButtonLock(bool),
}

/// Which setting a [`Setting`] is, without its value: what a reload compares and a log names.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SettingKind {
    /// [`Setting::Acceleration`].
    Acceleration,
    /// [`Setting::Speed`].
    Speed,
    /// [`Setting::ScrollSpeed`].
    ScrollSpeed,
    /// [`Setting::NaturalScroll`].
    NaturalScroll,
    /// [`Setting::TapToClick`].
    TapToClick,
    /// [`Setting::TapAndDrag`].
    TapAndDrag,
    /// [`Setting::TapAndDragLock`].
    TapAndDragLock,
    /// [`Setting::TapButtonMap`].
    TapButtonMap,
    /// [`Setting::ClickMethod`].
    ClickMethod,
    /// [`Setting::MiddleClickEmulation`].
    MiddleClickEmulation,
    /// [`Setting::LeftHanded`].
    LeftHanded,
    /// [`Setting::DisableWhileTyping`].
    DisableWhileTyping,
    /// [`Setting::DisableWhileTypingTimeout`].
    DisableWhileTypingTimeout,
    /// [`Setting::ScrollMethod`].
    ScrollMethod,
    /// [`Setting::ScrollWheelEmulationButton`].
    ScrollWheelEmulationButton,
    /// [`Setting::ScrollWheelEmulationButtonLock`].
    ScrollWheelEmulationButtonLock,
}

impl SettingKind {
    /// The setting's name as the document spells it (the last segment of its path).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Acceleration => "accel-profile",
            Self::Speed => "speed",
            Self::ScrollSpeed => "scroll-speed",
            Self::NaturalScroll => "natural-scroll",
            Self::TapToClick => "tap-to-click",
            Self::TapAndDrag => "tap-and-drag",
            Self::TapAndDragLock => "tap-and-drag-lock",
            Self::TapButtonMap => "tap-button-map",
            Self::ClickMethod => "click-method",
            Self::MiddleClickEmulation => "middle-click-emulation",
            Self::LeftHanded => "left-handed",
            Self::DisableWhileTyping => "disable-while-typing",
            Self::DisableWhileTypingTimeout => "disable-while-typing-timeout",
            Self::ScrollMethod => "scroll-method",
            Self::ScrollWheelEmulationButton => "scroll-wheel-emulation-button",
            Self::ScrollWheelEmulationButtonLock => "scroll-wheel-emulation-button-lock",
        }
    }
}

impl Setting {
    /// Which setting this is.
    #[must_use]
    pub const fn kind(&self) -> SettingKind {
        match self {
            Self::Acceleration(_) => SettingKind::Acceleration,
            Self::Speed(_) => SettingKind::Speed,
            Self::ScrollSpeed(_) => SettingKind::ScrollSpeed,
            Self::NaturalScroll(_) => SettingKind::NaturalScroll,
            Self::TapToClick(_) => SettingKind::TapToClick,
            Self::TapAndDrag(_) => SettingKind::TapAndDrag,
            Self::TapAndDragLock(_) => SettingKind::TapAndDragLock,
            Self::TapButtonMap(_) => SettingKind::TapButtonMap,
            Self::ClickMethod(_) => SettingKind::ClickMethod,
            Self::MiddleClickEmulation(_) => SettingKind::MiddleClickEmulation,
            Self::LeftHanded(_) => SettingKind::LeftHanded,
            Self::DisableWhileTyping(_) => SettingKind::DisableWhileTyping,
            Self::DisableWhileTypingTimeout(_) => SettingKind::DisableWhileTypingTimeout,
            Self::ScrollMethod(_) => SettingKind::ScrollMethod,
            Self::ScrollWheelEmulationButton(_) => SettingKind::ScrollWheelEmulationButton,
            Self::ScrollWheelEmulationButtonLock(_) => SettingKind::ScrollWheelEmulationButtonLock,
        }
    }
}

/// The evdev code of a button as the document numbers it, or `None` for a number no button has.
///
/// The document uses X's numbering, which is what the toolkits and the desktop settings use: 1 is the
/// left button, 2 the middle, 3 the right, 8 back and 9 forward. 4 to 7 are X's scroll wheel and have no
/// button, and 0 is "none" and is handled before this is asked.
#[must_use]
pub const fn evdev_button(number: u32) -> Option<u32> {
    match number {
        1 => Some(0x110),
        2 => Some(0x112),
        3 => Some(0x111),
        8 => Some(0x113),
        9 => Some(0x114),
        _ => None,
    }
}

/// Why a device did not take a setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingRefusal {
    /// The device has no such setting: a touchpad without a middle button refuses middle-click emulation.
    Unsupported,
    /// The device has the setting and rejected this value.
    Invalid,
    /// This build cannot ask the device for it, and says why. It is not the device's refusal: the
    /// same device on a build with a newer libinput would take it.
    Unavailable(&'static str),
}

impl fmt::Display for SettingRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("the device has no such setting"),
            Self::Invalid => formatter.write_str("the device rejected the value"),
            Self::Unavailable(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for SettingRefusal {}

/// One setting a device refused, and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RefusedSetting {
    /// The setting.
    pub setting: SettingKind,
    /// Why.
    pub refusal: SettingRefusal,
}

/// What a device is given, and what it refused.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DeviceSettings {
    /// The kind the settings were built for; `None` only for the `Default` value, which matches no kind.
    kind: Option<DeviceKind>,
    /// The settings to apply, in the order the configuration lists them.
    settings: Vec<Setting>,
    /// What the device refused on the last apply.
    refused: Vec<RefusedSetting>,
}

impl DeviceSettings {
    /// The settings the document gives a device of `info`'s kind.
    ///
    /// A device with no pointer capability gets none of the pointer sections, whatever its kind says: a
    /// drawing tablet's tool and a laptop's lid switch are classified as a mouse by the fallback rule of
    /// the libinput backend, and a `mouse.left-handed` must not rotate a tablet.
    #[must_use]
    pub fn for_device(input: Option<&Input>, info: &DeviceInfo) -> Self {
        let reads_a_pointer_section = matches!(
            info.kind,
            DeviceKind::Touchpad | DeviceKind::Mouse | DeviceKind::Trackball | DeviceKind::PointingStick
        );
        if reads_a_pointer_section && !info.caps.contains(DeviceCaps::POINTER) {
            return Self {
                kind: Some(info.kind),
                settings: Vec::new(),
                refused: Vec::new(),
            };
        }
        Self::from_config(input, info.kind)
    }

    /// The settings the configuration gives a device of this kind.
    ///
    /// A configuration with no `input` section, or a section for another kind, produces an
    /// empty list, which is the desired state: the device's own defaults and whatever the system
    /// set stay in force.
    #[must_use]
    pub fn from_config(input: Option<&Input>, kind: DeviceKind) -> Self {
        let mut settings = Vec::new();
        if let Some(input) = input {
            match kind {
                DeviceKind::Touchpad => {
                    if let Some(touchpad) = input.touchpad.as_ref() {
                        touchpad_settings(&mut settings, input, touchpad);
                    }
                }
                DeviceKind::Mouse => {
                    if let Some(mouse) = input.mouse.as_ref() {
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
                    if let Some(trackball) = input.trackball.as_ref() {
                        push(&mut settings, trackball.accel_profile.as_ref(), |profile| {
                            Some(Setting::Acceleration(device_accel(*profile)))
                        });
                        push(&mut settings, trackball.middle_click_emulation.as_ref(), |value| {
                            Some(Setting::MiddleClickEmulation(*value))
                        });
                        push(
                            &mut settings,
                            trackball.scroll_wheel_emulation_button.as_ref(),
                            |button| Some(Setting::ScrollWheelEmulationButton(u32::try_from(*button).unwrap_or(0))),
                        );
                        push(
                            &mut settings,
                            trackball.scroll_wheel_emulation_button_lock.as_ref(),
                            |value| Some(Setting::ScrollWheelEmulationButtonLock(*value)),
                        );
                    }
                }
                DeviceKind::PointingStick => {
                    if let Some(stick) = input.pointing_stick.as_ref() {
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
                // keymap's: neither has a device setting in this configuration.
                DeviceKind::Touch | DeviceKind::Keyboard => {}
            }
        }
        Self {
            kind: Some(kind),
            settings,
            refused: Vec::new(),
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

    /// What the device refused on the last [`DeviceSettings::apply`].
    #[must_use]
    pub fn refused(&self) -> &[RefusedSetting] {
        &self.refused
    }

    /// Whether the configuration had anything to say about this device.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }

    /// Hands every setting to the device, after putting back what `previous` applied and these no longer do.
    ///
    /// A setting the device refuses is recorded and the rest are still offered: a touchpad without a
    /// middle button refuses middle-click emulation and everything else must still apply, or one
    /// unsupported knob would undo the configuration. A reset that is refused is not recorded: the
    /// device refused the same setting when it was applied, and said so then.
    pub fn apply(&mut self, previous: Option<&Self>, applier: &mut dyn Applier) {
        self.refused.clear();
        if let Some(previous) = previous {
            for old in &previous.settings {
                let kind = old.kind();
                if self.settings.iter().all(|setting| setting.kind() != kind) {
                    let _ = applier.reset(kind);
                }
            }
        }
        for setting in &self.settings {
            if let Err(refusal) = applier.apply(setting) {
                self.refused.push(RefusedSetting {
                    setting: setting.kind(),
                    refusal,
                });
            }
        }
    }
}

/// One device that settings can be applied to.
///
/// It is a trait so the mapping above is testable against a recorder, and so the hardware glue is one
/// translation of [`Setting`] into the driver's calls and nothing else.
pub trait Applier {
    /// Offers one setting to the device.
    ///
    /// # Errors
    ///
    /// [`SettingRefusal`] when the device does not take it. A refused setting is not a failure of the
    /// session: a device without a feature simply does not have it.
    fn apply(&mut self, setting: &Setting) -> Result<(), SettingRefusal>;

    /// Puts one setting back to the device's own default.
    ///
    /// # Errors
    ///
    /// [`SettingRefusal`] when the device does not take it.
    fn reset(&mut self, kind: SettingKind) -> Result<(), SettingRefusal>;
}

/// A device that holds what it was given, for a backend with no hardware to configure.
///
/// The headless backend's virtual devices are these, so a test reads back what the document gave each
/// of them through the same contract the hardware backend implements.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct VirtualDevice {
    held: BTreeMap<SettingKind, Setting>,
}

impl VirtualDevice {
    /// The settings the device holds now, in a fixed order.
    pub fn settings(&self) -> impl Iterator<Item = &Setting> {
        self.held.values()
    }

    /// The setting of this kind the device holds, if any.
    #[must_use]
    pub fn get(&self, kind: SettingKind) -> Option<&Setting> {
        self.held.get(&kind)
    }
}

impl Applier for VirtualDevice {
    fn apply(&mut self, setting: &Setting) -> Result<(), SettingRefusal> {
        self.held.insert(setting.kind(), setting.clone());
        Ok(())
    }

    fn reset(&mut self, kind: SettingKind) -> Result<(), SettingRefusal> {
        self.held.remove(&kind);
        Ok(())
    }
}

/// A setting a device refused for the first time, for the caller to log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceRefusal {
    /// The device.
    pub device: DeviceId,
    /// Its name.
    pub name: String,
    /// The setting.
    pub setting: SettingKind,
    /// Why.
    pub refusal: SettingRefusal,
}

impl DeviceRefusal {
    /// Says it in the log, with the device and the setting named, so a person whose touchpad ignores a line of
    /// `shell.ts` can find out why. A [`DeviceBook`] hands each refusal out once per device.
    pub fn report(&self) {
        log::warn!(area = "input", device = %self.name, id = %self.device, setting = self.setting.name(), reason = %self.refusal, "a device does not take a setting from the document");
    }
}

/// One device in a [`DeviceBook`].
struct BookEntry<H> {
    info: DeviceInfo,
    handle: H,
    /// What was applied last, so a reload knows what to put back.
    applied: DeviceSettings,
    /// The settings the device has refused and the caller has been told about.
    told: BTreeSet<SettingKind>,
}

/// The accepted `input` section and the devices it was applied to.
///
/// A backend keeps one. [`DeviceBook::add`] configures a device from the section the moment it arrives,
/// [`DeviceBook::set_input`] re-applies a changed section to every device the backend has, and
/// [`DeviceBook::remove`] forgets a device that went away, so a device plugged in again is configured as
/// a new one.
pub struct DeviceBook<H> {
    input: Option<Input>,
    devices: BTreeMap<DeviceId, BookEntry<H>>,
}

impl<H> Default for DeviceBook<H> {
    fn default() -> Self {
        Self {
            input: None,
            devices: BTreeMap::new(),
        }
    }
}

impl<H> fmt::Debug for DeviceBook<H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceBook")
            .field("input", &self.input.is_some())
            .field("devices", &self.devices.len())
            .finish()
    }
}

impl<H: Applier> DeviceBook<H> {
    /// An empty book: no section accepted yet and no device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The section the book applies, or `None` before the first one.
    #[must_use]
    pub fn input(&self) -> Option<&Input> {
        self.input.as_ref()
    }

    /// How many devices the book holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// Whether the book holds no device.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// The handle of a device.
    #[must_use]
    pub fn device(&self, id: DeviceId) -> Option<&H> {
        self.devices.get(&id).map(|entry| &entry.handle)
    }

    /// What was applied to a device last.
    #[must_use]
    pub fn settings_of(&self, id: DeviceId) -> Option<&DeviceSettings> {
        self.devices.get(&id).map(|entry| &entry.applied)
    }

    /// Takes a device that arrived and configures it from the accepted section.
    ///
    /// A device announced twice replaces the first entry, and is configured as new. Returns what the
    /// device refused that nobody has been told about.
    pub fn add(&mut self, info: &DeviceInfo, mut handle: H) -> Vec<DeviceRefusal> {
        let mut applied = DeviceSettings::for_device(self.input.as_ref(), info);
        applied.apply(None, &mut handle);
        let mut entry = BookEntry {
            info: info.clone(),
            handle,
            applied,
            told: BTreeSet::new(),
        };
        let news = news_of(&mut entry);
        self.devices.insert(info.id, entry);
        news
    }

    /// Forgets a device that went away, handing its handle back.
    pub fn remove(&mut self, id: DeviceId) -> Option<H> {
        self.devices.remove(&id).map(|entry| entry.handle)
    }

    /// Takes a new `input` section and applies it to every device the book holds.
    ///
    /// A device whose settings did not change is not touched. Returns what devices refused that nobody has
    /// been told about.
    pub fn set_input(&mut self, input: Input) -> Vec<DeviceRefusal> {
        self.input = Some(input);
        let mut news = Vec::new();
        for entry in self.devices.values_mut() {
            let mut next = DeviceSettings::for_device(self.input.as_ref(), &entry.info);
            if next.settings == entry.applied.settings {
                continue;
            }
            next.apply(Some(&entry.applied), &mut entry.handle);
            entry.applied = next;
            news.extend(news_of(entry));
        }
        news
    }
}

/// The refusals of an entry's last apply that its device has not been told about.
fn news_of<H>(entry: &mut BookEntry<H>) -> Vec<DeviceRefusal> {
    let mut news = Vec::new();
    for refused in entry.applied.refused() {
        if entry.told.insert(refused.setting) {
            news.push(DeviceRefusal {
                device: entry.info.id,
                name: entry.info.name.clone(),
                setting: refused.setting,
                refusal: refused.refusal,
            });
        }
    }
    news
}

/// Why a backend did not configure its input devices from the document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputSettingsRefusal {
    /// The session's devices are another compositor's: a nested session reads the host's seat, and
    /// the host's own settings decide how its pointers and touchpads behave.
    HostOwnsDevices,
    /// The backend has no input devices to configure.
    NoDevices,
}

impl fmt::Display for InputSettingsRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostOwnsDevices => formatter
                .write_str("the input devices belong to the host compositor, which applies its own settings to them"),
            Self::NoDevices => formatter.write_str("the backend has no input devices to configure"),
        }
    }
}

impl std::error::Error for InputSettingsRefusal {}

/// The touchpad's settings, in the order the configuration declares them.
fn touchpad_settings(settings: &mut Vec<Setting>, input: &Input, touchpad: &Touchpad) {
    push(settings, touchpad.accel_profile.as_ref(), |profile| {
        Some(Setting::Acceleration(accel(profile, touchpad.accel_curve.as_ref())))
    });
    push(settings, touchpad.speed.as_ref(), |speed| {
        Some(Setting::Speed(clamp_speed(*speed)))
    });
    push(settings, touchpad.scroll_speed.as_ref(), |speed| {
        Some(Setting::ScrollSpeed(clamp_scroll_speed(*speed)))
    });
    push(settings, touchpad.natural_scroll.as_ref(), |value| {
        Some(Setting::NaturalScroll(*value))
    });
    push(settings, touchpad.tap_to_click.as_ref(), |value| {
        Some(Setting::TapToClick(*value))
    });
    push(settings, touchpad.tap_and_drag.as_ref(), |value| {
        Some(Setting::TapAndDrag(*value))
    });
    push(settings, touchpad.tap_and_drag_lock.as_ref(), |value| {
        Some(Setting::TapAndDragLock(*value))
    });
    push(settings, touchpad.tap_button_map.as_ref(), |map| {
        Some(Setting::TapButtonMap(button_map(*map)))
    });
    push(settings, touchpad.click_method.as_ref(), |method| {
        Some(Setting::ClickMethod(click(*method)))
    });
    push(settings, touchpad.middle_click_emulation.as_ref(), |value| {
        Some(Setting::MiddleClickEmulation(*value))
    });
    push(settings, touchpad.left_handed.as_ref(), |hand| match hand {
        Handedness::Left => Some(Setting::LeftHanded(true)),
        Handedness::Right => Some(Setting::LeftHanded(false)),
        // The touchpad follows the mouse section, when that one says anything.
        Handedness::FollowMouse => input
            .mouse
            .as_ref()
            .and_then(|mouse| mouse.left_handed.as_ref())
            .and_then(Inherit::value)
            .map(|left| Setting::LeftHanded(*left)),
    });
    push(settings, touchpad.disable_while_typing.as_ref(), |value| {
        Some(Setting::DisableWhileTyping(*value))
    });
    push(settings, touchpad.disable_while_typing_timeout.as_ref(), |timeout| {
        Some(Setting::DisableWhileTypingTimeout(u64::try_from(*timeout).unwrap_or(0)))
    });
    let edge = touchpad
        .edge_scrolling_enabled
        .as_ref()
        .and_then(Inherit::value)
        .copied();
    let two_finger = touchpad
        .two_finger_scrolling_enabled
        .as_ref()
        .and_then(Inherit::value)
        .copied();
    if let Some(method) = touchpad_scroll(edge, two_finger) {
        settings.push(Setting::ScrollMethod(method));
    }
}

/// The one scroll method a touchpad has, from the document's two switches for it.
///
/// A touchpad scrolls one way at a time, and the document has `two-finger-scrolling-enabled` and
/// `edge-scrolling-enabled`. When both are on, two fingers win, because it is the method libinput
/// itself defaults to wherever a device can do it. A switch turned off with the other left alone leaves
/// the other method, since turning one way off while keeping a touchpad that cannot scroll is not
/// something anyone writes; both off is no scrolling.
fn touchpad_scroll(edge: Option<bool>, two_finger: Option<bool>) -> Option<Scroll> {
    match (edge, two_finger) {
        (_, Some(true)) => Some(Scroll::TwoFinger),
        (Some(true), _) => Some(Scroll::Edge),
        (Some(false), Some(false)) => Some(Scroll::None),
        (Some(false), None) => Some(Scroll::TwoFinger),
        (None, Some(false)) => Some(Scroll::Edge),
        (None, None) => None,
    }
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
/// libinput refuses `Custom` without a curve, which leaves the device unaccelerated, a worse
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
/// value that did not come from the document: a script, or a reload whose validator ran
/// against a different schema.
fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() { speed.clamp(-1.0, 1.0) } else { 0.0 }
}

/// A scroll multiplier the seat accepts.
///
/// The configuration states 0 to 2 and this only catches a value that did not come from the document.
/// A non-finite value is no scaling at all, which is 1: zero would silence the touchpad's scrolling.
fn clamp_scroll_speed(speed: f64) -> f64 {
    if speed.is_finite() { speed.clamp(0.0, 2.0) } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::generated::AccelCurve;
    use gnoblin_config::{DeviceAccelProfile, Mouse, PointingStick, Trackball};

    /// A device that takes everything but what it is told to refuse, and records what it was given.
    #[derive(Default)]
    struct Fake {
        seen: Vec<Setting>,
        resets: Vec<SettingKind>,
        refuse: Vec<SettingKind>,
    }

    impl Applier for Fake {
        fn apply(&mut self, setting: &Setting) -> Result<(), SettingRefusal> {
            if self.refuse.contains(&setting.kind()) {
                return Err(SettingRefusal::Unsupported);
            }
            self.seen.push(setting.clone());
            Ok(())
        }

        fn reset(&mut self, kind: SettingKind) -> Result<(), SettingRefusal> {
            self.resets.push(kind);
            Ok(())
        }
    }

    fn touchpad_input(touchpad: Touchpad) -> Input {
        Input {
            touchpad: Some(touchpad),
            ..Input::default()
        }
    }

    fn device(id: u64, kind: DeviceKind) -> DeviceInfo {
        DeviceInfo::new(DeviceId::new(id), "test device", kind)
    }

    fn names(settings: &DeviceSettings) -> Vec<&'static str> {
        settings
            .settings()
            .iter()
            .map(|setting| setting.kind().name())
            .collect()
    }

    #[test]
    fn a_mouse_never_gets_a_touchpad_setting() {
        let input = Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            touchpad: Some(Touchpad {
                tap_to_click: Some(Inherit::Value(true)),
                ..Touchpad::default()
            }),
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
        let input = touchpad_input(Touchpad {
            scroll_speed: Some(Inherit::Value(7.0)),
            ..Touchpad::default()
        });
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad).settings(),
            &[Setting::ScrollSpeed(2.0)],
            "the scroll speed has its own range, 0 to 2"
        );
        let input = touchpad_input(Touchpad {
            scroll_speed: Some(Inherit::Value(f64::NAN)),
            ..Touchpad::default()
        });
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad).settings(),
            &[Setting::ScrollSpeed(1.0)],
            "a scroll speed that is not a number leaves the scrolling as it was"
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
        let mut fake = Fake {
            refuse: vec![SettingKind::NaturalScroll],
            ..Fake::default()
        };
        settings.apply(None, &mut fake);
        assert_eq!(fake.seen, vec![Setting::TapToClick(true)]);
        assert_eq!(
            settings.refused(),
            &[RefusedSetting {
                setting: SettingKind::NaturalScroll,
                refusal: SettingRefusal::Unsupported
            }]
        );
    }

    #[test]
    fn a_keyboard_has_no_device_setting() {
        let input = Input {
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
    fn a_device_without_a_pointer_gets_no_pointer_setting() {
        let input = Input {
            mouse: Some(Mouse {
                left_handed: Some(Inherit::Value(true)),
                speed: Some(Inherit::Value(0.5)),
                ..Mouse::default()
            }),
            ..Input::default()
        };
        // A drawing tablet and a lid switch are a "mouse" to the fallback rule, and have no pointer.
        let tablet = device(1, DeviceKind::Mouse).with_caps(DeviceCaps::TABLET_TOOL);
        assert!(DeviceSettings::for_device(Some(&input), &tablet).is_empty());
        let lid = device(2, DeviceKind::Mouse).with_caps(DeviceCaps::SWITCH);
        assert!(DeviceSettings::for_device(Some(&input), &lid).is_empty());
        let mouse = device(3, DeviceKind::Mouse);
        assert_eq!(
            DeviceSettings::for_device(Some(&input), &mouse).settings(),
            &[Setting::Speed(0.5), Setting::LeftHanded(true)]
        );
    }

    #[test]
    fn every_touchpad_key_reaches_the_device() {
        // The configuration's touchpad section and this mapping have to agree: a key that the document
        // accepts and nothing applies is a setting that silently does nothing.
        let touchpad = Touchpad {
            speed: Some(Inherit::Value(0.25)),
            scroll_speed: Some(Inherit::Value(1.5)),
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
        assert_eq!(
            names(&settings),
            vec![
                "accel-profile",
                "speed",
                "scroll-speed",
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
                "scroll-method",
            ]
        );
    }

    #[test]
    fn every_mouse_trackball_and_pointing_stick_key_reaches_the_device() {
        let input = Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.1)),
                double_click_time: Some(Inherit::Value(400)),
                drag_threshold: Some(Inherit::Value(8)),
                left_handed: Some(Inherit::Value(true)),
                natural_scroll: Some(Inherit::Value(true)),
                middle_click_emulation: Some(Inherit::Value(true)),
                accel_profile: Some(Inherit::Value(AccelProfile::Flat)),
                accel_curve: None,
            }),
            trackball: Some(Trackball {
                accel_profile: Some(Inherit::Value(DeviceAccelProfile::Adaptive)),
                middle_click_emulation: Some(Inherit::Value(true)),
                scroll_wheel_emulation_button: Some(Inherit::Value(2)),
                scroll_wheel_emulation_button_lock: Some(Inherit::Value(true)),
            }),
            pointing_stick: Some(PointingStick {
                speed: Some(Inherit::Value(-0.5)),
                accel_profile: Some(Inherit::Value(DeviceAccelProfile::Flat)),
                scroll_method: Some(Inherit::Value(gnoblin_config::ScrollMethod::OnButtonDown)),
            }),
            ..Input::default()
        };
        // The double-click time and the drag threshold are the toolkits' (they read them from the settings
        // portal), not a device's, so no device is given them.
        assert_eq!(
            names(&DeviceSettings::from_config(Some(&input), DeviceKind::Mouse)),
            vec![
                "accel-profile",
                "speed",
                "natural-scroll",
                "middle-click-emulation",
                "left-handed"
            ]
        );
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::Trackball).settings(),
            &[
                Setting::Acceleration(Accel::Adaptive),
                Setting::MiddleClickEmulation(true),
                Setting::ScrollWheelEmulationButton(2),
                Setting::ScrollWheelEmulationButtonLock(true),
            ]
        );
        assert_eq!(
            DeviceSettings::from_config(Some(&input), DeviceKind::PointingStick).settings(),
            &[
                Setting::Acceleration(Accel::Flat),
                Setting::Speed(-0.5),
                Setting::ScrollMethod(Scroll::OnButtonDown),
            ]
        );
    }

    #[test]
    fn the_two_touchpad_scroll_switches_become_one_method() {
        use Scroll::{Edge, None as Off, TwoFinger};
        for (edge, two_finger, expected) in [
            (None, None, Option::None),
            (Some(true), None, Some(Edge)),
            (None, Some(true), Some(TwoFinger)),
            (Some(true), Some(true), Some(TwoFinger)),
            (Some(false), Some(false), Some(Off)),
            (Some(false), None, Some(TwoFinger)),
            (None, Some(false), Some(Edge)),
            (Some(true), Some(false), Some(Edge)),
            (Some(false), Some(true), Some(TwoFinger)),
        ] {
            let input = touchpad_input(Touchpad {
                edge_scrolling_enabled: edge.map(Inherit::Value),
                two_finger_scrolling_enabled: two_finger.map(Inherit::Value),
                ..Touchpad::default()
            });
            let settings = DeviceSettings::from_config(Some(&input), DeviceKind::Touchpad);
            assert_eq!(
                settings.settings().first(),
                expected.map(Setting::ScrollMethod).as_ref(),
                "edge {edge:?}, two fingers {two_finger:?}"
            );
        }
    }

    #[test]
    fn a_touchpad_that_follows_the_mouse_takes_its_handedness() {
        let follow = |mouse: Option<Inherit<bool>>| Input {
            mouse: Some(Mouse {
                left_handed: mouse,
                ..Mouse::default()
            }),
            touchpad: Some(Touchpad {
                left_handed: Some(Inherit::Value(Handedness::FollowMouse)),
                ..Touchpad::default()
            }),
            ..Input::default()
        };
        assert_eq!(
            DeviceSettings::from_config(Some(&follow(Some(Inherit::Value(true)))), DeviceKind::Touchpad).settings(),
            &[Setting::LeftHanded(true)]
        );
        assert!(
            DeviceSettings::from_config(Some(&follow(None)), DeviceKind::Touchpad).is_empty(),
            "a mouse section that says nothing leaves the touchpad alone"
        );
    }

    #[test]
    fn the_scroll_button_is_numbered_as_the_toolkits_number_it() {
        assert_eq!(evdev_button(1), Some(0x110));
        assert_eq!(evdev_button(2), Some(0x112));
        assert_eq!(evdev_button(3), Some(0x111));
        assert_eq!(evdev_button(8), Some(0x113));
        assert_eq!(evdev_button(9), Some(0x114));
        for number in [0, 4, 5, 6, 7, 10, 24] {
            assert_eq!(evdev_button(number), None, "button {number}");
        }
    }

    fn mouse_speed(speed: f64) -> Input {
        Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(speed)),
                ..Mouse::default()
            }),
            ..Input::default()
        }
    }

    #[test]
    fn a_device_arriving_after_the_section_is_configured_at_once() {
        let mut book: DeviceBook<Fake> = DeviceBook::new();
        let mouse = device(1, DeviceKind::Mouse);
        assert!(book.add(&mouse, Fake::default()).is_empty());
        assert!(
            book.device(mouse.id).expect("held").seen.is_empty(),
            "no section accepted yet, so nothing is applied"
        );
        book.set_input(mouse_speed(0.5));
        let second = device(2, DeviceKind::Mouse);
        book.add(&second, Fake::default());
        assert_eq!(
            book.device(second.id).expect("held").seen,
            vec![Setting::Speed(0.5)],
            "a device plugged in after the login is configured when it arrives"
        );
    }

    #[test]
    fn a_reload_reaches_every_device_and_only_the_ones_it_changes() {
        let mut book: DeviceBook<Fake> = DeviceBook::new();
        let mouse = device(1, DeviceKind::Mouse);
        let keyboard = device(2, DeviceKind::Keyboard);
        book.add(&mouse, Fake::default());
        book.add(&keyboard, Fake::default());
        book.set_input(mouse_speed(0.5));
        assert_eq!(book.device(mouse.id).expect("held").seen, vec![Setting::Speed(0.5)]);
        // The same section again changes nothing for the mouse, so it is not touched.
        book.set_input(mouse_speed(0.5));
        assert_eq!(book.device(mouse.id).expect("held").seen, vec![Setting::Speed(0.5)]);
        book.set_input(mouse_speed(-0.25));
        assert_eq!(
            book.device(mouse.id).expect("held").seen,
            vec![Setting::Speed(0.5), Setting::Speed(-0.25)]
        );
        let keyboard = book.device(keyboard.id).expect("held");
        assert!(
            keyboard.seen.is_empty() && keyboard.resets.is_empty(),
            "a keyboard is never touched"
        );
    }

    #[test]
    fn a_setting_that_leaves_the_document_is_reset() {
        let mut book: DeviceBook<Fake> = DeviceBook::new();
        let mouse = device(1, DeviceKind::Mouse);
        book.add(&mouse, Fake::default());
        book.set_input(mouse_speed(0.5));
        // The key is removed: the document has a mouse section with nothing in it.
        book.set_input(Input {
            mouse: Some(Mouse::default()),
            ..Input::default()
        });
        assert_eq!(book.device(mouse.id).expect("held").resets, vec![SettingKind::Speed]);
        // And so is a document that drops the whole section, or sets it to inherit.
        book.set_input(mouse_speed(0.5));
        book.set_input(Input::default());
        assert_eq!(
            book.device(mouse.id).expect("held").resets,
            vec![SettingKind::Speed, SettingKind::Speed]
        );
        book.set_input(Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Value(0.1)),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        book.set_input(Input {
            mouse: Some(Mouse {
                speed: Some(Inherit::Inherit),
                ..Mouse::default()
            }),
            ..Input::default()
        });
        assert_eq!(
            book.device(mouse.id).expect("held").resets.len(),
            3,
            "`inherit` gives the setting back to the system as a reset does"
        );
    }

    #[test]
    fn a_removed_device_is_forgotten() {
        let mut book: DeviceBook<Fake> = DeviceBook::new();
        let mouse = device(1, DeviceKind::Mouse);
        book.add(&mouse, Fake::default());
        assert_eq!(book.len(), 1);
        assert!(book.remove(mouse.id).is_some());
        assert!(book.is_empty());
        assert!(book.remove(mouse.id).is_none(), "a second removal removes nothing");
        // A reload after the removal has nobody to apply to, and a device that returns is new.
        assert!(book.set_input(mouse_speed(0.5)).is_empty());
        book.add(&mouse, Fake::default());
        assert_eq!(book.device(mouse.id).expect("held").seen, vec![Setting::Speed(0.5)]);
    }

    #[test]
    fn a_refusal_is_reported_once_per_device() {
        let mut book: DeviceBook<Fake> = DeviceBook::new();
        book.set_input(mouse_speed(0.5));
        let mouse = device(1, DeviceKind::Mouse);
        let first = book.add(
            &mouse,
            Fake {
                refuse: vec![SettingKind::Speed],
                ..Fake::default()
            },
        );
        assert_eq!(
            first,
            vec![DeviceRefusal {
                device: mouse.id,
                name: "test device".to_owned(),
                setting: SettingKind::Speed,
                refusal: SettingRefusal::Unsupported,
            }]
        );
        assert!(
            book.set_input(mouse_speed(0.75)).is_empty(),
            "the same refusal on a reload is not said again"
        );
        let other = device(2, DeviceKind::Mouse);
        let second = book.add(
            &other,
            Fake {
                refuse: vec![SettingKind::Speed],
                ..Fake::default()
            },
        );
        assert_eq!(second.len(), 1, "another device is told about on its own");
    }

    #[test]
    fn a_recorder_holds_what_the_document_gave_it() {
        let mut book: DeviceBook<VirtualDevice> = DeviceBook::new();
        let mouse = device(1, DeviceKind::Mouse);
        book.add(&mouse, VirtualDevice::default());
        book.set_input(mouse_speed(0.5));
        let held = book.device(mouse.id).expect("held");
        assert_eq!(held.get(SettingKind::Speed), Some(&Setting::Speed(0.5)));
        book.set_input(Input::default());
        assert_eq!(book.device(mouse.id).expect("held").settings().count(), 0);
    }
}
