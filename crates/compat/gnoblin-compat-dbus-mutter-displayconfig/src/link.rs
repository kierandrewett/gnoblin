//! The IPC client half: one subscription, folded into the state the adapter reports.
//!
//! A compat adapter runs in the services role and reaches the compositor the way every
//! other client does (design/rust-compositor.md, "Compatibility adapters" rule 4), so
//! this module is a [`gnoblin_client::Client`] and nothing else. It subscribes once and
//! then folds every observation into a mirror of the views it was given, because a
//! subscription delivers diffs and the state a caller asks for is the whole thing.
//!
//! The fold is a re-read of the mirror after each observation rather than an
//! incremental update: there are a handful of outputs, and one code path that turns
//! views into [`DisplayState`] is what keeps a `Changed` diff and the first snapshot
//! from disagreeing about how a monitor is built.
//!
//! # Failure modes first
//!
//! 1. **The link ends.** [`DisplayLink::advance`] reports `Ok(false)`, so a host stops
//!    instead of folding the same state forever: the loop is a caller's, and it has to
//!    be able to end.
//! 2. **The graph publishes no `lock` object.** Refused at connect, because the gate's
//!    embargo would otherwise be built from a guess: [`DisplayLink::connect`].
//! 3. **A property the schema promises is missing or mistyped.** Refused with the
//!    object and property named, rather than a monitor with a zero scale:
//!    [`crate::state::DisplayState::read`].

use std::collections::BTreeMap;
use std::sync::Mutex;

use gnoblin_client::{Client, ClientError, Transport};
use gnoblin_compat_core::LockState;
use gnoblin_model::{Fields, Filter, ObjectId, ObjectView, Observation};

use crate::error::DisplayError;
use crate::service::OutputApplier;
use crate::state::{DisplayState, read_lock};

/// One subscription to the compositor, and the state it has delivered so far.
#[derive(Debug)]
pub struct DisplayLink<C: Transport> {
    client: Client<C>,
    views: BTreeMap<ObjectId, ObjectView>,
    state: DisplayState,
    lock: LockState,
}

impl<C: Transport> DisplayLink<C> {
    /// Subscribe and read the first snapshot.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Link`] when the subscription fails, and the state's own
    /// refusals when the snapshot is not a state this adapter can report.
    pub fn connect(client: Client<C>) -> Result<DisplayLink<C>, DisplayError> {
        let mut client = client;
        let snapshot = client.subscribe(Filter::All)?;
        let views: BTreeMap<ObjectId, ObjectView> = snapshot
            .objects()
            .iter()
            .map(|view| (view.id(), view.clone()))
            .collect();
        let state = DisplayState::read(views.values())?;
        let lock = read_lock(views.values())?;
        Ok(DisplayLink {
            client,
            views,
            state,
            lock,
        })
    }

    /// The state as of the last folded observation.
    pub fn state(&self) -> &DisplayState {
        &self.state
    }

    /// The lock state as of the last folded observation.
    pub fn lock(&self) -> LockState {
        self.lock
    }

    /// Fold one observation.
    ///
    /// `Ok(true)` means one observation was folded and [`DisplayLink::state`] may have
    /// changed; `Ok(false)` means the connection ended. A socket blocks here until
    /// there is something to fold, which is what makes this the loop a host runs.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Link`] for a transport failure, and the state's refusals when the
    /// observation left the graph in a state this adapter cannot report.
    pub fn advance(&mut self) -> Result<bool, DisplayError> {
        let event = match self.client.next_event() {
            Ok(event) => event,
            Err(ClientError::Disconnected) => return Ok(false),
            Err(error) => return Err(DisplayError::from(error)),
        };
        let gnoblin_client::Event::Model(observation) = event else {
            // Requests are the provider's business and no compat adapter offers one, so
            // an offer or a completion is not a state change.
            return Ok(true);
        };
        self.fold(observation)?;
        Ok(true)
    }

    /// Fold every observation the link has already delivered.
    ///
    /// This is the stepwise driver a test or a non-blocking host uses: the in-process
    /// link reports an empty queue as a disconnect, so `advance` cannot be called to
    /// discover that there is nothing left. On a socket this is the same as `next`
    /// because a read blocks.
    ///
    /// # Errors
    ///
    /// The same refusals as [`DisplayLink::advance`], except that an ended link is not one
    /// of them: it ends the loop and returns how many observations were folded.
    pub fn drain(&mut self) -> Result<usize, DisplayError> {
        let mut folded = 0;
        while self.advance()? {
            folded += 1;
        }
        Ok(folded)
    }

    /// The client, for a host that also writes through the same connection.
    pub fn client(&mut self) -> &mut Client<C> {
        &mut self.client
    }

    /// Apply one output's configuration through the object's own method.
    ///
    /// The reply is the operation's outcome, so a refusal from the compositor reaches
    /// the D-Bus caller as a failed apply rather than as a success that changed nothing.
    ///
    /// # Errors
    ///
    /// [`DisplayError::ApplyFailed`] when the compositor refuses or the operation never
    /// completes, and [`DisplayError::Link`] when the transport fails.
    pub fn apply(
        &mut self,
        output: ObjectId,
        connector: &str,
        fields: &gnoblin_model::Fields,
    ) -> Result<(), DisplayError> {
        let operation = self.client.call(output, "apply", fields)?;
        let outcome = self.client.await_operation(operation)?;
        match outcome {
            gnoblin_model::OperationOutcome::Done => Ok(()),
            gnoblin_model::OperationOutcome::Value(_) => Err(DisplayError::ApplyFailed {
                connector: connector.to_owned(),
                reason: "the compositor answered apply with a value".to_owned(),
            }),
            gnoblin_model::OperationOutcome::Failed(code) => Err(DisplayError::ApplyFailed {
                connector: connector.to_owned(),
                reason: code.to_string(),
            }),
        }
    }

    /// The object identity of the output a connector names.
    ///
    /// The plan addresses outputs by connector because that is what a caller sends, so
    /// the write path needs the identity behind the name.
    pub fn output_object(&self, connector: &str) -> Option<ObjectId> {
        self.views.iter().find_map(|(id, view)| {
            let is_output = view.ty() == &gnoblin_model::TypeKey::gnoblin(crate::state::OUTPUT);
            let named = matches!(view.property("name"), Some(gnoblin_model::Value::Text(name)) if name == connector);
            (is_output && named).then_some(*id)
        })
    }

    fn fold(&mut self, observation: Observation) -> Result<(), DisplayError> {
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
            // A signal carries nothing this adapter reports, and a completion is the
            // write path's own reply.
            Observation::Signalled { .. } | Observation::Finished { .. } => return Ok(()),
        }
        self.state = DisplayState::read(self.views.values())?;
        self.lock = read_lock(self.views.values())?;
        Ok(())
    }
}

/// The write half a host threads onto the IPC.
///
/// `Mutex<DisplayLink<C>>` is the applier a running adapter uses: the link already owns
/// the connection and the object identities, so a configuration is applied on the same
/// subscription that reads the result. It is `Send + Sync` whenever the transport is,
/// which is what a zbus interface object needs and what the in-process link the tests
/// use is not.
impl<C: Transport + Send> OutputApplier for Mutex<DisplayLink<C>> {
    fn apply(&self, output: &str, fields: &Fields) -> Result<(), String> {
        let mut link = self.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let object = link
            .output_object(output)
            .ok_or_else(|| format!("no `output` object is called `{output}`"))?;
        link.apply(object, output, fields).map_err(|error| error.to_string())
    }
}
