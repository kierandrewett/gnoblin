//! `org.freedesktop.ScreenSaver`, served from the services role.
//!
//! Clients such as media players call `Inhibit` while they play, and screen lockers
//! and idle daemons ask `GetActive` and `GetSessionIdleTime`. The service answers from
//! the compositor's `idle` and `lock` objects. It keeps the inhibit table itself, so a
//! cookie is checked here before the compositor's hold is released.
//!
//! # Holds in the compositor
//!
//! Each `Inhibit` records a hold named `<sender>:<cookie>` through the idle object's
//! `inhibit` method, and `UnInhibit`, or the sender leaving the bus, releases it through
//! `uninhibit`. While any hold is set the compositor does not fire idle notifications.
//! A hold the compositor refuses fails the `Inhibit` call and is not recorded here.
//!
//! # What the compositor does not read yet
//!
//! `GetActive` reports the lock object only. `Lock` asks the compositor and succeeds only
//! once the compositor confirms the session locked, within its bound; a refusal, a failed
//! cover or a timeout is a D-Bus error. `SimulateUserActivity` is refused, since the schema
//! declares no member for it. `ActiveChanged` is declared but not emitted, since no lock
//! change is watched.
//!
//! # Failure modes first
//!
//! 1. **An `UnInhibit` for a cookie nobody holds** is a no-op, and so is one for a cookie
//!    that a different sender owns: `tests::an_unknown_or_foreign_cookie_is_a_no_op`.
//! 2. **A sender leaves the bus** and its inhibits go with it, in the table and in the
//!    compositor, so a crashed player cannot hold the screen awake:
//!    `tests::a_departed_sender_releases_its_inhibits` and
//!    `tests::a_departed_sender_releases_its_holds_in_the_compositor`.
//! 3. **A sender holds 64 inhibits** and asks for a 65th: refused with `LimitsExceeded`,
//!    and the other senders are unaffected: `tests::the_65th_inhibit_from_one_sender_is_refused`.
//! 4. **`Lock` is called** and reaches the compositor's lock method exactly once:
//!    `tests::lock_calls_the_compositor_once`. A lock the compositor does not confirm is a
//!    D-Bus error, never success: `tests::a_lock_the_compositor_does_not_confirm_is_an_error`.
//!    A confirmed lock starts the clock: `tests::a_confirmed_lock_starts_the_clock`.
//! 5. **`SetActive(true)` is called**: refused with `false`, since clients cannot lock the
//!    screen by setting a flag: `tests::set_active_is_refused`.
//! 6. **The compositor's idle or lock object is missing**: the call fails rather than
//!    reporting zero idle time, which would say the user is active when nothing told us:
//!    `tests::a_missing_idle_object_fails_closed`.
//! 7. **The compositor refuses a hold** (its cap is reached): the `Inhibit` call fails and
//!    no inhibit is recorded, so the sender's cookie count is not inflated:
//!    `tests::a_refused_inhibit_leaves_no_entry`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use futures_lite::StreamExt;
use gnoblin_model::{Fields, TypeKey, Value};
use gnoblin_service_core::Host;
use tracing::{info, warn};
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

/// The well-known name this service owns on the session bus.
pub const BUS_NAME: &str = "org.freedesktop.ScreenSaver";
/// The two object paths the interface is served on, as clients use both.
pub const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
/// The most inhibits one sender may hold at once.
pub const MAX_INHIBITS_PER_SENDER: usize = 64;

/// The compositor operations the service needs, behind a seam so tests can run without
/// a compositor.
pub trait Compositor: Send + Sync {
    /// Milliseconds since the last input event, from the `idle` object.
    fn idle_ms(&self) -> Result<u64, String>;
    /// Whether the session is locked, from the `lock` object.
    fn locked(&self) -> Result<bool, String>;
    /// Ask the compositor to lock the session, and wait for it to confirm the lock. `Ok`
    /// means the compositor has proven the session locked; an error means it refused, the
    /// cover failed, or it did not confirm within its bound.
    fn lock(&self) -> Result<(), String>;
    /// Hold the session awake under `holder`, a name unique among the service's holders.
    /// `Ok` means the compositor recorded the hold.
    fn inhibit(&self, holder: &str) -> Result<(), String>;
    /// Release the hold `holder` names. Releasing a name nothing holds is `Ok`.
    fn uninhibit(&self, holder: &str) -> Result<(), String>;
}

/// One inhibit a client holds.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ScreenInhibit {
    sender: String,
    application: String,
    reason: String,
}

/// The inhibit table, keyed by cookie.
#[derive(Default, Debug)]
pub struct Inhibitors {
    next_cookie: u32,
    table: BTreeMap<u32, ScreenInhibit>,
}

impl Inhibitors {
    /// Record an inhibit for `sender` and return its cookie.
    fn inhibit(&mut self, sender: &str, application: &str, reason: &str) -> Result<u32, fdo::Error> {
        let held = self.table.values().filter(|held| held.sender == sender).count();
        if held >= MAX_INHIBITS_PER_SENDER {
            return Err(fdo::Error::LimitsExceeded(format!(
                "{sender} already holds {MAX_INHIBITS_PER_SENDER} inhibits"
            )));
        }
        self.next_cookie = self.next_cookie.wrapping_add(1).max(1);
        while self.table.contains_key(&self.next_cookie) {
            self.next_cookie = self.next_cookie.wrapping_add(1).max(1);
        }
        let cookie = self.next_cookie;
        self.table.insert(
            cookie,
            ScreenInhibit {
                sender: sender.to_owned(),
                application: application.to_owned(),
                reason: reason.to_owned(),
            },
        );
        Ok(cookie)
    }

    /// Whether `sender` owns `cookie`. Anything it does not own is treated as unheld.
    fn owned_by(&self, sender: &str, cookie: u32) -> bool {
        self.table.get(&cookie).is_some_and(|held| held.sender == sender)
    }

    /// Every cookie `sender` holds, for releasing a departed sender's holds.
    fn cookies_of(&self, sender: &str) -> Vec<u32> {
        self.table
            .iter()
            .filter(|(_, held)| held.sender == sender)
            .map(|(cookie, _)| *cookie)
            .collect()
    }

    /// Drop `cookie` from the table. Callers check ownership first.
    fn forget(&mut self, cookie: u32) {
        self.table.remove(&cookie);
    }
}

/// The screensaver service: the interface's state, shared between its object paths.
#[derive(Clone)]
pub struct ScreenSaver {
    inner: Arc<Core>,
}

struct Core {
    compositor: Box<dyn Compositor>,
    inhibitors: Mutex<Inhibitors>,
    locked_since: Mutex<Option<Instant>>,
}

impl ScreenSaver {
    /// A service answering from `compositor`.
    pub fn new(compositor: Box<dyn Compositor>) -> ScreenSaver {
        ScreenSaver {
            inner: Arc::new(Core {
                compositor,
                inhibitors: Mutex::new(Inhibitors::default()),
                locked_since: Mutex::new(None),
            }),
        }
    }

    fn inhibitors(&self) -> MutexGuard<'_, Inhibitors> {
        self.inner.inhibitors.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record an inhibit and hold the session awake in the compositor under its holder
    /// name. The table lock is held across the compositor call, so a release of the same
    /// cookie cannot interleave. A refused hold leaves no entry behind.
    fn add_inhibit(&self, sender: &str, application: &str, reason: &str) -> Result<u32, fdo::Error> {
        let mut table = self.inhibitors();
        let cookie = table.inhibit(sender, application, reason)?;
        if let Err(error) = self.inner.compositor.inhibit(&holder_name(sender, cookie)) {
            table.forget(cookie);
            return Err(fdo::Error::Failed(format!(
                "the compositor refused the inhibit: {error}"
            )));
        }
        Ok(cookie)
    }

    /// Release a hold `sender` owns, in the compositor first. If the compositor refuses the
    /// release the entry stays, so the client can retry. Anything not owned is a no-op.
    fn remove_inhibit(&self, sender: &str, cookie: u32) -> Result<(), fdo::Error> {
        let mut table = self.inhibitors();
        if !table.owned_by(sender, cookie) {
            return Ok(());
        }
        self.inner
            .compositor
            .uninhibit(&holder_name(sender, cookie))
            .map_err(|error| fdo::Error::Failed(format!("the compositor refused the release: {error}")))?;
        table.forget(cookie);
        Ok(())
    }

    /// Called when `sender` leaves the bus: releases every hold it owns in the compositor,
    /// so a vanished player cannot keep the session awake. A refusal is logged, since no
    /// client remains to retry it.
    pub fn release_sender(&self, sender: &str) {
        let mut table = self.inhibitors();
        for cookie in table.cookies_of(sender) {
            if let Err(error) = self.inner.compositor.uninhibit(&holder_name(sender, cookie)) {
                warn!(sender, cookie, %error, "the compositor refused to release a departed sender's hold");
            }
            table.forget(cookie);
        }
    }

    fn request_lock(&self) -> Result<(), fdo::Error> {
        self.inner.compositor.lock().map_err(fdo::Error::Failed)?;
        let mut since = self.inner.locked_since.lock().unwrap_or_else(PoisonError::into_inner);
        since.get_or_insert_with(Instant::now);
        Ok(())
    }

    fn is_locked(&self) -> Result<bool, fdo::Error> {
        self.inner.compositor.locked().map_err(fdo::Error::Failed)
    }

    fn active_seconds(&self) -> Result<u32, fdo::Error> {
        if !self.is_locked()? {
            return Ok(0);
        }
        let since = self.inner.locked_since.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(since.map_or(0, |since| u32::try_from(since.elapsed().as_secs()).unwrap_or(u32::MAX)))
    }

    fn idle_seconds(&self) -> Result<u32, fdo::Error> {
        let millis = self.inner.compositor.idle_ms().map_err(fdo::Error::Failed)?;
        Ok(u32::try_from(millis / 1000).unwrap_or(u32::MAX))
    }

    /// Serve the interface on both paths, own the bus name, and release the inhibits of
    /// each sender that leaves the bus.
    pub fn register(&self, connection: &Connection) -> zbus::Result<()> {
        for path in PATHS {
            block_on(connection.object_server().at(path, self.clone()))?;
        }
        block_on(connection.request_name(BUS_NAME))?;
        let watcher = connection.clone();
        let service = self.clone();
        std::thread::Builder::new()
            .name("screensaver-owners".to_owned())
            .spawn(move || block_on(watch_owners(watcher, service)))
            .map_err(|error| zbus::Error::InputOutput(Arc::new(error)))?;
        Ok(())
    }
}

/// Watch `NameOwnerChanged` and release the inhibits of every unique name that leaves.
async fn watch_owners(connection: Connection, service: ScreenSaver) {
    let proxy = match fdo::DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(error) => {
            warn!(%error, "[services] cannot watch bus names, so inhibits outlive their senders");
            return;
        }
    };
    let Ok(mut owners) = proxy.receive_name_owner_changed().await else {
        warn!("[services] cannot subscribe to NameOwnerChanged");
        return;
    };
    while let Some(signal) = owners.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_none() {
            service.release_sender(&args.name().to_string());
        }
    }
}

/// Run a future to completion on the calling thread.
fn block_on<F: Future>(future: F) -> F::Output {
    futures_lite::future::block_on(future)
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    /// Hold the screen awake for `application_name` until `UnInhibit`.
    async fn inhibit(
        &self,
        application_name: &str,
        reason: &str,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<u32> {
        let sender = sender_of(&header)?;
        let cookie = self.add_inhibit(&sender, application_name, reason)?;
        info!(%sender, cookie, application = application_name, "[services] screensaver inhibited");
        Ok(cookie)
    }

    /// Release the inhibit `cookie`. Unknown or foreign cookies are ignored.
    async fn un_inhibit(&self, cookie: u32, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = sender_of(&header)?;
        self.remove_inhibit(&sender, cookie)
    }

    /// Whether the session is locked.
    async fn get_active(&self) -> fdo::Result<bool> {
        self.is_locked()
    }

    /// Clients cannot lock the screen by setting a flag, so this always answers false.
    async fn set_active(&self, _active: bool) -> bool {
        false
    }

    /// Seconds the session has been locked through this service, or zero.
    async fn get_active_time(&self) -> fdo::Result<u32> {
        self.active_seconds()
    }

    /// Seconds since the last input event.
    async fn get_session_idle_time(&self) -> fdo::Result<u32> {
        self.idle_seconds()
    }

    /// Lock the session.
    async fn lock(&self) -> fdo::Result<()> {
        self.request_lock()
    }

    /// Not supported yet: the compositor has no method to take an activity event.
    async fn simulate_user_activity(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "the compositor does not take simulated activity yet".to_owned(),
        ))
    }

    /// Emitted when the lock state changes. Not emitted yet: the compositor's lock changes
    /// are not watched.
    #[zbus(signal)]
    async fn active_changed(emitter: &SignalEmitter<'_>, active: bool) -> zbus::Result<()>;
}

/// The unique name that sent the call, which is what an inhibit belongs to.
fn sender_of(header: &Header<'_>) -> fdo::Result<String> {
    header
        .sender()
        .map(|sender| sender.to_string())
        .ok_or_else(|| fdo::Error::Failed("the call has no sender".to_owned()))
}

/// The hold name the compositor records for an inhibit: `<sender>:<cookie>`, unique per
/// sender because cookies are never reused within the table.
fn holder_name(sender: &str, cookie: u32) -> String {
    format!("{sender}:{cookie}")
}

/// The compositor, reached through the services role's connection.
impl Compositor for Host {
    fn idle_ms(&self) -> Result<u64, String> {
        let snapshot = self.snapshot().map_err(|error| error.to_string())?;
        let view = snapshot
            .objects()
            .iter()
            .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
            .ok_or_else(|| "the compositor publishes no idle object".to_owned())?;
        match view.property("idle_ms") {
            Some(Value::Uint(millis)) => Ok(*millis),
            _ => Err("the idle object has no idle_ms".to_owned()),
        }
    }

    fn locked(&self) -> Result<bool, String> {
        let snapshot = self.snapshot().map_err(|error| error.to_string())?;
        let view = snapshot
            .objects()
            .iter()
            .find(|view| view.ty() == &TypeKey::gnoblin("lock"))
            .ok_or_else(|| "the compositor publishes no lock object".to_owned())?;
        match view.property("locked") {
            Some(Value::Bool(locked)) => Ok(*locked),
            _ => Err("the lock object has no locked".to_owned()),
        }
    }

    fn lock(&self) -> Result<(), String> {
        let snapshot = self.snapshot().map_err(|error| error.to_string())?;
        let view = snapshot
            .objects()
            .iter()
            .find(|view| view.ty() == &TypeKey::gnoblin("lock"))
            .ok_or_else(|| "the compositor publishes no lock object".to_owned())?;
        // The compositor answers `done` only once it has proven the session locked. Its
        // refusal, failed cover or timeout arrives as `failed`, and that is an error here.
        match self.call_operation(view.id(), "lock_session") {
            Ok(gnoblin_model::OperationOutcome::Done) => Ok(()),
            Ok(gnoblin_model::OperationOutcome::Failed(code)) => {
                Err(format!("the compositor did not lock the session: {}", code.as_str()))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn inhibit(&self, holder: &str) -> Result<(), String> {
        call_idle(self, "inhibit", holder)
    }

    fn uninhibit(&self, holder: &str) -> Result<(), String> {
        call_idle(self, "uninhibit", holder)
    }
}

/// Call `method` on the compositor's idle object with the hold name `holder`.
fn call_idle(host: &Host, method: &str, holder: &str) -> Result<(), String> {
    let snapshot = host.snapshot().map_err(|error| error.to_string())?;
    let view = snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .ok_or_else(|| "the compositor publishes no idle object".to_owned())?;
    let mut arguments = Fields::new();
    arguments.insert("holder".to_owned(), Value::Text(holder.to_owned()));
    match host.call_operation_with(view.id(), method, &arguments) {
        Ok(gnoblin_model::OperationOutcome::Done) => Ok(()),
        Ok(gnoblin_model::OperationOutcome::Failed(code)) => {
            Err(format!("the compositor refused {method}: {}", code.as_str()))
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Start the screensaver service on the session bus, or log why it is not served.
pub fn start(host: &Host) -> Option<(ScreenSaver, Connection)> {
    let connection = match block_on(Connection::session()) {
        Ok(connection) => connection,
        Err(error) => {
            info!(%error, "[services] no session bus, so no screensaver service");
            return None;
        }
    };
    let service = ScreenSaver::new(Box::new(host.clone()));
    match service.register(&connection) {
        Ok(()) => Some((service, connection)),
        Err(error) => {
            warn!(%error, "[services] the screensaver service could not be served");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        idle: Result<u64, String>,
        locked: Result<bool, String>,
        /// What the compositor answers a lock with: `Ok` once it has confirmed the lock.
        lock: Result<(), String>,
        locks: Mutex<u32>,
        /// The holder names currently set in the fake compositor.
        holds: Mutex<Vec<String>>,
        /// Whether the fake refuses every new hold, as a compositor at its cap would.
        refuse_inhibit: bool,
    }

    impl Compositor for Arc<Fake> {
        fn idle_ms(&self) -> Result<u64, String> {
            self.idle.clone()
        }
        fn locked(&self) -> Result<bool, String> {
            self.locked.clone()
        }
        fn lock(&self) -> Result<(), String> {
            *self.locks.lock().unwrap() += 1;
            self.lock.clone()
        }
        fn inhibit(&self, holder: &str) -> Result<(), String> {
            if self.refuse_inhibit {
                return Err("too-many-inhibitors".to_owned());
            }
            self.holds.lock().unwrap().push(holder.to_owned());
            Ok(())
        }
        fn uninhibit(&self, holder: &str) -> Result<(), String> {
            self.holds.lock().unwrap().retain(|held| held != holder);
            Ok(())
        }
    }

    fn fake(idle: Result<u64, String>) -> (Arc<Fake>, ScreenSaver) {
        let fake = Arc::new(Fake {
            idle,
            locked: Ok(false),
            lock: Ok(()),
            locks: Mutex::new(0),
            holds: Mutex::new(Vec::new()),
            refuse_inhibit: false,
        });
        (fake.clone(), ScreenSaver::new(Box::new(fake)))
    }

    #[test]
    fn an_unknown_or_foreign_cookie_is_a_no_op() {
        let (_, service) = fake(Ok(0));
        let cookie = service.add_inhibit(":1.5", "player", "playing").unwrap();
        service.remove_inhibit(":1.5", cookie + 100).unwrap();
        service.remove_inhibit(":1.9", cookie).unwrap();
        assert_eq!(service.inhibitors().table.len(), 1);
        service.remove_inhibit(":1.5", cookie).unwrap();
        assert!(service.inhibitors().table.is_empty());
    }

    #[test]
    fn a_departed_sender_releases_its_inhibits() {
        let (_, service) = fake(Ok(0));
        service.add_inhibit(":1.5", "player", "playing").unwrap();
        let kept = service.add_inhibit(":1.6", "video", "watching").unwrap();
        service.release_sender(":1.5");
        let table = &service.inhibitors().table;
        assert_eq!(table.len(), 1);
        assert!(table.contains_key(&kept));
    }

    #[test]
    fn the_65th_inhibit_from_one_sender_is_refused() {
        let (_, service) = fake(Ok(0));
        for _ in 0..MAX_INHIBITS_PER_SENDER {
            service.add_inhibit(":1.5", "player", "playing").unwrap();
        }
        let refused = service.add_inhibit(":1.5", "player", "playing");
        assert!(matches!(refused, Err(fdo::Error::LimitsExceeded(_))));
        assert!(service.add_inhibit(":1.6", "video", "watching").is_ok());
    }

    #[test]
    fn lock_calls_the_compositor_once() {
        let (fake, service) = fake(Ok(0));
        service.request_lock().unwrap();
        assert_eq!(*fake.locks.lock().unwrap(), 1);
    }

    #[test]
    fn a_lock_the_compositor_does_not_confirm_is_an_error() {
        let fake = Arc::new(Fake {
            idle: Ok(0),
            locked: Ok(false),
            lock: Err("the compositor did not lock the session: not-locked".to_owned()),
            locks: Mutex::new(0),
            holds: Mutex::new(Vec::new()),
            refuse_inhibit: false,
        });
        let service = ScreenSaver::new(Box::new(fake.clone()));
        assert!(matches!(service.request_lock(), Err(fdo::Error::Failed(_))));
        assert_eq!(*fake.locks.lock().unwrap(), 1);
        assert!(
            service.inner.locked_since.lock().unwrap().is_none(),
            "an unconfirmed lock starts no clock"
        );
    }

    #[test]
    fn a_confirmed_lock_starts_the_clock() {
        let (_, service) = fake(Ok(0));
        service.request_lock().unwrap();
        assert!(service.inner.locked_since.lock().unwrap().is_some());
    }

    #[test]
    fn set_active_is_refused() {
        assert!(!futures_lite::future::block_on(async {
            let (_, service) = fake(Ok(0));
            service.set_active(true).await
        }));
    }

    #[test]
    fn a_missing_idle_object_fails_closed() {
        let (_, service) = fake(Err("the compositor publishes no idle object".to_owned()));
        assert!(service.idle_seconds().is_err());
    }

    #[test]
    fn a_departed_sender_releases_its_holds_in_the_compositor() {
        let (fake, service) = fake(Ok(0));
        let cookie = service.add_inhibit(":1.5", "player", "playing").unwrap();
        assert_eq!(*fake.holds.lock().unwrap(), vec![format!(":1.5:{cookie}")]);
        service.release_sender(":1.5");
        assert!(fake.holds.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_inhibit_leaves_no_entry() {
        let fake = Arc::new(Fake {
            idle: Ok(0),
            locked: Ok(false),
            lock: Ok(()),
            locks: Mutex::new(0),
            holds: Mutex::new(Vec::new()),
            refuse_inhibit: true,
        });
        let service = ScreenSaver::new(Box::new(fake.clone()));
        assert!(service.add_inhibit(":1.5", "player", "playing").is_err());
        assert!(service.inhibitors().cookies_of(":1.5").is_empty());
        assert!(fake.holds.lock().unwrap().is_empty());
    }
}
