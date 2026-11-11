//! The portal backend end to end: D-Bus caller, portal, IPC, live compositor.
//!
//! Nothing between the caller and the compositor is a fake. The caller is a connection on a
//! private session bus calling the `org.freedesktop.impl.portal.*` methods the frontend calls;
//! the portal is `xdg_desktop_portal_gnoblin::serve::serve`, the body of the shipped binary's
//! `main`, on that bus and on the compositor's IPC socket; the compositor is a running
//! headless session; and the consent dialog is the compositor's own, answered with the keys a
//! person presses, sent over the `input` role's privileged connection. What the test asserts
//! is what a user and an app can observe: the response code of `Start`, whether
//! `ConnectToEIS` hands out a descriptor, whether that descriptor moves a Wayland client's
//! pointer, whether a PipeWire node exists, and what the compositor still holds once a
//! session ends.
//!
//! Each test owns its bus, its compositor and its sockets, so the tests run concurrently.

mod support;

#[path = "../../xdg-desktop-portal-gnoblin/tests/support/bus.rs"]
mod bus;

use std::collections::HashMap;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use bus::Bus;
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::{self, portal_key};
use gnoblin_client::{Client as IpcClient, ClientError, HandshakeError};
use gnoblin_input_eis::Caps;
use gnoblin_proto::CodecError;
use support::ei::{EiClient, focused_client, frame, wait_for_key};
use support::pipewire::Daemon;
use support::{PointerEvent, Session, Wire, wait_for};
use xdg_desktop_portal_gnoblin::serve::{Options as ServeOptions, serve};
use xdg_desktop_portal_gnoblin::{BUS_NAME, IpcHost};
use zbus::connection::Builder;
use zbus::{Connection, ObjectPath, OwnedVariant, Proxy, Variant};

/// The path the backend serves its interfaces at.
const PATH: &str = "/org/freedesktop/portal/desktop";
/// The interfaces under test.
const REMOTE_DESKTOP: &str = "org.freedesktop.impl.portal.RemoteDesktop";
const SCREEN_CAST: &str = "org.freedesktop.impl.portal.ScreenCast";
const INPUT_CAPTURE: &str = "org.freedesktop.impl.portal.InputCapture";
const SESSION: &str = "org.freedesktop.impl.portal.Session";

/// The RemoteDesktop `types` bits.
const KEYBOARD: u32 = 1;
const POINTER: u32 = 2;

/// `KEY_A` and `KEY_B` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
const KEY_B: u32 = 48;

/// The wait after a dialog opens before Enter can release Allow.
const ARM_DELAY: Duration = Duration::from_millis(700);
/// How long a dialog may take to open after the call that asks for it.
const DIALOG_WAIT: Duration = Duration::from_secs(20);
/// How long a state may take to arrive.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How long the portal may take to answer one D-Bus call, consent dialogs included.
const CALL_LIMIT: Duration = Duration::from_secs(30);
/// The response code of a user who said no, and of a request that failed for another reason.
const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;

/// The app the caller says it is.
const APP_ID: &str = "org.example.Remote";

/// The result dictionary of a portal call.
type Results = HashMap<String, OwnedVariant>;

/// What the user does with a dialog.
#[derive(Clone, Copy, Debug)]
enum Answer {
    Allow,
    Deny,
}

/// What the portal's IPC connection has done with the consent dialog.
///
/// The compositor accepts a key with no dialog open and does nothing with it, so a key cannot
/// tell the test whether the dialog it is for has opened. The portal's own connection can: it
/// writes one `portal.ui` request per dialog and reads one reply when the dialog settles, and
/// [`Tap`] counts both.
#[derive(Clone, Default)]
struct Dialogs {
    offered: Arc<AtomicUsize>,
    settled: Arc<AtomicUsize>,
}

/// The portal's transport, with its dialogs counted.
struct Tap {
    inner: UnixTransport,
    dialogs: Dialogs,
}

impl gnoblin_proto::Transport for Tap {
    fn send(&mut self, envelope: &gnoblin_proto::Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        if envelope.member == wire::member::PORTAL_UI {
            self.dialogs.offered.fetch_add(1, Ordering::SeqCst);
        }
        gnoblin_proto::Transport::send(&mut self.inner, envelope, fds)
    }

    fn receive(&mut self, fds: &mut Vec<OwnedFd>) -> Result<Option<gnoblin_proto::Envelope>, CodecError> {
        let received = gnoblin_proto::Transport::receive(&mut self.inner, fds)?;
        if received
            .as_ref()
            .is_some_and(|envelope| envelope.member == wire::member::PORTAL_UI)
        {
            self.dialogs.settled.fetch_add(1, Ordering::SeqCst);
        }
        Ok(received)
    }
}

impl gnoblin_client::Transport for Tap {
    fn send_handshake(&mut self, frame: &gnoblin_proto::Handshake) -> Result<(), HandshakeError> {
        gnoblin_client::Transport::send_handshake(&mut self.inner, frame)
    }

    fn receive_handshake(&mut self) -> Result<gnoblin_proto::Handshake, HandshakeError> {
        gnoblin_client::Transport::receive_handshake(&mut self.inner)
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), ClientError> {
        gnoblin_client::Transport::set_read_timeout(&mut self.inner, timeout)
    }
}

/// The keys a person presses: the input role's connection, and what the portal has offered.
struct Keyboard {
    wire: Wire,
    dialogs: Dialogs,
}

/// One key request number for the input role's connection.
fn next_key_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Keyboard {
    /// Presses `key` on the open dialog, if there is one.
    fn press(&mut self, key: &str) {
        self.wire.send(&portal_key(next_key_id(), key, false));
        let reply = self.wire.receive().expect("the compositor answers a key");
        assert_eq!(reply.error_code(), None, "the compositor refused the key `{key}`");
    }

    /// Waits until `count` dialogs have been offered, and a moment longer for the compositor to
    /// open the last one: the offer and this connection's keys reach it on different sockets.
    #[track_caller]
    fn wait_for_offer(&self, count: usize) {
        let deadline = Instant::now() + DIALOG_WAIT;
        while self.dialogs.offered.load(Ordering::SeqCst) < count {
            assert!(
                Instant::now() < deadline,
                "the portal never offered consent dialog {count}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(150));
    }

    /// Waits until `count` dialogs have settled.
    #[track_caller]
    fn wait_for_settled(&self, count: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while self.dialogs.settled.load(Ordering::SeqCst) < count {
            assert!(Instant::now() < deadline, "consent dialog {count} never settled");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Answers dialog number `number` (counting from one) the way a person would: Escape denies
    /// it, and Tab then Enter, once the dialog is armed, allows it.
    #[track_caller]
    fn answer(&mut self, number: usize, answer: Answer) {
        self.wait_for_offer(number);
        match answer {
            Answer::Deny => self.press("escape"),
            Answer::Allow => {
                thread::sleep(ARM_DELAY);
                self.press("tab");
                thread::sleep(Duration::from_millis(100));
                self.press("enter");
            }
        }
        self.wait_for_settled(number);
    }

    /// How many dialogs have settled so far.
    fn settled(&self) -> usize {
        self.dialogs.settled.load(Ordering::SeqCst)
    }
}

/// Runs `work` while another thread answers one dialog per entry of `answers`, in order.
fn with_dialogs<T>(keyboard: &mut Keyboard, answers: &[Answer], work: impl FnOnce() -> T) -> T {
    let first = keyboard.settled();
    thread::scope(|scope| {
        let answering = scope.spawn(move || {
            for (index, answer) in answers.iter().enumerate() {
                keyboard.answer(first + index + 1, *answer);
            }
        });
        let result = work();
        answering.join().expect("every dialog was answered");
        result
    })
}

/// The frontend's stand-in: one connection on the private bus that calls the backend.
struct Caller {
    connection: Connection,
    app_id: String,
    next: AtomicU32,
}

impl Caller {
    fn connect(bus: &Bus, app_id: &str) -> Caller {
        let connection = zbus::block_on(Builder::address(bus.address()).build()).expect("a caller on the private bus");
        Caller {
            connection,
            app_id: app_id.to_owned(),
            next: AtomicU32::new(0),
        }
    }

    /// A fresh path for a request or session, unique to this caller.
    fn path(&self, kind: &str) -> String {
        let sender = self
            .connection
            .unique_name()
            .expect("a connected caller has a name")
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let number = self.next.fetch_add(1, Ordering::Relaxed);
        format!("{PATH}/{kind}/{sender}/t{number}")
    }

    async fn proxy(&self, interface: &'static str, path: &str) -> Proxy<'static> {
        Proxy::new(&self.connection, BUS_NAME, path.to_owned(), interface)
            .await
            .expect("a proxy for the backend")
    }

    /// One method call, which fails the test if the portal does not answer within [`CALL_LIMIT`]:
    /// a handler that never returns is a hung portal, and the test must say so.
    async fn call(
        &self,
        interface: &'static str,
        path: &str,
        member: &str,
        body: &(impl serde::Serialize + zbus::DynamicType),
    ) -> zbus::Result<zbus::Message> {
        let proxy = self.proxy(interface, path).await;
        let answered = futures_lite::future::or(async { Some(proxy.call_method(member, body).await) }, async {
            async_io::Timer::after(CALL_LIMIT).await;
            None
        })
        .await;
        answered.unwrap_or_else(|| panic!("{interface}.{member} did not answer within {CALL_LIMIT:?}"))
    }

    /// `CreateSession`, answering the new session's path.
    async fn create_session(&self, interface: &'static str) -> String {
        let session = self.path("session");
        let handle = self.path("request");
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let handle = ObjectPath::try_from(handle.as_str()).expect("a request path");
        let session_path = ObjectPath::try_from(session.as_str()).expect("a session path");
        let message = if interface == INPUT_CAPTURE {
            // InputCapture's CreateSession also carries the parent window.
            self.call(
                interface,
                PATH,
                "CreateSession",
                &(handle, session_path, self.app_id.as_str(), "", options),
            )
            .await
        } else {
            self.call(
                interface,
                PATH,
                "CreateSession",
                &(handle, session_path, self.app_id.as_str(), options),
            )
            .await
        }
        .expect("CreateSession answers");
        let (response, _): (u32, Results) = message.body().deserialize().expect("a response and results");
        assert_eq!(response, RESPONSE_OK, "the session is created");
        session
    }

    /// `SelectDevices` or `SelectSources`, with `types` as the one option.
    async fn select(&self, interface: &'static str, member: &str, session: &str, types: u32) {
        let handle = self.path("request");
        let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
        options.insert("types", Variant::from(types));
        let handle = ObjectPath::try_from(handle.as_str()).expect("a request path");
        let session = ObjectPath::try_from(session).expect("a session path");
        let message = self
            .call(
                interface,
                PATH,
                member,
                &(handle, session, self.app_id.as_str(), options),
            )
            .await
            .unwrap_or_else(|error| panic!("{member} answers: {error}"));
        let (response, _): (u32, Results) = message.body().deserialize().expect("a response and results");
        assert_eq!(response, RESPONSE_OK, "{member} is recorded");
    }

    /// `Start`: the response code and results, or the error the backend answered with.
    async fn start(&self, interface: &'static str, session: &str) -> zbus::Result<(u32, Results)> {
        let handle = self.path("request");
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let handle = ObjectPath::try_from(handle.as_str()).expect("a request path");
        let session = ObjectPath::try_from(session).expect("a session path");
        let message = self
            .call(
                interface,
                PATH,
                "Start",
                &(handle, session, self.app_id.as_str(), "", options),
            )
            .await?;
        Ok(message.body().deserialize().expect("a response and results"))
    }

    /// `ConnectToEIS`: the descriptor the backend hands out, as a stream.
    async fn connect_to_eis(&self, interface: &'static str, session: &str) -> zbus::Result<UnixStream> {
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let session = ObjectPath::try_from(session).expect("a session path");
        let message = self
            .call(
                interface,
                PATH,
                "ConnectToEIS",
                &(session, self.app_id.as_str(), options),
            )
            .await?;
        let fd: zbus::OwnedFd = message.body().deserialize().expect("a file descriptor");
        Ok(UnixStream::from(OwnedFd::from(fd)))
    }

    /// `Session.Close` on the session's own object.
    async fn close(&self, session: &str) -> zbus::Result<()> {
        self.call(SESSION, session, "Close", &()).await.map(drop)
    }

    /// Creates, selects and starts a remote desktop session for a keyboard and a pointer.
    async fn remote_session(&self) -> String {
        let session = self.create_session(REMOTE_DESKTOP).await;
        self.select(REMOTE_DESKTOP, "SelectDevices", &session, KEYBOARD | POINTER)
            .await;
        session
    }

    /// Leaves the bus, which is all a crashed caller does.
    async fn leave(self) {
        let _ = self.connection.close().await;
    }
}

/// Whether a refusal is the D-Bus error `name`.
fn is_error(error: &zbus::Error, name: &str) -> bool {
    error.to_string().contains(name)
}

/// The node ids of a `Start` reply's `streams`.
fn stream_nodes(results: &Results) -> Vec<u32> {
    let Some(streams) = results.get("streams") else {
        return Vec::new();
    };
    let Variant::Array(array) = &**streams else {
        panic!("`streams` is an array, got {streams:?}");
    };
    array
        .inner()
        .iter()
        .map(|entry| {
            let Variant::Structure(stream) = entry else {
                panic!("a stream is a structure, got {entry:?}");
            };
            let Some(Variant::U32(node)) = stream.fields().first() else {
                panic!("a stream starts with its node id, got {stream:?}");
            };
            *node
        })
        .collect()
}

/// The EI capability mask the portal's keyboard and pointer stand for.
fn keyboard_and_pointer_mask() -> u64 {
    u64::from((Caps::KEYBOARD | Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL).bits())
}

/// A private bus, a live compositor, and the portal serving one against the other.
struct Rig {
    /// The portal's own bus connection; closing it ends the service.
    portal: Connection,
    /// The input role's end: the keys a person presses.
    input: Keyboard,
    bus: Bus,
    session: Session,
    /// Holds the directory of the PipeWire socket that is not there, when no daemon is given.
    _no_daemon: tempfile::TempDir,
}

impl Rig {
    /// Starts everything. `daemon` is the PipeWire daemon the compositor's casts go to; without
    /// one they go to a socket nothing listens on, so a stream is refused by name.
    fn start(daemon: Option<&Daemon>) -> Rig {
        let no_daemon = tempfile::tempdir().expect("a directory with no daemon in it");
        let socket = daemon.map_or_else(|| no_daemon.path().join("pipewire-0"), Daemon::socket);
        let bus = Bus::start();
        let (session, mut ends) = Session::start_privileged_configured(&["input"], move |config| {
            config.wayland.pipewire_socket = Some(socket);
        });
        let (_, stream) = ends.pop().expect("the session handed out the input role's end");
        let wire = Wire::over(stream);
        wire.set_read_timeout(Duration::from_secs(10));
        let dialogs = Dialogs::default();
        let input = Keyboard {
            wire,
            dialogs: dialogs.clone(),
        };

        // The portal's connection is the session owner's: a plain socket of the owner's uid.
        let transport = Tap {
            inner: UnixTransport::connect(session.socket()).expect("the session is listening"),
            dialogs,
        };
        let ipc = IpcClient::connect(transport, gnoblin_compositor::version()).expect("the session accepts this build");
        let shell = IpcHost::pair(ipc);
        let portal = zbus::block_on(Builder::address(bus.address()).build()).expect("the portal's bus connection");
        let (ready, serving) = mpsc::channel();
        let connection = portal.clone();
        thread::Builder::new()
            .name("portal-e2e".to_owned())
            .spawn(move || {
                let options = ServeOptions {
                    pictures: None,
                    secret_served: false,
                    consent_lifetime: 60_000,
                };
                let ended = zbus::block_on(serve(connection, Some(shell), options, &mut || {
                    let _ = ready.send(());
                }));
                if let Err(error) = ended {
                    eprintln!("the portal ended with an error: {error}");
                }
            })
            .expect("the portal thread");
        serving.recv_timeout(TIMEOUT).expect("the portal serves");
        Rig {
            portal,
            input,
            bus,
            session,
            _no_daemon: no_daemon,
        }
    }

    fn caller(&self, app_id: &str) -> Caller {
        Caller::connect(&self.bus, app_id)
    }

    /// Whether the compositor holds an approved EIS session under `path`.
    fn holds(&self, path: &str) -> bool {
        self.session.eis().has_session(path)
    }

    /// Waits until the compositor holds no EIS session under `path`.
    #[track_caller]
    fn wait_until_released(&self, path: &str) {
        wait_for(&format!("the compositor to drop `{path}`"), || {
            (!self.holds(path)).then_some(())
        });
    }

    /// A second connection on the compositor's IPC socket, as another process of the user's.
    fn other_ipc(&self) -> IpcClient<UnixTransport> {
        IpcClient::connect(
            UnixTransport::connect(self.session.socket()).expect("the session is listening"),
            gnoblin_compositor::version(),
        )
        .expect("the session accepts this build")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = zbus::block_on(self.portal.clone().close());
    }
}

/// Binds the approved devices of `remote` and waits until both are resumed.
#[track_caller]
fn bind_devices(remote: &mut EiClient) {
    remote.wait_for("the approved capabilities", |client| {
        client.capabilities == keyboard_and_pointer_mask()
    });
    remote.bind(keyboard_and_pointer_mask());
    remote.wait_for("the keyboard and pointer devices", |client| {
        client.devices.len() == 2
            && client
                .devices
                .iter()
                .all(|device| device.done && device.resumed.is_some())
    });
}

#[test]
fn an_approved_remote_desktop_moves_a_wayland_clients_pointer_over_the_descriptor() {
    let mut rig = Rig::start(None);
    let mut app = focused_client(&rig.session);
    app.pointers_on_every_seat();
    let (x, y, _, _) = frame(&rig.session, "eis-test");

    let caller = rig.caller(APP_ID);
    let path = zbus::block_on(caller.remote_session());
    // Both questions are asked in order, the screen first and the input second: two dialogs.
    let (response, _) = with_dialogs(&mut rig.input, &[Answer::Allow, Answer::Allow], || {
        zbus::block_on(caller.start(REMOTE_DESKTOP, &path))
    })
    .expect("Start answers");
    assert_eq!(response, RESPONSE_OK, "the user allowed both questions");
    assert!(rig.holds(&path), "the compositor was told the user approved `{path}`");

    let stream =
        zbus::block_on(caller.connect_to_eis(REMOTE_DESKTOP, &path)).expect("an approved session gets a descriptor");
    let mut remote = EiClient::over(stream);
    bind_devices(&mut remote);

    remote.motion_absolute(x + 50.0, y + 50.0);
    let deadline = Instant::now() + TIMEOUT;
    while !app
        .pointer_events()
        .iter()
        .any(|(_, event)| matches!(event, PointerEvent::Enter { .. }))
    {
        app.roundtrip();
        remote.pump();
        assert!(
            Instant::now() < deadline,
            "the Wayland client never saw the pointer arrive: {:?}",
            app.pointer_events()
        );
        thread::sleep(Duration::from_millis(5));
    }
    remote.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);
}

#[test]
fn a_denied_remote_desktop_answers_cancelled_and_opens_no_descriptor() {
    let mut rig = Rig::start(None);
    let caller = rig.caller(APP_ID);
    let path = zbus::block_on(caller.remote_session());

    // The user refuses the first question, so the second is never asked: one dialog.
    let (response, results) = with_dialogs(&mut rig.input, &[Answer::Deny], || {
        zbus::block_on(caller.start(REMOTE_DESKTOP, &path))
    })
    .expect("a refusal is a response, not an error");
    assert_eq!(response, RESPONSE_CANCELLED, "the user cancelled");
    assert!(results.is_empty(), "a refusal carries no results: {results:?}");

    let refused = zbus::block_on(caller.connect_to_eis(REMOTE_DESKTOP, &path));
    assert!(refused.is_err(), "a refused session has no descriptor");
    // What the compositor holds for the session is the user's refusal, not an approval: the
    // gate answers a connect for it by name, and opens no socket.
    let gate = rig
        .session
        .eis()
        .connect_session(&path, Vec::new())
        .expect_err("the compositor opened a socket for a refused session");
    assert_eq!(gate.to_string(), "the user refused the session");

    // The compositor was told the user said no: it records the refusal under the portal's
    // connection, so another connection cannot take the session's name and decide it.
    let mut other = rig.other_ipc();
    match other.remote_grant(&path, KEYBOARD | POINTER) {
        Err(ClientError::Refused { code, .. }) => {
            assert_eq!(
                code,
                wire::code::SESSION_TAKEN,
                "the refusal was recorded under the portal's connection"
            );
        }
        other => panic!("another connection decided the refused session: {other:?}"),
    }

    // Ending the refused session over D-Bus forgets the refusal: nothing is left behind.
    zbus::block_on(caller.close(&path)).expect("the session closes");
    rig.wait_until_released(&path);
}

#[test]
fn a_remote_desktop_with_screen_denied_never_asks_for_input() {
    // The second question is the narrower one: a user who denied the screen is not asked about
    // input, and no dialog is left open for it.
    let mut rig = Rig::start(None);
    let caller = rig.caller(APP_ID);
    let path = zbus::block_on(caller.remote_session());
    let (response, _) = with_dialogs(&mut rig.input, &[Answer::Deny], || {
        zbus::block_on(caller.start(REMOTE_DESKTOP, &path))
    })
    .expect("Start answers");
    assert_eq!(response, RESPONSE_CANCELLED);
    assert_eq!(
        rig.input.dialogs.offered.load(Ordering::SeqCst),
        1,
        "a second dialog was offered after the first question was refused"
    );
}

#[test]
fn closing_the_session_over_dbus_ends_the_compositor_session_and_revokes_input() {
    let mut rig = Rig::start(None);
    let mut app = focused_client(&rig.session);
    let caller = rig.caller(APP_ID);
    let path = zbus::block_on(caller.remote_session());
    with_dialogs(&mut rig.input, &[Answer::Allow, Answer::Allow], || {
        zbus::block_on(caller.start(REMOTE_DESKTOP, &path))
    })
    .expect("Start answers");
    let stream = zbus::block_on(caller.connect_to_eis(REMOTE_DESKTOP, &path)).expect("a descriptor");
    let mut remote = EiClient::over(stream);
    bind_devices(&mut remote);
    remote.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);

    zbus::block_on(caller.close(&path)).expect("the session closes");
    rig.wait_until_released(&path);
    let deadline = Instant::now() + TIMEOUT;
    while !app.keys().iter().any(|&(key, state)| key == KEY_A && state == 0) {
        app.roundtrip();
        assert!(
            Instant::now() < deadline,
            "the held key was never released: {:?}",
            app.keys()
        );
        thread::sleep(Duration::from_millis(5));
    }
    remote.wait_for("the disconnect", |client| client.disconnected);

    remote.key(KEY_B, true);
    for _ in 0..30 {
        app.roundtrip();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.keys().iter().all(|&(key, _)| key != KEY_B),
        "a closed session's events reached the seat: {:?}",
        app.keys()
    );
    assert!(
        zbus::block_on(caller.connect_to_eis(REMOTE_DESKTOP, &path)).is_err(),
        "a closed session has no descriptor"
    );
}

#[test]
fn the_caller_leaving_the_bus_ends_its_remote_desktop_session() {
    let mut rig = Rig::start(None);
    let mut app = focused_client(&rig.session);
    let caller = rig.caller(APP_ID);
    let path = zbus::block_on(caller.remote_session());
    with_dialogs(&mut rig.input, &[Answer::Allow, Answer::Allow], || {
        zbus::block_on(caller.start(REMOTE_DESKTOP, &path))
    })
    .expect("Start answers");
    let stream = zbus::block_on(caller.connect_to_eis(REMOTE_DESKTOP, &path)).expect("a descriptor");
    let mut remote = EiClient::over(stream);
    bind_devices(&mut remote);
    remote.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);
    assert!(rig.holds(&path));

    zbus::block_on(caller.leave());
    rig.wait_until_released(&path);
    remote.wait_for("the disconnect", |client| client.disconnected);
}

#[test]
fn an_approved_screen_cast_answers_a_pipewire_node_and_closing_removes_it() {
    let daemon = Daemon::start();
    let mut rig = Rig::start(Some(&daemon));
    let caller = rig.caller("org.example.Cast");
    let session = zbus::block_on(caller.create_session(SCREEN_CAST));
    zbus::block_on(caller.select(SCREEN_CAST, "SelectSources", &session, 1));

    let (response, results) = with_dialogs(&mut rig.input, &[Answer::Allow], || {
        zbus::block_on(caller.start(SCREEN_CAST, &session))
    })
    .expect("Start answers");
    assert_eq!(response, RESPONSE_OK, "the user allowed the cast");
    let nodes = stream_nodes(&results);
    assert_eq!(nodes.len(), 1, "one stream is answered: {results:?}");
    daemon.wait_until_present(nodes[0]);

    zbus::block_on(caller.close(&session)).expect("the session closes");
    daemon.wait_until_gone(nodes[0]);
}

#[test]
fn a_denied_screen_cast_answers_cancelled_and_makes_no_node() {
    let daemon = Daemon::start();
    let mut rig = Rig::start(Some(&daemon));
    let caller = rig.caller("org.example.Cast");
    let session = zbus::block_on(caller.create_session(SCREEN_CAST));
    zbus::block_on(caller.select(SCREEN_CAST, "SelectSources", &session, 1));

    let (response, results) = with_dialogs(&mut rig.input, &[Answer::Deny], || {
        zbus::block_on(caller.start(SCREEN_CAST, &session))
    })
    .expect("a refusal is a response, not an error");
    assert_eq!(response, RESPONSE_CANCELLED, "the user cancelled");
    assert!(stream_nodes(&results).is_empty(), "a refusal names no stream");
    assert!(!daemon.holds_stream(), "no node exists for a refused cast");
}

#[test]
fn the_caller_leaving_the_bus_removes_its_screen_cast_node() {
    let daemon = Daemon::start();
    let mut rig = Rig::start(Some(&daemon));
    let caller = rig.caller("org.example.Cast");
    let session = zbus::block_on(caller.create_session(SCREEN_CAST));
    zbus::block_on(caller.select(SCREEN_CAST, "SelectSources", &session, 1));
    let (response, results) = with_dialogs(&mut rig.input, &[Answer::Allow], || {
        zbus::block_on(caller.start(SCREEN_CAST, &session))
    })
    .expect("Start answers");
    assert_eq!(response, RESPONSE_OK);
    let nodes = stream_nodes(&results);
    daemon.wait_until_present(nodes[0]);

    zbus::block_on(caller.leave());
    daemon.wait_until_gone(nodes[0]);
}

#[test]
fn input_capture_start_answers_not_supported_and_holds_nothing() {
    let mut rig = Rig::start(None);
    let caller = rig.caller(APP_ID);
    let session = zbus::block_on(caller.create_session(INPUT_CAPTURE));

    // The user is asked about input, and a yes is answered `NotSupported`: the compositor
    // serves no input capture, so nothing was granted that nothing could use.
    let started = with_dialogs(&mut rig.input, &[Answer::Allow], || {
        zbus::block_on(caller.start(INPUT_CAPTURE, &session))
    });
    let error = started.expect_err("input capture is not served");
    assert!(
        is_error(&error, "NotSupported"),
        "the refusal names NotSupported: {error}"
    );
    assert!(
        !rig.holds(&session),
        "the compositor holds no session for input capture"
    );
    let descriptor = zbus::block_on(caller.connect_to_eis(INPUT_CAPTURE, &session));
    assert!(descriptor.is_err(), "input capture hands out no descriptor");
    assert!(!rig.holds(&session), "the compositor was never told an approval");
}

#[test]
fn a_denied_input_capture_answers_cancelled() {
    let mut rig = Rig::start(None);
    let caller = rig.caller(APP_ID);
    let session = zbus::block_on(caller.create_session(INPUT_CAPTURE));
    let (response, _) = with_dialogs(&mut rig.input, &[Answer::Deny], || {
        zbus::block_on(caller.start(INPUT_CAPTURE, &session))
    })
    .expect("a refusal is a response, not an error");
    assert_eq!(response, RESPONSE_CANCELLED);
}
