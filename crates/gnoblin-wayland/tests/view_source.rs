//! `gnoblin_view_source_manager_v1`: the owner-only global that makes a View node
//! capturable.
//!
//! The global is offered to the owner's privileged connection and to nothing else:
//! an application, a sandbox and a trusted service must not see it, because a capture
//! source reads a View's pixels. A request that names a node that is not a View is a
//! protocol error, not a silent no-op.

mod owned;

use gnoblin_wayland::ClientRole;
use owned::protocols::gnoblin_view_source_v1::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use owned::protocols::gnoblin_view_source_v1::gnoblin_view_source_manager_v1::{self, GnoblinViewSourceManagerV1};
use owned::{Connected, Session};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle};

const MANAGER: &str = "gnoblin_view_source_manager_v1";

/// What the client records. Nothing is needed beyond the globals and the requests.
#[derive(Default)]
struct App;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<GnoblinViewSourceManagerV1, ()> for App {
    fn event(
        _state: &mut Self,
        _manager: &GnoblinViewSourceManagerV1,
        _event: gnoblin_view_source_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(App: ignore ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

#[test]
fn the_owner_sees_the_view_source_global() {
    let session = Session::start(Some(ClientRole::Privileged));
    session.run(|handle| {
        let client = Connected::connect(handle.socket(), App);
        assert!(client.has(MANAGER), "the owner's connection advertises {MANAGER}");
    });
}

#[test]
fn an_ordinary_application_does_not_see_the_view_source_global() {
    let session = Session::start(None);
    session.run(|handle| {
        let client = Connected::connect(handle.socket(), App);
        assert!(!client.has(MANAGER), "an application must not read a View's pixels");
    });
}

#[test]
fn a_trusted_service_does_not_see_the_view_source_global() {
    let session = Session::start(Some(ClientRole::Trusted));
    session.run(|handle| {
        let client = Connected::connect(handle.socket(), App);
        assert!(
            !client.has(MANAGER),
            "a trusted service must not read another View's pixels"
        );
    });
}

#[test]
fn a_source_for_a_node_that_is_not_a_view_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::Privileged));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App);
        let manager: GnoblinViewSourceManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the owner sees the view source manager");
        // No View has this node id: the tree is empty, so the request must be refused.
        let _source = manager.create_source(9999, &client.qh, ());
        client.expect_protocol_error(MANAGER, 0);
    });
}
