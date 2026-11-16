//! The clipboard and the primary selection between a nested session and its host, with a real client on each side.
//!
//! Three programs run in one process. The **host** is this repository's own compositor on the headless backend. The
//! **session** is a second compositor whose backend is a [`Nested`] connected to the host, so its window is a window of
//! the host. A real `wayland-client` connection sits on each: one on the host (a program of the person's desktop) and
//! one in the session (a program running in the nested desktop). Each test copies on one side and pastes on the other,
//! and the bytes travel the whole way: the pasting client's pipe, the session compositor, the nested backend's
//! transfer, the host compositor and the copying client.
//!
//! The rules under test are those of `gnoblin_backend::selection`: only while the window has the host's keyboard, only
//! the types that cross, never a secret, lazily, within a size limit (the limit itself is proven on pipes in that
//! module). The host here is a second implementation of the data-device protocols only in the sense that it is not the
//! nested backend; a different host (Sway, Weston) is run against the same scenarios by `scripts/real-host-conformance.sh`.

mod stock;

mod nested_host;
mod session;

use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_backend::{Backend, SelectionTarget};
use gnoblin_wayland::{Config, NewWindowFocus, Server, WmOptions};
use wayland_client::protocol::{wl_data_device_manager, wl_data_source};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1, zwp_primary_selection_offer_v1,
    zwp_primary_selection_source_v1,
};

use nested_host::{Probe, Session as HostSession};
use session::{App, CLIPBOARD_BYTES, Client, TIMEOUT};

// ---- a primary selection client --------------------------------------------------------------------------------------

impl Dispatch<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
        _: zwp_primary_selection_device_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                state.primary_offers.insert(offer.id().protocol_id(), Vec::new());
            }
            zwp_primary_selection_device_v1::Event::Selection { id } => {
                state.primary = id
                    .as_ref()
                    .and_then(|offer| state.primary_offers.get(&offer.id().protocol_id()).cloned());
                state.primary_offer = id;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE =>
            (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()),
    ]);
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()> for App {
    fn event(
        state: &mut Self,
        offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
            state
                .primary_offers
                .entry(offer.id().protocol_id())
                .or_default()
                .push(mime_type);
        }
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_source_v1::Event::Send { mime_type, fd } = event {
            state.primary_sends.push(mime_type);
            let mut file = std::fs::File::from(fd);
            let _ = std::io::Write::write_all(&mut file, CLIPBOARD_BYTES);
        }
    }
}

// ---- the rig ---------------------------------------------------------------------------------------------------------

/// What a test can reach of the session that runs in a window of the host.
struct Hosted {
    socket: PathBuf,
    observation: std::sync::Arc<std::sync::Mutex<gnoblin_wayland::SessionSnapshot>>,
}

/// Starts the session compositor with a nested backend on the host, on a thread of its own, and returns once it is up.
fn start_session(host: &nested_host::Handle) -> Hosted {
    let host_socket = host.socket().to_path_buf();
    let (up, ready) = mpsc::channel();
    thread::spawn(move || {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("session-0.sock");
        let nested = Nested::connect(&NestedConfig::new(&host_socket)).expect("the session connects to the host");
        let config = Config {
            socket_path: Some(socket.clone()),
            wm: WmOptions {
                new_window_focus: NewWindowFocus::Allow,
                ..WmOptions::default()
            },
            ..crate::stock::config()
        };
        let server = Server::new(Box::new(nested), config).expect("the session starts");
        up.send(Hosted {
            socket,
            observation: server.observation(),
        })
        .expect("the test is waiting");
        let _ = server.run();
        drop(directory);
    });
    ready.recv_timeout(TIMEOUT).expect("the session came up")
}

/// The host's window that is the session.
fn session_window(host: &nested_host::Handle) -> gnoblin_wayland::ObservedWindow {
    host.wait_for(|observation| {
        observation
            .windows()
            .any(|window| window.app_id == "org.gnoblin.Session" && window.mapped)
    })
    .windows()
    .find(|window| window.app_id == "org.gnoblin.Session")
    .expect("the session's window")
    .clone()
}

/// Gives the host's keyboard to `window`, and waits until the host says it has it.
fn focus_on_host(host: &nested_host::Handle, window: gnoblin_types::WindowId) {
    assert_eq!(host.control.focus(window), Some(true), "the host focuses the window");
    host.wait_for(|observation| observation.focused().is_some_and(|focused| focused.id == window));
}

/// Waits until the host's own compositor holds a `target` selection that offers `mime`.
///
/// A copy in the session reaches the host after the session's program has been told its request was handled: the
/// session's compositor hands it to the nested backend, which asks the host. The host gives a selection to no window
/// that does not have its keyboard, so a test that takes the keyboard from the session's window before the host holds
/// the copy is the person leaving before the copy landed, and the copy is lost. This is the event to wait for.
fn wait_host_holds(host: &nested_host::Handle, target: SelectionTarget, mime: &str) {
    host.wait_for(|observation| {
        let held = match target {
            SelectionTarget::Clipboard => observation.selection(),
            SelectionTarget::Primary => observation.primary_selection(),
        };
        held.is_some_and(|types| types.iter().any(|name| name == mime))
    });
}

/// The two ends of the test: a program on the host, and one in the session, each with a window.
struct Rig {
    host_client: Client,
    session_client: Client,
    host_window_id: gnoblin_types::WindowId,
    nested_window_id: gnoblin_types::WindowId,
    _host_window: session::Window,
    _session_window: session::Window,
    session: Hosted,
}

impl Rig {
    fn start(host: &nested_host::Handle) -> Self {
        let session = start_session(host);
        let nested_window_id = session_window(host).id;
        let mut host_client = Client::connect(host.socket());
        let host_window = host_client.map_window("org.gnoblin.HostProgram", "Host program");
        let host_window_id = host
            .wait_for(|observation| {
                observation
                    .windows()
                    .any(|window| window.title == "Host program" && window.mapped)
            })
            .windows()
            .find(|window| window.title == "Host program")
            .expect("the host program's window")
            .id;
        let mut session_client = Client::connect(&session.socket);
        let session_window = session_client.map_window("org.gnoblin.SessionProgram", "Session program");
        wait_session(&session, |observation| {
            observation
                .focused()
                .is_some_and(|window| window.title == "Session program")
        });
        Self {
            host_client,
            session_client,
            host_window_id,
            nested_window_id,
            _host_window: host_window,
            _session_window: session_window,
            session,
        }
    }
}

fn wait_session(session: &Hosted, predicate: impl Fn(&gnoblin_wayland::SessionSnapshot) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let observation = session.observation.lock().expect("the observation lock").clone();
        if predicate(&observation) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the session never reached the state: {observation:#?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

// ---- the clients' parts ----------------------------------------------------------------------------------------------

/// Makes each of `copies` (the types it offers) the clipboard from this client, in one burst of requests.
fn copy_burst(
    client: &mut Client,
    copies: &[&[&str]],
) -> (
    wl_data_device_manager::WlDataDeviceManager,
    Vec<wl_data_source::WlDataSource>,
) {
    let manager: wl_data_device_manager::WlDataDeviceManager = client
        .globals
        .bind(&client.qh, 1..=3, ())
        .expect("every session advertises wl_data_device_manager");
    let device = manager.get_data_device(&client.seat, &client.qh, ());
    let sources = copies
        .iter()
        .map(|types| {
            let source = manager.create_data_source(&client.qh, ());
            for mime in *types {
                source.offer((*mime).to_owned());
            }
            device.set_selection(Some(&source), client.state.last_serial);
            source
        })
        .collect();
    client.roundtrip();
    (manager, sources)
}

/// Offers `types` on the clipboard from this client.
fn copy_types(
    client: &mut Client,
    types: &[&str],
) -> (
    wl_data_device_manager::WlDataDeviceManager,
    wl_data_source::WlDataSource,
) {
    let (manager, mut sources) = copy_burst(client, &[types]);
    (manager, sources.remove(0))
}

/// Makes the clipboard device a client is told selections through.
fn watch_clipboard(client: &mut Client) -> wl_data_device_manager::WlDataDeviceManager {
    let manager: wl_data_device_manager::WlDataDeviceManager = client
        .globals
        .bind(&client.qh, 1..=3, ())
        .expect("every session advertises wl_data_device_manager");
    let _device = manager.get_data_device(&client.seat, &client.qh, ());
    client.roundtrip();
    manager
}

/// Waits until a client's clipboard names an offer with `text/plain`, pumping both ends.
fn wait_clipboard_offer(watcher: &mut Client, others: &mut [&mut Client]) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        watcher.roundtrip();
        for other in others.iter_mut() {
            other.roundtrip();
        }
        if watcher
            .state
            .selection
            .as_ref()
            .is_some_and(|types| types.iter().any(|mime| mime == "text/plain"))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the clipboard never offered text/plain: {:?}",
            watcher.state.selection
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Pastes `mime` from `pasting`'s clipboard offer, while `copying` answers its source, and returns the bytes.
fn paste_clipboard(pasting: &mut Client, copying: &mut Client, mime: &str) -> Vec<u8> {
    let offer = pasting
        .state
        .selection_offer
        .clone()
        .expect("the clipboard names an offer");
    let (reader, writer) = std::io::pipe().expect("a pipe");
    offer.receive(mime.to_owned(), writer.as_fd());
    drop(writer);
    pasting.roundtrip();
    let deadline = Instant::now() + TIMEOUT;
    while copying.state.sends.is_empty() {
        copying.roundtrip();
        pasting.roundtrip();
        assert!(Instant::now() < deadline, "the source was never asked for the bytes");
        thread::sleep(Duration::from_millis(5));
    }
    let mut bytes = Vec::new();
    reader.take(1 << 20).read_to_end(&mut bytes).expect("the pasted bytes");
    bytes
}

/// Roundtrips both clients for a while, to prove something did not happen.
fn settle(clients: &mut [&mut Client]) {
    for _ in 0..20 {
        for client in clients.iter_mut() {
            client.roundtrip();
        }
        thread::sleep(Duration::from_millis(10));
    }
}

// ---- the tests -------------------------------------------------------------------------------------------------------

#[test]
fn a_copy_on_the_host_is_pasted_in_the_session() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(&mut rig.host_client, &["text/plain"]);
        let _session_manager = watch_clipboard(&mut rig.session_client);

        // The person moves to the nested window: the host sends it its clipboard, and the session offers it on.
        focus_on_host(host, rig.nested_window_id);
        wait_clipboard_offer(&mut rig.session_client, &mut [&mut rig.host_client]);
        let bytes = paste_clipboard(&mut rig.session_client, &mut rig.host_client, "text/plain");
        assert_eq!(
            bytes, CLIPBOARD_BYTES,
            "the session's program read the host program's bytes"
        );
    });
}

#[test]
fn a_copy_in_the_session_is_pasted_on_the_host() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        let _host_manager = watch_clipboard(&mut rig.host_client);
        focus_on_host(host, rig.nested_window_id);
        // Let the session see the host's keyboard before it copies: the copy is offered only while the window has it.
        settle(&mut [&mut rig.session_client]);
        let (_manager, _source) = copy_types(&mut rig.session_client, &["text/plain"]);
        wait_host_holds(host, SelectionTarget::Clipboard, "text/plain");

        focus_on_host(host, rig.host_window_id);
        wait_clipboard_offer(&mut rig.host_client, &mut [&mut rig.session_client]);
        let bytes = paste_clipboard(&mut rig.host_client, &mut rig.session_client, "text/plain");
        assert_eq!(
            bytes, CLIPBOARD_BYTES,
            "the host's program read the session program's bytes"
        );
    });
}

#[test]
fn a_session_that_copies_twice_in_a_row_is_still_pasted_on_the_host() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        let _host_manager = watch_clipboard(&mut rig.host_client);
        focus_on_host(host, rig.nested_window_id);
        settle(&mut [&mut rig.session_client]);
        // Three copies in one burst, as a program that selects while the pointer moves makes them, each on its way to the
        // host before the host has answered the one before. Only the last offers `text/html`, so the host holding it is
        // the host holding the whole burst.
        let (_manager, _sources) = copy_burst(
            &mut rig.session_client,
            &[&["text/plain"], &["text/plain"], &["text/plain", "text/html"]],
        );
        wait_host_holds(host, SelectionTarget::Clipboard, "text/html");

        focus_on_host(host, rig.host_window_id);
        wait_clipboard_offer(&mut rig.host_client, &mut [&mut rig.session_client]);
        let bytes = paste_clipboard(&mut rig.host_client, &mut rig.session_client, "text/plain");
        assert_eq!(
            bytes, CLIPBOARD_BYTES,
            "the host's program read the bytes of the session's last copy"
        );
    });
}

#[test]
fn nothing_is_pasted_until_somebody_pastes() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(&mut rig.host_client, &["text/plain"]);
        let _session_manager = watch_clipboard(&mut rig.session_client);
        focus_on_host(host, rig.nested_window_id);
        wait_clipboard_offer(&mut rig.session_client, &mut [&mut rig.host_client]);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        assert!(
            rig.host_client.state.sends.is_empty(),
            "the host program was never asked for the bytes of a copy nobody pasted: {:?}",
            rig.host_client.state.sends
        );
    });
}

#[test]
fn the_session_sees_the_host_clipboard_only_while_its_window_has_the_keyboard() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(&mut rig.host_client, &["text/plain"]);
        let _session_manager = watch_clipboard(&mut rig.session_client);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        assert_eq!(
            rig.session_client.state.selection, None,
            "a window that does not have the host's keyboard is offered nothing"
        );

        focus_on_host(host, rig.nested_window_id);
        wait_clipboard_offer(&mut rig.session_client, &mut [&mut rig.host_client]);

        focus_on_host(host, rig.host_window_id);
        let deadline = Instant::now() + TIMEOUT;
        while rig.session_client.state.selection.is_some() {
            rig.session_client.roundtrip();
            rig.host_client.roundtrip();
            assert!(
                Instant::now() < deadline,
                "leaving the window never withdrew the host's clipboard from the session"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn a_copy_in_the_session_while_the_window_is_not_focused_does_not_reach_the_host() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        let _host_manager = watch_clipboard(&mut rig.host_client);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(&mut rig.session_client, &["text/plain"]);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        // The person goes to the nested window and away again: a program that was not looked at left no trace, not
        // even at the moment the window gets the keyboard.
        focus_on_host(host, rig.nested_window_id);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        assert_eq!(
            host.windows().selection(),
            None,
            "the window getting the keyboard did not hand the host a copy it made while it did not have it"
        );
        focus_on_host(host, rig.host_window_id);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        assert_eq!(
            rig.host_client.state.selection, None,
            "a background program of the session cannot set the person's clipboard"
        );
    });
}

#[test]
fn only_the_types_that_cross_are_offered_to_the_session() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(
            &mut rig.host_client,
            &["text/uri-list", "text/plain;charset=utf-8", "application/x-private"],
        );
        let _session_manager = watch_clipboard(&mut rig.session_client);
        focus_on_host(host, rig.nested_window_id);
        let deadline = Instant::now() + TIMEOUT;
        while rig.session_client.state.selection.is_none() {
            rig.session_client.roundtrip();
            rig.host_client.roundtrip();
            assert!(Instant::now() < deadline, "the session was offered nothing");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            rig.session_client.state.selection,
            Some(vec!["text/plain;charset=utf-8".to_owned()]),
            "a file list and an application type stay on the host"
        );
    });
}

#[test]
fn a_secret_copy_stays_on_the_host() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);
        focus_on_host(host, rig.host_window_id);
        let (_manager, _source) = copy_types(&mut rig.host_client, &["text/plain", "x-kde-passwordManagerHint"]);
        let _session_manager = watch_clipboard(&mut rig.session_client);
        focus_on_host(host, rig.nested_window_id);
        settle(&mut [&mut rig.session_client, &mut rig.host_client]);
        assert_eq!(
            rig.session_client.state.selection, None,
            "a copy a password manager marked secret is not offered to the session at all"
        );
    });
}

#[test]
fn the_primary_selection_crosses_both_ways() {
    let host = HostSession::start();
    host.run(|host| {
        let mut rig = Rig::start(host);

        // Host to session.
        let host_manager: zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1 = rig
            .host_client
            .globals
            .bind(&rig.host_client.qh, 1..=1, ())
            .expect("the host serves the primary selection");
        let host_device = host_manager.get_device(&rig.host_client.seat, &rig.host_client.qh, ());
        let session_manager: zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1 = rig
            .session_client
            .globals
            .bind(&rig.session_client.qh, 1..=1, ())
            .expect("the session serves the primary selection");
        let session_device = session_manager.get_device(&rig.session_client.seat, &rig.session_client.qh, ());
        rig.host_client.roundtrip();
        rig.session_client.roundtrip();

        focus_on_host(host, rig.host_window_id);
        let host_source = host_manager.create_source(&rig.host_client.qh, ());
        host_source.offer("text/plain".to_owned());
        host_device.set_selection(Some(&host_source), rig.host_client.state.last_serial);
        rig.host_client.roundtrip();
        focus_on_host(host, rig.nested_window_id);
        let deadline = Instant::now() + TIMEOUT;
        while rig.session_client.state.primary.is_none() {
            rig.session_client.roundtrip();
            rig.host_client.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the session was never offered the host's primary selection"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let offer = rig.session_client.state.primary_offer.clone().expect("an offer");
        let (reader, writer) = std::io::pipe().expect("a pipe");
        offer.receive("text/plain".to_owned(), writer.as_fd());
        drop(writer);
        let deadline = Instant::now() + TIMEOUT;
        while rig.host_client.state.primary_sends.is_empty() {
            rig.host_client.roundtrip();
            rig.session_client.roundtrip();
            assert!(Instant::now() < deadline, "the host's source was never asked");
            thread::sleep(Duration::from_millis(5));
        }
        let mut bytes = Vec::new();
        reader.take(1 << 20).read_to_end(&mut bytes).expect("the pasted bytes");
        assert_eq!(bytes, CLIPBOARD_BYTES, "host to session");

        // Session to host: the session program selects while its window has the keyboard.
        // A type of its own, so the host program's wait cannot be satisfied by the selection it made itself.
        let session_source = session_manager.create_source(&rig.session_client.qh, ());
        session_source.offer("text/plain;charset=utf-8".to_owned());
        session_device.set_selection(Some(&session_source), rig.session_client.state.last_serial);
        rig.session_client.roundtrip();
        // The copy is the host's when the host says so; the person goes on to the host's program only then.
        wait_host_holds(host, SelectionTarget::Primary, "text/plain;charset=utf-8");
        focus_on_host(host, rig.host_window_id);
        let deadline = Instant::now() + TIMEOUT;
        while rig.host_client.state.primary_offer.is_none()
            || rig
                .host_client
                .state
                .primary
                .as_ref()
                .is_none_or(|types| !types.iter().any(|mime| mime == "text/plain;charset=utf-8"))
        {
            rig.host_client.roundtrip();
            rig.session_client.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the host was never offered the session's primary selection"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let offer = rig.host_client.state.primary_offer.clone().expect("an offer");
        let (reader, writer) = std::io::pipe().expect("a pipe");
        offer.receive("text/plain;charset=utf-8".to_owned(), writer.as_fd());
        drop(writer);
        let deadline = Instant::now() + TIMEOUT;
        while rig.session_client.state.primary_sends.is_empty() {
            rig.host_client.roundtrip();
            rig.session_client.roundtrip();
            assert!(Instant::now() < deadline, "the session's source was never asked");
            thread::sleep(Duration::from_millis(5));
        }
        let mut bytes = Vec::new();
        reader.take(1 << 20).read_to_end(&mut bytes).expect("the pasted bytes");
        assert_eq!(bytes, CLIPBOARD_BYTES, "session to host");
        let _ = &rig.session;
    });
}

// ---- the bridge, driven directly -------------------------------------------------------------------------------------

/// A nested backend of its own on `host`, driven by a [`Probe`], whose window has the host's keyboard.
///
/// There is no session compositor between the test and the backend here, so the order in which the backend hears
/// things is the order of the test's calls and nothing else.
fn focused_backend(host: &nested_host::Handle) -> (Nested, Probe) {
    let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
    let mut probe = Probe::drawing(nested_host::RED);
    nested_host::draw(&mut nested, &mut probe);
    let window = host
        .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .expect("one mapped window")
        .clone();
    focus_on_host(host, window.id);
    // The host sent the keyboard before it said the window had it, so one round trip brings it in.
    nested.roundtrip().expect("the host answers a round trip");
    let _ = nested.dispatch(&mut probe);
    (nested, probe)
}

#[test]
fn copies_the_session_makes_before_the_host_answers_are_not_handed_back_to_it() {
    let host = HostSession::start();
    host.run(|host| {
        let (mut nested, mut probe) = focused_backend(host);
        let plain = || vec!["text/plain;charset=utf-8".to_owned()];
        let html = || vec!["text/html".to_owned()];
        for target in [SelectionTarget::Clipboard, SelectionTarget::Primary] {
            // Three copies in a row, as a program that selects while the pointer moves makes them, each before the host
            // has answered the one before.
            for types in [plain(), plain(), html()] {
                <Nested as Backend<Probe>>::session_selection(&mut nested, target, Some(types));
            }
        }
        // The host answers the requests in the order it got them, so its answer to this comes after all six.
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(&mut probe);
        assert_eq!(
            probe.selections,
            vec![],
            "the host tells the window of each copy the session made, and none of them is the host's own selection"
        );
    });
}
