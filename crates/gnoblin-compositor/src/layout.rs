//! Per-workspace layout engines, chosen by the `layout` configuration section.
//!
//! Each workspace's engine is resolved once when the wm first needs it: a workspace
//! override (matched against the workspace's decimal id) wins over `default`. A name
//! of `floating` is answered by [`FloatingEngine`], which gives the same reply the
//! shipped `gnoblin.layouts.floating` module gives (no placements, a float effect for
//! every tiled window, and a declined geometry request), without a Lua VM. The
//! equivalence is checked by a test that runs both.
//!
//! Failure modes, all resolved to floating so the session still starts:
//! - a name whose module cannot be loaded (missing module, syntax error, load
//!   deadline, a wedged VM): refused at startup with a warning naming the layout and
//!   the reason, and every workspace that asks for it floats;
//! - a module that raises or stalls while arranging: the wm reports it per run as
//!   `LayoutFailed` and applies its own fallback, as it does for any layout.
//!
//! Not applied yet: `output` overrides. The wayland layer hands the factory only a
//! workspace id, so an output override is logged at startup and otherwise ignored.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gnoblin_config::{Layout, SearchPaths, Store};
use gnoblin_lua::layout::{Data, Gaps, Host, HostConfig, LuaEngine, default_prefix, installed_module_dir};
use gnoblin_types::WorkspaceId;
use gnoblin_wm::layout::{
    Camera, Generation, Handled, Job, LayoutEffect, LayoutEngine, LayoutFailure, LayoutOutcome, Reply, Ticket,
};
use tracing::{debug, warn};

use crate::SessionConfig;

/// The layout name that floats every window. It is never loaded as a module.
pub const FLOATING: &str = "floating";

/// The `layout` section as the session reads it: plain data, so it can travel in a
/// [`SessionConfig`](crate::SessionConfig) to the thread that runs the compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutSettings {
    /// The layout of a workspace with no override.
    pub default: String,
    /// Layout names by output connector. Not applied yet; see the module docs.
    pub outputs: Vec<(String, String)>,
    /// Layout names by workspace id, as the decimal id the wm uses.
    pub workspaces: Vec<(String, String)>,
    /// Directories searched for layout modules, before the installed ones.
    pub module_dirs: Vec<PathBuf>,
}

impl Default for LayoutSettings {
    /// Every workspace floats and no configured directory is searched.
    fn default() -> Self {
        Self {
            default: FLOATING.to_owned(),
            outputs: Vec::new(),
            workspaces: Vec::new(),
            module_dirs: Vec::new(),
        }
    }
}

impl LayoutSettings {
    /// The settings of an accepted `layout` section. Overrides with a missing half are
    /// dropped, and an absent `default` is floating.
    fn from_section(section: Layout) -> Self {
        Self {
            default: section.default.unwrap_or_else(|| FLOATING.to_owned()),
            outputs: section
                .outputs
                .unwrap_or_default()
                .into_iter()
                .filter_map(|o| Some((o.output?, o.layout?)))
                .collect(),
            workspaces: section
                .workspaces
                .unwrap_or_default()
                .into_iter()
                .filter_map(|w| Some((w.workspace?, w.layout?)))
                .collect(),
            module_dirs: section
                .module_dirs
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        }
    }
}

/// Sets `config.layouts` from the last accepted configuration in `store`, read from
/// `source`. A document with no `layout` section keeps every workspace floating.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.layout.clone().unwrap_or_default();
            let settings = LayoutSettings::from_section(section);
            debug!(default = %settings.default, "[layout] loaded from the last accepted configuration");
            config.layouts = settings;
        }
        Err(error) => warn!(%error, "[layout] configuration not readable; every workspace floats"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// The session binary calls this before it starts the compositor. With no configuration
/// file there is no accepted document, so every workspace floats.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug!("[layout] no state directory is set; every workspace floats");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug!(%error, "[layout] no configuration file; every workspace floats"),
    }
}

/// The resolved layout settings for the session's workspaces, with the Lua host that
/// runs them. Built once, inside the thread that runs the compositor.
pub struct LayoutPlan {
    default: String,
    workspaces: BTreeMap<String, String>,
    host: Rc<RefCell<Host>>,
    failed: BTreeSet<String>,
}

impl LayoutPlan {
    /// Loads every layout the settings name. A layout that cannot be loaded is named
    /// in a warning, and every workspace that asks for it floats.
    pub fn new(settings: LayoutSettings) -> Self {
        for (output, _) in &settings.outputs {
            warn!(
                output,
                "[layout] output layout overrides are not applied yet; the output's workspaces use the default"
            );
        }
        let mut dirs = settings.module_dirs;
        dirs.push(installed_module_dir(&default_prefix(
            std::env::var_os("GNOBLIN_PREFIX").as_deref(),
        )));
        let mut host = Host::new(HostConfig::new(dirs));
        let mut failed = BTreeSet::new();
        let names: BTreeSet<&str> = std::iter::once(settings.default.as_str())
            .chain(settings.workspaces.iter().map(|(_, layout)| layout.as_str()))
            .filter(|name| *name != FLOATING)
            .collect();
        for name in names {
            if let Err(failure) = host.register(name, &format!("gnoblin.layouts.{name}"), Data::Table(BTreeMap::new()))
            {
                warn!(layout = name, reason = ?failure.reason, message = %failure.message, "[layout] layout cannot be loaded; workspaces that use it float");
                failed.insert(name.to_owned());
            }
        }
        Self {
            default: settings.default,
            workspaces: settings.workspaces.into_iter().collect(),
            host: Rc::new(RefCell::new(host)),
            failed,
        }
    }

    /// The layout a workspace uses: its override, else `default`. A name that is
    /// floating or failed to load resolves to [`FLOATING`].
    pub fn resolve(&self, workspace: WorkspaceId) -> &str {
        let key = workspace.get().to_string();
        let name = self.workspaces.get(&key).unwrap_or(&self.default);
        if name == FLOATING || self.failed.contains(name) {
            FLOATING
        } else {
            name
        }
    }

    /// The engine for `workspace`. Floating workspaces get [`FloatingEngine`];
    /// others get a Lua engine over the shared host.
    pub fn engine_for(&self, workspace: WorkspaceId) -> Box<dyn LayoutEngine> {
        match self.resolve(workspace) {
            FLOATING => Box::new(FloatingEngine::default()),
            name => Box::new(LuaEngine::new(self.host.clone(), name, workspace, Gaps::default())),
        }
    }
}

/// The native equivalent of the shipped `gnoblin.layouts.floating`: an arrange
/// floats every tiled window and places none; a geometry request is declined so the
/// wm applies its own logic.
#[derive(Default)]
pub struct FloatingEngine {
    ready: Option<(Generation, Result<Reply, LayoutFailure>)>,
}

impl LayoutEngine for FloatingEngine {
    fn submit(&mut self, job: Job, generation: Generation) -> Ticket {
        let reply = match job {
            Job::Arrange(params) => Reply::Handled(Handled {
                placements: Vec::new(),
                camera: Camera::Orthographic,
                effects: params
                    .windows()
                    .iter()
                    .map(|window| LayoutEffect::Float { window: window.id })
                    .collect(),
            }),
            Job::Request(_) => Reply::Declined,
        };
        self.ready = Some((generation, Ok(reply)));
        Ticket::new(generation)
    }

    fn collect(&mut self, generation: Generation, _deadline: Instant) -> LayoutOutcome {
        match self.ready.take() {
            Some((found, result)) if found == generation => LayoutOutcome::Ready {
                generation: found,
                result,
            },
            other => {
                self.ready = other;
                LayoutOutcome::Pending
            }
        }
    }
}
