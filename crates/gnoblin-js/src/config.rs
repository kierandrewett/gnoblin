//! The configuration document, from the shell's side: what `defineConfig` writes to it and how a reload takes it back.
//!
//! `defineConfig` keys that are document settings (`dialogs`, `appearance`, `cursor`, ...) are one patch of the
//! document the compositor holds. The runtime role's retired host commits the document it evaluated; the shell reads
//! that document back (`config.current`), lays its patch over it and commits the whole result (`config.commit`),
//! the same two verbs the retired host uses. The compositor holds only the last accepted document, so a commit must
//! carry everything.
//!
//! **Why the full document and not a patch.** A reload that drops a key must put the setting back to its default,
//! and the compositor re-derives its settings from each document it accepts (a section a document leaves out is
//! the default). So the shell remembers the key paths its last committed patch set ([`KeyPath`]), and the next
//! commit is built from the current document with those paths taken out and the new patch laid over it. A key the
//! new generation no longer sets is gone from the document; a section the removal leaves empty goes with it. A
//! setting the retired host's own file wrote is not the shell's to take back, because only the paths the shell set are
//! removed.
//!
//! **Validation.** The same validator the compositor and the retired host use ([`Config::from_value`]) runs on the
//! candidate document, so a bad value is refused with its dotted path (`dialogs.style.radius: ...`). The check
//! runs when `defineConfig` is called, which is while a reload is still staging: a refused value fails the stage,
//! the running generation stays, and the error is in the reload report. The commit itself happens once the
//! generation is swapped in.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
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

/// One commit the shell made, as the compositor accepted it. The runtime role reads these to tell the guardian
/// which revision is in force and which autostart entries it enables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsCommitRecord {
    /// The revision now in force.
    pub revision: u64,
    /// The settings the compositor accepted without applying, one sentence each.
    pub notices: Vec<String>,
    /// The document that was committed, as JSON.
    pub document: Vec<u8>,
}

/// The commits a shell has made and its owner has not yet read, oldest first.
pub type JsCommitLog = Rc<RefCell<Vec<JsCommitRecord>>>;

/// A [`JsConfigLink`] that records every commit its inner link accepted in a [`JsCommitLog`].
pub(crate) struct JsRecordingLink {
    inner: Box<dyn JsConfigLink>,
    log: JsCommitLog,
}

impl JsRecordingLink {
    pub(crate) fn new(inner: Box<dyn JsConfigLink>, log: JsCommitLog) -> Self {
        Self { inner, log }
    }
}

impl JsConfigLink for JsRecordingLink {
    fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError> {
        self.inner.current()
    }

    fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError> {
        let committed = self.inner.commit(document)?;
        self.log.borrow_mut().push(JsCommitRecord {
            revision: committed.revision,
            notices: committed.notices.clone(),
            document: document.to_vec(),
        });
        Ok(committed)
    }
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
pub(crate) enum JsApplied {
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
    /// Whether the shell owns the whole document: the JavaScript runtime role, where no retired host commits a file of
    /// its own. Every commit is then the patch alone, laid over an empty document, and is always sent: a reload
    /// that changed nothing is still the answer the compositor is waiting for, and a role that restarts must not
    /// inherit keys the previous run committed.
    owns_document: bool,
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
            owns_document: false,
        }
    }

    /// Makes the shell the owner of the whole document. See [`ConfigState::owns_document`].
    pub(crate) fn set_owns_document(&mut self, owns: bool) {
        self.owns_document = owns;
    }

    /// Whether the shell owns the whole document.
    pub(crate) fn owns_document(&self) -> bool {
        self.owns_document
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
        // The document the shell owns is built from nothing, so what the compositor holds is not read.
        let current = if self.owns_document {
            empty()
        } else {
            self.read_current()?
        };
        let candidate = self.candidate(current.clone(), patch, retract);
        match Config::from_value(candidate) {
            Ok(config) => {
                // The guardian starts the document's autostart entries, so one that would start a second
                // keyring is refused here, before the commit, as the retired host refuses it.
                if self.owns_document {
                    gnoblin_config::enabled_autostart(&config)?;
                }
                Ok((current, config.document().clone()))
            }
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
    pub(crate) fn apply(&mut self, patch: &Value, retract: bool) -> Result<JsApplied, String> {
        let owned = leaves(patch);
        // A shell with no connection to the document and nothing to say about it has nothing to commit.
        if self.link.is_none() && owned.is_empty() {
            return Ok(JsApplied::Unchanged);
        }
        let (current, document) = self.validated(patch, retract)?;
        if current == document && !self.owns_document {
            self.owned = owned;
            return Ok(JsApplied::Unchanged);
        }
        let Some(link) = self.link.as_mut() else {
            return Err("the shell has no configuration connection".to_owned());
        };
        let bytes = document.to_json().to_string().into_bytes();
        let committed = link.commit(&bytes).map_err(|error| error.0)?;
        self.owned = owned;
        Ok(JsApplied::Committed(committed))
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
        if self.owns_document {
            document = empty();
        } else if retract {
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

/// Installs `__gnoblinHost.config`: `check(patch, autostart)` answers the validator's problems for a patch laid over
/// the document in force, or nothing when the document would be accepted. `autostart` is the config's commands as
/// JSON; a shell that owns the document commits them as its `autostart` section, so they are checked with it.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let config = Object::new(ctx.clone())?;
    let shared = state.clone();
    config.set(
        "check",
        Function::new(ctx.clone(), move |patch: String, autostart: String| -> Option<String> {
            let mut patch = match parse_patch(&patch) {
                Ok(patch) => patch,
                Err(problem) => return Some(problem),
            };
            let mut state = shared.borrow_mut();
            if state.config.owns_document() {
                match crate::autostart::parse(&autostart) {
                    Ok(commands) => crate::autostart::lay_over(&mut patch, &commands),
                    Err(problem) => return Some(problem.to_string()),
                }
            }
            state.config.check(&patch, true).err()
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

    use std::collections::VecDeque;

    /// A document link that holds `current` and answers each commit with the next revision, remembering what it
    /// was sent.
    struct FakeLink {
        current: Option<Vec<u8>>,
        sent: Rc<RefCell<Vec<Vec<u8>>>>,
        revisions: VecDeque<u64>,
    }

    impl FakeLink {
        fn new(current: Option<&str>) -> (Self, Rc<RefCell<Vec<Vec<u8>>>>) {
            let sent = Rc::new(RefCell::new(Vec::new()));
            let link = Self {
                current: current.map(|text| text.as_bytes().to_vec()),
                sent: Rc::clone(&sent),
                revisions: (1..).take(8).collect(),
            };
            (link, sent)
        }
    }

    impl JsConfigLink for FakeLink {
        fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError> {
            Ok(self.current.clone())
        }

        fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError> {
            self.sent.borrow_mut().push(document.to_vec());
            self.current = Some(document.to_vec());
            Ok(JsConfigCommit {
                revision: self.revisions.pop_front().unwrap_or(99),
                notices: Vec::new(),
            })
        }
    }

    fn state_over(link: FakeLink, owns: bool) -> ConfigState {
        let mut state = ConfigState::new();
        state.set_link(Box::new(link));
        state.set_owns_document(owns);
        state
    }

    fn sent_json(sent: &Rc<RefCell<Vec<Vec<u8>>>>, index: usize) -> serde_json::Value {
        serde_json::from_slice(&sent.borrow()[index]).expect("a committed document is JSON")
    }

    #[test]
    fn an_owned_document_is_the_patch_alone_even_when_the_compositor_holds_other_keys() {
        // A runtime that restarts finds the document its predecessor committed in force. Keys the new config no
        // longer sets must not survive it.
        let (link, sent) = FakeLink::new(Some(r#"{"cursor":{"size":24},"keyring":{"enabled":false}}"#));
        let mut state = state_over(link, true);
        let applied = state
            .apply(&json(r#"{"cursor":{"size":32}}"#), true)
            .expect("the patch is accepted");
        assert!(matches!(applied, JsApplied::Committed(_)));
        assert_eq!(sent_json(&sent, 0), serde_json::json!({"cursor": {"size": 32}}));
    }

    #[test]
    fn an_owned_document_is_committed_even_when_nothing_changed() {
        // The compositor holds a `config.reload` open until some commit answers it.
        let (link, sent) = FakeLink::new(None);
        let mut state = state_over(link, true);
        for _ in 0..2 {
            let applied = state.apply(&json(r#"{"cursor":{"size":32}}"#), true).expect("accepted");
            assert!(matches!(applied, JsApplied::Committed(_)), "{applied:?}");
        }
        assert_eq!(sent.borrow().len(), 2);
    }

    #[test]
    fn an_owned_empty_config_commits_the_empty_document() {
        let (link, sent) = FakeLink::new(None);
        let mut state = state_over(link, true);
        let applied = state.apply(&empty(), true).expect("an empty document is valid");
        assert!(matches!(applied, JsApplied::Committed(_)));
        assert_eq!(sent_json(&sent, 0), serde_json::json!({}));
    }

    #[test]
    fn a_shared_document_keeps_what_the_compositor_holds_and_skips_an_unchanged_commit() {
        let (link, sent) = FakeLink::new(Some(r#"{"cursor":{"size":24}}"#));
        let mut state = state_over(link, false);
        state
            .apply(&json(r#"{"keyring":{"enabled":false}}"#), true)
            .expect("accepted");
        assert_eq!(
            sent_json(&sent, 0),
            serde_json::json!({"cursor": {"size": 24}, "keyring": {"enabled": false}})
        );
        let again = state
            .apply(&json(r#"{"keyring":{"enabled":false}}"#), true)
            .expect("accepted");
        assert_eq!(again, JsApplied::Unchanged);
        assert_eq!(sent.borrow().len(), 1);
    }

    #[test]
    fn an_owned_document_refuses_an_autostart_entry_that_starts_a_second_keyring() {
        let (link, sent) = FakeLink::new(None);
        let mut state = state_over(link, true);
        let reason = state
            .apply(
                &json(r#"{"autostart":{"daemon":{"command":["gnome-keyring-daemon","--start"]}}}"#),
                true,
            )
            .expect_err("a second keyring is refused");
        assert!(reason.contains("keyring.enabled"), "{reason}");
        assert!(sent.borrow().is_empty(), "nothing was committed");
    }

    #[test]
    fn the_recording_link_keeps_each_commit_with_its_document_and_revision() {
        let (link, _sent) = FakeLink::new(None);
        let log: JsCommitLog = Rc::default();
        let mut recording = JsRecordingLink::new(Box::new(link), Rc::clone(&log));
        let committed = recording.commit(br#"{"cursor":{"size":32}}"#).expect("committed");
        assert_eq!(committed.revision, 1);
        let records = log.borrow();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].revision, 1);
        assert_eq!(records[0].document, br#"{"cursor":{"size":32}}"#.to_vec());
    }
}
