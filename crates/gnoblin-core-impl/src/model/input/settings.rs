//! The settings a shell writes to the configuration document: what it wrote to `keyboard`, `mouse` and `touchpad`, and to the compositor's own
//! settings, checked when it is written and kept as one ledger.
//!
//! The compositor reads the device settings from the `input` section of the configuration document, once for each kind of device and not for
//! each device, so a write on one mouse is the setting of every mouse, and another mouse reads it too. Its own settings are the `compositor`
//! section, which it reads when it starts, so a write to them applies at the next login. The ledger is the generation's: it holds what was
//! written, in the order each setting was first written, and a number that grows with each change. `document` is its text in the document's
//! own spelling (`{"input":{"mouse":{"speed":0.5}},"compositor":{"user-shaders":false}}`); a host commits it, and commits it again when
//! `document_revision` has grown: the JavaScript runtime for a TypeScript shell, and for every other shell the core's shell host
//! (`crate::document`). Writing none takes a setting back to the system's. This module reaches no compositor: a write is checked
//! against the ranges the configuration schema accepts, and nothing more.

use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res, Val};

use super::devices::InputDeviceKind;
use super::with;

/// The acceleration profiles, in the spelling of the catalog's `acceleration_profile` and of the document.
const PROFILES: [&str; 3] = ["default", "flat", "adaptive"];

/// One setting of a kind of device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeviceSetting {
    RepeatRate,
    RepeatDelay,
    MouseSensitivity,
    MouseAcceleration,
    MouseNaturalScroll,
    TouchpadSensitivity,
    TouchpadAcceleration,
    TouchpadNaturalScroll,
    TapToClick,
    /// `compositor.user-shaders`, the compositor's own setting (`gnoblin.compositor.user_shaders`).
    UserShaders,
}

/// What a setting holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DeviceSettingValue {
    Number(f64),
    Flag(bool),
    Profile(&'static str),
}

impl DeviceSetting {
    /// The kind of device it belongs to, none for a setting of the compositor.
    fn kind(self) -> Option<InputDeviceKind> {
        match self {
            DeviceSetting::RepeatRate | DeviceSetting::RepeatDelay => Some(InputDeviceKind::Keyboard),
            DeviceSetting::MouseSensitivity | DeviceSetting::MouseAcceleration | DeviceSetting::MouseNaturalScroll => {
                Some(InputDeviceKind::Mouse)
            }
            DeviceSetting::TouchpadSensitivity
            | DeviceSetting::TouchpadAcceleration
            | DeviceSetting::TouchpadNaturalScroll
            | DeviceSetting::TapToClick => Some(InputDeviceKind::Touchpad),
            DeviceSetting::UserShaders => None,
        }
    }

    /// The name of the property on the class of the kind.
    pub(super) fn property(self) -> &'static str {
        match self {
            DeviceSetting::RepeatRate => "repeat_rate",
            DeviceSetting::RepeatDelay => "repeat_delay",
            DeviceSetting::MouseSensitivity | DeviceSetting::TouchpadSensitivity => "sensitivity",
            DeviceSetting::MouseAcceleration | DeviceSetting::TouchpadAcceleration => "acceleration",
            DeviceSetting::MouseNaturalScroll | DeviceSetting::TouchpadNaturalScroll => "natural_scroll",
            DeviceSetting::TapToClick => "tap_to_click",
            DeviceSetting::UserShaders => "user_shaders",
        }
    }

    /// The key of the setting in its group of the document.
    fn key(self) -> &'static str {
        match self {
            DeviceSetting::RepeatRate => "repeat-interval",
            DeviceSetting::RepeatDelay => "delay",
            DeviceSetting::MouseSensitivity | DeviceSetting::TouchpadSensitivity => "speed",
            DeviceSetting::MouseAcceleration | DeviceSetting::TouchpadAcceleration => "accel-profile",
            DeviceSetting::MouseNaturalScroll | DeviceSetting::TouchpadNaturalScroll => "natural-scroll",
            DeviceSetting::TapToClick => "tap-to-click",
            DeviceSetting::UserShaders => "user-shaders",
        }
    }

    /// What the setting accepts, for the refusal.
    fn accepts(self) -> &'static str {
        match self {
            DeviceSetting::RepeatRate => "a number of repeats per second from 0.1 to 1000",
            DeviceSetting::RepeatDelay => "a whole number of milliseconds from 1 to 10000",
            DeviceSetting::MouseSensitivity | DeviceSetting::TouchpadSensitivity => "a number from -1 to 1",
            DeviceSetting::MouseAcceleration | DeviceSetting::TouchpadAcceleration => "one of default, flat, adaptive",
            DeviceSetting::MouseNaturalScroll
            | DeviceSetting::TouchpadNaturalScroll
            | DeviceSetting::TapToClick
            | DeviceSetting::UserShaders => "true or false",
        }
    }

    /// What the document holds for `value`.
    fn document(self, value: DeviceSettingValue) -> String {
        match (self, value) {
            // The document stores the time between two repeats, in whole milliseconds.
            (DeviceSetting::RepeatRate, DeviceSettingValue::Number(rate)) => number_text((1000.0 / rate).round()),
            (_, DeviceSettingValue::Number(number)) => number_text(number),
            (_, DeviceSettingValue::Flag(flag)) => flag.to_string(),
            (_, DeviceSettingValue::Profile(name)) => format!("\"{name}\""),
        }
    }
}

/// A number as JSON writes it: a whole number has no fraction.
pub(super) fn number_text(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// What a shell wrote to the settings of the devices in this generation.
#[derive(Default)]
pub(in crate::model) struct SettingsLedger {
    /// Each written setting, in the order it was first written.
    entries: Vec<(DeviceSetting, DeviceSettingValue)>,
    revision: u64,
}

impl SettingsLedger {
    pub(super) fn get(&self, setting: DeviceSetting) -> Option<DeviceSettingValue> {
        self.entries
            .iter()
            .find(|(held, _)| *held == setting)
            .map(|(_, value)| *value)
    }

    /// Writes or, for none, gives up a setting. True when the ledger changed.
    fn set(&mut self, setting: DeviceSetting, value: Option<DeviceSettingValue>) -> bool {
        let at = self.entries.iter().position(|(held, _)| *held == setting);
        let changed = match (at, value) {
            (Some(at), Some(value)) => {
                let same = self.entries[at].1 == value;
                self.entries[at].1 = value;
                !same
            }
            (None, Some(value)) => {
                self.entries.push((setting, value));
                true
            }
            (Some(at), None) => {
                self.entries.remove(at);
                true
            }
            (None, None) => false,
        };
        if changed {
            self.revision += 1;
        }
        changed
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// The sections of the configuration document the written settings make: `input` with a group for each kind of device, then
    /// `compositor`; `{}` when none is written.
    pub(super) fn document(&self) -> String {
        let mut groups: Vec<(&'static str, Vec<String>)> = Vec::new();
        let mut compositor: Vec<String> = Vec::new();
        for (setting, value) in &self.entries {
            let item = format!("\"{}\":{}", setting.key(), setting.document(*value));
            match setting.kind() {
                Some(kind) => match groups.iter_mut().find(|(name, _)| *name == kind.group()) {
                    Some((_, items)) => items.push(item),
                    None => groups.push((kind.group(), vec![item])),
                },
                None => compositor.push(item),
            }
        }
        let mut sections: Vec<String> = Vec::new();
        if !groups.is_empty() {
            let kinds: Vec<String> = groups
                .iter()
                .map(|(name, items)| format!("\"{name}\":{{{}}}", items.join(",")))
                .collect();
            sections.push(format!("\"input\":{{{}}}", kinds.join(",")));
        }
        if !compositor.is_empty() {
            sections.push(format!("\"compositor\":{{{}}}", compositor.join(",")));
        }
        format!("{{{}}}", sections.join(","))
    }
}

/// What a setting holds as a value for the shell: none when it is not written.
pub(super) fn read(cx: &Cx<'_>, setting: DeviceSetting) -> Res<Val> {
    let Some(value) = with(cx.ctx(), |table| table.settings.get(setting)) else {
        return Ok(Val::None);
    };
    Ok(match value {
        DeviceSettingValue::Number(number) if setting == DeviceSetting::RepeatDelay => Val::Uint(number as u64),
        DeviceSettingValue::Number(number) => Val::Double(number),
        DeviceSettingValue::Flag(flag) => Val::Bool(flag),
        DeviceSettingValue::Profile(name) => cx.enum_val("acceleration_profile", name)?,
    })
}

/// Checks `value` for `setting`. None for a value of none, which takes the setting back to the system's.
fn accept(cx: &Cx<'_>, setting: DeviceSetting, value: &Val, target: &str) -> Res<Option<DeviceSettingValue>> {
    if value.is_none() {
        return Ok(None);
    }
    let refuse = |shown: String| Fail::type_error(format!("{target}: must be {}, got {shown}", setting.accepts()));
    Ok(Some(match setting {
        DeviceSetting::RepeatRate => {
            let rate = value.double()?;
            if !(rate.is_finite() && (0.1..=1000.0).contains(&rate)) {
                return Err(refuse(number_text(rate)));
            }
            DeviceSettingValue::Number(rate)
        }
        DeviceSetting::RepeatDelay => {
            let delay = value.uint()?;
            if !(1..=10_000).contains(&delay) {
                return Err(refuse(delay.to_string()));
            }
            DeviceSettingValue::Number(delay as f64)
        }
        DeviceSetting::MouseSensitivity | DeviceSetting::TouchpadSensitivity => {
            let speed = value.double()?;
            if !(speed.is_finite() && (-1.0..=1.0).contains(&speed)) {
                return Err(refuse(number_text(speed)));
            }
            DeviceSettingValue::Number(speed)
        }
        DeviceSetting::MouseAcceleration | DeviceSetting::TouchpadAcceleration => {
            let wanted = value.enum_value()?;
            let found = PROFILES.iter().find(|name| {
                cx.enum_val("acceleration_profile", name)
                    .and_then(|known| known.enum_value())
                    .is_ok_and(|known| known == wanted)
            });
            match found {
                Some(name) => DeviceSettingValue::Profile(name),
                None => return Err(refuse(format!("enum value {wanted}"))),
            }
        }
        DeviceSetting::MouseNaturalScroll
        | DeviceSetting::TouchpadNaturalScroll
        | DeviceSetting::TapToClick
        | DeviceSetting::UserShaders => DeviceSettingValue::Flag(value.bool()?),
    }))
}

/// Writes `setting` for the device `device`, which is where the refusal says it was. The setting is the kind's: the devices of the kind
/// are told the property changed.
pub(super) fn write(cx: &Cx<'_>, device: Obj, setting: DeviceSetting, value: &Val) -> Res<()> {
    let (id, kind) = with(cx.ctx(), |table| {
        table.devices.get(&device).map(|held| (held.id.clone(), held.kind))
    })
    .ok_or_else(|| Fail::stale("the device is gone"))?;
    let target = format!("{}#{id}.{}", kind.title(), setting.property());
    let accepted = accept(cx, setting, value, &target)?;
    let (changed, peers) = with(cx.ctx(), |table| {
        let changed = table.settings.set(setting, accepted);
        let peers: Vec<Obj> = table
            .devices
            .iter()
            .filter(|(_, held)| Some(held.kind) == setting.kind())
            .map(|(obj, _)| *obj)
            .collect();
        (changed, peers)
    });
    if changed {
        for peer in peers {
            cx.changed_on(peer, setting.property());
        }
    }
    Ok(())
}

/// Writes `setting` of the compositor (`gnoblin.compositor`). The setting applies at the next login: the ledger holds it, the document the host
/// commits has it, and the object that was written is told the property changed when it did.
pub(super) fn write_compositor(cx: &Cx<'_>, setting: DeviceSetting, value: &Val) -> Res<()> {
    let target = format!("CompositorSettings.{}", setting.property());
    let accepted = accept(cx, setting, value, &target)?;
    if with(cx.ctx(), |table| table.settings.set(setting, accepted)) {
        cx.changed_on(cx.me(), setting.property());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_writes_the_document_in_the_order_the_settings_were_first_written() {
        let mut ledger = SettingsLedger::default();
        assert_eq!(ledger.document(), "{}");
        assert!(ledger.set(DeviceSetting::MouseSensitivity, Some(DeviceSettingValue::Number(0.5))));
        assert!(ledger.set(DeviceSetting::RepeatRate, Some(DeviceSettingValue::Number(40.0))));
        assert!(ledger.set(
            DeviceSetting::MouseAcceleration,
            Some(DeviceSettingValue::Profile("flat"))
        ));
        assert!(
            !ledger.set(DeviceSetting::RepeatRate, Some(DeviceSettingValue::Number(40.0))),
            "the same value is no change"
        );
        assert_eq!(
            ledger.document(),
            r#"{"input":{"mouse":{"speed":0.5,"accel-profile":"flat"},"keyboard":{"repeat-interval":25}}}"#
        );
        assert!(ledger.set(DeviceSetting::MouseSensitivity, None));
        assert!(!ledger.set(DeviceSetting::MouseSensitivity, None));
        assert_eq!(ledger.revision(), 4);
    }

    #[test]
    fn the_compositor_settings_make_a_section_after_the_input_section_whatever_the_order_they_were_written_in() {
        let mut ledger = SettingsLedger::default();
        assert!(ledger.set(DeviceSetting::UserShaders, Some(DeviceSettingValue::Flag(false))));
        assert_eq!(ledger.document(), r#"{"compositor":{"user-shaders":false}}"#);
        assert!(ledger.set(DeviceSetting::MouseSensitivity, Some(DeviceSettingValue::Number(0.5))));
        assert_eq!(
            ledger.document(),
            r#"{"input":{"mouse":{"speed":0.5}},"compositor":{"user-shaders":false}}"#
        );
        assert!(ledger.set(DeviceSetting::UserShaders, None));
        assert_eq!(ledger.document(), r#"{"input":{"mouse":{"speed":0.5}}}"#);
    }
}
