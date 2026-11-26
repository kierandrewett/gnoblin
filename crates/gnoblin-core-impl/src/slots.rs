//! The shell's slot connection: the requests the compositor offers to a slot the shell claimed.
//!
//! The tree socket carries the `tree` namespace only. A slot is claimed, and a request offered to it is claimed and answered, on the
//! owner socket, so a shell that wants the compositor's own dialogs (consent, passphrase,
//! polkit) shown to it holds one more owner connection, a [`SlotLink`] of `gnoblin-tree-client`.
//!
//! The link is served by a thread of its own, as the verbs' connection is ([`crate::verbs`]): the calls on it wait for the compositor's reply,
//! and a turn of the shell must never wait. The thread makes what it is told ([`SlotConnection::claim`], [`SlotConnection::release`],
//! [`SlotConnection::answer`]) and publishes what the compositor offers as [`SlotNotice`]s, which wake the shell's dispatch. The link is made
//! *on the thread* by the provider, so a link need not be `Send`.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_tree_client::slot_link::{ClientSlotLink, SlotEvent, SlotLink};
use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use rustix::time::Timespec;

use crate::events::{ShellEvent, SlotNotice};
use crate::{log, registry};

/// How long the thread waits for the compositor before it looks at its commands again, when the link has no descriptor to wait on.
const IDLE_WAIT: Duration = Duration::from_millis(4);

/// What the slot connection does once no generation has observed the slot for the grace: the owner's decision about a shell that never comes
/// back. The one place it is set is [`AFTER_GRACE`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterGrace {
    /// Give the claim back. The compositor's built-in dialog serves the requests from then on, and still needs the user: nothing is
    /// answered for them and nothing is approved.
    BuiltIn,
    /// Keep the claim and go on refusing every request until a generation observes the slot again. The default: a shell that installed a
    /// policy and is gone does not leave its requests to a dialog that has none of its checks.
    Deny,
}

/// What happens after the grace. Flip this one constant to change it for the whole core. It matters only for a session in which a shell
/// installed a dialog policy and has gone: a session in which none ever did holds no claim, and the built-in dialog serves as it always has.
pub const AFTER_GRACE: AfterGrace = AfterGrace::Deny;

/// How long the connection holds a request that no generation has taken up before it refuses it itself, and how long it keeps a claim
/// that no generation observes.
///
/// The connection never lets silence reach the built-in dialog: a request that no generation takes up within `hold` is answered `deny`, so
/// the gap between one generation's end and the next one's start is closed, not open. A request a generation took up
/// ([`SlotConnection::taken`]) is the generation's: its verdict, `pass` or `deny`, is what is sent, and a generation that gives none
/// because its listener ran out its budget leaves the request to the compositor, whose own window then ends and whose built-in dialog
/// asks the user. A generation that ends with the request in hand refuses it. A claim nobody observes for `grace` is given back, because
/// the shell that held the policy is gone.
#[derive(Clone, Copy, Debug)]
pub struct SlotTimings {
    /// How long an offered request waits for a generation to take it up. The compositor's own window is 250 ms.
    pub hold: Duration,
    /// How long a claim is kept while no generation observes the slot.
    pub grace: Duration,
    /// What is done when that time has passed.
    pub after_grace: AfterGrace,
}

impl Default for SlotTimings {
    fn default() -> Self {
        Self {
            hold: Duration::from_millis(150),
            grace: Duration::from_secs(10),
            after_grace: AFTER_GRACE,
        }
    }
}

/// What the host gives the core to reach the compositor's slots: a way to make the link, called on the connection's own thread the first
/// time a shell wants the slot.
pub trait SlotLinkProvider: Send + Sync {
    /// Makes the link.
    ///
    /// # Errors
    ///
    /// The compositor cannot be reached.
    fn open(&self) -> Result<Box<dyn SlotLink>, String>;
}

/// The compositor's owner socket, found as every client finds it: the slot link a session uses.
pub struct OwnerSlots {
    path: Result<PathBuf, String>,
}

impl OwnerSlots {
    /// The socket named by `$GNOBLIN_IPC_SOCKET`, or by `$XDG_RUNTIME_DIR`, as the environment is when this is called.
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            path: gnoblin_client::unix::socket_path(|name| std::env::var(name).ok()),
        }
    }

    /// The socket at `path`.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Ok(path) }
    }
}

impl SlotLinkProvider for OwnerSlots {
    fn open(&self) -> Result<Box<dyn SlotLink>, String> {
        let path = self.path.as_ref().map_err(Clone::clone)?;
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        let link = ClientSlotLink::connect(path, gnoblin_proto::Version::new(major, minor, patch))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(link))
    }
}

/// What the shell tells the thread.
enum SlotCommand {
    Claim(String),
    Release(String),
    /// Claims `request` and answers it with `answer`.
    Answer {
        request: u64,
        answer: &'static [u8],
    },
    /// No generation observes the slot now (`true`), or one does again (`false`).
    Gap(bool),
    /// A generation took `request` up: it is shown to the listeners, and the verdict is the generation's to give.
    Taken(u64),
}

/// The slot link and its thread, as a session holds them.
pub(crate) struct SlotConnection {
    commands: Sender<SlotCommand>,
    /// Written to wake the thread when a command is waiting.
    wake: OwnedFd,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SlotConnection {
    /// Starts the thread, which opens the link through `provider`.
    pub(crate) fn start(provider: Arc<dyn SlotLinkProvider>, timings: SlotTimings) -> Result<Self, String> {
        let (commands, command_rx) = mpsc::channel();
        let wake = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(|error| error.to_string())?;
        let thread_wake = wake.try_clone().map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("gnoblin-slots".to_owned())
                .spawn(move || serve(&*provider, &command_rx, &thread_wake, &stop, timings))
                .map_err(|error| format!("cannot start the slot thread: {error}"))?
        };
        Ok(Self {
            commands,
            wake,
            stop,
            thread: Mutex::new(Some(thread)),
        })
    }

    fn send(&self, command: SlotCommand) {
        if self.commands.send(command).is_ok() {
            let _ = rustix::io::write(&self.wake, &1_u64.to_ne_bytes());
        }
    }

    /// Claims `slot` as the shell. A refusal is published as [`SlotNotice::Refused`].
    pub(crate) fn claim(&self, slot: &str) {
        self.send(SlotCommand::Claim(slot.to_owned()));
    }

    /// Gives a claimed slot back.
    pub(crate) fn release(&self, slot: &str) {
        self.send(SlotCommand::Release(slot.to_owned()));
    }

    /// Says that no generation observes the slot (`gap`), or that one does again. The claim stays through a gap, and a request that
    /// comes in it is refused after the hold, never served by the built-in dialog.
    pub(crate) fn gap(&self, gap: bool) {
        self.send(SlotCommand::Gap(gap));
    }

    /// Says that a generation took `request` up: the listeners are being told of it. From then on the hold is not what answers it, the
    /// generation's verdict is, and one that never comes (a listener ran out its budget and was stopped) leaves the request to the
    /// compositor's own window.
    pub(crate) fn taken(&self, request: u64) {
        self.send(SlotCommand::Taken(request));
    }

    /// Answers `request`: `deny` refuses it by policy, anything else is `pass`, which leaves it to the built-in dialog.
    pub(crate) fn answer(&self, request: u64, deny: bool) {
        self.send(SlotCommand::Answer {
            request,
            answer: if deny { b"deny" } else { b"pass" },
        });
    }
}

impl Drop for SlotConnection {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = rustix::io::write(&self.wake, &1_u64.to_ne_bytes());
        if let Some(thread) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = thread.join();
        }
    }
}

fn publish(notice: SlotNotice) {
    registry::publish_event(ShellEvent::Slot(notice));
}

/// The thread's loop: make the link, then serve commands and forward what the compositor offers until the link fails or the connection is
/// dropped. A request no generation takes up within `timings.hold` is refused here.
fn serve(
    provider: &dyn SlotLinkProvider,
    commands: &Receiver<SlotCommand>,
    wake: &OwnedFd,
    stop: &AtomicBool,
    timings: SlotTimings,
) {
    let mut link = match provider.open() {
        Ok(link) => link,
        Err(why) => {
            log::warn!(area = "slots", "the slot connection could not be opened: {why}");
            publish(SlotNotice::Refused(why));
            return;
        }
    };
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut unanswered: BTreeMap<u64, Instant> = BTreeMap::new();
    // Requests this connection refused itself: a verdict that comes for one later is moot, and is not sent.
    let mut refused: BTreeSet<u64> = BTreeSet::new();
    let mut gap_since: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(command) = commands.try_recv() {
            match command {
                SlotCommand::Claim(slot) => {
                    // A claim a generation made stands for the next one: it is not made twice.
                    gap_since = None;
                    if !claimed.contains(&slot) {
                        match link.claim_slot(&slot) {
                            Ok(()) => {
                                claimed.insert(slot);
                            }
                            Err(error) => publish(SlotNotice::Refused(format!("{slot} was not claimed: {error}"))),
                        }
                    }
                }
                SlotCommand::Release(slot) => {
                    claimed.remove(&slot);
                    if let Err(error) = link.release_slot(&slot) {
                        log::debug!(area = "slots", "{slot} was not released: {error}");
                    }
                }
                SlotCommand::Answer { request, answer } => {
                    unanswered.remove(&request);
                    if refused.remove(&request) {
                        continue;
                    }
                    // A request that ended already is not an error: the observation window is short.
                    if let Err(error) = link.claim_request(request).and_then(|()| link.answer(request, answer)) {
                        log::debug!(area = "slots", request, "the verdict was not delivered: {error}");
                    }
                }
                SlotCommand::Gap(gap) => gap_since = gap.then(Instant::now),
                SlotCommand::Taken(request) => {
                    unanswered.remove(&request);
                }
            }
        }
        match link.poll() {
            Ok(events) => {
                for event in events {
                    match &event {
                        SlotEvent::Offered { request, slot, .. } if claimed.contains(slot) => {
                            unanswered.insert(*request, Instant::now());
                        }
                        SlotEvent::Finished { request, .. } => {
                            unanswered.remove(request);
                            refused.remove(request);
                        }
                        _ => {}
                    }
                    publish(match event {
                        SlotEvent::Offered {
                            request,
                            deadline,
                            slot,
                            title,
                            message,
                            subject,
                        } => SlotNotice::Offered {
                            request,
                            deadline,
                            slot,
                            title,
                            message,
                            subject,
                        },
                        SlotEvent::Finished { request, completion } => SlotNotice::Finished {
                            request,
                            completion: completion.to_owned(),
                        },
                        SlotEvent::Lost => SlotNotice::Lost("the compositor ended the connection".to_owned()),
                    });
                }
            }
            Err(error) => {
                log::warn!(area = "slots", "the slot connection ended: {error}");
                publish(SlotNotice::Lost(error.to_string()));
                return;
            }
        }
        // Silence is not consent: a request no generation took up in time is refused, never left to the built-in dialog.
        let late: Vec<u64> = unanswered
            .iter()
            .filter(|(_, offered)| offered.elapsed() >= timings.hold)
            .map(|(request, _)| *request)
            .collect();
        for request in late {
            unanswered.remove(&request);
            refused.insert(request);
            log::warn!(
                area = "slots",
                request,
                "no generation answered a dialog request: it is refused"
            );
            let _ = link.claim_request(request).and_then(|()| link.answer(request, b"deny"));
        }
        // A claim that no generation observes for the grace is given back: the shell that held the policy is gone.
        if timings.after_grace == AfterGrace::BuiltIn && gap_since.is_some_and(|since| since.elapsed() >= timings.grace)
        {
            gap_since = None;
            for slot in std::mem::take(&mut claimed) {
                let _ = link.release_slot(&slot);
            }
        }
        // Sleep until the compositor says something or a command is waiting, and wake to refuse what is not answered.
        let mut fds = vec![PollFd::new(wake, PollFlags::IN)];
        let socket = link.wait_fd();
        if let Some(socket) = &socket {
            fds.push(PollFd::new(socket, PollFlags::IN));
        }
        let watching = !unanswered.is_empty() || gap_since.is_some();
        let wait = if socket.is_none() {
            Some(IDLE_WAIT)
        } else if watching {
            Some(Duration::from_millis(10))
        } else {
            None
        };
        let timeout = wait.map(|wait| Timespec {
            tv_sec: 0,
            tv_nsec: i64::try_from(wait.as_nanos()).unwrap_or(i64::MAX),
        });
        if poll(&mut fds, timeout.as_ref()).is_ok() && fds[0].revents().contains(PollFlags::IN) {
            let mut counter = [0_u8; 8];
            let _ = rustix::io::read(wake.as_fd(), &mut counter);
        }
    }
}
