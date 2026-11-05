//! `zwlr_gamma_control_manager_v1` and `zwlr_output_power_manager_v1` against a real client.
//!
//! The scenarios are the failure modes listed at the top of
//! `src/protocols/zwlr_gamma_control_v1.rs` and `src/protocols/zwlr_output_power_v1.rs`.
//! The headless backend records what the host applied, so each test checks the
//! output's state through [`HeadlessHandle`](gnoblin_backend::headless::HeadlessHandle)
//! rather than trusting what the client was told.

mod owned;

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::HEADLESS_GAMMA_SIZE;
use gnoblin_types::OutputId;
use gnoblin_wayland::ClientRole;
use owned::{Connected, Handle, Session, TIMEOUT};
use wayland_client::backend::protocol::{Argument, Message};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
};
use wayland_protocols_wlr::output_power_management::v1::client::{
    zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1,
    zwlr_output_power_v1::{self, Mode, ZwlrOutputPowerV1},
};

/// The `invalid_gamma` error of `zwlr_gamma_control_v1`.
const INVALID_GAMMA: u32 = 1;
/// The `invalid_mode` error of `zwlr_output_power_v1`.
const INVALID_MODE: u32 = 1;
/// The request opcode of `set_mode` in `zwlr_output_power_v1`: its first request.
const SET_MODE: u16 = 0;

/// The first output the headless backend creates.
fn output() -> OutputId {
    OutputId::new(0)
}

/// What a client has heard from its gamma and power objects.
#[derive(Default)]
struct App {
    /// The table size the gamma control announced, if it was granted.
    gamma_size: Option<u32>,
    /// Whether a gamma control was refused or lost.
    gamma_failed: bool,
    /// Every mode the power object reported, in order.
    modes: Vec<Mode>,
    /// Whether a power object was refused or lost.
    power_failed: bool,
}

impl Dispatch<ZwlrGammaControlV1, ()> for App {
    fn event(
        state: &mut Self,
        _control: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => state.gamma_size = Some(size),
            zwlr_gamma_control_v1::Event::Failed => state.gamma_failed = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputPowerV1, ()> for App {
    fn event(
        state: &mut Self,
        _power: &ZwlrOutputPowerV1,
        event: zwlr_output_power_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_power_v1::Event::Mode { mode } => match mode {
                WEnum::Value(mode) => state.modes.push(mode),
                WEnum::Unknown(bits) => panic!("the server announced an unknown mode {bits}"),
            },
            zwlr_output_power_v1::Event::Failed => state.power_failed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
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

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(App: ignore ZwlrGammaControlManagerV1);
wayland_client::delegate_noop!(App: ignore ZwlrOutputPowerManagerV1);

/// Binds the `wl_output` the headless backend advertises.
fn bind_output(client: &Connected<App>) -> wl_output::WlOutput {
    client
        .globals
        .bind(&client.qh, 1..=4, ())
        .expect("the session advertises wl_output")
}

fn gamma_manager(client: &Connected<App>) -> ZwlrGammaControlManagerV1 {
    client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("a privileged client sees the gamma control manager")
}

fn power_manager(client: &Connected<App>) -> ZwlrOutputPowerManagerV1 {
    client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("a privileged client sees the output power manager")
}

/// A ramp descriptor holding three tables of `size` entries, each entry `value`.
fn ramp(size: usize, value: u16) -> File {
    let mut bytes = Vec::with_capacity(3 * size * 2);
    for _ in 0..3 * size {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    file_with(&bytes)
}

/// A readable descriptor holding exactly `bytes`, rewound to the start.
fn file_with(bytes: &[u8]) -> File {
    let mut file = tempfile::tempfile().expect("a temporary file for the ramp");
    file.write_all(bytes).expect("the ramp is written");
    file.seek(SeekFrom::Start(0)).expect("the ramp is rewound");
    file
}

/// Polls `predicate` until it holds, failing with `what` at the deadline.
fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !predicate() {
        assert!(Instant::now() < deadline, "the session never reached: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A client that holds gamma control of the first output, with its table size heard.
fn holding_gamma(handle: &Handle) -> (Connected<App>, ZwlrGammaControlV1) {
    let mut client = Connected::connect(handle.socket(), App::default());
    let output = bind_output(&client);
    let manager = gamma_manager(&client);
    let control = manager.get_gamma_control(&output, &client.qh, ());
    client.wait_until("the gamma table size", |state| state.gamma_size.is_some());
    (client, control)
}

#[test]
fn a_second_gamma_control_for_an_output_fails() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut first, held) = holding_gamma(handle);
        assert_eq!(first.state.gamma_size, Some(HEADLESS_GAMMA_SIZE));

        let mut second = Connected::connect(handle.socket(), App::default());
        let wl_out = bind_output(&second);
        let refused = gamma_manager(&second).get_gamma_control(&wl_out, &second.qh, ());
        second.wait_until("the refusal", |state| state.gamma_failed);
        assert_eq!(
            second.state.gamma_size, None,
            "the refused client is given no table size"
        );

        // The first holder keeps the output: its ramp lands, the refused client's does not.
        let size = HEADLESS_GAMMA_SIZE as usize;
        held.set_gamma(ramp(size, 1000).as_fd());
        first.roundtrip();
        wait_for("the first holder's ramp", || {
            handle.backend.applied_gamma(output()).is_some()
        });
        refused.set_gamma(ramp(size, 2000).as_fd());
        second.roundtrip();
        let applied = handle.backend.applied_gamma(output()).expect("the first holder's ramp");
        assert_eq!(
            applied[0][0], 1000,
            "the refused client's ramp never reached the output"
        );
    });
}

#[test]
fn releasing_gamma_control_restores_the_original_ramp() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, control) = holding_gamma(handle);
        control.set_gamma(ramp(HEADLESS_GAMMA_SIZE as usize, 1000).as_fd());
        client.roundtrip();
        wait_for("the loaded ramp", || handle.backend.applied_gamma(output()).is_some());

        control.destroy();
        client.roundtrip();
        wait_for("the original ramp", || handle.backend.applied_gamma(output()).is_none());
    });
}

#[test]
fn a_holder_disconnect_restores_the_original_ramp() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, control) = holding_gamma(handle);
        control.set_gamma(ramp(HEADLESS_GAMMA_SIZE as usize, 1000).as_fd());
        client.roundtrip();
        wait_for("the loaded ramp", || handle.backend.applied_gamma(output()).is_some());

        drop(control);
        drop(client);
        wait_for("the original ramp after the holder left", || {
            handle.backend.applied_gamma(output()).is_none()
        });

        // The output is free again: a new client can take it.
        let (_next, _next_control) = holding_gamma(handle);
    });
}

#[test]
fn a_ramp_of_the_wrong_size_is_a_protocol_error() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, control) = holding_gamma(handle);
        // Three tables plus one extra byte: longer than the output's tables allow.
        let bytes = vec![0u8; 3 * HEADLESS_GAMMA_SIZE as usize * 2 + 1];
        control.set_gamma(file_with(&bytes).as_fd());
        client.expect_protocol_error("zwlr_gamma_control_v1", INVALID_GAMMA);
        assert!(
            handle.backend.applied_gamma(output()).is_none(),
            "an oversized ramp must load nothing"
        );
    });
}

#[test]
fn a_short_ramp_fails_and_loads_nothing() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, control) = holding_gamma(handle);
        // Two bytes short of three tables.
        let bytes = vec![0u8; 3 * HEADLESS_GAMMA_SIZE as usize * 2 - 2];
        control.set_gamma(file_with(&bytes).as_fd());
        client.wait_until("the failed ramp", |state| state.gamma_failed);
        assert!(
            handle.backend.applied_gamma(output()).is_none(),
            "a short ramp must load nothing"
        );
    });
}

#[test]
fn an_unreadable_ramp_descriptor_fails() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, control) = holding_gamma(handle);
        // A descriptor opened for writing only: the server cannot read it.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let write_only = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(directory.path().join("write-only-ramp"))
            .expect("a write-only ramp file");
        control.set_gamma(write_only.as_fd());
        client.wait_until("the failed read", |state| state.gamma_failed);
        assert!(
            handle.backend.applied_gamma(output()).is_none(),
            "an unreadable ramp must load nothing"
        );
    });
}

#[test]
fn removing_an_output_fails_its_gamma_control() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, _control) = holding_gamma(handle);
        handle.backend.disconnect_output(output());
        client.wait_until("the gamma control's failure", |state| state.gamma_failed);
    });
}

#[test]
fn an_unsupported_backend_does_not_offer_gamma_control() {
    Session::start_without_output_controls().run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        assert!(
            !client.has("zwlr_gamma_control_manager_v1"),
            "a backend that cannot load a ramp must not offer gamma control"
        );
    });
}

/// A client that holds the power object of the first output, with its first mode heard.
fn holding_power(handle: &Handle) -> (Connected<App>, ZwlrOutputPowerV1) {
    let mut client = Connected::connect(handle.socket(), App::default());
    let wl_output = bind_output(&client);
    let manager = power_manager(&client);
    let object = manager.get_output_power(&wl_output, &client.qh, ());
    client.wait_until("the first power mode", |state| !state.modes.is_empty());
    (client, object)
}

#[test]
fn a_second_power_object_for_an_output_fails() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut first, _held) = holding_power(handle);
        assert_eq!(
            first.state.modes,
            vec![Mode::On],
            "a new power object reports the output on"
        );

        let mut second = Connected::connect(handle.socket(), App::default());
        let wl_output = bind_output(&second);
        let _refused = power_manager(&second).get_output_power(&wl_output, &second.qh, ());
        second.wait_until("the refusal", |state| state.power_failed);
        assert!(second.state.modes.is_empty(), "the refused client hears no mode");
        first.roundtrip();
    });
}

#[test]
fn a_mode_change_is_reported() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, object) = holding_power(handle);
        object.set_mode(Mode::Off);
        client.wait_until("the output switching off", |state| {
            state.modes.last() == Some(&Mode::Off)
        });
        wait_for("the backend to record the output off", || {
            !handle.backend.power(output())
        });

        object.set_mode(Mode::On);
        client.wait_until("the output switching back on", |state| {
            state.modes.last() == Some(&Mode::On)
        });
        wait_for("the backend to record the output on", || handle.backend.power(output()));
    });
}

#[test]
fn an_invalid_mode_is_a_protocol_error() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, object) = holding_power(handle);
        // `set_mode` takes a typed `Mode`, so a value outside off/on is sent raw.
        client
            .connection
            .backend()
            .send_request(
                Message {
                    sender_id: object.id(),
                    opcode: SET_MODE,
                    args: vec![Argument::Uint(7)].into(),
                },
                None,
                None,
            )
            .expect("the request reaches the server");
        client.expect_protocol_error("zwlr_output_power_v1", INVALID_MODE);
    });
}

#[test]
fn removing_an_output_fails_its_power_object() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let (mut client, _object) = holding_power(handle);
        handle.backend.disconnect_output(output());
        client.wait_until("the power object's failure", |state| state.power_failed);
    });
}

#[test]
fn an_unsupported_backend_does_not_offer_power_control() {
    Session::start_without_output_controls().run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        assert!(
            !client.has("zwlr_output_power_manager_v1"),
            "a backend that cannot switch an output must not offer power control"
        );
    });
}
