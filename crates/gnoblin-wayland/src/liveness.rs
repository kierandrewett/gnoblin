//! Detects xdg-shell clients that have stopped answering pings.
//!
//! The compositor pings each client that owns a mapped toplevel every half of the
//! check-alive timeout. A client that has left a ping unanswered for the whole
//! timeout is unresponsive, and every window it owns reports that until it pongs.
//! A timeout of zero turns the pings off, so no client is ever marked.

use std::time::Duration;

use gnoblin_types::Tick;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::xdg::{ShellClient, ToplevelSurface};

/// One client the pings are tracked for.
struct Tracked {
    id: ClientId,
    /// When the outstanding ping was sent, if one is outstanding.
    pending_since: Option<Tick>,
    /// When the last ping was sent, which paces the next one.
    last_ping: Option<Tick>,
    /// Whether the client has missed its timeout.
    unresponsive: bool,
}

/// The liveness state for every xdg-shell client.
pub(crate) struct Liveness {
    /// The check-alive timeout in milliseconds; zero disables the pings.
    timeout_ms: u64,
    tracked: Vec<Tracked>,
}

impl Liveness {
    /// Creates the tracker for `timeout`. A zero timeout disables the pings.
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            tracked: Vec::new(),
        }
    }

    /// Whether the client `id` has missed its timeout.
    pub(crate) fn is_unresponsive(&self, id: &ClientId) -> bool {
        self.tracked
            .iter()
            .any(|tracked| tracked.unresponsive && tracked.id == *id)
    }

    /// Advances the pings to `now` for the clients that own mapped toplevels.
    ///
    /// Returns whether any client's unresponsive state changed, so the caller knows
    /// to publish the window state.
    pub(crate) fn poll(&mut self, now: Tick, clients: &[(ClientId, ShellClient)]) -> bool {
        if self.timeout_ms == 0 {
            let changed = self.tracked.iter().any(|tracked| tracked.unresponsive);
            self.tracked.clear();
            return changed;
        }
        self.tracked
            .retain(|tracked| clients.iter().any(|(id, _)| *id == tracked.id));
        for (id, _) in clients {
            if self.tracked.iter().all(|tracked| tracked.id != *id) {
                self.tracked.push(Tracked {
                    id: id.clone(),
                    pending_since: None,
                    last_ping: None,
                    unresponsive: false,
                });
            }
        }
        let mut changed = false;
        let half = self.timeout_ms / 2;
        for (id, client) in clients {
            let Some(tracked) = self.tracked.iter_mut().find(|tracked| tracked.id == *id) else {
                continue;
            };
            if tracked.pending_since.is_none()
                && tracked.last_ping.is_none_or(|sent| now.since(sent) >= half)
                && client.send_ping(SERIAL_COUNTER.next_serial()).is_ok()
            {
                tracked.pending_since = Some(now);
                tracked.last_ping = Some(now);
            }
            if let Some(sent) = tracked.pending_since
                && !tracked.unresponsive
                && now.since(sent) >= self.timeout_ms
            {
                tracked.unresponsive = true;
                changed = true;
            }
        }
        changed
    }

    /// Records a pong from the client `id`, which clears its outstanding ping.
    ///
    /// Returns whether the client had been unresponsive, so the caller knows to
    /// publish the window state.
    pub(crate) fn pong(&mut self, id: &ClientId) -> bool {
        let Some(tracked) = self.tracked.iter_mut().find(|tracked| tracked.id == *id) else {
            return false;
        };
        tracked.pending_since = None;
        std::mem::replace(&mut tracked.unresponsive, false)
    }
}

/// The wayland client that owns `surface`, which keys its liveness entry.
pub(crate) fn owner(surface: &ToplevelSurface) -> Option<ClientId> {
    surface.wl_surface().client().map(|client| client.id())
}
