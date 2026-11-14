//! The polkit request as the agent hands it to the compositor, and the part of it an observer may see.
//!
//! The services role's agent holds the D-Bus object, so it is the one that hears `BeginAuthentication`. The
//! compositor draws the dialog and runs the helper, so it needs the action, the message, the identities the user
//! may authenticate as and the cookie the helper names the authentication by. All of it crosses the existing
//! offer as one JSON document in the offer's subject: the offer carries the bytes unread, and this module is the
//! one place that writes and reads them.
//!
//! An observer (a script that watches the compositor's dialogs, and may only refuse) is shown
//! [`AuthRequest::observed`] and never the cookie.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::user;

/// The largest payload [`AuthRequest::decode`] reads, in bytes. polkitd's own messages are a few hundred bytes; a
/// payload past this is not one the agent wrote.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// Who polkitd lets authenticate. polkitd expands a group into its users before it asks the agent, so a
/// `unix-user` is the only kind a person can be asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Identity {
    /// A user, by uid.
    UnixUser {
        /// The user's uid.
        uid: u32,
    },
    /// An identity kind the helper cannot authenticate (`unix-group`, `unix-netgroup`, ...). It is shown and never
    /// offered as a choice.
    Other {
        /// polkit's name for the kind.
        kind: String,
    },
}

impl Identity {
    /// The text a person reads for this identity: the user's name, or `uid 1000` when no account has that uid.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Identity::UnixUser { uid } => user::name_of(*uid).unwrap_or_else(|| format!("uid {uid}")),
            Identity::Other { kind } => kind.clone(),
        }
    }
}

/// What one `BeginAuthentication` asks, with the cookie that names the authentication to the helper.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    /// polkitd's cookie. The helper reads it on its standard input, and `CancelAuthentication` names it.
    pub cookie: String,
    /// The action being authorized, such as `org.freedesktop.policykit.exec`.
    pub action_id: String,
    /// The sentence polkitd wants shown, in the agent's locale.
    pub message: String,
    /// The icon name polkitd suggested, or the empty string.
    pub icon_name: String,
    /// The action's details, in the agent's locale.
    pub details: BTreeMap<String, String>,
    /// Who may authenticate.
    pub identities: Vec<Identity>,
    /// The program asking, as the agent resolved it from the details, or the empty string.
    pub requester: String,
}

impl fmt::Debug for AuthRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The cookie is the one value that lets a caller of the helper name this authentication.
        formatter
            .debug_struct("AuthRequest")
            .field("action_id", &self.action_id)
            .field("requester", &self.requester)
            .field("identities", &self.identities.len())
            .finish_non_exhaustive()
    }
}

/// Why a payload was refused.
#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    /// The bytes are larger than [`MAX_PAYLOAD`].
    #[error("the authentication request is {0} bytes, more than the {MAX_PAYLOAD} the compositor reads")]
    TooLarge(usize),
    /// The bytes are not an authentication request.
    #[error("the offer does not carry an authentication request: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// What an observer is shown of a request: everything a policy needs to decide, and nothing the helper needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    /// The action being authorized.
    #[serde(rename = "actionId")]
    pub action_id: String,
    /// The program asking, or the empty string.
    pub requester: String,
    /// Who may authenticate, as polkit writes it: `unix-user:alice`.
    pub identities: Vec<String>,
}

impl AuthRequest {
    /// A request for the details polkitd sent, with the requesting program resolved.
    #[must_use]
    pub fn new(
        cookie: String,
        action_id: String,
        message: String,
        icon_name: String,
        details: BTreeMap<String, String>,
        identities: Vec<Identity>,
    ) -> Self {
        let requester = requester_of(&details);
        Self {
            cookie,
            action_id,
            message,
            icon_name,
            details,
            identities,
            requester,
        }
    }

    /// The bytes the agent puts in the offer's subject.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        // A struct of strings and string maps always serialises; an empty payload, were it ever returned, is
        // refused by the compositor as malformed, which fails the request closed.
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Reads the bytes [`AuthRequest::encode`] wrote.
    ///
    /// # Errors
    ///
    /// [`PayloadError`] when the bytes are too many or are not a request.
    pub fn decode(bytes: &[u8]) -> Result<Self, PayloadError> {
        if bytes.len() > MAX_PAYLOAD {
            return Err(PayloadError::TooLarge(bytes.len()));
        }
        Ok(serde_json::from_slice(bytes)?)
    }

    /// The request as an observer sees it, as the JSON an offer's subject carries.
    #[must_use]
    pub fn observed(&self) -> Vec<u8> {
        let observed = Observed {
            action_id: self.action_id.clone(),
            requester: self.requester.clone(),
            identities: self
                .identities
                .iter()
                .map(|identity| match identity {
                    Identity::UnixUser { uid } => {
                        format!("unix-user:{}", user::name_of(*uid).unwrap_or_else(|| uid.to_string()))
                    }
                    Identity::Other { kind } => kind.clone(),
                })
                .collect(),
        };
        serde_json::to_vec(&observed).unwrap_or_default()
    }
}

/// The program polkitd says is asking: the `program` detail pkexec sets, else the name of the process in
/// `polkit.subject-pid`, else the one in `polkit.caller-pid`.
fn requester_of(details: &BTreeMap<String, String>) -> String {
    if let Some(program) = details.get("program").filter(|program| !program.is_empty()) {
        return program.rsplit('/').next().unwrap_or(program).to_owned();
    }
    ["polkit.subject-pid", "polkit.caller-pid"]
        .into_iter()
        .filter_map(|key| details.get(key))
        .filter_map(|pid| pid.parse::<u32>().ok())
        .find_map(user::process_name)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AuthRequest {
        let mut details = BTreeMap::new();
        details.insert("program".to_owned(), "/usr/bin/pkexec".to_owned());
        AuthRequest::new(
            "3-abc-def".to_owned(),
            "org.freedesktop.policykit.exec".to_owned(),
            "Authentication is needed to run `/usr/bin/true' as the super user".to_owned(),
            String::new(),
            details,
            vec![
                Identity::UnixUser { uid: 0 },
                Identity::Other {
                    kind: "unix-netgroup".to_owned(),
                },
            ],
        )
    }

    #[test]
    fn a_request_survives_the_offer() {
        let sent = request();
        assert_eq!(AuthRequest::decode(&sent.encode()).expect("it decodes"), sent);
    }

    #[test]
    fn the_requester_is_the_program_the_details_name() {
        assert_eq!(request().requester, "pkexec");
    }

    #[test]
    fn an_observer_never_sees_the_cookie() {
        let observed = String::from_utf8(request().observed()).expect("it is text");
        assert!(!observed.contains("3-abc-def"), "{observed}");
        assert!(observed.contains("org.freedesktop.policykit.exec"), "{observed}");
        assert!(observed.contains("pkexec"), "{observed}");
    }

    #[test]
    fn debug_leaves_the_cookie_out() {
        assert!(!format!("{:?}", request()).contains("3-abc-def"));
    }

    #[test]
    fn garbage_and_oversized_payloads_are_refused() {
        assert!(matches!(
            AuthRequest::decode(b"not json"),
            Err(PayloadError::Malformed(_))
        ));
        assert!(matches!(AuthRequest::decode(b""), Err(PayloadError::Malformed(_))));
        assert!(matches!(
            AuthRequest::decode(&vec![b' '; MAX_PAYLOAD + 1]),
            Err(PayloadError::TooLarge(_))
        ));
    }
}
