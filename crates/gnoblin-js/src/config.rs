//! The configuration document, from the shell's side: what `defineConfig` writes to it and how a reload takes it back.
//!
//! `defineConfig` keys that are document settings (`dialogs`, `appearance`, `cursor`, ...) are one patch of the
//! document the compositor holds. The runtime role's Lua host commits the document it evaluated; the shell reads
//! that document back (`config.current`), lays its patch over it and commits the whole result (`config.commit`),
//! the same two verbs the Lua host uses. The compositor holds only the last accepted document, so a commit must
//! carry everything.
//!
//! **Why the full document and not a patch.** A reload that drops a key must put the setting back to its default,
//! and the compositor re-derives its settings from each document it accepts (a section a document leaves out is
//! the default). So the shell remembers the key paths its last committed patch set ([`KeyPath`]), and the next
//! commit is built from the current document with those paths taken out and the new patch laid over it. A key the
//! new generation no longer sets is gone from the document; a section the removal leaves empty goes with it. A
//! setting the Lua host's own file wrote is not the shell's to take back, because only the paths the shell set are
//! removed.
//!
//! **Validation.** The same validator the compositor and the Lua host use ([`Config::from_value`]) runs on the
//! candidate document, so a bad value is refused with its dotted path (`dialogs.style.radius: ...`). The check
//! runs when `defineConfig` is called, which is while a reload is still staging: a refused value fails the stage,
//! the running generation stays, and the error is in the reload report. The commit itself happens once the
//! generation is swapped in.

use std::path::Path;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_config::{Config, Value};
use gnoblin_proto::Version;
use rquickjs::{Ctx, Function, Object};
use thiserror::Error;

use crate::bindings::SharedState;

/// How long a config call waits for the compositor's reply. It answers a commit at once, so a later reply is a
/// compositor that is stuck.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// A key of the document, from the section down: `["dialogs", "style", "radius"]`.
pub(crate) type KeyPath = Vec<String>;

/// A config call that failed. The compositor's refusal text, or why the connection is gone.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct JsLinkError(pub String);

/// What the compositor accepted from a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsConfigCommit {
    /// The revision now in force.
    pub revision: u64,
    /// The settings the compositor accepted without applying, one sentence each.
    pub notices: Vec<String>,
}

/// The shell's connection for the configuration document: the owner socket's `config.current` and
/// `config.commit`.
pub trait JsConfigLink {
    /// The last document the compositor accepted, as JSON, or `None` when it has accepted none.
    fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError>;
    /// Commits `document` (JSON), which replaces the one in force.
    fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError>;
}

/// [`JsConfigLink`] over a handshaken owner connection.
pub struct JsClientConfigLink {
    client: Client<UnixTransport>,
}

impl JsClientConfigLink {
    /// Connects to the owner socket at `path` and completes the handshake.
    pub fn connect(path: &Path, version: Version) -> Result<Self, JsLinkError> {
        let transport = UnixTransport::connect(path)
            .map_err(|error| JsLinkError(format!("cannot connect to {}: {error}", path.display())))?;
        let mut client = Client::connect(transport, version).map_err(|error| JsLinkError(error.to_string()))?;
        client
            .set_read_timeout(Some(REPLY_WAIT))
            .map_err(|error| JsLinkError(error.to_string()))?;
        Ok(Self { client })
    }
}

impl JsConfigLink for JsClientConfigLink {
    fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError> {
        self.client
            .configuration()
            .map(|current| current.map(|(_, document)| document))
            .map_err(|error| JsLinkError(error.to_string()))
    }

    fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError> {
        let committed = self
            .client
            .commit_config_reporting(document)
            .map_err(|error| JsLinkError(error.to_string()))?;
        Ok(JsConfigCommit {
            revision: committed.revision,
            notices: committed.notices.into_iter().map(|notice| notice.text).collect(),
        })
    }
}

/// How an application of the patch ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    /// The document in force already says what the patch and the removals make it say; nothing was committed.
    Unchanged,
    /// The document was committed.
    Committed(JsConfigCommit),
}

/// The shell's hold on the document: its link, and what the committed generation set.
pub(crate) struct ConfigState {
    link: Option<Box<dyn JsConfigLink>>,
    /// The key paths the last committed patch set, so the next commit can take them back.
    owned: Vec<KeyPath>,
}

/// A document with no keys.
pub(crate) fn empty() -> Value {
    Value::Map(std::collections::BTreeMap::new())
}

impl ConfigState {
    pub(crate) fn new() -> Self {
        Self {
            link: None,
            owned: Vec::new(),
        }
    }

    pub(crate) fn set_link(&mut self, link: Box<dyn JsConfigLink>) {
        self.link = Some(link);
    }

    /// Whether `patch` would be refused. `Err` is the validator's problems, each as `path: reason`, joined; it is
    /// what `defineConfig` throws, so a reload stage fails with the key path in the report.
    ///
    /// The candidate is what a commit would send, built from the document in force.
    pub(crate) fn check(&mut self, patch: &Value, retract: bool) -> Result<(), String> {
        self.validated(patch, retract).map(drop)
    }

    /// The document in force and the document `patch` would make of it, the second one validated.
    fn validated(&mut self, patch: &Value, retract: bool) -> Result<(Value, Value), String> {
        let current = self.read_current()?;
        let candidate = self.candidate(current.clone(), patch, retract);
        match Config::from_value(candidate) {
            Ok(config) => Ok((current, config.document().clone())),
            Err(report) => Err(report
                .problems()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")),
        }
    }

    /// Lays `patch` over the document in force (after taking back what the last patch set, when `retract`) and
    /// commits the result. Nothing is sent when the result is what is already in force.
    ///
    /// # Errors
    ///
    /// The message of a refusal: the validator's problems, or the compositor's reason. The document in force
    /// and what the shell remembers of it are unchanged.
    pub(crate) fn apply(&mut self, patch: &Value, retract: bool) -> Result<Applied, String> {
        let owned = leaves(patch);
        // A shell with no connection to the document and nothing to say about it has nothing to commit.
        if self.link.is_none() && owned.is_empty() {
            return Ok(Applied::Unchanged);
        }
        let (current, document) = self.validated(patch, retract)?;
        if current == document {
            self.owned = owned;
            return Ok(Applied::Unchanged);
        }
        let Some(link) = self.link.as_mut() else {
            return Err("the shell has no configuration connection".to_owned());
        };
        let bytes = document.to_json().to_string().into_bytes();
        let committed = link.commit(&bytes).map_err(|error| error.0)?;
        self.owned = owned;
        Ok(Applied::Committed(committed))
    }

    /// The document in force, as a tree. An empty one when nothing is committed or there is no link.
    fn read_current(&mut self) -> Result<Value, String> {
        let Some(link) = self.link.as_mut() else {
            return Ok(empty());
        };
        let Some(bytes) = link.current().map_err(|error| error.0)? else {
            return Ok(empty());
        };
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| format!("the document in force is not JSON: {error}"))?;
        match Value::from_json(&json) {
            Some(document @ Value::Map(_)) => Ok(document),
            _ => Err("the document in force is not a configuration tree".to_owned()),
        }
    }

    fn candidate(&self, mut document: Value, patch: &Value, retract: bool) -> Value {
        if retract {
            for path in &self.owned {
                remove(&mut document, path);
            }
        }
        overlay(&mut document, patch);
        document
    }
}

/// A document patch as the script hands it over: JSON text of nested tables.
pub(crate) fn parse_patch(text: &str) -> Result<Value, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("the document patch is not JSON: {error}"))?;
    match Value::from_json(&json) {
        Some(patch @ Value::Map(_)) => Ok(patch),
        _ => Err("the document patch is not a table of sections".to_owned()),
    }
}

/// Installs `__gnoblinHost.config`: `check(patch)` answers the validator's problems for a patch laid over the
/// document in force, or nothing when the document would be accepted.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let config = Object::new(ctx.clone())?;
    let shared = state.clone();
    config.set(
        "check",
        Function::new(ctx.clone(), move |patch: String| -> Option<String> {
            match parse_patch(&patch) {
                Ok(patch) => shared.borrow_mut().config.check(&patch, true).err(),
                Err(problem) => Some(problem),
            }
        })?,
    )?;
    host.set("config", config)
}

/// Every key path `patch` sets: each scalar and each list (a list is one value), and no empty table.
pub(crate) fn leaves(patch: &Value) -> Vec<KeyPath> {
    fn walk(value: &Value, path: &mut KeyPath, out: &mut Vec<KeyPath>) {
        match value {
            Value::Map(entries) => {
                for (key, value) in entries {
                    path.push(key.clone());
                    walk(value, path, out);
                    path.pop();
                }
            }
            _ if path.is_empty() => {}
            _ => out.push(path.clone()),
        }
    }
    let mut out = Vec::new();
    walk(patch, &mut Vec::new(), &mut out);
    out
}

/// Sets every leaf of `patch` in `document`; the tables between are made when they are not there.
pub(crate) fn overlay(document: &mut Value, patch: &Value) {
    let (Value::Map(target), Value::Map(entries)) = (document, patch) else {
        return;
    };
    for (key, value) in entries {
        match value {
            Value::Map(inner) if inner.is_empty() => {}
            Value::Map(_) => {
                let slot = target.entry(key.clone()).or_insert_with(empty);
                if !matches!(slot, Value::Map(_)) {
                    *slot = empty();
                }
                overlay(slot, value);
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Takes `path` out of `document`, and any table that is left empty above it.
pub(crate) fn remove(document: &mut Value, path: &[String]) {
    let (Some((first, rest)), Value::Map(entries)) = (path.split_first(), document) else {
        return;
    };
    if rest.is_empty() {
        entries.remove(first);
        return;
    }
    let emptied = match entries.get_mut(first) {
        Some(inner @ Value::Map(_)) => {
            remove(inner, rest);
            matches!(inner, Value::Map(entries) if entries.is_empty())
        }
        _ => false,
    };
    if emptied {
        entries.remove(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        let parsed: serde_json::Value = serde_json::from_str(text).expect("test json");
        Value::from_json(&parsed).expect("a configuration tree")
    }

    #[test]
    fn leaves_are_scalars_and_lists_and_never_empty_tables() {
        let patch = json(r#"{"dialogs":{"style":{"radius":12,"font":{"size":14}}},"input":{"keyboard":{}},"x":[1,2]}"#);
        let mut found = leaves(&patch);
        found.sort();
        assert_eq!(
            found,
            vec![
                vec![
                    "dialogs".to_owned(),
                    "style".to_owned(),
                    "font".to_owned(),
                    "size".to_owned()
                ],
                vec!["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
                vec!["x".to_owned()],
            ]
        );
    }

    #[test]
    fn overlay_keeps_what_the_patch_does_not_name() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":3,"scale":2}},"cursor":{"size":24}}"#);
        overlay(&mut document, &json(r#"{"dialogs":{"style":{"radius":12}}}"#));
        assert_eq!(
            document,
            json(r#"{"dialogs":{"style":{"radius":12,"scale":2}},"cursor":{"size":24}}"#)
        );
    }

    #[test]
    fn removing_a_path_prunes_the_tables_it_leaves_empty_and_no_others() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":12}},"cursor":{"size":24}}"#);
        remove(
            &mut document,
            &["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
        );
        assert_eq!(document, json(r#"{"cursor":{"size":24}}"#));
        // A path that is not there changes nothing.
        remove(&mut document, &["dialogs".to_owned(), "style".to_owned()]);
        assert_eq!(document, json(r#"{"cursor":{"size":24}}"#));
    }

    #[test]
    fn a_table_the_removal_does_not_empty_stays() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":12,"scale":2}}}"#);
        remove(
            &mut document,
            &["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
        );
        assert_eq!(document, json(r#"{"dialogs":{"style":{"scale":2}}}"#));
    }
}
