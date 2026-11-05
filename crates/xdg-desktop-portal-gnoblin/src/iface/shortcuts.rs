//! `org.freedesktop.impl.portal.GlobalShortcuts` and `Clipboard`.
//!
//! Neither of these is a grant in `gnoblin-authority`'s model, and neither has a
//! compositor call this build can make, so their shape is the same: the session lifetime
//! and the state machine are real, and every member that reaches the compositor answers
//! `NotSupported` with the missing member's name.
//!
//! # Why there is no prompt
//!
//! `GlobalShortcuts` and `Clipboard` carry no capability: a shortcut binding and a
//! clipboard transfer are choices a user makes, but they are not "may this app capture"
//! or "may this app inject", and the authority grants only those two. The plan's rule that
//! every user choice is a `portal.ui` request therefore has nothing to attach to here: the
//! shell's own shortcut editor and clipboard UI are the choice, and this backend has no
//! call to make once it has one. Asking for consent to a call that can never be made would
//! be a dialogue with no consequence, so the members refuse first and name what is missing.
//!
//! # Clipboard sessions
//!
//! The `Clipboard` interface declares no `CreateSession`: the frontend creates the session
//! through `RemoteDesktop` and passes the same handle here. The methods therefore look the
//! session up by the caller's handle without requiring a particular creator, and a handle
//! the caller does not own is refused.

use std::collections::BTreeMap;

use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};
use zbus::{OwnedFd, OwnedVariant};

use super::capture::{Options, text};
use super::{Portal, Reply, export_request, export_session, refused, remove, sender, string_of, unsupported};
use crate::backends::Shortcut;
use crate::consent::{Interface, NewSession, State};

/// The version of `GlobalShortcuts` this build serves.
const GLOBAL_SHORTCUTS_VERSION: u32 = 1;
/// The version of `Clipboard` this build serves.
const CLIPBOARD_VERSION: u32 = 1;

/// `org.freedesktop.impl.portal.GlobalShortcuts`.
#[derive(Debug)]
pub struct GlobalShortcuts {
    portal: Portal,
}

impl GlobalShortcuts {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> GlobalShortcuts {
        GlobalShortcuts {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcuts {
    /// Create a shortcut session.
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller,
                    app_id: app_id.clone(),
                    sandboxed: super::capture::sandboxed(&options),
                    interface: Interface::GlobalShortcuts,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        export_session(&self.portal, &path).await?;
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let mut results = BTreeMap::new();
        results.insert("session_id".to_owned(), text(&path));
        tracing::info!(app = %app_id, session = %path, "a global shortcuts session was created");
        Ok((0, results))
    }

    /// Bind the shortcuts the caller wants.
    ///
    /// The session is configured first, so a caller that asks twice is refused by the
    /// session rather than by the compositor, and then the call is attempted: the default
    /// backend refuses it, and the reply names the missing member.
    async fn bind_shortcuts(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Vec<(String, BTreeMap<String, OwnedVariant>)>,
        _parent_window: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .configure(&caller, &path, BTreeMap::new())
                .map_err(refused)?;
        }
        let wanted: Vec<Shortcut> = shortcuts
            .into_iter()
            .map(|(id, properties)| Shortcut {
                id,
                description: properties.get("description").and_then(string_of).unwrap_or_default(),
                trigger: properties.get("preferred_trigger").and_then(string_of),
            })
            .collect();
        let bound = {
            let mut shared = self.portal.lock();
            shared.shortcuts.bind(&path, &wanted)
        };
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let bound = bound.map_err(unsupported)?;
        let mut results = BTreeMap::new();
        results.insert("shortcuts".to_owned(), shortcut_list(&bound));
        Ok((0, results))
    }

    /// The shortcuts this session holds.
    async fn list_shortcuts(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let shared = self.portal.lock();
            let session = shared
                .guard
                .session(&caller, &path)
                .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
            if session.state == State::Created {
                return Err(refused(crate::consent::Refusal::WrongState {
                    path: path.clone(),
                    state: session.state,
                }));
            }
        }
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        // Nothing was ever bound: the call that binds refuses, and the session records
        // only what it accepted.
        let mut results = BTreeMap::new();
        results.insert("shortcuts".to_owned(), shortcut_list(&[]));
        Ok((0, results))
    }

    /// Reopen the shortcut editor.
    async fn configure_shortcuts(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _parent_window: String,
        _options: Options,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        let configured = {
            let mut shared = self.portal.lock();
            if shared.guard.session(&caller, &path).is_none() {
                return Err(refused(crate::consent::Refusal::UnknownSession { path: path.clone() }));
            }
            shared.shortcuts.configure(&path)
        };
        configured.map_err(unsupported)
    }

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        GLOBAL_SHORTCUTS_VERSION
    }

    /// A shortcut the user pressed.
    #[zbus(signal)]
    async fn activated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: BTreeMap<String, OwnedVariant>,
    ) -> zbus::Result<()>;

    /// A shortcut the user released, for a binding that reports both edges.
    #[zbus(signal)]
    async fn deactivated(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        shortcut_id: &str,
        timestamp: u64,
        options: BTreeMap<String, OwnedVariant>,
    ) -> zbus::Result<()>;

    /// The bindings changed, for the frontend to forward.
    #[zbus(signal)]
    async fn shortcuts_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Vec<(String, BTreeMap<String, OwnedVariant>)>,
    ) -> zbus::Result<()>;
}

/// The `shortcuts` result: `a(sa{sv})`.
fn shortcut_list(shortcuts: &[Shortcut]) -> OwnedVariant {
    use zbus::{Array, Signature, Structure, Variant};
    let signature = Signature::try_from("(sa{sv})").expect("the shortcut signature is a constant");
    let mut array = Array::new(&signature);
    for shortcut in shortcuts {
        let mut properties: std::collections::HashMap<String, OwnedVariant> = std::collections::HashMap::new();
        properties.insert("description".to_owned(), text(&shortcut.description));
        if let Some(trigger) = &shortcut.trigger {
            properties.insert("trigger_description".to_owned(), text(trigger));
        }
        let entry = Structure::from((shortcut.id.clone(), properties));
        if let Err(error) = array.append(Variant::Structure(entry)) {
            tracing::warn!(%error, "a shortcut could not be added to the reply");
        }
    }
    OwnedVariant::try_from(Variant::Array(array)).expect("an array of shortcuts is a value")
}

/// `org.freedesktop.impl.portal.Clipboard`.
#[derive(Debug)]
pub struct Clipboard {
    portal: Portal,
}

impl Clipboard {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> Clipboard {
        Clipboard {
            portal: Portal { shared, connection },
        }
    }

    /// Require a live session the caller owns.
    fn owned(&self, caller: &str, path: &str) -> fdo::Result<State> {
        let shared = self.portal.lock();
        let session = shared
            .guard
            .session(caller, path)
            .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.to_owned() }))?;
        Ok(session.state)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Clipboard")]
impl Clipboard {
    /// Ask to own the session's clipboard.
    async fn request_clipboard(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        self.owned(&caller, &path)?;
        let mut shared = self.portal.lock();
        shared.clipboard.request(&path).map_err(unsupported)
    }

    /// Offer these MIME types to the compositor's selection.
    async fn set_selection(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        options: Options,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        self.owned(&caller, &path)?;
        let mime_types = match options.get("mime_types") {
            Some(value) => Vec::<String>::try_from(value.clone()).unwrap_or_default(),
            None => Vec::new(),
        };
        let mut shared = self.portal.lock();
        shared.clipboard.set_selection(&path, &mime_types).map_err(unsupported)
    }

    /// A descriptor to write the caller's data into.
    async fn selection_write(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        serial: u32,
    ) -> fdo::Result<OwnedFd> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        self.owned(&caller, &path)?;
        let mut shared = self.portal.lock();
        let fd = shared.clipboard.selection_write(&path, serial).map_err(unsupported)?;
        Ok(OwnedFd::from(fd))
    }

    /// Report whether the caller's write succeeded.
    async fn selection_write_done(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        serial: u32,
        success: bool,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        self.owned(&caller, &path)?;
        let mut shared = self.portal.lock();
        shared.clipboard.write_done(&path, serial, success);
        Ok(())
    }

    /// A descriptor to read the compositor's selection from.
    async fn selection_read(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        mime_type: String,
    ) -> fdo::Result<OwnedFd> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        self.owned(&caller, &path)?;
        let mut shared = self.portal.lock();
        let fd = shared
            .clipboard
            .selection_read(&path, &mime_type)
            .map_err(unsupported)?;
        Ok(OwnedFd::from(fd))
    }

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        CLIPBOARD_VERSION
    }

    /// The selection's owner changed.
    #[zbus(signal)]
    async fn selection_owner_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        options: BTreeMap<String, OwnedVariant>,
    ) -> zbus::Result<()>;

    /// The selection is being read.
    #[zbus(signal)]
    async fn selection_transfer(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        mime_type: &str,
        serial: u32,
    ) -> zbus::Result<()>;
}
