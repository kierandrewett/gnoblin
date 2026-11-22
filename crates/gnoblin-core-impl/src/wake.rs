//! `wake_fd`: one descriptor that says "`dispatch` has something to do", for a shell that owns its event loop.
//!
//! A default shell never sees the loop: the host turns `dispatch` for it. A shell with a loop of its own (tokio, calloop, GLib, an
//! interpreter's) needs the opposite: something it can poll beside its own sources, and a promise that `dispatch(ctx, 0)` will not
//! block and will drain what made the descriptor readable. The descriptor is an epoll
//! set, which is itself pollable and is readable while any member is:
//!
//! - an **eventfd**, written by [`signal`] from `post` and `publish` (any thread) and read by [`drain`] at the start of every
//!   `dispatch`, before the mailbox is taken, so a post that arrives while `dispatch` runs signals again instead of being lost;
//! - a **timerfd**, armed for the next timer of an active generation ([`sync`]) and read by [`drain`];
//! - the **link's descriptor**, when the shell attached to the tree: it stays readable until `dispatch`'s pump reads the link.
//!
//! All three are level triggered, so a wake that is not served is not lost, and a spurious wake costs one `dispatch(ctx, 0)` that
//! returns zero. The set is the shell instance's, not a generation's: a reload's generation is handed a duplicate of the same open
//! file description, and the core keeps the set's members in step as the instance's link and timers change.
//!
//! The members are plain descriptors of this process; nothing here is per generation, so there is one set per process, as there is
//! one host ([`crate::registry`]).

#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use rustix::event::{EventfdFlags, epoll, eventfd};
use rustix::io::Errno;
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create, timerfd_settime,
};

/// What epoll reports a member by, to tell them apart: unused beyond being distinct.
const TAG_EVENT: u64 = 1;
const TAG_TIMER: u64 = 2;
const TAG_LINK: u64 = 3;

/// The wake set of this process.
pub(crate) struct WakeSet {
    epoll: OwnedFd,
    event: OwnedFd,
    timer: OwnedFd,
    state: Mutex<WakeState>,
}

/// What the set currently holds beyond the two descriptors it owns.
#[derive(Default)]
struct WakeState {
    /// The link descriptor registered with the set, if one is.
    link: Option<RawFd>,
    /// When the timerfd is armed for, if it is.
    armed: Option<Instant>,
}

static WAKE: OnceLock<Result<WakeSet, String>> = OnceLock::new();

/// The process's wake set, made on first use.
///
/// # Errors
///
/// The kernel refused one of the descriptors (the process is out of them).
pub(crate) fn get() -> Result<&'static WakeSet, String> {
    WAKE.get_or_init(|| WakeSet::new().map_err(|error| format!("cannot make the wake descriptor: {error}")))
        .as_ref()
        .map_err(Clone::clone)
}

/// Wakes a shell that sleeps on its descriptor. Any thread. A process that never asked for the descriptor has no set, and this is
/// free.
pub(crate) fn signal() {
    if let Some(Ok(wake)) = WAKE.get() {
        wake.signal();
    }
}

/// Reads what woke the set, so it is quiet until something else does. Shell thread, at the start of `dispatch`.
pub(crate) fn drain() {
    if let Some(Ok(wake)) = WAKE.get() {
        wake.drain();
    }
}

/// Brings the set in step with the shell instance: the next timer that falls due, and the link's descriptor. Shell thread.
pub(crate) fn sync(next_timer: Option<Duration>, link: Option<BorrowedFd<'_>>) {
    if let Some(Ok(wake)) = WAKE.get() {
        wake.sync(next_timer, link);
    }
}

impl WakeSet {
    fn new() -> io::Result<Self> {
        let epoll = epoll::create(epoll::CreateFlags::CLOEXEC)?;
        let event = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let timer = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )?;
        epoll::add(
            &epoll,
            &event,
            epoll::EventData::new_u64(TAG_EVENT),
            epoll::EventFlags::IN,
        )?;
        epoll::add(
            &epoll,
            &timer,
            epoll::EventData::new_u64(TAG_TIMER),
            epoll::EventFlags::IN,
        )?;
        Ok(Self {
            epoll,
            event,
            timer,
            state: Mutex::new(WakeState::default()),
        })
    }

    fn signal(&self) {
        // A full counter (u64::MAX - 1 posts) would fail with EAGAIN, and then the descriptor is readable anyway.
        let _ = rustix::io::write(&self.event, &1_u64.to_ne_bytes());
    }

    fn drain(&self) {
        let mut counter = [0_u8; 8];
        let _ = rustix::io::read(&self.event, &mut counter);
        if rustix::io::read(&self.timer, &mut counter).is_ok() {
            self.state.lock().unwrap_or_else(PoisonError::into_inner).armed = None;
        }
    }

    fn sync(&self, next_timer: Option<Duration>, link: Option<BorrowedFd<'_>>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.sync_link(&mut state, link);
        self.sync_timer(&mut state, next_timer);
    }

    fn sync_link(&self, state: &mut WakeState, link: Option<BorrowedFd<'_>>) {
        let wanted = link.map(|fd| fd.as_raw_fd());
        if state.link == wanted {
            return;
        }
        if let Some(old) = state.link.take() {
            // The old descriptor may be closed already, which removes it from the set by itself: EBADF and ENOENT are fine.
            // SAFETY: `old` is a descriptor number this set registered; `BorrowedFd` here only names it for `delete`, which
            // does not use it after the call, and a closed number is refused by the kernel, not used.
            let old = unsafe { BorrowedFd::borrow_raw(old) };
            let _ = epoll::delete(&self.epoll, old);
        }
        if let Some(fd) = link {
            match epoll::add(
                &self.epoll,
                fd,
                epoll::EventData::new_u64(TAG_LINK),
                epoll::EventFlags::IN,
            ) {
                Ok(()) => state.link = wanted,
                // The number was reused by a link that replaced one the set forgot: take it over.
                Err(Errno::EXIST) => {
                    let _ = epoll::modify(
                        &self.epoll,
                        fd,
                        epoll::EventData::new_u64(TAG_LINK),
                        epoll::EventFlags::IN,
                    );
                    state.link = wanted;
                }
                Err(_) => {}
            }
        }
    }

    fn sync_timer(&self, state: &mut WakeState, next_timer: Option<Duration>) {
        let target = next_timer.map(|due_in| Instant::now() + due_in);
        // A target within a millisecond of the armed one is the same wake: do not call the kernel for it.
        let same = match (state.armed, target) {
            (None, None) => true,
            (Some(armed), Some(target)) => {
                let (early, late) = if armed <= target {
                    (armed, target)
                } else {
                    (target, armed)
                };
                late.duration_since(early) < Duration::from_millis(1)
            }
            _ => false,
        };
        if same {
            return;
        }
        let value = match next_timer {
            // A zero value disarms, so a timer that is due now is armed for a nanosecond.
            Some(due_in) => Timespec {
                tv_sec: i64::try_from(due_in.as_secs()).unwrap_or(i64::MAX),
                tv_nsec: if due_in.is_zero() {
                    1
                } else {
                    i64::from(due_in.subsec_nanos())
                },
            },
            None => Timespec { tv_sec: 0, tv_nsec: 0 },
        };
        let spec = Itimerspec {
            it_interval: Timespec { tv_sec: 0, tv_nsec: 0 },
            it_value: value,
        };
        if timerfd_settime(&self.timer, TimerfdTimerFlags::empty(), &spec).is_ok() {
            state.armed = target;
        }
    }

    /// A duplicate of the set for the shell, who closes it.
    pub(crate) fn duplicate(&self) -> io::Result<OwnedFd> {
        self.epoll.as_fd().try_clone_to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rustix::event::{PollFd, PollFlags, poll};
    use rustix::time::Timespec;

    use super::*;

    /// Whether `fd` is readable within `wait`.
    fn readable(fd: &OwnedFd, wait: Duration) -> bool {
        let mut fds = [PollFd::new(fd, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(wait.subsec_nanos()),
        };
        poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0)
    }

    #[test]
    fn a_signal_wakes_the_set_until_it_is_drained() {
        let wake = WakeSet::new().expect("the set");
        let fd = wake.duplicate().expect("a duplicate");
        assert!(!readable(&fd, Duration::from_millis(5)), "quiet at first");
        wake.signal();
        assert!(readable(&fd, Duration::from_millis(500)), "a signal wakes it");
        assert!(
            readable(&fd, Duration::from_millis(5)),
            "level triggered: still readable"
        );
        wake.drain();
        assert!(!readable(&fd, Duration::from_millis(5)), "drained");
    }

    #[test]
    fn the_timer_wakes_the_set_when_it_falls_due() {
        let wake = WakeSet::new().expect("the set");
        let fd = wake.duplicate().expect("a duplicate");
        wake.sync(Some(Duration::from_millis(30)), None);
        assert!(!readable(&fd, Duration::from_millis(5)), "not due yet");
        assert!(readable(&fd, Duration::from_millis(2000)), "due");
        wake.drain();
        assert!(
            !readable(&fd, Duration::from_millis(5)),
            "drained, and a timer is a one-shot"
        );
        // Disarmed before it falls due: no wake.
        wake.sync(Some(Duration::from_millis(30)), None);
        wake.sync(None, None);
        assert!(!readable(&fd, Duration::from_millis(80)));
    }

    #[test]
    fn the_links_descriptor_is_a_member_while_it_is_registered() {
        let wake = WakeSet::new().expect("the set");
        let fd = wake.duplicate().expect("a duplicate");
        let link = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).expect("a stand-in link");
        wake.sync(None, Some(link.as_fd()));
        assert!(!readable(&fd, Duration::from_millis(5)));
        rustix::io::write(&link, &1_u64.to_ne_bytes()).expect("the link has bytes");
        assert!(readable(&fd, Duration::from_millis(500)), "the link wakes the set");
        let mut counter = [0_u8; 8];
        rustix::io::read(&link, &mut counter).expect("the pump reads the link");
        assert!(!readable(&fd, Duration::from_millis(5)), "the pump quietened it");
        // The link goes: its bytes no longer wake the set.
        wake.sync(None, None);
        rustix::io::write(&link, &1_u64.to_ne_bytes()).expect("more bytes");
        assert!(!readable(&fd, Duration::from_millis(20)));
    }
}
