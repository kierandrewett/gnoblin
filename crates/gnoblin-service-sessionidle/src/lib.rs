//! `org.gnoblin.SessionIdle`, served from the services role on the ScreenSaver connection.
//!
//! The interface reports the idle state Gnoblin derives from the compositor's `idle`
//! object, as four values: whether idle state is available, whether the fixed threshold
//! has elapsed, the threshold itself, and how long the session has been idle. It is served
//! at `/org/gnoblin/SessionIdle` on the connection that owns `org.freedesktop.ScreenSaver`.
//!
//! # Where the state comes from
//!
//! [`serve`] takes one snapshot of the compositor, which is also the one subscription this
//! service holds. Every later `idle` observation the role receives is folded into the state
//! by [`SessionIdle::observe`], and `GetActivity` answers from that folded state. It never
//! makes a compositor call of its own: `Host::snapshot` subscribes afresh on every call, so
//! a per-call read would leave one feed alive in the compositor for each call.
//!
//! # Failure modes first
//!
//! 1. **The idle object is absent**: `available` is false, `idle` is false and `idle_for_ms`
//!    is zero, rather than an error, since the state genuinely is unavailable:
//!    `tests/session_idle.rs::an_absent_idle_object_is_unavailable`.
//! 2. **The idle object goes away** after it was seen: the same unavailable answer, and one
//!    `ActivityChanged`: `tests/session_idle.rs::losing_the_idle_object_reports_unavailable_once`.
//! 3. **The threshold is crossed** only at 120000 ms, inclusive: `tests/session_idle.rs::the_threshold_is_inclusive`.
//! 4. **Duration changes alone** emit nothing, since the signal is a sample:
//!    `tests/session_idle.rs::a_change_to_the_duration_alone_emits_nothing`.
//! 5. **A duplicate observation** arrives, as it can when another subscription on the same
//!    connection reports the same change: nothing changes and nothing is emitted:
//!    `tests/session_idle.rs::a_duplicate_observation_emits_nothing`.
//! 6. **The compositor cannot be read at start**: the interface is not served rather than
//!    answering an idle state nobody observed: `tests/session_idle.rs::an_unreadable_compositor_is_not_served`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_model::{ObjectId, Observation, TypeKey, Value};
use gnoblin_service_core::Host;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

gnoblin_log::logger!(log = "sessionidle");

/// The object path the interface is served on.
pub const PATH: &str = "/org/gnoblin/SessionIdle";
/// The idle threshold, fixed by the spec and not configurable here.
pub const THRESHOLD_MS: u64 = 120_000;

/// The compositor read the service takes once, when it starts.
pub trait Compositor {
    /// The idle object as the compositor publishes it now, or `None` when it publishes none.
    fn idle(&self) -> Result<Option<Reading>, String>;
}

/// The idle object's identity and its idle time when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The idle object, whose later `idle_ms` changes arrive by this identity.
    pub object: ObjectId,
    /// Milliseconds since the last input event.
    pub idle_ms: u64,
}

/// The four values the interface reports, and the signal carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activity {
    /// Whether the compositor publishes an idle object.
    pub available: bool,
    /// Whether the idle time has reached the threshold.
    pub idle: bool,
    /// The fixed threshold in milliseconds.
    pub threshold_ms: u64,
    /// The idle time while idle, or zero when not idle or unavailable.
    pub idle_for_ms: u64,
}

impl Activity {
    /// The activity a reading describes. No reading means nothing is available.
    fn of(reading: Option<Reading>) -> Activity {
        match reading {
            None => Activity {
                available: false,
                idle: false,
                threshold_ms: THRESHOLD_MS,
                idle_for_ms: 0,
            },
            Some(reading) => {
                let idle = reading.idle_ms >= THRESHOLD_MS;
                Activity {
                    available: true,
                    idle,
                    threshold_ms: THRESHOLD_MS,
                    idle_for_ms: if idle { reading.idle_ms } else { 0 },
                }
            }
        }
    }

    /// The pair the signal is keyed on: only a change in this pair emits.
    const fn state(self) -> (bool, bool) {
        (self.available, self.idle)
    }
}

/// The state the service folds, and the last `(available, idle)` pair it emitted.
#[derive(Default)]
struct Tracking {
    current: Option<Reading>,
    published: Option<(bool, bool)>,
}

struct Core {
    connection: Connection,
    tracking: Mutex<Tracking>,
}

/// The session idle service: the interface's state, shared between its callers and the
/// role that folds the compositor's observations into it.
#[derive(Clone)]
pub struct SessionIdle {
    inner: Arc<Core>,
}

impl SessionIdle {
    fn tracking(&self) -> MutexGuard<'_, Tracking> {
        self.inner.tracking.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fold one compositor observation. `Some` means the `(available, idle)` pair changed
    /// since the last emission, and the caller must publish the returned activity; `None`
    /// means nothing the signal reports changed.
    pub fn observe(&self, observation: &Observation) -> Option<Activity> {
        let mut tracking = self.tracking();
        match observation {
            Observation::Opened { view, .. } if view.ty() == &TypeKey::gnoblin("idle") => {
                if let Some(Value::Uint(idle_ms)) = view.property("idle_ms") {
                    tracking.current = Some(Reading {
                        object: view.id(),
                        idle_ms: *idle_ms,
                    });
                }
            }
            Observation::Changed {
                object,
                property,
                value: Value::Uint(idle_ms),
                ..
            } if property == "idle_ms" => {
                if let Some(current) = tracking.current.as_mut().filter(|current| current.object == *object) {
                    current.idle_ms = *idle_ms;
                }
            }
            Observation::Closed { object, .. } if tracking.current.is_some_and(|current| current.object == *object) => {
                tracking.current = None;
            }
            _ => return None,
        }
        let activity = Activity::of(tracking.current);
        if tracking.published == Some(activity.state()) {
            return None;
        }
        tracking.published = Some(activity.state());
        Some(activity)
    }

    /// Emit `ActivityChanged` with `activity` on the connection. A refused emit is logged;
    /// `GetActivity` still answers the folded state, so the next change re-reports it.
    pub fn publish(&self, activity: Activity) {
        let result = futures_lite::future::block_on(async {
            let emitter = SignalEmitter::new(&self.inner.connection, PATH)?;
            SessionIdle::activity_changed(
                &emitter,
                activity.available,
                activity.idle,
                activity.threshold_ms,
                activity.idle_for_ms,
            )
            .await
        });
        if let Err(error) = result {
            log::warn!(area = "services", %error, "could not emit ActivityChanged");
        }
    }

    /// The activity the folded state describes now.
    fn activity(&self) -> Activity {
        Activity::of(self.tracking().current)
    }
}

/// Serve the interface on `connection` at [`PATH`], seeded from the compositor's idle
/// object. `None` means the interface is not served: the compositor could not be read, or
/// the path was already taken.
pub fn serve(compositor: &dyn Compositor, connection: &Connection) -> Option<SessionIdle> {
    let reading = match compositor.idle() {
        Ok(reading) => reading,
        Err(error) => {
            log::warn!(area = "services", %error, "the session idle service cannot read the compositor");
            return None;
        }
    };
    let service = SessionIdle {
        inner: Arc::new(Core {
            connection: connection.clone(),
            tracking: Mutex::new(Tracking {
                current: reading,
                published: Some(Activity::of(reading).state()),
            }),
        }),
    };
    match futures_lite::future::block_on(connection.object_server().at(PATH, service.clone())) {
        Ok(None) => {}
        Ok(Some(_)) => {
            log::warn!(area = "services", "the session idle path is already served");
            return None;
        }
        Err(error) => {
            log::warn!(area = "services", %error, "the session idle interface could not be served");
            return None;
        }
    }
    log::info!(
        area = "services",
        available = reading.is_some(),
        "session idle interface served"
    );
    Some(service)
}

#[interface(name = "org.gnoblin.SessionIdle")]
impl SessionIdle {
    /// The idle state as the folded state describes it.
    async fn get_activity(&self) -> (bool, bool, u64, u64) {
        let activity = self.activity();
        (
            activity.available,
            activity.idle,
            activity.threshold_ms,
            activity.idle_for_ms,
        )
    }

    /// Emitted when availability or idle state changes.
    #[zbus(signal)]
    async fn activity_changed(
        emitter: &SignalEmitter<'_>,
        available: bool,
        idle: bool,
        threshold_ms: u64,
        idle_for_ms: u64,
    ) -> zbus::Result<()>;
}

/// The compositor, reached through the services role's host.
impl Compositor for Host {
    fn idle(&self) -> Result<Option<Reading>, String> {
        let snapshot = self.snapshot().map_err(|error| error.to_string())?;
        let Some(view) = snapshot
            .objects()
            .iter()
            .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        else {
            return Ok(None);
        };
        match view.property("idle_ms") {
            Some(Value::Uint(idle_ms)) => Ok(Some(Reading {
                object: view.id(),
                idle_ms: *idle_ms,
            })),
            _ => Err("the idle object has no idle_ms".to_owned()),
        }
    }
}
