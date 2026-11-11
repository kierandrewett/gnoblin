//! `gnoblin_view_source_manager_v1`: the owner-only global that makes a View node
//! capturable, and the `view` role that reads it.
//!
//! The global is offered to the owner's privileged connection and to nothing else:
//! an application, a sandbox and a trusted service must not see it, because a capture
//! source reads a View's pixels. A request that names a node that is not a View is a
//! protocol error, not a silent no-op.
//!
//! The `view` role is the guardian-started extension: capture authority only. Its registry is
//! exactly the capture globals, and a global it was never told about is refused when it guesses
//! the name (`design/xr-extension.md`, G7).

mod owned;

use gnoblin_wayland::ClientRole;
use owned::protocols::gnoblin_view_source_v1::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use owned::protocols::gnoblin_view_source_v1::gnoblin_view_source_manager_v1::{self, GnoblinViewSourceManagerV1};
use owned::{Bare, Connected, Session};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1;
use wayland_protocols::xdg::shell::client::xdg_wm_base;

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
wayland_client::delegate_noop!(App: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(App: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

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

/// The whole registry of a `view` connection, by name. A global added to the compositor later
/// is hidden from the role until this list is changed on purpose, which is the point of
/// asserting the full set rather than the absence of a few names.
const VIEW_REGISTRY: [&str; 3] = [
    "ext_image_copy_capture_manager_v1",
    "gnoblin_view_source_manager_v1",
    "wl_shm",
];

/// The globals every ordinary client sees, whatever else it earned.
const CORE: [&str; 7] = [
    "wl_compositor",
    "wl_subcompositor",
    "wl_shm",
    "wl_output",
    "wl_seat",
    "xdg_wm_base",
    "ext_session_lock_manager_v1",
];

#[test]
fn a_view_role_client_sees_only_the_capture_globals() {
    let session = Session::start(None);
    session.run(|handle| {
        let client = handle.connect_bare(ClientRole::View, App);
        assert_eq!(
            client.interfaces(),
            VIEW_REGISTRY,
            "the view role's registry is exactly the capture globals"
        );
    });
}

#[test]
fn an_adopted_socket_is_a_view_client_whatever_process_holds_the_other_end() {
    // The guardian makes a socket pair, gives the extension one end and hands the compositor the
    // other. The pair's credentials name the guardian for both ends, so the role cannot come from
    // a promise by pid: it is the adopted connection's own. No promise is recorded here, and the
    // peer is this very process, yet the registry is the view role's.
    let session = Session::start(None);
    session.run(|handle| {
        let (server_end, client_end) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        handle.control.adopt_view(server_end);
        let client = Bare::from_stream(client_end, App);
        assert_eq!(
            client.interfaces(),
            VIEW_REGISTRY,
            "an adopted connection sees exactly the capture globals"
        );
        // An ordinary connection from the same process still gets the core globals.
        let ordinary = Bare::connect(handle.socket(), App);
        assert!(ordinary.interfaces().contains(&"wl_compositor".to_owned()));
    });
}

#[test]
fn a_view_role_client_cannot_bind_a_hidden_global_by_name() {
    let session = Session::start(None);
    session.run(|handle| {
        // The names a guessing client would send are the ones an ordinary connection was told.
        let probe = Bare::connect(handle.socket(), App);
        let (wm_base, wm_version) = probe
            .global_name("xdg_wm_base")
            .expect("an application sees xdg_wm_base");
        let (lock, lock_version) = probe
            .global_name("ext_session_lock_manager_v1")
            .expect("an application sees ext_session_lock_manager_v1");
        let (seat, seat_version) = probe.global_name("wl_seat").expect("an application sees wl_seat");

        // The guess is a valid bind: an ordinary connection that sends it is not refused.
        let mut ordinary = Bare::connect(handle.socket(), App);
        ordinary.bind_by_name::<xdg_wm_base::XdgWmBase>(wm_base, wm_version);
        ordinary.roundtrip();

        // A `view` connection that sends it is disconnected with a protocol error, one
        // connection per guess because the first error ends the connection.
        let mut viewer = handle.connect_bare(ClientRole::View, App);
        viewer.bind_by_name::<xdg_wm_base::XdgWmBase>(wm_base, wm_version);
        assert_eq!(viewer.protocol_error().0, "wl_display", "xdg_wm_base is refused");

        let mut viewer = handle.connect_bare(ClientRole::View, App);
        viewer.bind_by_name::<ext_session_lock_manager_v1::ExtSessionLockManagerV1>(lock, lock_version);
        assert_eq!(
            viewer.protocol_error().0,
            "wl_display",
            "ext_session_lock_manager_v1 is refused"
        );

        let mut viewer = handle.connect_bare(ClientRole::View, App);
        viewer.bind_by_name::<wl_seat::WlSeat>(seat, seat_version);
        assert_eq!(viewer.protocol_error().0, "wl_display", "wl_seat is refused");
    });
}

#[test]
fn the_other_roles_keep_the_core_globals() {
    // The view role is narrowed by name and no other role is: an application, a trusted
    // service and the owner each still see every core global.
    for role in [None, Some(ClientRole::Trusted), Some(ClientRole::Privileged)] {
        let session = Session::start(role);
        session.run(move |handle| {
            let client = Bare::connect(handle.socket(), App);
            for interface in CORE {
                assert!(client.has(interface), "{role:?} must see {interface}");
            }
        });
    }
}

#[test]
fn the_privileged_lists_differ_from_an_applications_by_exactly_the_privileged_globals() {
    // Three sessions, three roles, one comparison: nothing the narrowing added or removed is
    // visible to a role it does not apply to.
    let list = |role: Option<ClientRole>| {
        let (sender, receiver) = std::sync::mpsc::channel();
        Session::start(role).run(move |handle| {
            let client = Bare::connect(handle.socket(), App);
            sender.send(client.interfaces()).expect("the list");
        });
        receiver.recv().expect("a list of interfaces")
    };
    let application = list(None);
    let trusted = list(Some(ClientRole::Trusted));
    let owner = list(Some(ClientRole::Privileged));

    let extra = |larger: &[String], smaller: &[String]| -> Vec<String> {
        larger.iter().filter(|name| !smaller.contains(name)).cloned().collect()
    };
    assert!(
        extra(&application, &trusted).is_empty(),
        "a trusted service sees everything an application sees"
    );
    assert_eq!(
        extra(&owner, &trusted),
        [MANAGER],
        "the owner sees the view source manager and a trusted service does not"
    );
    assert!(
        extra(&trusted, &application).len() > 3,
        "a trusted service sees the privileged globals"
    );
    assert!(
        extra(&application, &owner).is_empty(),
        "the owner sees everything an application sees"
    );
}
