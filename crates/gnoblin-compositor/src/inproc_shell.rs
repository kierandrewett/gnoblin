//! A shell that runs inside the compositor process, on a thread of its own (design/modular-framework.md, D1 and M2.8).
//!
//! The compositor thread owns Smithay, the calloop loop and the scene, and it **never runs shell code**: a shell that is
//! slow or hung cannot stall a frame. The shell runs on [`InProcShell`]'s thread and talks to the tree through an
//! [`InProcLink`], an implementation of the same `TreeLink` a socket shell uses, so a shell runtime (the JavaScript one, a
//! native module through the shell host) cannot tell which one it was given.
//!
//! # One tree host
//!
//! The link calls the tree host's own `attach`, `transact` and `detach` (`ipc::tree_host`), the calls the socket's
//! dispatcher makes for a socket connection. The single holder, the epoch, the snapshot, the refusals, the transaction
//! rules and the 4 ms frame budget are therefore the socket's, not a copy of them. What differs is only where the frame's
//! answer goes: a socket connection hears it from the dispatcher, written as frames; this connection hears it through a
//! [`Mailbox`], which the frame fills from its own thread without waiting and the shell thread drains. The messages a
//! delivery is, and their order, are decided once (`TreeDelivery::into_messages`) and used by both.
//!
//! # A hung shell
//!
//! The frame only ever *pushes* into the mailbox: a short lock and an eventfd write, never a wait for the shell. A shell
//! that stops draining it therefore costs nothing but its own mailbox, and that is bounded: more than
//! [`MailboxLimits::items`] waiting, or nothing taken for [`MailboxLimits::stall`] (the socket's own limit), and the
//! shell is **detached**, as the dispatcher drops a socket client that does not read. The hold is released, the shell's
//! nodes are reaped and the compositor's window management takes the windows again. A locked session is protected by the
//! lock lease before that (`LOCK_UI_LEASE_MS`): a shell that stops sending heartbeats loses the lock UI to the built-in
//! card whether it is hung or only stopped renewing.
//!
//! # When the shell ends
//!
//! The shell's entry may return, return an error or panic. The thread contains all three the same way: the mailbox is
//! closed, the shell is detached (so `holder_attached` is cleared and the standard window manager, or the null one, takes
//! over), the end is logged as an error when it was not a requested stop, and [`ShellReports::ended`] is told. The
//! compositor continues. A segfault or an abort in native shell code cannot be contained in process (design section
//! 5.7, 6); that is the price of the default, and `isolate` is the way back.
//!
//! # Stopping with the compositor
//!
//! [`InProcShell::stop`] asks the shell to stop ([`ShellStop`], and its link reports the end of the compositor), waits up
//! to a grace for the thread to end and joins it. A thread that does not end within the grace is *abandoned*: it cannot be
//! killed, and waiting for it would hold the compositor's exit hostage to the shell, so the compositor logs it and goes
//! on; the process's exit ends it.

use std::collections::VecDeque;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_proto::tree::tree::TransactArgs;
use gnoblin_tree_client::tree_link::{
    incoming_answer, incoming_attached, incoming_changes, incoming_chord, incoming_event, incoming_gesture,
    incoming_pointer_button, incoming_pointer_chord, incoming_reply, incoming_snapshot, incoming_track_ended, to_op,
};
use gnoblin_tree_client::tree_mirror::{LinkError, TreeIncoming, TreeLink, TreeTx};
use gnoblin_types::ClientId;

use crate::ipc::{AttachMode, LocalSink, STALL_LIMIT, TREE_VERSION, TreeDelivery, TreeHostHandle, TreeMessage};
use crate::log;

/// The identity of the first in-process shell. Socket connections are numbered up from one, so the identities of shells
/// served in process start far above any a session hands out.
const FIRST_CLIENT: u64 = 1 << 63;

/// How long [`InProcShell::stop`] waits for the shell's thread to end by default.
pub const STOP_GRACE: Duration = Duration::from_secs(2);

/// What a failed shell is: the error its entry returned.
pub type ShellError = Box<dyn std::error::Error + Send + Sync>;

/// The code of a shell: it is given its link and its context on the shell thread and returns when the shell is done. A
/// shell that owns an event loop of its own sleeps on [`TreeLink::wait_fd`] in it (the descriptor is readable when the
/// compositor sent something, and when the compositor is stopping).
pub type ShellEntry = Box<dyn FnOnce(Box<dyn TreeLink>, InProcContext) -> Result<(), ShellError> + Send>;

/// How a shell ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEnd {
    /// The entry returned.
    Returned,
    /// The entry returned an error.
    Failed(String),
    /// The entry panicked.
    Panicked(String),
    /// The thread did not end within the grace of a stop, and was left to the process's exit.
    Abandoned,
}

impl std::fmt::Display for ShellEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Returned => f.write_str("the shell returned"),
            Self::Failed(why) => write!(f, "the shell failed: {why}"),
            Self::Panicked(why) => write!(f, "the shell panicked: {why}"),
            Self::Abandoned => f.write_str("the shell did not stop within the grace and was abandoned"),
        }
    }
}

/// What the compositor reports to the guardian on behalf of an in-process shell.
///
/// A shell in its own process reports over the descriptor the guardian gave it (`gnoblin_proto::report`); an in-process
/// shell has no such process, so it calls this instead and the compositor, which holds the guardian's descriptor, writes
/// the report. The four reports are the ones the script runtime sends (`gnoblin-js`'s runner): one `autostart` per
/// enabled entry of a configuration revision, then `config-accepted` or `config-rejected`, and `ready` once the shell
/// serves. This is the interface; the startup work decides what the guardian does with them and fills it in. Every
/// method defaults to doing nothing, so a shell that is run without a guardian reports into the void, as a child with no
/// report descriptor does.
pub trait ShellReports: Send + Sync {
    /// An enabled autostart entry of configuration `revision`.
    fn autostart(&self, _revision: u64, _name: &str, _command: &[String]) {}

    /// The shell accepted configuration `revision`.
    fn config_accepted(&self, _revision: u64) {}

    /// The shell refused a configuration revision, and the last accepted one stays in force.
    fn config_rejected(&self, _revision: u64, _diagnostic: &str) {}

    /// The shell serves, at configuration `revision`.
    fn ready(&self, _revision: u64) {}

    /// The shell's thread ended. Called once, on the shell's thread for a return, an error or a panic, and on the stopping
    /// thread for an abandoned shell.
    fn ended(&self, _end: &ShellEnd) {}
}

/// The reports of a session that has no guardian to tell.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoReports;

impl ShellReports for NoReports {}

/// The reports an in-process shell makes, written on the guardian's report descriptor in the wire form of
/// [`gnoblin_proto::report`] (the compositor's own `ready` and `x11` reports go down the same descriptor, so the reporter
/// is shared and each report is one whole write).
///
/// - `autostart`, `config_accepted` and `config_rejected` are the script runtime's three lines, unchanged.
/// - `ready` is the runtime's `ready` block with `GNOBLIN_CONFIG_REVISION`. The compositor reported its own `ready` when it
///   began to serve, so a guardian that sees this one has a duplicate of a report it has acted on and ignores it (it says so
///   at debug level); the block is kept so a shell that is not a script reports as a script does, and so a guardian that
///   one day waits for the shell has the report to wait for.
/// - `ended` is `config-rejected` with revision `0` ("the document never got that far") and the way the shell ended, for a
///   shell that failed, panicked or was abandoned: the guardian logs the warning and records the status, and the session
///   continues, because the compositor does. A shell that returned is not reported: it ended on purpose or with the session.
///
/// What it was told is passed on to the spec's own reports afterwards, so a test's hook still sees every report.
pub struct GuardianReports {
    reporter: Arc<Mutex<gnoblin_client::Reporter>>,
    then: Arc<dyn ShellReports>,
}

impl GuardianReports {
    fn write(&self, text: &str) {
        self.reporter.lock().unwrap_or_else(PoisonError::into_inner).write(text);
    }
}

impl ShellReports for GuardianReports {
    fn autostart(&self, revision: u64, name: &str, command: &[String]) {
        self.write(&gnoblin_proto::report::autostart(revision, name, command));
        self.then.autostart(revision, name, command);
    }

    fn config_accepted(&self, revision: u64) {
        self.write(&gnoblin_proto::report::config_accepted(revision));
        self.then.config_accepted(revision);
    }

    fn config_rejected(&self, revision: u64, diagnostic: &str) {
        self.write(&gnoblin_proto::report::config_rejected(revision, diagnostic));
        self.then.config_rejected(revision, diagnostic);
    }

    fn ready(&self, revision: u64) {
        self.write(&gnoblin_proto::report::ready(&[(
            "GNOBLIN_CONFIG_REVISION".to_owned(),
            revision.to_string(),
        )]));
        self.then.ready(revision);
    }

    fn ended(&self, end: &ShellEnd) {
        if !matches!(end, ShellEnd::Returned) {
            self.write(&gnoblin_proto::report::config_rejected(0, &end.to_string()));
        }
        self.then.ended(end);
    }
}

/// A request, from the compositor, that the shell stop. The shell's link reports it too (its descriptor turns readable and
/// its `poll` fails), so a shell that sleeps on the link wakes without watching this.
#[derive(Clone, Debug, Default)]
pub struct ShellStop {
    requested: Arc<AtomicBool>,
}

impl ShellStop {
    /// Whether the compositor asked the shell to stop.
    #[must_use]
    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

/// What a shell is given besides its link.
pub struct InProcContext {
    /// Whether the compositor asked the shell to stop.
    pub stop: ShellStop,
    /// Where the shell reports to the guardian.
    pub reports: Arc<dyn ShellReports>,
}

impl std::fmt::Debug for InProcContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcContext")
            .field("stop", &self.stop)
            .finish_non_exhaustive()
    }
}

/// How much a shell that does not drain its mailbox may hold the compositor to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MailboxLimits {
    /// The most items waiting for the shell: a batch counts its changes, events, answers and track ends, a snapshot its
    /// nodes. A socket client is held to a byte count of what waits for it; an in-process mailbox holds values, not
    /// frames, so it is counted in them.
    pub items: usize,
    /// How long something may wait with nothing taken, the socket's own limit.
    pub stall: Duration,
}

impl Default for MailboxLimits {
    fn default() -> Self {
        Self {
            items: 1 << 16,
            stall: STALL_LIMIT,
        }
    }
}

/// A shell to run in the compositor process, and how.
pub struct InProcShellSpec {
    name: String,
    entry: ShellEntry,
    reports: Arc<dyn ShellReports>,
    limits: MailboxLimits,
    grace: Duration,
}

impl InProcShellSpec {
    /// A shell called `name` (the thread's name, and the name in the log) that runs `entry`.
    pub fn new(
        name: impl Into<String>,
        entry: impl FnOnce(Box<dyn TreeLink>, InProcContext) -> Result<(), ShellError> + Send + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            entry: Box::new(entry),
            reports: Arc::new(NoReports),
            limits: MailboxLimits::default(),
            grace: STOP_GRACE,
        }
    }

    /// Where the shell reports to the guardian, instead of nowhere.
    #[must_use]
    pub fn with_reports(mut self, reports: Arc<dyn ShellReports>) -> Self {
        self.reports = reports;
        self
    }

    /// Also reports to the guardian over `reporter`, the compositor's own report descriptor, in front of the reports the spec
    /// already had ([`GuardianReports`]).
    #[must_use]
    pub(crate) fn reporting_to(mut self, reporter: Arc<Mutex<gnoblin_client::Reporter>>) -> Self {
        let then = std::mem::replace(&mut self.reports, Arc::new(NoReports));
        self.reports = Arc::new(GuardianReports { reporter, then });
        self
    }

    /// The limits of the shell's mailbox, instead of the defaults.
    #[must_use]
    pub const fn with_limits(mut self, limits: MailboxLimits) -> Self {
        self.limits = limits;
        self
    }

    /// How long the compositor's exit waits for the shell's thread, instead of [`STOP_GRACE`].
    #[must_use]
    pub const fn with_stop_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// The name of the shell.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Debug for InProcShellSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcShellSpec")
            .field("name", &self.name)
            .field("limits", &self.limits)
            .field("grace", &self.grace)
            .finish_non_exhaustive()
    }
}

/// How many items a delivery is, for the mailbox's bound.
fn weight(delivery: &TreeDelivery) -> usize {
    match delivery {
        TreeDelivery::Changed(batch) => {
            1 + batch.changes.len() + batch.events.len() + batch.answers.len() + batch.tracks.len()
        }
        TreeDelivery::Attached {
            outcome: Ok((_, snapshot)),
            ..
        } => 1 + snapshot.chunks.iter().map(Vec::len).sum::<usize>(),
        TreeDelivery::Attached { .. } | TreeDelivery::Reply { .. } => 1,
    }
}

/// What the frame has told the shell and the shell has not yet read, and the descriptor that says so.
struct Mailbox {
    limits: MailboxLimits,
    state: Mutex<MailboxState>,
    /// An eventfd: readable while something waits, and for good once the mailbox is closed.
    wake: OwnedFd,
}

#[derive(Default)]
struct MailboxState {
    queue: VecDeque<TreeDelivery>,
    items: usize,
    /// When the oldest delivery that is still waiting arrived.
    waiting_since: Option<Instant>,
    /// Why the shell is no longer served, once it is not. What is waiting is still read first.
    closed: Option<String>,
}

impl Mailbox {
    fn new(limits: MailboxLimits) -> std::io::Result<Self> {
        let wake = rustix::event::eventfd(
            0,
            rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
        )?;
        Ok(Self {
            limits,
            state: Mutex::new(MailboxState::default()),
            wake,
        })
    }

    fn state(&self) -> MutexGuard<'_, MailboxState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn signal(&self) {
        // A full counter or a closed descriptor has nothing more to say: the shell is already told, or gone.
        let _ = rustix::io::write(&self.wake, &1u64.to_ne_bytes());
    }

    /// Queues `delivery` and wakes the shell. `false` when the shell took nothing for too long or has too much waiting:
    /// the mailbox is then closed, and the caller detaches the shell.
    fn push(&self, delivery: TreeDelivery) -> bool {
        let now = Instant::now();
        let mut state = self.state();
        if state.closed.is_some() {
            // The shell is gone or stopping; nobody reads this.
            return true;
        }
        let stalled = state
            .waiting_since
            .is_some_and(|since| now.saturating_duration_since(since) > self.limits.stall);
        let weight = weight(&delivery);
        if stalled || state.items + weight > self.limits.items {
            let why = if stalled {
                format!("the shell took nothing for {} seconds", self.limits.stall.as_secs_f64())
            } else {
                format!("more than {} items would wait for the shell", self.limits.items)
            };
            state.queue.clear();
            state.items = 0;
            state.closed = Some(why);
            drop(state);
            self.signal();
            return false;
        }
        state.items += weight;
        state.waiting_since.get_or_insert(now);
        state.queue.push_back(delivery);
        drop(state);
        self.signal();
        true
    }

    /// Takes everything that waits, in arrival order. Once the mailbox is closed and empty this is the error the shell
    /// reads as a closed socket.
    fn drain(&self) -> Result<Vec<TreeDelivery>, LinkError> {
        let mut state = self.state();
        if state.queue.is_empty()
            && let Some(why) = &state.closed
        {
            return Err(LinkError(why.clone()));
        }
        let taken: Vec<TreeDelivery> = state.queue.drain(..).collect();
        state.items = 0;
        state.waiting_since = None;
        if state.closed.is_none() {
            // Cleared under the lock, so a push that follows signals again: no delivery is left unsignalled.
            let mut counter = [0u8; 8];
            let _ = rustix::io::read(&self.wake, &mut counter);
        }
        Ok(taken)
    }

    /// Ends the service: what is waiting is still read, and then every read is `why`.
    fn close(&self, why: &str) {
        let mut state = self.state();
        if state.closed.is_none() {
            state.closed = Some(why.to_owned());
        }
        drop(state);
        self.signal();
    }
}

impl LocalSink for Mailbox {
    fn deliver(&self, delivery: TreeDelivery) -> bool {
        self.push(delivery)
    }
}

/// The shell's end of the tree: a [`TreeLink`] that calls the tree host in this process and reads what the frame tells it
/// from a mailbox. It is what a shell is handed instead of a socket.
pub struct InProcLink {
    host: TreeHostHandle,
    client: ClientId,
    mailbox: Arc<Mailbox>,
    next_request: u64,
    pending: Vec<TreeIncoming>,
}

impl InProcLink {
    fn take_request_id(&mut self) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        id
    }
}

impl TreeLink for InProcLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        if version != TREE_VERSION {
            return Err(LinkError(format!("tree version {version} is not served")));
        }
        let id = self.take_request_id();
        // The in-process shell is the session owner's by construction: the compositor started it.
        self.host.attach(self.client, id, true, AttachMode::Holder);
        Ok(())
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        // The same wire form, and the same conversion in the host, as a socket shell's transaction, so a write is
        // admitted or refused identically. The host takes the ops out of the arguments, and the call texts with them.
        let args = TransactArgs {
            base_serial: tx.base_serial,
            serial: tx.serial,
            ops: tx.ops.iter().map(to_op).collect(),
        };
        let id = self.take_request_id();
        if let Some(reply) = self.host.transact(self.client, id, args) {
            // Decided at once (not the holder, or a transaction that does not convert): answered through the mailbox,
            // in order with everything else, as the dispatcher answers on the connection.
            self.host
                .deliver(self.client, TreeDelivery::Reply { request_id: id, reply });
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        for delivery in self.mailbox.drain()? {
            for message in delivery.into_messages() {
                self.pending.push(incoming(message)?);
            }
        }
        Ok(std::mem::take(&mut self.pending))
    }

    fn wait_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.mailbox.wake.as_fd())
    }
}

/// One message as the shell reads it: the socket link's own translation of the same payload.
fn incoming(message: TreeMessage) -> Result<TreeIncoming, LinkError> {
    match message {
        TreeMessage::Attached { payload, .. } => Ok(incoming_attached(payload)),
        TreeMessage::AttachRefused { code, .. } => {
            Err(LinkError(format!("the compositor refused tree attach: {code}")))
        }
        TreeMessage::Snapshot(payload) => incoming_snapshot(payload),
        TreeMessage::Replied { payload, .. } => Ok(incoming_reply(payload)),
        TreeMessage::Changes(payload) => incoming_changes(payload),
        TreeMessage::Answer(payload) => Ok(incoming_answer(payload)),
        TreeMessage::TrackEnded(payload) => Ok(incoming_track_ended(payload)),
        TreeMessage::Chord(payload) => Ok(incoming_chord(payload)),
        TreeMessage::Gesture(payload) => Ok(incoming_gesture(payload)),
        TreeMessage::PointerChord(payload) => Ok(incoming_pointer_chord(payload)),
        TreeMessage::PointerButton(payload) => Ok(incoming_pointer_button(payload)),
        TreeMessage::Event(payload) => Ok(incoming_event(payload)),
    }
}

/// How the shell's thread ended, for the thread that stops it to wait on.
struct Ended {
    end: Mutex<Option<ShellEnd>>,
    ended: Condvar,
    /// Whether [`ShellReports::ended`] has been called: a thread abandoned by a stop that ends later does not report
    /// again.
    reported: AtomicBool,
}

/// A shell running on its own thread in the compositor process, and what stops it.
pub(crate) struct InProcShell {
    name: String,
    client: ClientId,
    host: TreeHostHandle,
    mailbox: Arc<Mailbox>,
    stop: ShellStop,
    reports: Arc<dyn ShellReports>,
    grace: Duration,
    ended: Arc<Ended>,
    thread: Option<JoinHandle<()>>,
}

impl InProcShell {
    /// Starts `spec`'s shell on a thread of its own, served by `host`. The shell attaches itself, through its link, when
    /// it is ready; nothing is attached here.
    ///
    /// # Errors
    ///
    /// The mailbox's descriptor or the thread could not be created.
    pub(crate) fn spawn(host: &TreeHostHandle, spec: InProcShellSpec) -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let client = ClientId::new(FIRST_CLIENT + NEXT.fetch_add(1, Ordering::Relaxed));
        let mailbox = Arc::new(Mailbox::new(spec.limits)?);
        let stop = ShellStop::default();
        let ended = Arc::new(Ended {
            end: Mutex::new(None),
            ended: Condvar::new(),
            reported: AtomicBool::new(false),
        });
        host.serve_locally(client, Arc::clone(&mailbox) as Arc<dyn LocalSink>);
        let InProcShellSpec {
            name,
            entry,
            reports,
            grace,
            ..
        } = spec;
        let thread = std::thread::Builder::new().name(format!("gnoblin-shell-{name}")).spawn({
            let link = InProcLink {
                host: host.clone(),
                client,
                mailbox: Arc::clone(&mailbox),
                next_request: 1,
                pending: Vec::new(),
            };
            let context = InProcContext {
                stop: stop.clone(),
                reports: Arc::clone(&reports),
            };
            let ended = Arc::clone(&ended);
            let host = host.clone();
            let mailbox = Arc::clone(&mailbox);
            let name = name.clone();
            let stop = stop.clone();
            let reports = Arc::clone(&reports);
            move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| entry(Box::new(link), context)));
                let end = match outcome {
                    Ok(Ok(())) => ShellEnd::Returned,
                    Ok(Err(error)) => ShellEnd::Failed(error.to_string()),
                    Err(payload) => ShellEnd::Panicked(panic_message(payload.as_ref())),
                };
                // Contained before anything is reported: the mailbox is closed and the shell detached, so the hold is
                // released, its nodes are reaped and the compositor's window management has the windows again.
                mailbox.close("the shell ended");
                host.detach_locally(client);
                match &end {
                    ShellEnd::Returned if stop.requested() => {
                        log::info!(area = "shell", shell = %name, "the in-process shell stopped");
                    }
                    ShellEnd::Returned => {
                        log::error!(area = "shell", shell = %name, "the in-process shell returned while the session runs, so the tree is detached");
                    }
                    ShellEnd::Failed(why) | ShellEnd::Panicked(why) => {
                        log::error!(area = "shell", shell = %name, %why, "the in-process shell ended with an error, so the tree is detached and the session continues without it");
                    }
                    ShellEnd::Abandoned => {}
                }
                *ended.end.lock().unwrap_or_else(PoisonError::into_inner) = Some(end.clone());
                ended.ended.notify_all();
                if !ended.reported.swap(true, Ordering::AcqRel) {
                    reports.ended(&end);
                }
            }
        });
        let thread = match thread {
            Ok(thread) => thread,
            Err(error) => {
                host.detach_locally(client);
                return Err(error);
            }
        };
        Ok(Self {
            name,
            client,
            host: host.clone(),
            mailbox,
            stop,
            reports,
            grace,
            ended,
            thread: Some(thread),
        })
    }

    /// Stops the shell with the compositor: asks it to stop, wakes it, waits up to the grace for its thread to end and
    /// joins it. A thread that does not end in time is abandoned, and the tree is detached for it. Returns how the shell
    /// ended.
    pub(crate) fn stop(mut self) -> ShellEnd {
        self.stop.requested.store(true, Ordering::Release);
        self.mailbox.close("the compositor is stopping");
        let deadline = Instant::now() + self.grace;
        let mut end = self.ended.end.lock().unwrap_or_else(PoisonError::into_inner);
        while end.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            end = self
                .ended
                .ended
                .wait_timeout(end, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let finished = end.clone();
        drop(end);
        if let Some(finished) = finished {
            // The thread has recorded its end and is about to return: the join is short.
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            return finished;
        }
        log::error!(area = "shell", shell = %self.name, grace_ms = self.grace.as_millis(), "the in-process shell did not stop in time and is abandoned");
        self.host.detach_locally(self.client);
        // The handle is dropped, which detaches the thread: it ends with the process.
        self.thread = None;
        let end = ShellEnd::Abandoned;
        if !self.ended.reported.swap(true, Ordering::AcqRel) {
            self.reports.ended(&end);
        }
        end
    }
}

/// The message of a panic payload: a `&str` or a `String`, and a note for anything else.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic that carried no text".to_owned())
}

#[cfg(test)]
mod tests {
    //! The shell host without a compositor: the test thread plays the compositor thread, driving a `TreeFrame` the way
    //! the session's loop does, and a real shell thread talks to it through the link.

    use std::sync::mpsc::{Receiver, channel};

    use gnoblin_render::transaction::{TreeOp, TreeTransaction};
    use gnoblin_render::{Actor, NodeSpec, Scene};
    use gnoblin_tree_client::tree_mirror::{TxOp, TxReply};
    use gnoblin_types::{NodeId, Rect};
    use gnoblin_wayland::TreeFrame;

    use super::*;
    use crate::ipc::IpcEvent;

    const ROOT: NodeId = NodeId::new(1);

    /// One tree host, its frame and its scene, driven from the test's thread.
    struct Rig {
        host: TreeHostHandle,
        frame: TreeFrame,
        scene: Scene,
        events: Receiver<IpcEvent>,
    }

    impl Rig {
        fn new() -> Self {
            let (sender, events) = crate::ipc::events();
            let host = TreeHostHandle::new(sender).expect("the host builds");
            let frame = TreeFrame::with_hooks(host.hooks());
            let mut scene = Scene::new();
            scene
                .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
                .expect("the root inserts");
            scene.set_root(Some(ROOT)).expect("the root is named");
            Self {
                host,
                frame,
                scene,
                events,
            }
        }

        fn pump(&mut self) {
            self.frame.drain(&mut self.scene, Instant::now);
        }

        /// Runs frames until `done`, as the compositor's loop would, or fails.
        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(self) {
                assert!(Instant::now() < deadline, "never saw {what}");
                self.pump();
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        /// Whether a second connection can take the tree now: the answer to its attach, decided by the frame.
        fn another_holder_may_attach(&mut self, client: u64) -> bool {
            self.host.attach(ClientId::new(client), 1, true, AttachMode::Holder);
            let mut outcome = None;
            self.until("the attach outcome", |rig| {
                while let Ok(event) = rig.events.try_recv() {
                    if let IpcEvent::Tree {
                        delivery: TreeDelivery::Attached { outcome: answer, .. },
                        ..
                    } = event
                    {
                        outcome = Some(answer.is_ok());
                    }
                }
                outcome.is_some()
            });
            if outcome == Some(true) {
                // Let go again, so the next question is asked of a free tree.
                self.host.detach(ClientId::new(client));
                self.pump();
                return true;
            }
            false
        }

        /// Queues a transaction of the compositor's own that adds a node under the root, so the frame publishes a batch.
        fn make_a_change(&self, id: u64) {
            let node = NodeId::new(id);
            self.frame.queue().submit(
                0,
                TreeTransaction::new(vec![
                    TreeOp::New {
                        class: "node".to_owned(),
                        id: node,
                    },
                    TreeOp::Insert {
                        node,
                        parent: ROOT,
                        index: None,
                    },
                ]),
                || true,
                |_, _| {},
            );
        }
    }

    /// Reads the link until `done` holds of everything read so far, sleeping on its descriptor between reads.
    fn read_until(link: &mut dyn TreeLink, mut done: impl FnMut(&[TreeIncoming]) -> bool) -> Vec<TreeIncoming> {
        let mut read = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&read) {
            assert!(Instant::now() < deadline, "the link never said it: {read:?}");
            if let Some(fd) = link.wait_fd() {
                let _ = gnoblin_client::wait_readable(&[fd], Duration::from_millis(50));
            }
            read.extend(link.poll().expect("the link is open"));
        }
        read
    }

    fn attached_and_snapshotted(read: &[TreeIncoming]) -> bool {
        read.iter().any(|message| matches!(message, TreeIncoming::Attached(_)))
            && read
                .iter()
                .any(|message| matches!(message, TreeIncoming::Snapshot { last: true, .. }))
    }

    fn reply_of(read: &[TreeIncoming]) -> Option<TxReply> {
        read.iter().find_map(|message| match message {
            TreeIncoming::Reply(reply) => Some(reply.clone()),
            _ => None,
        })
    }

    /// A latch a thread waits on until the test opens it.
    #[derive(Clone, Default)]
    struct Gate {
        open: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Gate {
        fn wait(&self) {
            let (open, changed) = &*self.open;
            let mut open = open.lock().unwrap_or_else(PoisonError::into_inner);
            while !*open {
                open = changed.wait(open).unwrap_or_else(PoisonError::into_inner);
            }
        }

        fn release(&self) {
            let (open, changed) = &*self.open;
            *open.lock().unwrap_or_else(PoisonError::into_inner) = true;
            changed.notify_all();
        }
    }

    /// Records how the shell ended.
    #[derive(Default)]
    struct Ends(Mutex<Vec<ShellEnd>>);

    impl Ends {
        fn all(&self) -> Vec<ShellEnd> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
        }
    }

    impl ShellReports for Ends {
        fn ended(&self, end: &ShellEnd) {
            self.0.lock().unwrap_or_else(PoisonError::into_inner).push(end.clone());
        }
    }

    /// A shell that attaches, makes a node under the root and reports the answer, then returns.
    fn shell_that_makes_a_node(answer: std::sync::mpsc::Sender<(u64, TxReply)>) -> InProcShellSpec {
        InProcShellSpec::new("makes-a-node", move |mut link, _context| {
            link.attach(1).expect("the attach is sent");
            let read = read_until(link.as_mut(), attached_and_snapshotted);
            let Some(TreeIncoming::Attached(attached)) = read.first() else {
                return Err("the attach answer comes first".into());
            };
            let id = (attached.epoch << 32) | 1;
            link.transact(&TreeTx {
                base_serial: attached.serial,
                serial: attached.serial + 1,
                ops: vec![
                    TxOp::New {
                        id,
                        class: "node".to_owned(),
                    },
                    TxOp::Insert {
                        node: id,
                        parent: attached.root,
                        index: 0,
                    },
                ],
            })
            .expect("the transaction is sent");
            let reply = reply_of(&read_until(link.as_mut(), |read| reply_of(read).is_some())).expect("a reply");
            answer.send((id, reply)).expect("the test is waiting");
            Ok(())
        })
    }

    #[test]
    fn a_shell_on_its_own_thread_attaches_reads_the_snapshot_and_transacts_through_the_link() {
        let mut rig = Rig::new();
        let (answer, answered) = channel();
        let shell = InProcShell::spawn(&rig.host, shell_that_makes_a_node(answer)).expect("the shell starts");
        let mut reply = None;
        rig.until("the shell's transaction", |_| {
            reply = answered.try_recv().ok();
            reply.is_some()
        });
        let (id, reply) = reply.expect("an answer");
        assert!(reply.applied, "the transaction applies: {reply:?}");
        assert!(rig.scene.contains(NodeId::new(id)), "the shell's node is in the scene");
        assert_eq!(shell.stop(), ShellEnd::Returned);
    }

    /// A shell that attaches and then waits on `gate` without ever reading its link, as a hung one does.
    fn shell_that_hangs(gate: &Gate, limits: MailboxLimits) -> (InProcShellSpec, Receiver<()>) {
        let held = gate.clone();
        let (attached, attached_rx) = channel();
        let spec = InProcShellSpec::new("hangs", move |mut link, _context| {
            link.attach(1).expect("the attach is sent");
            read_until(link.as_mut(), attached_and_snapshotted);
            attached.send(()).expect("the test is waiting");
            held.wait();
            Ok(())
        })
        .with_limits(limits)
        .with_stop_grace(Duration::from_millis(150));
        (spec, attached_rx)
    }

    #[test]
    fn a_second_holder_is_refused_while_the_shell_holds_the_tree_and_may_attach_once_the_shell_ended() {
        let mut rig = Rig::new();
        let gate = Gate::default();
        let (spec, attached) = shell_that_hangs(&gate, MailboxLimits::default());
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        rig.until("the shell's attach", |_| attached.try_recv().is_ok());
        assert!(rig.scene.holder_attached(), "the holder is attached");
        assert!(!rig.another_holder_may_attach(5), "a second holder is refused");

        gate.release();
        assert_eq!(shell.stop(), ShellEnd::Returned);
        // The shell ended, so the hold is released.
        rig.until("the hold released", |rig| rig.another_holder_may_attach(6));
    }

    /// How a shell that attaches and then ends the way `ends` says was ended and reported, with the tree free for another.
    fn ended_as(ends: impl FnOnce() -> Result<(), ShellError> + Send + 'static) -> (ShellEnd, Vec<ShellEnd>) {
        let mut rig = Rig::new();
        let reports = Arc::new(Ends::default());
        let spec = InProcShellSpec::new("ends", move |mut link, _context| {
            link.attach(1).expect("the attach is sent");
            read_until(link.as_mut(), attached_and_snapshotted);
            ends()
        })
        .with_reports(reports.clone());
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        rig.until("the shell's end", |_| !reports.all().is_empty());
        let stopped = shell.stop();
        // The compositor goes on: the frame still runs, and no holder is left to refuse another.
        rig.until("the hold released", |rig| rig.another_holder_may_attach(9));
        assert!(!rig.scene.holder_attached(), "the window manager has the windows again");
        (stopped, reports.all())
    }

    #[test]
    fn a_shell_that_returns_is_detached_and_reported_once() {
        let (end, reported) = ended_as(|| Ok(()));
        assert_eq!(end, ShellEnd::Returned);
        assert_eq!(reported, [ShellEnd::Returned]);
    }

    #[test]
    fn a_shell_that_fails_is_detached_and_reported_with_its_error() {
        let (end, reported) = ended_as(|| Err("no fonts".into()));
        assert_eq!(end, ShellEnd::Failed("no fonts".to_owned()));
        assert_eq!(reported, [end]);
    }

    #[test]
    fn a_shell_that_panics_is_contained_detached_and_reported_with_its_message() {
        let (end, reported) = ended_as(|| panic!("the runtime fell over"));
        assert_eq!(end, ShellEnd::Panicked("the runtime fell over".to_owned()));
        assert_eq!(reported, [end]);
    }

    #[test]
    fn a_hung_shell_costs_the_frame_nothing_and_is_detached_once_its_mailbox_is_over_the_limit() {
        let mut rig = Rig::new();
        let gate = Gate::default();
        let (spec, attached) = shell_that_hangs(
            &gate,
            MailboxLimits {
                items: 64,
                stall: Duration::from_secs(3600),
            },
        );
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        rig.until("the shell's attach", |_| attached.try_recv().is_ok());

        // Frames that each publish a batch to a shell that reads none of them: none may wait for the shell.
        let mut slowest = Duration::ZERO;
        for id in 0x100..0x140 {
            rig.make_a_change(id);
            let started = Instant::now();
            rig.pump();
            slowest = slowest.max(started.elapsed());
        }
        assert!(
            slowest < Duration::from_millis(250),
            "a frame waited for the hung shell: {slowest:?}"
        );
        // Over the limit: the shell was detached, and another may hold the tree.
        rig.until("the hung shell detached", |rig| rig.another_holder_may_attach(7));

        gate.release();
        assert_eq!(shell.stop(), ShellEnd::Returned);
    }

    #[test]
    fn a_shell_that_takes_nothing_for_the_stall_limit_is_detached() {
        let mut rig = Rig::new();
        let gate = Gate::default();
        let (spec, attached) = shell_that_hangs(
            &gate,
            MailboxLimits {
                items: usize::MAX / 2,
                stall: Duration::from_millis(100),
            },
        );
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        rig.until("the shell's attach", |_| attached.try_recv().is_ok());
        rig.make_a_change(0x100);
        rig.pump();
        assert!(
            !rig.another_holder_may_attach(7),
            "inside the stall limit the shell still holds the tree"
        );
        std::thread::sleep(Duration::from_millis(150));
        rig.make_a_change(0x101);
        rig.until("the stalled shell detached", |rig| rig.another_holder_may_attach(8));
        gate.release();
        assert_eq!(shell.stop(), ShellEnd::Returned);
    }

    #[test]
    fn stop_wakes_a_shell_asleep_on_its_link_and_joins_its_thread() {
        let mut rig = Rig::new();
        let (attached, attached_rx) = channel();
        let spec = InProcShellSpec::new("sleeps", move |mut link, context| {
            link.attach(1).expect("the attach is sent");
            read_until(link.as_mut(), attached_and_snapshotted);
            attached.send(()).expect("the test is waiting");
            // Asleep on the link for as long as it takes: only the end of the compositor wakes it.
            while link.poll().is_ok() && !context.stop.requested() {
                if let Some(fd) = link.wait_fd() {
                    let _ = gnoblin_client::wait_readable(&[fd], Duration::from_secs(60));
                }
            }
            Ok(())
        });
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        rig.until("the shell's attach", |_| attached_rx.try_recv().is_ok());
        let started = Instant::now();
        assert_eq!(shell.stop(), ShellEnd::Returned);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the stop waited for the sleeping shell"
        );
    }

    #[test]
    fn stop_abandons_a_shell_that_will_not_end_after_the_grace_and_reports_it_once() {
        let mut rig = Rig::new();
        let gate = Gate::default();
        let reports = Arc::new(Ends::default());
        let (spec, attached) = shell_that_hangs(&gate, MailboxLimits::default());
        let shell = InProcShell::spawn(&rig.host, spec.with_reports(reports.clone())).expect("the shell starts");
        rig.until("the shell's attach", |_| attached.try_recv().is_ok());
        let started = Instant::now();
        assert_eq!(shell.stop(), ShellEnd::Abandoned);
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(150) && waited < Duration::from_secs(5),
            "{waited:?}"
        );
        // The abandoned shell's hold is released, so the compositor's window management has the windows.
        rig.until("the hold released", |rig| rig.another_holder_may_attach(7));
        // When the thread does end it does not report again.
        gate.release();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(reports.all(), [ShellEnd::Abandoned]);
    }

    #[test]
    fn a_transaction_before_the_shell_holds_the_tree_is_refused_through_the_mailbox_in_order() {
        let mut rig = Rig::new();
        let (answer, answered) = channel();
        let spec = InProcShellSpec::new("early", move |mut link, _context| {
            link.transact(&TreeTx {
                base_serial: 0,
                serial: 1,
                ops: vec![TxOp::New {
                    id: 1 << 32,
                    class: "node".to_owned(),
                }],
            })
            .expect("the transaction is sent");
            answer
                .send(reply_of(&read_until(link.as_mut(), |read| reply_of(read).is_some())))
                .expect("the test is waiting");
            Ok(())
        });
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        let reply = answered
            .recv_timeout(Duration::from_secs(10))
            .expect("the shell answered");
        rig.pump();
        assert_eq!(reply.expect("a reply").code, "not-privileged");
        assert_eq!(shell.stop(), ShellEnd::Returned);
    }

    /// A reporter whose descriptor is a pipe the test reads, and the pipe's read end.
    fn pipe_reporter() -> (Arc<Mutex<gnoblin_client::Reporter>>, std::io::PipeReader) {
        use std::os::fd::IntoRawFd as _;
        let (read, write) = std::io::pipe().expect("a pipe");
        let number = write.into_raw_fd().to_string();
        let reporter = gnoblin_client::Reporter::new(move |name| {
            (name == gnoblin_proto::report::READY_VARIABLE).then(|| number.clone())
        });
        (Arc::new(Mutex::new(reporter)), read)
    }

    #[test]
    fn the_guardian_hears_what_the_shell_reports_in_the_script_runtimes_wire_form() {
        use std::io::Read as _;
        let (reporter, mut read) = pipe_reporter();
        let seen = Arc::new(Ends::default());
        let spec = InProcShellSpec::new("reports", |_link, context| {
            context.reports.autostart(0, "bar", &["waybar".to_owned()]);
            context.reports.config_accepted(0);
            context.reports.ready(0);
            Err("a failure after the reports".into())
        })
        .with_reports(seen.clone())
        .reporting_to(reporter.clone());
        let rig = Rig::new();
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        let mut waited = 0;
        while seen.all().is_empty() && waited < 1000 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        let end = shell.stop();
        assert_eq!(end, ShellEnd::Failed("a failure after the reports".to_owned()));
        // The shell's thread has ended and dropped its copy of the reporter; this is the last one, and dropping it closes the
        // pipe's write end, which is what lets the test read to the end.
        drop(reporter);
        let mut text = String::new();
        read.read_to_string(&mut text).expect("the report");
        assert_eq!(
            text,
            "autostart 0 bar waybar\nconfig-accepted 0\nready\nGNOBLIN_CONFIG_REVISION=0\n\n\
             config-rejected 0 the shell failed: a failure after the reports\n"
        );
        assert_eq!(seen.all(), [ShellEnd::Failed("a failure after the reports".to_owned())]);
    }

    #[test]
    fn a_shell_that_returns_is_not_reported_to_the_guardian_as_a_failure() {
        use std::io::Read as _;
        let (reporter, mut read) = pipe_reporter();
        let seen = Arc::new(Ends::default());
        let spec = InProcShellSpec::new("returns", |_link, _context| Ok(()))
            .with_reports(seen.clone())
            .reporting_to(reporter.clone());
        let rig = Rig::new();
        let shell = InProcShell::spawn(&rig.host, spec).expect("the shell starts");
        let mut waited = 0;
        while seen.all().is_empty() && waited < 1000 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        assert_eq!(shell.stop(), ShellEnd::Returned);
        drop(reporter);
        let mut text = String::new();
        read.read_to_string(&mut text).expect("the report");
        assert_eq!(text, "");
    }
}
