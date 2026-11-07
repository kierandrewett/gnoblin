//! The IPC client half: one subscription, for the one thing this adapter needs from it.
//!
//! The screenshot adapter serves no reads, because the IPC publishes no image capture: what it
//! needs from the compositor is the lock state, and only so the gate's embargo is the
//! compositor's rather than the adapter's own guess. That is why this module is a
//! [`gnoblin_client::Client`] over the `lock` object and nothing else.
//!
//! # Failure modes first
//!
//! 1. **The graph publishes no lock object.** Refused at connect, because a released embargo
//!    inferred from an absent object is the one guess this adapter must not make:
//!    [`ScreenshotLink::connect`].
//! 2. **The link ends.** [`ScreenshotLink::advance`] reports `Ok(false)`, so a host stops instead
//!    of folding the same state forever.

use std::collections::BTreeMap;

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_compat_core::LockState;
use gnoblin_model::{Filter, ObjectId, ObjectView, Observation, TypeKey, Value};

use crate::error::ScreenshotError;

/// The lock object's type.
pub const LOCK: &str = "lock";

/// One subscription to the compositor, and the lock state it has delivered.
#[derive(Debug)]
pub struct ScreenshotLink<C: Transport> {
    client: Client<C>,
    views: BTreeMap<ObjectId, ObjectView>,
    lock: LockState,
}

impl<C: Transport> ScreenshotLink<C> {
    /// Subscribe and read the first snapshot.
    ///
    /// # Errors
    ///
    /// [`ScreenshotError::Link`] when the subscription fails and [`ScreenshotError::NoLock`]
    /// when the graph publishes no lock object.
    pub fn connect(client: Client<C>) -> Result<ScreenshotLink<C>, ScreenshotError> {
        let mut client = client;
        let snapshot = client.subscribe(Filter::All)?;
        let views: BTreeMap<ObjectId, ObjectView> = snapshot
            .objects()
            .iter()
            .map(|view| (view.id(), view.clone()))
            .collect();
        let lock = read_lock(views.values())?;
        Ok(ScreenshotLink { client, views, lock })
    }

    /// The lock state as of the last folded observation.
    pub const fn lock(&self) -> LockState {
        self.lock
    }

    /// Fold one observation.
    ///
    /// `Ok(true)` means one observation was folded and [`ScreenshotLink::lock`] may have
    /// changed; `Ok(false)` means the connection ended.
    ///
    /// # Errors
    ///
    /// [`ScreenshotError::Link`] for a transport failure, and the lock's own refusal when the
    /// observation left the graph unreadable.
    pub fn advance(&mut self) -> Result<bool, ScreenshotError> {
        let event = match self.client.next_event() {
            Ok(event) => event,
            Err(ClientError::Disconnected) => return Ok(false),
            Err(error) => return Err(ScreenshotError::from(error)),
        };
        let Event::Model(observation) = event else {
            return Ok(true);
        };
        self.fold(observation)?;
        Ok(true)
    }

    /// Fold every observation the link has already delivered.
    ///
    /// # Errors
    ///
    /// The same refusals as [`ScreenshotLink::advance`], except that an ended link is not one of
    /// them.
    pub fn drain(&mut self) -> Result<usize, ScreenshotError> {
        let mut folded = 0;
        while self.advance()? {
            folded += 1;
        }
        Ok(folded)
    }

    fn fold(&mut self, observation: Observation) -> Result<(), ScreenshotError> {
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
        self.lock = read_lock(self.views.values())?;
        Ok(())
    }
}

/// The lock state the graph publishes, as the gate's embargo value.
fn read_lock<'a>(views: impl IntoIterator<Item = &'a ObjectView>) -> Result<LockState, ScreenshotError> {
    for view in views {
        if view.ty() == &TypeKey::gnoblin(LOCK) {
            return match view.property("locked") {
                Some(Value::Bool(true)) => Ok(LockState::Locked),
                Some(Value::Bool(false)) => Ok(LockState::Unlocked),
                Some(other) => Err(ScreenshotError::Unreadable {
                    object: "lock".to_owned(),
                    property: "locked".to_owned(),
                    reason: format!("expected a bool, found {}", other.kind()),
                }),
                None => Err(ScreenshotError::Unreadable {
                    object: "lock".to_owned(),
                    property: "locked".to_owned(),
                    reason: "the property is absent".to_owned(),
                }),
            };
        }
    }
    Err(ScreenshotError::NoLock)
}
