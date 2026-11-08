//! Per-request negotiation (D19 "Negotiation per request", steps 3 to 5).
//!
//! A [`Negotiation`] is pure: [`Negotiation::start`] and [`Negotiation::handle`]
//! take the event and the current [`Tick`], and return the [`Effect`]s the
//! services role or the compositor must carry out. The first terminal outcome
//! wins; after it, every event is ignored.

use gnoblin_portal_core::model::Interface;
use gnoblin_types::{RequestId, Tick};

use super::outcome::{CancelReason, PortalError, Verdict};

/// The D19 negotiation contract for an interface: which ones a timeout may fall
/// back from, and which the built-in UI can serve.
pub trait InterfaceContract: Copy {
    /// `true` when a repeated or abandoned request has no effect at the provider,
    /// so a timeout may fall back to the built-in UI (D19 `idempotent` column).
    #[must_use]
    fn idempotent(self) -> bool;

    /// `true` when the built-in UI has a step for this interface (D19 "UI step").
    #[must_use]
    fn has_ui_step(self) -> bool;
}

impl InterfaceContract for Interface {
    fn idempotent(self) -> bool {
        matches!(
            self,
            Self::Screenshot | Self::PickColor | Self::Settings | Self::FileChooser | Self::Lockdown
        )
    }

    fn has_ui_step(self) -> bool {
        matches!(
            self,
            Self::ScreenCast
                | Self::Screenshot
                | Self::PickColor
                | Self::RemoteDesktop
                | Self::InputCapture
                | Self::FileChooser
                | Self::AppChooser
                | Self::Print
                | Self::Account
                | Self::Wallpaper
                | Self::DynamicLauncher
                | Self::Background
                | Self::Secret
                | Self::Access
        )
    }
}

/// Who serves the request first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalProvider {
    /// The built-in UI serves it directly (the `"builtin"` handler form).
    Builtin,
    /// A configured provider (process or `portal.ui` claim) serves it first.
    Remote,
}

/// Where the request is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for the provider's verdict.
    AwaitingProvider,
    /// Waiting for the built-in UI's verdict.
    AwaitingUi,
    /// Terminal. Nothing further happens.
    Finished,
}

/// Something that happened to a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The provider returned a verdict (a `Declined` one triggers fallback).
    ProviderVerdict(Verdict),
    /// The provider is absent, or failed to start, before it accepted the request.
    ProviderUnavailable,
    /// The built-in UI returned a verdict.
    UiVerdict(Verdict),
    /// The session locked.
    Locked,
    /// The requester (the D-Bus caller, the relay or the `Request` object) went away.
    RequesterGone,
    /// Time advanced. Checked against the current deadline.
    Tick,
}

/// What the caller must do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start the configured provider and give it the deadline.
    CallProvider { deadline: Tick },
    /// Tell the provider to stop (`portal-cancel`, SIGTERM then SIGKILL).
    CancelProvider,
    /// Open the built-in dialog for the request, with its deadline.
    OpenUi { deadline: Tick },
    /// Close the built-in dialog and tear down its input grab.
    CloseUi,
    /// The one terminal outcome, handed to the caller.
    Finish(Verdict),
}

/// One request's negotiation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negotiation {
    request: RequestId,
    interface: Interface,
    stage: Stage,
    deadline: Tick,
    ui_timeout_ms: u64,
}

impl Negotiation {
    /// Begins the negotiation and returns the first effects.
    ///
    /// `timeout_ms` bounds the provider; `ui_timeout_ms` bounds the built-in UI,
    /// whether it is the first step or a fallback after the provider's turn.
    #[must_use]
    pub fn start(
        request: RequestId,
        interface: Interface,
        provider: PortalProvider,
        now: Tick,
        timeout_ms: u64,
        ui_timeout_ms: u64,
    ) -> (Self, Vec<Effect>) {
        let mut negotiation = Self {
            request,
            interface,
            stage: Stage::Finished,
            deadline: now,
            ui_timeout_ms,
        };
        let effects = match provider {
            PortalProvider::Remote => {
                negotiation.stage = Stage::AwaitingProvider;
                negotiation.deadline = now.saturating_add(timeout_ms);
                vec![Effect::CallProvider {
                    deadline: negotiation.deadline,
                }]
            }
            PortalProvider::Builtin => negotiation.enter_ui(now),
        };
        (negotiation, effects)
    }

    /// The request this negotiation serves.
    #[must_use]
    pub const fn request(&self) -> RequestId {
        self.request
    }

    /// The current stage.
    #[must_use]
    pub const fn stage(&self) -> Stage {
        self.stage
    }

    /// Applies one event at time `now` and returns the effects to carry out.
    pub fn handle(&mut self, event: Event, now: Tick) -> Vec<Effect> {
        if self.stage == Stage::Finished {
            return Vec::new();
        }
        if let Event::RequesterGone = event {
            return self.finish_cancelled(CancelReason::RequesterGone);
        }
        if let Event::Tick = event {
            return if now >= self.deadline {
                self.on_deadline()
            } else {
                Vec::new()
            };
        }
        match self.stage {
            Stage::AwaitingProvider => self.on_provider(event, now),
            Stage::AwaitingUi => self.on_ui(event),
            Stage::Finished => Vec::new(),
        }
    }

    fn on_provider(&mut self, event: Event, now: Tick) -> Vec<Effect> {
        match event {
            Event::ProviderVerdict(Verdict::Handled(grant)) => self.finish(Verdict::Handled(grant)),
            Event::ProviderVerdict(Verdict::Cancelled(reason)) => self.finish(Verdict::Cancelled(reason)),
            Event::ProviderVerdict(Verdict::Error(error)) => self.finish(Verdict::Error(error)),
            Event::ProviderVerdict(Verdict::Declined) | Event::ProviderUnavailable => {
                // Declined and absent providers did not act: fall back.
                self.fall_back(now)
            }
            Event::Locked => self.finish_with(Verdict::Error(PortalError::Locked), vec![Effect::CloseUi]),
            Event::UiVerdict(_) => Vec::new(),
            Event::RequesterGone | Event::Tick => unreachable!("handled before dispatch"),
        }
    }

    fn on_ui(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::UiVerdict(Verdict::Handled(grant)) => self.finish_with(Verdict::Handled(grant), Vec::new()),
            Event::UiVerdict(Verdict::Cancelled(reason)) => {
                self.finish_with(Verdict::Cancelled(reason), vec![Effect::CloseUi])
            }
            Event::UiVerdict(Verdict::Error(error)) => self.finish_with(Verdict::Error(error), vec![Effect::CloseUi]),
            // The UI cannot decline, and a failed UI never auto-accepts.
            Event::UiVerdict(Verdict::Declined) => {
                self.finish_with(Verdict::Cancelled(CancelReason::UiFailed), vec![Effect::CloseUi])
            }
            Event::Locked => self.finish_with(Verdict::Error(PortalError::Locked), vec![Effect::CloseUi]),
            Event::ProviderVerdict(_) | Event::ProviderUnavailable => Vec::new(),
            Event::RequesterGone | Event::Tick => unreachable!("handled before dispatch"),
        }
    }

    fn on_deadline(&mut self) -> Vec<Effect> {
        match self.stage {
            Stage::AwaitingProvider if self.interface.idempotent() => {
                let mut effects = vec![Effect::CancelProvider];
                effects.extend(self.fall_back(self.deadline));
                effects
            }
            Stage::AwaitingProvider => {
                let mut effects = vec![Effect::CancelProvider];
                effects.extend(self.finish(Verdict::Cancelled(CancelReason::Timeout)));
                effects
            }
            Stage::AwaitingUi => self.finish_with(Verdict::Cancelled(CancelReason::Timeout), vec![Effect::CloseUi]),
            Stage::Finished => Vec::new(),
        }
    }

    /// Falls back to the built-in UI, or ends with `NotSupported` when the
    /// interface has no UI step to fall back to.
    fn fall_back(&mut self, now: Tick) -> Vec<Effect> {
        if self.interface.has_ui_step() {
            self.enter_ui(now)
        } else {
            self.finish(Verdict::Error(PortalError::NotSupported))
        }
    }

    fn enter_ui(&mut self, now: Tick) -> Vec<Effect> {
        self.stage = Stage::AwaitingUi;
        self.deadline = now.saturating_add(self.ui_timeout_ms);
        vec![Effect::OpenUi {
            deadline: self.deadline,
        }]
    }

    fn finish_cancelled(&mut self, reason: CancelReason) -> Vec<Effect> {
        let mut effects = match self.stage {
            Stage::AwaitingProvider => vec![Effect::CancelProvider],
            Stage::AwaitingUi => vec![Effect::CloseUi],
            Stage::Finished => Vec::new(),
        };
        effects.extend(self.finish(Verdict::Cancelled(reason)));
        effects
    }

    fn finish(&mut self, verdict: Verdict) -> Vec<Effect> {
        self.finish_with(verdict, Vec::new())
    }

    fn finish_with(&mut self, verdict: Verdict, mut leading: Vec<Effect>) -> Vec<Effect> {
        self.stage = Stage::Finished;
        leading.push(Effect::Finish(verdict));
        leading
    }
}
