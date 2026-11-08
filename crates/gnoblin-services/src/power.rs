//! The logind power backend: `gnoblin.session.poweroff()`, `reboot()`, `suspend()` and
//! `hibernate()`, carried out by `org.freedesktop.login1.Manager` on the system bus.
//!
//! Every action is asked with `interactive = false`. The session never asks logind to start a
//! polkit prompt, because the shell confirms a power verb before it reaches the session. So a
//! request polkit would have to ask about is refused as [`PowerErrorKind::Denied`] instead of
//! waiting for an answer nobody can give.
//!
//! [`LogindPower`] holds the decisions: which D-Bus error name is which refusal, and the check
//! `CanHibernate` makes before a hibernate is asked. The calls themselves go through
//! [`Login1Manager`]. The system bus implements it ([`system_power_backend`]); the tests implement
//! it with recorded answers.

use futures_lite::future::block_on;
use gnoblin_model::{PowerBackend, PowerError, PowerErrorKind, PowerVerb};
use zbus::proxy;

use crate::log;

/// The logind manager, as the power backend uses it.
#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LogindManager {
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;

    fn reboot(&self, interactive: bool) -> zbus::Result<()>;

    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    fn hibernate(&self, interactive: bool) -> zbus::Result<()>;

    fn can_power_off(&self) -> zbus::Result<String>;

    fn can_reboot(&self) -> zbus::Result<String>;

    fn can_suspend(&self) -> zbus::Result<String>;

    fn can_hibernate(&self) -> zbus::Result<String>;
}

/// A call to the manager that did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFailure {
    /// The D-Bus error name the manager replied with. `None` when there was no reply at all:
    /// the system bus is absent or the call did not complete.
    name: Option<String>,
    /// What the failure said, for the log and the refusal's message.
    detail: String,
}

impl CallFailure {
    /// A failure the manager replied to with the D-Bus error `name`.
    pub fn replied(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            detail: detail.into(),
        }
    }

    /// A failure with no reply: there is no system bus, or the call never completed.
    pub fn unreachable(detail: impl Into<String>) -> Self {
        Self {
            name: None,
            detail: detail.into(),
        }
    }

    fn from_zbus(error: &zbus::Error) -> Self {
        match error {
            zbus::Error::MethodError(name, detail, _) => {
                Self::replied(name.to_string(), detail.clone().unwrap_or_else(|| name.to_string()))
            }
            other => Self::unreachable(other.to_string()),
        }
    }
}

/// The calls the power backend makes to the init system. The system bus implements it; a test
/// implements it with recorded answers.
pub trait Login1Manager: Send {
    /// Asks the manager to carry out `verb`, with `interactive` as its flag.
    ///
    /// # Errors
    ///
    /// The [`CallFailure`] the manager replied with, or that there was no reply.
    fn action(&mut self, verb: PowerVerb, interactive: bool) -> Result<(), CallFailure>;

    /// The manager's capability answer for `verb`: `yes`, `no`, `na` or `challenge`.
    ///
    /// # Errors
    ///
    /// The [`CallFailure`] the manager replied with, or that there was no reply.
    fn capability(&mut self, verb: PowerVerb) -> Result<String, CallFailure>;
}

/// The power backend the session links: each verb is one call on a [`Login1Manager`].
pub struct LogindPower {
    manager: Box<dyn Login1Manager>,
}

impl LogindPower {
    /// A backend that makes its calls through `manager`.
    #[must_use]
    pub fn new(manager: Box<dyn Login1Manager>) -> Self {
        Self { manager }
    }

    /// Hibernate is asked only when logind says the machine can do it. `yes` and `challenge`
    /// go ahead; `challenge` needs authorisation, which the refusal path reports as denied.
    fn check_hibernate(&mut self) -> Result<(), PowerError> {
        let verb = PowerVerb::Hibernate;
        let answer = self
            .manager
            .capability(verb)
            .map_err(|failure| refusal(verb, &failure))?;
        match answer.as_str() {
            "yes" | "challenge" => Ok(()),
            other => Err(PowerError::new(
                verb,
                PowerErrorKind::Unsupported,
                format!("CanHibernate answered {other}"),
            )),
        }
    }
}

impl PowerBackend for LogindPower {
    fn perform(&mut self, verb: PowerVerb) -> Result<(), PowerError> {
        if verb == PowerVerb::Hibernate {
            self.check_hibernate()?;
        }
        match self.manager.action(verb, false) {
            Ok(()) => {
                log::info!(verb = verb.name(), "logind accepted the power verb");
                Ok(())
            }
            Err(failure) => {
                let refused = refusal(verb, &failure);
                log::warn!(verb = verb.name(), kind = ?refused.kind(), "logind refused the power verb");
                Err(refused)
            }
        }
    }
}

/// The refusal for a failed call to `verb`. A failure with no reply means the init system cannot
/// be reached, which is [`PowerErrorKind::Unavailable`].
fn refusal(verb: PowerVerb, failure: &CallFailure) -> PowerError {
    let kind = match failure.name.as_deref() {
        None => PowerErrorKind::Unavailable,
        Some(name) => error_kind(name),
    };
    PowerError::new(verb, kind, failure.detail.clone())
}

/// Which refusal kind a D-Bus error name from the manager stands for. Names that are not listed
/// are [`PowerErrorKind::Internal`], so an unknown answer still fails rather than passing as success.
#[must_use]
pub fn error_kind(name: &str) -> PowerErrorKind {
    match name {
        "org.freedesktop.DBus.Error.AccessDenied"
        | "org.freedesktop.PolicyKit1.Error.NotAuthorized"
        | "org.freedesktop.login1.InteractiveAuthorizationRequired" => PowerErrorKind::Denied,
        "org.freedesktop.DBus.Error.NotSupported" | "org.freedesktop.login1.SleepVerbNotSupported" => {
            PowerErrorKind::Unsupported
        }
        "org.freedesktop.login1.Inhibited" | "org.freedesktop.login1.ShuttingDown" => PowerErrorKind::Busy,
        "org.freedesktop.DBus.Error.ServiceUnknown" => PowerErrorKind::Unavailable,
        "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => PowerErrorKind::TimedOut,
        _ => PowerErrorKind::Internal,
    }
}

/// The manager on the system bus. Each call opens its own connection, so a bus that comes up
/// after the session started is used by the next verb without a restart.
struct SystemManager;

impl SystemManager {
    /// A proxy for the manager on a fresh system bus connection, so a bus that comes up after
    /// the session started is used by the next verb without a restart.
    async fn proxy() -> Result<LogindManagerProxy<'static>, CallFailure> {
        let connection = zbus::Connection::system()
            .await
            .map_err(|error| CallFailure::from_zbus(&error))?;
        LogindManagerProxy::new(&connection)
            .await
            .map_err(|error| CallFailure::from_zbus(&error))
    }
}

impl Login1Manager for SystemManager {
    fn action(&mut self, verb: PowerVerb, interactive: bool) -> Result<(), CallFailure> {
        block_on(async {
            let proxy = Self::proxy().await?;
            let result = match verb {
                PowerVerb::PowerOff => proxy.power_off(interactive).await,
                PowerVerb::Reboot => proxy.reboot(interactive).await,
                PowerVerb::Suspend => proxy.suspend(interactive).await,
                PowerVerb::Hibernate => proxy.hibernate(interactive).await,
            };
            result.map_err(|error| CallFailure::from_zbus(&error))
        })
    }

    fn capability(&mut self, verb: PowerVerb) -> Result<String, CallFailure> {
        block_on(async {
            let proxy = Self::proxy().await?;
            let result = match verb {
                PowerVerb::PowerOff => proxy.can_power_off().await,
                PowerVerb::Reboot => proxy.can_reboot().await,
                PowerVerb::Suspend => proxy.can_suspend().await,
                PowerVerb::Hibernate => proxy.can_hibernate().await,
            };
            result.map_err(|error| CallFailure::from_zbus(&error))
        })
    }
}

/// The power backend over the system bus's logind manager. A machine without a system bus gets a
/// backend that answers every verb [`PowerErrorKind::Unavailable`].
#[must_use]
pub fn system_power_backend() -> Box<dyn PowerBackend> {
    Box::new(LogindPower::new(Box::new(SystemManager)))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Records each call and answers with the configured results.
    struct Fake {
        calls: Arc<Mutex<Vec<String>>>,
        action: Result<(), CallFailure>,
        capability: Result<String, CallFailure>,
    }

    impl Fake {
        fn answering(action: Result<(), CallFailure>) -> (Self, Arc<Mutex<Vec<String>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let fake = Self {
                calls: Arc::clone(&calls),
                action,
                capability: Ok("yes".to_owned()),
            };
            (fake, calls)
        }
    }

    impl Login1Manager for Fake {
        fn action(&mut self, verb: PowerVerb, interactive: bool) -> Result<(), CallFailure> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("action:{}:{interactive}", verb.name()));
            self.action.clone()
        }

        fn capability(&mut self, verb: PowerVerb) -> Result<String, CallFailure> {
            self.calls.lock().unwrap().push(format!("capability:{}", verb.name()));
            self.capability.clone()
        }
    }

    fn backend(fake: Fake) -> LogindPower {
        LogindPower::new(Box::new(fake))
    }

    #[test]
    fn each_verb_is_one_non_interactive_call() {
        for verb in [PowerVerb::PowerOff, PowerVerb::Reboot, PowerVerb::Suspend] {
            let (fake, calls) = Fake::answering(Ok(()));
            assert_eq!(backend(fake).perform(verb), Ok(()));
            assert_eq!(*calls.lock().unwrap(), [format!("action:{}:false", verb.name())]);
        }
    }

    #[test]
    fn hibernate_checks_the_capability_then_asks_once() {
        let (fake, calls) = Fake::answering(Ok(()));
        assert_eq!(backend(fake).perform(PowerVerb::Hibernate), Ok(()));
        assert_eq!(
            *calls.lock().unwrap(),
            ["capability:hibernate", "action:hibernate:false"]
        );
    }

    #[test]
    fn a_machine_that_cannot_hibernate_is_unsupported_and_never_asked() {
        for answer in ["no", "na"] {
            let (mut fake, calls) = Fake::answering(Ok(()));
            fake.capability = Ok(answer.to_owned());
            let error = backend(fake).perform(PowerVerb::Hibernate).unwrap_err();
            assert_eq!(error.kind(), PowerErrorKind::Unsupported);
            assert_eq!(error.code(), gnoblin_types::ErrorCode::from_static("unsupported"));
            assert_eq!(*calls.lock().unwrap(), ["capability:hibernate"]);
        }
    }

    #[test]
    fn a_challenge_answer_still_asks_and_the_refusal_is_denied() {
        let (mut fake, calls) = Fake::answering(Err(CallFailure::replied(
            "org.freedesktop.PolicyKit1.Error.NotAuthorized",
            "not authorised",
        )));
        fake.capability = Ok("challenge".to_owned());
        let error = backend(fake).perform(PowerVerb::Hibernate).unwrap_err();
        assert_eq!(error.kind(), PowerErrorKind::Denied);
        assert_eq!(error.code(), gnoblin_types::ErrorCode::from_static("denied"));
        assert_eq!(
            *calls.lock().unwrap(),
            ["capability:hibernate", "action:hibernate:false"]
        );
    }

    #[test]
    fn a_failed_capability_check_refuses_without_asking() {
        let (mut fake, calls) = Fake::answering(Ok(()));
        fake.capability = Err(CallFailure::unreachable("no system bus"));
        let error = backend(fake).perform(PowerVerb::Hibernate).unwrap_err();
        assert_eq!(error.kind(), PowerErrorKind::Unavailable);
        assert_eq!(*calls.lock().unwrap(), ["capability:hibernate"]);
    }

    #[test]
    fn a_refused_action_keeps_the_verb_and_the_detail() {
        let (fake, _) = Fake::answering(Err(CallFailure::replied(
            "org.freedesktop.login1.Inhibited",
            "an inhibitor is active",
        )));
        let error = backend(fake).perform(PowerVerb::Reboot).unwrap_err();
        assert_eq!(error.verb(), PowerVerb::Reboot);
        assert_eq!(error.kind(), PowerErrorKind::Busy);
        assert_eq!(error.detail(), "an inhibitor is active");
    }

    #[test]
    fn no_reply_is_unavailable_and_interactive_refusals_are_denied() {
        let (fake, _) = Fake::answering(Err(CallFailure::unreachable("no system bus")));
        assert_eq!(
            backend(fake).perform(PowerVerb::PowerOff).unwrap_err().kind(),
            PowerErrorKind::Unavailable
        );
        let (fake, _) = Fake::answering(Err(CallFailure::replied(
            "org.freedesktop.login1.InteractiveAuthorizationRequired",
            "needs a prompt",
        )));
        assert_eq!(
            backend(fake).perform(PowerVerb::Suspend).unwrap_err().kind(),
            PowerErrorKind::Denied
        );
    }

    #[test]
    fn error_names_map_to_their_kinds() {
        assert_eq!(
            error_kind("org.freedesktop.DBus.Error.AccessDenied"),
            PowerErrorKind::Denied
        );
        assert_eq!(
            error_kind("org.freedesktop.DBus.Error.NotSupported"),
            PowerErrorKind::Unsupported
        );
        assert_eq!(error_kind("org.freedesktop.login1.ShuttingDown"), PowerErrorKind::Busy);
        assert_eq!(
            error_kind("org.freedesktop.DBus.Error.ServiceUnknown"),
            PowerErrorKind::Unavailable
        );
        assert_eq!(
            error_kind("org.freedesktop.DBus.Error.NoReply"),
            PowerErrorKind::TimedOut
        );
        assert_eq!(error_kind("org.example.Surprise"), PowerErrorKind::Internal);
    }
}
