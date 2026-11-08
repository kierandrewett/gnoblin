//! `zwp_virtual_keyboard_manager_v1` and `zwp_virtual_keyboard_v1` against a real client.
//!
//! The scenarios are the failure modes listed at the top of
//! `src/protocols/zwp_virtual_keyboard_v1.rs`. The keymap a virtual keyboard installs is
//! the seat's keymap while it is in use, so each test is a protocol-level check: what the
//! server accepts, refuses, or ends the connection over.

mod owned;

use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, BorrowedFd};

use gnoblin_layers::InputPolicy;
use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{KeyState, OutputId, Rect, Size};
use gnoblin_wayland::{ClientRole, Modal, ModalEvent};
use owned::{Connected, Session};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{Connection, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// The `NoKeymap` error of `zwp_virtual_keyboard_v1`.
const NO_KEYMAP: u32 = 0;
/// The `xkb_v1` keymap format of `zwp_virtual_keyboard_v1`.
const XKB_V1: u32 = 1;

/// A keymap that compiles against the system's xkb data.
const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
    xkb_geometry { include \"pc(pc105)\" };\n\
};\n";

/// What the client has heard. Nothing the tests need is sent back, so this is empty.
#[derive(Default)]
struct App;

impl wayland_client::Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(App: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(App: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(App: ignore ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

/// Binds the manager, which a privileged client sees.
fn manager(client: &Connected<App>) -> ZwpVirtualKeyboardManagerV1 {
    client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("a privileged client sees the virtual keyboard manager")
}

/// A virtual keyboard on the session's seat.
fn keyboard(client: &Connected<App>) -> ZwpVirtualKeyboardV1 {
    let seat: wl_seat::WlSeat = client
        .globals
        .bind(&client.qh, 1..=5, ())
        .expect("the session advertises a seat");
    manager(client).create_virtual_keyboard(&seat, &client.qh, ())
}

/// A readable descriptor holding exactly `bytes`, rewound to the start.
fn file_with(bytes: &[u8]) -> std::fs::File {
    let mut file = tempfile::tempfile().expect("a temporary file for the keymap");
    file.write_all(bytes).expect("the keymap is written");
    file.seek(SeekFrom::Start(0)).expect("the keymap is rewound");
    file
}

/// Sends a keymap in `format` whose descriptor holds `bytes`, with `size` as the length.
fn send_keymap(keyboard: &ZwpVirtualKeyboardV1, format: u32, bytes: &[u8], size: u32) {
    let file = file_with(bytes);
    let fd: BorrowedFd<'_> = file.as_fd();
    keyboard.keymap(format, fd, size);
}

/// Sends a keymap the way `wtype` does: the descriptor holds the keymap text with no NUL,
/// its file position is left at the end of what was written, and `size` counts the NUL
/// that is not there. Only a read from offset 0 sees the keymap.
fn send_keymap_as_wtype(keyboard: &ZwpVirtualKeyboardV1, bytes: &[u8]) {
    let mut file = tempfile::tempfile().expect("a temporary file for the keymap");
    file.write_all(bytes).expect("the keymap is written");
    let fd: BorrowedFd<'_> = file.as_fd();
    keyboard.keymap(XKB_V1, fd, bytes.len() as u32 + 1);
}

/// A key press of `key` as the protocol sends it.
fn press(keyboard: &ZwpVirtualKeyboardV1, key: u32) {
    keyboard.key(0, key, 1);
}

#[test]
fn a_privileged_client_sees_the_virtual_keyboard_manager() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let client = Connected::connect(handle.socket(), App);
        assert!(client.has("zwp_virtual_keyboard_manager_v1"));
    });
}

#[test]
fn a_key_before_any_keymap_is_a_protocol_error() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        press(&keyboard, 30);
        client.expect_protocol_error("zwp_virtual_keyboard_v1", NO_KEYMAP);
    });
}

#[test]
fn modifiers_before_any_keymap_are_a_protocol_error() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        keyboard.modifiers(1, 0, 0, 0);
        client.expect_protocol_error("zwp_virtual_keyboard_v1", NO_KEYMAP);
    });
}

#[test]
fn a_key_after_a_valid_keymap_is_delivered() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        send_keymap(&keyboard, XKB_V1, KEYMAP.as_bytes(), KEYMAP.len() as u32);
        press(&keyboard, 30);
        keyboard.key(0, 30, 0);
        client.roundtrip();
        assert!(
            client.has("zwp_virtual_keyboard_manager_v1"),
            "the connection survives a valid keymap"
        );
    });
}

#[test]
fn a_keymap_is_read_up_to_its_first_nul() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        // The descriptor carries the keymap, a NUL, and bytes that are not part of it.
        let mut bytes = KEYMAP.as_bytes().to_vec();
        bytes.push(0);
        bytes.extend_from_slice(b"trailing garbage that is not xkb");
        send_keymap(&keyboard, XKB_V1, &bytes, bytes.len() as u32);
        press(&keyboard, 30);
        client.roundtrip();
        assert!(
            client.has("zwp_virtual_keyboard_manager_v1"),
            "the keymap ends at its NUL"
        );
    });
}

#[test]
fn a_keymap_sent_the_way_wtype_sends_it_is_accepted() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        send_keymap_as_wtype(&keyboard, KEYMAP.as_bytes());
        press(&keyboard, 30);
        client.roundtrip();
        assert!(
            client.has("zwp_virtual_keyboard_manager_v1"),
            "a keymap one byte short of its size is still the keymap"
        );
    });
}

#[test]
fn an_invalid_keymap_is_refused_and_leaves_no_keymap() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        let garbage = b"this is not an xkb keymap at all";
        send_keymap(&keyboard, XKB_V1, garbage, garbage.len() as u32);
        press(&keyboard, 30);
        client.expect_protocol_error("zwp_virtual_keyboard_v1", NO_KEYMAP);
    });
}

#[test]
fn an_unsupported_keymap_format_is_refused_and_leaves_no_keymap() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        send_keymap(&keyboard, 7, KEYMAP.as_bytes(), KEYMAP.len() as u32);
        press(&keyboard, 30);
        client.expect_protocol_error("zwp_virtual_keyboard_v1", NO_KEYMAP);
    });
}

/// A key release of `key` as the protocol sends it.
fn release(keyboard: &ZwpVirtualKeyboardV1, key: u32) {
    keyboard.key(0, key, 0);
}

/// The output the headless backend starts with.
fn default_output() -> OutputId {
    OutputId::new(0)
}

/// An opaque white modal over the default output, the shape a consent prompt takes.
fn modal() -> Modal {
    let size = Size { width: 2, height: 2 };
    let image = Image::new(size, 8, vec![255; 16]).expect("a 2x2 image");
    let overlay = Overlay::new(Rect::from_size(size), &image).expect("a non-empty overlay");
    Modal::new(InputPolicy::ModalToOutput, default_output(), overlay)
}

#[test]
fn a_virtual_keyboard_key_under_a_modal_goes_to_the_modal() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        send_keymap(&keyboard, XKB_V1, KEYMAP.as_bytes(), KEYMAP.len() as u32);
        handle.control.set_modal(Some(modal()));
        client.roundtrip();
        press(&keyboard, 30);
        release(&keyboard, 30);
        client.roundtrip();
        assert_eq!(
            handle.control.modal_events(),
            Some(vec![
                ModalEvent::Key {
                    keycode: 30,
                    state: KeyState::Pressed,
                },
                ModalEvent::Key {
                    keycode: 30,
                    state: KeyState::Released,
                },
            ]),
            "the modal takes the virtual keyboard's edges, not the client"
        );
    });
}

#[test]
fn a_key_held_when_its_keyboard_is_destroyed_is_released_to_the_modal() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let keyboard = keyboard(&client);
        send_keymap(&keyboard, XKB_V1, KEYMAP.as_bytes(), KEYMAP.len() as u32);
        press(&keyboard, 30);
        client.roundtrip();
        handle.control.set_modal(Some(modal()));
        client.roundtrip();
        keyboard.destroy();
        client.roundtrip();
        assert_eq!(
            handle.control.modal_events(),
            Some(vec![ModalEvent::Key {
                keycode: 30,
                state: KeyState::Released,
            }]),
            "the release a destroyed keyboard sends is taken by the modal too"
        );
    });
}
