//! One socket, one dispatcher, one reader thread per client.
//!
//! Design/rust-compositor.md, "One IPC": every client of the compositor — shells,
//! `gnoblinctl`, the portal, the services role, the script runtime — speaks one
//! binary protocol on one socket, and the first frame is the fixed-layout
//! handshake. This module is that socket.
//!
//! # Why the threads are arranged this way
//!
//! A calloop loop and the backend it drives cannot move between threads, so the
//! Wayland session owns the thread that called [`crate::Compositor::run`]. The IPC
//! server therefore owns a thread of its own, and the two sides meet at two seams
//! and nowhere else:
//!
//! - the session publishes [`SessionSnapshot`]s, and an [`Observer`] queues one
//!   "look again" into this thread's channel;
//! - a `window.focus` call goes out as
//!   [`ControlHandle::focus`](gnoblin_wayland::ControlHandle::focus), which the
//!   loop answers in its next turn.
//!
//! Inside this thread there is exactly one owner of the graph: the dispatcher. It
//! alone subscribes, calls, completes and writes, so nothing above the socket layer
//! needs a lock and the model's ordering rule holds without one. Each connection's
//! reader has a thread of its own because a frame arrives when it arrives: a
//! subscription with nothing to say must not make the socket unreadable, and a
//! client that never sends must not stop another client's frame from being read.
//!
//! # Failure modes first
//!
//! 1. **A peer that dies mid-subscription stops the compositor.** Its reader sees the
//!    closed socket, the dispatcher drops the connection, its subscriptions and its
//!    writer, and every other client keeps being served:
//!    `an_ipc_peer_that_dies_does_not_stop_the_compositor`.
//! 2. **A silent peer blocks the session.** Its reader is the only thread that waits
//!    on it, and the handshake's own deadline ends a connection that never speaks:
//!    `a_silent_peer_does_not_block_the_session`.
//! 3. **A focus call answers before the diff it caused.** The dispatcher publishes
//!    the snapshot the loop published before it completes the operation, so a client
//!    that waits for the completion and then reads `focused` sees the new value:
//!    `focus_completes_after_the_diff_it_caused`.
//! 4. **A call on a dead object is accepted.** The graph refuses it with
//!    `object-unknown`, and a method this build does not serve with
//!    `unknown-member`, so neither can hang:
//!    `focus_on_a_closed_window_is_refused`.
//! 5. **A malformed frame is half-applied.** A payload the wire refuses is answered
//!    with an error frame and changes nothing, and the connection survives to send a
//!    frame that does decode: `a_malformed_frame_is_refused_and_the_connection_survives`.
//! 6. **A client's requests outlive its connection.** Leaving releases its
//!    subscriptions and closes its writer, so an identity it will never use again is
//!    not queued forever, and the connection that dies is the one case failure mode 1
//!    covers.
//! 7. **A client that is not privileged calls a privileged member.** The capability
//!    gate answers `denied` before the member is looked at, so a plain socket connection
//!    can neither offer a request nor commit a configuration. The session owner's connection
//!    may read the configuration and ask for a reload, which the runtime still performs:
//!    `tests/privilege.rs::a_socket_client_cannot_use_a_privileged_member`.
//! 8. **An offer is routed to a connection that never answers.** The slot decides, a
//!    slot with no provider is refused with `no-provider`, and both sides of a request
//!    always learn its one completion:
//!    `src/ipc/requests.rs`'s own tests.

use std::collections::BTreeMap;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_capture::still::{self, Refusal};
use gnoblin_client::wire;
use gnoblin_config::{LayoutReference, LayoutRoot, LayoutSelector};

use gnoblin_layers::provider::COMPOSITOR_CRATE;
use gnoblin_layers::{Band, LayerProvider, LayerScope, LayerSpec};
use gnoblin_model::{ModelError, OBJECT_GONE, OperationOutcome, Subscription, Tick};
use gnoblin_proto::{Capabilities, Capability, Envelope, Handshake, HandshakeKind, read_frame, write_frame};
use gnoblin_types::{ClientId, ErrorCode, Version, WindowId, WorkspaceId};
use gnoblin_wayland::{
    ControlHandle, EffectSpec, Observer, PresentAnswer, ProcessIdentity, SessionSnapshot, WindowChange,
    WindowChangeRefusal, WindowProcess, WindowVisual, WindowVisualRefusal, WorkspaceSetRefusal,
};

use super::extension::{ExtensionRoute, ExtensionRouter};
use super::portal::{MALFORMED, PortalScope, PortalService, resolve_portal_scope};
use super::reload::{Reload, ReloadVerdict};
use super::requests::{FrameDelivery, Requests};
use super::{IpcError, Privileged, SessionSync, SocketPath};

use crate::log;

/// How long a new connection gets to send its handshake.
///
/// A connection that speaks nothing is a stuck process this compositor must not
/// spend a thread on forever, and the handshake is one fixed 20-byte frame.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long one write may block before the connection is dropped.
///
/// The dispatcher writes to every subscriber from one thread, so a client that stops
/// reading must cost its own connection and not the session.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the dispatcher looks at its own clock.
///
/// The dispatcher is the only owner of the request layer's deadlines, and it is woken
/// by a channel rather than by a timer, so this is the resolution at which a request
/// times out. It is also the only thing that happens on an idle session: one deadline
/// check per tick and no writes.
const DEADLINE_TICK: Duration = Duration::from_millis(50);

/// One thing the IPC server has to do, in the order it arrived.
pub(crate) enum IpcEvent {
    /// A client finished the handshake and may be served.
    Accepted {
        /// Which connection it is.
        client: ClientId,
        /// The writing half the dispatcher writes frames to.
        writer: UnixStream,
        /// What the compositor granted it. A connection accepted from the socket holds
        /// nothing; a privileged one holds what its role grants.
        capabilities: Capabilities,
    },
    /// One frame a client sent.
    Request { client: ClientId, envelope: Envelope },
    /// A privileged connection's stream ended. The connection keeps its identity and its
    /// grants, because they belong to the role and not to the child that held the stream;
    /// only what that stream carried ends.
    Lost {
        /// Which connection it is.
        client: ClientId,
    },
    /// A client's connection ended, cleanly or otherwise.
    Closed { client: ClientId },
    /// The session's snapshot may have changed.
    Changed,
    /// Stop serving.
    Shutdown,
}

/// The channel every producer of an [`Event`] shares.
pub(crate) fn events() -> (Sender<IpcEvent>, Receiver<IpcEvent>) {
    channel()
}

/// The observer that turns a published snapshot into one queued `Changed`.
///
/// It returns the flag the dispatcher must clear when it looks: the snapshot is
/// whole state, so two wake-ups between two looks are one look, and a queue of them
/// would make a busy session write the same diff twice.
pub(crate) fn wake(events: Sender<IpcEvent>) -> (Arc<dyn Observer>, Arc<AtomicBool>) {
    let pending = Arc::new(AtomicBool::new(false));
    let observer = Wake {
        events,
        pending: Arc::clone(&pending),
    };
    (Arc::new(observer), pending)
}

/// The observer handed to the Wayland session.
#[derive(Debug)]
struct Wake {
    events: Sender<IpcEvent>,
    pending: Arc<AtomicBool>,
}

impl Observer for Wake {
    fn published(&self, _snapshot: &SessionSnapshot) {
        // The swap is the whole coalescing rule: the first publisher since the last
        // look queues one wake-up, and the rest are already implied by it.
        if !self.pending.swap(true, Ordering::AcqRel) {
            let _ = self.events.send(IpcEvent::Changed);
        }
    }
}

/// The IPC socket server of one compositor process.
pub(crate) struct IpcServer {
    events: Sender<IpcEvent>,
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
    dispatcher: Option<JoinHandle<()>>,
    /// A duplicate of the guardian's control socket, so shutdown can end the control thread's
    /// blocking read. `None` when the session was started without a guardian.
    guardian: Option<UnixStream>,
    /// The thread that adopts the streams the guardian hands over.
    control: Option<JoinHandle<()>>,
}

impl IpcServer {
    /// Binds the socket and starts serving on two threads.
    ///
    /// The listener is bound before this returns, so a client that sees the file can
    /// connect at once and waits in the backlog until the dispatcher runs.
    ///
    /// # Errors
    ///
    /// [`IpcError::Bind`] when the socket cannot be bound or its mode cannot be set,
    /// and [`IpcError::Io`] when a thread cannot be created.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        prepared: SocketPath,
        store: SessionSync,
        events: Sender<IpcEvent>,
        received: Receiver<IpcEvent>,
        pending: Arc<AtomicBool>,
        observation: Arc<Mutex<SessionSnapshot>>,
        control: ControlHandle,
        privileged: Vec<Privileged>,
        guardian: Option<UnixStream>,
        routes: &[ExtensionRoute],
        xwayland: crate::xwayland::XwaylandReconciler,
        power: Option<Box<dyn gnoblin_model::PowerBackend>>,
    ) -> Result<IpcServer, IpcError> {
        // Each connection's frame is encoded before anything is spawned, so a bad grant
        // list fails the whole start instead of leaving threads behind.
        let no_grants: Arc<[u8]> = gnoblin_proto::encode_grants(&[]).map_err(IpcError::Grants)?.into();
        let mut frames = Vec::with_capacity(privileged.len());
        for connection in &privileged {
            let frame: Arc<[u8]> = gnoblin_proto::encode_grants(&connection.extension_grants)
                .map_err(IpcError::Grants)?
                .into();
            frames.push(frame);
        }
        let path = prepared.path().to_path_buf();
        let listener = UnixListener::bind(&path).map_err(|source| IpcError::Bind {
            path: path.clone(),
            source,
        })?;
        prepared.restrict()?;

        let stopping = Arc::new(AtomicBool::new(false));
        let readers: Arc<Mutex<BTreeMap<ClientId, JoinHandle<()>>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let clients = Arc::new(AtomicU64::new(1));
        let version = prepared.version();
        // The privileged connections get their ids before the dispatcher starts, so the
        // extension router knows each provider and grantee by the id it will serve.
        let ids: Vec<ClientId> = privileged
            .iter()
            .map(|_| ClientId::new(clients.fetch_add(1, Ordering::Relaxed)))
            .collect();
        let mut extensions = ExtensionRouter::new(routes);
        for (connection, client) in privileged.iter().zip(&ids) {
            extensions.connect(
                *client,
                &connection.role,
                connection.extension_grants.iter().map(|grant| grant.namespace.clone()),
            );
        }

        let dispatcher = Dispatcher {
            store,
            observation,
            control,
            pending,
            requests: Requests::new(),
            reload: Reload::default(),
            portal: PortalService::new(),
            connections: BTreeMap::new(),
            readers: Arc::clone(&readers),
            start: Instant::now(),
            extensions,
            xwayland,
            power,
        };
        let dispatcher = spawn("gnoblin-ipc-dispatch", move || dispatcher.run(received), &path)?;
        // The guardian hands each privileged child's stream over the control socket, one
        // handover per spawn. Each role's identity, capabilities and grants frame are fixed
        // here, before the dispatcher serves anything, so every handover of a role is served
        // as the same connection. A role the guardian never hands over is simply idle.
        let mut roles = BTreeMap::new();
        for ((connection, frame), client) in privileged.into_iter().zip(frames).zip(ids) {
            let capabilities = super::privileged::grants(&connection.role);
            roles.insert(
                connection.role,
                PrivilegedRole {
                    client,
                    capabilities,
                    frame,
                },
            );
        }
        let (guardian_clone, control_thread) = match guardian {
            Some(guardian) => {
                // The clone is what shutdown uses to end the control thread's blocking read.
                let clone = guardian.try_clone().map_err(|source| IpcError::Io {
                    path: path.clone(),
                    source,
                })?;
                let served = events.clone();
                let thread = spawn(
                    "gnoblin-ipc-control",
                    move || serve_control(guardian, roles, served, version),
                    &path,
                )?;
                (Some(clone), Some(thread))
            }
            None => {
                // Without the guardian's socket no privileged child can be handed over, so the
                // roles stay unserved. Saying so keeps a missing handover from looking like a
                // crash of the child it was meant for.
                if !roles.is_empty() {
                    log::warn!(
                        area = "ipc",
                        "no guardian control socket, so privileged roles are not served"
                    );
                }
                (None, None)
            }
        };
        let acceptor = spawn(
            "gnoblin-ipc-accept",
            {
                let events = events.clone();
                let stopping = Arc::clone(&stopping);
                let grants = Arc::clone(&no_grants);
                move || accept_loop(listener, events, stopping, readers, clients, version, grants)
            },
            &path,
        )?;
        log::info!(area = "ipc", socket = %path.display(), "listening");
        Ok(IpcServer {
            events,
            path,
            stopping,
            acceptor: Some(acceptor),
            dispatcher: Some(dispatcher),
            guardian: guardian_clone,
            control: control_thread,
        })
    }

    /// Stops serving, joins both threads and removes the socket.
    ///
    /// It is idempotent in effect — the socket is gone afterwards either way — and it
    /// never blocks on a client: every connection is shut down so its reader returns.
    pub fn shutdown(mut self) {
        // The accept loop waits in `accept`, so it is woken with a connection of its
        // own; it then sees the flag and returns without serving it.
        self.stopping.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        // Shutting the guardian's socket down is what ends the control thread's wait for
        // handovers, and with it the privileged streams it is serving. It must happen before
        // the join below, or the join would wait on a socket nothing closes.
        if let Some(guardian) = self.guardian.take() {
            let _ = guardian.shutdown(std::net::Shutdown::Both);
        }
        let _ = self.events.send(IpcEvent::Shutdown);
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
        }
        if let Some(control) = self.control.take() {
            let _ = control.join();
        }
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Accepts connections until the server stops.
fn accept_loop(
    listener: UnixListener,
    events: Sender<IpcEvent>,
    stopping: Arc<AtomicBool>,
    readers: Arc<Mutex<BTreeMap<ClientId, JoinHandle<()>>>>,
    clients: Arc<AtomicU64>,
    version: Version,
    extension_grants: Arc<[u8]>,
) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                // A listener that cannot accept is a socket this process cannot serve
                // anyone on. Saying so and returning leaves the session running, which
                // is the only choice that does not end the desktop for an IPC fault.
                log::warn!(area = "ipc", %error, "accept failed, the IPC socket is no longer served");
                return;
            }
        };
        if stopping.load(Ordering::SeqCst) {
            return;
        }
        let client = ClientId::new(clients.fetch_add(1, Ordering::Relaxed));
        let served = events.clone();
        let grants = Arc::clone(&extension_grants);
        // What this connection may do follows from its kernel peer uid, read before the stream
        // leaves this thread: the handshake answers with it, and nothing the client sends can
        // change it.
        let capabilities = super::privileged::socket_grants(&stream);
        // Holding the lock across the spawn and the insert is what lets the dispatcher
        // join a reader it has just been told about: the handle is in the map before
        // the reader can report anything.
        let mut handles = lock(&readers);
        match std::thread::Builder::new()
            .name(format!("gnoblin-ipc-{}", client.get()))
            .spawn(move || {
                serve(
                    stream,
                    client,
                    version,
                    served,
                    ConnectionPolicy {
                        capabilities,
                        extension_grants: grants,
                        ending: ConnectionEnd::Closed,
                    },
                )
            }) {
            Ok(handle) => {
                handles.insert(client, handle);
            }
            Err(error) => log::warn!(area = "ipc", %error, "a client connection could not be served"),
        }
    }
}

/// Serves one client: the handshake, then its frames, until the socket ends.
///
/// `capabilities` is what this connection was granted before it said anything, which is
/// what the compositor answers the handshake with: a connection accepted from the
/// socket is granted nothing, and only the ends the guardian passed can carry more.
///
/// Every accept is followed by `extension_grants`, the grants frame that tells the client
/// which extension namespaces it may use.
/// `ending` says what the stream's end means to the dispatcher: a client is closed, and a
/// privileged stream is lost.
fn serve(stream: UnixStream, client: ClientId, version: Version, events: Sender<IpcEvent>, policy: ConnectionPolicy) {
    let ConnectionPolicy {
        capabilities,
        extension_grants,
        ending,
    } = policy;
    let writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(error) => {
            log::debug!(area = "ipc", %error, "a connection could not be duplicated");
            return;
        }
    };
    if let Err(error) = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)) {
        log::debug!(area = "ipc", %error, "a connection could not be given a handshake deadline");
        return;
    }
    let mut reader = std::io::BufReader::new(stream);
    let hello = match read_handshake(&mut reader) {
        Some(hello) => hello,
        None => return,
    };
    let answer = match gnoblin_proto::check(version, hello.version) {
        Ok(()) => Handshake::grant(version, capabilities),
        Err(mismatch) => {
            // The refusal names this compositor's version, so the client can name both
            // and say which side to upgrade.
            log::info!(area = "ipc", %mismatch, "refused a client version");
            let _ = write_handshake(&writer, &Handshake::refuse(version));
            return;
        }
    };
    if write_accept(&writer, &answer, &extension_grants).is_err() {
        return;
    }
    // An idle subscriber waits as long as its compositor lives, so the handshake's
    // deadline is the only one this socket has.
    if let Err(error) = reader.get_ref().set_read_timeout(None) {
        log::debug!(area = "ipc", %error, "a connection could not drop its handshake deadline");
        return;
    }
    if let Err(error) = writer.set_write_timeout(Some(WRITE_TIMEOUT)) {
        log::debug!(area = "ipc", %error, "a connection could not be given a write deadline");
        return;
    }
    if events
        .send(IpcEvent::Accepted {
            client,
            writer,
            capabilities,
        })
        .is_err()
    {
        return;
    }
    loop {
        match read_frame(&mut reader) {
            Ok(Some(envelope)) => {
                if events.send(IpcEvent::Request { client, envelope }).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                // A peer's protocol error ends that connection, never the compositor.
                log::debug!(area = "ipc", %error, client = client.get(), "a client sent a frame that did not decode");
                break;
            }
        }
    }
    let _ = events.send(ending.event(client));
}

/// How a served connection ends, which decides the event the dispatcher hears last.
///
/// A client that goes away is closed. A privileged stream that goes away is lost: its role and
/// grants belong to the guardian, so the connection keeps its identity and only what the stream
/// carried ends.
#[derive(Clone, Copy)]
enum ConnectionEnd {
    Closed,
    Lost,
}

impl ConnectionEnd {
    /// The event that tells the dispatcher how the connection ended.
    fn event(self, client: ClientId) -> IpcEvent {
        match self {
            ConnectionEnd::Closed => IpcEvent::Closed { client },
            ConnectionEnd::Lost => IpcEvent::Lost { client },
        }
    }
}

/// The policy one served connection runs under: what it may do, the grants frame it is told,
/// and how its end is reported.
struct ConnectionPolicy {
    /// The capabilities the connection is granted.
    capabilities: Capabilities,
    /// The extension grants frame sent on every accept.
    extension_grants: Arc<[u8]>,
    /// What the stream's end means to the dispatcher.
    ending: ConnectionEnd,
}

/// What the control thread knows about one privileged role, fixed before serving starts.
struct PrivilegedRole {
    /// The identity every handover of the role is served as.
    client: ClientId,
    /// The capabilities the role's connection is granted on every accept.
    capabilities: Capabilities,
    /// The grants frame the role's connection carries on every accept.
    frame: Arc<[u8]>,
}

/// One privileged stream the control thread is serving, kept so a later handover of the same
/// role can replace it, and so shutdown can end it.
struct Serving {
    /// A duplicate of the stream, used to end it from the control thread.
    stream: UnixStream,
    /// The thread that reads it.
    handle: JoinHandle<()>,
}

/// Adopts every stream the guardian hands over until the control socket closes.
///
/// Each handover is a fresh pair for one spawn of a privileged child. The previous stream for the
/// role is ended and joined before the new one is served. Its `Lost` is then queued ahead of the
/// new connection's `Accepted`, so the dispatcher never mixes the two sessions, and the new peer
/// never reads a byte the old one had buffered, because it is a different socket.
fn serve_control(
    guardian: UnixStream,
    roles: BTreeMap<String, PrivilegedRole>,
    events: Sender<IpcEvent>,
    version: Version,
) {
    let mut serving: BTreeMap<ClientId, Serving> = BTreeMap::new();
    super::privileged::receive_handovers(&guardian, |role, stream| {
        let Some(declared) = roles.get(&role) else {
            // A role this session does not configure has nothing to serve, so the stream is
            // dropped and whatever holds the other end sees its peer go.
            log::warn!(area = "ipc", %role, "a handover named a role this session does not serve");
            return;
        };
        let client = declared.client;
        if let Some(old) = serving.remove(&client) {
            let _ = old.stream.shutdown(std::net::Shutdown::Both);
            let _ = old.handle.join();
        }
        let shared = match stream.try_clone() {
            Ok(shared) => shared,
            Err(error) => {
                log::warn!(
                    area = "ipc",
                    role = %role,
                    %error,
                    "a privileged connection could not be duplicated"
                );
                return;
            }
        };
        let capabilities = declared.capabilities;
        let frame = Arc::clone(&declared.frame);
        let served = events.clone();
        match std::thread::Builder::new()
            .name(format!("gnoblin-ipc-{}", client.get()))
            .spawn(move || {
                serve(
                    stream,
                    client,
                    version,
                    served,
                    ConnectionPolicy {
                        capabilities,
                        extension_grants: frame,
                        ending: ConnectionEnd::Lost,
                    },
                )
            }) {
            Ok(handle) => {
                serving.insert(client, Serving { stream: shared, handle });
            }
            Err(error) => {
                log::warn!(
                    area = "ipc",
                    role = %role,
                    %error,
                    "a privileged connection could not be served"
                )
            }
        }
    });
    // The guardian closed its socket, so the session's privileged children are gone. Their streams
    // are ended here, because nothing else will close them.
    for (_, old) in serving {
        let _ = old.stream.shutdown(std::net::Shutdown::Both);
        let _ = old.handle.join();
    }
}

/// Reads the fixed-layout handshake, or nothing when the peer never sent one.
fn read_handshake(reader: &mut std::io::BufReader<UnixStream>) -> Option<Handshake> {
    let mut frame = [0u8; gnoblin_proto::HANDSHAKE_LEN];
    let mut filled = 0;
    while filled < frame.len() {
        match io::Read::read(reader, &mut frame[filled..]) {
            Ok(0) => return None,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                log::debug!(area = "ipc", %error, "a client did not send its handshake");
                return None;
            }
        }
    }
    match Handshake::decode(&frame) {
        Ok(hello) if hello.kind == HandshakeKind::Hello => Some(hello),
        Ok(handshake) => {
            log::debug!(area = "ipc", kind = ?handshake.kind, "a connection did not start with a hello");
            None
        }
        Err(error) => {
            log::debug!(area = "ipc", %error, "a handshake did not decode");
            None
        }
    }
}

/// Writes one handshake frame.
fn write_handshake(writer: &UnixStream, handshake: &Handshake) -> io::Result<()> {
    let mut writer = writer;
    io::Write::write_all(&mut writer, &handshake.encode())?;
    io::Write::flush(&mut writer)
}

/// Writes a granted handshake, then the grants frame that follows every accept.
fn write_accept(writer: &UnixStream, handshake: &Handshake, extension_grants: &[u8]) -> io::Result<()> {
    let mut writer = writer;
    io::Write::write_all(&mut writer, &handshake.encode())?;
    io::Write::write_all(&mut writer, extension_grants)?;
    io::Write::flush(&mut writer)
}

/// What the dispatcher holds for one client.
struct Connection {
    /// The socket this connection's frames are written to.
    writer: UnixStream,
    /// What this connection may do beyond reading the graph. Empty for every
    /// connection that reached the socket.
    capabilities: Capabilities,
    /// Diff streams this connection subscribed to. They end when the connection is dropped.
    subscriptions: Vec<Subscription>,
    /// Set once a write fails, so the connection is dropped after this turn instead
    /// of being written to again.
    broken: bool,
}

impl Connection {
    /// Writes one frame, and marks the connection broken when it cannot.
    fn send(&mut self, envelope: &Envelope) {
        if self.broken {
            return;
        }
        if let Err(error) = write_frame(&mut self.writer, envelope) {
            log::debug!(area = "ipc", %error, "a connection could not be written, dropping it");
            self.broken = true;
        }
    }
}

/// When the dispatcher next owes a deadline sweep. Events that keep arriving faster than the
/// tick do not postpone it: a sweep is due once a whole tick has passed since the last one.
struct DeadlineClock {
    next: Instant,
}

impl DeadlineClock {
    fn new(now: Instant) -> Self {
        Self {
            next: now + DEADLINE_TICK,
        }
    }

    /// How long the loop may wait for an event before a sweep is owed.
    fn wait(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    /// Whether a sweep is due at `now`; when it is, the next one is scheduled a tick later.
    fn due(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + DEADLINE_TICK;
        true
    }
}

/// The one owner of the graph: everything a connection asks for runs here, in order.
struct Dispatcher {
    /// The graph owner: every mutation and every published diff goes through it.
    store: SessionSync,
    /// The last published snapshot, shared so observers can read it without entering the loop.
    observation: Arc<Mutex<SessionSnapshot>>,
    /// Hands control changes (focus, lock, idle holds) to the compositor's loop.
    control: ControlHandle,
    /// Set by an observer once it has queued a `Changed` wake, so it does not queue another
    /// while one is still pending.
    pending: Arc<AtomicBool>,
    /// The request layer: the broker, the slot registry and the last accepted
    /// configuration.
    requests: Requests,
    /// The `config.reload` requests waiting for the runtime to answer them.
    reload: Reload,
    /// The portal UI: admitted requests, their owners, and the open dialog.
    portal: PortalService,
    /// Every connection that reached the socket, keyed by client.
    connections: BTreeMap<ClientId, Connection>,
    /// Reader threads, one per client, kept so shutdown can join them.
    readers: Arc<Mutex<BTreeMap<ClientId, JoinHandle<()>>>>,
    /// When this process started serving, which is the origin of every request
    /// deadline it measures. The compositor is the only party that needs a clock here:
    /// the broker takes its tick as an argument, so the model stays replayable.
    start: Instant,
    /// The routing state of the loaded extensions: which connection serves and which may call each.
    extensions: ExtensionRouter,
    /// The X11 session the accepted document asks for, and the loop that runs it.
    xwayland: crate::xwayland::XwaylandReconciler,
    /// The power backend the power verbs reach; [`None`] refuses them `unavailable`.
    power: Option<Box<dyn gnoblin_model::PowerBackend>>,
}

impl Dispatcher {
    /// Serves events until the server is told to stop.
    fn run(mut self, events: Receiver<IpcEvent>) {
        let mut clock = DeadlineClock::new(Instant::now());
        loop {
            // The sweep is driven by the clock, not by a quiet queue: a client that keeps the
            // loop busy must not postpone every deadline it is owed.
            let event = match events.recv_timeout(clock.wait(Instant::now())) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                // Every sender is gone: nothing can ask for anything any more.
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if clock.due(Instant::now()) {
                self.sweep_deadlines();
            }
            let Some(event) = event else { continue };
            match event {
                IpcEvent::Accepted {
                    client,
                    writer,
                    capabilities,
                } => {
                    self.extensions.began(client);
                    self.connections.insert(
                        client,
                        Connection {
                            writer,
                            capabilities,
                            subscriptions: Vec::new(),
                            broken: false,
                        },
                    );
                }
                IpcEvent::Request { client, envelope } => self.request(client, &envelope),
                IpcEvent::Lost { client } => self.lost(client),
                IpcEvent::Closed { client } => self.close(client, false),
                IpcEvent::Changed => {
                    self.pending.store(false, Ordering::Release);
                    self.sync();
                }
                IpcEvent::Shutdown => break,
            }
            self.flush();
        }
        for client in self.connections.keys().copied().collect::<Vec<_>>() {
            self.close(client, true);
        }
    }

    /// Ends every request and reload whose deadline has passed, and delivers what that
    /// settles. A request that nobody answered within its deadline ends here, so the session
    /// never holds a prompt past the time its host asked for.
    fn sweep_deadlines(&mut self) {
        let now = self.now();
        let expired = self.requests.tick(now);
        let reloads = self.reload.expire(now);
        self.deliver_replies(reloads);
        let replies = self.portal.tick(now);
        self.deliver_replies(replies);
        self.pump_portal();
        self.publish_slots();
        self.deliver(expired);
        self.flush();
    }

    /// Decodes and answers one frame.
    fn request(&mut self, client: ClientId, envelope: &Envelope) {
        if envelope.namespace != wire::NAMESPACE {
            let deliveries = self.extensions.route(client, envelope);
            self.deliver(deliveries);
            return;
        }
        // The gate is before the member is looked at: a member this connection may not
        // call is `denied` whether or not this build serves it, so a client learns what
        // it is missing rather than that the member does not exist.
        if let Some(required) = required_capability(&envelope.member)
            && !self.capabilities(client).contains(required)
        {
            self.refuse(
                client,
                envelope,
                wire::code::DENIED,
                &format!(
                    "`{}` needs the `{required}` capability, which this connection does not hold",
                    envelope.member
                ),
            );
            return;
        }
        match envelope.member.as_str() {
            wire::member::SUBSCRIBE => self.subscribe(client, envelope),
            wire::member::CALL => self.call(client, envelope),
            wire::member::CLAIM => self.claim_request(client, envelope),
            wire::member::ANSWER => self.answer(client, envelope),
            wire::member::DECLINE => self.decline(client, envelope),
            wire::member::OFFER => self.offer(client, envelope),
            wire::member::SLOT_CLAIM => self.claim_slot(client, envelope),
            wire::member::SLOT_RELEASE => self.release_slot(client, envelope),
            wire::member::CONFIG_COMMIT => self.commit(client, envelope),
            wire::member::CONFIG_CURRENT => self.configuration(client, envelope),
            wire::member::CONFIG_RELOAD => self.reload(client, envelope),
            wire::member::CONFIG_REJECT => self.reject(client, envelope),
            wire::member::PORTAL_UI => self.portal_ui(client, envelope),
            wire::member::PORTAL_CANCEL => self.portal_cancel(client, envelope),
            wire::member::PORTAL_KEY => self.portal_key(client, envelope),
            wire::member::CAPTURE_SCREEN => self.capture_screen(client, envelope),
            _ => self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this connection does not serve that member",
            ),
        }
    }

    /// `portal.ui`: admit a request for the built-in dialog. A request the session
    /// cannot take answers its verdict now; an admitted one answers when it settles.
    fn portal_ui(&mut self, client: ClientId, envelope: &Envelope) {
        let request = match wire::read_portal_ui(envelope.payload().unwrap_or(&[])) {
            Ok(request) => request,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    MALFORMED,
                    &format!("the portal request did not decode: {reason}"),
                );
                return;
            }
        };
        let now = self.now();
        // Every portal dialog opens on the output. The parent is resolved only so the log
        // line can say what was found; no path places a dialog on a parent window.
        let scope = resolve_portal_scope(
            &request.caller,
            &request.parent_window,
            |handle| self.control.exported_window(handle),
            |window| self.control.window_process(window),
        );
        match &scope {
            PortalScope::Output { reason } => {
                log::warn!(area = "ipc", %reason, "a portal dialog opens on its output scope; the parent window is not used");
            }
        }
        match self.portal.submit(client, envelope.request_id, &request, now) {
            Ok(replies) => self.deliver_replies(replies),
            Err(refused) => self.refuse(client, envelope, refused.code, &refused.message),
        }
    }

    /// `portal.cancel`: the connection withdraws the portal requests it holds. Each
    /// withdrawn request settles `cancelled(user)` to the requester before the ack.
    fn portal_cancel(&mut self, client: ClientId, envelope: &Envelope) {
        match self.portal.cancel(client) {
            Ok(replies) => {
                self.deliver_replies(replies);
                self.reply(client, envelope);
            }
            Err(refused) => self.refuse(client, envelope, refused.code, &refused.message),
        }
    }

    /// `portal.key`: one key for the open portal dialog.
    fn portal_key(&mut self, client: ClientId, envelope: &Envelope) {
        let (key, repeat) = match wire::read_portal_key(envelope.payload().unwrap_or(&[])) {
            Ok(key) => key,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    MALFORMED,
                    &format!("the portal key did not decode: {reason}"),
                );
                return;
            }
        };
        match self.portal.key(&key, repeat, self.now()) {
            Ok(replies) => {
                self.reply(client, envelope);
                self.deliver_replies(replies);
            }
            Err(refused) => self.refuse(client, envelope, refused.code, &refused.message),
        }
    }

    /// Writes portal replies, each to the connection it is addressed to.
    fn deliver_replies(&mut self, replies: Vec<(ClientId, Envelope)>) {
        for (client, envelope) in replies {
            if let Some(connection) = self.connections.get_mut(&client) {
                connection.send(&envelope);
            }
        }
    }

    /// What this connection was granted.
    fn capabilities(&self, client: ClientId) -> Capabilities {
        self.connections
            .get(&client)
            .map_or(Capabilities::NONE, |connection| connection.capabilities)
    }

    /// The layer's clock, in the tick the model measures deadlines in.
    fn now(&self) -> Tick {
        Tick::new(u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    /// Write every frame the request layer produced, in order.
    fn deliver(&mut self, deliveries: Vec<FrameDelivery>) {
        for delivery in deliveries {
            if let Some(connection) = self.connections.get_mut(&delivery.client) {
                connection.send(&delivery.envelope);
            }
        }
    }

    /// Publish the slot registry when a claim changed it.
    ///
    /// The properties are the schema's `session.slot_*`, and they are written only when
    /// the value changed: `Graph::set` publishes a diff for every write, so a repeat
    /// would wake every subscriber for nothing.
    fn publish_slots(&mut self) {
        let Some(fields) = self.requests.publication() else {
            return;
        };
        if let Err(error) = self.store.publish_session(&fields) {
            log::warn!(area = "ipc", %error, "the object graph refused a slot publication");
        }
    }

    /// `request.claim`: one owner takes an offered request.
    fn claim_request(&mut self, client: ClientId, envelope: &Envelope) {
        let request = match wire::read_claim(envelope.payload().unwrap_or(&[])) {
            Ok(request) => request,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the claim payload did not decode: {reason}"),
                );
                return;
            }
        };
        match self.requests.claim_request(client, request) {
            Ok(deliveries) => {
                self.reply(client, envelope);
                self.deliver(deliveries);
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `request.answer`: the provider decided, and here is what it decided.
    fn answer(&mut self, client: ClientId, envelope: &Envelope) {
        let (request, mut answer) = match wire::read_answer(envelope.payload().unwrap_or(&[])) {
            Ok(answer) => answer,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the answer payload did not decode: {reason}"),
                );
                return;
            }
        };
        // The store takes the bytes over, so the decoded wrapper is left empty.
        match self.requests.answer(client, request, std::mem::take(&mut *answer)) {
            Ok(deliveries) => {
                self.reply(client, envelope);
                self.deliver(deliveries);
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `request.decline`: the provider will not answer, which is its own outcome.
    fn decline(&mut self, client: ClientId, envelope: &Envelope) {
        let (request, reason) = match wire::read_decline(envelope.payload().unwrap_or(&[])) {
            Ok(decoded) => decoded,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the decline payload did not decode: {reason}"),
                );
                return;
            }
        };
        match self.requests.decline(client, request, reason) {
            Ok(deliveries) => {
                self.reply(client, envelope);
                self.deliver(deliveries);
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `request.offer`: a privileged host asks a provider to decide something.
    fn offer(&mut self, client: ClientId, envelope: &Envelope) {
        let (slot, title, message, subject, timeout) = match wire::read_offer(envelope.payload().unwrap_or(&[])) {
            Ok(offer) => offer,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the offer payload did not decode: {reason}"),
                );
                return;
            }
        };
        // The capability depends on the slot, so the table above cannot name it. The relay
        // holds `RetrieveSecret` and may offer `secret.derive` only; every other slot needs
        // `OfferRequest`.
        let required = if slot == gnoblin_model::CapabilitySlot::SecretDerive.name() {
            Capability::RetrieveSecret
        } else {
            Capability::OfferRequest
        };
        if !self.capabilities(client).contains(required) {
            self.refuse(
                client,
                envelope,
                wire::code::DENIED,
                &format!("offering `{slot}` needs the `{required}` capability, which this connection does not hold"),
            );
            return;
        }
        // The relay asks for an app's secret, but only the compositor, which showed the
        // consent dialog, may say the user allowed it. Without a grant the offer never leaves.
        if slot == gnoblin_model::CapabilitySlot::SecretDerive.name() {
            let granted = std::str::from_utf8(&subject).is_ok_and(|app| self.portal.secret_granted(app));
            if !granted {
                self.refuse(
                    client,
                    envelope,
                    wire::code::DENIED,
                    "the user has not allowed this app its secret in this session",
                );
                return;
            }
        }
        let deadline = self.now().saturating_add(timeout);
        match self.requests.offer(client, &slot, &title, &message, &subject, deadline) {
            Ok((request, deliveries)) => {
                let request = request.get();
                if let Some(connection) = self.connections.get_mut(&client) {
                    connection.send(&Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::OFFER,
                        envelope.request_id,
                        request.to_be_bytes().to_vec(),
                    ));
                }
                self.deliver(deliveries);
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `slot.claim`: a shell takes a slot over, or the services role publishes its own.
    fn claim_slot(&mut self, client: ClientId, envelope: &Envelope) {
        let (slot, provider) = match wire::read_slot_claim(envelope.payload().unwrap_or(&[])) {
            Ok(claim) => claim,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the slot claim payload did not decode: {reason}"),
                );
                return;
            }
        };
        let slot = match gnoblin_model::CapabilitySlot::from_str(&slot) {
            Ok(slot) => slot,
            Err(error) => {
                self.refuse(client, envelope, wire::code::UNKNOWN_SLOT, &error.to_string());
                return;
            }
        };
        // Only the services role holds `OfferRequest`, so it is the role that may claim the
        // internal `secret.derive` slot.
        let services_role = self.capabilities(client).contains(Capability::OfferRequest);
        match self.requests.claim_slot(client, slot, provider, services_role) {
            Ok(deliveries) => {
                self.reply(client, envelope);
                self.deliver(deliveries);
                self.publish_slots();
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `slot.release`: the claiming connection gives a slot back.
    fn release_slot(&mut self, client: ClientId, envelope: &Envelope) {
        let name = match wire::read_slot_release(envelope.payload().unwrap_or(&[])) {
            Ok(name) => name,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the slot release payload did not decode: {reason}"),
                );
                return;
            }
        };
        let slot = match gnoblin_model::CapabilitySlot::from_str(&name) {
            Ok(slot) => slot,
            Err(error) => {
                self.refuse(client, envelope, wire::code::UNKNOWN_SLOT, &error.to_string());
                return;
            }
        };
        match self.requests.release_slot(client, slot) {
            Ok(deliveries) => {
                self.reply(client, envelope);
                self.deliver(deliveries);
                self.publish_slots();
            }
            Err(refusal) => self.refuse(client, envelope, refusal.code, &refusal.message),
        }
    }

    /// `config.commit`: the runtime committed a validated document.
    fn commit(&mut self, client: ClientId, envelope: &Envelope) {
        let document = match wire::read_config_commit(envelope.payload().unwrap_or(&[])) {
            Ok(document) => document,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the configuration payload did not decode: {reason}"),
                );
                return;
            }
        };
        // A document whose window rules this session cannot carry out is refused before it is
        // recorded, so the last accepted document stays in force and the caller sees the field.
        let config = gnoblin_config::Config::from_json_bytes(&document);
        let rules = match config
            .as_ref()
            .map(|config| crate::window_rules::convert(&config.window_rules, &config.workspaces))
        {
            Ok(Ok(rules)) => Some(rules),
            Ok(Err(message)) => {
                log::warn!(area = "ipc", %message, "the committed window rules were refused; the last accepted document stays in force");
                self.refuse(client, envelope, wire::code::CONFIG_REJECTED, &message);
                let answers = self.reload.finish(&ReloadVerdict::Rejected { message });
                self.deliver_replies(answers);
                return;
            }
            // A document that did not decode keeps the rules already in force.
            Err(_) => None,
        };
        // The X11 session the document asks for is worked out before the document is recorded, so
        // settings this process cannot run are refused and the last accepted document stays in force.
        let xwayland = match config.as_ref().map(|config| self.xwayland.desired(config)) {
            Ok(Ok(desired)) => Some(desired),
            Ok(Err(message)) => {
                log::warn!(area = "ipc", %message, "the committed xwayland settings were refused; the last accepted document stays in force");
                self.refuse(client, envelope, wire::code::CONFIG_REJECTED, &message);
                let answers = self.reload.finish(&ReloadVerdict::Rejected { message });
                self.deliver_replies(answers);
                return;
            }
            // A document that did not decode keeps the X11 session already in force.
            Err(_) => None,
        };
        // The request layer records the document before it is parsed, so the revision counts
        // every commit the runtime made, including one this process cannot decode. The
        // settings object then keeps its last good values.
        let revision = self.requests.commit(document);
        match config {
            Ok(config) => {
                Self::persist_last_good(&config);
                self.store.publish_settings(&config);
                self.portal.configure(&config);
                // Reduced motion turns animations off in the compositor loop. A document that
                // does not name the key is not reduced, so it animates as before.
                let reduced_motion = config
                    .appearance
                    .as_ref()
                    .and_then(|appearance| appearance.reduced_motion)
                    .unwrap_or(false);
                self.control.set_reduced_motion(reduced_motion);
                // The output scale is re-derived from the accepted document, so a scale set in
                // init.lua after the session started takes effect on this commit.
                self.control
                    .set_global_scale(crate::monitors::settings_of(&config).global_scale);
                if let Some(rules) = rules {
                    self.control.set_window_rules(rules);
                }
                // The X11 session follows the accepted document. An unchanged session is left running; a
                // changed one restarts the server only when no X11 client is connected, else it is refused.
                if let Some(desired) = xwayland
                    && let Err(message) = self.xwayland.apply(&self.control, revision, desired)
                {
                    log::warn!(area = "xwayland", %message, "the accepted xwayland settings were not applied; the running X11 session stays");
                }
                // The keyboard repeat and layouts are applied from the accepted document. A section the
                // document does not name keeps what the seat already has, as at startup.
                if let Some(keyboard) = config.input.as_ref().and_then(|input| input.keyboard.clone()) {
                    self.control.set_keyboard_settings(keyboard);
                }
                if let Some(sources) = config.input_sources.clone() {
                    self.control.set_input_sources(sources);
                }
                // The cursor is re-derived from the accepted document, as at startup: a document with no
                // `cursor` section gets the defaults, so removing the section restores them.
                self.control.set_cursor_settings(crate::cursor::cursor_settings(
                    config.cursor.clone().unwrap_or_default(),
                ));
                // The magnifier and colour filter are re-derived the same way, so removing the
                // `accessibility` section turns them off on this commit.
                let accessibility = config.accessibility.unwrap_or_default();
                self.control
                    .set_visual_accessibility(crate::accessibility::visual_accessibility_settings(&accessibility));
                // The visual bell is re-derived the same way, so removing the `compositor` section
                // turns the flash off on this commit.
                self.control.set_visual_bell(crate::bell::visual_bell_settings(
                    &config.compositor.unwrap_or_default(),
                ));
            }
            Err(reason) => {
                log::warn!(area = "ipc", revision, %reason, "the committed configuration did not decode; the settings object keeps its last values");
                self.xwayland.report(revision);
            }
        }
        log::info!(
            area = "ipc",
            revision,
            "the runtime committed a configuration document; it stays in force until the next one"
        );
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                wire::member::CONFIG_COMMIT,
                envelope.request_id,
                revision.to_be_bytes().to_vec(),
            ));
        }
        // A commit made while a reload waits is the runtime's answer to that reload.
        let answers = self.reload.finish(&ReloadVerdict::Accepted { revision });
        self.deliver_replies(answers);
    }

    /// Save the accepted document where the services role reads it at start. The services role
    /// decides `keyring.enabled` and `prompts.enabled` from that file before the runtime has
    /// committed anything, so a commit that is not saved is invisible to them. A failed save only
    /// means a restart cannot recover this document; the session keeps running on it.
    fn persist_last_good(config: &gnoblin_config::Config) {
        let Some(store) = gnoblin_config::Store::from_env() else {
            return;
        };
        let path = match gnoblin_config::locate(&gnoblin_config::SearchPaths::from_env()) {
            Ok(path) => path,
            Err(error) => {
                log::debug!(area = "ipc", %error, "no configuration file to record as last good");
                return;
            }
        };
        if let Err(error) = store.save(path.path(), config) {
            log::warn!(area = "ipc", %error, "the accepted configuration could not be saved as last good");
        }
    }

    /// `config.reload`: a privileged caller asks the runtime to re-read its document.
    ///
    /// The compositor does not parse the document, so the answer waits on the runtime,
    /// which replies with `config.commit` or `config.reject` on its own connection. With
    /// no runtime connected nothing can answer, so the request is refused now rather than
    /// held until its deadline.
    fn reload(&mut self, client: ClientId, envelope: &Envelope) {
        let runtime_connected = self
            .connections
            .iter()
            .any(|(other, connection)| *other != client && connection.capabilities.contains(Capability::CommitConfig));
        if !runtime_connected {
            self.refuse(
                client,
                envelope,
                wire::code::NO_RUNTIME,
                "no runtime role is connected, so nothing can reload the configuration",
            );
            return;
        }
        let now = self.now();
        if let Some(generation) = self.reload.request(client, envelope.request_id, now) {
            self.store.request_reload(generation);
        }
    }

    /// `config.reject`: the runtime refused the document it was asked to reload.
    ///
    /// Every caller waiting on that reload is answered with the runtime's diagnostic. The
    /// last accepted document stays in force, because nothing here replaces it.
    fn reject(&mut self, client: ClientId, envelope: &Envelope) {
        let message = match wire::read_config_reject(envelope.payload().unwrap_or(&[])) {
            Ok(message) => message,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the rejection did not decode: {reason}"),
                );
                return;
            }
        };
        log::warn!(
            area = "ipc",
            %message,
            "the runtime refused the configuration reload; the last accepted document stays in force"
        );
        let answers = self.reload.finish(&ReloadVerdict::Rejected {
            message: message.clone(),
        });
        self.deliver_replies(answers);
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                wire::member::CONFIG_REJECT,
                envelope.request_id,
                Vec::new(),
            ));
        }
    }

    /// `config.current`: the last document this process accepted.
    fn configuration(&mut self, client: ClientId, envelope: &Envelope) {
        let (revision, document) = self.requests.configuration();
        let payload = wire::configuration_payload(revision, document);
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                wire::member::CONFIG_CURRENT,
                envelope.request_id,
                payload,
            ));
        }
    }

    /// Answers `capture.screen` with one PNG of the first output.
    ///
    /// The lock is asked first: a locked or embargoed session is refused before any pixel
    /// is read. The picture comes from the backend's screenshot, which applies the
    /// configured secure treatment to secure layers, so a masked or omitted layer never
    /// reaches the PNG.
    fn capture_screen(&mut self, client: ClientId, envelope: &Envelope) {
        // Headroom for the envelope around the payload, so the reply still fits one frame.
        const ENVELOPE_HEADROOM: usize = 4096;
        let snapshot = match self.observation.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let output = match still::choose_output(snapshot.lock_active(), snapshot.capture_allowed(), snapshot.outputs())
        {
            Ok(output) => output,
            Err(refusal) => {
                let (code, message) = capture_refusal(refusal);
                return self.refuse(client, envelope, code, message);
            }
        };
        let Some(image) = self.control.screenshot(output) else {
            let (code, message) = capture_refusal(Refusal::Unavailable);
            return self.refuse(client, envelope, code, message);
        };
        let max_len = gnoblin_proto::MAX_FRAME_LEN - ENVELOPE_HEADROOM;
        match still::encode_png(image.size(), image.stride(), image.data(), max_len) {
            Ok(png) => {
                if let Some(connection) = self.connections.get_mut(&client) {
                    connection.send(&Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::CAPTURE_SCREEN,
                        envelope.request_id,
                        png,
                    ));
                }
            }
            Err(refusal) => {
                let (code, message) = capture_refusal(refusal);
                self.refuse(client, envelope, code, message);
            }
        }
    }

    /// Answer a member that succeeded and has nothing to carry.
    fn reply(&mut self, client: ClientId, envelope: &Envelope) {
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                envelope.member.clone(),
                envelope.request_id,
                Vec::new(),
            ));
        }
    }

    /// `object.subscribe`: the snapshot now, every later diff in order.
    fn subscribe(&mut self, client: ClientId, envelope: &Envelope) {
        let Some(connection) = self.connections.get_mut(&client) else {
            return;
        };
        let filter = match wire::read_subscribe(envelope.payload().unwrap_or(&[])) {
            Ok(filter) => filter,
            Err(reason) => {
                let refusal = refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the subscribe payload did not decode: {reason}"),
                );
                connection.send(&refusal);
                return;
            }
        };
        let subscription = self.store.subscribe(filter);
        // Queueing the snapshot before the subscription is registered is what makes
        // the late-subscriber rule hold on the wire: no diff can precede it.
        connection.send(&Envelope::reply(
            wire::NAMESPACE,
            wire::member::SUBSCRIBE,
            envelope.request_id,
            wire::snapshot_payload(subscription.snapshot()),
        ));
        connection.subscriptions.push(subscription);
    }

    /// `object.call`: the operation now, its one completion when it has happened.
    fn call(&mut self, client: ClientId, envelope: &Envelope) {
        let (object, method, arguments) = match wire::read_call(envelope.payload().unwrap_or(&[])) {
            Ok(call) => call,
            Err(reason) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    &format!("the call payload did not decode: {reason}"),
                );
                return;
            }
        };
        let ty = self
            .store
            .graph()
            .object(object)
            .map(|live| live.ty().name().to_owned());
        let window = self.store.window_of(object);
        let workspace = self.store.workspace_of(object);
        let output = self.store.output_of(object);
        match (ty, window, workspace, output) {
            (Some(ty), Some(window), _, _) if ty == "window" && method == "focus" => {
                self.focus(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _) if ty == "window" && method == "present" => {
                self.present_call(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _) if ty == "window" && method == "request_menu" => {
                self.request_menu_call(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _) if ty == "window" && method == "kill" => {
                self.kill(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _)
                if ty == "window"
                    && matches!(
                        method.as_str(),
                        "close"
                            | "set_minimized"
                            | "set_maximized"
                            | "set_fullscreen"
                            | "move"
                            | "resize"
                            | "set_workspace"
                            | "set_output"
                    ) =>
            {
                self.window_control_call(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _)
                if ty == "window"
                    && matches!(
                        method.as_str(),
                        "set_opacity" | "set_scale" | "set_offset" | "reset_visuals"
                    ) =>
            {
                self.window_visual_call(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), Some(window), _, _) if ty == "window" && method == "add_effect" => {
                self.add_effect_call(client, envelope, object, &method, &arguments, window);
            }
            (Some(ty), _, Some(workspace), _) if ty == "workspace" && method == "set_layout" => {
                self.set_workspace_layout(client, envelope, object, &method, &arguments, workspace);
            }
            (Some(ty), _, Some(workspace), _) if ty == "workspace" && method == "layout_command" => {
                self.layout_command_call(client, envelope, object, &method, &arguments, workspace);
            }
            (Some(ty), _, _, Some((_, name))) if ty == "output" && method == "set_layout" => {
                self.set_output_layout(client, envelope, object, &method, &arguments, &name);
            }
            (Some(ty), _, _, _) if ty == "lock" && method == "lock_session" => {
                self.lock_call(client, envelope, object, &method, &arguments);
            }
            (Some(ty), _, _, _) if ty == "session" && method == "logout" => {
                self.session_call(client, envelope, object, &method, &arguments);
            }
            (Some(ty), _, _, _) if ty == "session" && power_verb(&method).is_some() => {
                if let Some(verb) = power_verb(&method) {
                    self.power_call(client, envelope, object, verb, &arguments);
                }
            }
            (Some(ty), _, _, _) if ty == "idle" && (method == "inhibit" || method == "uninhibit") => {
                self.idle_call(client, envelope, object, &method, &arguments);
            }
            (Some(ty), _, _, _) if ty == "workspace_set" => {
                self.workspace_set_call(client, envelope, object, &method, &arguments);
            }
            (Some(ty), _, _, _) if ty == "layers" => {
                self.layers_call(client, envelope, object, &method, &arguments);
            }
            (None, _, _, _) => self.refuse(
                client,
                envelope,
                wire::code::OBJECT_UNKNOWN,
                &format!("no live object has identity {}", object.get()),
            ),
            (Some(_), None, None, None) => self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this object is not a window, workspace or output, and this build serves no method on it",
            ),
            (Some(_), _, _, _) => self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this build does not serve that method on that object",
            ),
        }
    }

    /// Serves `window.focus`: the graph issues the operation, the loop decides.
    fn focus(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: gnoblin_types::WindowId,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                wire::member::CALL,
                envelope.request_id,
                operation.id().get().to_be_bytes().to_vec(),
            ));
        }
        // Focus policy decides, and it decides in the loop: the answer is whether the
        // window holds focus now, so the diff this caused is published before the
        // completion reaches the client.
        let granted = self.control.focus(window);
        self.sync();
        let outcome = match granted {
            // The window closed while the request was on its way, so the call has no
            // window to have focused. The close may already have finished the
            // operation with the same code, which the graph then refuses.
            Some(_) if self.store.graph().object(object).is_none() => Some(OperationOutcome::Failed(OBJECT_GONE)),
            // `Some(false)` is the policy declining, which the client learns from the
            // `focused` property rather than from a failed operation: the request was
            // carried out, the answer was no.
            Some(_) => Some(OperationOutcome::Done),
            // The loop did not answer, which means it is stopping. The connection ends
            // with the process, so the operation is left pending rather than given an
            // outcome this server would have invented.
            None => None,
        };
        if let Some(outcome) = outcome {
            let _ = self.store.graph_mut().complete(operation.id(), outcome);
        }
    }

    /// Serves `window.present`: the loop decides whether the window may take focus now, and
    /// the operation completes `done` only once the window holds focus. A closed window fails
    /// `not-found`, a refused or locked session `denied`, and a window that never became
    /// shown or stayed under input before about [`PRESENT_DEADLINE`] fails `busy`. A single
    /// ask may itself wait up to the control timeout, so the bound is a start deadline.
    fn present_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: gnoblin_types::WindowId,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let deadline = Instant::now() + PRESENT_DEADLINE;
        let outcome = loop {
            // Check before asking, not only after an answer: a slow ask must not start past
            // the deadline and push the whole call past the advertised five seconds.
            if Instant::now() >= deadline {
                break OperationOutcome::Failed(BUSY);
            }
            let answer = self.control.present(window);
            self.sync();
            match answer {
                // The window closed while the request waited, so there is nothing to focus.
                Some(PresentAnswer::Gone) => break OperationOutcome::Failed(NOT_FOUND),
                Some(PresentAnswer::Locked | PresentAnswer::Focused(false)) => {
                    break OperationOutcome::Failed(DENIED);
                }
                Some(PresentAnswer::Focused(true)) => break OperationOutcome::Done,
                // The window is not shown yet, or the user is typing: ask again until the
                // deadline, then give up rather than steal focus mid-keystroke.
                Some(PresentAnswer::NotShown | PresentAnswer::Typing) => {
                    if Instant::now() >= deadline {
                        break OperationOutcome::Failed(BUSY);
                    }
                    std::thread::sleep(PRESENT_RETRY);
                }
                // The loop did not answer, which means it is stopping; the operation stays
                // pending rather than being given an outcome this server would invent.
                None => return,
            }
        };
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves `session.logout`: the loop is asked to stop and the session ends as a logout. The
    /// operation completes `done` once the request is taken. The socket may close before the
    /// caller reads the reply, because the session is ending.
    fn session_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        self.control.logout();
        self.finish(object, &operation, Some(OperationOutcome::Done));
    }

    /// Runs a power verb through the backend this session was started with. The call is
    /// recorded as an operation like the others: the backend's answer completes it, and a
    /// refusal reaches the caller under the code the error carries.
    fn power_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        verb: gnoblin_model::PowerVerb,
        arguments: &gnoblin_model::Fields,
    ) {
        let operation = match self.store.graph_mut().call(object, verb.name(), arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let outcome = match self.power.as_mut() {
            Some(backend) => match backend.perform(verb) {
                Ok(()) => OperationOutcome::Done,
                Err(error) => {
                    log::warn!(area = "ipc", %error, "power verb refused");
                    OperationOutcome::Failed(error.code())
                }
            },
            None => OperationOutcome::Failed(POWER_UNAVAILABLE),
        };
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves `lock.lock_session`: the loop locks the session, and the operation completes `done`
    /// only once the cover has proven `locked`. A refusal, a failed cover, or no
    /// confirmation within the loop's bound completes `not-locked`, so a caller never takes
    /// a queued request for a protected screen.
    fn lock_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        // The loop answers `Some(true)` once the cover proves `locked`, and `Some(false)` on a
        // refusal or a failed cover. `None` is the bound: a lock that never confirms fails here
        // rather than leaving the caller waiting on a screen that may not be covered.
        let confirmed = self.control.lock();
        self.sync();
        let outcome = match confirmed {
            Some(true) => OperationOutcome::Done,
            Some(false) | None => OperationOutcome::Failed(NOT_LOCKED),
        };
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves `idle.inhibit` and `idle.uninhibit`: one named hold on idle notifications is
    /// added or removed. The hold is recorded on the compositor's idle tables, so the next
    /// snapshot publishes the new `inhibitors` count.
    fn idle_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let outcome = match text_argument(arguments, "holder") {
            Some(holder) => match self.control.idle_hold(&holder, method == "inhibit") {
                Some(Ok(_)) => OperationOutcome::Done,
                Some(Err(_)) => OperationOutcome::Failed(TOO_MANY_INHIBITORS),
                None => OperationOutcome::Failed(IDLE_UNAVAILABLE),
            },
            None => OperationOutcome::Failed(IDLE_UNAVAILABLE),
        };
        self.sync();
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves `layers.register`: a trusted built-in registers a window-scope layer above a
    /// window. The provider is the compositor's own, derived here from the connection, so no
    /// name from the wire can choose it. A connection without `register-layer` is refused
    /// `power-missing` before the graph sees the call.
    fn layers_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) {
        if !self.capabilities(client).contains(Capability::RegisterLayer) {
            self.refuse(
                client,
                envelope,
                "power-missing",
                &format!(
                    "`{}` needs the `register-layer` capability, which this connection does not hold",
                    envelope.member
                ),
            );
            return;
        }
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let outcome = self.register_layer_outcome(arguments);
        self.sync();
        self.finish(object, &operation, Some(outcome));
    }

    /// Registers the layer `layers.register` asked for and names the result as an outcome.
    /// A missing argument, a name the provider may not use, and a refused registration all
    /// complete `layer-refused`; the reason is logged here, since the code names no rule.
    fn register_layer_outcome(&mut self, arguments: &gnoblin_model::Fields) -> OperationOutcome {
        let (Some(local), Some(target)) = (text_argument(arguments, "local"), uint_argument(arguments, "target"))
        else {
            return OperationOutcome::Failed(LAYER_REFUSED);
        };
        let provider = LayerProvider::Gnoblin(COMPOSITOR_CRATE);
        let local = match provider.resolve_local(&local) {
            Ok(local) => local,
            Err(error) => {
                log::warn!(area = "layers", ?error, "a built-in layer name was refused");
                return OperationOutcome::Failed(LAYER_REFUSED);
            }
        };
        let spec = LayerSpec::new(local, LayerScope::Window(Some(WindowId::new(target))), Band::Windows);
        match self.control.register_layer(&provider, spec) {
            Some(Ok(_)) => OperationOutcome::Done,
            Some(Err(reason)) => {
                log::warn!(area = "layers", %reason, "the compositor refused a layer");
                OperationOutcome::Failed(LAYER_REFUSED)
            }
            None => {
                log::warn!(area = "layers", "the compositor did not answer a layer registration");
                OperationOutcome::Failed(LAYER_REFUSED)
            }
        }
    }

    /// Serves `window.kill`: the window's client process is sent SIGKILL, whatever its
    /// state, so an unresponsive window is not the only one that can be killed. Refused
    /// when the window is gone, its process is unknown or its client has disconnected,
    /// the pid now names another process than the one that connected, or the pid is the
    /// compositor's own or init's, since those are never a client.
    fn kill(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: gnoblin_types::WindowId,
    ) {
        let identity = match self.control.window_process(window) {
            None => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::OBJECT_UNKNOWN,
                    "kill: the compositor did not answer in time",
                );
                return;
            }
            Some(None) => {
                self.refuse(client, envelope, wire::code::OBJECT_UNKNOWN, "kill: window not found");
                return;
            }
            Some(Some(WindowProcess::Unknown)) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "kill: the window's pid is unknown",
                );
                return;
            }
            Some(Some(WindowProcess::Disconnected)) => {
                self.refuse(
                    client,
                    envelope,
                    wire::code::PROCESS_CHANGED,
                    "kill: the window's client has disconnected",
                );
                return;
            }
            Some(Some(WindowProcess::Connected(identity))) => identity,
        };
        if identity.pid == std::process::id() || identity.pid == 1 {
            self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "kill: refuses the compositor's own process and init",
            );
            return;
        }
        // The pidfd is opened before the identity is checked. Once it is open it names the
        // process that holds the pid, and the check shows whether that process is the one
        // that connected: a pid reused since then fails the check.
        let pidfd = i32::try_from(identity.pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .and_then(|pid| rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).ok());
        let current = ProcessIdentity::of(identity.pid);
        let Some(pidfd) = pidfd.filter(|_| current == Some(identity)) else {
            self.refuse(
                client,
                envelope,
                wire::code::PROCESS_CHANGED,
                "kill: the window's process has exited or changed",
            );
            return;
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        // The signal goes through the pidfd, so it cannot land on a process that took the
        // pid after the check; a process that has already exited fails it instead.
        let signalled = rustix::process::pidfd_send_signal(&pidfd, rustix::process::Signal::KILL);
        let outcome = match signalled {
            Ok(()) => OperationOutcome::Done,
            Err(_) => OperationOutcome::Failed(OBJECT_GONE),
        };
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves the window verbs that change a window's state, place or workspace: `close`,
    /// `set_minimized`, `set_maximized`, `set_fullscreen`, `move`, `resize`, `set_workspace`
    /// and `set_output`. The loop applies the change through the window manager, so the
    /// constraints a drag obeys apply to these too. The operation completes `done` only when
    /// the change was made; a refusal completes with the reason the window manager gave. A
    /// connection without `window-control` is refused `power-missing` before the graph sees
    /// the call, and arguments that name no change are refused before an operation exists.
    fn window_control_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: WindowId,
    ) {
        if !self.capabilities(client).contains(Capability::WindowControl) {
            self.refuse(
                client,
                envelope,
                "power-missing",
                &format!("`{method}` needs the `window-control` capability, which this connection does not hold"),
            );
            return;
        }
        let change = match self.window_change(method, arguments) {
            Ok(change) => change,
            Err((code, message)) => {
                self.refuse(client, envelope, code, &message);
                return;
            }
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        // The change is made in the loop, so the diff it caused is published before the
        // completion reaches the client.
        let applied = self.control.window_control(window, change);
        self.sync();
        let outcome = match applied {
            Some(Ok(())) => Some(OperationOutcome::Done),
            // The window manager's reason becomes the completion's code. `finish` still maps
            // a gone object to `object-gone` itself.
            Some(Err(refusal)) => Some(OperationOutcome::Failed(window_refusal_code(refusal))),
            // The loop did not answer, which means it is stopping: the operation is left
            // pending rather than given an outcome this server would have invented.
            None => None,
        };
        self.finish(object, &operation, outcome);
    }

    /// Serves the visual verbs that change how a window is drawn: `set_opacity`, `set_scale`,
    /// `set_offset` and `reset_visuals`. The frame, workspace, focus and what the application
    /// sees do not change. The loop applies the value to the window's own shell visuals, so a
    /// window rule's opacity and the placement animations keep applying alongside them. The
    /// operation completes `done` only when the value was applied; a refusal completes with its
    /// reason. A connection without `window-control` is refused `power-missing` before the graph
    /// sees the call, and arguments that are not numbers are refused before an operation exists.
    fn window_visual_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: WindowId,
    ) {
        if !self.capabilities(client).contains(Capability::WindowControl) {
            self.refuse(
                client,
                envelope,
                "power-missing",
                &format!("`{method}` needs the `window-control` capability, which this connection does not hold"),
            );
            return;
        }
        let change = match Self::visual_change(method, arguments) {
            Ok(change) => change,
            Err((code, message)) => {
                self.refuse(client, envelope, code, &message);
                return;
            }
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        // The drawing changes in the loop, so the diff it caused is published before the
        // completion reaches the client.
        let applied = self.control.window_visual(window, change);
        self.sync();
        let outcome = match applied {
            Some(Ok(())) => Some(OperationOutcome::Done),
            Some(Err(refusal)) => Some(OperationOutcome::Failed(visual_refusal_code(refusal))),
            // The loop did not answer, so it is stopping: the operation is left pending rather
            // than given an outcome this server would have invented.
            None => None,
        };
        self.finish(object, &operation, outcome);
    }

    /// Reads a visual verb's arguments into the value the loop applies. The error is the
    /// refusal code and message for an argument that is missing or not a number.
    fn visual_change(method: &str, arguments: &gnoblin_model::Fields) -> Result<WindowVisual, (&'static str, String)> {
        let missing = |needs: &str| (wire::code::UNKNOWN_MEMBER, format!("`{method}` needs {needs}"));
        Ok(match method {
            "set_opacity" => WindowVisual::Opacity(
                double_argument(arguments, "opacity").ok_or_else(|| missing("its number `opacity`"))?,
            ),
            "set_scale" => {
                WindowVisual::Scale(double_argument(arguments, "scale").ok_or_else(|| missing("its number `scale`"))?)
            }
            "set_offset" => match (double_argument(arguments, "x"), double_argument(arguments, "y")) {
                (Some(x), Some(y)) => WindowVisual::Offset { x, y },
                _ => return Err(missing("numbers `x` and `y`")),
            },
            "reset_visuals" => WindowVisual::Reset,
            _ => return Err(missing("a visual verb this server serves")),
        })
    }

    /// Reads a window verb's arguments into the change the loop applies. The error is the
    /// refusal code and message for arguments that name no change, or for a workspace or
    /// output object the session does not declare.
    fn window_change(
        &self,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) -> Result<WindowChange, (&'static str, String)> {
        let missing = |needs: &str| (wire::code::UNKNOWN_MEMBER, format!("`{method}` needs {needs}"));
        let flag = bool_argument(arguments, "value").ok_or_else(|| missing("its boolean `value`"));
        Ok(match method {
            "close" => WindowChange::Close,
            "set_minimized" => WindowChange::Minimised(flag?),
            "set_maximized" => WindowChange::Maximised(flag?),
            "set_fullscreen" => WindowChange::Fullscreen(flag?),
            "move" => {
                let x = int_argument(arguments, "x").and_then(|x| i32::try_from(x).ok());
                let y = int_argument(arguments, "y").and_then(|y| i32::try_from(y).ok());
                match (x, y) {
                    (Some(x), Some(y)) => WindowChange::MoveTo { x, y },
                    _ => return Err(missing("integer `x` and `y`")),
                }
            }
            "resize" => {
                let width = uint_argument(arguments, "width").and_then(|w| u32::try_from(w).ok());
                let height = uint_argument(arguments, "height").and_then(|h| u32::try_from(h).ok());
                match (width, height) {
                    (Some(width), Some(height)) => WindowChange::Resize { width, height },
                    _ => return Err(missing("unsigned `width` and `height`")),
                }
            }
            "set_workspace" => {
                let object =
                    object_argument(arguments, "workspace").ok_or_else(|| missing("its `workspace` object"))?;
                let workspace = self.store.workspace_of(object).ok_or((
                    wire::code::OBJECT_UNKNOWN,
                    format!("`{method}` needs a workspace the session declares"),
                ))?;
                WindowChange::Workspace(workspace)
            }
            "set_output" => {
                let object = object_argument(arguments, "output").ok_or_else(|| missing("its `output` object"))?;
                let (output, _) = self.store.output_of(object).ok_or((
                    wire::code::OBJECT_UNKNOWN,
                    format!("`{method}` needs an output the session declares"),
                ))?;
                WindowChange::Output(output)
            }
            _ => return Err(missing("a window verb this server serves")),
        })
    }

    /// Serves `workspace.set_layout`: the layout is swapped, and the completion says
    /// whether it was. A layout that cannot be named or loaded is refused and the
    /// workspace keeps its layout.
    fn set_workspace_layout(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        workspace: WorkspaceId,
    ) {
        let selector = match layout_selector(arguments) {
            Ok(selector) => selector,
            Err(message) => {
                self.refuse(client, envelope, wire::code::UNKNOWN_MEMBER, &message);
                return;
            }
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let answered = self.control.set_workspace_layout(workspace, &selector);
        self.sync();
        self.finish_layout(object, &operation, answered);
    }

    /// Serves `workspace_set.activate`: the configured set `name` becomes the live set. The
    /// operation completes `done` once the window manager shows the set's workspaces, and
    /// fails with `unknown-set` when the configuration defines no set of that name.
    fn workspace_set_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
    ) {
        if method != "activate" {
            self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "the workspace_set object has one method, activate",
            );
            return;
        }
        let Some(name) = text_argument(arguments, "name") else {
            self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "activate takes the set's name as text",
            );
            return;
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let answered = self.control.activate_workspace_set(&name);
        self.sync();
        let outcome = match answered {
            Some(Ok(())) => OperationOutcome::Done,
            Some(Err(WorkspaceSetRefusal::UnknownSet)) => {
                OperationOutcome::Failed(ErrorCode::from_static("unknown-set"))
            }
            // The window manager refused the set, or the loop did not answer in time: a set
            // that is not shown is never reported as done.
            Some(Err(WorkspaceSetRefusal::Refused)) | None => {
                OperationOutcome::Failed(ErrorCode::from_static("refused"))
            }
        };
        self.finish(object, &operation, Some(outcome));
    }

    /// Serves `workspace.layout_command`: the named command runs in the workspace's
    /// layout and its placements are applied. An unknown command is `refused`; a
    /// workspace without a layout is `unknown-layout`; `params` must be a JSON object.
    fn layout_command_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        workspace: WorkspaceId,
    ) {
        let Some(name) = text_argument(arguments, "name") else {
            self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "layout_command takes a text `name`",
            );
            return;
        };
        let params = text_argument(arguments, "params").unwrap_or_default();
        let args = match gnoblin_lua::layout::command_arguments(&params) {
            Ok(args) => args,
            Err(message) => {
                self.refuse(client, envelope, wire::code::UNKNOWN_MEMBER, &message);
                return;
            }
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let command = gnoblin_wm::layout::LayoutCommand { name, args };
        let answered = self.control.layout_command(workspace, command);
        self.sync();
        self.finish_layout(object, &operation, answered);
    }

    /// Serves `output.set_layout`: the output's default layout is replaced.
    fn set_output_layout(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        output: &str,
    ) {
        let selector = match layout_selector(arguments) {
            Ok(selector) => selector,
            Err(message) => {
                self.refuse(client, envelope, wire::code::UNKNOWN_MEMBER, &message);
                return;
            }
        };
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let answered = self.control.set_output_layout(output, &selector);
        self.sync();
        self.finish_layout(object, &operation, answered);
    }

    /// Sends the call's reply: the operation's identity, which its completion will reach.
    fn reply_with_operation(&mut self, client: ClientId, envelope: &Envelope, operation: &gnoblin_model::Operation) {
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&Envelope::reply(
                wire::NAMESPACE,
                wire::member::CALL,
                envelope.request_id,
                operation.id().get().to_be_bytes().to_vec(),
            ));
        }
    }

    /// Completes an operation with `outcome` unless the loop stopped answering.
    fn finish(
        &mut self,
        object: gnoblin_types::ObjectId,
        operation: &gnoblin_model::Operation,
        outcome: Option<OperationOutcome>,
    ) {
        let Some(outcome) = outcome else {
            return;
        };
        if self.store.graph().object(object).is_none() {
            let _ = self
                .store
                .graph_mut()
                .complete(operation.id(), OperationOutcome::Failed(OBJECT_GONE));
            return;
        }
        let _ = self.store.graph_mut().complete(operation.id(), outcome);
    }

    /// Completes a layout operation: `Some(Ok)` is done, `Some(Err(reason))` fails with
    /// the reason the layout gave, and `None` leaves it pending because the loop stopped.
    fn finish_layout(
        &mut self,
        object: gnoblin_types::ObjectId,
        operation: &gnoblin_model::Operation,
        answered: Option<Result<(), String>>,
    ) {
        match answered {
            Some(Ok(())) => self.finish(object, operation, Some(OperationOutcome::Done)),
            Some(Err(reason)) => {
                let code = if reason.starts_with("unknown layout") {
                    ErrorCode::from_static("unknown-layout")
                } else {
                    ErrorCode::from_static("refused")
                };
                let _ = self
                    .store
                    .graph_mut()
                    .complete(operation.id(), OperationOutcome::Failed(code));
            }
            None => {}
        }
    }

    /// Serves `window.add_effect`: the effect is appended to the window's stack. The
    /// operation completes done when the window accepts it, and fails with
    /// `unknown-effect-kind` or `too-many-passes` when it refuses.
    fn add_effect_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: gnoblin_types::WindowId,
    ) {
        let spec = effect_spec_from(arguments);
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let answered = self.control.add_effect(window, spec);
        self.sync();
        self.finish_effect(object, &operation, answered);
    }

    /// Serves `window.request_menu`: the graph issues the operation, the loop publishes
    /// the menu request with the extension as its source, and the operation completes
    /// once that is done. A window that is gone when the loop answers completes
    /// `object-gone`, the same as `focus`.
    fn request_menu_call(
        &mut self,
        client: ClientId,
        envelope: &Envelope,
        object: gnoblin_types::ObjectId,
        method: &str,
        arguments: &gnoblin_model::Fields,
        window: gnoblin_types::WindowId,
    ) {
        let operation = match self.store.graph_mut().call(object, method, arguments.clone()) {
            Ok(operation) => operation,
            Err(error) => {
                let code = refusal_code(&error);
                self.refuse(client, envelope, code, &error.to_string());
                return;
            }
        };
        self.reply_with_operation(client, envelope, &operation);
        let answered = self.control.request_menu(window, menu_anchor_from(arguments));
        self.sync();
        let outcome = match answered {
            Some(true) => Some(OperationOutcome::Done),
            Some(false) => Some(OperationOutcome::Failed(OBJECT_GONE)),
            None => None,
        };
        if let Some(outcome) = outcome {
            let _ = self.store.graph_mut().complete(operation.id(), outcome);
        }
    }

    /// Completes an `add_effect` operation from the window's answer. A refusal's reason
    /// begins with its error code, which becomes the operation's failure code.
    fn finish_effect(
        &mut self,
        object: gnoblin_types::ObjectId,
        operation: &gnoblin_model::Operation,
        answered: Option<Result<(), String>>,
    ) {
        match answered {
            Some(Ok(())) => self.finish(object, operation, Some(OperationOutcome::Done)),
            Some(Err(reason)) => {
                let code = if reason.starts_with("unknown-effect-kind") {
                    ErrorCode::from_static("unknown-effect-kind")
                } else if reason.starts_with("too-many-passes") {
                    ErrorCode::from_static("too-many-passes")
                } else {
                    ErrorCode::from_static("refused")
                };
                let _ = self
                    .store
                    .graph_mut()
                    .complete(operation.id(), OperationOutcome::Failed(code));
            }
            None => {}
        }
    }

    /// Answers one frame with an error.
    fn refuse(&mut self, client: ClientId, envelope: &Envelope, code: &str, message: &str) {
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&refusal(envelope, code, message));
        }
    }

    /// Routes the modal events the session queued and shows or clears the dialog
    /// modal. The dialog sits on the primary output and follows the session lock.
    fn pump_portal(&mut self) {
        if !self.portal.wants_pump() {
            return;
        }
        let snapshot = match self.observation.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let output = snapshot
            .outputs()
            .first()
            .and_then(|id| snapshot.output_rect(*id).map(|rect| (*id, rect)));
        let events = self.control.modal_events().unwrap_or_default();
        let scene = self.portal.pump(output, snapshot.lock_active(), &events);
        if let Some(modal) = scene.modal {
            self.control.set_modal(modal);
        }
        self.deliver_replies(scene.replies);
    }

    /// Applies the session's current snapshot to the graph.
    fn sync(&mut self) {
        let snapshot = match self.observation.lock() {
            Ok(guard) => guard.clone(),
            // A reader that panicked while holding the lock left the snapshot in
            // place; the state is whole and the graph still needs it.
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Err(error) = self.store.apply(&snapshot) {
            for failure in error.failures() {
                log::warn!(area = "ipc", kind = failure.kind(),
                    id = failure.id(),
                    error = %failure.error(),
                    "the object graph refused an object in a session snapshot"
                );
            }
        }
    }

    /// Moves every queued observation into the outbox of every subscribed client.
    fn flush(&mut self) {
        let mut broken = Vec::new();
        for (client, connection) in self.connections.iter_mut() {
            // Moved aside so the subscription list is not borrowed while the
            // connection is written to.
            let subscriptions = std::mem::take(&mut connection.subscriptions);
            for subscription in &subscriptions {
                while let Some(observation) = self.store.next(subscription) {
                    connection.send(&wire::observation(&observation));
                }
            }
            connection.subscriptions = subscriptions;
            if connection.broken {
                broken.push(*client);
            }
        }
        for client in broken {
            self.close(client, false);
        }
    }

    /// Ends the live state of a privileged connection whose stream ended.
    ///
    /// What the stream carried ends: its subscriptions, what it was waiting on, and what it offered,
    /// because the process that offered it is gone. The identity and the grants stay, since they
    /// belong to the role, and the next child handed the role is served with them. The writer is
    /// dropped rather than shut down, because the stream has already ended.
    fn lost(&mut self, client: ClientId) {
        if let Some(connection) = self.connections.remove(&client) {
            for subscription in connection.subscriptions {
                self.store.unsubscribe(subscription.id());
            }
            drop(connection.writer);
        }
        let ended = self.extensions.ended(client);
        self.deliver(ended);
        let departed = self.requests.departed(client);
        self.deliver(departed);
        let replies = self.portal.departed(client);
        self.deliver_replies(replies);
    }

    /// Ends one connection: its subscriptions, its writer and its reader.
    fn close(&mut self, client: ClientId, shutdown: bool) {
        let ended = self.extensions.ended(client);
        self.deliver(ended);
        let Some(connection) = self.connections.remove(&client) else {
            return;
        };
        for subscription in connection.subscriptions {
            self.store.unsubscribe(subscription.id());
        }
        // What the connection held and offered ends with it: a request whose owner is
        // gone is nobody's to answer, and one whose host is gone is nobody's to wait
        // for.
        let deliveries = self.requests.departed(client);
        self.deliver(deliveries);
        let replies = self.portal.departed(client);
        self.deliver_replies(replies);
        // A peer that is gone does not need its socket shut down; a peer that is still
        // there is told, so its reader returns instead of waiting for a next frame.
        let _ = connection.writer.shutdown(if shutdown {
            std::net::Shutdown::Both
        } else {
            std::net::Shutdown::Write
        });
        let reader = lock(&self.readers).remove(&client);
        if let Some(reader) = reader {
            // A reader that is the caller cannot happen (it sends the event and then
            // returns), so joining it here is what keeps the thread count honest.
            let _ = reader.join();
        }
    }
}

/// An error frame answering a request.
fn refusal(envelope: &Envelope, code: &str, message: &str) -> Envelope {
    Envelope::error(
        wire::NAMESPACE,
        envelope.member.clone(),
        envelope.request_id,
        code,
        message,
    )
}

/// The capability a member needs, or `None` when every connection may call it.
///
/// This is the one table the gate reads, so a privileged member cannot be served
/// without its capability by omission: the arm that adds a member adds it here.
fn required_capability(member: &str) -> Option<Capability> {
    match member {
        // `OFFER` is gated per slot inside `offer`: `secret.derive` needs `RetrieveSecret`.
        wire::member::CONFIG_COMMIT | wire::member::CONFIG_REJECT => Some(Capability::CommitConfig),
        wire::member::CONFIG_CURRENT => Some(Capability::ReadConfig),
        wire::member::CONFIG_RELOAD => Some(Capability::ReloadConfig),
        wire::member::PORTAL_UI | wire::member::PORTAL_CANCEL => Some(Capability::PortalUi),
        wire::member::PORTAL_KEY => Some(Capability::PortalInput),
        wire::member::CAPTURE_SCREEN => Some(Capability::CaptureScreen),
        _ => None,
    }
}

/// The wire code and detail a still-picture refusal reports. The detail names the
/// condition only; it never carries pixels or a secret.
fn capture_refusal(refusal: Refusal) -> (&'static str, &'static str) {
    match refusal {
        Refusal::Locked => (wire::code::CAPTURE_LOCKED, "the session is locked"),
        Refusal::NoOutput => (wire::code::CAPTURE_NO_OUTPUT, "there is no output to capture"),
        Refusal::Unavailable => (wire::code::CAPTURE_UNAVAILABLE, "the output has no frame yet"),
        Refusal::TooLarge => (wire::code::CAPTURE_TOO_LARGE, "the picture is too large for one frame"),
        Refusal::Encode => (wire::code::CAPTURE_ENCODE_FAILED, "the frame could not be encoded"),
    }
}

/// The wire code a model refusal reports.
/// A text argument by name, `None` when it is absent or not text.
fn text_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<String> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

/// The layout a `set_layout` call names: a module by its root, dotted path and hash, with
/// the options the Lua call passed. The options arrive as the encoded plain data that
/// `ref:with()` produced, so anything the layout could not take as an option is refused.
fn layout_selector(arguments: &gnoblin_model::Fields) -> Result<LayoutSelector, String> {
    let root = match text_argument(arguments, "root").as_deref() {
        Some("shipped") => LayoutRoot::Shipped,
        Some("user") => LayoutRoot::User,
        _ => return Err("set_layout takes `root` as `shipped` or `user`".to_owned()),
    };
    let (Some(module), Some(hash)) = (text_argument(arguments, "module"), text_argument(arguments, "hash")) else {
        return Err("set_layout takes text `module` and `hash`".to_owned());
    };
    let Some(gnoblin_model::Value::Bytes(options)) = arguments.get("options") else {
        return Err("set_layout takes bytes `options`".to_owned());
    };
    let options = crate::layout::reference_options(options)
        .map_err(|reason| format!("set_layout takes valid `options`: {reason}"))?;
    Ok(LayoutSelector::Reference(LayoutReference {
        root,
        module,
        hash,
        options,
    }))
}

/// Builds the effect a `window.add_effect` call asks for. A parameter the call leaves out
/// keeps its `EffectSpec` default, so a call with no `kind` gives the empty kind, which
/// the window refuses as `unknown-effect-kind`.
fn effect_spec_from(arguments: &gnoblin_model::Fields) -> EffectSpec {
    let mut spec = EffectSpec {
        kind: text_argument(arguments, "kind").unwrap_or_default(),
        ..EffectSpec::default()
    };
    if let Some(value) = bool_argument(arguments, "enabled") {
        spec.enabled = value;
    }
    if let Some(value) = uint_argument(arguments, "downscale").and_then(|value| u32::try_from(value).ok()) {
        spec.downscale = value;
    }
    if let Some(value) = uint_argument(arguments, "passes").and_then(|value| u32::try_from(value).ok()) {
        spec.passes = value;
    }
    if let Some(value) = text_argument(arguments, "algorithm") {
        spec.algorithm = value;
    }
    if let Some(value) = text_argument(arguments, "tint") {
        spec.tint = value;
    }
    if let Some(value) = text_argument(arguments, "colour") {
        spec.colour = value;
    }
    if let Some(value) = text_argument(arguments, "source") {
        spec.source = value;
    }
    if let Some(value) = text_argument(arguments, "entry") {
        spec.entry = value;
    }
    for (name, slot) in [
        ("noise", &mut spec.noise),
        ("brightness", &mut spec.brightness),
        ("contrast", &mut spec.contrast),
        ("saturation", &mut spec.saturation),
        ("offset_x", &mut spec.offset_x),
        ("offset_y", &mut spec.offset_y),
        ("blur_radius", &mut spec.blur_radius),
        ("spread", &mut spec.spread),
    ] {
        if let Some(value) = double_argument(arguments, name) {
            *slot = value;
        }
    }
    spec
}

/// Reads a `window.request_menu` anchor from its arguments. A missing or out-of-range
/// coordinate becomes zero, so a call the schema already typed still yields an anchor
/// rather than a guessed position.
fn menu_anchor_from(arguments: &gnoblin_model::Fields) -> gnoblin_wayland::MenuAnchor {
    let coordinate = |name| {
        int_argument(arguments, name)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0)
    };
    let size = |name| {
        uint_argument(arguments, name)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    };
    gnoblin_wayland::MenuAnchor::new(coordinate("x"), coordinate("y"), size("width"), size("height"))
}

/// The object identity at `name`; `None` when it is absent or of another type.
fn object_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<gnoblin_types::ObjectId> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Object(object)) => Some(*object),
        _ => None,
    }
}

/// The code a window verb completes with when the window manager refused it. The window's
/// own `Gone` is `object-gone`; each other code names the constraint that refused the change.
fn window_refusal_code(refusal: WindowChangeRefusal) -> ErrorCode {
    match refusal {
        WindowChangeRefusal::Gone => OBJECT_GONE,
        WindowChangeRefusal::UnknownTarget => ErrorCode::from_static(wire::code::OBJECT_UNKNOWN),
        WindowChangeRefusal::NotFloating => ErrorCode::from_static("not-floating"),
        WindowChangeRefusal::TransientFollowsParent => ErrorCode::from_static("transient-follows-parent"),
        WindowChangeRefusal::Refused => ErrorCode::from_static("refused"),
    }
}

/// The code a visual verb completes with when the loop refused it. The window's own `Gone` is
/// `object-gone`; `locked` names the lock cover that holds the drawing, and `out-of-range` names
/// a value the method's documented range does not accept.
fn visual_refusal_code(refusal: WindowVisualRefusal) -> ErrorCode {
    match refusal {
        WindowVisualRefusal::Gone => OBJECT_GONE,
        WindowVisualRefusal::Locked => ErrorCode::from_static("locked"),
        WindowVisualRefusal::OutOfRange => ErrorCode::from_static("out-of-range"),
    }
}

/// The signed integer at `name`; `None` when it is absent or of another type.
fn int_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<i64> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Int(value)) => Some(*value),
        _ => None,
    }
}

/// The unsigned integer at `name`; `None` when it is absent or of another type.
fn uint_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<u64> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Uint(value)) => Some(*value),
        _ => None,
    }
}

/// The double at `name`; `None` when it is absent or of another type.
fn double_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<f64> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Double(value)) => Some(*value),
        _ => None,
    }
}

/// The boolean at `name`; `None` when it is absent or of another type.
fn bool_argument(arguments: &gnoblin_model::Fields, name: &str) -> Option<bool> {
    match arguments.get(name) {
        Some(gnoblin_model::Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn refusal_code(error: &ModelError) -> &'static str {
    match error {
        ModelError::UnknownObject { .. } | ModelError::UnknownOperation { .. } => wire::code::OBJECT_UNKNOWN,
        _ => wire::code::UNKNOWN_MEMBER,
    }
}

/// The power verb a `session` method names, or [`None`] when the method is not one.
fn power_verb(method: &str) -> Option<gnoblin_model::PowerVerb> {
    match method {
        "poweroff" => Some(gnoblin_model::PowerVerb::PowerOff),
        "reboot" => Some(gnoblin_model::PowerVerb::Reboot),
        "suspend" => Some(gnoblin_model::PowerVerb::Suspend),
        "hibernate" => Some(gnoblin_model::PowerVerb::Hibernate),
        _ => None,
    }
}

/// The code a power verb completes with when this session was started with no power backend.
const POWER_UNAVAILABLE: ErrorCode = ErrorCode::from_static("unavailable");

/// The code a `lock` call completes with when the session is not confirmed locked.
const NOT_LOCKED: ErrorCode = ErrorCode::from_static("not-locked");

/// How long `window.present` keeps asking the loop before it gives up with `busy`. The doc
/// promises five seconds, so the retry is bounded by the same number the caller was told.
const PRESENT_DEADLINE: Duration = Duration::from_secs(5);

/// The pause between asks while the loop reports the window not shown or under input.
const PRESENT_RETRY: Duration = Duration::from_millis(25);

/// The code a `window.present` call completes with when the window is not a started
/// program's, the session is locked, or the focus policy declined.
const DENIED: ErrorCode = ErrorCode::from_static("denied");

/// The code a `window.present` call completes with when the window was not shown, or the user
/// kept typing, for the whole deadline.
const BUSY: ErrorCode = ErrorCode::from_static("busy");

/// The code a `window.present` call completes with when the window closed first.
const NOT_FOUND: ErrorCode = ErrorCode::from_static("not-found");

/// The code an `idle.inhibit` call completes with when the holder cap is full.
const TOO_MANY_INHIBITORS: ErrorCode = ErrorCode::from_static("too-many-inhibitors");

/// The code an idle hold call completes with when the compositor is not serving the idle state.
const IDLE_UNAVAILABLE: ErrorCode = ErrorCode::from_static("idle-unavailable");

/// The code a `layers.register` call completes with when the registry or the stack refused the layer.
const LAYER_REFUSED: ErrorCode = ErrorCode::from_static("layer-refused");

/// A locked map, with a poisoned lock read as its contents rather than a panic.
fn lock<T>(map: &Arc<Mutex<T>>) -> MutexGuard<'_, T> {
    map.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Spawns one named thread, reporting the failure instead of panicking.
fn spawn(name: &str, body: impl FnOnce() + Send + 'static, path: &Path) -> Result<JoinHandle<()>, IpcError> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map_err(|source| IpcError::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use gnoblin_client::wire;
    use gnoblin_proto::Capability;

    use super::{DEADLINE_TICK, DeadlineClock, IpcEvent, capture_refusal, events, required_capability, wake};
    use gnoblin_capture::still::Refusal;

    /// The regression: deadlines were swept only on a quiet `recv_timeout`, so a client that
    /// kept the loop busy starved every deadline and `config reload` never returned.
    #[test]
    fn events_faster_than_the_tick_still_make_a_sweep_due() {
        let start = Instant::now();
        let mut clock = DeadlineClock::new(start);
        let step = Duration::from_millis(10);
        let mut now = start;
        let mut sweeps = 0;
        for _ in 0..100 {
            now += step;
            if clock.due(now) {
                sweeps += 1;
            }
        }
        // One event every 10 ms for a second: a sweep every tick, never starved.
        assert_eq!(sweeps, 1000 / DEADLINE_TICK.as_millis() as usize);
    }

    #[test]
    fn a_quiet_loop_waits_out_the_tick_then_sweeps_once() {
        let start = Instant::now();
        let mut clock = DeadlineClock::new(start);
        assert_eq!(clock.wait(start), DEADLINE_TICK);
        assert!(!clock.due(start + DEADLINE_TICK - Duration::from_millis(1)));
        assert!(clock.due(start + DEADLINE_TICK));
        assert!(!clock.due(start + DEADLINE_TICK), "one sweep per tick");
    }

    #[test]
    fn two_publishes_before_one_look_queue_one_wake_up() {
        let (sender, received) = events();
        let (observer, pending) = wake(sender);
        let snapshot = gnoblin_wayland::SessionSnapshot::default();
        observer.published(&snapshot);
        observer.published(&snapshot);
        observer.published(&snapshot);
        assert!(matches!(received.recv().expect("one wake"), IpcEvent::Changed));
        assert!(
            received.try_recv().is_err(),
            "the publishes between two looks are one look"
        );
        assert!(pending.load(Ordering::Acquire), "the dispatcher has not looked yet");
        // What the dispatcher does when it looks: the next publish queues again.
        pending.store(false, Ordering::Release);
        observer.published(&snapshot);
        assert!(matches!(received.recv().expect("a new wake"), IpcEvent::Changed));
    }

    #[test]
    fn a_channel_with_no_sender_ends_the_dispatch() {
        // A compositor that stops without a shutdown event still ends its dispatcher,
        // which is what makes the thread joinable rather than leaked.
        let (sender, received) = events();
        drop(sender);
        assert!(received.recv().is_err());
    }

    #[test]
    fn config_members_map_to_their_own_capabilities() {
        // Reading and reloading are split from committing so a same-uid CLI can
        // use `config get`/`config reload` without being able to write the document.
        assert_eq!(
            required_capability(wire::member::CONFIG_CURRENT),
            Some(Capability::ReadConfig)
        );
        assert_eq!(
            required_capability(wire::member::CONFIG_RELOAD),
            Some(Capability::ReloadConfig)
        );
        assert_eq!(
            required_capability(wire::member::CONFIG_COMMIT),
            Some(Capability::CommitConfig)
        );
        assert_eq!(
            required_capability(wire::member::CONFIG_REJECT),
            Some(Capability::CommitConfig)
        );
    }

    #[test]
    fn a_capture_is_gated_on_the_capture_screen_capability() {
        // Failure mode: an ungranted client must be refused, so the member must map to
        // a capability at all. An omitted arm would serve pixels to any connection.
        assert_eq!(
            required_capability(wire::member::CAPTURE_SCREEN),
            Some(Capability::CaptureScreen)
        );
    }

    #[test]
    fn every_capture_refusal_names_a_wire_code_the_client_knows() {
        // The client reads these codes back; a refusal that fell through to an unknown
        // code would reach the caller as a failure it cannot act on.
        assert_eq!(capture_refusal(Refusal::Locked).0, wire::code::CAPTURE_LOCKED);
        assert_eq!(capture_refusal(Refusal::NoOutput).0, wire::code::CAPTURE_NO_OUTPUT);
        assert_eq!(capture_refusal(Refusal::Unavailable).0, wire::code::CAPTURE_UNAVAILABLE);
        assert_eq!(capture_refusal(Refusal::TooLarge).0, wire::code::CAPTURE_TOO_LARGE);
        assert_eq!(capture_refusal(Refusal::Encode).0, wire::code::CAPTURE_ENCODE_FAILED);
    }
}
