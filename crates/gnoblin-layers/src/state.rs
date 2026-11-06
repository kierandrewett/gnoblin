//! A layer's lifecycle state, and why a layer ended.
//!
//! A layer is `Mapped` (on screen), `Hidden` (registered but not drawn, for
//! example while its window is minimised) or `Ended` (gone for good, with a
//! reason). Transitions are restricted so a layer cannot come back after it ends.
//! The transition rules are in [`LayerState::transition`] (added with the follow
//! rules).
//!
//! # Transition table
//!
//! | From      | To        | Allowed |
//! |-----------|-----------|---------|
//! | `Mapped`  | `Hidden`  | yes     |
//! | `Hidden`  | `Mapped`  | yes     |
//! | `Mapped`  | `Ended`   | yes     |
//! | `Hidden`  | `Ended`   | yes     |
//! | `Ended`   | anything  | no, refused as `EndedIsTerminal` |
//! | same      | same      | no, refused as `NoChange` |
//!
//! `Ended` is terminal because a layer's name is released when it ends. A layer
//! that comes back is a new registration with a new handle, not the old one
//! revived, so a stale handle can never point at a different layer.

/// Why a layer ended. Each reason is a different cause, so logs and `layers.list`
/// can say what happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// The provider removed the layer with `LayerRegistry::remove`.
    Removed,
    /// The provider disconnected, or an extension was unloaded.
    ProviderGone,
    /// The window the layer follows closed.
    TargetGone,
    /// The output the layer is attached to was removed.
    OutputGone,
}

/// The state of one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerState {
    /// On screen, and part of the stack.
    Mapped,
    /// Registered, but not drawn. It can return to `Mapped`.
    Hidden,
    /// Gone for good, with the reason. This is terminal.
    Ended(EndReason),
}

/// Why a state change was refused. Callers match on the variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransitionError {
    /// The layer has ended. `Ended` is terminal, so nothing leaves it.
    EndedIsTerminal,
    /// The layer is already in the requested state, so there is nothing to change.
    NoChange,
}

impl LayerState {
    /// Returns `to` when the move is allowed, following the table in the module
    /// docs. An `Ended` layer never changes, and a same-state move is refused so a
    /// caller that repeats a change notices it.
    ///
    /// # Errors
    ///
    /// [`TransitionError::EndedIsTerminal`] when `self` is `Ended`, and
    /// [`TransitionError::NoChange`] when `to` equals `self`.
    pub fn transition(self, to: LayerState) -> Result<LayerState, TransitionError> {
        match (self, to) {
            (LayerState::Ended(_), _) => Err(TransitionError::EndedIsTerminal),
            (from, to) if from == to => Err(TransitionError::NoChange),
            (_, to) => Ok(to),
        }
    }
}
