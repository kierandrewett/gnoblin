//! What authority asks the compositor or the portal to do.

use gnoblin_types::{ClientId, Tick};

use crate::denial::{Decision, Denial};
use crate::event::{ContextRequest, GestureKind, PortalCapability};
use crate::ids::{AppId, GestureId, RequestId, RequesterId, SessionLeader, SurfaceId, TokenId};

/// One instruction, in the order it must happen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Record a launch's token and hand it to the launched process.
    IssueToken {
        /// The token the compositor minted.
        token: TokenId,
        /// The launch it belongs to.
        leader: SessionLeader,
        /// The label the launcher expected, for the runtime's own bookkeeping.
        app_id: Option<AppId>,
        /// When it stops being redeemable.
        expires_at: Tick,
    },
    /// The launch could not be recorded, so no token exists for it.
    RefuseLaunch {
        /// The token that was offered.
        token: TokenId,
        /// Why.
        reason: Denial,
    },
    /// Focus this client's surface.
    Activate {
        /// The client to focus.
        client: ClientId,
        /// The surface to focus.
        surface: SurfaceId,
    },
    /// The activation claim was refused.
    RefuseActivation {
        /// The claimant.
        client: ClientId,
        /// The surface it asked for, so an error can name it.
        surface: SurfaceId,
        /// Why.
        reason: Denial,
    },
    /// A trusted input context now exists for this gesture.
    Context {
        /// The gesture that created it.
        gesture: GestureId,
        /// The connection it belongs to.
        client: ClientId,
        /// The surface it belongs to.
        surface: SurfaceId,
        /// The kind of gesture, for the runtime's `FocusContext`.
        kind: GestureKind,
        /// When it stops being usable.
        expires_at: Tick,
    },
    /// The gesture is over, so its context is gone.
    EndContext {
        /// The context to drop.
        gesture: GestureId,
    },
    /// A context request was authorised.
    Authorise {
        /// The gesture that authorises it.
        gesture: GestureId,
        /// The client it belongs to.
        client: ClientId,
        /// The surface the request named.
        surface: SurfaceId,
        /// What was authorised.
        request: ContextRequest,
    },
    /// A context request was refused.
    RefuseContext {
        /// The gesture that was claimed.
        gesture: GestureId,
        /// The client that asked.
        client: ClientId,
        /// What it asked for.
        request: ContextRequest,
        /// Why.
        reason: Denial,
    },
    /// Ask the `portal.ui` provider to show a consent prompt.
    Prompt {
        /// The request being asked about.
        request: RequestId,
        /// The capability it asks for, and the caller it is for, so the UI can
        /// say who is asking.
        capability: PortalCapability,
        /// Who is asking.
        requester: RequesterId,
        /// The provider that must answer.
        provider: ClientId,
    },
    /// Answer the portal's request.
    Answer {
        /// The request being answered.
        request: RequestId,
        /// What the caller may do.
        decision: Decision,
    },
    /// An answer from something other than the provider, or for a prompt that
    /// no longer exists. The prompt state is unchanged.
    Ignored {
        /// The request that was answered.
        request: RequestId,
        /// Who answered it.
        from: ClientId,
    },
    /// Withdraw a prompt that can no longer be answered.
    Dismiss {
        /// The request to withdraw.
        request: RequestId,
    },
    /// Every grant for this caller is gone; the compositor ends its streams.
    EndGrants {
        /// The caller.
        requester: RequesterId,
    },
    /// Capture is allowed or must stop entirely.
    Capture {
        /// Whether frames may flow.
        allowed: bool,
    },
    /// The `portal.ui` slot changed owner.
    PortalUi {
        /// The provider now, or none.
        provider: Option<ClientId>,
    },
    /// A capability was exercised, or refused at the gate.
    Gate {
        /// The caller.
        requester: RequesterId,
        /// The capability.
        capability: PortalCapability,
        /// The answer.
        decision: Decision,
    },
}
