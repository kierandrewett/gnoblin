//! `zwlr_output_manager_v1` against a real client.
//!
//! The composition root publishes the headless outputs as heads when the server starts, so
//! a fresh manager announces them under the serial its first publish set.

mod owned;

use std::collections::HashMap;
use std::sync::Arc;

use gnoblin_wayland::ClientRole;
use owned::{Connected, Session};
use wayland_client::backend::{ObjectData, ObjectId};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};

/// What the manager has told one head.
#[derive(Default, Clone)]
struct Head {
    name: String,
    scale: f64,
}

/// What the manager has told the client.
#[derive(Default)]
struct App {
    /// The serial of the first `done`, once received.
    done_serial: Option<u32>,
    /// Whether the manager announced it is finished.
    finished: bool,
    /// Heads announced to the client.
    heads: u32,
    /// The head objects, in announcement order.
    handles: Vec<ZwlrOutputHeadV1>,
    /// What each head said about itself, keyed by object.
    head_state: HashMap<ObjectId, Head>,
    /// The verdict on the last configuration this client sent.
    outcome: Option<&'static str>,
}

impl App {
    /// The head state the client last heard for each head, in announcement order.
    fn current(&self) -> Vec<Head> {
        self.handles
            .iter()
            .filter_map(|handle| self.head_state.get(&handle.id()).cloned())
            .collect()
    }
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
            zwlr_output_manager_v1::Event::Head { head } => {
                state.heads += 1;
                state.handles.push(head);
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.done_serial.get_or_insert(serial);
            }
            zwlr_output_manager_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
    fn event_created_child(opcode: u16, qhandle: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        // Opcode 0 of the manager announces a head, which is a child object.
        assert_eq!(opcode, 0, "only the head event creates a child of the manager");
        qhandle.make_data::<ZwlrOutputHeadV1, ()>(())
    }
}

impl Dispatch<ZwlrOutputHeadV1, ()> for App {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = state.head_state.entry(head.id()).or_default();
        match event {
            zwlr_output_head_v1::Event::Name { name } => entry.name = name,
            zwlr_output_head_v1::Event::Scale { scale } => entry.scale = scale,
            _ => {}
        }
    }
    fn event_created_child(opcode: u16, qhandle: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        // Opcode 3 of a head announces a mode, which is a child object too.
        assert_eq!(opcode, 3, "only the mode event creates a child of a head");
        qhandle.make_data::<ZwlrOutputModeV1, ()>(())
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for App {
    fn event(
        state: &mut Self,
        _configuration: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.outcome = Some(match event {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            zwlr_output_configuration_v1::Event::Cancelled => "cancelled",
            _ => return,
        });
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
wayland_client::delegate_noop!(App: ignore ZwlrOutputModeV1);
wayland_client::delegate_noop!(App: ignore ZwlrOutputConfigurationHeadV1);

/// Binds the manager and waits for the first `done`, so the heads are all known.
fn bind_manager(client: &mut Connected<App>) -> ZwlrOutputManagerV1 {
    let manager: ZwlrOutputManagerV1 = client
        .globals
        .bind(&client.qh, 1..=4, ())
        .expect("a privileged client sees the output manager");
    client.wait_until("the first done", |state| state.done_serial.is_some());
    manager
}

#[test]
fn a_privileged_client_is_told_the_serial_when_it_binds_the_manager() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut client);
        assert!(client.state.done_serial.is_some());
        assert!(client.state.heads >= 1, "the headless outputs are published at start");
        assert!(!client.state.finished);
    });
}

#[test]
fn a_scale_applied_by_one_client_is_seen_by_the_next() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let serial = client.state.done_serial.expect("bound");
        let configuration = manager.create_configuration(serial, &client.qh, ());
        let head = client.state.handles[0].clone();
        let configuration_head = configuration.enable_head(&head, &client.qh, ());
        configuration_head.set_scale(2.0);
        configuration.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));

        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        let heads = second.state.current();
        assert!(
            heads.iter().any(|head| (head.scale - 2.0).abs() < f64::EPSILON),
            "the second client sees the new scale, got {:?}",
            heads.iter().map(|head| head.scale).collect::<Vec<_>>()
        );
    });
}

#[test]
fn a_configuration_on_a_stale_serial_is_cancelled() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let stale = client.state.done_serial.expect("bound").wrapping_sub(1);
        let configuration = manager.create_configuration(stale, &client.qh, ());
        let head = client.state.handles[0].clone();
        configuration.enable_head(&head, &client.qh, ()).set_scale(2.0);
        configuration.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("cancelled"));
    });
}

#[test]
fn a_test_only_configuration_changes_nothing() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let serial = client.state.done_serial.expect("bound");
        let before = client.state.current().iter().map(|head| head.scale).collect::<Vec<_>>();
        let configuration = manager.create_configuration(serial, &client.qh, ());
        let head = client.state.handles[0].clone();
        configuration.enable_head(&head, &client.qh, ()).set_scale(2.0);
        configuration.test();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));

        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        let after = second.state.current().iter().map(|head| head.scale).collect::<Vec<_>>();
        assert_eq!(before, after, "a test-only configuration leaves the scale alone");
    });
}

#[test]
fn a_configuration_that_disables_every_output_is_failed() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let serial = client.state.done_serial.expect("bound");
        let configuration = manager.create_configuration(serial, &client.qh, ());
        for head in client.state.handles.clone() {
            configuration.disable_head(&head);
        }
        configuration.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("failed"));
    });
}
