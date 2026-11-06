//! The IPC client half: one subscription, folded into the idle time and the lock state.
//!
//! The counter is the compositor's, published as the `idle` object's `idle_ms` property
//! (design/rust-compositor.md, the redirect map's `IdleMonitor` row), so this module is a
//! [`gnoblin_client::Client`] and nothing else. The lock state rides along because the
//! gate needs it: idle watches answer while the session is locked, which is exactly what a
//! screensaver needs, but the service still has to be able to say which embargo it built
//! its decision from.
//!
//! # Failure modes first
//!
//! 1. **The graph publishes no idle counter.** Refused at connect, because a zero would be
//!    indistinguishable from "the user is active" and every watch would silently never
//!    fire: [`IdleLink::connect`].
//! 2. **The link ends.** [`IdleLink::advance`] reports `Ok(false)`, so a host stops rather
//!    than folding the same counter forever.
//! 3. **A property is missing or mistyped.** Refused with the property named, rather than
//!    an idle time of zero: [`IdleState::read`].

use std::collections::BTreeMap;

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_compat_core::LockState;
use gnoblin_model::{Filter, ObjectId, ObjectView, Observation, TypeKey, Value};

use crate::error::IdleError;

/// The object type that publishes the counter.
pub const IDLE: &str = "idle";

/// The lock object's type.
pub const LOCK: &str = "lock";

/// What the adapter reports: how long the session has been idle, and whether it is locked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct IdleState {
    /// Milliseconds since the last input event.
    pub idle_ms: u64,
    /// The gate's embargo value.
    pub lock: LockState,
}

impl IdleState {
    /// Read the state from every object a subscription delivered.
    ///
    /// # Errors
    ///
    /// [`IdleError::NoCounter`] when the graph publishes no `idle` object, and
    /// [`IdleError::Unreadable`] when a property is absent or mistyped.
    pub fn read<'a>(views: impl IntoIterator<Item = &'a ObjectView>) -> Result<IdleState, IdleError> {
        let mut idle = None;
        let mut locked = None;
        for view in views {
            if view.ty() == &TypeKey::gnoblin(IDLE) {
                match view.property("idle_ms") {
                    Some(Value::Uint(value)) => idle = Some(*value),
                    Some(other) => {
                        return Err(IdleError::Unreadable {
                            object: "idle".to_owned(),
                            property: "idle_ms".to_owned(),
                            reason: format!("expected a uint, found {}", other.kind()),
                        });
                    }
                    None => {
                        return Err(IdleError::Unreadable {
                            object: "idle".to_owned(),
                            property: "idle_ms".to_owned(),
                            reason: "the property is absent".to_owned(),
                        });
                    }
                }
            }
            if view.ty() == &TypeKey::gnoblin(LOCK) {
                match view.property("locked") {
                    Some(Value::Bool(value)) => locked = Some(*value),
                    Some(other) => {
                        return Err(IdleError::Unreadable {
                            object: "lock".to_owned(),
                            property: "locked".to_owned(),
                            reason: format!("expected a bool, found {}", other.kind()),
                        });
                    }
                    None => {
                        return Err(IdleError::Unreadable {
                            object: "lock".to_owned(),
                            property: "locked".to_owned(),
                            reason: "the property is absent".to_owned(),
                        });
                    }
                }
            }
        }
        let idle_ms = idle.ok_or(IdleError::NoCounter)?;
        let lock = match locked {
            Some(true) => LockState::Locked,
            Some(false) => LockState::Unlocked,
            None => {
                return Err(IdleError::Unreadable {
                    object: "lock".to_owned(),
                    property: "locked".to_owned(),
                    reason: "the graph publishes no `lock` object".to_owned(),
                });
            }
        };
        Ok(IdleState { idle_ms, lock })
    }
}

/// One subscription to the compositor, and the state it has delivered so far.
#[derive(Debug)]
pub struct IdleLink<C: Transport> {
    client: Client<C>,
    views: BTreeMap<ObjectId, ObjectView>,
    state: IdleState,
}

impl<C: Transport> IdleLink<C> {
    /// Subscribe and read the first snapshot.
    ///
    /// # Errors
    ///
    /// [`IdleError::Link`] when the subscription fails, and the state's own refusals when
    /// the snapshot publishes no counter.
    pub fn connect(client: Client<C>) -> Result<IdleLink<C>, IdleError> {
        let mut client = client;
        let snapshot = client.subscribe(Filter::All)?;
        let views: BTreeMap<ObjectId, ObjectView> = snapshot
            .objects()
            .iter()
            .map(|view| (view.id(), view.clone()))
            .collect();
        let state = IdleState::read(views.values())?;
        Ok(IdleLink { client, views, state })
    }

    /// The state as of the last folded observation.
    pub const fn state(&self) -> IdleState {
        self.state
    }

    /// Fold one observation.
    ///
    /// `Ok(true)` means one observation was folded and [`IdleLink::state`] may have
    /// changed; `Ok(false)` means the connection ended.
    ///
    /// # Errors
    ///
    /// [`IdleError::Link`] for a transport failure, and the state's refusals when the
    /// observation left the graph unreadable.
    pub fn advance(&mut self) -> Result<bool, IdleError> {
        let event = match self.client.next_event() {
            Ok(event) => event,
            Err(ClientError::Disconnected) => return Ok(false),
            Err(error) => return Err(IdleError::from(error)),
        };
        let Event::Model(observation) = event else {
            // No compat adapter offers a request, so an offer is not a state change.
            return Ok(true);
        };
        self.fold(observation)?;
        Ok(true)
    }

    /// Fold every observation the link has already delivered.
    ///
    /// The stepwise driver a test or a non-blocking host uses: the in-process link reports
    /// an empty queue as a disconnect, so `advance` cannot be called to discover that
    /// there is nothing left.
    ///
    /// # Errors
    ///
    /// The same refusals as [`IdleLink::advance`], except that an ended link is not one of
    /// them.
    pub fn drain(&mut self) -> Result<usize, IdleError> {
        let mut folded = 0;
        while self.advance()? {
            folded += 1;
        }
        Ok(folded)
    }

    fn fold(&mut self, observation: Observation) -> Result<(), IdleError> {
        match observation {
            Observation::Opened { view, .. } => {
                self.views.insert(view.id(), view);
            }
            Observation::Changed {
                object,
                property,
                value,
                ..
            } => {
                if let Some(view) = self.views.get(&object) {
                    let mut properties = view.properties().clone();
                    properties.insert(property, value);
                    let updated = ObjectView::new(object, view.ty().clone(), properties);
                    self.views.insert(object, updated);
                }
            }
            Observation::Closed { object, .. } => {
                self.views.remove(&object);
            }
            Observation::Signalled { .. } | Observation::Finished { .. } => return Ok(()),
        }
        self.state = IdleState::read(self.views.values())?;
        Ok(())
    }
}
