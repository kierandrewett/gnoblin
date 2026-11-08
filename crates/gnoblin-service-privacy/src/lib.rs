//! `org.gnoblin.Privacy`, served on the session bus by the services role.
//!
//! The interface reports whether a microphone or camera is in use right now, as three
//! values: whether the PipeWire graph has been read on the current connection, whether a
//! running microphone capture node exists, and whether a running camera capture node
//! exists. `GetActivity` returns the last published triple, and `ActivityChanged` carries
//! the same triple whenever it changes.
//!
//! # Where the state comes from
//!
//! A dedicated thread owns the PipeWire client and folds registry, node and core events
//! into [`model::Tracker`]. The thread is the only writer of the published triple, so an
//! emitted change always matches the state a caller reads next. The model is pure and is
//! unit-tested; the `pipewire` module only translates PipeWire callbacks into calls on it.
//!
//! # Failure modes
//!
//! The PipeWire state machine (what counts as in use, loss, reset, errors) is covered by
//! unit tests in `model`, which need no bus or PipeWire. This module adds the bus layer:
//!
//! - **No session bus, or the name is taken**: the interface is not served and nothing is
//!   published, rather than answering an activity nobody observed. Each is logged once.
//! - **PipeWire is not running**: the published state stays all-false and the thread retries
//!   every [`RECONNECT_DELAY`]; the retry is logged once per outage.
//! - **PipeWire goes away after it was read**: the connection is rebuilt and the published
//!   state drops to unavailable through the same [`Privacy::publish`] path.
//! - **Repeated identical state** emits nothing, because [`Privacy::publish`] compares with
//!   the stored value first.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_lite::future::block_on;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

mod model;
mod pipewire;

pub use model::Activity;

gnoblin_log::logger!(log = "privacy");

/// The well-known name the service owns on the session bus.
pub const BUS_NAME: &str = "org.gnoblin.Privacy";

/// The object path the interface is served on.
pub const PATH: &str = "/org/gnoblin/Privacy";

/// How long the PipeWire thread waits before it reconnects after a failure. Short enough
/// that a restarted PipeWire is noticed quickly, long enough not to spin while it is down.
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

struct SharedActivity {
    connection: Connection,
    published: Mutex<Activity>,
}

/// The privacy service: the published activity, shared between the interface's callers and
/// the PipeWire thread that updates it.
#[derive(Clone)]
pub struct Privacy {
    shared: Arc<SharedActivity>,
}

impl Privacy {
    fn published(&self) -> MutexGuard<'_, Activity> {
        self.shared.published.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publish `activity`. A value equal to the published one emits nothing, so the caller
    /// may pass every state it computes.
    pub fn publish(&self, activity: Activity) {
        {
            let mut published = self.published();
            if *published == activity {
                return;
            }
            *published = activity;
        }
        let result = block_on(async {
            let emitter = SignalEmitter::new(&self.shared.connection, PATH)?;
            Privacy::activity_changed(
                &emitter,
                activity.available,
                activity.microphone_in_use,
                activity.camera_in_use,
            )
            .await
        });
        if let Err(error) = result {
            log::warn!(area = "services", ?error, "could not emit ActivityChanged");
        }
    }

    fn activity(&self) -> Activity {
        *self.published()
    }
}

#[interface(name = "org.gnoblin.Privacy")]
impl Privacy {
    /// The published microphone and camera activity.
    async fn get_activity(&self) -> (bool, bool, bool) {
        let activity = self.activity();
        (activity.available, activity.microphone_in_use, activity.camera_in_use)
    }

    /// Emitted when availability, microphone use or camera use changes.
    #[zbus(signal)]
    async fn activity_changed(
        emitter: &SignalEmitter<'_>,
        available: bool,
        microphone_in_use: bool,
        camera_in_use: bool,
    ) -> zbus::Result<()>;
}

/// Connect to the session bus, own [`BUS_NAME`], serve [`PATH`] and start the PipeWire
/// thread. `None` when any of those fails; each failure is logged once.
pub fn start() -> Option<Privacy> {
    let connection = match block_on(Connection::session()) {
        Ok(connection) => connection,
        Err(error) => {
            log::info!(area = "services", %error, "no session bus, so no privacy service");
            return None;
        }
    };
    match block_on(connection.request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            log::warn!(
                area = "services",
                "{BUS_NAME} is already owned, so privacy is not served"
            );
            return None;
        }
        Err(error) => {
            log::warn!(area = "services", %error, "the privacy bus name could not be requested");
            return None;
        }
    }
    let service = Privacy {
        shared: Arc::new(SharedActivity {
            connection,
            published: Mutex::new(Activity::default()),
        }),
    };
    match block_on(service.shared.connection.object_server().at(PATH, service.clone())) {
        Ok(None) => {}
        Ok(Some(_)) => {
            log::warn!(area = "services", "the privacy path is already served");
            return None;
        }
        Err(error) => {
            log::warn!(area = "services", %error, "the privacy interface could not be served");
            return None;
        }
    }
    let worker = service.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("privacy-pipewire".to_owned())
        .spawn(move || pipewire::run(&worker))
    {
        log::warn!(area = "services", %error, "the PipeWire thread could not start");
        return None;
    }
    log::info!(area = "services", "privacy interface served");
    Some(service)
}
