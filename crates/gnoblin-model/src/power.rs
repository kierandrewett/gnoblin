//! The power verbs `gnoblin.session.poweroff()`, `reboot()`, `suspend()` and `hibernate()` ask
//! for, and the backend that carries them out.
//!
//! The model names the verbs and the errors a refused verb reports, and nothing else: it has no
//! bus. A [`PowerBackend`] is whatever the session binary links for the machine's init system;
//! the services role provides the logind one. A session that links none answers every verb with
//! [`PowerErrorKind::Unavailable`], so a caller never waits for a request that cannot be carried out.

use gnoblin_types::ErrorCode;

/// One power action the session can ask the machine to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerVerb {
    /// Power the machine off.
    PowerOff,
    /// Restart the machine.
    Reboot,
    /// Suspend the machine to RAM.
    Suspend,
    /// Suspend the machine to disk.
    Hibernate,
}

impl PowerVerb {
    /// The name the verb has in the Lua and IPC surface: the method on the `session` object.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PowerOff => "poweroff",
            Self::Reboot => "reboot",
            Self::Suspend => "suspend",
            Self::Hibernate => "hibernate",
        }
    }
}

/// Why a power verb was refused. Each kind has one schema error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerErrorKind {
    /// The init system refused the action: a challenge that cannot be answered here, or authorisation was denied.
    Denied,
    /// The machine cannot do the action: its capability answer is `no` or `na`, or the init system has no such action.
    Unsupported,
    /// The init system is already running another power action.
    Busy,
    /// No power backend is available in this session, for example the system bus is absent.
    Unavailable,
    /// The init system did not answer in time.
    TimedOut,
    /// Any other failure.
    Internal,
}

/// A refused power verb. The caller receives [`Self::code`] and the message from `Display`;
/// the verb and the detail are logged where the code alone would not say enough.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerError {
    verb: PowerVerb,
    kind: PowerErrorKind,
    detail: String,
}

impl PowerError {
    /// A refusal of `verb` of `kind`, with the detail that says what the init system answered.
    pub fn new(verb: PowerVerb, kind: PowerErrorKind, detail: impl Into<String>) -> Self {
        Self {
            verb,
            kind,
            detail: detail.into(),
        }
    }

    /// The verb that was refused.
    #[must_use]
    pub const fn verb(&self) -> PowerVerb {
        self.verb
    }

    /// The kind of refusal.
    #[must_use]
    pub const fn kind(&self) -> PowerErrorKind {
        self.kind
    }

    /// The detail the init system gave, for example the D-Bus error name.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// The kebab-case code a caller receives for this refusal.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self.kind {
            PowerErrorKind::Denied => ErrorCode::from_static("denied"),
            PowerErrorKind::Unsupported => ErrorCode::from_static("unsupported"),
            PowerErrorKind::Busy => ErrorCode::from_static("busy"),
            PowerErrorKind::Unavailable => ErrorCode::from_static("unavailable"),
            PowerErrorKind::TimedOut => ErrorCode::from_static("timed-out"),
            PowerErrorKind::Internal => ErrorCode::from_static("internal"),
        }
    }
}

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the init system could not {}: {}", self.verb.name(), self.detail)
    }
}

/// Carries out one power verb. The compositor calls it on its IPC dispatch thread, so an
/// implementation must return within its own bound rather than wait for the machine.
pub trait PowerBackend: std::fmt::Debug + Send {
    /// Ask for `verb`. `Ok` means the init system accepted the request; the action may still be
    /// under way.
    ///
    /// # Errors
    ///
    /// The [`PowerError`] that says why the request was refused.
    fn perform(&mut self, verb: PowerVerb) -> Result<(), PowerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_verb_has_its_session_method_name() {
        assert_eq!(PowerVerb::PowerOff.name(), "poweroff");
        assert_eq!(PowerVerb::Reboot.name(), "reboot");
        assert_eq!(PowerVerb::Suspend.name(), "suspend");
        assert_eq!(PowerVerb::Hibernate.name(), "hibernate");
    }

    #[test]
    fn every_kind_has_a_kebab_code_and_the_message_names_the_verb() {
        let kinds = [
            (PowerErrorKind::Denied, "denied"),
            (PowerErrorKind::Unsupported, "unsupported"),
            (PowerErrorKind::Busy, "busy"),
            (PowerErrorKind::Unavailable, "unavailable"),
            (PowerErrorKind::TimedOut, "timed-out"),
            (PowerErrorKind::Internal, "internal"),
        ];
        for (kind, code) in kinds {
            let error = PowerError::new(PowerVerb::Hibernate, kind, "org.example.Detail: text");
            assert_eq!(error.verb(), PowerVerb::Hibernate);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.code(), ErrorCode::from_static(code));
            assert_eq!(error.detail(), "org.example.Detail: text");
            assert_eq!(
                error.to_string(),
                "the init system could not hibernate: org.example.Detail: text"
            );
        }
    }
}
