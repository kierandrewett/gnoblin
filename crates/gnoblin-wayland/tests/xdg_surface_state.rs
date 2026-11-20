//! The rules about an `xdg_surface` that Smithay raises late or not at all, against a real client.
//!
//! `xdg_surface.unconfigured_buffer` is the error for attaching a buffer to a surface whose first configure has not
//! been acknowledged. Smithay raises it at commit, so a client that attaches and does not commit was never told,
//! and the wlcs `XdgSurfaceStableTest.attaching_buffer_to_unconfigured_xdg_surface_is_an_error` expects it at the
//! attach. The cases here are the same rule from a client that does not use the wlcs harness, plus the cases it must
//! leave alone: an acknowledged surface, a surface whose `xdg_surface` is gone, and the null attach that unmaps.

use gnoblin_wayland_testkit::session;

use session::{Client, Session};
use wayland_client::backend::WaylandError;
use wayland_client::{DispatchError, Proxy};
use wayland_protocols::xdg::shell::client::xdg_surface;

/// Round-trips and returns the protocol error the server closed the connection with, as its object's interface and
/// code, or `None` while the connection is open.
fn protocol_error(client: &mut Client) -> Option<(String, u32)> {
    match client.queue.roundtrip(&mut client.state) {
        Ok(_) => None,
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
            Some((error.object_interface.to_string(), error.code))
        }
        Err(other) => panic!("the round trip failed for another reason: {other}"),
    }
}

#[test]
fn a_buffer_attached_to_an_xdg_surface_before_any_role_is_an_error_at_the_attach() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor.create_surface(&client.qh, ());
        let buffer = client.buffer(64, 64);
        client.roundtrip();

        let _xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        // No commit follows: Smithay would have said nothing.
        assert_eq!(
            protocol_error(&mut client),
            Some((
                xdg_surface::XdgSurface::interface().name.to_owned(),
                xdg_surface::Error::UnconfiguredBuffer as u32
            )),
            "the attach is answered with xdg_surface.unconfigured_buffer"
        );
    });
}

#[test]
fn a_buffer_attached_to_a_toplevel_before_its_first_configure_is_an_error_at_the_attach() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor.create_surface(&client.qh, ());
        let buffer = client.buffer(64, 64);
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let _toplevel = xdg_surface.get_toplevel(&client.qh, ());
        // The initial commit that asks for the configure has not been made, so nothing could have been acknowledged.
        surface.attach(Some(&buffer), 0, 0);
        assert_eq!(
            protocol_error(&mut client),
            Some((
                xdg_surface::XdgSurface::interface().name.to_owned(),
                xdg_surface::Error::UnconfiguredBuffer as u32
            )),
        );
    });
}

#[test]
fn a_buffer_attached_after_the_configure_is_acknowledged_maps_the_window() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        // The first commit asks for the configure and the client's handler acknowledges it while it round-trips.
        let window = client.create_window("org.gnoblin.Configured", "Configured");
        let buffer = client.buffer(64, 64);
        window.surface.attach(Some(&buffer), 0, 0);
        assert_eq!(
            protocol_error(&mut client),
            None,
            "an attach after the acknowledged configure is the protocol's order"
        );
        window.surface.damage(0, 0, 64, 64);
        window.surface.commit();
        client.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
    });
}

#[test]
fn a_null_attach_before_the_first_configure_is_not_an_error() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let _toplevel = xdg_surface.get_toplevel(&client.qh, ());
        // A client that unmaps before it was ever mapped, or one that commits "no buffer" as its first commit,
        // attaches nothing.
        surface.attach(None, 0, 0);
        surface.commit();
        assert_eq!(protocol_error(&mut client), None);
    });
}

#[test]
fn a_surface_whose_xdg_surface_is_gone_takes_a_buffer_again() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        let surface = client.compositor.create_surface(&client.qh, ());
        let buffer = client.buffer(64, 64);
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        xdg_surface.destroy();
        client.roundtrip();

        // The wl_surface never got a role, so it is free for any other use, and an attach is the client's business.
        surface.attach(Some(&buffer), 0, 0);
        assert_eq!(
            protocol_error(&mut client),
            None,
            "nothing is left to be unconfigured once the xdg_surface is destroyed"
        );
    });
}
