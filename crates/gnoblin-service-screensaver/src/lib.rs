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
//! # Lock state
//!
//! The lock object is read once when the service starts, and its later `locked` changes
//! arrive as observations, which [`ScreenSaver::observe`] folds. `GetActive` answers from
//! that folded state and makes no compositor call of its own, since a per-call snapshot
//! subscribes afresh each time. `ActiveChanged` is emitted only when the folded state
//! changes. `SetActive(true)` requests a lock and answers `true` once the compositor
//! confirms it. `SetActive(false)` is refused, since clients cannot unlock the session
//! through this service. `Lock` asks the compositor and succeeds only once the compositor
//! confirms the session locked, within its bound; a refusal, a failed cover or a timeout
//! is a D-Bus error. `SimulateUserActivity` is refused, since the schema declares no
//! member for it.
//!
//! # logind
//!
//! [`start`] also follows logind on the system bus, see [`logind`]. On `PrepareForSleep(true)`
//! the session locks while a sleep delay inhibitor is still held, then the inhibitor is
//! released; a `Lock` signal for this process's session locks it too. With no system bus or
//! no logind the service serves without this, and logs that it does.
//!
//! # Failure modes first
//!
//! 1. **An `UnInhibit` for a cookie nobody holds, or one another sender owns** is refused
//!    with `AccessDenied` and changes nothing:
//!    `tests::an_unknown_or_foreign_cookie_is_refused`, and over the bus
//!    `tests/screensaver.rs::an_uninhibit_for_a_foreign_cookie_is_access_denied`.
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
//! 5. **`SetActive(true)` is called**: a confirmed lock answers `true`
//!    (`tests::set_active_true_requests_a_lock`), and a refused lock is an error, never
//!    `false` (`tests::a_refused_set_active_lock_is_an_error`).
//! 6. **`SetActive(false)` is called**: refused with `NotSupported`:
//!    `tests::set_active_false_is_not_supported`.
//! 7. **The lock changes**: `ActiveChanged` is emitted once per change and never for a
//!    repeat: `tests::a_lock_change_is_emitted_once`, and over the bus
//!    `tests/screensaver.rs::active_changed_reaches_a_subscriber_once_per_change`.
//! 8. **The lock object is absent or unreadable**: `GetActive` fails rather than reporting
//!    unlocked, since nothing told us the state: `tests::a_missing_lock_object_fails_closed`.
//! 9. **The compositor's idle object is missing**: the call fails rather than reporting
//!    zero idle time, which would say the user is active when nothing told us:
//!    `tests::a_missing_idle_object_fails_closed`.
//! 10. **The compositor refuses a hold** (its cap is reached): the `Inhibit` call fails and
//!     no inhibit is recorded, so the sender's cookie count is not inflated:
//!     `tests::a_refused_inhibit_leaves_no_entry`.
//! 11. **The machine sleeps** with the session unlocked: the lock is requested while the sleep
//!     delay inhibitor is still held, and the inhibitor is released after it:
//!     `tests/logind.rs::a_sleep_locks_the_session_while_the_inhibitor_is_still_held`. A
//!     refused lock still releases it: `tests/logind.rs::a_refused_lock_still_releases_the_sleep_inhibitor`.
//!     A resume takes the inhibitor again: `tests/logind.rs::a_resume_takes_the_sleep_inhibitor_again_without_locking`.
//! 12. **logind asks for our session to lock** (`loginctl lock-session`): it locks, and a lock
//!     for another session does not: `tests/logind.rs::a_session_lock_on_our_own_session_locks_the_session`
//!     and `tests/logind.rs::a_session_lock_on_another_session_is_ignored`.

pub mod logind;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use futures_lite::StreamExt;

use gnoblin_model::{Fields, ObjectId, Observation, TypeKey, Value};
use gnoblin_service_core::Host;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

gnoblin_log::logger!(log = "screensaver");

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
    /// The `lock` object as the compositor publishes it now, or `None` when it publishes
    /// none. Read once when the service starts; later changes arrive as observations.
    fn lock_reading(&self) -> Result<Option<LockReading>, String>;
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

/// The lock object's identity and whether the session was locked when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockReading {
    /// The lock object, whose later `locked` changes arrive by this identity.
    pub object: ObjectId,
    /// Whether the session is locked.
    pub locked: bool,
}

/// The inhibit table: the sender that holds each cookie. The application and reason are
/// only reported when the inhibit is made, so they are not kept here.
#[derive(Default, Debug)]
pub struct Inhibitors {
    next_cookie: u32,
    table: BTreeMap<u32, String>,
}

impl Inhibitors {
    /// Record an inhibit for `sender` and return its cookie.
    fn inhibit(&mut self, sender: &str) -> Result<u32, fdo::Error> {
        let held = self.table.values().filter(|owner| owner.as_str() == sender).count();
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
        self.table.insert(cookie, sender.to_owned());
        Ok(cookie)
    }

    /// Whether `sender` owns `cookie`. Anything it does not own is treated as unheld.
    fn owned_by(&self, sender: &str, cookie: u32) -> bool {
        self.table.get(&cookie).is_some_and(|owner| owner == sender)
    }

    /// Every cookie `sender` holds, for releasing a departed sender's holds.
    fn cookies_of(&self, sender: &str) -> Vec<u32> {
        self.table
            .iter()
            .filter(|(_, owner)| owner.as_str() == sender)
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

/// The lock object the service follows, and the last lock state it emitted.
#[derive(Default)]
struct LockTracking {
    /// The lock object and its folded `locked` value, or `None` when none is known.
    current: Option<LockReading>,
    /// The last state `observe` returned for emission, so a repeat emits nothing.
    published: Option<bool>,
}

struct Core {
    compositor: Box<dyn Compositor>,
    inhibitors: Mutex<Inhibitors>,
    locked_since: Mutex<Option<Instant>>,
    lock_state: Mutex<LockTracking>,
}

impl ScreenSaver {
    /// A service answering from `compositor`. The lock object is read once here, so the
    /// state is known before the first change arrives; the state read here is not
    /// announced, since nothing changed yet.
    pub fn new(compositor: Box<dyn Compositor>) -> ScreenSaver {
        let current = match compositor.lock_reading() {
            Ok(reading) => reading,
            Err(error) => {
                log::warn!(area = "services", %error, "the screensaver service cannot read the lock object");
                None
            }
        };
        ScreenSaver {
            inner: Arc::new(Core {
                compositor,
                inhibitors: Mutex::new(Inhibitors::default()),
                locked_since: Mutex::new(None),
                lock_state: Mutex::new(LockTracking {
                    current,
                    published: current.map(|reading| reading.locked),
                }),
            }),
        }
    }

    fn inhibitors(&self) -> MutexGuard<'_, Inhibitors> {
        self.inner.inhibitors.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_tracking(&self) -> MutexGuard<'_, LockTracking> {
        self.inner.lock_state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fold one compositor observation into the lock state. `Some` means the folded state
    /// is known and differs from the last state emitted, and the caller must publish it.
    /// `None` means nothing the signal reports changed, or the lock object is not known.
    pub fn observe(&self, observation: &Observation) -> Option<bool> {
        let mut tracking = self.lock_tracking();
        match observation {
            Observation::Opened { view, .. } if view.ty() == &TypeKey::gnoblin("lock") => {
                if let Some(Value::Bool(locked)) = view.property("locked") {
                    tracking.current = Some(LockReading {
                        object: view.id(),
                        locked: *locked,
                    });
                }
            }
            Observation::Changed {
                object,
                property,
                value: Value::Bool(locked),
                ..
            } if property == "locked" => {
                if let Some(current) = tracking.current.as_mut().filter(|current| current.object == *object) {
                    current.locked = *locked;
                }
            }
            Observation::Closed { object, .. } if tracking.current.is_some_and(|current| current.object == *object) => {
                tracking.current = None;
            }
            _ => return None,
        }
        // With no lock object known there is no state to announce, and the last emitted
        // state stays, so a later object that reports the same state emits nothing.
        let locked = tracking.current?.locked;
        if tracking.published == Some(locked) {
            return None;
        }
        tracking.published = Some(locked);
        Some(locked)
    }

    /// Emit `ActiveChanged` with `active` on `connection`, from the primary object path.
    /// A refused emit is logged; `GetActive` still answers the folded state, so the next
    /// change re-reports it.
    pub fn publish(connection: &Connection, active: bool) {
        let result = block_on(async {
            let emitter = SignalEmitter::new(connection, PATHS[0])?;
            ScreenSaver::active_changed(&emitter, active).await
        });
        if let Err(error) = result {
            log::warn!(area = "services", %error, "could not emit ActiveChanged");
        }
    }

    /// Record an inhibit and hold the session awake in the compositor under its holder
    /// name. The table lock is held across the compositor call, so a release of the same
    /// cookie cannot interleave. A refused hold leaves no entry behind.
    fn add_inhibit(&self, sender: &str) -> Result<u32, fdo::Error> {
        let mut table = self.inhibitors();
        let cookie = table.inhibit(sender)?;
        if let Err(error) = self.inner.compositor.inhibit(&holder_name(sender, cookie)) {
            table.forget(cookie);
            return Err(fdo::Error::Failed(format!(
                "the compositor refused the inhibit: {error}"
            )));
        }
        Ok(cookie)
    }

    /// Release a hold `sender` owns, in the compositor first. If the compositor refuses the
    /// release the entry stays, so the client can retry. A cookie `sender` does not own is
    /// refused with `AccessDenied`, and the table is left as it was.
    fn remove_inhibit(&self, sender: &str, cookie: u32) -> Result<(), fdo::Error> {
        let mut table = self.inhibitors();
        if !table.owned_by(sender, cookie) {
            return Err(fdo::Error::AccessDenied(format!(
                "cookie {cookie} is not an inhibit held by {sender}"
            )));
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
                log::warn!(sender, cookie, %error, "the compositor refused to release a departed sender's hold");
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

    /// The folded lock state, or an error when no lock object has been observed.
    fn is_locked(&self) -> Result<bool, fdo::Error> {
        self.lock_tracking()
            .current
            .map(|reading| reading.locked)
            .ok_or_else(|| fdo::Error::Failed("the compositor publishes no lock object".to_owned()))
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
            log::warn!(area = "services", %error, "cannot watch bus names, so inhibits outlive their senders");
            return;
        }
    };
    let Ok(mut owners) = proxy.receive_name_owner_changed().await else {
        log::warn!(area = "services", "cannot subscribe to NameOwnerChanged");
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
        let cookie = self.add_inhibit(&sender)?;
        log::info!(area = "services", %sender, cookie, application = application_name, reason = reason, "screensaver inhibited");
        Ok(cookie)
    }

    /// Release the inhibit `cookie`. A cookie the caller does not own is `AccessDenied`.
    async fn un_inhibit(&self, cookie: u32, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let sender = sender_of(&header)?;
        self.remove_inhibit(&sender, cookie)
    }

    /// Whether the session is locked, as the folded lock state describes it.
    async fn get_active(&self) -> fdo::Result<bool> {
        self.is_locked()
    }

    /// `true` requests a lock and answers `true` once the compositor confirms it. A refused
    /// lock is an error, so the answer is never a silent `false`. `false` is refused, since
    /// clients cannot unlock the session through this service.
    async fn set_active(&self, active: bool) -> fdo::Result<bool> {
        if !active {
            return Err(fdo::Error::NotSupported(
                "clients cannot unlock the session through the screensaver service".to_owned(),
            ));
        }
        self.request_lock()?;
        Ok(true)
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

    /// Emitted with the new lock state when it changes. Never emitted for a repeat of the
    /// state already announced.
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

    fn lock_reading(&self) -> Result<Option<LockReading>, String> {
        let snapshot = self.snapshot().map_err(|error| error.to_string())?;
        let Some(view) = snapshot
            .objects()
            .iter()
            .find(|view| view.ty() == &TypeKey::gnoblin("lock"))
        else {
            return Ok(None);
        };
        match view.property("locked") {
            Some(Value::Bool(locked)) => Ok(Some(LockReading {
                object: view.id(),
                locked: *locked,
            })),
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
            Ok(gnoblin_model::OperationOutcome::Value(_)) => {
                Err("the compositor answered lock_session with a value".to_owned())
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
        Ok(gnoblin_model::OperationOutcome::Value(_)) => Err(format!("the compositor answered {method} with a value")),
        Err(error) => Err(error.to_string()),
    }
}

/// Start the screensaver service on the session bus, or log why it is not served.
pub fn start(host: &Host) -> Option<(ScreenSaver, Connection)> {
    let connection = match block_on(Connection::session()) {
        Ok(connection) => connection,
        Err(error) => {
            log::info!(area = "services", %error, "no session bus, so no screensaver service");
            return None;
        }
    };
    let service = ScreenSaver::new(Box::new(host.clone()));
    match service.register(&connection) {
        Ok(()) => {
            watch_logind(&service);
            Some((service, connection))
        }
        Err(error) => {
            log::warn!(area = "services", %error, "the screensaver service could not be served");
            None
        }
    }
}

/// Follow logind on the system bus, so the session locks before sleep and on request. The
/// service serves without it when there is no system bus, or no logind on it.
fn watch_logind(service: &ScreenSaver) {
    let system = match block_on(Connection::system()) {
        Ok(system) => system,
        Err(error) => {
            log::info!(area = "services", %error, "no system bus, so no logind lock before sleep");
            return;
        }
    };
    if let Err(error) = logind::watch(&system, service) {
        log::warn!(area = "services", %error, "logind is not watched, so the session does not lock before sleep");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_model::{ObjectView, Revision};

    struct Fake {
        idle: Result<u64, String>,
        /// What the `lock` object reads as when the service starts.
        lock_at_start: Result<Option<LockReading>, String>,
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
        fn lock_reading(&self) -> Result<Option<LockReading>, String> {
            self.lock_at_start.clone()
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

    fn lock_at(object: u64, locked: bool) -> LockReading {
        LockReading {
            object: ObjectId::new(object),
            locked,
        }
    }

    fn served_with(
        idle: Result<u64, String>,
        lock_at_start: Result<Option<LockReading>, String>,
        lock: Result<(), String>,
        refuse_inhibit: bool,
    ) -> (Arc<Fake>, ScreenSaver) {
        let fake = Arc::new(Fake {
            idle,
            lock_at_start,
            lock,
            locks: Mutex::new(0),
            holds: Mutex::new(Vec::new()),
            refuse_inhibit,
        });
        (fake.clone(), ScreenSaver::new(Box::new(fake)))
    }

    /// A service whose lock object starts unlocked, as object 7.
    fn fake(idle: Result<u64, String>) -> (Arc<Fake>, ScreenSaver) {
        served_with(idle, Ok(Some(lock_at(7, false))), Ok(()), false)
    }

    fn lock_opened(object: u64, locked: bool) -> Observation {
        let mut properties = Fields::new();
        properties.insert("locked".to_owned(), Value::Bool(locked));
        Observation::Opened {
            view: ObjectView::new(ObjectId::new(object), TypeKey::gnoblin("lock"), properties),
            revision: Revision::new(1),
        }
    }

    fn lock_changed(object: u64, locked: bool) -> Observation {
        Observation::Changed {
            object: ObjectId::new(object),
            property: "locked".to_owned(),
            value: Value::Bool(locked),
            revision: Revision::new(2),
        }
    }

    fn lock_closed(object: u64) -> Observation {
        Observation::Closed {
            object: ObjectId::new(object),
            revision: Revision::new(3),
        }
    }

    #[test]
    fn an_unknown_or_foreign_cookie_is_refused() {
        // The failure mode: a client releases a cookie it does not own, and the release
        // silently succeeds, dropping the owner's hold from under it.
        let (fake, service) = fake(Ok(0));
        let cookie = service.add_inhibit(":1.5").unwrap();
        assert!(matches!(
            service.remove_inhibit(":1.5", cookie + 100),
            Err(fdo::Error::AccessDenied(_))
        ));
        assert!(matches!(
            service.remove_inhibit(":1.9", cookie),
            Err(fdo::Error::AccessDenied(_))
        ));
        assert_eq!(service.inhibitors().table.len(), 1, "a refused release changes nothing");
        assert_eq!(
            fake.holds.lock().unwrap().len(),
            1,
            "the compositor keeps the owner's hold"
        );
        service.remove_inhibit(":1.5", cookie).unwrap();
        assert!(
            matches!(service.remove_inhibit(":1.5", cookie), Err(fdo::Error::AccessDenied(_))),
            "a released cookie is unknown"
        );
    }

    #[test]
    fn a_departed_sender_releases_its_inhibits() {
        let (_, service) = fake(Ok(0));
        service.add_inhibit(":1.5").unwrap();
        let kept = service.add_inhibit(":1.6").unwrap();
        service.release_sender(":1.5");
        let table = &service.inhibitors().table;
        assert_eq!(table.len(), 1);
        assert!(table.contains_key(&kept));
    }

    #[test]
    fn the_65th_inhibit_from_one_sender_is_refused() {
        let (_, service) = fake(Ok(0));
        for _ in 0..MAX_INHIBITS_PER_SENDER {
            service.add_inhibit(":1.5").unwrap();
        }
        let refused = service.add_inhibit(":1.5");
        assert!(matches!(refused, Err(fdo::Error::LimitsExceeded(_))));
        assert!(service.add_inhibit(":1.6").is_ok());
    }

    #[test]
    fn lock_calls_the_compositor_once() {
        let (fake, service) = fake(Ok(0));
        service.request_lock().unwrap();
        assert_eq!(*fake.locks.lock().unwrap(), 1);
    }

    #[test]
    fn a_lock_the_compositor_does_not_confirm_is_an_error() {
        let (fake, service) = served_with(
            Ok(0),
            Ok(Some(lock_at(7, false))),
            Err("the compositor did not lock the session: not-locked".to_owned()),
            false,
        );
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
    fn set_active_true_requests_a_lock() {
        // The failure mode: `SetActive` always answered `false`, so a caller could not
        // tell the lock was requested. A confirmed lock answers `true`.
        let (fake, service) = fake(Ok(0));
        assert!(matches!(block_on(service.set_active(true)), Ok(true)));
        assert_eq!(*fake.locks.lock().unwrap(), 1);
    }

    #[test]
    fn a_refused_set_active_lock_is_an_error() {
        let (fake, service) = served_with(
            Ok(0),
            Ok(Some(lock_at(7, false))),
            Err("the compositor did not lock the session: not-locked".to_owned()),
            false,
        );
        assert!(matches!(block_on(service.set_active(true)), Err(fdo::Error::Failed(_))));
        assert_eq!(*fake.locks.lock().unwrap(), 1);
        assert!(service.inner.locked_since.lock().unwrap().is_none());
    }

    #[test]
    fn set_active_false_is_not_supported() {
        let (fake, service) = fake(Ok(0));
        assert!(matches!(
            block_on(service.set_active(false)),
            Err(fdo::Error::NotSupported(_))
        ));
        assert_eq!(
            *fake.locks.lock().unwrap(),
            0,
            "a refused unlock reaches no compositor method"
        );
    }

    #[test]
    fn a_missing_idle_object_fails_closed() {
        let (_, service) = fake(Err("the compositor publishes no idle object".to_owned()));
        assert!(service.idle_seconds().is_err());
    }

    #[test]
    fn a_departed_sender_releases_its_holds_in_the_compositor() {
        let (fake, service) = fake(Ok(0));
        let cookie = service.add_inhibit(":1.5").unwrap();
        assert_eq!(*fake.holds.lock().unwrap(), vec![format!(":1.5:{cookie}")]);
        service.release_sender(":1.5");
        assert!(fake.holds.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_inhibit_leaves_no_entry() {
        let (fake, service) = served_with(Ok(0), Ok(Some(lock_at(7, false))), Ok(()), true);
        assert!(service.add_inhibit(":1.5").is_err());
        assert!(service.inhibitors().cookies_of(":1.5").is_empty());
        assert!(fake.holds.lock().unwrap().is_empty());
    }

    #[test]
    fn a_lock_change_is_emitted_once() {
        // The failure mode: `ActiveChanged` was declared and never emitted, so a locker
        // that waited for it never saw the session lock.
        let (_, service) = fake(Ok(0));
        assert_eq!(service.observe(&lock_changed(7, true)), Some(true));
        assert_eq!(service.observe(&lock_changed(7, true)), None, "a repeat emits nothing");
        assert_eq!(
            service.observe(&lock_opened(7, true)),
            None,
            "a duplicate open emits nothing"
        );
        assert_eq!(service.observe(&lock_changed(7, false)), Some(false));
    }

    #[test]
    fn the_state_read_at_start_is_not_announced() {
        let (_, service) = fake(Ok(0));
        assert_eq!(service.observe(&lock_opened(7, false)), None);
    }

    #[test]
    fn a_change_to_another_object_is_ignored() {
        let (_, service) = fake(Ok(0));
        assert_eq!(service.observe(&lock_changed(9, true)), None);
        assert!(matches!(block_on(service.get_active()), Ok(false)));
    }

    #[test]
    fn a_closed_lock_object_is_unknown_and_a_returning_one_announces_only_a_change() {
        let (_, service) = fake(Ok(0));
        assert_eq!(
            service.observe(&lock_closed(7)),
            None,
            "losing the object is not a lock change"
        );
        assert!(matches!(block_on(service.get_active()), Err(fdo::Error::Failed(_))));
        assert_eq!(service.observe(&lock_opened(8, false)), None, "same state as before");
        assert_eq!(service.observe(&lock_opened(8, true)), Some(true));
    }

    #[test]
    fn get_active_answers_the_folded_lock_state() {
        let (_, service) = fake(Ok(0));
        assert!(matches!(block_on(service.get_active()), Ok(false)));
        service.observe(&lock_changed(7, true));
        assert!(matches!(block_on(service.get_active()), Ok(true)));
    }

    #[test]
    fn a_missing_lock_object_fails_closed() {
        let (_, service) = served_with(Ok(0), Ok(None), Ok(()), false);
        assert!(matches!(block_on(service.get_active()), Err(fdo::Error::Failed(_))));
    }

    #[test]
    fn an_unreadable_lock_object_at_start_fails_closed() {
        let (_, service) = served_with(Ok(0), Err("the lock object has no locked".to_owned()), Ok(()), false);
        assert!(matches!(block_on(service.get_active()), Err(fdo::Error::Failed(_))));
    }
}
