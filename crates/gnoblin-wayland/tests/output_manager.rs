//! `zwlr_output_manager_v1` against a real client.
//!
//! The composition root publishes the headless outputs as heads when the server starts, so
//! a fresh manager announces them under the serial its first publish set.

mod owned;

use std::collections::HashMap;
use std::sync::Arc;

use gnoblin_wayland::{ClientRole, MonitorSettings};
use owned::{Connected, Session};
use wayland_client::backend::{ObjectData, ObjectId};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

/// What the manager has told one head.
#[derive(Default, Clone)]
struct Head {
    name: String,
    scale: f64,
    /// The mode object the head said it is driven in.
    current_mode: Option<ObjectId>,
}

/// What the manager has told the client.
#[derive(Default)]
struct App {
    /// The serial of the first `done`, once received.
    done_serial: Option<u32>,
    /// The serial of the latest `done`: the one a configuration built now is valid for.
    latest_serial: Option<u32>,
    /// The size each announced mode said it has, keyed by object.
    mode_sizes: HashMap<ObjectId, (i32, i32)>,
    /// The scale the output last told the client it composes at.
    wl_output_scale: Option<i32>,
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

    /// The size of the mode the first head is driven in, as the client last heard it.
    fn size(&self) -> Option<(i32, i32)> {
        let head = self.current().into_iter().next()?;
        self.mode_sizes.get(&head.current_mode?).copied()
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
                state.latest_serial = Some(serial);
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
        if let zwlr_output_head_v1::Event::Finished = event {
            // The manager retires a head object by finishing it; a later announcement brings a new one.
            state.handles.retain(|known| known != head);
            state.head_state.remove(&head.id());
            return;
        }
        let entry = state.head_state.entry(head.id()).or_default();
        match event {
            zwlr_output_head_v1::Event::Name { name } => entry.name = name,
            zwlr_output_head_v1::Event::Scale { scale } => entry.scale = scale,
            zwlr_output_head_v1::Event::CurrentMode { mode } => entry.current_mode = Some(mode.id()),
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
impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            state.wl_output_scale = Some(factor);
        }
    }
}
impl Dispatch<ZwlrOutputModeV1, ()> for App {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_output_mode_v1::Event::Size { width, height } = event {
            state.mode_sizes.insert(mode.id(), (width, height));
        }
    }
}
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

#[test]
fn a_global_scale_is_the_scale_a_new_output_starts_at() {
    let monitors = MonitorSettings {
        privacy_screen: None,
        global_scale: Some(2),
    };
    Session::start_with_monitors(Some(ClientRole::Privileged), monitors).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut client);
        let heads = client.state.current();
        assert!(!heads.is_empty(), "the headless outputs are published at start");
        assert!(
            heads.iter().all(|head| (head.scale - 2.0).abs() < f64::EPSILON),
            "every output starts at the global scale, got {:?}",
            heads.iter().map(|head| head.scale).collect::<Vec<_>>()
        );
    });
}

#[test]
fn an_explicit_scale_from_the_output_manager_wins_over_the_global_scale() {
    let monitors = MonitorSettings {
        privacy_screen: None,
        global_scale: Some(2),
    };
    Session::start_with_monitors(Some(ClientRole::Privileged), monitors).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let serial = client.state.done_serial.expect("bound");
        let configuration = manager.create_configuration(serial, &client.qh, ());
        let head = client.state.handles[0].clone();
        configuration.enable_head(&head, &client.qh, ()).set_scale(3.0);
        configuration.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));

        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        let heads = second.state.current();
        assert!(
            heads.iter().any(|head| (head.scale - 3.0).abs() < f64::EPSILON),
            "the explicit scale is kept, got {:?}",
            heads.iter().map(|head| head.scale).collect::<Vec<_>>()
        );
    });
}

#[test]
fn a_global_scale_committed_after_start_reaches_the_published_heads() {
    // The runtime commits its document after the compositor started, so the first boot has
    // no saved global scale. A commit must still reach the outputs and the heads a client
    // reads, not wait for the next session start.
    Session::start_with_monitors(Some(ClientRole::Privileged), MonitorSettings::default()).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut client);
        let _output: wl_output::WlOutput = client
            .globals
            .bind(&client.qh, 1..=4, ())
            .expect("a client sees the wl_output global");
        client.wait_until("the wl_output scale at start", |state| state.wl_output_scale == Some(1));
        assert!(
            client
                .state
                .current()
                .iter()
                .all(|head| (head.scale - 1.0).abs() < f64::EPSILON)
        );

        handle.control.set_global_scale(Some(2));
        client.wait_until("the committed scale", |state| {
            let heads = state.current();
            !heads.is_empty() && heads.iter().all(|head| (head.scale - 2.0).abs() < f64::EPSILON)
        });
        client.wait_until("the committed wl_output scale", |state| {
            state.wl_output_scale == Some(2)
        });

        handle.control.set_global_scale(None);
        client.wait_until("the scale the system setting gives", |state| {
            let heads = state.current();
            !heads.is_empty() && heads.iter().all(|head| (head.scale - 1.0).abs() < f64::EPSILON)
        });
        client.wait_until("the wl_output scale the system setting gives", |state| {
            state.wl_output_scale == Some(1)
        });
    });
}

/// A configuration for the first head that asks for a custom mode, built on the serial the client heard last.
fn mode_configuration(
    client: &Connected<App>,
    manager: &ZwlrOutputManagerV1,
    width: i32,
    height: i32,
) -> ZwlrOutputConfigurationV1 {
    let serial = client.state.latest_serial.expect("bound");
    let configuration = manager.create_configuration(serial, &client.qh, ());
    let head = client.state.handles[0].clone();
    configuration
        .enable_head(&head, &client.qh, ())
        .set_custom_mode(width, height, 60_000);
    configuration
}

#[test]
fn a_configuration_with_a_new_mode_is_applied_and_published_to_every_client() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        assert_eq!(
            client.state.size(),
            Some((1024, 768)),
            "the headless output starts at its default"
        );
        mode_configuration(&client, &manager, 800, 600).apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));
        client.wait_until("the new mode", |state| state.size() == Some((800, 600)));

        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        assert_eq!(
            second.state.size(),
            Some((800, 600)),
            "the next client reads the mode that is driven"
        );
        assert!(second.state.size().is_some());
    });
}

#[test]
fn a_test_only_mode_configuration_asks_the_backend_and_drives_nothing() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        mode_configuration(&client, &manager, 800, 600).test();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(
            client.state.outcome,
            Some("succeeded"),
            "the backend would take the mode"
        );
        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        assert_eq!(second.state.size(), Some((1024, 768)), "a test changes nothing");
    });
}

#[test]
fn a_mode_the_backend_refuses_is_failed_and_leaves_the_output_as_it_was() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        // The driver refuses the next modeset, as one that cannot drive the timing does.
        handle.backend.refuse_configure_after(0);
        mode_configuration(&client, &manager, 1280, 720).apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("failed"));

        let mut second = Connected::connect(handle.socket(), App::default());
        let _manager = bind_manager(&mut second);
        assert_eq!(second.state.size(), Some((1024, 768)), "the refused mode was not kept");
        client.wait_until("the heads the loop republished or kept", |state| state.size().is_some());

        // The refusal is not remembered: the same configuration, asked again, is applied.
        client.state.outcome = None;
        let serial_now = client.state.latest_serial;
        let retry = mode_configuration(&client, &manager, 1280, 720);
        assert_eq!(client.state.latest_serial, serial_now);
        retry.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));
    });
}

#[test]
fn a_configuration_built_before_the_layout_changed_is_cancelled() {
    Session::start(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        let first_serial = client.state.latest_serial.expect("bound");
        // Two configurations on the same layout: the first is applied, the second waits in the client.
        let waiting = mode_configuration(&client, &manager, 640, 480);
        mode_configuration(&client, &manager, 800, 600).apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("succeeded"));
        client.wait_until("the layout's new serial", |state| {
            state.latest_serial.is_some_and(|serial| serial != first_serial)
        });

        client.state.outcome = None;
        waiting.apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(
            client.state.outcome,
            Some("cancelled"),
            "a configuration built on the old layout is not applied over the new one"
        );
        assert_eq!(client.state.size(), Some((800, 600)));
    });
}

#[test]
fn a_backend_that_cannot_change_modes_fails_the_configuration() {
    Session::start_without_output_controls(Some(ClientRole::Privileged)).run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager = bind_manager(&mut client);
        mode_configuration(&client, &manager, 800, 600).apply();
        client.wait_until("the verdict", |state| state.outcome.is_some());
        assert_eq!(client.state.outcome, Some("failed"));
        assert_eq!(client.state.size(), Some((1024, 768)));
    });
}
