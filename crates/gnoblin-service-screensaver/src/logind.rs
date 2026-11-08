//! The logind side of the screensaver service: lock before the machine sleeps, and lock
//! when logind asks for this session to be locked.
//!
//! logind waits for every delay inhibitor to be released before it suspends, up to its
//! `InhibitDelayMaxSec`. The service holds one delay inhibitor for `sleep`. On
//! `PrepareForSleep(true)` it locks the session while the inhibitor is still held, then
//! releases it so the suspend can go ahead. On `PrepareForSleep(false)` (resume) it takes
//! a new inhibitor for the next suspend. A refused lock still releases the inhibitor, so a
//! compositor that cannot lock never blocks the suspend past logind's delay.
//!
//! A session `Lock` signal (`loginctl lock-session`) locks the service's own session. The
//! service watches only the session logind reports for its own process, so a lock aimed at
//! another session on the seat does not lock this one.

use std::sync::{Mutex, PoisonError};

use futures_lite::StreamExt;
use zbus::{Connection, OwnedFd, OwnedObjectPath, Proxy};

use crate::{ScreenSaver, block_on, log};

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
/// The reason logind shows for the delay, in its own list of inhibitors.
const WHY: &str = "Locking the session before the machine sleeps";

/// The delay inhibitor the service holds, or `None` while it is released.
type Inhibitor = Mutex<Option<OwnedFd>>;

/// Watch logind on `system` for sleep and for locks of this process's session, locking
/// `service` as they arrive. The sleep inhibitor is taken before this returns, so a
/// suspend that starts at once still waits for the lock.
pub fn watch(system: &Connection, service: &ScreenSaver) -> zbus::Result<()> {
    block_on(async {
        let manager = Proxy::new(system, LOGIND, MANAGER_PATH, MANAGER).await?;
        let session_path: OwnedObjectPath = manager.call("GetSessionByPID", &std::process::id()).await?;
        let session = Proxy::new(system, LOGIND, session_path.to_string(), SESSION).await?;

        // Subscribe before the threads start, so no signal sent after this returns is missed.
        let sleeps = manager.receive_signal("PrepareForSleep").await?;
        let locks = session.receive_signal("Lock").await?;

        let inhibitor: Inhibitor = Mutex::new(None);
        take_inhibitor(&manager, &inhibitor).await?;

        let sleep_service = service.clone();
        let sleep_manager = manager.clone();
        std::thread::Builder::new()
            .name("screensaver-sleep".to_owned())
            .spawn(move || {
                block_on(sleep_loop(sleep_service, sleep_manager, sleeps, inhibitor));
            })
            .map_err(|error| zbus::Error::InputOutput(std::sync::Arc::new(error)))?;

        let lock_service = service.clone();
        std::thread::Builder::new()
            .name("screensaver-session-lock".to_owned())
            .spawn(move || {
                block_on(lock_loop(lock_service, locks));
            })
            .map_err(|error| zbus::Error::InputOutput(std::sync::Arc::new(error)))?;
        Ok(())
    })
}

/// Take a delay inhibitor for `sleep` and keep its descriptor in `slot`. logind closes the
/// inhibit when the descriptor is closed, so dropping it releases the delay.
async fn take_inhibitor(manager: &Proxy<'_>, slot: &Inhibitor) -> zbus::Result<()> {
    let fd: OwnedFd = manager.call("Inhibit", &("sleep", "Gnoblin", WHY, "delay")).await?;
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(fd);
    Ok(())
}

/// Lock before each suspend, release the delay after the lock, and take a new delay on
/// resume. Runs until logind's signal stream ends.
async fn sleep_loop(
    service: ScreenSaver,
    manager: Proxy<'static>,
    mut sleeps: zbus::proxy::SignalStream<'static>,
    inhibitor: Inhibitor,
) {
    while let Some(message) = sleeps.next().await {
        let going_down = match message.body().deserialize::<bool>() {
            Ok(going_down) => going_down,
            Err(error) => {
                log::warn!(area = "services", %error, "a PrepareForSleep signal did not carry a boolean");
                continue;
            }
        };
        if going_down {
            if let Err(error) = service.request_lock() {
                log::warn!(area = "services", %error, "the session could not be locked before sleep");
            }
            // Released whether or not the lock held: logind's delay is only there to let
            // the lock finish, and a refused lock must not hold the suspend back.
            inhibitor.lock().unwrap_or_else(PoisonError::into_inner).take();
        } else if let Err(error) = take_inhibitor(&manager, &inhibitor).await {
            log::warn!(area = "services", %error, "the sleep inhibitor could not be taken again after resume");
        }
    }
}

/// Lock the session on each `Lock` signal logind sends for it.
async fn lock_loop(service: ScreenSaver, mut locks: zbus::proxy::SignalStream<'static>) {
    while locks.next().await.is_some() {
        if let Err(error) = service.request_lock() {
            log::warn!(area = "services", %error, "the session could not be locked on logind's request");
        }
    }
}
