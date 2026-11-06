//! A sandboxed client is refused the background-effect manager by the access roster.
//!
//! The roster's promise is keyed by process, so this test owns its test binary: a
//! sandboxed promise made here must not reach a test that expects an unprivileged
//! client.

mod owned;

use std::os::unix::net::UnixStream;

use gnoblin_wayland::ClientRole;
use owned::Session;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

/// The registry's state: the globals the session advertised.
struct Probe;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
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

#[test]
fn a_sandboxed_client_is_not_offered_the_background_effect_manager() {
    let session = Session::start(Some(ClientRole::Sandboxed));
    session.run(|handle| {
        let stream = UnixStream::connect(handle.socket()).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, _queue) = registry_queue_init::<Probe>(&connection).expect("the global list");
        let mut interfaces = Vec::new();
        globals.contents().with_list(|list| {
            for global in list {
                interfaces.push(global.interface.clone());
            }
        });
        assert!(
            interfaces.iter().any(|name| name == "wl_compositor"),
            "the registry is live: the sandbox still gets the core globals; it saw {interfaces:?}"
        );
        assert!(
            !interfaces.iter().any(|name| name == "ext_background_effect_manager_v1"),
            "a sandboxed client must not see the blur manager; it saw {interfaces:?}"
        );
    });
}
