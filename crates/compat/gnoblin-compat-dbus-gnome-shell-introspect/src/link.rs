//! The IPC client half: one subscription, folded into the window list and the screen size.
//!
//! The window list is the compositor's (`design/rust-compositor.md`, the redirect map's
//! `org.gnome.Shell.Introspect` row), so this module is a [`gnoblin_client::Client`] and
//! nothing else: it subscribes once and folds every observation into a mirror of the views,
//! because a subscription delivers diffs and `GetWindows` answers with the whole list.
//!
//! # Failure modes first
//!
//! 1. **The link ends.** [`IntrospectLink::advance`] reports `Ok(false)`, so a host stops
//!    rather than folding the same state forever.
//! 2. **The graph publishes no lock object.** Refused at connect, because the gate's embargo
//!    would otherwise be built from a guess: [`IntrospectLink::connect`].
//! 3. **A property is missing or mistyped.** Refused with the object and property named:
//!    [`crate::state::IntrospectState::read`].

use std::collections::BTreeMap;

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_model::{Filter, ObjectId, ObjectView, Observation};

use crate::error::IntrospectError;
use crate::state::IntrospectState;

/// One subscription to the compositor, and the state it has delivered so far.
#[derive(Debug)]
pub struct IntrospectLink<C: Transport> {
    client: Client<C>,
    views: BTreeMap<ObjectId, ObjectView>,
    state: IntrospectState,
}

impl<C: Transport> IntrospectLink<C> {
    /// Subscribe and read the first snapshot.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::Link`] when the subscription fails, and the state's own refusals.
    pub fn connect(client: Client<C>) -> Result<IntrospectLink<C>, IntrospectError> {
        let mut client = client;
        let snapshot = client.subscribe(Filter::All)?;
        let views: BTreeMap<ObjectId, ObjectView> = snapshot
            .objects()
            .iter()
            .map(|view| (view.id(), view.clone()))
            .collect();
        let state = IntrospectState::read(views.values())?;
        Ok(IntrospectLink { client, views, state })
    }

    /// The state as of the last folded observation.
    pub fn state(&self) -> &IntrospectState {
        &self.state
    }

    /// Fold one observation.
    ///
    /// `Ok(true)` means one observation was folded and [`IntrospectLink::state`] may have
    /// changed; `Ok(false)` means the connection ended.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::Link`] for a transport failure, and the state's refusals when the
    /// observation left the graph unreadable.
    pub fn advance(&mut self) -> Result<bool, IntrospectError> {
        let event = match self.client.next_event() {
            Ok(event) => event,
            Err(ClientError::Disconnected) => return Ok(false),
            Err(error) => return Err(IntrospectError::from(error)),
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
    /// The stepwise driver a test or a non-blocking host uses: the in-process link reports an
    /// empty queue as a disconnect, so `advance` cannot be called to discover that there is
    /// nothing left.
    ///
    /// # Errors
    ///
    /// The same refusals as [`IntrospectLink::advance`], except that an ended link is not one
    /// of them.
    pub fn drain(&mut self) -> Result<usize, IntrospectError> {
        let mut folded = 0;
        while self.advance()? {
            folded += 1;
        }
        Ok(folded)
    }

    fn fold(&mut self, observation: Observation) -> Result<(), IntrospectError> {
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
        self.state = IntrospectState::read(self.views.values())?;
        Ok(())
    }
}
