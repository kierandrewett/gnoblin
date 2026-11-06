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
//! Resolution order for a workspace: its workspace override, else the override of
//! the output it is on, else `default`. The output is the connector name the wm
//! knows for the workspace's current output; it is unknown until the output is
//! identified, and then the output override does not apply.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gnoblin_config::{Layout, SearchPaths, Store};
use gnoblin_log::{debug, warn};
use gnoblin_lua::layout::{Data, Gaps, Host, HostConfig, LuaEngine, default_prefix, installed_module_dir};
use gnoblin_types::WorkspaceId;
use gnoblin_wayland::LayoutControl;
use gnoblin_wm::layout::{
    Camera, Generation, Handled, Job, LayoutEffect, LayoutEngine, LayoutFailure, LayoutOutcome, Reply, Ticket,
};

use crate::SessionConfig;

/// The layout name that floats every window. It is never loaded as a module.
pub const FLOATING: &str = "floating";

/// The `layout` section as the session reads it: plain data, so it can travel in a
/// [`SessionConfig`](crate::SessionConfig) to the thread that runs the compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutSettings {
    /// The layout of a workspace with no override.
    pub default: String,
    /// Layout names by output connector.
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
pub struct WorkspaceLayouts {
    default: RefCell<String>,
    outputs: RefCell<BTreeMap<String, String>>,
    workspaces: RefCell<BTreeMap<WorkspaceId, String>>,
    host: Rc<RefCell<Host>>,
    failed: RefCell<BTreeSet<String>>,
}

impl WorkspaceLayouts {
    /// Loads every layout the settings name. A layout that cannot be loaded is named
    /// in a warning, and every workspace that asks for it floats.
    pub fn new(settings: LayoutSettings) -> Self {
        let mut dirs = settings.module_dirs;
        dirs.push(installed_module_dir(&default_prefix(
            std::env::var_os("GNOBLIN_PREFIX").as_deref(),
        )));
        let mut host = Host::new(HostConfig::new(dirs));
        let mut failed = BTreeSet::new();
        let names: BTreeSet<&str> = std::iter::once(settings.default.as_str())
            .chain(settings.outputs.iter().map(|(_, layout)| layout.as_str()))
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
            default: RefCell::new(settings.default),
            outputs: RefCell::new(settings.outputs.into_iter().collect()),
            workspaces: RefCell::new(
                settings
                    .workspaces
                    .into_iter()
                    .filter_map(|(key, layout)| key.parse::<u64>().ok().map(|id| (WorkspaceId::new(id), layout)))
                    .collect(),
            ),
            host: Rc::new(RefCell::new(host)),
            failed: RefCell::new(failed),
        }
    }

    /// The layout a workspace on `output` uses: its runtime override, else its
    /// configured workspace override, else the override of `output`, else `default`.
    /// A name that is floating or failed to load resolves to [`FLOATING`].
    pub fn resolve(&self, workspace: WorkspaceId, output: Option<&str>) -> String {
        let name = self
            .workspaces
            .borrow()
            .get(&workspace)
            .cloned()
            .or_else(|| output.and_then(|output| self.outputs.borrow().get(output).cloned()))
            .unwrap_or_else(|| self.default.borrow().clone());
        if name == FLOATING || self.failed.borrow().contains(&name) {
            FLOATING.to_owned()
        } else {
            name
        }
    }

    /// The engine for `workspace` on `output`. Floating workspaces get
    /// [`FloatingEngine`]; others get a Lua engine over the shared host.
    pub fn engine_for(&self, workspace: WorkspaceId, output: Option<&str>) -> Box<dyn LayoutEngine> {
        let name = self.resolve(workspace, output);
        if name == FLOATING {
            Box::new(FloatingEngine::default())
        } else {
            Box::new(LuaEngine::new(self.host.clone(), &name, workspace, Gaps::default()))
        }
    }

    /// Loads `name` into the host when it is not registered yet, so a runtime choice
    /// names a real layout or is refused. A layout that fails to load is refused too.
    fn ensure_layout(&self, name: &str) -> Result<(), String> {
        if name == FLOATING {
            return Ok(());
        }
        if self.failed.borrow().contains(name) {
            return Err(format!("layout `{name}` failed to load"));
        }
        let mut host = self.host.borrow_mut();
        match host.register(name, &format!("gnoblin.layouts.{name}"), Data::Table(BTreeMap::new())) {
            Ok(()) => Ok(()),
            Err(failure) => {
                warn!(layout = name, reason = ?failure.reason, message = %failure.message, "[layout] runtime layout cannot be loaded");
                if failure.message.contains("not found") {
                    return Err(format!("unknown layout: `{name}` has no layout module"));
                }
                self.failed.borrow_mut().insert(name.to_owned());
                Err(format!("layout `{name}` cannot be loaded: {}", failure.message))
            }
        }
    }
}

impl LayoutControl for WorkspaceLayouts {
    fn engine(&self, workspace: WorkspaceId, output: Option<&str>) -> Box<dyn LayoutEngine> {
        self.engine_for(workspace, output)
    }

    fn set_workspace(&self, workspace: WorkspaceId, name: &str) -> Result<(), String> {
        self.ensure_layout(name)?;
        self.workspaces.borrow_mut().insert(workspace, name.to_owned());
        Ok(())
    }

    fn set_output(&self, output: &str, name: &str) -> Result<(), String> {
        self.ensure_layout(name)?;
        self.outputs.borrow_mut().insert(output.to_owned(), name.to_owned());
        Ok(())
    }

    fn layout_name(&self, workspace: WorkspaceId, output: Option<&str>) -> String {
        self.resolve(workspace, output)
    }

    fn output_layout(&self, output: &str) -> String {
        let name = self
            .outputs
            .borrow()
            .get(output)
            .cloned()
            .unwrap_or_else(|| self.default.borrow().clone());
        if name == FLOATING || self.failed.borrow().contains(&name) {
            FLOATING.to_owned()
        } else {
            name
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
    fn kind(&self) -> gnoblin_wm::layout::LayoutKind {
        gnoblin_wm::layout::LayoutKind::Floating
    }

    fn extent(&self) -> gnoblin_wm::layout::LayoutExtent {
        gnoblin_wm::layout::LayoutExtent::Output
    }

    fn declares(&self, _command: &str) -> bool {
        false
    }

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
            Job::Request(_) | Job::Command(..) => Reply::Declined,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use gnoblin_lua::layout::{Gaps, Host, HostConfig, LuaEngine};
    use gnoblin_types::{OutputId, WindowId, WorkspaceId};
    use gnoblin_wm::layout::{Job, LayoutEngine, LayoutOutput, LayoutWindow, Params, Slot};

    use super::{FLOATING, FloatingEngine, LayoutSettings, WorkspaceLayouts};

    /// The repository's shipped Lua layouts, which `gnoblin.layouts.floating` lives in.
    fn shipped_modules() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/data/lua")
    }

    fn params() -> Params {
        let output = LayoutOutput {
            id: OutputId::new(1),
            name: "TEST-1".to_owned(),
            scale: gnoblin_types::Scale::default(),
        };
        let windows = [(1, 300.0, 200.0), (2, 120.0, 80.0), (3, 640.0, 480.0)]
            .into_iter()
            .map(|(id, width, height)| LayoutWindow {
                id: WindowId::new(id),
                width,
                height,
            })
            .collect();
        Params::new(output, 1280.0, 720.0, windows).expect("valid params")
    }

    /// Runs `engine` on `params` as an arrange and returns the layout the wm would apply.
    fn arrange(engine: Box<dyn LayoutEngine>, params: &Params) -> gnoblin_wm::layout::Layout {
        let mut slot = Slot::new(engine);
        slot.submit(Job::Arrange(params.clone()));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match slot.collect(deadline) {
                gnoblin_wm::layout::Collected::Settled(layout) => return layout,
                gnoblin_wm::layout::Collected::Pending => assert!(Instant::now() < deadline, "the arrange timed out"),
                other => panic!("the arrange did not settle: {other:?}"),
            }
        }
    }

    /// The floating module is equivalent to the native `FloatingEngine`: both give the
    /// same arrange reply (no placements, a float per window) and decline geometry
    /// requests, so a floating workspace behaves the same with or without a Lua VM.
    /// A module in a configured directory wins over the shipped layout of the same
    /// name. The user copy here does not load, so the name fails: the shipped
    /// `monocle` would load, and resolving to floating shows the user copy was read.
    #[test]
    fn a_configured_module_overrides_the_shipped_layout_of_the_same_name() {
        let user = tempfile::tempdir().expect("a temporary module directory");
        let layouts = user.path().join("gnoblin/layouts");
        std::fs::create_dir_all(&layouts).expect("the layouts directory");
        std::fs::write(layouts.join("monocle.lua"), "this is not lua (").expect("the broken module");
        let settings = LayoutSettings {
            default: "monocle".to_owned(),
            module_dirs: vec![user.path().to_path_buf()],
            ..LayoutSettings::default()
        };
        let layouts = WorkspaceLayouts::new(settings);
        assert_eq!(layouts.resolve(WorkspaceId::new(1), None), FLOATING);

        let shipped = WorkspaceLayouts::new(LayoutSettings {
            default: "monocle".to_owned(),
            module_dirs: vec![shipped_modules()],
            ..LayoutSettings::default()
        });
        assert_eq!(shipped.resolve(WorkspaceId::new(1), None), "monocle");
    }

    #[test]
    fn native_floating_matches_the_shipped_floating_module() {
        let params = params();
        let host = Host::new(HostConfig::new(vec![shipped_modules()]));
        let host = std::rc::Rc::new(std::cell::RefCell::new(host));
        host.borrow_mut()
            .register(
                "floating",
                "gnoblin.layouts.floating",
                gnoblin_lua::layout::Data::Table(Default::default()),
            )
            .expect("the shipped floating module loads");
        let lua = LuaEngine::new(host, "floating", WorkspaceId::new(1), Gaps::default());

        let native = arrange(Box::new(FloatingEngine::default()), &params);
        let shipped = arrange(Box::new(lua), &params);
        assert_eq!(native.placements, shipped.placements);
        assert_eq!(native.effects, shipped.effects);
        assert!(native.placements.is_empty(), "floating places nothing");
        assert_eq!(native.effects.len(), 3, "every tiled window floats");
    }
}
