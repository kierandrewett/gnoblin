//! The wlcs C ABI (`wlcs/display_server.h`, `pointer.h`, `touch.h`, `keyboard.h`) over the safe [`ConformanceSession`].
//!
//! wlcs loads the library, reads the `wlcs_server_integration` symbol and calls the function tables below. Every
//! `unsafe` block in the crate is here, for one of two reasons: a raw pointer wlcs handed back is dereferenced, or
//! one of the two `libwayland-client` functions wlcs's own process provides is called. Each states which.
//!
//! The layouts are the headers' at version 4 of `WlcsDisplayServer`, and a `#[repr(C)]` struct that wlcs reads is
//! always the first field of the object it lives in, so a pointer to one is a pointer to the other.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::fd::RawFd;
use std::process;
use std::ptr;
use std::sync::OnceLock;

use gnoblin_backend::TouchPhase;

use crate::harness::{ConformanceSession, Injector};

/// `wl_fixed_t`: signed 24.8 fixed point.
type WlFixed = i32;

/// Rounds a `wl_fixed_t` to whole pixels, which is the unit the session's pointer and touch positions are in.
fn fixed_to_pixels(value: WlFixed) -> i32 {
    value.saturating_add(128) >> 8
}

const WLCS_DISPLAY_SERVER_VERSION: u32 = 4;
const WLCS_SERVER_INTEGRATION_VERSION: u32 = 1;
const WLCS_INTEGRATION_DESCRIPTOR_VERSION: u32 = 1;
const WLCS_POINTER_VERSION: u32 = 1;
const WLCS_TOUCH_VERSION: u32 = 1;
const WLCS_KEYBOARD_VERSION: u32 = 1;

unsafe extern "C" {
    // SAFETY (for both): they are `libwayland-client`'s, which the wlcs executable links, so they resolve when it
    // loads this library. Each is called with a pointer wlcs handed to this library for that purpose.
    fn wl_display_get_fd(display: *mut c_void) -> c_int;
    fn wl_proxy_get_id(proxy: *mut c_void) -> u32;
}

/// `WlcsExtensionDescriptor`.
#[repr(C)]
pub struct WlcsExtensionDescriptor {
    name: *const c_char,
    version: u32,
}

// SAFETY: the name points at a `'static` C string literal and nothing mutates the table.
unsafe impl Sync for WlcsExtensionDescriptor {}

impl WlcsExtensionDescriptor {
    /// An entry for a protocol the session advertises, at the highest version it serves.
    pub(crate) const fn new(name: &'static CStr, version: u32) -> Self {
        Self {
            name: name.as_ptr(),
            version,
        }
    }

    /// The protocol's name.
    pub(crate) fn name(&self) -> &'static str {
        // SAFETY: `new` is the only constructor and it takes a `'static` C string.
        unsafe { CStr::from_ptr(self.name) }.to_str().unwrap_or("")
    }

    /// The highest version the session serves.
    pub(crate) fn version(&self) -> u32 {
        self.version
    }
}

/// `WlcsIntegrationDescriptor`.
#[repr(C)]
pub struct WlcsIntegrationDescriptor {
    version: u32,
    num_extensions: usize,
    supported_extensions: *const WlcsExtensionDescriptor,
}

// SAFETY: the table it points at is `'static` and read-only.
unsafe impl Sync for WlcsIntegrationDescriptor {}

/// Declares the protocols the session serves, each at the highest version it advertises.
///
/// The table is what wlcs reads to decide which tests to skip, so it lists exactly what a client's registry shows
/// and the crate's `descriptor` test fails when the two differ.
macro_rules! extensions {
    ($(($name:literal, $version:literal)),* $(,)?) => {
        static EXTENSIONS: [WlcsExtensionDescriptor; [$($name),*].len()] =
            [$(WlcsExtensionDescriptor::new($name, $version)),*];
    };
}

extensions![
    (c"ext_background_effect_manager_v1", 1),
    (c"ext_idle_notifier_v1", 2),
    (c"ext_session_lock_manager_v1", 1),
    (c"wl_compositor", 5),
    (c"wl_data_device_manager", 3),
    (c"wl_output", 4),
    (c"wl_seat", 9),
    (c"wl_shm", 2),
    (c"wl_subcompositor", 1),
    (c"wp_alpha_modifier_v1", 1),
    (c"wp_content_type_manager_v1", 1),
    (c"wp_cursor_shape_manager_v1", 2),
    (c"wp_fifo_manager_v1", 1),
    (c"wp_fractional_scale_manager_v1", 1),
    (c"wp_presentation", 2),
    (c"wp_security_context_manager_v1", 1),
    (c"wp_single_pixel_buffer_manager_v1", 1),
    (c"wp_tearing_control_manager_v1", 1),
    (c"wp_viewporter", 1),
    (c"xdg_activation_v1", 1),
    (c"xdg_system_bell_v1", 1),
    (c"xdg_toplevel_icon_manager_v1", 1),
    (c"xdg_toplevel_tag_manager_v1", 1),
    (c"xdg_wm_base", 7),
    (c"xdg_wm_dialog_v1", 1),
    (c"zwp_idle_inhibit_manager_v1", 1),
    (c"zwp_input_method_manager_v2", 1),
    (c"zwp_keyboard_shortcuts_inhibit_manager_v1", 1),
    (c"zwp_pointer_constraints_v1", 1),
    (c"zwp_pointer_gestures_v1", 3),
    (c"zwp_primary_selection_device_manager_v1", 1),
    (c"zwp_relative_pointer_manager_v1", 1),
    (c"zwp_text_input_manager_v3", 1),
    (c"zwp_virtual_keyboard_manager_v1", 1),
    (c"zxdg_decoration_manager_v1", 1),
    (c"zxdg_exporter_v2", 1),
    (c"zxdg_importer_v2", 1),
    (c"zxdg_output_manager_v1", 3),
];

static DESCRIPTOR: WlcsIntegrationDescriptor = WlcsIntegrationDescriptor {
    version: WLCS_INTEGRATION_DESCRIPTOR_VERSION,
    num_extensions: EXTENSIONS.len(),
    supported_extensions: EXTENSIONS.as_ptr(),
};

/// `WlcsPointer`.
#[repr(C)]
pub struct WlcsPointer {
    version: u32,
    move_absolute: unsafe extern "C" fn(*mut WlcsPointer, WlFixed, WlFixed),
    move_relative: unsafe extern "C" fn(*mut WlcsPointer, WlFixed, WlFixed),
    button_up: unsafe extern "C" fn(*mut WlcsPointer, c_int),
    button_down: unsafe extern "C" fn(*mut WlcsPointer, c_int),
    destroy: unsafe extern "C" fn(*mut WlcsPointer),
}

/// A pointer device: the table wlcs calls and the injector behind it.
#[repr(C)]
struct Pointer {
    table: WlcsPointer,
    injector: Injector,
}

/// `WlcsTouch`.
#[repr(C)]
pub struct WlcsTouch {
    version: u32,
    touch_down: unsafe extern "C" fn(*mut WlcsTouch, WlFixed, WlFixed),
    touch_move: unsafe extern "C" fn(*mut WlcsTouch, WlFixed, WlFixed),
    touch_up: unsafe extern "C" fn(*mut WlcsTouch),
    destroy: unsafe extern "C" fn(*mut WlcsTouch),
}

/// A touch device: the table wlcs calls, its injector and the slot its one point uses.
///
/// `x` and `y` remember where the point was last, because a lift reports the position it was last at.
#[repr(C)]
struct Touch {
    table: WlcsTouch,
    injector: Injector,
    slot: u32,
    x: i32,
    y: i32,
}

/// `WlcsKeyboard`.
#[repr(C)]
pub struct WlcsKeyboard {
    version: u32,
    key_down: unsafe extern "C" fn(*mut WlcsKeyboard, c_int),
    key_up: unsafe extern "C" fn(*mut WlcsKeyboard, c_int),
    destroy: unsafe extern "C" fn(*mut WlcsKeyboard),
}

/// A keyboard device: the table wlcs calls and the injector behind it.
#[repr(C)]
struct Keyboard {
    table: WlcsKeyboard,
    injector: Injector,
}

/// `WlcsDisplayServer`, version 4.
#[repr(C)]
pub struct WlcsDisplayServer {
    version: u32,
    start: unsafe extern "C" fn(*mut WlcsDisplayServer),
    stop: unsafe extern "C" fn(*mut WlcsDisplayServer),
    create_client_socket: unsafe extern "C" fn(*mut WlcsDisplayServer) -> c_int,
    position_window_absolute: unsafe extern "C" fn(*mut WlcsDisplayServer, *mut c_void, *mut c_void, c_int, c_int),
    create_pointer: unsafe extern "C" fn(*mut WlcsDisplayServer) -> *mut WlcsPointer,
    create_touch: unsafe extern "C" fn(*mut WlcsDisplayServer) -> *mut WlcsTouch,
    get_descriptor: unsafe extern "C" fn(*const WlcsDisplayServer) -> *const WlcsIntegrationDescriptor,
    /// Optional; this integration provides `start` instead.
    start_on_this_thread: Option<unsafe extern "C" fn(*mut WlcsDisplayServer, *mut c_void)>,
    create_keyboard: unsafe extern "C" fn(*mut WlcsDisplayServer) -> *mut WlcsKeyboard,
}

/// The compositor behind the table. The table is first, so wlcs's pointer is also this object's.
#[repr(C)]
struct DisplayServer {
    table: WlcsDisplayServer,
    harness: OnceLock<ConformanceSession>,
}

/// `WlcsServerIntegration`.
#[repr(C)]
pub struct WlcsServerIntegration {
    version: u32,
    create_server: unsafe extern "C" fn(c_int, *mut *const c_char) -> *mut WlcsDisplayServer,
    destroy_server: unsafe extern "C" fn(*mut WlcsDisplayServer),
}

// SAFETY: the struct holds a version and two function pointers, all immutable.
unsafe impl Sync for WlcsServerIntegration {}

/// The symbol wlcs looks up.
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static wlcs_server_integration: WlcsServerIntegration = WlcsServerIntegration {
    version: WLCS_SERVER_INTEGRATION_VERSION,
    create_server,
    destroy_server,
};

/// The compositor of a display server wlcs called, or a refusal wlcs cannot recover from.
///
/// # Safety
///
/// `server` must be a pointer [`create_server`] returned and [`destroy_server`] has not freed.
unsafe fn harness<'a>(server: *mut WlcsDisplayServer) -> &'a ConformanceSession {
    // SAFETY: the caller's contract; the table is the object's first field.
    let object = unsafe { &*server.cast::<DisplayServer>() };
    object.harness.get().unwrap_or_else(|| {
        eprintln!("gnoblin-wlcs: the display server was used before `start`");
        process::abort();
    })
}

unsafe extern "C" fn create_server(_argc: c_int, _argv: *mut *const c_char) -> *mut WlcsDisplayServer {
    let object = Box::new(DisplayServer {
        table: WlcsDisplayServer {
            version: WLCS_DISPLAY_SERVER_VERSION,
            start,
            stop,
            create_client_socket,
            position_window_absolute,
            create_pointer,
            create_touch,
            get_descriptor,
            start_on_this_thread: None,
            create_keyboard,
        },
        harness: OnceLock::new(),
    });
    Box::into_raw(object).cast()
}

unsafe extern "C" fn destroy_server(server: *mut WlcsDisplayServer) {
    if server.is_null() {
        return;
    }
    // SAFETY: `create_server` made it with `Box::new` and wlcs calls this once; dropping the harness stops the
    // session if `stop` did not.
    drop(unsafe { Box::from_raw(server.cast::<DisplayServer>()) });
}

unsafe extern "C" fn start(server: *mut WlcsDisplayServer) {
    // SAFETY: `create_server`'s pointer, not yet freed.
    let object = unsafe { &*server.cast::<DisplayServer>() };
    match ConformanceSession::start() {
        Ok(started) => {
            let _ = object.harness.set(started);
        }
        Err(reason) => {
            eprintln!("gnoblin-wlcs: {reason}");
            process::abort();
        }
    }
}

unsafe extern "C" fn stop(server: *mut WlcsDisplayServer) {
    // SAFETY: `create_server`'s pointer, not yet freed.
    unsafe { harness(server) }.stop();
}

unsafe extern "C" fn create_client_socket(server: *mut WlcsDisplayServer) -> c_int {
    // SAFETY: `create_server`'s pointer, not yet freed.
    match unsafe { harness(server) }.client_socket() {
        Ok(fd) => fd,
        Err(error) => {
            eprintln!("gnoblin-wlcs: no client socket: {error}");
            -1
        }
    }
}

unsafe extern "C" fn position_window_absolute(
    server: *mut WlcsDisplayServer,
    client: *mut c_void,
    surface: *mut c_void,
    x: c_int,
    y: c_int,
) {
    // SAFETY: `client` is the client's `wl_display` and `surface` its `wl_surface`, both live for the call.
    let (fd, id): (RawFd, u32) = unsafe { (wl_display_get_fd(client), wl_proxy_get_id(surface)) };
    // SAFETY: `create_server`'s pointer, not yet freed.
    if !unsafe { harness(server) }.position_window(fd, id, x, y) {
        eprintln!("gnoblin-wlcs: no window to place for surface {id} on connection {fd}");
    }
}

unsafe extern "C" fn create_pointer(server: *mut WlcsDisplayServer) -> *mut WlcsPointer {
    // SAFETY: `create_server`'s pointer, not yet freed.
    let injector = unsafe { harness(server) }.injector();
    Box::into_raw(Box::new(Pointer {
        table: WlcsPointer {
            version: WLCS_POINTER_VERSION,
            move_absolute: pointer_move_absolute,
            move_relative: pointer_move_relative,
            button_up: pointer_button_up,
            button_down: pointer_button_down,
            destroy: pointer_destroy,
        },
        injector,
    }))
    .cast()
}

/// The device a pointer table belongs to.
///
/// # Safety
///
/// `table` must be a pointer [`create_pointer`] returned and [`pointer_destroy`] has not freed.
unsafe fn pointer<'a>(table: *mut WlcsPointer) -> &'a Pointer {
    // SAFETY: the caller's contract; the table is the device's first field.
    unsafe { &*table.cast::<Pointer>() }
}

unsafe extern "C" fn pointer_move_absolute(table: *mut WlcsPointer, x: WlFixed, y: WlFixed) {
    // SAFETY: wlcs passes the pointer it was given.
    unsafe { pointer(table) }
        .injector
        .pointer_move_to(fixed_to_pixels(x), fixed_to_pixels(y));
}

unsafe extern "C" fn pointer_move_relative(table: *mut WlcsPointer, dx: WlFixed, dy: WlFixed) {
    // SAFETY: wlcs passes the pointer it was given.
    unsafe { pointer(table) }
        .injector
        .pointer_move_by(fixed_to_pixels(dx), fixed_to_pixels(dy));
}

unsafe extern "C" fn pointer_button_up(table: *mut WlcsPointer, button: c_int) {
    // SAFETY: wlcs passes the pointer it was given.
    unsafe { pointer(table) }
        .injector
        .pointer_button(button.unsigned_abs(), false);
}

unsafe extern "C" fn pointer_button_down(table: *mut WlcsPointer, button: c_int) {
    // SAFETY: wlcs passes the pointer it was given.
    unsafe { pointer(table) }
        .injector
        .pointer_button(button.unsigned_abs(), true);
}

unsafe extern "C" fn pointer_destroy(table: *mut WlcsPointer) {
    // SAFETY: `create_pointer` made it with `Box::new` and wlcs destroys it once.
    drop(unsafe { Box::from_raw(table.cast::<Pointer>()) });
}

unsafe extern "C" fn create_touch(server: *mut WlcsDisplayServer) -> *mut WlcsTouch {
    // SAFETY: `create_server`'s pointer, not yet freed.
    let harness = unsafe { harness(server) };
    Box::into_raw(Box::new(Touch {
        table: WlcsTouch {
            version: WLCS_TOUCH_VERSION,
            touch_down,
            touch_move,
            touch_up,
            destroy: touch_destroy,
        },
        injector: harness.injector(),
        slot: harness.next_touch_slot(),
        x: 0,
        y: 0,
    }))
    .cast()
}

/// The device a touch table belongs to.
///
/// # Safety
///
/// `table` must be a pointer [`create_touch`] returned and [`touch_destroy`] has not freed, and no other reference
/// to the device may be live: wlcs calls one device from one thread at a time.
unsafe fn touch<'a>(table: *mut WlcsTouch) -> &'a mut Touch {
    // SAFETY: the caller's contract; the table is the device's first field.
    unsafe { &mut *table.cast::<Touch>() }
}

unsafe extern "C" fn touch_down(table: *mut WlcsTouch, x: WlFixed, y: WlFixed) {
    // SAFETY: wlcs passes the pointer it was given.
    let device = unsafe { touch(table) };
    (device.x, device.y) = (touch_pixels(x), touch_pixels(y));
    device.injector.touch(device.slot, device.x, device.y, TouchPhase::Down);
}

unsafe extern "C" fn touch_move(table: *mut WlcsTouch, x: WlFixed, y: WlFixed) {
    // SAFETY: wlcs passes the pointer it was given.
    let device = unsafe { touch(table) };
    (device.x, device.y) = (touch_pixels(x), touch_pixels(y));
    device
        .injector
        .touch(device.slot, device.x, device.y, TouchPhase::Motion);
}

unsafe extern "C" fn touch_up(table: *mut WlcsTouch) {
    // SAFETY: wlcs passes the pointer it was given.
    let device = unsafe { touch(table) };
    device.injector.touch(device.slot, device.x, device.y, TouchPhase::Up);
}

unsafe extern "C" fn touch_destroy(table: *mut WlcsTouch) {
    // SAFETY: `create_touch` made it with `Box::new` and wlcs destroys it once.
    drop(unsafe { Box::from_raw(table.cast::<Touch>()) });
}

/// A touch coordinate, in whole pixels.
///
/// wlcs's `Touch::down_at(int, int)` passes its integers straight to a parameter the header types `wl_fixed_t`
/// (the conversion `wlcs::Pointer` applies is missing for touch), so the value that arrives is already whole pixels.
fn touch_pixels(value: WlFixed) -> i32 {
    value
}

unsafe extern "C" fn create_keyboard(server: *mut WlcsDisplayServer) -> *mut WlcsKeyboard {
    // SAFETY: `create_server`'s pointer, not yet freed.
    let injector = unsafe { harness(server) }.injector();
    Box::into_raw(Box::new(Keyboard {
        table: WlcsKeyboard {
            version: WLCS_KEYBOARD_VERSION,
            key_down,
            key_up,
            destroy: keyboard_destroy,
        },
        injector,
    }))
    .cast()
}

unsafe extern "C" fn key_down(table: *mut WlcsKeyboard, scancode: c_int) {
    // SAFETY: wlcs passes the pointer it was given; the table is the device's first field.
    unsafe { &*table.cast::<Keyboard>() }
        .injector
        .key(scancode.unsigned_abs(), true);
}

unsafe extern "C" fn key_up(table: *mut WlcsKeyboard, scancode: c_int) {
    // SAFETY: wlcs passes the pointer it was given; the table is the device's first field.
    unsafe { &*table.cast::<Keyboard>() }
        .injector
        .key(scancode.unsigned_abs(), false);
}

unsafe extern "C" fn keyboard_destroy(table: *mut WlcsKeyboard) {
    // SAFETY: `create_keyboard` made it with `Box::new` and wlcs destroys it once.
    drop(unsafe { Box::from_raw(table.cast::<Keyboard>()) });
}

unsafe extern "C" fn get_descriptor(_server: *const WlcsDisplayServer) -> *const WlcsIntegrationDescriptor {
    ptr::addr_of!(DESCRIPTOR)
}

/// The protocols the descriptor lists, as `(name, version)`, for the crate's own tests.
#[must_use]
pub fn extensions() -> Vec<(&'static str, u32)> {
    EXTENSIONS.iter().map(|entry| (entry.name(), entry.version())).collect()
}
