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

use std::collections::BTreeMap;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use gnoblin_client::wire;
use gnoblin_model::{ModelError, OBJECT_GONE, OperationOutcome, Subscription};
use gnoblin_proto::{Envelope, Handshake, HandshakeKind, read_frame, write_frame};
use gnoblin_types::{ClientId, Version};
use gnoblin_wayland::{ControlHandle, Observer, SessionSnapshot};
use tracing::{debug, info, warn};

use super::{IpcError, SessionSync, SocketPath};

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

/// One thing the IPC server has to do, in the order it arrived.
pub(crate) enum IpcEvent {
    /// A client finished the handshake and may be served.
    Accepted { client: ClientId, writer: UnixStream },
    /// One frame a client sent.
    Request { client: ClientId, envelope: Envelope },
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
    pub fn start(
        prepared: SocketPath,
        store: SessionSync,
        events: Sender<IpcEvent>,
        received: Receiver<IpcEvent>,
        pending: Arc<AtomicBool>,
        observation: Arc<Mutex<SessionSnapshot>>,
        control: ControlHandle,
    ) -> Result<IpcServer, IpcError> {
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

        let dispatcher = Dispatcher {
            store,
            observation,
            control,
            pending,
            connections: BTreeMap::new(),
            readers: Arc::clone(&readers),
        };
        let dispatcher = spawn("gnoblin-ipc-dispatch", move || dispatcher.run(received), &path)?;
        let acceptor = spawn(
            "gnoblin-ipc-accept",
            {
                let events = events.clone();
                let stopping = Arc::clone(&stopping);
                move || accept_loop(listener, events, stopping, readers, clients, version)
            },
            &path,
        )?;
        info!(socket = %path.display(), "[ipc] listening");
        Ok(IpcServer {
            events,
            path,
            stopping,
            acceptor: Some(acceptor),
            dispatcher: Some(dispatcher),
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
        let _ = self.events.send(IpcEvent::Shutdown);
        if let Some(acceptor) = self.acceptor.take() {
            let _ = acceptor.join();
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
) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                // A listener that cannot accept is a socket this process cannot serve
                // anyone on. Saying so and returning leaves the session running, which
                // is the only choice that does not end the desktop for an IPC fault.
                warn!(%error, "[ipc] accept failed, the IPC socket is no longer served");
                return;
            }
        };
        if stopping.load(Ordering::SeqCst) {
            return;
        }
        let client = ClientId::new(clients.fetch_add(1, Ordering::Relaxed));
        let served = events.clone();
        // Holding the lock across the spawn and the insert is what lets the dispatcher
        // join a reader it has just been told about: the handle is in the map before
        // the reader can report anything.
        let mut handles = lock(&readers);
        match std::thread::Builder::new()
            .name(format!("gnoblin-ipc-{}", client.get()))
            .spawn(move || serve(stream, client, version, served))
        {
            Ok(handle) => {
                handles.insert(client, handle);
            }
            Err(error) => warn!(%error, "[ipc] a client connection could not be served"),
        }
    }
}

/// Serves one client: the handshake, then its frames, until the socket ends.
fn serve(stream: UnixStream, client: ClientId, version: Version, events: Sender<IpcEvent>) {
    let writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(error) => {
            debug!(%error, "[ipc] a connection could not be duplicated");
            return;
        }
    };
    if let Err(error) = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)) {
        debug!(%error, "[ipc] a connection could not be given a handshake deadline");
        return;
    }
    let mut reader = std::io::BufReader::new(stream);
    let hello = match read_handshake(&mut reader) {
        Some(hello) => hello,
        None => return,
    };
    let answer = match gnoblin_proto::check(version, hello.version) {
        Ok(()) => Handshake::accept(version),
        Err(mismatch) => {
            // The refusal names this compositor's version, so the client can name both
            // and say which side to upgrade.
            info!(%mismatch, "[ipc] refused a client version");
            let _ = write_handshake(&writer, &Handshake::refuse(version));
            return;
        }
    };
    if write_handshake(&writer, &answer).is_err() {
        return;
    }
    // An idle subscriber waits as long as its compositor lives, so the handshake's
    // deadline is the only one this socket has.
    if let Err(error) = reader.get_ref().set_read_timeout(None) {
        debug!(%error, "[ipc] a connection could not drop its handshake deadline");
        return;
    }
    if let Err(error) = writer.set_write_timeout(Some(WRITE_TIMEOUT)) {
        debug!(%error, "[ipc] a connection could not be given a write deadline");
        return;
    }
    if events.send(IpcEvent::Accepted { client, writer }).is_err() {
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
                debug!(%error, client = client.get(), "[ipc] a client sent a frame that did not decode");
                break;
            }
        }
    }
    let _ = events.send(IpcEvent::Closed { client });
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
                debug!(%error, "[ipc] a client did not send its handshake");
                return None;
            }
        }
    }
    match Handshake::decode(&frame) {
        Ok(hello) if hello.kind == HandshakeKind::Hello => Some(hello),
        Ok(handshake) => {
            debug!(kind = ?handshake.kind, "[ipc] a connection did not start with a hello");
            None
        }
        Err(error) => {
            debug!(%error, "[ipc] a handshake did not decode");
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

/// What the dispatcher holds for one client.
struct Connection {
    writer: UnixStream,
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
            debug!(%error, "[ipc] a connection could not be written, dropping it");
            self.broken = true;
        }
    }
}

/// The one owner of the graph: everything a connection asks for runs here, in order.
struct Dispatcher {
    store: SessionSync,
    observation: Arc<Mutex<SessionSnapshot>>,
    control: ControlHandle,
    pending: Arc<AtomicBool>,
    connections: BTreeMap<ClientId, Connection>,
    readers: Arc<Mutex<BTreeMap<ClientId, JoinHandle<()>>>>,
}

impl Dispatcher {
    /// Serves events until the server is told to stop.
    fn run(mut self, events: Receiver<IpcEvent>) {
        loop {
            let event = match events.recv() {
                Ok(event) => event,
                // Every sender is gone: nothing can ask for anything any more.
                Err(_) => break,
            };
            match event {
                IpcEvent::Accepted { client, writer } => {
                    self.connections.insert(
                        client,
                        Connection {
                            writer,
                            subscriptions: Vec::new(),
                            broken: false,
                        },
                    );
                }
                IpcEvent::Request { client, envelope } => self.request(client, &envelope),
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

    /// Decodes and answers one frame.
    fn request(&mut self, client: ClientId, envelope: &Envelope) {
        if envelope.namespace != wire::NAMESPACE {
            self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this connection serves the `gnoblin` namespace only",
            );
            return;
        }
        match envelope.member.as_str() {
            wire::member::SUBSCRIBE => self.subscribe(client, envelope),
            wire::member::CALL => self.call(client, envelope),
            wire::member::CLAIM | wire::member::ANSWER => self.refuse(
                client,
                envelope,
                wire::code::REQUEST_UNKNOWN,
                // Requests are offered by the host that needs them, and this slice
                // offers none: prompts, polkit and GeoClue arrive with the services
                // role, and until then a claim names a request nobody has.
                "this build offers no requests",
            ),
            _ => self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this connection does not serve that member",
            ),
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
        match (ty, window) {
            (Some(ty), Some(window)) if ty == "window" && method == "focus" => {
                self.focus(client, envelope, object, &method, &arguments, window);
            }
            (None, _) => self.refuse(
                client,
                envelope,
                wire::code::OBJECT_UNKNOWN,
                &format!("no live object has identity {}", object.get()),
            ),
            (Some(_), None) => self.refuse(
                client,
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this object is not a window, and this build serves no method on it",
            ),
            (Some(_), Some(_)) => self.refuse(
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

    /// Answers one frame with an error.
    fn refuse(&mut self, client: ClientId, envelope: &Envelope, code: &str, message: &str) {
        if let Some(connection) = self.connections.get_mut(&client) {
            connection.send(&refusal(envelope, code, message));
        }
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
            warn!(%error, "[ipc] the object graph refused a session snapshot");
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

    /// Ends one connection: its subscriptions, its writer and its reader.
    fn close(&mut self, client: ClientId, shutdown: bool) {
        let Some(connection) = self.connections.remove(&client) else {
            return;
        };
        for subscription in connection.subscriptions {
            self.store.unsubscribe(subscription.id());
        }
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

/// The wire code a model refusal reports.
fn refusal_code(error: &ModelError) -> &'static str {
    match error {
        ModelError::UnknownObject { .. } | ModelError::UnknownOperation { .. } => wire::code::OBJECT_UNKNOWN,
        _ => wire::code::UNKNOWN_MEMBER,
    }
}

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

    use super::{IpcEvent, events, wake};

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
}
