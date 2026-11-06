//! `org.freedesktop.impl.portal.Settings`.
//!
//! The one interface in this backend that is fully real: its values come from the
//! compositor's `settings` object (`crate::settings`), arrive as property diffs, and are
//! published as they are. Nothing here writes back, because the interface has no writable
//! member and the table has no outbound path (D15).
//!
//! `ReadAll`'s namespaces argument is the frontend's filter, and the matching rule is the
//! one the interface documents: an empty list or an empty string matches everything, and
//! globbing is for trailing sections only.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;
use zbus::{Connection, fdo};
use zbus::{OwnedVariant, Structure, Variant};

use super::{Shared, locked};
use crate::settings::{Change, Setting, Table};

/// The version of the interface this build serves.
///
/// 2 is `Read` beside `ReadAll`, which is what this build implements; a toolkit that asks
/// for a newer member gets an error rather than a wrong answer.
const VERSION: u32 = 2;

/// One served `Settings` object.
#[derive(Debug)]
pub struct Settings {
    shared: Arc<Mutex<Shared>>,
}

impl Settings {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: Arc<Mutex<Shared>>, _connection: Connection) -> Settings {
        Settings { shared }
    }

    /// Announce one change to every subscriber.
    pub async fn emit(emitter: &SignalEmitter<'_>, change: Change) -> zbus::Result<()> {
        let value = owned(&change.value);
        Settings::setting_changed(emitter, change.namespace, change.key, value).await
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    /// Every published value, filtered by the requested namespaces.
    fn read_all(&self, namespaces: Vec<String>) -> BTreeMap<String, BTreeMap<String, OwnedVariant>> {
        let shared = locked(&self.shared);
        let all = namespaces.is_empty() || namespaces.iter().any(String::is_empty);
        let mut reply: BTreeMap<String, BTreeMap<String, OwnedVariant>> = BTreeMap::new();
        for (namespace, key, setting) in shared.settings.values() {
            if !all && !namespaces.iter().any(|pattern| Table::matches(pattern, namespace)) {
                continue;
            }
            reply
                .entry(namespace.to_owned())
                .or_default()
                .insert(key.to_owned(), owned(setting));
        }
        reply
    }

    /// One value, or an error naming the namespace and key.
    ///
    /// The interface's own rule is an error rather than an empty answer for a key this
    /// backend does not publish: a toolkit that receives `None` for a key it asked for
    /// cannot tell "unset" from "unknown".
    fn read(&self, namespace: String, key: String) -> fdo::Result<OwnedVariant> {
        let shared = locked(&self.shared);
        shared
            .settings
            .read(&namespace, &key)
            .map(owned)
            .ok_or_else(|| fdo::Error::UnknownProperty(format!("`{namespace}` has no `{key}`")))
    }

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        VERSION
    }

    /// A published value changed.
    #[zbus(signal)]
    async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: OwnedVariant,
    ) -> zbus::Result<()>;
}

/// One setting as the variant the interface carries.
fn owned(setting: &Setting) -> OwnedVariant {
    match setting {
        Setting::Uint(value) => OwnedVariant::from(*value),
        Setting::Text(value) => OwnedVariant::try_from(Variant::from(value.as_str())).expect("a string is a value"),
        Setting::Bool(value) => OwnedVariant::from(*value),
        Setting::Double(value) => OwnedVariant::from(*value),
        // The accent colour is `(ddd)`: three doubles in one structure, which is the
        // signature toolkits read for `accent-color`.
        Setting::Color(red, green, blue) => {
            let structure = Structure::from((*red, *green, *blue));
            OwnedVariant::try_from(Variant::Structure(structure)).expect("three doubles are a value")
        }
    }
}
