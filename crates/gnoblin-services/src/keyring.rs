//! The Secret Service the services role serves, when `keyring.enabled` is true.
//!
//! `design/keyring.md` ("Coexistence"): with `keyring.enabled` on, which is the default, Gnoblin
//! takes `org.freedesktop.secrets` and serves its own store. With it off, the role does not request
//! the name, creates no store file or directory, and serves nothing; another program keeps the name.
//!
//! The flag comes from the configuration the compositor has accepted in this session, not from the
//! last accepted file. On a first boot that file does not exist until the runtime has evaluated
//! `init.lua` and the compositor has committed it, which is after the services role starts. So the
//! agent waits for an accepted configuration for at most [`CONFIGURATION_WAIT`]. Only when none
//! arrives in that time does it read the last accepted file, and then fall back to the schema
//! default. [`decide`] is that rule. The agent runs it on its own thread, so starting the role
//! never blocks on the compositor.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_model::{CapabilitySlot, DeclineReason};
use gnoblin_service_keyring::{
    PasswordOffer, PromptAnswers, PromptId, PromptOutcome, PromptRefusal, Secrets, ServeError,
};

use crate::link::{Host, LinkedAgent, RequestId, RequestOutcome, Tick};
use crate::log;

/// How long the agent waits for the compositor to accept a configuration before it falls back to the
/// last accepted file. `docs/config/configure/keyring.md` states this value.
pub const CONFIGURATION_WAIT: Duration = Duration::from_secs(20);

/// How often the agent asks the compositor for its accepted configuration while it waits.
pub const CONFIGURATION_POLL: Duration = Duration::from_millis(100);

/// Where the `keyring.enabled` flag the agent acted on came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyringSource {
    /// The compositor accepted this configuration revision in this session.
    Accepted {
        /// The revision the compositor reported.
        revision: u64,
    },
    /// No accepted configuration arrived in time, and the last accepted file decided.
    LastAccepted,
    /// No accepted configuration arrived and there is no readable last accepted file, so the schema
    /// default decided.
    Default,
}

impl fmt::Display for KeyringSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyringSource::Accepted { revision } => write!(formatter, "accepted revision {revision}"),
            KeyringSource::LastAccepted => formatter.write_str("last accepted file"),
            KeyringSource::Default => formatter.write_str("schema default"),
        }
    }
}

/// The outcome of [`decide`]: whether the keyring runs, where that came from, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyringDecision {
    /// Whether Gnoblin serves the Secret Service.
    pub enabled: bool,
    /// Which source the flag came from.
    pub source: KeyringSource,
    /// A human-readable reason for the source, for the log.
    pub reason: String,
}

/// Decides `keyring.enabled` for this session.
///
/// `accepted` is asked for the compositor's accepted configuration on every poll, until it has one
/// or `wait` has passed. An accepted document decides at once. A document that is not a readable
/// configuration also decides at once, by falling back, because the compositor only accepts
/// documents its runtime validated. A read error is kept as the reason, and polling continues.
/// `last_accepted` is read only when no accepted configuration decided. It returns the flag of the
/// last accepted file, or `None` when there is no such file.
pub fn decide(
    mut accepted: impl FnMut() -> Result<Option<(u64, Vec<u8>)>, String>,
    last_accepted: impl FnOnce() -> Result<Option<bool>, String>,
    wait: Duration,
    poll: Duration,
) -> KeyringDecision {
    let started = Instant::now();
    let mut last_problem = None;
    loop {
        match accepted() {
            Ok(Some((revision, document))) => {
                return match gnoblin_config::keyring_enabled_from_document(&document) {
                    Ok(enabled) => KeyringDecision {
                        enabled,
                        source: KeyringSource::Accepted { revision },
                        reason: format!("configuration revision {revision} was accepted this session"),
                    },
                    Err(problem) => fallback(
                        last_accepted,
                        format!("accepted configuration revision {revision} is unreadable: {problem}"),
                    ),
                };
            }
            Ok(None) => {}
            Err(problem) => last_problem = Some(problem),
        }
        let elapsed = started.elapsed();
        if elapsed >= wait {
            break;
        }
        thread::sleep(poll.min(wait - elapsed));
    }
    let reason = match last_problem {
        Some(problem) => format!("no accepted configuration within {wait:?} (last error: {problem})"),
        None => format!("no accepted configuration within {wait:?}"),
    };
    fallback(last_accepted, reason)
}

/// The decision when no accepted configuration decided: the last accepted file, else the default.
fn fallback(last_accepted: impl FnOnce() -> Result<Option<bool>, String>, reason: String) -> KeyringDecision {
    match last_accepted() {
        Ok(Some(enabled)) => KeyringDecision {
            enabled,
            source: KeyringSource::LastAccepted,
            reason: format!("{reason}; the last accepted file decides"),
        },
        Ok(None) => KeyringDecision {
            enabled: true,
            source: KeyringSource::Default,
            reason: format!("{reason}; there is no last accepted file, so the schema default decides"),
        },
        Err(problem) => KeyringDecision {
            enabled: true,
            source: KeyringSource::Default,
            reason: format!(
                "{reason}; the last accepted file is unreadable ({problem}), so the schema default decides"
            ),
        },
    }
}

/// The store the Secret Service keeps its items in: `$XDG_DATA_HOME/gnoblin/keyring.db`, or under
/// `$HOME/.local/share` when `XDG_DATA_HOME` is unset. `None` when neither is an absolute path.
#[must_use]
pub fn store_path_from_environment() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let data_home = absolute("XDG_DATA_HOME").or_else(|| absolute("HOME").map(|home| home.join(".local/share")))?;
    Some(data_home.join("gnoblin").join("keyring.db"))
}

/// The hand-off socket the unlock path listens on. It is `$XDG_RUNTIME_DIR` when that is absolute,
/// and `/run/user/<uid>` otherwise, which is where the PAM module looks as well.
#[must_use]
pub fn socket_path_from_environment() -> PathBuf {
    let runtime_dir = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw())),
    };
    gnoblin_keyring_handoff::socket_path(&runtime_dir)
}

/// What the agent holds once the store serves: the bus connection that keeps the name, the secrets
/// behind `secret.derive`, and the answers to the unlock prompt.
struct Served {
    /// Held so the bus name stays ours for as long as the role runs.
    _connection: zbus::Connection,
    secrets: Secrets,
    prompt_answers: PromptAnswers,
}

/// The Secret Service agent. On its own thread it decides `keyring.enabled`, then, when the flag is
/// on, serves `org.freedesktop.secrets` from its store and socket, or logs why it cannot.
pub struct KeyringAgent {
    store: PathBuf,
    socket: PathBuf,
    /// Set by the decision thread once the store serves. Absent while the decision is pending, when
    /// the keyring is off, and when the name is taken or the store cannot be served.
    served: Arc<Mutex<Option<Served>>>,
}

impl KeyringAgent {
    /// An agent for the store at `store` and the hand-off socket at `socket`. It has not started.
    #[must_use]
    pub fn new(store: PathBuf, socket: PathBuf) -> KeyringAgent {
        KeyringAgent {
            store,
            socket,
            served: Arc::new(Mutex::new(None)),
        }
    }

    /// The served state, with a poisoned lock recovered rather than propagated.
    fn served(&self) -> MutexGuard<'_, Option<Served>> {
        self.served.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How long the password prompt waits for its answer before the compositor times it out.
const PROMPT_DEADLINE_MS: u64 = 120_000;

/// Offers the unlock password as a `prompt.secret` request. The compositor routes it to the shell
/// that claimed the slot. An unclaimed slot refuses the offer at once, so the keyring stays locked
/// rather than waiting on a prompt nobody draws.
struct HostOffer {
    host: Host,
}

/// The keyring's view of a password request's outcome. The answer is copied into the keyring's
/// own zeroizing buffer; the original stays wiped by `RequestOutcome`.
fn prompt_outcome(outcome: &RequestOutcome) -> PromptOutcome {
    match outcome {
        RequestOutcome::Answered(answer) => PromptOutcome::answered(answer.to_vec()),
        RequestOutcome::Ended => PromptOutcome::Ended,
        RequestOutcome::Refused(reason) => PromptOutcome::Refused(match reason {
            DeclineReason::Locked => PromptRefusal::Locked,
            DeclineReason::Denied => PromptRefusal::Denied,
            DeclineReason::Unavailable => PromptRefusal::Unavailable,
        }),
        RequestOutcome::TimedOut => PromptOutcome::TimedOut,
    }
}

impl PasswordOffer for HostOffer {
    fn offer(&mut self, title: &str, message: &str) -> Result<PromptId, String> {
        self.host
            .offer(
                CapabilitySlot::PromptSecret.name(),
                title,
                message,
                b"",
                Tick::new(PROMPT_DEADLINE_MS),
            )
            .map(|request| PromptId::new(request.get()))
            .map_err(|error| format!("{error:?}"))
    }
}

/// Serves `org.freedesktop.secrets` from `store` and `socket`, and keeps what it serves in `served`.
fn serve_store(host: &Host, store: &Path, socket: &Path, served: &Mutex<Option<Served>>) {
    let offers = Box::new(HostOffer { host: host.clone() });
    match futures_lite::future::block_on(gnoblin_service_keyring::serve(store, socket, offers)) {
        Ok((connection, secrets, prompt_answers)) => {
            *served.lock().unwrap_or_else(PoisonError::into_inner) = Some(Served {
                _connection: connection,
                secrets,
                prompt_answers,
            });
        }
        Err(ServeError::NameTaken { owner }) => {
            // Apps reach whichever program holds the name, so this is loud on purpose. The
            // role keeps running with the rest of the services; it does not take the name back.
            log::warn!(
                area = "services",
                %owner,
                "keyring.enabled is on but org.freedesktop.secrets is held by another program: \
                 apps will use that program, not the Gnoblin keyring. Stop it, or set keyring.enabled = false"
            );
        }
        Err(error) => {
            log::warn!(area = "services", %error, "the Gnoblin keyring could not be served");
        }
    }
}

/// The decision thread's body: decide `keyring.enabled` from the accepted configuration, then serve
/// the store only when it is on. When it is off, nothing is served and the store is not touched.
fn decide_and_serve(host: &Host, store: &Path, socket: &Path, served: &Mutex<Option<Served>>) {
    let decision = decide(
        || host.configuration().map_err(|error| format!("{error:?}")),
        gnoblin_config::last_good_keyring_enabled_from_environment,
        CONFIGURATION_WAIT,
        CONFIGURATION_POLL,
    );
    log::info!(
        area = "services",
        enabled = decision.enabled,
        source = %decision.source,
        reason = %decision.reason,
        "keyring.enabled decided"
    );
    if decision.enabled {
        serve_store(host, store, socket, served);
    }
}

impl LinkedAgent for KeyringAgent {
    /// Starts the decision thread and returns at once: the role must not wait for the compositor's
    /// configuration. A thread that cannot start leaves the agent unlinked.
    fn start(&mut self, host: &Host, _deadline: Tick) -> bool {
        let host = host.clone();
        let store = self.store.clone();
        let socket = self.socket.clone();
        let served = Arc::clone(&self.served);
        match thread::Builder::new()
            .name("gnoblin-keyring".to_owned())
            .spawn(move || decide_and_serve(&host, &store, &socket, &served))
        {
            Ok(_) => true,
            Err(error) => {
                log::warn!(area = "services", %error, "the Gnoblin keyring cannot start its decision thread");
                false
            }
        }
    }

    fn answer(&self, request: RequestId, outcome: &RequestOutcome) {
        if let Some(served) = self.served().as_ref() {
            served
                .prompt_answers
                .answer(PromptId::new(request.get()), prompt_outcome(outcome));
        }
    }

    /// Derive the app secret for `secret.derive`. The subject is the app id, which the portal
    /// sends as the offer's subject. A store that is not serving, or a missing app id, declines,
    /// because the caller is left with nothing it can use and must not wait for the deadline.
    fn offer(&self, host: &Host, request: RequestId, slot: &str, subject: &[u8]) -> bool {
        if slot != CapabilitySlot::SecretDerive.name() {
            return false;
        }
        let secret = self
            .served()
            .as_ref()
            .ok_or_else(|| "the store is not serving".to_owned())
            .and_then(|served| {
                let app_id = std::str::from_utf8(subject).map_err(|error| error.to_string())?;
                served.secrets.app_secret(app_id).map_err(|error| format!("{error:?}"))
            });
        match secret {
            Ok(secret) => host.answer(request, &secret[..]),
            Err(reason) => {
                log::info!(area = "services", request = request.get(), %reason, "declining a secret offer");
                host.decline(request);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use super::{KeyringSource, decide};

    /// A configuration document as the compositor sends it, with `keyring.enabled` set to `enabled`.
    fn document(enabled: bool) -> Vec<u8> {
        format!(r#"{{"keyring":{{"enabled":{enabled}}}}}"#).into_bytes()
    }

    const WAIT: Duration = Duration::from_millis(40);
    const POLL: Duration = Duration::ZERO;

    #[test]
    fn with_no_configuration_yet_the_decision_waits_out_the_window_before_it_falls_back() {
        // First boot: the compositor has accepted nothing. The decision must not be taken early:
        // the store is only touched once it returns, so waiting here is what keeps it untouched.
        let started = Instant::now();
        let decision = decide(|| Ok(None), || Ok(None), WAIT, Duration::from_millis(5));
        assert!(started.elapsed() >= WAIT, "the decision waited for the configuration");
        assert_eq!(decision.source, KeyringSource::Default);
        assert!(decision.enabled);
    }

    #[test]
    fn first_boot_with_the_keyring_off_in_init_lua_decides_off_even_with_no_last_accepted_file() {
        // The reported bug: `keyring.enabled = false` in init.lua, no last accepted file yet. The
        // accepted configuration decides, so the keyring is not served.
        let decision = decide(|| Ok(Some((1, document(false)))), || Ok(None), WAIT, POLL);
        assert!(!decision.enabled);
        assert_eq!(decision.source, KeyringSource::Accepted { revision: 1 });
    }

    #[test]
    fn an_accepted_configuration_with_the_keyring_on_decides_on() {
        let decision = decide(|| Ok(Some((2, document(true)))), || Ok(Some(false)), WAIT, POLL);
        assert!(
            decision.enabled,
            "the accepted configuration beats the last accepted file"
        );
        assert_eq!(decision.source, KeyringSource::Accepted { revision: 2 });
    }

    #[test]
    fn a_configuration_accepted_late_is_the_one_that_decides() {
        let mut polls = 0;
        let decision = decide(
            || {
                polls += 1;
                Ok((polls > 3).then(|| (1, document(false))))
            },
            || Ok(Some(true)),
            Duration::from_secs(5),
            POLL,
        );
        assert!(!decision.enabled);
        assert_eq!(decision.source, KeyringSource::Accepted { revision: 1 });
        assert_eq!(polls, 4, "the agent kept asking until the configuration arrived");
    }

    #[test]
    fn a_timeout_falls_back_to_the_last_accepted_file_and_says_so() {
        let decision = decide(|| Ok(None), || Ok(Some(false)), WAIT, POLL);
        assert!(!decision.enabled, "the last accepted file says off");
        assert_eq!(decision.source, KeyringSource::LastAccepted);
        assert!(
            decision.reason.contains("no accepted configuration within"),
            "{}",
            decision.reason
        );
        assert!(
            decision.reason.contains("the last accepted file decides"),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn a_timeout_with_no_last_accepted_file_falls_back_to_the_schema_default_and_says_so() {
        let decision = decide(|| Ok(None), || Ok(None), WAIT, POLL);
        assert!(decision.enabled);
        assert_eq!(decision.source, KeyringSource::Default);
        assert!(
            decision.reason.contains("the schema default decides"),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn a_timeout_with_an_unreadable_last_accepted_file_falls_back_to_the_default_and_says_so() {
        let decision = decide(|| Ok(None), || Err("configuration not readable".to_owned()), WAIT, POLL);
        assert!(decision.enabled);
        assert_eq!(decision.source, KeyringSource::Default);
        assert!(
            decision.reason.contains("configuration not readable"),
            "{}",
            decision.reason
        );
    }

    #[test]
    fn a_compositor_read_error_is_kept_as_the_reason_and_polling_continues() {
        let mut polls = 0;
        let decision = decide(
            || {
                polls += 1;
                if polls < 3 {
                    Err("connection refused".to_owned())
                } else {
                    Ok(Some((1, document(false))))
                }
            },
            || Ok(None),
            Duration::from_secs(5),
            POLL,
        );
        assert!(!decision.enabled);
        assert_eq!(decision.source, KeyringSource::Accepted { revision: 1 });
    }

    #[test]
    fn a_timeout_after_a_read_error_names_the_error() {
        let decision = decide(|| Err("connection refused".to_owned()), || Ok(None), WAIT, POLL);
        assert!(decision.reason.contains("connection refused"), "{}", decision.reason);
    }

    #[test]
    fn an_accepted_document_that_is_not_a_configuration_falls_back_at_once() {
        let polls = Cell::new(0);
        let decision = decide(
            || {
                polls.set(polls.get() + 1);
                Ok(Some((1, b"not json".to_vec())))
            },
            || Ok(Some(false)),
            Duration::from_secs(5),
            POLL,
        );
        assert!(!decision.enabled);
        assert_eq!(decision.source, KeyringSource::LastAccepted);
        assert!(decision.reason.contains("is unreadable"), "{}", decision.reason);
        assert_eq!(polls.get(), 1, "an unreadable document does not wait for another");
    }
}
