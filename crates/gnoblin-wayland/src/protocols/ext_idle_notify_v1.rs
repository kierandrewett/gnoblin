//! XML protocol `ext_idle_notify_v1` (`<protocol name="ext_idle_notify_v1">`); interfaces served: `ext_idle_notifier_v1`.
//!
//! `ext_idle_notifier_v1`: a client is told when the session goes idle and when it
//! comes back.
//!
//! ## Why this is not Smithay's implementation
//!
//! Smithay's `IdleNotifierState` arms a callback timer per notification in the event
//! loop. That makes idle a property of wall time, and a test then has to sleep to see
//! it — which is both slow and, on a loaded machine, flaky. Idle here is a property of
//! the session's **tick counter**: `gnoblin-input`'s [`Activity`] records when the last
//! input happened, and a notification fires when the session has been idle for its
//! timeout *in ticks the caller advanced*. The loop advances one tick per elapsed
//! millisecond and a test advances ticks of its own, so the same code serves both and
//! neither depends on how long a machine takes to schedule a thread.
//!
//! ## Inhibition
//!
//! `zwp_idle_inhibit_manager_v1` is the other half: while any client holds an inhibitor
//! on a live surface, nothing is idle, and every notification that was idled is resumed
//! at once. It is implemented in [`crate::protocols::zwp_idle_inhibit_v1`]; the read side is
//! here, because the answer is "is this session idle" and there is one place that
//! decides it.
//!
//! # Failure modes
//!
//! 1. **An idle notification never fires.** A session with no input is idle from its
//!    start, so a notification whose timeout has passed in ticks is sent `idled` on the
//!    first advance and not again: `idle_notify_fires_after_its_timeout` in
//!    `tests/protocols.rs`.
//! 2. **An idle session never resumes.** Input records activity and every idled
//!    notification is sent `resumed` in the same turn, so a screen blanker cannot stay
//!    blank while the user types: `input_resumes_an_idled_notification`.
//! 3. **An inhibitor leaks past its client.** An inhibitor is a surface, and a surface
//!    that has been destroyed no longer inhibits; the check prunes dead surfaces rather
//!    than trusting a `destroy` request that a killed client never sends:
//!    `an_idle_inhibitor_suppresses_idle_and_never_leaks`.
//! 4. **A notification outlives its client.** The object's `destroyed` hook removes the
//!    record, and a record whose object is gone is skipped, so a dead client's
//!    notification cannot be sent to a recycled object id.

use std::sync::Arc;

use gnoblin_log::debug_in;
use smithay::reexports::wayland_protocols::ext::idle_notify::v1::server::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::{self, ExtIdleNotifierV1},
};
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::{DataInit, DisplayHandle, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_input::Activity;

use crate::State;
use crate::access::{Access, ClientRole};

/// One `ext_idle_notification_v1` a client created.
#[derive(Debug)]
struct ExtIdleNotifyV1Notification {
    /// The protocol object.
    resource: ExtIdleNotificationV1,
    /// How long the session must be idle before `idled`, in milliseconds.
    timeout: u64,
    /// Whether `idled` has been sent and not yet answered with `resumed`.
    idled: bool,
}

/// The most named idle holders the session keeps at once. A holder is one ScreenSaver
/// inhibit, so this bounds what one D-Bus caller can pile up.
pub(crate) const IDLE_HOLDER_CAP: usize = 256;

/// The idle notifications and inhibitors the session holds.
#[derive(Default, Debug)]
pub(crate) struct ExtIdleNotifyV1Tables {
    /// The notifications, in creation order.
    notifications: Vec<ExtIdleNotifyV1Notification>,
    /// The surfaces whose clients asked that the session never idle.
    inhibits: Vec<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    /// The named holders the session's own clients (the ScreenSaver bridge) asked to keep
    /// it awake. A holder is an opaque name; adding a held name again changes nothing.
    holders: std::collections::BTreeSet<String>,
    /// The idle state last published to the observation snapshot: idle seconds and the
    /// inhibitor count. The snapshot is republished when either moves, not on every turn.
    published: Option<(u64, usize)>,
}

impl ExtIdleNotifyV1Tables {
    /// How many notifications are held, for a test or a diagnostics reader.
    pub(crate) fn notifications(&self) -> usize {
        self.notifications.len()
    }
}

/// User data of an `ext_idle_notifier_v1` object.
///
/// It carries the access roster, which is the only thing that tells a sandboxed
/// connection from an ordinary one: the roster's entry is written when the connection is
/// accepted, so the filter never has to ask the backend about the client it is filtering.
#[derive(Debug)]
pub struct ExtIdleNotifyV1NotifierUserData {
    /// The session's roster of connections.
    access: Arc<Access>,
}

/// User data of an `ext_idle_notification_v1` object.
///
/// The record itself lives in [`State`]'s tables, keyed by the object: the user data only
/// has to exist so the object has an owner and the `destroyed` hook can find it.
#[derive(Debug)]
pub struct ExtIdleNotifyV1NotificationUserData;

impl GlobalDispatch2<ExtIdleNotifierV1, State> for ExtIdleNotifyV1NotifierUserData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &smithay::reexports::wayland_server::Client,
        resource: smithay::reexports::wayland_server::New<ExtIdleNotifierV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(
            resource,
            ExtIdleNotifyV1NotifierUserData {
                access: Arc::clone(&self.access),
            },
        );
    }

    fn can_view(&self, client: &smithay::reexports::wayland_server::Client) -> bool {
        // Idle state is session-wide information about the user, so a sandboxed client
        // is not told when the session is idle or in use. Idle notification is not a
        // privileged global — any application may ask — so the question is "is this
        // connection a sandbox", not "did it earn the global".
        self.access.role(&client.id()) != ClientRole::Sandboxed
    }
}

impl Dispatch2<ExtIdleNotifierV1, State> for ExtIdleNotifyV1NotifierUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &smithay::reexports::wayland_server::Client,
        _resource: &ExtIdleNotifierV1,
        request: ext_idle_notifier_v1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_idle_notifier_v1::Request::GetIdleNotification {
                id,
                seat: _seat,
                timeout,
            } => {
                let notification: ExtIdleNotificationV1 = data_init.init(id, ExtIdleNotifyV1NotificationUserData);
                state.add_idle_notification(notification, u64::from(timeout));
            }
            ext_idle_notifier_v1::Request::Destroy => {}
            _ => unreachable!("ext_idle_notifier_v1 has two requests"),
        }
    }
}

impl Dispatch2<ExtIdleNotificationV1, State> for ExtIdleNotifyV1NotificationUserData {
    fn request(
        &self,
        _state: &mut State,
        _client: &smithay::reexports::wayland_server::Client,
        _resource: &ExtIdleNotificationV1,
        request: ext_idle_notification_v1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_idle_notification_v1::Request::Destroy => {}
            _ => unreachable!("ext_idle_notification_v1 has one request"),
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ExtIdleNotificationV1,
    ) {
        debug_in!("wayland", ?client, instance = ?resource.id(), "[idle] a notification is gone");
        state.forget_idle_notification(resource);
    }
}

/// Creates the `ext_idle_notifier_v1` global.
pub(crate) fn global(display: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display.create_global::<State, ExtIdleNotifierV1, _>(
        2,
        ExtIdleNotifyV1NotifierUserData {
            access: Arc::clone(access),
        },
    )
}

impl State {
    /// Records a notification a client created and answers it at once if the session is
    /// already idle.
    pub(crate) fn add_idle_notification(&mut self, resource: ExtIdleNotificationV1, timeout: u64) {
        debug_in!("wayland", instance = ?resource.id(), timeout, "[idle] a client asked to be told");
        self.idle_tables.notifications.push(ExtIdleNotifyV1Notification {
            resource,
            timeout,
            idled: false,
        });
        self.refresh_idle();
    }

    /// Forgets a notification whose object is gone.
    pub(crate) fn forget_idle_notification(&mut self, resource: &ExtIdleNotificationV1) {
        self.idle_tables
            .notifications
            .retain(|notification| notification.resource != *resource);
    }

    /// Advances the session clock, in milliseconds, and answers every notification the
    /// new time changes.
    pub(crate) fn advance_clock(&mut self, milliseconds: u64) {
        self.clock = self.clock.saturating_add(milliseconds);
        self.refresh_idle();
    }

    /// Records input activity, which is what ends an idle period.
    pub(crate) fn record_activity(&mut self) {
        self.activity.record(self.clock);
        self.refresh_idle();
    }

    /// Whether any client's inhibitor currently suppresses idle.
    ///
    /// An inhibitor whose surface has gone is pruned here rather than trusted: a client
    /// that is killed sends no `destroy`, and an inhibitor that outlives its client
    /// would keep the session awake for ever.
    #[must_use]
    pub(crate) fn idle_inhibited(&mut self) -> bool {
        self.idle_inhibitors() > 0
    }

    /// How many inhibitors currently hold idle off: inhibiting surfaces and named holders.
    #[must_use]
    pub(crate) fn idle_inhibitors(&mut self) -> usize {
        self.idle_tables.inhibits.retain(|surface| surface.is_alive());
        self.idle_tables.inhibits.len() + self.idle_tables.holders.len()
    }

    /// Adds (`held` true) or removes one named idle holder. Adding a name already held and
    /// removing one not held change nothing. `Ok` carries whether the set changed; `Err`
    /// refuses a new name once [`IDLE_HOLDER_CAP`] names are held.
    pub(crate) fn idle_hold(&mut self, holder: String, held: bool) -> Result<bool, String> {
        let changed = if held {
            if !self.idle_tables.holders.contains(&holder) && self.idle_tables.holders.len() >= IDLE_HOLDER_CAP {
                return Err(format!("at most {IDLE_HOLDER_CAP} idle inhibitors may be held at once"));
            }
            self.idle_tables.holders.insert(holder)
        } else {
            self.idle_tables.holders.remove(&holder)
        };
        if changed {
            self.refresh_idle();
        }
        Ok(changed)
    }

    /// Records or drops one inhibitor.
    pub(crate) fn set_idle_inhibited(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        inhibited: bool,
    ) {
        self.idle_tables.inhibits.retain(|held| held != surface);
        if inhibited {
            self.idle_tables.inhibits.push(surface.clone());
        }
        self.refresh_idle();
    }

    /// Sends `idled` and `resumed` for every notification the current idle state
    /// changes.
    pub(crate) fn refresh_idle(&mut self) {
        let inhibited = self.idle_inhibited();
        let now = self.clock;
        let activity: Activity = self.activity;
        let mut dead: Vec<ExtIdleNotificationV1> = Vec::new();
        let mut changes: Vec<(ExtIdleNotificationV1, bool)> = Vec::new();
        for notification in &mut self.idle_tables.notifications {
            if !notification.resource.is_alive() {
                dead.push(notification.resource.clone());
                continue;
            }
            let idle = !inhibited && activity.is_idle(now, notification.timeout);
            if idle != notification.idled {
                notification.idled = idle;
                changes.push((notification.resource.clone(), idle));
            }
        }
        for resource in dead {
            self.idle_tables
                .notifications
                .retain(|notification| notification.resource != resource);
        }
        for (resource, idle) in changes {
            if idle {
                debug_in!("wayland", instance = ?resource.id(), "[idle] the session is idle");
                resource.idled();
            } else {
                debug_in!("wayland", instance = ?resource.id(), "[idle] the session resumed");
                resource.resumed();
            }
        }
        // The observation snapshot reports idle time and the inhibitor count, so it is
        // republished when either changes. Idle time moves every tick, so it is keyed to
        // whole seconds: a reader sees the count move each second, not every turn.
        let key = (self.activity.idle_ticks(self.clock) / 1000, self.idle_inhibitors());
        if self.idle_tables.published != Some(key) {
            self.idle_tables.published = Some(key);
            self.publish();
        }
    }

    /// How many idle notifications the session holds.
    #[must_use]
    pub fn idle_notifications(&self) -> usize {
        self.idle_tables.notifications()
    }
}
