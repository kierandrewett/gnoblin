//! The configuration document of a shell that commits none of its own: what a Rust or a Python shell wrote to the settings of the devices
//! and of the compositor reaches the compositor, and applies from the next login.
//!
//! A write to `Keyboard.repeat_delay`, `Mouse.sensitivity` or `gnoblin.compositor.user_shaders` is checked where it is written and kept in
//! the generation's ledger (`gnoblin.input.document`, `document_revision`); this module is the host that commits it, as the JavaScript runtime
//! is for a TypeScript shell. A shell calls nothing: the shell host looks after each turn of the dispatch loop ([`commit_changes`]).
//!
//! # Who commits
//!
//! The runtime that owns the document. The JavaScript runtime builds the document of its generation (the script's own settings and the
//! ledger text), commits it on every load and tells the guardian the revision, so this module never commits for a JavaScript or a
//! TypeScript generation: two commits would be two revisions, and a commit of the ledger alone would take the script's other sections out of
//! the document. Every other generation (native, Python) has no runtime to do it, and this module does.
//!
//! # What a commit is
//!
//! The ledger lays over the document in force, and nothing else changes: the document is what the compositor last accepted, else the one the
//! session started with (the last accepted document of the previous login, which the compositor reads from its store), so a section no
//! setting of the ledger names (`appearance`, `cursor`, the autostart entries) is carried over untouched. The keys an earlier commit of this shell
//! set are taken out first ([`gnoblin_config::patch`]), so a reload whose new generation no longer writes a setting takes it back, as
//! it does for a TypeScript shell. A shell that wrote nothing commits nothing and changes nothing: the document of a session whose shell has no
//! settings is the previous login's. Nothing is sent when the result is the document already in force.
//!
//! # When it commits
//!
//! After the generation that serves has written: once it started (before the shell is reported ready, so a commit its `init` made is in
//! force by then), and after each turn when the revision of the ledger or the generation changed. A turn that made several writes is one
//! commit. A generation that is only staged, and a reload that fails, commit nothing: the host asks about the generation that serves.
//!
//! A write made after the turn that asked the shell to exit, or by a shutdown hook once the shell is stopped, is not committed: the host
//! looks at the turn that asked to exit before it returns, and the generation is retired by the time its hooks run.
//!
//! A commit that fails (the connection, a refusal, a reply that did not come) is retried on a later turn after a wait that doubles from one
//! second to half a minute, at most eight times for one revision of the ledger; the next write starts again. An unconfirmed commit may have
//! been applied, so the keys it carried are taken back by the next one like the keys of a confirmed one.
//!
//! The commit is fenced to the shell the stored configuration belongs to. The compositor keeps the accepted document under the file
//! `locate` resolves (the shell discovery selects), so a shell run by another path (`gnoblin --shell ./dev.so`) while a configuration shell
//! exists would write over that shell's document: the host then commits nothing and says so once. With no shell at all (a first login) the
//! file is nobody's and the commit goes ahead.
//!
//! A commit is not reported to the guardian. The guardian starts what the `autostart` entries of a committed document name, and these
//! entries are another writer's (the document is the previous login's): a native shell reports revision zero and no entry, as it always did.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_client::{Client, Committed, UnixTransport};
use gnoblin_config::patch::{KeyPath, empty, leaves, overlay, remove};
use gnoblin_config::{Config, Environment, Store, StoreError, Value};
use gnoblin_core_abi::{self as abi, GnoblinCtx};

use crate::registry::GenerationState;
use crate::verbs::OwnerSocket;
use crate::{log, model, registry, session};

/// How long a call to the compositor waits for its reply. The compositor answers a commit at once, so a later reply is a compositor that is
/// stuck.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// The wait before the first retry of a commit that failed, in milliseconds; it doubles up to [`RETRY_MAX_MS`].
const RETRY_FIRST_MS: f64 = 1_000.0;

/// The longest wait between two tries of one commit, in milliseconds.
const RETRY_MAX_MS: f64 = 30_000.0;

/// How many times one revision of the ledger is tried before the host waits for the next write.
const ATTEMPTS: u32 = 8;

/// The shell host's connection for the configuration document: `config.current` and `config.commit` of the owner socket.
pub trait DocumentLink {
    /// The document in force as JSON: the last one the compositor accepted, else the one the session started with. None when there is none.
    ///
    /// # Errors
    ///
    /// The connection failed or the compositor refused.
    fn current(&mut self) -> Result<Option<Vec<u8>>, String>;

    /// Commits `document` (JSON), which replaces the one in force.
    ///
    /// # Errors
    ///
    /// The connection failed or the compositor refused; the document in force stays, unless the commit was applied and only its reply was lost.
    fn commit(&mut self, document: &[u8]) -> Result<Committed, String>;
}

/// What the host gives the core to reach the compositor's document: a way to open a link, when a shell has settings to commit.
pub trait DocumentLinkProvider {
    /// Opens the link.
    ///
    /// # Errors
    ///
    /// The compositor cannot be reached.
    fn open(&self) -> Result<Box<dyn DocumentLink>, String>;
}

/// [`DocumentLink`] over a connection of its own to the owner socket, opened for one commit.
struct OwnerDocument(Client<UnixTransport>);

impl DocumentLinkProvider for OwnerSocket {
    fn open(&self) -> Result<Box<dyn DocumentLink>, String> {
        let path = self.path()?;
        let transport =
            UnixTransport::connect(path).map_err(|error| format!("cannot connect to {}: {error}", path.display()))?;
        let mut client = Client::connect(transport, crate::verbs::ipc_version()).map_err(|error| error.to_string())?;
        client
            .set_read_timeout(Some(REPLY_WAIT))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(OwnerDocument(client)))
    }
}

impl DocumentLink for OwnerDocument {
    fn current(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self.0.configuration().map_err(|error| error.to_string())? {
            Some((_, document)) => Ok(Some(document)),
            // The compositor holds nothing it was committed in this session: the document it started with is the last accepted one of the
            // previous login, which every part of the session that reads a setting at start reads from the same store.
            None => Ok(started_with()),
        }
    }

    fn commit(&mut self, document: &[u8]) -> Result<Committed, String> {
        self.0
            .commit_config_reporting(document)
            .map_err(|error| error.to_string())
    }
}

/// The document the session started with, as JSON: the last accepted document of the configuration the session reads. None when nothing is
/// stored for it (a first login, or a document of another shell, which is what the session itself then starts without); a store that is there
/// and cannot be read is said, and the session starts from the defaults as the compositor does.
fn started_with() -> Option<Vec<u8>> {
    let store = Store::from_env()?;
    let path = gnoblin_config::locate(&Environment::from_env()).ok()?;
    match store.load(path.path()) {
        Ok(stored) => Some(stored.config.document_json().into_bytes()),
        Err(error) if error.is_absent() || matches!(error, StoreError::OtherSource { .. }) => {
            log::debug!(area = "document", %error, "no accepted document is stored for this configuration");
            None
        }
        Err(error) => {
            log::warn!(area = "document", %error, "the last accepted document cannot be read, so the shell's settings are laid over none");
            None
        }
    }
}

/// A commit that failed, and when it may be tried again.
struct FailedCommit {
    ctx: GnoblinCtx,
    revision: u64,
    attempts: u32,
    /// The session clock's milliseconds from which the next try may go out.
    retry_at: f64,
}

/// What the committer keeps for the shell instance of its thread.
#[derive(Default)]
struct Committer {
    /// How to reach the compositor's document. It outlives [`reset`].
    provider: Option<Box<dyn DocumentLinkProvider>>,
    /// The shell file the host runs, which the fence compares with the configuration's.
    shell: Option<PathBuf>,
    /// Whether the shell that runs is the one the stored configuration belongs to, once it was asked.
    fence_open: Option<bool>,
    /// The generation and ledger revision whose settings are in the compositor's document, or were found there already.
    seen: Option<(GnoblinCtx, u64)>,
    /// The commit that failed last, until it succeeds or the ledger changes.
    failed: Option<FailedCommit>,
    /// The key paths the commits so far set, which the next commit takes back before it lays its own.
    owned: Vec<KeyPath>,
}

thread_local! {
    static COMMITTER: RefCell<Committer> = RefCell::new(Committer::default());
}

/// Installs how the shell thread reaches the compositor's document, replacing any earlier one.
pub(crate) fn set_provider(provider: Box<dyn DocumentLinkProvider>) {
    COMMITTER.with(|committer| committer.borrow_mut().provider = Some(provider));
}

/// Names the shell file the host runs, for the fence.
pub(crate) fn set_shell(path: &Path) {
    COMMITTER.with(|committer| {
        let mut committer = committer.borrow_mut();
        committer.shell = Some(path.to_path_buf());
        committer.fence_open = None;
    });
}

/// The shell instance ended: what it committed is forgotten. The way to reach the document stays.
pub(crate) fn reset() {
    COMMITTER.with(|committer| {
        let mut committer = committer.borrow_mut();
        committer.seen = None;
        committer.failed = None;
        committer.owned.clear();
        committer.fence_open = None;
    });
}

/// The same file, by canonical path where both can be resolved.
fn same_file(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Whether the shell that runs may write the stored document: it is the shell the configuration belongs to, or none is (a first login).
/// Said once, at info, when it is not.
fn fence_open() -> bool {
    if let Some(known) = COMMITTER.with(|committer| committer.borrow().fence_open) {
        return known;
    }
    let shell = COMMITTER.with(|committer| committer.borrow().shell.clone());
    let open = match (shell, gnoblin_config::locate(&Environment::from_env())) {
        (Some(shell), Ok(located)) if located.exists() && !same_file(&shell, located.path()) => {
            log::info!(
                area = "document",
                "the shell {} runs, and the stored configuration belongs to {}: the settings it writes will not be kept for this run",
                shell.display(),
                located.path().display()
            );
            false
        }
        _ => true,
    };
    COMMITTER.with(|committer| committer.borrow_mut().fence_open = Some(open));
    open
}

/// Commits what the shell of the generation `ctx` wrote to its settings, if it wrote something since the last look. `ctx` is the generation
/// that serves; a staged generation, one that ended and one of the JavaScript runtime are left alone.
pub(crate) fn commit_changes(ctx: GnoblinCtx) {
    match registry::language_of(ctx) {
        Some(abi::LANG_JS | abi::LANG_TS) | None => return,
        Some(_) => {}
    }
    if registry::state_of(ctx) != Some(GenerationState::Active) {
        return;
    }
    // The revision alone is read each turn; the text of the ledger is built only for a change.
    let revision = model::written_revision(ctx);
    let now = session::with(|session| session.now_ms());
    let wanted = COMMITTER.with(|committer| {
        let mut committer = committer.borrow_mut();
        if committer.seen == Some((ctx, revision)) {
            return false;
        }
        if let Some(failure) = &committer.failed {
            if (failure.ctx, failure.revision) != (ctx, revision) {
                committer.failed = None;
            } else if failure.attempts >= ATTEMPTS || now < failure.retry_at {
                return false;
            }
        }
        // A generation that wrote nothing has nothing to commit, unless the one before it wrote something it must now take back.
        if revision == 0 && committer.owned.is_empty() {
            committer.seen = Some((ctx, revision));
            return false;
        }
        true
    });
    if !wanted {
        return;
    }
    if !fence_open() {
        COMMITTER.with(|committer| committer.borrow_mut().seen = Some((ctx, revision)));
        return;
    }
    let (revision, ledger) = model::written_settings(ctx).unwrap_or_else(|| (0, "{}".to_owned()));
    match commit(&ledger) {
        Ok(committed) => {
            COMMITTER.with(|committer| {
                let mut committer = committer.borrow_mut();
                committer.seen = Some((ctx, revision));
                committer.failed = None;
            });
            match committed {
                Some(committed) => {
                    for notice in &committed.notices {
                        log::warn!(area = "document", "not applied: {}", notice.text);
                    }
                    log::info!(
                        area = "document",
                        revision = committed.revision,
                        "the shell committed the settings it wrote as configuration revision {}",
                        committed.revision
                    );
                }
                None => log::debug!(
                    area = "document",
                    "the document in force already says what the shell wrote"
                ),
            }
        }
        Err(reason) => {
            let attempts = COMMITTER.with(|committer| {
                let mut committer = committer.borrow_mut();
                let attempts = committer
                    .failed
                    .as_ref()
                    .filter(|failure| (failure.ctx, failure.revision) == (ctx, revision))
                    .map_or(1, |failure| failure.attempts + 1);
                let wait = (RETRY_FIRST_MS * f64::from(1_u32 << (attempts - 1).min(16))).min(RETRY_MAX_MS);
                committer.failed = Some(FailedCommit {
                    ctx,
                    revision,
                    attempts,
                    retry_at: now + wait,
                });
                attempts
            });
            log::warn!(
                area = "document",
                "the settings the shell wrote were not committed (try {attempts} of {ATTEMPTS}): {reason}"
            );
        }
    }
}

/// The document that results from `ledger` laid over `current`, after the keys of `owned` are taken back.
fn candidate(current: &Value, owned: &[KeyPath], ledger: &Value) -> Value {
    let mut document = current.clone();
    for path in owned {
        remove(&mut document, path);
    }
    overlay(&mut document, ledger);
    document
}

/// Commits the settings of `ledger` (the document text of the input manager) over the document in force. `None` when that document already
/// says what the result would.
fn commit(ledger: &str) -> Result<Option<Committed>, String> {
    let ledger = Config::from_json_bytes(ledger.as_bytes())
        .map_err(|problems| format!("what the shell wrote is not a configuration: {problems}"))?;
    let opened = COMMITTER.with(|committer| committer.borrow().provider.as_ref().map(|provider| provider.open()));
    let mut link = opened.ok_or_else(|| {
        "this core has no owner connection to the compositor for the document (the host gave it none)".to_owned()
    })??;
    let current = match link.current()? {
        Some(bytes) => Config::from_json_bytes(&bytes)
            .map_err(|problems| format!("the document in force does not read back: {problems}"))?
            .document()
            .clone(),
        None => empty(),
    };
    let owned = COMMITTER.with(|committer| committer.borrow().owned.clone());
    let wanted = candidate(&current, &owned, ledger.document());
    let now = leaves(ledger.document());
    if wanted == current {
        COMMITTER.with(|committer| committer.borrow_mut().owned = now);
        return Ok(None);
    }
    let config = Config::from_value(wanted).map_err(|report| {
        report
            .problems()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    // The commit may be applied though its reply is lost, so what it carries is owned from here on, whatever the answer.
    let mut sent = owned;
    sent.extend(
        now.iter()
            .filter(|path| !sent_has(&sent, path))
            .cloned()
            .collect::<Vec<_>>(),
    );
    COMMITTER.with(|committer| committer.borrow_mut().owned = sent);
    let committed = link.commit(config.document_json().as_bytes())?;
    COMMITTER.with(|committer| committer.borrow_mut().owned = now);
    Ok(Some(committed))
}

/// Whether `paths` holds `path`.
fn sent_has(paths: &[KeyPath], path: &KeyPath) -> bool {
    paths.contains(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        Config::from_json_bytes(text.as_bytes())
            .expect("a valid configuration")
            .document()
            .clone()
    }

    #[test]
    fn the_ledger_lays_over_the_document_and_names_only_its_own_keys() {
        let current = json(r#"{"appearance":{"reduced-motion":true},"input":{"mouse":{"speed":0.1}}}"#);
        let ledger = json(r#"{"input":{"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#);
        assert_eq!(
            candidate(&current, &[], &ledger),
            json(
                r#"{"appearance":{"reduced-motion":true},"input":{"mouse":{"speed":0.1},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#
            )
        );
    }

    #[test]
    fn a_key_the_last_commit_set_and_this_ledger_does_not_is_taken_back() {
        let current =
            json(r#"{"input":{"mouse":{"speed":0.5},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#);
        let owned = leaves(&json(
            r#"{"input":{"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#,
        ));
        let ledger = json(r#"{"input":{"keyboard":{"delay":300}}}"#);
        assert_eq!(
            candidate(&current, &owned, &ledger),
            json(r#"{"input":{"mouse":{"speed":0.5},"keyboard":{"delay":300}}}"#),
            "the compositor section went with its last key, and the mouse was never this shell's"
        );
    }

    #[test]
    fn a_ledger_that_says_what_is_in_force_changes_nothing() {
        let current = json(r#"{"compositor":{"user-shaders":false},"cursor":{"size":24}}"#);
        let ledger = json(r#"{"compositor":{"user-shaders":false}}"#);
        assert_eq!(candidate(&current, &[], &ledger), current);
    }
}
