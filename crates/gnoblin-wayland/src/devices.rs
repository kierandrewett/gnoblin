//! The input devices the seat has: the list the shell reads, and each keyboard's own key state.
//!
//! The backend announces a device with [`InputEvent::DeviceAdded`] and ends it with
//! [`InputEvent::DeviceRemoved`]. This module is what the compositor does with that:
//!
//! ```text
//! DeviceAdded   -> the device list -> Seat.device (one "id:name:caps" item per device)
//!               -> the router's directory, so a binding's device selector can match its name
//! DeviceRemoved -> the same, backwards; the removed device's held keys are forgotten
//! ```
//!
//! # `Seat.device`
//!
//! The main seat's `device` property is a list of text items, one per device in arrival order, each
//! `id:name:caps`: the backend's number for the device, the name it reports and its capabilities
//! joined by commas (`keyboard`, `pointer`, `touch`, `gesture`, `tablet-tool`, `tablet-pad`,
//! `switch`). It is the same shape as `workspaces` (`id:name`) with the capabilities last, so a name
//! that holds a colon still parses: the id is up to the first colon and the capabilities after the
//! last. A claim's `device` selector is the *name* from this list.
//!
//! # A keyboard's own modifiers
//!
//! The seat has one xkb state, shared by every keyboard: a Shift held on the desk keyboard is a Shift
//! on a macro pad's keys too. That is right for typing and wrong for a binding scoped to the pad, so
//! each keyboard also has a state of its own, fed only by its own keys. [`DeviceKeys::key`] updates it
//! and says what that keyboard alone resolves the key to; the router matches a scoped binding against
//! that, and an unscoped binding against the seat's.
//!
//! # Failure modes first
//!
//! 1. **A removed keyboard's key stays held in its own state.** [`DeviceKeys::forget`] drops it with the
//!    device: `a_removed_keyboard_loses_its_modifiers`.
//! 2. **A layout change strands a held modifier.** The device's state is rebuilt from the new keymap and the
//!    keys the device still holds are pressed into it again: `a_keymap_change_keeps_what_a_keyboard_holds`.
//! 3. **One keyboard's Shift changes another's chord.** Each state sees only its own keys:
//!    `a_shift_on_one_keyboard_is_not_a_shift_on_another`.
//! 4. **A device name breaks the list item.** The capabilities are after the last colon, so a name with a colon
//!    in it still parses: `an_item_with_a_colon_in_the_name_parses`.

use std::collections::{BTreeMap, BTreeSet};

use smithay::input::keyboard::xkb;

use gnoblin_backend::DeviceInfo;
use gnoblin_input::DeviceKey;
use gnoblin_types::{DeviceId, Keysym, Modifiers};

use crate::State;
use crate::log;
use crate::tree_frame::{ChordEvent, ChordState};

/// The offset between an `evdev` key code and the XKB key code of the same key.
const KEYCODE_OFFSET: u32 = 8;

/// The modifiers that are toggled rather than held, so another keyboard's lock still applies to a binding
/// scoped to this one: a Caps Lock or Num Lock someone pressed elsewhere is the seat's state, not a key this
/// keyboard is holding.
pub(crate) const LOCKS: Modifiers = Modifiers::LOCK.union(Modifiers::MOD2);

/// One keyboard's own xkb state and the keys it holds.
struct DeviceKeyboard {
    state: xkb::State,
    /// The `evdev` codes this keyboard holds down, so a keymap change can press them again.
    held: BTreeSet<u32>,
}

/// Each keyboard's own key state, built from the seat's keymap.
pub(crate) struct DeviceKeys {
    context: xkb::Context,
    /// The seat's keymap as this module compiled it, or `None` until the seat has one or if it would not compile.
    keymap: Option<xkb::Keymap>,
    /// Which generation of the seat's keymap `keymap` is.
    generation: Option<u64>,
    keyboards: BTreeMap<DeviceId, DeviceKeyboard>,
}

impl std::fmt::Debug for DeviceKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceKeys")
            .field("generation", &self.generation)
            .field("keyboards", &self.keyboards.len())
            .finish_non_exhaustive()
    }
}

impl DeviceKeys {
    /// A table with no keymap and no keyboard.
    pub(crate) fn new() -> Self {
        Self {
            context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymap: None,
            generation: None,
            keyboards: BTreeMap::new(),
        }
    }

    /// Whether the keymap held is the seat's generation `generation`.
    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.generation == Some(generation)
    }

    /// Makes `text` the keymap every keyboard's state is built from.
    ///
    /// Every keyboard's state is rebuilt from it and the keys that keyboard holds are pressed again, so a
    /// layout switch under a held modifier leaves the modifier held. A keymap that does not compile leaves no keymap, and
    /// [`DeviceKeys::key`] says nothing until the next one does.
    pub(crate) fn set_keymap(&mut self, text: &str, generation: u64) {
        self.generation = Some(generation);
        self.keymap = xkb::Keymap::new_from_string(
            &self.context,
            text.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        );
        let Some(keymap) = self.keymap.as_ref() else {
            log::warn!(
                area = "keyboard",
                "the seat's keymap would not compile for the per-device key state"
            );
            self.keyboards.clear();
            return;
        };
        for keyboard in self.keyboards.values_mut() {
            keyboard.state = xkb::State::new(keymap);
            for code in &keyboard.held {
                keyboard
                    .state
                    .update_key((code + KEYCODE_OFFSET).into(), xkb::KeyDirection::Down);
            }
        }
    }

    /// Applies one edge of `device`'s own and says what that keyboard alone makes of the key.
    ///
    /// The state is updated before the symbol is read, as the seat's is, so a modifier key reads
    /// the same on both. `None` when there is no keymap.
    pub(crate) fn key(&mut self, device: DeviceId, keycode: u32, pressed: bool) -> Option<DeviceKey> {
        let keymap = self.keymap.as_ref()?;
        let keyboard = self.keyboards.entry(device).or_insert_with(|| DeviceKeyboard {
            state: xkb::State::new(keymap),
            held: BTreeSet::new(),
        });
        let direction = if pressed {
            keyboard.held.insert(keycode);
            xkb::KeyDirection::Down
        } else {
            keyboard.held.remove(&keycode);
            xkb::KeyDirection::Up
        };
        let code: xkb::Keycode = (keycode + KEYCODE_OFFSET).into();
        keyboard.state.update_key(code, direction);
        Some(DeviceKey {
            symbol: Keysym::new(keyboard.state.key_get_one_sym(code).raw()),
            modifiers: modifiers_of(&keyboard.state),
        })
    }

    /// Forgets a keyboard that was removed, and every key it held.
    pub(crate) fn forget(&mut self, device: DeviceId) {
        self.keyboards.remove(&device);
    }

    /// How many keys a keyboard holds, for a test.
    #[cfg(test)]
    fn held(&self, device: DeviceId) -> usize {
        self.keyboards.get(&device).map_or(0, |keyboard| keyboard.held.len())
    }
}

/// The modifiers a state holds, by the same table the seat's are read with.
fn modifiers_of(state: &xkb::State) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    for (flag, name) in [
        (Modifiers::SHIFT, xkb::MOD_NAME_SHIFT),
        (Modifiers::LOCK, xkb::MOD_NAME_CAPS),
        (Modifiers::CONTROL, xkb::MOD_NAME_CTRL),
        (Modifiers::MOD1, xkb::MOD_NAME_ALT),
        (Modifiers::MOD2, xkb::MOD_NAME_NUM),
        (Modifiers::MOD3, xkb::MOD_NAME_ISO_LEVEL3_SHIFT),
        (Modifiers::SUPER, xkb::MOD_NAME_LOGO),
    ] {
        if state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE) {
            modifiers = modifiers.union(flag);
        }
    }
    modifiers
}

/// One `Seat.device` item: `id:name:caps`.
pub(crate) fn item_of(device: &DeviceInfo) -> String {
    format!("{}:{}:{}", device.id.get(), device.name, device.caps)
}

/// The id, name and capabilities an item states: the id is up to the first colon and the
/// capabilities are after the last, so a name may hold colons.
#[cfg(test)]
fn parse_item(item: &str) -> Option<(u64, &str, &str)> {
    let (id, rest) = item.split_once(':')?;
    let (name, caps) = rest.rsplit_once(':')?;
    Some((id.parse().ok()?, name, caps))
}

/// Splits a claim's text into its device selector and its subject.
///
/// A claim for every device carries the subject alone (a chord, or a gesture's kind). A claim scoped to a
/// device carries the device's name, a line feed and then the subject. A device name is one line, so the first
/// line feed is always the separator.
pub(crate) fn split_selector(text: &str) -> (Option<&str>, &str) {
    match text.split_once('\n') {
        Some((device, subject)) => (Some(device), subject),
        None => (None, text),
    }
}

impl State {
    /// Records a device the backend announced: it attaches to the default seat, joins the list, the router learns
    /// its name, and the list is published. A device announced twice replaces its first entry.
    ///
    /// There is one input seat, the default seat (`seat0` unless the session names another), and every device
    /// attaches to it whatever logical seat the system tagged it with: a plugged-in device that names a seat this
    /// session does not have (libinput's `default`, a udev `WL_SEAT` of another machine's) is not left without
    /// one. A device that was already announced, or one announced while a binding-only claim names it, takes the
    /// same path, so a hot-plugged pad is binding-only from its first key.
    pub(crate) fn device_added(&mut self, mut info: DeviceInfo) {
        if info.seat != self.seat_name {
            log::debug!(
                area = "input",
                device = %info.id,
                tagged = %info.seat,
                seat = %self.seat_name,
                "a device tagged for a seat this session does not have attached to the default seat"
            );
            info.seat = self.seat_name.clone();
        }
        log::debug!(area = "input", device = %info.id, name = %info.name, caps = %info.caps, "a device joined the seat");
        self.keyboard_tables.router.device_added(info.id, &info.name);
        self.keyboard_tables.devices.retain(|known| known.id != info.id);
        self.keyboard_tables.devices.push(info);
        self.publish_devices();
    }

    /// Forgets a device that left: the list, the router's name for it, its keyboard state and every chord press
    /// it still held. A claim whose press that device made is told of the release the device will never send.
    pub(crate) fn device_removed(&mut self, id: DeviceId) {
        let before = self.keyboard_tables.devices.len();
        self.keyboard_tables.devices.retain(|known| known.id != id);
        if self.keyboard_tables.devices.len() == before {
            log::debug!(area = "input", device = %id, "a device that was never announced left");
        }
        // The router forgets that device's held keys and no other device's.
        let dropped = self.keyboard_tables.router.device_removed(id);
        log::debug!(area = "input", device = %id, held = dropped.len(), "a device left the seat");
        self.keyboard_tables.device_keys.forget(id);
        // The seat lets go of every key that keyboard held, so no other keyboard's press of one is taken for a
        // key that is still down.
        self.release_keyboard(id);
        let pending: Vec<(DeviceId, u32)> = self
            .keyboard_tables
            .pressed_claims
            .keys()
            .filter(|(device, _)| *device == id)
            .copied()
            .collect();
        for key in pending {
            if let Some(claim) = self.keyboard_tables.pressed_claims.remove(&key) {
                self.tree.chord(
                    claim.node,
                    ChordEvent {
                        id: claim.id,
                        chord: claim.chord,
                        state: ChordState::Release,
                    },
                );
            }
        }
        self.publish_devices();
    }

    /// Publishes the device list on the main seat's `device` property, one `id:name:caps` item per device.
    pub(crate) fn publish_devices(&mut self) {
        let items: Vec<String> = self.keyboard_tables.devices.iter().map(item_of).collect();
        self.composer.publish_main_seat_devices(items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_backend::{DeviceCaps, DeviceKind};

    /// `evdev` codes: left shift, `a`, `f13`, caps lock.
    const LEFT_SHIFT: u32 = 42;
    const KEY_A: u32 = 30;
    const KEY_F13: u32 = 183;

    /// A table over the keymap the default rules give `us`, as the seat's own keymap would be.
    fn table() -> DeviceKeys {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(&context, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("the default us keymap compiles");
        let mut keys = DeviceKeys::new();
        keys.set_keymap(&keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1), 1);
        keys
    }

    const DESK: DeviceId = DeviceId::new(1);
    const PAD: DeviceId = DeviceId::new(2);

    #[test]
    fn a_shift_on_one_keyboard_is_not_a_shift_on_another() {
        let mut keys = table();
        let shifted = keys.key(DESK, LEFT_SHIFT, true).expect("a keymap");
        assert!(
            shifted.modifiers.contains(Modifiers::SHIFT),
            "the desk keyboard holds Shift"
        );
        let on_pad = keys.key(PAD, KEY_F13, true).expect("a keymap");
        assert!(
            !on_pad.modifiers.contains(Modifiers::SHIFT),
            "the pad's own state has no Shift: it is not holding one"
        );
        let on_desk = keys.key(DESK, KEY_A, true).expect("a keymap");
        assert!(on_desk.modifiers.contains(Modifiers::SHIFT));
        assert_eq!(
            on_desk.symbol,
            Keysym::new(0x41),
            "Shift+a is `A` on the keyboard that holds Shift"
        );
        let pad_a = keys.key(PAD, KEY_A, true).expect("a keymap");
        assert_eq!(
            pad_a.symbol,
            Keysym::new(0x61),
            "and plain `a` on the one that does not"
        );
    }

    #[test]
    fn a_released_modifier_is_gone() {
        let mut keys = table();
        keys.key(DESK, LEFT_SHIFT, true);
        let released = keys.key(DESK, LEFT_SHIFT, false).expect("a keymap");
        assert!(released.modifiers.is_empty());
        assert_eq!(keys.held(DESK), 0);
    }

    #[test]
    fn a_removed_keyboard_loses_its_modifiers() {
        let mut keys = table();
        keys.key(DESK, LEFT_SHIFT, true);
        keys.forget(DESK);
        let after = keys.key(DESK, KEY_A, true).expect("a keymap");
        assert!(
            !after.modifiers.contains(Modifiers::SHIFT),
            "a fresh state: nothing is held"
        );
        assert_eq!(keys.held(DESK), 1);
    }

    #[test]
    fn a_keymap_change_keeps_what_a_keyboard_holds() {
        let mut keys = table();
        keys.key(DESK, LEFT_SHIFT, true);
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let german = xkb::Keymap::new_from_names(&context, "", "", "de", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("the de keymap compiles");
        keys.set_keymap(&german.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1), 2);
        assert!(keys.is_current(2) && !keys.is_current(1));
        let still = keys.key(DESK, KEY_A, true).expect("a keymap");
        assert!(
            still.modifiers.contains(Modifiers::SHIFT),
            "the Shift the keyboard holds is pressed into the new state"
        );
    }

    #[test]
    fn a_keymap_that_does_not_compile_says_nothing() {
        let mut keys = table();
        keys.set_keymap("this is not a keymap", 2);
        assert_eq!(keys.key(DESK, KEY_A, true), None);
    }

    #[test]
    fn with_no_keymap_a_key_says_nothing() {
        assert_eq!(DeviceKeys::new().key(DESK, KEY_A, true), None);
    }

    #[test]
    fn an_item_is_the_id_the_name_and_the_capabilities() {
        let pad = DeviceInfo::new(DeviceId::new(7), "Macro Pad", DeviceKind::Keyboard);
        assert_eq!(item_of(&pad), "7:Macro Pad:keyboard");
        let both = pad.with_caps(DeviceCaps::KEYBOARD.union(DeviceCaps::POINTER));
        assert_eq!(item_of(&both), "7:Macro Pad:keyboard,pointer");
    }

    #[test]
    fn an_item_with_a_colon_in_the_name_parses() {
        let odd = DeviceInfo::new(DeviceId::new(3), "Pad: rev 2", DeviceKind::Keyboard);
        let item = item_of(&odd);
        assert_eq!(parse_item(&item), Some((3, "Pad: rev 2", "keyboard")));
    }
}
