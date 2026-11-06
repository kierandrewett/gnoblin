//! The clipboard and the privileged data-control managers, against real clients.
//!
//! Two clients and one session: one copies, one pastes, and the bytes travel through a
//! pipe between them because that is the protocol — the compositor never holds the
//! data. The privileged managers are asserted from both sides of the socket the
//! connection arrived on, which is the only thing that decides their visibility.

mod session;

use std::io::Read;
use std::os::fd::AsFd;
use std::thread;
use std::time::{Duration, Instant};

use session::{Client, Session, TIMEOUT};
use wayland_client::protocol::{wl_data_device_manager, wl_data_source};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1;

/// Binds the clipboard, which every client sees.
fn data_device(client: &mut Client) -> wl_data_device_manager::WlDataDeviceManager {
    client
        .globals
        .bind(&client.qh, 1..=3, ())
        .expect("every session advertises wl_data_device_manager")
}

/// Puts `text/plain` on the clipboard from this client.
fn copy(client: &mut Client, manager: &wl_data_device_manager::WlDataDeviceManager) -> wl_data_source::WlDataSource {
    let device = manager.get_data_device(&client.seat, &client.qh, ());
    let source = manager.create_data_source(&client.qh, ());
    source.offer("text/plain".to_owned());
    // The serial is the one the client's own input carried; the clipboard only accepts
    // a selection from the client that has the keyboard.
    device.set_selection(Some(&source), client.state.last_serial);
    client.roundtrip();
    source
}

/// Creates the data device that receives the offer and sends the paste request.
///
/// A client without a `wl_data_device` object is never told about a selection, so the
/// destination has to ask for one before it can be offered anything.
fn paste_device(client: &mut Client) -> wayland_client::protocol::wl_data_device::WlDataDevice {
    // The manager must be the client's own: a protocol object belongs to the
    // connection that created it, and a device made through another connection's
    // manager is a foreign object the server cannot match to this client.
    let manager = data_device(client);
    let device = manager.get_data_device(&client.seat, &client.qh, ());
    client.roundtrip();
    device
}

/// Waits for this client to be offered a selection with `text/plain`.
fn wait_offer(client: &mut Client) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if client
            .state
            .selection
            .as_ref()
            .is_some_and(|mime_types| mime_types.iter().any(|mime| mime == "text/plain"))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no selection was offered: {:?}",
            client.state.selection
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_copy_between_two_clients_carries_the_source_bytes() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut copier = Client::connect(session.socket());
        let manager = data_device(&mut copier);
        let _window = copier.map_window("org.gnoblin.Copier", "Copier");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let source = copy(&mut copier, &manager);
        assert!(
            session.windows().selection().is_some(),
            "the session published the selection it is serving"
        );

        // The window that takes the focus is the one that may paste.
        let mut paster = Client::connect(session.socket());
        let _device = paste_device(&mut paster);
        let _other = paster.map_window("org.gnoblin.Paster", "Paster");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Paster"));
        wait_offer(&mut paster);

        let offer = paster
            .state
            .selection_offer
            .clone()
            .expect("the selection names an offer object");
        let (reader, writer) = std::io::pipe().expect("a pipe");
        offer.receive("text/plain".to_owned(), writer.as_fd());
        paster.roundtrip();

        // The source client answers the request, which is the whole data path.
        let deadline = Instant::now() + TIMEOUT;
        while copier.state.sends.is_empty() {
            copier.roundtrip();
            assert!(Instant::now() < deadline, "the source was never asked for the bytes");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(copier.state.sends, vec!["text/plain".to_owned()]);

        let mut bytes = Vec::new();
        drop(writer);
        reader.take(1024).read_to_end(&mut bytes).expect("the pasted bytes");
        assert_eq!(
            bytes,
            session::CLIPBOARD_BYTES,
            "the destination read the source's bytes"
        );

        drop(source);
    });
}

#[test]
fn a_data_control_manager_needs_a_privileged_client() {
    let session = Session::start();
    session.run(|session| {
        let application = Client::connect(session.socket());
        assert!(
            !application.has_global("ext_data_control_manager_v1"),
            "an application does not see ext_data_control_manager_v1"
        );
        assert!(
            !application.has_global("zwlr_data_control_manager_v1"),
            "an application does not see zwlr_data_control_manager_v1"
        );
        let refused: Result<ext_data_control_manager_v1::ExtDataControlManagerV1, _> =
            application.globals.bind(&application.qh, 1..=1, ());
        assert!(refused.is_err(), "and binding it is refused, not merely unanswered");

        let mut manager = Client::connect(session.privileged_socket());
        assert!(
            manager.has_global("ext_data_control_manager_v1"),
            "a privileged client sees the standard manager"
        );
        let standard: ext_data_control_manager_v1::ExtDataControlManagerV1 =
            manager.globals.bind(&manager.qh, 1..=1, ()).expect("and may bind it");
        let _device = standard.get_data_device(&manager.seat, &manager.qh, ());
        manager.roundtrip();

        let wlroots: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 = manager
            .globals
            .bind(&manager.qh, 1..=2, ())
            .expect("a privileged client sees the wlroots manager too");
        let _device = wlroots.get_data_device(&manager.seat, &manager.qh, ());
        manager.roundtrip();

        // The privileged manager reading the clipboard is the capability the role buys:
        // an ordinary application kept a working clipboard the whole time.
        assert!(
            application.has_global("wl_data_device_manager"),
            "the ordinary clipboard is not privileged"
        );
    });
}

#[test]
fn turning_data_control_off_hides_both_managers() {
    let mut protocols = gnoblin_config::Protocols::default();
    protocols
        .toggles
        .insert(gnoblin_config::Protocol::ExtDataControl, false);
    let session = Session::start_with(protocols);
    session.run(|session| {
        let privileged = Client::connect(session.privileged_socket());
        assert!(
            !privileged.has_global("ext_data_control_manager_v1"),
            "a disabled protocol is advertised to nobody"
        );
        assert!(
            !privileged.has_global("zwlr_data_control_manager_v1"),
            "the one configuration name covers both spellings"
        );
        assert!(
            privileged.has_global("wl_data_device_manager"),
            "the ordinary clipboard is not the same protocol"
        );
    });
}

#[test]
fn a_selection_does_not_outlive_its_owner() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut copier = Client::connect(session.socket());
        let manager = data_device(&mut copier);
        let _window = copier.map_window("org.gnoblin.Copier", "Copier");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));
        let _source = copy(&mut copier, &manager);

        let mut paster = Client::connect(session.socket());
        let _device = paste_device(&mut paster);
        let _other = paster.map_window("org.gnoblin.Paster", "Paster");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Paster"));
        wait_offer(&mut paster);
        assert!(session.windows().selection().is_some());

        // The source's connection dies without a word: no clear request, no destroy.
        drop(copier);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            paster.roundtrip();
            if paster.state.selection.is_none() && paster.state.selection_events >= 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the destination kept an offer whose owner is gone: {:?}",
                paster.state.selection
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}
