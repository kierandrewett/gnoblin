//! Which device a binding takes its events from.
//!
//! A binding with no selector takes events from every device, as it always did. A binding with a
//! selector takes them only from the devices that selector names, so a macro pad can drive shortcuts
//! without the keyboard on the desk firing them too.
//!
//! The selector is the device's **name**, as the device reports it (`Seat.device` lists the names). It
//! is not the backend's number: a number is allocated when the device arrives and a replugged pad gets
//! a new one, while a binding is written once and has to survive the replug. Two devices that report
//! the same name (two identical pads) are both matched; a binding that wants one of them has no name to
//! tell them apart by.
//!
//! The name is matched exactly, byte for byte. A selector is one line, so it can travel in a claim's
//! text, which splits on the first line feed.

use std::fmt;

use gnoblin_types::{DeviceId, KeyCode, Keysym, Modifiers};

/// What one keyboard's own state says about a key edge.
///
/// The seat's keymap state is shared by every keyboard, so a Shift held on one keyboard changes the
/// symbol and the modifiers another keyboard's keys report. A binding scoped to a device matches what
/// *that device* says instead, so a Shift on the desk keyboard does not turn a macro pad's `F13` into
/// `Shift+F13`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceKey {
    /// The keysym the layout resolves the key to with only this device's keys held.
    pub symbol: Keysym,
    /// The modifiers this device's own held keys make.
    pub modifiers: Modifiers,
}

/// Why a device name is not a selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectorError {
    /// The name is empty, which names no device.
    Empty,
    /// The name spans lines or holds a control character, which a device name never does.
    NotOneLine,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "a device selector is not empty",
            Self::NotOneLine => "a device selector is one line without control characters",
        })
    }
}

impl std::error::Error for SelectorError {}

/// The name of the device a binding takes its events from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceSelector(String);

impl DeviceSelector {
    /// A selector for the device that reports this name.
    ///
    /// # Errors
    ///
    /// [`SelectorError`] for an empty name or one that is not a single line.
    pub fn new(name: &str) -> Result<Self, SelectorError> {
        if name.is_empty() {
            return Err(SelectorError::Empty);
        }
        if name.chars().any(char::is_control) {
            return Err(SelectorError::NotOneLine);
        }
        Ok(Self(name.to_owned()))
    }

    /// The device name this selector matches.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether a device that reports `name` is the one this selector names.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        self.0 == name
    }
}

impl fmt::Display for DeviceSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A key or button a device holds, as the router remembers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HeldEdge {
    /// A key, by the hardware code and the device it is held on.
    Key(DeviceId, KeyCode),
    /// A button, by number and the device it is held on.
    Button(DeviceId, u8),
}

impl HeldEdge {
    /// The device that holds it.
    #[must_use]
    pub const fn device(self) -> DeviceId {
        match self {
            Self::Key(device, _) | Self::Button(device, _) => device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selector_is_a_non_empty_single_line() {
        assert_eq!(DeviceSelector::new(""), Err(SelectorError::Empty));
        assert_eq!(DeviceSelector::new("pad\nextra"), Err(SelectorError::NotOneLine));
        assert_eq!(DeviceSelector::new("pad\u{0}"), Err(SelectorError::NotOneLine));
        assert_eq!(
            DeviceSelector::new("Macro Pad: 12").expect("ok").as_str(),
            "Macro Pad: 12"
        );
    }

    #[test]
    fn a_selector_matches_the_exact_name_only() {
        let selector = DeviceSelector::new("Macro Pad").expect("ok");
        assert!(selector.matches("Macro Pad"));
        assert!(!selector.matches("macro pad"));
        assert!(!selector.matches("Macro Pad 2"));
    }
}
