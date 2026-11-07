//! The `org.freedesktop.a11y.KeyboardMonitor` service on the session bus.
//!
//! An assistive technology talks to the compositor on the well-known name
//! `org.freedesktop.a11y.Manager`, at `/org/freedesktop/a11y/Manager`. Its
//! method calls become [`gnoblin_a11y::Event`]s, and the compositor loop drains
//! them from [`KeyboardMonitorService::events`]. The keys the monitors must hear go the
//! other way: the loop hands each one to [`KeyReporter::report`], and the `KeyEvent`
//! signal goes unicast to each owner it names.
//!
//! The D-Bus side never decides anything about keys. The accessibility machines
//! own every decision; this module only translates the calls in and the
//! reports out, and it keeps the one table that maps a bus name to the
//! [`OwnerId`] the machines know it by.
//!
//! # Bounded channels
//!
//! Both channels are bounded. The key path must never wait on the bus, so the
//! compositor side only ever calls `try_send` and `try_recv`:
//!
//! - A full report queue drops that `KeyEvent` delivery and counts it. The
//!   count is logged on each drop, so a slow monitor is visible without ever
//!   stalling a key.
//! - A full event queue refuses the D-Bus call with a "busy" error, so the
//!   caller can retry; the machines never see a half-applied grab.
//!
//! Threads: zbus serves the object on its own executor. Two plain threads sit
//! beside it: one watches `NameOwnerChanged` so a departed monitor's grabs end,
//! and one turns each report into signals.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_lite::StreamExt;
use gnoblin_a11y::ids::{Keysym, OwnerId};
use gnoblin_a11y::{Event, Keystroke, Modifiers};
use gnoblin_log::warn_in;
use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::{Connection, interface};

/// The well-known name the compositor owns for the accessibility manager.
pub const BUS_NAME: &str = "org.freedesktop.a11y.Manager";
/// The object path the interface is served at.
pub const PATH: &str = "/org/freedesktop/a11y/Manager";
/// The interface name, as the specification writes it.
pub const INTERFACE: &str = "org.freedesktop.a11y.KeyboardMonitor";

/// How many calls may wait for the compositor loop before new ones are refused.
///
/// A monitor makes a handful of calls at startup, so this is far above what a
/// well-behaved client needs and small enough that a stuck loop shows up fast.
pub const EVENT_CAPACITY: usize = 256;

/// How many key reports may wait for their signals before new ones are dropped.
///
/// Sized for a burst of typing with one monitor that is briefly slow. A monitor
/// that stays behind for longer than this loses keys, which is counted rather
/// than hidden.
pub const REPORT_CAPACITY: usize = 256;

/// One key event the monitors must hear, with the owners that asked for it.
///
/// The fields are the `KeyEvent` signal's body in order, so the D-Bus side
/// sends them without converting anything. `keycode` is `u16` because the
/// specification says so; a hardware keycode never needs more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyReport {
    /// The owners that receive the signal, in owner order. An empty list sends
    /// nothing, which is what a pass-through key looks like here.
    pub owners: Vec<OwnerId>,
    /// Whether this is the key-up half of the pair.
    pub released: bool,
    /// The modifier mask currently pressed, in XKB bits.
    pub state: u32,
    /// The XKB keysym of the key.
    pub keysym: u32,
    /// The character the key produces, or zero.
    pub unichar: u32,
    /// The hardware keycode of the key.
    pub keycode: u16,
}

/// The compositor loop's handle on the report queue.
///
/// [`report`](Self::report) never blocks. A report that finds the queue full is
/// dropped and counted, so a monitor that cannot keep up costs keys it never
/// sees rather than a stall on every key the user types.
#[derive(Debug)]
pub struct KeyReporter {
    /// The bounded queue the report thread drains.
    queue: SyncSender<KeyReport>,
    /// How many reports were dropped because the queue was full.
    dropped: AtomicU64,
}

impl KeyReporter {
    /// Queue one report for its signals, or drop and count it if the queue is
    /// full. Returns whether the report was queued.
    pub fn report(&self, report: KeyReport) -> bool {
        match self.queue.try_send(report) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                warn_in!("a11y", %dropped, "[a11y] KeyEvent queue is full, so a monitor missed a key");
                false
            }
            // The report thread is gone: the bus connection closed. There is no
            // one left to tell, and the key still reaches the client.
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// How many reports were dropped because the queue was full, since start.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The two ends the compositor loop holds: the calls that arrived, and the
/// reporter for key deliveries.
#[derive(Debug)]
pub struct KeyboardMonitorService {
    /// Calls from monitors and the departure of one, in arrival order.
    pub events: Receiver<Event>,
    /// Key reports to turn into `KeyEvent` signals.
    pub reports: KeyReporter,
}

/// The name-to-owner table, shared by the method calls, the owner watch and the
/// report thread.
///
/// An owner is allocated on a monitor's first call and forgotten when its bus
/// name leaves. Owners are never reused, so an event that arrives late can name
/// a departed monitor without ever naming its successor.
#[derive(Debug, Default)]
struct MonitorOwners {
    /// The last owner handed out. Starts at zero so the first owner is one.
    last: u64,
    /// The owner of each unique bus name.
    by_name: BTreeMap<String, OwnerId>,
    /// The unique bus name of each owner, for addressing the signal.
    by_owner: BTreeMap<OwnerId, String>,
}

impl MonitorOwners {
    /// The owner of `name`, allocated on first sight.
    fn owner_of(&mut self, name: &str) -> OwnerId {
        if let Some(owner) = self.by_name.get(name) {
            return *owner;
        }
        self.last += 1;
        let owner = OwnerId::new(self.last);
        self.by_name.insert(name.to_owned(), owner);
        self.by_owner.insert(owner, name.to_owned());
        owner
    }

    /// Forgets `name`, returning its owner if it had one.
    fn forget(&mut self, name: &str) -> Option<OwnerId> {
        let owner = self.by_name.remove(name)?;
        self.by_owner.remove(&owner);
        Some(owner)
    }

    /// The unique bus name of `owner`, if it is still connected.
    fn name_of(&self, owner: OwnerId) -> Option<&str> {
        self.by_owner.get(&owner).map(String::as_str)
    }
}

/// The interface object served at [`PATH`].
#[derive(Clone)]
struct KeyboardMonitor {
    /// Where the calls go, for the compositor loop to apply.
    events: SyncSender<Event>,
    /// The shared name-to-owner table.
    owners: Arc<Mutex<MonitorOwners>>,
}

impl KeyboardMonitor {
    /// The table, unpoisoned: a panic elsewhere must not stop a grab from ending.
    fn owners(&self) -> MutexGuard<'_, MonitorOwners> {
        self.owners.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The owner of the caller of this call.
    fn caller(&self, header: &Header<'_>) -> fdo::Result<OwnerId> {
        let sender = header
            .sender()
            .ok_or_else(|| fdo::Error::Failed("the call has no sender".to_owned()))?;
        Ok(self.owners().owner_of(sender.as_str()))
    }

    /// Hands one event to the compositor loop without waiting for it.
    ///
    /// A full queue is refused with a "busy" error rather than waited on, so the
    /// D-Bus executor never parks behind a slow compositor.
    fn send(&self, event: Event) -> fdo::Result<()> {
        self.events.try_send(event).map_err(|error| match error {
            TrySendError::Full(_) => fdo::Error::Failed("the accessibility manager is busy, try again".to_owned()),
            TrySendError::Disconnected(_) => fdo::Error::Failed("the compositor is no longer listening".to_owned()),
        })
    }
}

#[interface(name = "org.freedesktop.a11y.KeyboardMonitor")]
impl KeyboardMonitor {
    /// Start sending every key event to the caller, without taking it away.
    async fn watch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let owner = self.caller(&header)?;
        self.send(Event::WatchKeyboard { owner })
    }

    /// Stop sending every key event to the caller. Grabs stay in effect.
    async fn unwatch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let owner = self.caller(&header)?;
        self.send(Event::UnwatchKeyboard { owner })
    }

    /// Take every key away from clients and send it to the caller.
    async fn grab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let owner = self.caller(&header)?;
        self.send(Event::GrabKeyboard { owner })
    }

    /// Stop taking every key away. Keystroke and modifier grabs stay in effect.
    async fn ungrab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        let owner = self.caller(&header)?;
        self.send(Event::UngrabKeyboard { owner })
    }

    /// Replace the caller's modifier and keystroke grabs.
    ///
    /// `modifiers` are XKB keysyms. `keystrokes` pair an XKB keysym with the
    /// exact XKB modifier mask that must be held with it.
    async fn set_key_grabs(
        &self,
        modifiers: Vec<u32>,
        keystrokes: Vec<(u32, u32)>,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let owner = self.caller(&header)?;
        let modifiers = modifiers.into_iter().map(Keysym::new).collect();
        let keystrokes = keystrokes
            .into_iter()
            .map(|(keysym, mask)| Keystroke {
                keysym: Keysym::new(keysym),
                modifiers: Modifiers::new(mask),
            })
            .collect();
        self.send(Event::SetKeyGrabs {
            owner,
            modifiers,
            keystrokes,
        })
    }
}

/// Connect to the bus, serve the interface, own the name and start the two
/// threads. `address` names a private bus for tests; `None` is the session bus.
///
/// Fails when no bus is reachable or the name is taken. A caller that gets an
/// error keeps running without the service: keys then pass through untouched.
pub fn start(address: Option<&str>) -> zbus::Result<KeyboardMonitorService> {
    let connection = block_on(connect(address))?;
    let (event_tx, event_rx) = mpsc::sync_channel(EVENT_CAPACITY);
    let (report_tx, report_rx) = mpsc::sync_channel(REPORT_CAPACITY);
    let owners = Arc::new(Mutex::new(MonitorOwners::default()));
    let monitor = KeyboardMonitor {
        events: event_tx.clone(),
        owners: Arc::clone(&owners),
    };
    block_on(connection.object_server().at(PATH, monitor))?;
    block_on(connection.request_name(BUS_NAME))?;

    let watcher = connection.clone();
    let watched_owners = Arc::clone(&owners);
    std::thread::Builder::new()
        .name("a11y-monitor-owners".to_owned())
        .spawn(move || block_on(watch_owners(watcher, event_tx, watched_owners)))
        .map_err(|error| zbus::Error::InputOutput(Arc::new(error)))?;

    std::thread::Builder::new()
        .name("a11y-monitor-reports".to_owned())
        .spawn(move || emit_reports(connection, report_rx, owners))
        .map_err(|error| zbus::Error::InputOutput(Arc::new(error)))?;

    Ok(KeyboardMonitorService {
        events: event_rx,
        reports: KeyReporter {
            queue: report_tx,
            dropped: AtomicU64::new(0),
        },
    })
}

/// Connect to the given bus, or the session bus when there is none.
async fn connect(address: Option<&str>) -> zbus::Result<Connection> {
    match address {
        Some(address) => Builder::address(address).build().await,
        None => Connection::session().await,
    }
}

/// Watch `NameOwnerChanged` and forget every monitor whose bus name leaves.
///
/// The compositor learns of the departure as [`Event::MonitorDropped`], which
/// is the only way a grab ends when its owner vanishes without unwatching. This
/// is a dedicated thread, so it may wait for the queue; the key path never does.
async fn watch_owners(connection: Connection, events: SyncSender<Event>, owners: Arc<Mutex<MonitorOwners>>) {
    let proxy = match fdo::DBusProxy::new(&connection).await {
        Ok(proxy) => proxy,
        Err(error) => {
            warn_in!("a11y", %error, "[a11y] cannot watch bus names, so a departed monitor keeps its grabs");
            return;
        }
    };
    let Ok(mut changes) = proxy.receive_name_owner_changed().await else {
        warn_in!("a11y", "[a11y] cannot subscribe to NameOwnerChanged");
        return;
    };
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_some() {
            continue;
        }
        let name = args.name().to_string();
        let departed = owners.lock().unwrap_or_else(PoisonError::into_inner).forget(&name);
        if let Some(owner) = departed {
            // A closed channel means the compositor is gone; there is no one left
            // to tell, so the send is allowed to fail.
            let _ = events.send(Event::MonitorDropped { owner });
        }
    }
}

/// Send each report's `KeyEvent` to every owner it names that is still here.
///
/// Returns when the compositor drops its end of the report queue.
fn emit_reports(connection: Connection, reports: Receiver<KeyReport>, owners: Arc<Mutex<MonitorOwners>>) {
    while let Ok(report) = reports.recv() {
        let names: Vec<String> = {
            let table = owners.lock().unwrap_or_else(PoisonError::into_inner);
            report
                .owners
                .iter()
                .filter_map(|owner| table.name_of(*owner).map(str::to_owned))
                .collect()
        };
        let body = (
            report.released,
            report.state,
            report.keysym,
            report.unichar,
            report.keycode,
        );
        for name in names {
            let sent = block_on(connection.emit_signal(Some(name.as_str()), PATH, INTERFACE, "KeyEvent", &body));
            if let Err(error) = sent {
                warn_in!("a11y", %error, "[a11y] cannot send a KeyEvent to a monitor");
            }
        }
    }
}

/// Run a future to completion on the calling thread.
fn block_on<F: Future>(future: F) -> F::Output {
    futures_lite::future::block_on(future)
}
