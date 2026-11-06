//! `org.gnome.keyring.SystemPrompter` and `PrivatePrompter` for gcr callers.
//!
//! This is `gnoblin-compat-gnome-keyring-prompter` of `design/rust-compositor.md`
//! ("Providers and slots" and "Compatibility crates"): the adapter that lets
//! `gnome-keyring`, `pinentry-gnome3` and anything else built on gcr ask Gnoblin for
//! a passphrase. Each prompt becomes a `prompt.secret` request offered to the
//! compositor, a shell claims it, and the answer comes back encrypted under the
//! caller's own key.
//!
//! # What it is not
//!
//! - **Not a keyring.** Gnoblin stores no secrets and depends on none. This adapter
//!   shows a prompt and returns what the user typed; the program that asked decides
//!   what to do with it.
//! - **Not libgcr.** The interface is implemented directly, so the adapter does not
//!   link GObject, GLib or gcr. The one part of gcr that has to be reproduced is the
//!   secret exchange, which is protocol rather than library: [`exchange`] says what it
//!   is and which parts a reimplementation usually gets wrong.
//! - **Not the provider.** `prompt.secret` has no default provider: without a shell
//!   that claimed it, every prompt fails closed with an error naming the slot, which
//!   is the plan's D14 answer for a session with no shell.
//!
//! # Pieces
//!
//! - [`prompter`] serves `org.gnome.keyring.internal.Prompter` and [`callback`] is the
//!   half it calls on the caller's object. Each interface is one module.
//! - [`state`] is the state machine: begun callbacks, offered requests, tombstones and
//!   the effects the bus still has to be told. It knows nothing about D-Bus.
//! - [`exchange`] is the gcr secret exchange, and the only place a passphrase exists
//!   as bytes.
//! - [`compat`] is the frame: the checked-in support matrix and the mode gate.
//!
//! # Running it
//!
//! ```no_run
//! use gnoblin_compat_gnome_keyring_prompter::{Names, PromptService, export, run};
//! use gnoblin_compat_core::Gate;
//! use gnoblin_model::Tick;
//! use zbus::Connection;
//!
//! # async fn run_it(host: impl gnoblin_compat_gnome_keyring_prompter::Offerer + Send + 'static) -> Result<(), Box<dyn std::error::Error>> {
//! let connection = Connection::session().await?;
//! let service = PromptService::new(host, Gate::default(), Tick::new(120_000))?;
//! export(&connection, &service, Names::Both).await?;
//! run(connection.clone(), service).await;
//! # Ok(())
//! # }
//! ```
//!
//! # Failure modes first
//!
//! 1. **Another process owns the name.** `gcr-prompter` running beside this adapter
//!    is the ordinary case, and the adapter refuses to start rather than half-serving
//!    one name: `a_stolen_name_is_refused`.
//! 2. **The adapter registers twice.** A second export in one process is refused, not
//!    silently a second object server entry: `an_adapter_registers_once`.
//! 3. **A member the matrix does not declare is called.** Refused with the frame's
//!    rule, before the handler runs: `an_undeclared_member_is_refused`.
//! 4. **The adapters are off.** `Mode::Off` refuses every call:
//!    `the_mode_gate_refuses_a_disabled_adapter`.
//! 5. **A prompt is answered but the caller gets nothing.** The answer travels on the
//!    callback in the encrypted exchange, which the test decrypts as a gcr caller
//!    would: `a_prompt_is_offered_answered_and_encrypted`.

#![forbid(unsafe_code)]

/// The `Prompter.Callback` object that carries each prompt's answer back to the caller.
pub mod callback;
/// Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
pub mod compat;
/// The gcr secret exchange, `sx-aes-1`, implemented here without libgcr.
pub mod exchange;
/// The `org.gnome.keyring.internal.Prompter` service that answers gcr's prompt requests.
pub mod prompter;
/// The prompt state machine: one gcr prompt, one request, one answer.
pub mod state;

/// The proxy that calls back into the prompt requester with the answer.
pub use callback::PromptCallbackProxy;
/// Admission check and the call it gates, as the prompter uses them.
pub use compat::{Admission, Call};
/// The secret exchange: its error type and the secret it carries.
pub use exchange::{Exchange, ExchangeError, Secret};
/// The bus names, interface and object path the prompter serves on.
pub use prompter::{PRIVATE_NAME, PROMPTER_INTERFACE, PROMPTER_PATH, PromptService, SYSTEM_NAME};
/// The prompt state types: pending prompts, their effects and their outcomes.
pub use state::{Callback, Decision, Effect, EndedPrompt, Offerer, Pending, Prompt, PromptKind, Prompter, Unavailable};

use futures_lite::StreamExt as _;
use zbus::Connection;
use zbus::fdo::RequestNameFlags;

/// Which of gcr's two bus names this adapter serves.
///
/// A session normally serves both, which is what GNOME's `gcr-prompter` does. A
/// distribution that only needs the system prompter can own one, and the plan's rule
/// 5 ("optional per name") is why this is a value rather than two `bool`s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Names {
    /// `org.gnome.keyring.SystemPrompter` only.
    System,
    /// `org.gnome.keyring.PrivatePrompter` only.
    Private,
    /// Both names.
    Both,
}

impl Names {
    /// The names this choice owns.
    pub const fn as_slice(self) -> &'static [&'static str] {
        match self {
            Names::System => &[SYSTEM_NAME],
            Names::Private => &[PRIVATE_NAME],
            Names::Both => &[SYSTEM_NAME, PRIVATE_NAME],
        }
    }
}

/// Export the prompter object and own the chosen names.
///
/// The object is exported before the names are requested, because a caller that
/// resolves a name must find an object behind it: a name without an object is a
/// method call that never gets a reply, which is worse than a name that was never
/// taken.
///
/// A name another process owns is refused with [`Unavailable::NameUnavailable`],
/// which is the `name_unavailable` state `docs/config/configure/prompts.md` documents:
/// a session cannot serve prompts from two prompters at once, and the one that lost
/// says so instead of queueing behind the winner.
pub async fn export(connection: &Connection, service: &PromptService, names: Names) -> Result<(), Unavailable> {
    service.take_export()?;
    connection
        .object_server()
        .at(PROMPTER_PATH, service.clone())
        .await
        .map_err(|error| Unavailable::Bus {
            reason: error.to_string(),
        })?;
    for name in names.as_slice() {
        connection
            .request_name_with_flags(*name, RequestNameFlags::DO_NOT_QUEUE)
            .await
            .map_err(|_| Unavailable::NameUnavailable {
                name: (*name).to_owned(),
            })?;
    }
    gnoblin_log::info_in!("compat-keyring", names = ?names.as_slice(), "serving gcr prompts");
    Ok(())
}

/// Answer the caller of every prompt until the connection's service loop ends.
///
/// This is the loop [`export`]'s counterpart needs: the state machine queues the
/// `PromptReady` and `PromptDone` calls it cannot make itself (the ordering reason is
/// in [`state`]), and this performs them on the same connection. It ends when the
/// service's effects channel does, which is when the service is dropped.
pub async fn run(connection: Connection, service: PromptService) {
    let Some(mut effects) = service.take_effects() else {
        return;
    };
    while let Some(effect) = effects.next().await {
        if let Err(error) = prompter::perform(&connection, effect).await {
            // The caller has closed its prompt object, or the bus is gone. Either way
            // there is nobody left to answer, and the request itself was already
            // finished.
            gnoblin_log::debug_in!("compat-keyring", %error, "a prompt callback could not be delivered");
        }
    }
}
