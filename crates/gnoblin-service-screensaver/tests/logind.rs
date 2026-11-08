//! The logind integration over a private bus. A fake `org.freedesktop.login1` serves the
//! manager (`GetSessionByPID`, `Inhibit`) and emits the signals logind sends (`PrepareForSleep`
//! and a session `Lock`). The compositor seam records each lock and whether the sleep
//! inhibitor was still held when the lock was requested.
//!
//! The inhibitor is a file descriptor logind hands out. The fake keeps the peer end of each
//! socket it returns, so "the service still holds the inhibitor" is "the peer has not seen
//! end of file". The service closing its copy is what releases the sleep delay.
//!
//! Every test starts its own `dbus-daemon` through `gnoblin-service-testkit`, so none of
//! them can reach the developer's system or session bus.

use std::io::{ErrorKind, Read};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_lite::future::block_on;
use gnoblin_model::ObjectId;
use gnoblin_service_screensaver::logind;
use gnoblin_service_screensaver::{Compositor, LockReading, ScreenSaver};
use gnoblin_service_testkit::Bus;
use zbus::Connection;
use zbus::connection::Builder;
use zbus::fdo;
use zbus::{OwnedFd, OwnedObjectPath};

/// How long a test waits for the service to act on a bus event.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The lock object the recorder reports, unlocked when the service starts.
const LOCK_OBJECT: u64 = 7;

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
/// The session the fake reports for every process, so the service's own session is this one.
const OWN_SESSION: &str = "/org/freedesktop/login1/session/_31";
/// Another session on the same seat, which must not lock this service.
const OTHER_SESSION: &str = "/org/freedesktop/login1/session/_32";

/// The state the fake logind and the compositor recorder share.
#[derive(Clone, Default)]
struct World {
    /// The peer end of every inhibitor the fake handed out, oldest first.
    peers: Arc<Mutex<Vec<UnixStream>>>,
    /// Whether the newest inhibitor was still held at each lock request.
    locks: Arc<Mutex<Vec<bool>>>,
    /// Makes the compositor refuse a lock.
    refuse_lock: Arc<AtomicBool>,
}

impl World {
    /// Whether the newest inhibitor is still held by the service. A peer that reads end of
    /// file has lost its last write end, so the service has released it.
    fn newest_inhibitor_held(&self) -> bool {
        let peers = self.peers.lock().unwrap();
        peers.last().is_some_and(is_held)
    }

    fn inhibitor_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    fn lock_count(&self) -> usize {
        self.locks.lock().unwrap().len()
    }

    fn locks(&self) -> Vec<bool> {
        self.locks.lock().unwrap().clone()
    }
}

/// Whether the service still holds the other end of `peer`. The read is non-blocking, and
/// nothing is ever written to the socket, so a held inhibitor reports `WouldBlock`.
fn is_held(peer: &UnixStream) -> bool {
    peer.set_nonblocking(true).expect("the peer socket accepts a flag");
    let mut byte = [0u8; 1];
    let held = match (&*peer).read(&mut byte) {
        Ok(0) => false,
        Ok(_) => true,
        Err(error) => error.kind() == ErrorKind::WouldBlock,
    };
    peer.set_nonblocking(false).expect("the peer socket accepts a flag");
    held
}

impl Compositor for World {
    fn idle_ms(&self) -> Result<u64, String> {
        Ok(0)
    }

    fn lock_reading(&self) -> Result<Option<LockReading>, String> {
        Ok(Some(LockReading {
            object: ObjectId::new(LOCK_OBJECT),
            locked: false,
        }))
    }

    fn lock(&self) -> Result<(), String> {
        self.locks.lock().unwrap().push(self.newest_inhibitor_held());
        if self.refuse_lock.load(Ordering::SeqCst) {
            return Err("the compositor refused the lock".to_owned());
        }
        Ok(())
    }

    fn inhibit(&self, _holder: &str) -> Result<(), String> {
        Ok(())
    }

    fn uninhibit(&self, _holder: &str) -> Result<(), String> {
        Ok(())
    }
}

/// The fake manager: the two methods the service calls, with logind's own argument checks.
struct Manager {
    world: World,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    /// Every process is in `OWN_SESSION`, so the service's session is the one it locks.
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, _pid: u32) -> fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(OWN_SESSION.to_owned()).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// Hands out one end of a socket pair. Only a delay inhibitor for sleep is a real
    /// inhibitor, so anything else is refused the way logind refuses it.
    fn inhibit(&self, what: String, _who: String, _why: String, mode: String) -> fdo::Result<OwnedFd> {
        if what != "sleep" || mode != "delay" {
            return Err(fdo::Error::InvalidArgs(format!("unsupported inhibit {what} {mode}")));
        }
        let (peer, end) = UnixStream::pair().map_err(|error| fdo::Error::Failed(error.to_string()))?;
        self.world.peers.lock().unwrap().push(peer);
        Ok(OwnedFd::from(std::os::fd::OwnedFd::from(end)))
    }
}

async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

/// Serve the fake logind on `address` as `org.freedesktop.login1`. The returned connection
/// is the one the fake emits its signals from.
fn fake_logind(address: &str, world: World) -> Connection {
    let server = block_on(connect(address));
    block_on(server.object_server().at(MANAGER_PATH, Manager { world })).expect("the manager path is free");
    block_on(server.request_name(LOGIND)).expect("the fake owns the logind name");
    server
}

/// Start the service's logind watch on its own connection, as the service does at start.
fn watch(address: &str, service: &ScreenSaver) -> Connection {
    let system = block_on(connect(address));
    logind::watch(&system, service).expect("the logind watch starts");
    system
}

/// Emit `PrepareForSleep` from the manager, as logind does before the machine suspends
/// (`true`) and after it resumes (`false`).
fn prepare_for_sleep(logind: &Connection, going_down: bool) {
    block_on(logind.emit_signal(None::<&str>, MANAGER_PATH, MANAGER, "PrepareForSleep", &going_down))
        .expect("the signal is emitted");
}

/// Emit a session `Lock` on `path`, as logind does for `loginctl lock-session`.
fn session_lock(logind: &Connection, path: &str) {
    block_on(logind.emit_signal(None::<&str>, path, SESSION, "Lock", &())).expect("the signal is emitted");
}

/// Poll `condition` until it holds, failing the test at the deadline.
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The world, a service answering from it, and the logind watch the service started.
fn started(bus: &Bus) -> (World, Connection, Connection) {
    let world = World::default();
    let logind = fake_logind(bus.address(), world.clone());
    let service = ScreenSaver::new(Box::new(world.clone()));
    let watcher = watch(bus.address(), &service);
    wait_until("the service takes its sleep inhibitor", || {
        world.inhibitor_count() == 1 && world.newest_inhibitor_held()
    });
    (world, logind, watcher)
}

#[test]
fn a_sleep_locks_the_session_while_the_inhibitor_is_still_held() {
    let bus = Bus::start();
    let (world, logind, _watcher) = started(&bus);

    prepare_for_sleep(&logind, true);

    wait_until("the session locks before sleep", || world.lock_count() == 1);
    assert_eq!(
        world.locks(),
        vec![true],
        "the lock must be requested while the sleep inhibitor is still held"
    );
    wait_until("the sleep inhibitor is released after the lock", || {
        !world.newest_inhibitor_held()
    });
}

#[test]
fn a_resume_takes_the_sleep_inhibitor_again_without_locking() {
    let bus = Bus::start();
    let (world, logind, _watcher) = started(&bus);

    prepare_for_sleep(&logind, true);
    wait_until("the sleep inhibitor is released", || {
        world.lock_count() == 1 && !world.newest_inhibitor_held()
    });

    prepare_for_sleep(&logind, false);

    wait_until("the sleep inhibitor is taken again", || {
        world.inhibitor_count() == 2 && world.newest_inhibitor_held()
    });
    assert_eq!(world.lock_count(), 1, "a resume must not lock the session again");
}

#[test]
fn a_refused_lock_still_releases_the_sleep_inhibitor() {
    let bus = Bus::start();
    let (world, logind, _watcher) = started(&bus);
    world.refuse_lock.store(true, Ordering::SeqCst);

    prepare_for_sleep(&logind, true);

    wait_until("the refused lock is attempted", || world.lock_count() == 1);
    wait_until("the sleep inhibitor is released anyway", || {
        !world.newest_inhibitor_held()
    });
    assert_eq!(world.locks(), vec![true]);
}

#[test]
fn a_session_lock_on_our_own_session_locks_the_session() {
    let bus = Bus::start();
    let (world, logind, _watcher) = started(&bus);

    session_lock(&logind, OWN_SESSION);

    wait_until("the session locks on logind's Lock", || world.lock_count() == 1);
}

#[test]
fn a_session_lock_on_another_session_is_ignored() {
    let bus = Bus::start();
    let (world, logind, _watcher) = started(&bus);

    session_lock(&logind, OTHER_SESSION);
    session_lock(&logind, OWN_SESSION);
    // One connection delivers in order, and the watch matches Lock on our session's path
    // only. A Lock for the other session would therefore reach the service before ours,
    // so a count of one once ours has arrived means it was ignored.
    wait_until("our own session's lock arrives", || world.lock_count() == 1);
    assert_eq!(world.lock_count(), 1, "a lock for the other session locked this one");
}
