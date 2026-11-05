//! `zwlr_output_manager_v1` against a real client.
//!
//! Only the bind path is exercised here: the composition root does not yet publish
//! heads to the manager, so a fresh manager announces an empty `done` at serial 0.

mod owned;

use gnoblin_wayland::ClientRole;
use owned::{Connected, Session};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1,
    zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};

/// What the manager has told the client.
#[derive(Default)]
struct App {
    /// The serial of the first `done`, once received.
    done_serial: Option<u32>,
    /// Whether the manager announced it is finished.
    finished: bool,
    /// Heads announced to the client.
    heads: u32,
}

impl Dispatch<ZwlrOutputManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _manager: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { .. } => state.heads += 1,
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.done_serial.get_or_insert(serial);
            }
            zwlr_output_manager_v1::Event::Finished => state.finished = true,
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
wayland_client::delegate_noop!(App: ignore ZwlrOutputHeadV1);
wayland_client::delegate_noop!(App: ignore ZwlrOutputModeV1);
wayland_client::delegate_noop!(App: ignore ZwlrOutputConfigurationV1);
wayland_client::delegate_noop!(App: ignore ZwlrOutputConfigurationHeadV1);

#[test]
fn a_privileged_client_is_told_the_serial_when_it_binds_the_manager() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let _manager: ZwlrOutputManagerV1 = client
            .globals
            .bind(&client.qh, 1..=4, ())
            .expect("a privileged client sees the output manager");
        client.wait_until("the first done", |state| state.done_serial.is_some());
        assert_eq!(client.state.done_serial, Some(0), "a fresh manager starts at serial 0");
        assert_eq!(
            client.state.heads, 0,
            "no heads are published before the host reports any"
        );
        assert!(!client.state.finished);
    });
}
