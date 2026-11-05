//! Loaded extensions, read into the session before it starts.
//!
//! The compositor starts before the Lua runtime, and only the runtime evaluates the
//! configuration. So the extensions come from the last-good store: the document the
//! runtime accepted most recently. A session that has never accepted a document starts
//! with no extensions, and a store that cannot be read is named in the log and starts
//! the session without them. The compositor keeps serving either way.
//!
//! Each accepted entry becomes an [`ExtensionRoute`] to its provider connection, and one
//! [`Grant`] on each granted privileged connection. An entry the session cannot honour is
//! refused with a named reason, and the other entries still load.

use std::io;
use std::path::Path;

use gnoblin_config::extensions::{ExtensionEntry, ExtensionRefusal, LoadedExtensions, load_extensions};
use gnoblin_config::{SearchPaths, Store, StoreError};
use gnoblin_proto::Grant;
use tracing::{debug, info, warn};

use crate::SessionConfig;
use crate::ipc::ExtensionRoute;
use crate::schema::catalog;

/// What the accepted extensions become in one session.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtensionPlan {
    /// Where each loaded namespace is served.
    pub(crate) routes: Vec<ExtensionRoute>,
    /// The grant each granted privileged role advertises, by role.
    pub(crate) grants: Vec<(String, Grant)>,
    /// Every entry that did not load, with the reason.
    pub(crate) refused: Vec<ExtensionRefusal>,
}

/// Turns loaded entries into routes and grants for the privileged roles this session holds.
///
/// An entry is refused when its provider is not a privileged connection of this session,
/// or when one of its grants names a role that is not one. The second case is the
/// restricted-socket case: every unprivileged client shares one grants frame, so a grant
/// to such a role cannot be honoured yet, and the entry says so instead of loading half.
pub(crate) fn plan(loaded: LoadedExtensions, privileged: &[&str]) -> ExtensionPlan {
    let mut plan = ExtensionPlan {
        refused: loaded.refused,
        ..ExtensionPlan::default()
    };
    for entry in loaded.accepted {
        if let Err(reason) = check(&entry, privileged) {
            plan.refused.push(ExtensionRefusal {
                namespace: entry.namespace,
                reason,
            });
            continue;
        }
        for role in &entry.grants {
            plan.grants.push((
                role.clone(),
                Grant {
                    namespace: entry.namespace.clone(),
                    version: entry.schema.version,
                },
            ));
        }
        plan.routes.push(ExtensionRoute {
            namespace: entry.namespace,
            provider: entry.provider,
        });
    }
    plan
}

fn check(entry: &ExtensionEntry, privileged: &[&str]) -> Result<(), String> {
    if !privileged.contains(&entry.provider.as_str()) {
        return Err(format!(
            "the provider `{}` is not a privileged connection of this session",
            entry.provider
        ));
    }
    if let Some(role) = entry.grants.iter().find(|role| !privileged.contains(&role.as_str())) {
        return Err(format!(
            "the grant to `{role}` names a restricted socket; restricted sockets share one grants frame and cannot carry grants yet"
        ));
    }
    Ok(())
}

/// Reads the last accepted configuration from `store` and loads its extensions into `config`.
///
/// `source` is the configuration path the document must have been accepted from; the store
/// refuses a document read back for a different path. Relative schema paths resolve against
/// its directory. A store with no document yet is not an error and loads nothing.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    let stored = match store.load(source) {
        Ok(stored) => stored,
        Err(StoreError::Io { error, .. }) if error.kind() == io::ErrorKind::NotFound => {
            debug!("[extensions] no accepted configuration yet; starting without extensions");
            return;
        }
        Err(error) => {
            warn!(%error, "[extensions] the last accepted configuration cannot be read; starting without extensions");
            return;
        }
    };
    let catalog = match catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            warn!(%error, "[extensions] the stock schema cannot be read; starting without extensions");
            return;
        }
    };
    let base = source.parent().unwrap_or_else(|| Path::new("."));
    let loaded = load_extensions(stored.config.document(), base, catalog.schemas());
    let privileged: Vec<&str> = config
        .privileged
        .iter()
        .map(|connection| connection.role.as_str())
        .collect();
    let plan = plan(loaded, &privileged);

    for refusal in &plan.refused {
        warn!(namespace = %refusal.namespace, reason = %refusal.reason, "[extensions] {refusal}");
    }
    for (role, grant) in plan.grants {
        match config.privileged.iter_mut().find(|connection| connection.role == role) {
            Some(connection) => connection.extension_grants.push(grant),
            None => warn!(role = %role, "[extensions] a grant names a role with no privileged connection"),
        }
    }
    info!(
        loaded = plan.routes.len(),
        refused = plan.refused.len(),
        "[extensions] loaded from the last accepted configuration"
    );
    config.extensions.extend(plan.routes);
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// The session binary calls this before it starts the compositor. When no configuration
/// file is found, there is no accepted document either, so nothing is loaded.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        warn!("[extensions] no state directory is set; starting without extensions");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug!(%error, "[extensions] no configuration file; starting without extensions"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gnoblin_config::extensions::{ExtensionEntry, LoadedExtensions};
    use gnoblin_schema::parse_schema;

    use super::plan;
    use crate::ipc::ExtensionRoute;

    const TEXT: &str = include_str!("../../gnoblin-schema/tests/fixtures/extensions/example-ext.toml");

    fn entry(provider: &str, grants: &[&str]) -> ExtensionEntry {
        let schema = parse_schema("example-ext.toml", TEXT).expect("the fixture parses");
        ExtensionEntry {
            namespace: schema.namespace.as_str().to_owned(),
            schema_path: PathBuf::from("example-ext.toml"),
            provider: provider.to_owned(),
            grants: grants.iter().map(|role| (*role).to_owned()).collect(),
            schema,
        }
    }

    #[test]
    fn an_accepted_entry_becomes_a_route_and_one_grant_per_granted_role() {
        let loaded = LoadedExtensions {
            accepted: vec![entry("services", &["viewer"])],
            refused: Vec::new(),
        };
        let plan = plan(loaded, &["services", "viewer"]);
        assert!(plan.refused.is_empty());
        assert_eq!(
            plan.routes,
            vec![ExtensionRoute {
                namespace: "example.ext".to_owned(),
                provider: "services".to_owned(),
            }]
        );
        assert_eq!(plan.grants.len(), 1);
        assert_eq!(plan.grants[0].0, "viewer");
        assert_eq!(plan.grants[0].1.namespace, "example.ext");
    }

    #[test]
    fn a_provider_the_session_does_not_hold_is_refused_by_name() {
        let loaded = LoadedExtensions {
            accepted: vec![entry("nobody", &[])],
            refused: Vec::new(),
        };
        let plan = plan(loaded, &["services"]);
        assert!(plan.routes.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0]
                .reason
                .contains("`nobody` is not a privileged connection")
        );
    }

    #[test]
    fn a_grant_to_a_restricted_socket_is_refused_by_name() {
        let loaded = LoadedExtensions {
            accepted: vec![entry("services", &["shell"])],
            refused: Vec::new(),
        };
        let plan = plan(loaded, &["services"]);
        assert!(plan.routes.is_empty(), "a half-granted extension does not load");
        assert!(plan.grants.is_empty());
        assert!(plan.refused[0].reason.contains("`shell` names a restricted socket"));
    }
}
