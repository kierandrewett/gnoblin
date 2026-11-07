//! The EI wire: a socket, one thread and `reis`.
//!
//! This is the only module in the crate that owns an I/O type. It speaks the protocol
//! `reis` implements, turns every request into a [`EiRequest`] for [`super::service`], and
//! performs the [`Action`]s that come back: seat capabilities, devices, events for the
//! seat, and the end of a connection.
//!
//! # Why a thread and not the compositor's loop
//!
//! The endpoint owns a thread and one `poll`, because the compositor's event loop
//! (`gnoblin-wayland`'s calloop loop) belongs to another crate and this crate may not
//! name it. The thread touches nothing the compositor owns: injected events travel
//! through the caller's sink, which is a queue push, and every policy decision is the
//! pure state machine's. The cost is one thread per session that has an EIS endpoint,
//! not one per client.
//!
//! # Consent arrives later than the connection
//!
//! A client connects, and the decision that lets it inject anything may not exist yet.
//! [`Endpoint::grant`] and [`Endpoint::refuse`] are how the caller answers, on its own
//! thread: they queue a request and wake the poll. A context that connects after the
//! decision is granted its capabilities immediately.

use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use gnoblin_backend::InputEvent;

use reis::PendingRequestResult;
use reis::eis;
use reis::enumflags2::BitFlags;
use reis::handshake::EisHandshaker;
use reis::request::{Connection, DeviceCapability, EisRequest, EisRequestConverter, Seat};
use rustix::event::{PollFd, PollFlags, poll};

use super::service::{
    Action, Caps, ContextId, ContextType, DeviceId, EiRequest, Ending, Grant, Quota, Region, Service, Violation,
};

use crate::log;

/// Where an injected event goes: the caller's seat handle, as a closure.
///
/// It is a `dyn Fn` rather than a trait because the seat's own handle belongs to
/// `gnoblin-wayland`, which this crate may not name. A closure is the smallest thing a
/// caller can hand over that this module can call.
type Sink = dyn Fn(InputEvent) + Send;

/// How the endpoint is set up.
#[derive(Clone, Debug)]
pub struct Options {
    /// Where the socket goes.
    ///
    /// A session's portal hands out a socket path of its own, so the path is the
    /// caller's: nothing here guesses `XDG_RUNTIME_DIR`.
    pub path: PathBuf,
    /// The consent decision a client gets when it connects.
    ///
    /// [`Grant::pending`] is the real default. [`Grant::granted`] exists for the two
    /// callers that already hold the decision when the endpoint starts: a session that
    /// resumes an approved remote desktop connection, and the test harness. There is no
    /// unconsented path anywhere else, and a session that is granted nothing can inject
    /// nothing.
    pub grant: Grant,
    /// The layout absolute devices are created with.
    pub regions: Vec<Region>,
    /// What one device may send between two frames.
    pub quota: Quota,
}

/// A running EIS endpoint.
///
/// Dropping it stops the thread and removes the socket, so a test cannot leak one.
pub struct Endpoint {
    path: PathBuf,
    shared: Arc<Channel>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
}

/// What the caller thread asks the endpoint's thread to do.
#[derive(Clone, Debug)]
enum Request {
    /// Apply a consent decision to every context that is still waiting for one.
    Grant(Grant),
    /// Replace the layout new devices are created with.
    Regions(Vec<Region>),
}

/// The channel between the caller and the endpoint's thread.
struct Channel {
    stop: AtomicBool,
    requests: Mutex<Vec<Request>>,
}

impl Channel {
    /// Whether the caller asked the endpoint to stop. The thread checks this between
    /// polls, so a stop takes effect within one wake.
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Queue a request for the endpoint's thread. The lock keeps the critical section
    /// to a single push, so the caller never waits on the protocol.
    fn push(&self, request: Request) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    /// Take every queued request, leaving the queue empty. Poisoning is ignored because
    /// the queue holds plain values that remain valid after a panic elsewhere.
    fn take(&self) -> Vec<Request> {
        std::mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Endpoint {
    /// Binds the socket and starts serving.
    ///
    /// `sink` receives every event an approved client produces, in the order the state
    /// machine released it, as the same [`InputEvent`] a backend device produces. The
    /// caller is expected to hand it to the seat's own input path
    /// (`gnoblin-wayland`'s `InputSink`), which is what makes the lock gate, grabs and
    /// bindings apply to injected input.
    ///
    /// # Errors
    ///
    /// Whatever the socket bind reports. The bind happens on this thread so a caller
    /// learns about a path it may not use before a client can connect to it.
    pub fn bind(options: Options, sink: impl Fn(InputEvent) + Send + 'static) -> io::Result<Self> {
        let listener = eis::Listener::bind(&options.path)?;
        let (wake, wake_read) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        wake_read.set_nonblocking(true)?;
        let shared = Arc::new(Channel {
            stop: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
        });
        let service = Service::new(options.grant, options.regions, options.quota);
        let path = listener.path().to_path_buf();
        let thread = thread::Builder::new().name("gnoblin-eis".to_owned()).spawn({
            let shared = Arc::clone(&shared);
            move || serve(listener, service, Box::new(sink), &shared, &wake_read)
        })?;
        // `wake_read` is moved into the thread by the closure above; `wake` stays here.
        Ok(Self {
            path,
            shared,
            wake,
            thread: Some(thread),
        })
    }

    /// The socket path clients connect to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Grants these capabilities to every context that is waiting for a decision.
    ///
    /// This is the portal path: the user approved the request, so the seat advertises
    /// what the approval covers and nothing else.
    pub fn grant(&self, capabilities: Caps) {
        self.request(Request::Grant(Grant::granted(capabilities)));
    }

    /// Refuses: every waiting context ends, and so does every later one.
    pub fn refuse(&self) {
        self.request(Request::Grant(Grant::refused()));
    }

    /// Replaces the layout for devices created from now on.
    pub fn set_regions(&self, regions: Vec<Region>) {
        self.request(Request::Regions(regions));
    }

    /// Stops the thread and waits for it, and removes the socket.
    pub fn stop(mut self) {
        self.stop_inner();
    }

    /// Queue a request and wake the thread. A stopped endpoint drops it, which is the
    /// only correct answer: there is no thread left to apply it.
    fn request(&self, request: Request) {
        if self.shared.stopped() {
            return;
        }
        self.shared.push(request);
        wake(&self.wake);
    }

    /// Ask the thread to stop, wake it, and wait for it to exit. The join is what makes
    /// the socket removal in the thread happen before `stop` returns.
    fn stop_inner(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        wake(&self.wake);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Stopping on drop means a test that forgets to call [`Endpoint::stop`] still removes
/// its socket and joins its thread.
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Shows only the socket path. The thread and channel hold no state worth printing.
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Endpoint")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Wakes the endpoint's poll. A full or closed socket needs nothing: the poll is already
/// awake or the endpoint is stopping.
fn wake(socket: &UnixStream) {
    let _ = (&*socket).write_all(&[1]);
}

/// One connection, from accept to the socket closing.
enum Progress {
    /// The EI handshake is still running.
    Handshake(EisHandshaker),
    /// The client is connected and its context exists.
    Connected(Connected),
}

/// One accepted socket and where it is in its life: still shaking hands, or connected.
struct Peer {
    socket: eis::Context,
    state: Progress,
}

/// A connected client: the protocol objects and the context they belong to.
struct Connected {
    converter: EisRequestConverter,
    connection: Connection,
    context: ContextId,
    seat: Option<Seat>,
    /// The devices on the wire and the identities their events name.
    ///
    /// A short list rather than a map: a context has at most three devices, and a map
    /// keyed by `reis`'s handle would be a map keyed by an interior-mutable type.
    devices: Vec<(reis::request::Device, DeviceId)>,
    ended: bool,
}

impl Connected {
    /// The context this connection belongs to, once the handshake finished.
    const fn context(&self) -> ContextId {
        self.context
    }
}

/// Peer state helpers, used by the poll loop to find the context a request belongs to.
impl Peer {
    /// The context this session belongs to, if the handshake finished.
    fn context(&self) -> Option<ContextId> {
        match &self.state {
            Progress::Handshake(_) => None,
            Progress::Connected(connected) => Some(connected.context()),
        }
    }

    /// The socket, for the poll set.
    const fn watch(&self) -> &eis::Context {
        &self.socket
    }
}

/// What the loop wants to do with a session after handling it.
enum Next {
    /// Keep it.
    Continue,
    /// Close it: the client is gone or the state machine ended it.
    Close,
}

/// The endpoint's thread: accept, read, answer, until told to stop.
fn serve(listener: eis::Listener, mut service: Service, sink: Box<Sink>, shared: &Channel, wake_read: &UnixStream) {
    let mut sessions: BTreeMap<u64, Peer> = BTreeMap::new();
    let mut next_session = 0;
    loop {
        if shared.stopped() {
            break;
        }
        let targets = match wait(&listener, &sessions, wake_read) {
            Ok(targets) => targets,
            Err(error) => {
                log::warn!(%error, "the poll failed; the endpoint stops");
                break;
            }
        };
        for target in targets {
            if shared.stopped() {
                break;
            }
            match target {
                PollTarget::Wake => {
                    drain(wake_read);
                    answer(shared.take(), &mut service, &mut sessions, &*sink);
                }
                PollTarget::Listener => {
                    while let Ok(Some(socket)) = listener.accept() {
                        let handshaker = EisHandshaker::new(&socket, 1);
                        log::debug!("a client connected");
                        sessions.insert(
                            next_session,
                            Peer {
                                socket,
                                state: Progress::Handshake(handshaker),
                            },
                        );
                        next_session += 1;
                    }
                }
                PollTarget::Peer(key) => {
                    let Some(session) = sessions.get_mut(&key) else {
                        continue;
                    };
                    if let Next::Close = handle(session, &mut service, &*sink) {
                        // The client is gone or the state machine ended it: release what
                        // it held, forget the context and close the socket.
                        if let Some(context) = session.context() {
                            let actions = releases(&mut service, context);
                            perform_all(session, actions, &*sink);
                            if let Some(ended) = service.context(context) {
                                log::debug!(%context,
                                    name = ended.name().unwrap_or(""),
                                    ending = ?ended.ended(),
                                    "the client is gone"
                                );
                            }
                            service.forget(context);
                        }
                        sessions.remove(&key);
                    }
                }
            }
        }
        // A tick of flushing per iteration keeps a client that sent one request from
        // waiting for the next poll to hear back.
        for session in sessions.values() {
            if let Progress::Connected(connected) = &session.state
                && let Err(error) = connected.connection.flush()
            {
                log::debug!(%error, "a flush failed; the client will see it as a read error");
            }
        }
    }
    for session in sessions.values_mut() {
        if let Some(context) = session.context() {
            let actions = releases(&mut service, context);
            perform_all(session, actions, &*sink);
        }
    }
}

/// What the poll reported as ready.
enum PollTarget {
    /// The listening socket has a connection waiting.
    Listener,
    /// The caller asked for something.
    Wake,
    /// A client's socket has data.
    Peer(u64),
}

/// Waits for anything, and reports what became ready.
fn wait(
    listener: &eis::Listener,
    sessions: &BTreeMap<u64, Peer>,
    wake_read: &UnixStream,
) -> io::Result<Vec<PollTarget>> {
    let order: Vec<u64> = sessions.keys().copied().collect();
    let mut fds = Vec::with_capacity(order.len() + 2);
    fds.push(PollFd::new(listener, PollFlags::IN));
    fds.push(PollFd::new(wake_read, PollFlags::IN));
    for key in &order {
        if let Some(session) = sessions.get(key) {
            fds.push(PollFd::new(session.watch(), PollFlags::IN));
        }
    }
    // No timeout: the wake socket is the only thing that interrupts this poll, and a
    // spurious timeout would burn a whole turn of the compositor's loop.
    loop {
        match poll(&mut fds, None) {
            Ok(_) => break,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(io::Error::from_raw_os_error(error.raw_os_error())),
        }
    }
    let ready = PollFlags::IN | PollFlags::HUP | PollFlags::ERR;
    let mut targets = Vec::new();
    for (index, fd) in fds.iter().enumerate() {
        if !fd.revents().intersects(ready) {
            continue;
        }
        targets.push(match index {
            0 => PollTarget::Listener,
            1 => PollTarget::Wake,
            _ => match index - 2 < order.len() {
                true => PollTarget::Peer(order[index - 2]),
                false => continue,
            },
        });
    }
    Ok(targets)
}

/// Reads the wake socket empty, so the next poll blocks again.
fn drain(socket: &UnixStream) {
    let mut buffer = [0u8; 64];
    while let Ok(read) = (&*socket).read(&mut buffer) {
        if read == 0 {
            break;
        }
    }
}

/// Applies the caller's requests and performs what they produce.
fn answer(requests: Vec<Request>, service: &mut Service, sessions: &mut BTreeMap<u64, Peer>, sink: &Sink) {
    for request in requests {
        match request {
            Request::Grant(grant) => {
                for (context, action) in service.set_grant(grant) {
                    if let Some(session) = session_of(sessions, context) {
                        perform_all(session, vec![action], sink);
                    }
                }
            }
            Request::Regions(regions) => service.set_regions(regions),
        }
    }
}

/// The session holding a context.
fn session_of(sessions: &mut BTreeMap<u64, Peer>, context: ContextId) -> Option<&mut Peer> {
    sessions.values_mut().find(|session| session.context() == Some(context))
}

/// Reads everything one client sent and answers it.
fn handle(session: &mut Peer, service: &mut Service, sink: &Sink) -> Next {
    if let Err(error) = session.socket.read() {
        if error.kind() == ErrorKind::WouldBlock {
            return Next::Continue;
        }
        log::debug!(%error, "the client's socket ended");
        return Next::Close;
    }
    loop {
        let Some(pending) = session.socket.pending_request() else {
            return Next::Continue;
        };
        let request = match pending {
            PendingRequestResult::Request(request) => request,
            PendingRequestResult::ParseError(error) => {
                // A stream the protocol cannot parse leaves the object table out of step,
                // so the client is told and closed instead of every later request being
                // answered against the wrong object.
                log::warn!(%error, "the client sent an unparseable request");
                if let Progress::Connected(connected) = &mut session.state {
                    refuse(connected, service, &error.to_string(), sink);
                }
                return Next::Close;
            }
            PendingRequestResult::InvalidObject(object) => {
                if let Progress::Connected(connected) = &session.state {
                    connected
                        .connection
                        .connection()
                        .invalid_object(connected.connection.last_serial(), object);
                }
                continue;
            }
        };
        let control = match &session.state {
            Progress::Handshake(_) => handshake(session, request, service, sink),
            Progress::Connected(_) => request_connected(session, request, service, sink),
        };
        if let Next::Close = control {
            return Next::Close;
        }
    }
}

/// Advances the handshake, and switches the session to connected when it finishes.
fn handshake(session: &mut Peer, request: eis::Request, service: &mut Service, sink: &Sink) -> Next {
    let finished = {
        let Progress::Handshake(handshaker) = &mut session.state else {
            return Next::Continue;
        };
        match handshaker.handle_request(request) {
            Ok(finished) => finished,
            Err(error) => {
                log::warn!(%error, "the handshake failed");
                return Next::Close;
            }
        }
    };
    let Some(response) = finished else {
        return Next::Continue;
    };
    let context_type = match response.context_type {
        eis::handshake::ContextType::Sender => ContextType::Sender,
        eis::handshake::ContextType::Receiver => ContextType::Receiver,
    };
    log::debug!(
        name = response.name.as_deref().unwrap_or(""),
        ?context_type,
        "the handshake finished"
    );
    let converter = EisRequestConverter::new(&session.socket, response.clone(), 1);
    // The converter owns the connection it was built over; a clone of its handle is what
    // the seat and the devices are created through, so they outlive the request loop's
    // borrow of the converter.
    let connection = converter.handle().clone();
    let (context, actions) = service.connect(response.name.as_deref(), context_type);
    let mut connected = Connected {
        converter,
        connection,
        context,
        seat: None,
        devices: Vec::new(),
        ended: false,
    };
    perform_all_connected(&mut connected, actions, sink);
    session.state = Progress::Connected(connected);
    Next::Continue
}

/// Answers one request on a connected session.
fn request_connected(session: &mut Peer, request: eis::Request, service: &mut Service, sink: &Sink) -> Next {
    let Progress::Connected(connected) = &mut session.state else {
        return Next::Continue;
    };
    if let Err(error) = connected.converter.handle_request(request) {
        refuse(connected, service, &error.to_string(), sink);
        return Next::Close;
    }
    while let Some(request) = connected.converter.next_request() {
        let Some(command) = command(request, &connected.devices) else {
            continue;
        };
        let actions = service.apply(connected.context, command);
        perform_all_connected(connected, actions, sink);
        if connected.ended {
            return Next::Close;
        }
    }
    Next::Continue
}
/// Ends a session the protocol layer refused.
///
/// The client is told why on the wire, the seat gets the releases the client owed it, and
/// the record keeps the reason. The `End` action is not performed: the client has already
/// been told, and a second `ei_connection.disconnected` on a connection that is closing is
/// noise.
fn refuse(connected: &mut Connected, service: &mut Service, explanation: &str, sink: &Sink) {
    log::warn!(explanation, "the session ends");
    connected
        .connection
        .disconnected(eis::connection::DisconnectReason::Protocol, Some(explanation));
    connected.ended = true;
    let context = connected.context;
    for action in service.end(context, Ending::Violation(Violation::Malformed)) {
        if !matches!(action, Action::End { .. }) {
            perform(connected, action, sink);
        }
    }
}

/// The actions a closing connection still owes the seat: its releases, not its goodbye.
fn releases(service: &mut Service, context: ContextId) -> Vec<Action> {
    service
        .disconnect(context)
        .into_iter()
        .filter(|action| !matches!(action, Action::End { .. }))
        .collect()
}

/// Performs actions for a session, which must be connected.
fn perform_all(session: &mut Peer, actions: Vec<Action>, sink: &Sink) {
    if let Progress::Connected(connected) = &mut session.state {
        perform_all_connected(connected, actions, sink);
    }
}

/// Performs actions for a connected client.
fn perform_all_connected(connected: &mut Connected, actions: Vec<Action>, sink: &Sink) {
    for action in actions {
        perform(connected, action, sink);
    }
}

/// Performs one action on the wire.
fn perform(connected: &mut Connected, action: Action, sink: &Sink) {
    match action {
        Action::Seat { capabilities } => advertise(connected, capabilities),
        Action::Device {
            device,
            capabilities,
            regions,
        } => create_device(connected, device, capabilities, &regions),
        Action::DestroyDevice { device } => {
            if let Some((wire, _)) = connected.devices.iter().find(|(_, identity)| *identity == device) {
                wire.remove();
            }
            connected.devices.retain(|(_, identity)| *identity != device);
        }
        Action::Inject(injection) => sink(injection.into_event()),
        Action::End { ending } => {
            let (reason, explanation) = ending_of(ending);
            connected.connection.disconnected(reason, explanation.as_deref());
            connected.ended = true;
        }
    }
}

/// The wire reason for an ending.
///
/// A deliberate disconnect states no explanation: the protocol reserves one for an
/// error, so a refusal is told as a plain disconnect rather than as a fault the client
/// could not have avoided.
fn ending_of(ending: Ending) -> (eis::connection::DisconnectReason, Option<String>) {
    match ending {
        Ending::Disconnected | Ending::Refused => (eis::connection::DisconnectReason::Disconnected, None),
        Ending::Violation(violation) => (eis::connection::DisconnectReason::Protocol, Some(violation.to_string())),
    }
}

/// Advertises capabilities on the seat.
///
/// `reis` fixes the set a seat may be bound to when the seat is created, so a grant that
/// widens the capabilities cannot be advertised on the existing seat. The old seat and its
/// devices are removed and a new seat carries the new set; the client binds to that one.
fn advertise(connected: &mut Connected, capabilities: Caps) {
    if let Some(seat) = connected.seat.take() {
        seat.remove();
        connected.devices.clear();
    }
    connected.seat = Some(connected.connection.add_seat(Some("gnoblin"), flags(capabilities)));
}

/// Creates a device and remembers which context identity its events name.
fn create_device(connected: &mut Connected, device: DeviceId, capabilities: Caps, regions: &[Region]) {
    let Some(seat) = &connected.seat else {
        log::warn!(%device, "a device was asked for before the seat existed");
        return;
    };
    let profile = profile(capabilities);
    let name = device_name(capabilities);
    let absolute = capabilities.intersects(Caps::POINTER_ABSOLUTE | Caps::TOUCH);
    let added = seat.add_device(
        Some(name),
        eis::device::DeviceType::Virtual,
        flags(capabilities),
        |added| {
            if !absolute {
                return;
            }
            // The mapping id names the output the region is, which is how a client that
            // also receives a stream pairs the two; `ei_device.region` then states the
            // rectangle itself, and both must be sent before `ei_device.done`.
            for region in regions {
                let mapping = format!("gnoblin-output-{}", region.output.get());
                let wire = added.device();
                wire.region_mapping_id(&mapping);
                wire.region(
                    u32::try_from(region.rect.x).unwrap_or(0),
                    u32::try_from(region.rect.y).unwrap_or(0),
                    region.rect.width,
                    region.rect.height,
                    1.0,
                );
            }
        },
    );
    log::debug!(%device, ?profile, "a device was created");
    // The device is ready for the client to emulate on: `resumed` is the event that says
    // so, and the client answers it with `ei_device.start_emulating`.
    added.resumed();
    connected.devices.push((added, device));
}

/// A name a client can show in a device list.
const fn device_name(capabilities: Caps) -> &'static str {
    if capabilities.contains(Caps::KEYBOARD) {
        "gnoblin-keyboard"
    } else if capabilities.contains(Caps::TOUCH) {
        "gnoblin-touch"
    } else {
        "gnoblin-pointer"
    }
}

/// A label for logs, so a refused bind names what was asked for.
fn profile(capabilities: Caps) -> String {
    let mut names = Vec::new();
    for capability in capabilities.iter() {
        names.push(interface(capability).1);
    }
    names.join(",")
}

/// The protocol mask and interface name of a capability.
///
/// `reis` keeps its own copy of the names private, so the six strings live here. They
/// are the protocol's, not this server's choice.
const fn interface(capability: Caps) -> (u64, &'static str) {
    match capability {
        Caps::POINTER => (Caps::POINTER.bits() as u64, "ei_pointer"),
        Caps::POINTER_ABSOLUTE => (Caps::POINTER_ABSOLUTE.bits() as u64, "ei_pointer_absolute"),
        Caps::KEYBOARD => (Caps::KEYBOARD.bits() as u64, "ei_keyboard"),
        Caps::TOUCH => (Caps::TOUCH.bits() as u64, "ei_touchscreen"),
        Caps::SCROLL => (Caps::SCROLL.bits() as u64, "ei_scroll"),
        Caps::BUTTON => (Caps::BUTTON.bits() as u64, "ei_button"),
        _ => (0, ""),
    }
}

/// The identity a device on the wire was created with.
fn identity_of(devices: &[(reis::request::Device, DeviceId)], wire: &reis::request::Device) -> Option<DeviceId> {
    devices
        .iter()
        .find(|(known, _)| known == wire)
        .map(|(_, identity)| *identity)
}

/// The capability flags `reis` takes.
fn flags(capabilities: Caps) -> BitFlags<DeviceCapability> {
    let mut flags = BitFlags::empty();
    for capability in capabilities.iter() {
        flags |= match capability {
            Caps::POINTER => DeviceCapability::Pointer,
            Caps::POINTER_ABSOLUTE => DeviceCapability::PointerAbsolute,
            Caps::KEYBOARD => DeviceCapability::Keyboard,
            Caps::TOUCH => DeviceCapability::Touch,
            Caps::SCROLL => DeviceCapability::Scroll,
            Caps::BUTTON => DeviceCapability::Button,
            _ => continue,
        };
    }
    flags
}

/// One high-level request as a command for the state machine.
///
/// Requests with no seat event are dropped with a debug line rather than guessed at.
/// `ei_scroll.scroll_stop` is the one that matters: the seat has no event for "the
/// scroll stopped", so the momentum it would start is not something this server can
/// ask for, and inventing an axis event would scroll a client by an amount nobody
/// sent. Text events are dropped for the same reason: the seat has no text path yet.
fn command(request: EisRequest, devices: &[(reis::request::Device, DeviceId)]) -> Option<EiRequest> {
    let device = |wire: &reis::request::Device| identity_of(devices, wire);
    Some(match request {
        EisRequest::Disconnect => EiRequest::Disconnect,
        EisRequest::Bind(bind) => EiRequest::Bind {
            capabilities: Caps::from_bits(u8::try_from(bind.capabilities.bits()).unwrap_or(u8::MAX)),
        },
        EisRequest::DeviceClosed(closed) => EiRequest::Release {
            device: device(&closed.device)?,
        },
        EisRequest::DeviceStartEmulating(started) => EiRequest::StartEmulating {
            device: device(&started.device)?,
            sequence: started.sequence,
        },
        EisRequest::DeviceStopEmulating(stopped) => EiRequest::StopEmulating {
            device: device(&stopped.device)?,
        },
        EisRequest::Frame(frame) => EiRequest::Frame {
            device: device(&frame.device)?,
        },
        EisRequest::PointerMotion(motion) => EiRequest::RelativeMotion {
            device: device(&motion.device)?,
            dx: motion.dx,
            dy: motion.dy,
        },
        EisRequest::PointerMotionAbsolute(motion) => EiRequest::AbsoluteMotion {
            device: device(&motion.device)?,
            x: motion.dx_absolute,
            y: motion.dy_absolute,
        },
        EisRequest::Button(button) => EiRequest::Button {
            device: device(&button.device)?,
            button: button.button,
            pressed: matches!(button.state, eis::button::ButtonState::Press),
        },
        EisRequest::KeyboardKey(key) => EiRequest::Key {
            device: device(&key.device)?,
            key: key.key,
            pressed: matches!(key.state, eis::keyboard::KeyState::Press),
        },
        EisRequest::ScrollDiscrete(scroll) => EiRequest::ScrollDiscrete {
            device: device(&scroll.device)?,
            dx: scroll.discrete_dx,
            dy: scroll.discrete_dy,
        },
        EisRequest::ScrollDelta(scroll) => EiRequest::ScrollDelta {
            device: device(&scroll.device)?,
            dx: scroll.dx,
            dy: scroll.dy,
        },
        EisRequest::TouchDown(touch) => EiRequest::TouchDown {
            device: device(&touch.device)?,
            contact: touch.touch_id,
            x: touch.x,
            y: touch.y,
        },
        EisRequest::TouchMotion(touch) => EiRequest::TouchMotion {
            device: device(&touch.device)?,
            contact: touch.touch_id,
            x: touch.x,
            y: touch.y,
        },
        EisRequest::TouchUp(touch) => EiRequest::TouchUp {
            device: device(&touch.device)?,
            contact: touch.touch_id,
        },
        EisRequest::TouchCancel(touch) => EiRequest::TouchCancel {
            device: device(&touch.device)?,
            contact: touch.touch_id,
        },
        EisRequest::ScrollStop(stop) => {
            log::debug!(x = stop.x, y = stop.y, "a scroll stop has no seat event");
            return None;
        }
        EisRequest::ScrollCancel(stop) => {
            log::debug!(x = stop.x, y = stop.y, "a scroll cancel has no seat event");
            return None;
        }
        EisRequest::TextKeysym(_) | EisRequest::TextUtf8(_) => {
            log::debug!("a text event has no seat path yet");
            return None;
        }
        EisRequest::Ready(_) | EisRequest::RequestDevice(_) => return None,
    })
}
