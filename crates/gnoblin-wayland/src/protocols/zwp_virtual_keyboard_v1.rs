//! XML protocol `virtual-keyboard-unstable-v1`: the interfaces `zwp_virtual_keyboard_manager_v1`
//! and `zwp_virtual_keyboard_v1`.
//!
//! A virtual keyboard injects key events into whatever has focus, the way a physical one
//! does. It is privileged: the access roster decides which connections may bind the
//! manager, through [`crate::access`].
//!
//! ## The keymap
//!
//! A client sends its own keymap as `xkb_v1` text in a file descriptor. While a virtual
//! keyboard owns the seat's keymap, the seat compiles that text; when the virtual keyboard
//! is destroyed the seat gets its layout keymap back. The text is read from the start of the
//! descriptor, whatever its file position, and cut at its first NUL: clients such as `wtype`
//! count a terminating NUL in `size` without writing it, and xkbcommon does not accept it.
//! Any other keymap format is refused, and so is text xkbcommon cannot compile.
//!
//! ## Failure modes
//!
//! Each one is a test in `tests/virtual_keyboard.rs`:
//!
//! 1. A key before any keymap is a protocol error, `no_keymap`:
//!    `a_key_before_any_keymap_is_a_protocol_error`.
//! 2. Modifiers before any keymap are a protocol error, `no_keymap`:
//!    `modifiers_before_any_keymap_are_a_protocol_error`.
//! 3. A valid keymap is accepted and keys are delivered:
//!    `a_key_after_a_valid_keymap_is_delivered`.
//! 4. The NUL a client counts in `size` is not part of the keymap:
//!    `a_keymap_is_read_up_to_its_first_nul`.
//! 5. A keymap xkbcommon refuses leaves the keyboard with no keymap, so a later key is
//!    `no_keymap`: `an_invalid_keymap_is_refused_and_leaves_no_keymap`.
//! 6. A keymap format other than `xkb_v1` is refused the same way:
//!    `an_unsupported_keymap_format_is_refused_and_leaves_no_keymap`.
//! 7. A privileged client sees the manager: `a_privileged_client_sees_the_virtual_keyboard_manager`.
//! 8. A keymap sent the way `wtype` sends it, text with no NUL and a file position at EOF,
//!    is read from the start and accepted: `a_keymap_sent_the_way_wtype_sends_it_is_accepted`.
//!
//! Destroying a keyboard that still holds keys releases them, and gives the layout keymap
//! back. That path has no test of its own yet; it is covered by reading
//! [`KeyboardUserData::destroyed`].

use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, PoisonError};

use smithay::backend::input::InputTime;
use smithay::reexports::wayland_protocols_misc::zwp_virtual_keyboard_v1::server::{
    zwp_virtual_keyboard_manager_v1::{self as manager, ZwpVirtualKeyboardManagerV1},
    zwp_virtual_keyboard_v1::{self as keyboard, Error as KeyboardError, ZwpVirtualKeyboardV1},
};
use smithay::reexports::wayland_server::backend::{ClientId, GlobalId};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_types::KeyState;

use crate::State;
use crate::access::{Access, Global};
use crate::log;

/// The version of `zwp_virtual_keyboard_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// The only keymap format the protocol defines for a keymap a client sends: xkbcommon text.
const XKB_V1_FORMAT: u32 = 1;

/// The `wl_keyboard` key states a virtual keyboard may send.
const KEY_RELEASED: u32 = 0;
const KEY_PRESSED: u32 = 1;

/// Creates the `zwp_virtual_keyboard_manager_v1` global.
///
/// The access roster is captured in the global's data, so the answer to "may this
/// connection bind it" is the same answer the other privileged globals give.
pub(crate) fn virtual_keyboard_global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ZwpVirtualKeyboardManagerV1, ManagerGlobalData>(
        MANAGER_VERSION,
        ManagerGlobalData {
            access: Arc::clone(access),
        },
    )
}

/// The global data of the manager: who may bind it.
pub(crate) struct ManagerGlobalData {
    /// The roster that answers which connection may.
    access: Arc<Access>,
}

/// The user data of a bound manager object.
pub(crate) struct ManagerUserData;

/// The user data of one virtual keyboard.
pub(crate) struct KeyboardUserData {
    /// The keyboard's identity in the seat's keymap table.
    id: u64,
    /// The keys it holds down, so that a destroyed keyboard can release them.
    held: Mutex<HeldKeys>,
}

/// The keys one virtual keyboard holds down.
#[derive(Default)]
struct HeldKeys {
    /// The `evdev` codes of the keys that are down, in the order they were pressed.
    keys: Vec<u32>,
    /// The time of the last key event in the client's clock; a release reuses it.
    last_time: u32,
}

impl GlobalDispatch2<ZwpVirtualKeyboardManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwpVirtualKeyboardManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerUserData);
        log::debug!(area = "virtual-keyboard", client = ?client.id(), "a client bound the virtual keyboard manager");
    }

    fn can_view(&self, client: &Client) -> bool {
        self.access.allows(&client.id(), Global::VirtualKeyboard)
    }
}

impl Dispatch2<ZwpVirtualKeyboardManagerV1, State> for ManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &ZwpVirtualKeyboardManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            // The compositor has one seat, so the seat the client names is that seat.
            manager::Request::CreateVirtualKeyboard { seat: _, id } => {
                let keyboard_id = state.new_virtual_keyboard();
                data_init.init(
                    id,
                    KeyboardUserData {
                        id: keyboard_id,
                        held: Mutex::new(HeldKeys::default()),
                    },
                );
                log::debug!(
                    area = "virtual-keyboard",
                    keyboard = keyboard_id,
                    "a client created a virtual keyboard"
                );
            }
            _ => log::warn!(
                area = "virtual-keyboard",
                "a manager request this compositor does not serve was ignored"
            ),
        }
    }
}

impl Dispatch2<ZwpVirtualKeyboardV1, State> for KeyboardUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        keyboard_object: &ZwpVirtualKeyboardV1,
        request: keyboard::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            keyboard::Request::Keymap { format, fd, size } => match keymap_text(format, fd, size) {
                Ok(text) => state.set_virtual_keymap(self.id, text),
                Err(reason) => {
                    log::warn!(area = "virtual-keyboard", reason = %reason, "a virtual keyboard's keymap was refused");
                    state.release_virtual_keymap(self.id);
                }
            },
            keyboard::Request::Key {
                time,
                key,
                state: key_state,
            } => {
                if !state.use_virtual_keymap(self.id) {
                    keyboard_object.post_error(KeyboardError::NoKeymap as u32, "no keymap was set");
                    return;
                }
                let edge = match key_state {
                    KEY_PRESSED => KeyState::Pressed,
                    KEY_RELEASED => KeyState::Released,
                    other => {
                        log::warn!(
                            area = "virtual-keyboard",
                            key_state = other,
                            "a key state is not pressed or released"
                        );
                        return;
                    }
                };
                let serial = SERIAL_COUNTER.next_serial();
                state.dispatch_key(key, edge, serial, InputTime::from_millis(time));
                self.track(key, edge, time);
            }
            keyboard::Request::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                if !state.use_virtual_keymap(self.id) {
                    keyboard_object.post_error(KeyboardError::NoKeymap as u32, "no keymap was set");
                    return;
                }
                state.set_modifier_state(mods_depressed, mods_latched, mods_locked, group);
            }
            keyboard::Request::Destroy => {}
            _ => log::warn!(
                area = "virtual-keyboard",
                "a keyboard request this compositor does not serve was ignored"
            ),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &ZwpVirtualKeyboardV1) {
        let (keys, time) = {
            let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            (std::mem::take(&mut held.keys), held.last_time)
        };
        if !keys.is_empty() && state.use_virtual_keymap(self.id) {
            // The releases go through this keyboard's keymap, so the router sees the keys
            // it saw pressed go up.
            for key in keys {
                let serial = SERIAL_COUNTER.next_serial();
                state.dispatch_key(key, KeyState::Released, serial, InputTime::from_millis(time));
            }
        }
        state.release_virtual_keymap(self.id);
        log::debug!(
            area = "virtual-keyboard",
            keyboard = self.id,
            "a virtual keyboard was destroyed"
        );
    }
}

impl KeyboardUserData {
    /// Records a key edge so a later destroy can release the key.
    fn track(&self, key: u32, edge: KeyState, time: u32) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        held.last_time = time;
        match edge {
            KeyState::Pressed => {
                if !held.keys.contains(&key) {
                    held.keys.push(key);
                }
            }
            KeyState::Released => held.keys.retain(|&down| down != key),
        }
    }
}

/// Reads the keymap a client sent: the descriptor's first `size` bytes, cut at the first NUL.
///
/// A format other than `xkb_v1`, or text that is not UTF-8, is refused with its reason.
/// Whether xkbcommon compiles the text is decided when the seat takes it.
fn keymap_text(format: u32, fd: OwnedFd, size: u32) -> Result<String, &'static str> {
    if format != XKB_V1_FORMAT {
        return Err("the keymap format is not xkb_v1");
    }
    let bytes = read_keymap_bytes(&File::from(fd), size).map_err(|_| "the keymap descriptor could not be read")?;
    String::from_utf8(bytes).map_err(|_| "the keymap is not UTF-8")
}

/// Reads up to `size` bytes from the start of the descriptor, and stops at the first NUL.
///
/// The read starts at offset 0 and leaves the file position alone: a client such as `wtype`
/// writes the keymap and sends the descriptor without rewinding it, so the position is at the
/// end of the text. The descriptor may also be shorter than `size`, because `size` counts the
/// terminating NUL and clients do not always write it; the end of the file then ends the
/// keymap, as the zero-filled tail of a mapping would.
fn read_keymap_bytes(file: &File, size: u32) -> std::io::Result<Vec<u8>> {
    let limit = u64::from(size);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut offset = 0u64;
    while offset < limit {
        let want = usize::try_from((limit - offset).min(chunk.len() as u64)).unwrap_or(chunk.len());
        let read = file.read_at(&mut chunk[..want], offset)?;
        if read == 0 {
            break;
        }
        offset += read as u64;
        let text = &chunk[..read];
        if let Some(nul) = text.iter().position(|&byte| byte == 0) {
            bytes.extend_from_slice(&text[..nul]);
            break;
        }
        bytes.extend_from_slice(text);
    }
    Ok(bytes)
}
