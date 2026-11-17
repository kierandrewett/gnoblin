//! The library's built-in layouts, from the shell's side: a typed property set going in and answers coming out.
//!
//! The tiling algorithms (master-stack, grid, monocle, dwindle, strip, arc, carousel) live in `libgnoblin-std`. The shell names a kind
//! and gives options; `gnoblin_std_abi::layouts::LAYOUT_KINDS` declares which options and actions each kind has, and the library
//! refuses any other. The typed layouts of this crate ([`DwindleLayout`](crate::DwindleLayout) and the rest) only build the property
//! set and call the three functions below, so a change to an algorithm or an option is a change to the library alone.
#![allow(unsafe_code)]

use gnoblin_core::Rect;
use gnoblin_core_abi::GnoblinStr;
use gnoblin_std_abi::{
    GnoblinStdOption, GnoblinStdRect, GnoblinStdRingSlot, STD_VALUE_BOOL, STD_VALUE_ENUM, STD_VALUE_NODE,
    STD_VALUE_NONE, STD_VALUE_NUMBER, STD_VALUE_TEXT,
};

use crate::api::Std;
use crate::error::{Error, ErrorKind, Result};
use crate::workspace::Workspace;

/// One value of a built-in layout's or binding's option, argument or answer.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutValue {
    /// Absent: an optional option that is not set, or an action that answers nothing.
    None,
    /// A boolean.
    Bool(bool),
    /// A number (a whole number for an integer option).
    Number(f64),
    /// The number of an enumeration member, as the table declares them.
    Enum(u32),
    /// A window, by node id.
    Node(u64),
    /// Text (a binding's trigger).
    Text(String),
}

impl LayoutValue {
    pub(crate) fn record(&self, name: &str) -> GnoblinStdOption {
        let mut raw = GnoblinStdOption {
            size: size_of::<GnoblinStdOption>() as u32,
            kind: STD_VALUE_NONE,
            flag: 0,
            reserved: 0,
            name: GnoblinStr::borrowed(name),
            text: GnoblinStr::borrowed(""),
            number: 0.0,
            node: 0,
        };
        match self {
            LayoutValue::None => {}
            LayoutValue::Bool(flag) => {
                raw.kind = STD_VALUE_BOOL;
                raw.flag = u32::from(*flag);
            }
            LayoutValue::Number(number) => {
                raw.kind = STD_VALUE_NUMBER;
                raw.number = *number;
            }
            LayoutValue::Enum(member) => {
                raw.kind = STD_VALUE_ENUM;
                raw.number = f64::from(*member);
            }
            LayoutValue::Node(node) => {
                raw.kind = STD_VALUE_NODE;
                raw.node = *node;
            }
            LayoutValue::Text(text) => {
                raw.kind = STD_VALUE_TEXT;
                raw.text = GnoblinStr::borrowed(text);
            }
        }
        raw
    }

    fn from_record(raw: &GnoblinStdOption) -> LayoutValue {
        match raw.kind {
            STD_VALUE_BOOL => LayoutValue::Bool(raw.flag != 0),
            STD_VALUE_NUMBER => LayoutValue::Number(raw.number),
            STD_VALUE_ENUM => LayoutValue::Enum(raw.number as u32),
            STD_VALUE_NODE => LayoutValue::Node(raw.node),
            _ => LayoutValue::None,
        }
    }

    /// The number, when it is one.
    pub fn number(&self) -> Option<f64> {
        match self {
            LayoutValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// The boolean, when it is one.
    pub fn flag(&self) -> Option<bool> {
        match self {
            LayoutValue::Bool(flag) => Some(*flag),
            _ => None,
        }
    }
}

/// A built-in layout as the shell names it: the kind and the options it sets (the others have the library's defaults).
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinLayout {
    pub(crate) kind: u32,
    pub(crate) options: Vec<(&'static str, LayoutValue)>,
}

impl BuiltinLayout {
    /// The `STD_BUILTIN_*` kind.
    pub fn kind(&self) -> u32 {
        self.kind
    }

    /// The options that are set, by name.
    pub fn options(&self) -> &[(&'static str, LayoutValue)] {
        &self.options
    }
}

impl Workspace {
    /// Gives the workspace the built-in layout `kind`, with the options set (the library's defaults for the rest).
    ///
    /// # Errors
    ///
    /// The kind or an option is unknown, a value has the wrong type or is out of range, or the generation retired.
    pub(crate) fn install_builtin(&self, layout: &BuiltinLayout) -> Result<()> {
        let records: Vec<GnoblinStdOption> = layout.options.iter().map(|(name, value)| value.record(name)).collect();
        let std = self.std();
        // SAFETY: the records and the names they borrow are live for the call; the library copies what it keeps.
        std.check(unsafe {
            (std.api().workspace_set_layout_kind)(std.ctx(), self.raw(), layout.kind, records.as_ptr(), records.len())
        })
        .map(|_| ())
    }

    /// The `STD_BUILTIN_*` kind of the workspace's layout, or `None` for the floating layout and a layout of the shell's own.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn layout_kind(&self) -> Result<Option<u32>> {
        let std = self.std();
        let mut kind = 0;
        // SAFETY: `workspace_layout_kind` writes one number to the pointer it is given.
        std.check(unsafe { (std.api().workspace_layout_kind)(std.ctx(), self.raw(), &mut kind) })?;
        Ok((kind != 0).then_some(kind))
    }

    /// Changes one option of the workspace's built-in layout: the tree it keeps is kept, and the workspace computes again.
    ///
    /// # Errors
    ///
    /// The layout is not a built-in one, the option is unknown, the value has the wrong type or is out of range, or the generation
    /// retired.
    pub fn set_layout_option(&self, name: &str, value: LayoutValue) -> Result<()> {
        let record = value.record(name);
        let std = self.std();
        // SAFETY: the record and the name it borrows are live for the call.
        std.check(unsafe { (std.api().workspace_layout_set)(std.ctx(), self.raw(), &record) })
            .map(|_| ())
    }

    /// One option of the workspace's built-in layout.
    ///
    /// # Errors
    ///
    /// The layout is not a built-in one, the option is unknown, or the generation retired.
    pub fn layout_option(&self, name: &str) -> Result<LayoutValue> {
        let std = self.std();
        let mut out = LayoutValue::None.record("");
        // SAFETY: `workspace_layout_get` reads `name.len()` bytes and fills the record it is given up to its size.
        std.check(unsafe {
            (std.api().workspace_layout_get)(std.ctx(), self.raw(), name.as_ptr(), name.len(), &mut out)
        })?;
        Ok(LayoutValue::from_record(&out))
    }

    /// Runs an action of the workspace's built-in layout and returns what it answers ([`LayoutValue::None`] when it answers nothing).
    /// An action that needs a window works on the one that has the keyboard.
    ///
    /// # Errors
    ///
    /// The layout is not a built-in one, the action is unknown, an argument is missing, of the wrong type or out of range, or the
    /// generation retired.
    pub fn layout_act(&self, action: &str, args: &[(&str, LayoutValue)]) -> Result<LayoutValue> {
        let records: Vec<GnoblinStdOption> = args.iter().map(|(name, value)| value.record(name)).collect();
        let std = self.std();
        let mut out = LayoutValue::None.record("");
        // SAFETY: the records, the names they borrow and `action` are live for the call; `out` is filled up to its size.
        std.check(unsafe {
            (std.api().workspace_layout_act)(
                std.ctx(),
                self.raw(),
                action.as_ptr(),
                action.len(),
                records.as_ptr(),
                records.len(),
                &mut out,
            )
        })?;
        Ok(LayoutValue::from_record(&out))
    }
}

/// One card of the ring that [`Std::ring`] computes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingSlot {
    /// The projected rectangle.
    pub rect: Rect,
    /// Fade from 0.4 at the back to 1 at the front.
    pub opacity: f64,
    /// Degrees: 0 at the front, 180 at the back, positive to the right of the front card.
    pub rotation: f64,
    /// Negative in front of the camera, more so for the back of the ring.
    pub z: f64,
}

/// The options of the ring, which the carousel lays its cards out with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingOptions {
    /// Vertical field of view in degrees. Default 40.
    pub fov: f64,
    /// A card's width as a fraction of the area's. Default 0.36.
    pub card_width: f64,
    /// A card's height as a fraction of the area's. Default 0.6.
    pub card_height: f64,
}

impl Default for RingOptions {
    fn default() -> RingOptions {
        RingOptions {
            fov: 40.0,
            card_width: 0.36,
            card_height: 0.6,
        }
    }
}

impl RingOptions {
    pub(crate) fn options(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("fov", LayoutValue::Number(self.fov)),
            ("card_width", LayoutValue::Number(self.card_width)),
            ("card_height", LayoutValue::Number(self.card_height)),
        ]
    }
}

impl Std {
    /// The ring of `count` cards with the 1-based `focus` at the front, inside `area`, as the library computes it: one slot per card.
    ///
    /// # Errors
    ///
    /// `focus` is not an index in `1..=count`, an option is out of range, or the generation retired.
    pub fn ring(&self, count: usize, focus: usize, area: Rect, options: RingOptions) -> Result<Vec<RingSlot>> {
        let records: Vec<GnoblinStdOption> = options
            .options()
            .iter()
            .map(|(name, value)| value.record(name))
            .collect();
        let raw_area = GnoblinStdRect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        };
        let blank = GnoblinStdRingSlot {
            rect: raw_area,
            opacity: 0.0,
            rotation: 0.0,
            z: 0.0,
        };
        let mut out = vec![blank; count];
        // SAFETY: the records and the area are live for the call, and `out` has room for `count` slots.
        self.check(unsafe {
            (self.api().layout_ring)(
                self.ctx(),
                count,
                focus,
                &raw_area,
                records.as_ptr(),
                records.len(),
                out.as_mut_ptr(),
            )
        })?;
        Ok(out
            .iter()
            .map(|slot| RingSlot {
                rect: Rect::new(slot.rect.x, slot.rect.y, slot.rect.width, slot.rect.height),
                opacity: slot.opacity,
                rotation: slot.rotation,
                z: slot.z,
            })
            .collect())
    }
}

/// The error an action's wrapper gives when the library answers with the wrong kind of value.
pub(crate) fn unexpected(action: &str) -> Error {
    Error::new(
        ErrorKind::Type,
        format!("the library's answer to `{action}` has an unexpected type"),
    )
}

/// The boolean an action answers.
pub(crate) fn answer_flag(action: &str, value: LayoutValue) -> Result<bool> {
    value.flag().ok_or_else(|| unexpected(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_std_abi as abi;
    use gnoblin_std_abi::layouts::{LAYOUT_KINDS, ValueShape};

    #[test]
    fn every_value_round_trips_through_its_record() {
        for value in [
            LayoutValue::None,
            LayoutValue::Bool(true),
            LayoutValue::Number(2.5),
            LayoutValue::Enum(3),
            LayoutValue::Node(9),
        ] {
            assert_eq!(LayoutValue::from_record(&value.record("x")), value);
        }
    }

    #[test]
    fn the_ring_options_name_what_the_carousel_declares() {
        let carousel = LAYOUT_KINDS
            .iter()
            .find(|kind| kind.id == abi::STD_BUILTIN_CAROUSEL)
            .expect("the carousel is declared");
        for (name, value) in RingOptions::default().options() {
            let desc = carousel.option(name).expect("a declared option");
            assert!(matches!(desc.value.ty, ValueShape::Number));
            assert_eq!(
                Some(value.number().expect("a number")),
                match desc.default {
                    abi::layouts::Initial::Number(default) => Some(default),
                    _ => None,
                }
            );
        }
    }
}
