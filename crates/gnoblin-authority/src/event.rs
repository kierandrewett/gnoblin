//! What the compositor, the runtime and the portal ask authority.

use gnoblin_types::ClientId;

use crate::ids::{AppId, GestureId, RequestId, RequesterId, SessionLeader, SurfaceId, TokenId};

/// Everything that can grant, refuse or expire authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// A launcher started a process and the compositor minted its token.
    ///
    /// The launcher is the trusted party: it is the compositor's own action, a
    /// shell's request or a session restart. This is the only place a token
    /// enters the machine.
    Launch {
        /// What the launcher started, and the label the process will claim.
        origin: LaunchOrigin,
        /// The token the compositor minted for it.
        token: TokenId,
        /// How long the token stays redeemable.
        lifetime: u64,
    },
    /// A client asks to be activated, optionally with a token.
    Activate {
        /// Who is asking, and what it offers as authority.
        claim: Claimant,
    },
    /// A trusted gesture began, so an input context may exist for it.
    GestureBegan {
        /// The gesture the compositor observed.
        gesture: Gesture,
        /// How long the context stays usable after the gesture.
        lifetime: u64,
    },
    /// The gesture is over: a pointer was released, a key was released, a touch
    /// sequence was cancelled or the compositor lost it.
    GestureEnded {
        /// The gesture that ended.
        gesture: GestureId,
    },
    /// A client uses a trusted input context to do something privileged.
    ContextRequest {
        /// The gesture it claims.
        gesture: GestureId,
        /// The connection making the request.
        client: ClientId,
        /// The surface the request names.
        surface: SurfaceId,
        /// What it wants to do.
        request: ContextRequest,
    },
    /// A client claims the `portal.ui` slot: it draws consent and picker UI.
    PortalUiClaimed {
        /// The claiming connection.
        client: ClientId,
    },
    /// The `portal.ui` provider went away or gave the slot up.
    PortalUiReleased {
        /// The connection that held it.
        client: ClientId,
    },
    /// A portal asks whether a caller may capture or inject input.
    ConsentRequested {
        /// The portal's handle for this request.
        request: RequestId,
        /// The caller, named by the portal from peer credentials.
        requester: RequesterId,
        /// What it asks for.
        capability: PortalCapability,
        /// How long the consent prompt stays open.
        lifetime: u64,
    },
    /// The `portal.ui` provider answered a prompt.
    ConsentAnswered {
        /// Which prompt.
        request: RequestId,
        /// Which connection answered.
        from: ClientId,
        /// What it said.
        answer: Answer,
    },
    /// The caller's connection or process is gone: its grants end with it.
    RequesterGone {
        /// The caller that left.
        requester: RequesterId,
    },
    /// A granted caller actually uses the capability.
    GrantUsed {
        /// The caller.
        requester: RequesterId,
        /// What it is using.
        capability: PortalCapability,
    },
    /// The session lock became active or stopped being active.
    LockChanged {
        /// Whether anything is covered. `gnoblin-lock`'s `State::is_active`.
        active: bool,
    },
}

/// What a launch was, as the launcher knew it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchOrigin {
    /// The process group leader the launcher started.
    pub leader: SessionLeader,
    /// The label the process is expected to claim, if the launcher knows it.
    pub app_id: Option<AppId>,
}

/// One input gesture the compositor observed while dispatching an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Gesture {
    /// The compositor's serial for it.
    pub id: GestureId,
    /// The connection that handled it.
    pub client: ClientId,
    /// The surface it happened on.
    pub surface: SurfaceId,
    /// What kind of gesture it is, for readers of the context.
    pub kind: GestureKind,
}

/// A client asking for focus, with everything it offers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claimant {
    /// The connection asking.
    pub client: ClientId,
    /// Its session leader, from `SO_PEERCRED`.
    pub leader: SessionLeader,
    /// The surface it wants focused.
    pub surface: SurfaceId,
    /// The label it claims. Never authority on its own.
    pub app_id: Option<AppId>,
    /// The token it presents, if any.
    pub token: Option<TokenId>,
}

/// A privileged action only a trusted input context may authorise.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextRequest {
    /// Begin an interactive move of the gesture's surface.
    Move,
    /// Begin an interactive resize of the gesture's surface.
    Resize,
    /// Offer snap targets for the gesture's surface.
    SnapTarget,
    /// Set the text input target of the focused client.
    TextInput,
    /// Focus the gesture's surface. The plan's safety table allows activation
    /// from a token *or* from a trusted context; this is the second half, so a
    /// shell can raise a window the user just acted on without any token.
    Focus,
}

impl ContextRequest {
    /// A stable lower-case name for logs, events and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Resize => "resize",
            Self::SnapTarget => "snap-target",
            Self::TextInput => "text-input",
            Self::Focus => "focus",
        }
    }
}

/// What kind of input produced a context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GestureKind {
    /// A pointer button press.
    Pointer {
        /// The evdev button number.
        button: u32,
    },
    /// A key press with its modifiers.
    Key {
        /// The xkb keycode.
        key: u32,
        /// The active modifier mask when it was pressed.
        modifiers: u32,
    },
    /// A touch sequence.
    Touch,
}

/// What a portal caller may be granted.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PortalCapability {
    /// Read frames of the session.
    Capture,
    /// Inject input into the session.
    InputInjection,
    /// Read one app's secret from the keyring. Granted per app, never remembered.
    Secret,
}

impl PortalCapability {
    /// A stable lower-case name for logs, events and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::InputInjection => "input-injection",
            Self::Secret => "secret",
        }
    }
}

/// A consent prompt's answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    /// The user agreed.
    Allow,
    /// The user refused.
    Deny,
}
