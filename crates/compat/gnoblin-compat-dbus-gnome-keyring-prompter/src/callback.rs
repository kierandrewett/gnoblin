//! The caller's callback object: `org.gnome.keyring.internal.Prompter.Callback`.
//!
//! A gcr caller exports an object at a path of its choosing and names that path in
//! `BeginPrompting`. The prompter answers on it: `PromptReady` when the user decided
//! something and `PromptDone` when the prompt is over without an answer.
//!
//! The methods carry no return value and the prompter treats a failed call as the
//! caller having gone: a `GcrSystemPrompt` that closed between the offer and the
//! answer is not an error to report, it is a prompt nobody is waiting for.

use std::collections::HashMap;

use zbus::OwnedVariant;

/// The interface name gcr's callers export.
pub const INTERFACE: &str = "org.gnome.keyring.internal.Prompter.Callback";

/// The two calls a prompter makes on a caller's object.
///
/// `PromptReady`'s arguments are the response (`yes`, `no`, or nothing before the
/// user has decided), the changed prompt properties as `a{sv}`, and the secret
/// exchange string. This adapter changes no properties, so the map is always empty
/// and the exchange carries the passphrase.
#[zbus::proxy(interface = "org.gnome.keyring.internal.Prompter.Callback")]
pub trait PromptCallback {
    /// Tell the caller the prompt is ready, or answer it.
    fn prompt_ready(
        &self,
        response: &str,
        properties: HashMap<String, OwnedVariant>,
        exchange: &str,
    ) -> zbus::Result<()>;

    /// Tell the caller the prompt is over without an answer.
    fn prompt_done(&self) -> zbus::Result<()>;
}
