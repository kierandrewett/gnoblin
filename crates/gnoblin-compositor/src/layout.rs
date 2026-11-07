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

use gnoblin_config::{Layout, LayoutOptionValue, LayoutRoot, LayoutSelector, SearchPaths, Store};
use gnoblin_lua::layout::{
    Data, Gaps, Host, HostConfig, HostFailure, LayoutKey, LuaEngine, ModuleRoots, default_prefix, installed_module_dir,
};
use gnoblin_types::WorkspaceId;
use gnoblin_wayland::LayoutControl;
use gnoblin_wm::layout::{
    Camera, Generation, Handled, Job, LayoutEffect, LayoutEngine, LayoutFailure, LayoutOutcome, Reply, Ticket,
};

use crate::SessionConfig;

use crate::log;

/// The layout name that floats every window. It is never loaded as a module.
pub const FLOATING: &str = "floating";

/// The `layout` section as the session reads it: plain data, so it can travel in a
/// [`SessionConfig`](crate::SessionConfig) to the thread that runs the compositor.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSettings {
    /// The layout of a workspace with no override.
    pub default: LayoutSelector,
    /// Layout names by output connector.
    pub outputs: Vec<(String, LayoutSelector)>,
    /// Layout names by workspace id, as the decimal id the wm uses.
    pub workspaces: Vec<(String, LayoutSelector)>,
    /// The directory of the configuration file, which is the user root: a layout name
    /// without the `gnoblin.` prefix loads from here. `None` when no configuration file is
    /// known, so such names fail to load. Set from the store by [`configure_from_store`].
    pub config_dir: Option<PathBuf>,
    /// The installed `share/gnoblin/lua` tree: the shipped root that a `gnoblin.` layout
    /// name loads from. [`Default`] reads it from `GNOBLIN_PREFIX`. Tests set it directly,
    /// because a test process cannot set its own environment safely.
    pub shipped_dir: PathBuf,
}

impl Default for LayoutSettings {
    /// Every workspace floats, the shipped tree is the installed prefix's, and no
    /// configuration directory is known.
    fn default() -> Self {
        Self {
            default: LayoutSelector::Named(FLOATING.to_owned()),
            outputs: Vec::new(),
            workspaces: Vec::new(),
            config_dir: None,
            shipped_dir: installed_module_dir(&default_prefix(std::env::var_os("GNOBLIN_PREFIX").as_deref())),
        }
    }
}

impl LayoutSettings {
    /// The settings of an accepted `layout` section. Overrides with a missing half are
    /// dropped, and an absent `default` is floating.
    fn from_section(section: Layout) -> Self {
        Self {
            default: section
                .default
                .unwrap_or_else(|| LayoutSelector::Named(FLOATING.to_owned())),
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
            ..Self::default()
        }
    }
}

/// Sets `config.layouts` from the last accepted configuration in `store`, read from
/// `source`. A document with no `layout` section keeps every workspace floating.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.layout.clone().unwrap_or_default();
            let mut settings = LayoutSettings::from_section(section);
            // The user root is the directory the accepted document was read from, so a
            // `require` in that document and the layouts it names resolve from one place.
            settings.config_dir = source.parent().map(Path::to_path_buf);
            log::debug!(area = "layout", default = %selector_key(&settings.default), "loaded from the last accepted configuration");
            config.layouts = settings;
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "layout", %error, "no configuration file; every workspace floats")
        }
        Err(error) => log::warn!(area = "layout", %error, "configuration not readable; every workspace floats"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// The session binary calls this before it starts the compositor. With no configuration
/// file there is no accepted document, so every workspace floats.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(area = "layout", "no state directory is set; every workspace floats");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => log::debug!(area = "layout", %error, "no configuration file; every workspace floats"),
    }
}

/// The resolved layout settings for the session's workspaces, with the Lua host that
/// runs them. Built once, inside the thread that runs the compositor.
pub struct WorkspaceLayouts {
    /// Layout name used for any workspace the outputs and workspaces maps do not name.
    default: RefCell<String>,
    /// Layout name per output connector, from the settings' per-output table.
    outputs: RefCell<BTreeMap<String, String>>,
    /// Layout name per workspace, overriding the output and default choices.
    workspaces: RefCell<BTreeMap<WorkspaceId, String>>,
    /// The Lua host that runs the layout modules. Shared with the rest of the compositor.
    host: Rc<RefCell<Host>>,
    /// Layout names that failed to load, so the failure is reported once and not retried.
    failed: RefCell<BTreeSet<String>>,
}

/// Whether `selector` names the floating layout, which is never loaded as a module.
fn is_floating(selector: &LayoutSelector) -> bool {
    matches!(selector, LayoutSelector::Named(name) if name == FLOATING)
}

/// The key a selector's layout is registered and looked up under. A name is its own
/// key. A reference is keyed by its root, module, hash and options, so two references
/// that differ in any of them are distinct layouts. The `root:` prefix keeps a reference
/// key apart from a plain name.
fn selector_key(selector: &LayoutSelector) -> String {
    match selector {
        LayoutSelector::Named(name) => name.clone(),
        LayoutSelector::Reference(reference) => {
            let root = match reference.root {
                LayoutRoot::Shipped => "shipped",
                LayoutRoot::User => "user",
            };
            // A reference without options keeps the key it always had. The options are
            // spelled in with `Debug`, which is deterministic because the map is ordered.
            let options = if reference.options.is_empty() {
                String::new()
            } else {
                format!("{:?}", reference.options)
            };
            format!("{root}:{}#{}{options}", reference.module, reference.hash)
        }
    }
}

/// Registers the layout `selector` names in `host` and returns its key. A reference is
/// checked against its hash by the host before the module runs, so a file that changed
/// after the configuration was accepted is refused rather than loaded.
fn load_selector(host: &mut Host, selector: &LayoutSelector) -> Result<String, HostFailure> {
    let key = selector_key(selector);
    match selector {
        LayoutSelector::Named(name) => host.register(name, name, Data::Table(BTreeMap::new()))?,
        LayoutSelector::Reference(reference) => host.register_reference(
            &key,
            matches!(reference.root, LayoutRoot::Shipped),
            &reference.module,
            &reference.hash,
            Data::Table(
                reference
                    .options
                    .iter()
                    .map(|(name, value)| (LayoutKey::String(name.clone()), option_data(value)))
                    .collect(),
            ),
        )?,
    }
    Ok(key)
}

/// A layout option as the Lua value a module receives.
fn option_data(value: &LayoutOptionValue) -> Data {
    match value {
        LayoutOptionValue::Bool(value) => Data::Boolean(*value),
        LayoutOptionValue::Integer(value) => Data::Integer(*value),
        LayoutOptionValue::Float(value) => Data::Number(*value),
        LayoutOptionValue::Text(value) => Data::String(value.clone()),
    }
}

/// The options a layout reference carries on the wire, in the form the configuration
/// uses. The wire holds the plain data `ref:with()` encoded. Empty bytes mean no options.
/// A layout option is a scalar, so a table or list is refused here, as the configuration
/// refuses it.
pub fn reference_options(bytes: &[u8]) -> Result<BTreeMap<String, LayoutOptionValue>, String> {
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let Data::Table(entries) = Data::decode(bytes).map_err(|error| error.0)? else {
        return Err("layout options must be a table".to_owned());
    };
    entries
        .into_iter()
        .map(|(key, value)| {
            let LayoutKey::String(name) = key else {
                return Err("layout option names must be text".to_owned());
            };
            let value = match value {
                Data::Boolean(value) => LayoutOptionValue::Bool(value),
                Data::Integer(value) => LayoutOptionValue::Integer(value),
                Data::Number(value) if value.is_finite() => LayoutOptionValue::Float(value),
                Data::String(value) => LayoutOptionValue::Text(value),
                Data::Number(_) | Data::Table(_) => {
                    return Err(format!(
                        "layout option `{name}` must be a boolean, finite number or text"
                    ));
                }
            };
            Ok((name, value))
        })
        .collect()
}

impl WorkspaceLayouts {
    /// Loads every layout the settings name. A layout that cannot be loaded is named
    /// in a warning, and every workspace that asks for it floats.
    pub fn new(settings: LayoutSettings) -> Self {
        // The shipped root is the settings' installed tree. The user root is the
        // configuration directory, when the session knows one. Both are fixed, so a
        // layout resolves the same way whatever document names it.
        let roots = ModuleRoots {
            shipped: settings.shipped_dir.clone(),
            user: settings.config_dir,
        };
        let mut host = Host::new(HostConfig::new(roots));
        let mut failed = BTreeSet::new();
        // Each distinct layout is loaded once, under its key. A failed layout is recorded
        // by key, so every workspace that names it floats.
        let mut selectors: BTreeMap<String, &LayoutSelector> = BTreeMap::new();
        for selector in std::iter::once(&settings.default)
            .chain(settings.outputs.iter().map(|(_, selector)| selector))
            .chain(settings.workspaces.iter().map(|(_, selector)| selector))
            .filter(|selector| !is_floating(selector))
        {
            selectors.entry(selector_key(selector)).or_insert(selector);
        }
        for (key, selector) in selectors {
            if let Err(failure) = load_selector(&mut host, selector) {
                log::warn!(area = "layout", layout = %key, reason = ?failure.reason, message = %failure.message, "layout cannot be loaded; workspaces that use it float");
                failed.insert(key);
            }
        }
        Self {
            default: RefCell::new(selector_key(&settings.default)),
            outputs: RefCell::new(
                settings
                    .outputs
                    .iter()
                    .map(|(output, selector)| (output.clone(), selector_key(selector)))
                    .collect(),
            ),
            workspaces: RefCell::new(
                settings
                    .workspaces
                    .into_iter()
                    .filter_map(|(key, selector)| {
                        key.parse::<u64>()
                            .ok()
                            .map(|id| (WorkspaceId::new(id), selector_key(&selector)))
                    })
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

    /// Loads the layout `selector` names into the host when it is not registered yet, so a
    /// runtime choice names a real layout or is refused. Returns the key the layout runs
    /// under. A layout that fails to load is refused too.
    fn ensure_layout(&self, selector: &LayoutSelector) -> Result<String, String> {
        if is_floating(selector) {
            return Ok(FLOATING.to_owned());
        }
        let key = selector_key(selector);
        if self.failed.borrow().contains(&key) {
            return Err(format!("layout `{key}` failed to load"));
        }
        let mut host = self.host.borrow_mut();
        // A layout already registered keeps its registration. The key names the options
        // too, so a registered key is the same module at the same hash with the same
        // options, and reloading it would only repeat the load.
        if host.is_registered(&key) {
            return Ok(key);
        }
        match load_selector(&mut host, selector) {
            Ok(_) => Ok(key),
            Err(failure) => {
                log::warn!(area = "layout", layout = %key, reason = ?failure.reason, message = %failure.message, "runtime layout cannot be loaded");
                if failure.reason == gnoblin_lua::layout::LayoutReason::UnknownModule {
                    return Err(format!("unknown layout: `{key}` has no layout module"));
                }
                self.failed.borrow_mut().insert(key.clone());
                Err(format!("layout `{key}` cannot be loaded: {}", failure.message))
            }
        }
    }
}

impl LayoutControl for WorkspaceLayouts {
    fn engine(&self, workspace: WorkspaceId, output: Option<&str>) -> Box<dyn LayoutEngine> {
        self.engine_for(workspace, output)
    }

    fn set_workspace(&self, workspace: WorkspaceId, selector: &LayoutSelector) -> Result<(), String> {
        let key = self.ensure_layout(selector)?;
        self.workspaces.borrow_mut().insert(workspace, key);
        Ok(())
    }

    fn set_output(&self, output: &str, selector: &LayoutSelector) -> Result<(), String> {
        let key = self.ensure_layout(selector)?;
        self.outputs.borrow_mut().insert(output.to_owned(), key);
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
    /// The last arrange's outcome, held until the wm collects it. `None` until one runs.
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
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use gnoblin_config::{LayoutOptionValue, LayoutRoot, LayoutSelector};
    use gnoblin_lua::layout::{Data, Gaps, Host, HostConfig, LayoutKey, LuaEngine, MAX_ENCODED_LEN, ModuleRoots};
    use gnoblin_types::{OutputId, WindowId, WorkspaceId};
    use gnoblin_wm::layout::{Job, LayoutEngine, LayoutOutput, LayoutWindow, Params, Slot};

    use super::{FloatingEngine, reference_options, selector_key};

    /// The repository's shipped Lua layouts, which `gnoblin.layouts.floating` lives in.
    fn shipped_modules() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/lua")
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
    #[test]
    fn native_floating_matches_the_shipped_floating_module() {
        let params = params();
        let roots = ModuleRoots {
            shipped: shipped_modules(),
            user: None,
        };
        let host = Host::new(HostConfig::new(roots));
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

    /// The options a reference carries survive the wire: what `ref:with()` encodes, the
    /// compositor reads back as the same typed options the configuration would hold.
    #[test]
    fn reference_options_round_trip_through_the_wire() {
        let table = Data::Table(
            [
                (LayoutKey::String("gap".to_owned()), Data::Integer(12)),
                (LayoutKey::String("ratio".to_owned()), Data::Number(0.5)),
                (LayoutKey::String("dense".to_owned()), Data::Boolean(true)),
                (LayoutKey::String("name".to_owned()), Data::String("main".to_owned())),
            ]
            .into_iter()
            .collect(),
        );
        let bytes = table.encode().expect("a small options table encodes");
        let options = reference_options(&bytes).expect("the options decode");
        assert_eq!(options.get("gap"), Some(&LayoutOptionValue::Integer(12)));
        assert_eq!(options.get("ratio"), Some(&LayoutOptionValue::Float(0.5)));
        assert_eq!(options.get("dense"), Some(&LayoutOptionValue::Bool(true)));
        assert_eq!(options.get("name"), Some(&LayoutOptionValue::Text("main".to_owned())));
        assert_eq!(options.len(), 4);
    }

    /// Empty bytes are the wire form of a reference without options.
    #[test]
    fn empty_options_bytes_mean_no_options() {
        assert!(reference_options(&[]).expect("empty bytes are no options").is_empty());
    }

    /// A layout option is a scalar, so a nested table is refused, as the configuration
    /// refuses it.
    #[test]
    fn a_table_option_is_refused_on_the_wire() {
        let nested = Data::Table(
            [(
                LayoutKey::String("inner".to_owned()),
                Data::Table([(LayoutKey::Integer(1), Data::Integer(2))].into_iter().collect()),
            )]
            .into_iter()
            .collect(),
        );
        let bytes = nested.encode().expect("a nested table encodes");
        let error = reference_options(&bytes).expect_err("a nested table is refused");
        assert!(error.contains("inner"), "the refusal names the option: {error}");
    }

    /// The options are a table, so a bare scalar is refused rather than read as none.
    #[test]
    fn a_scalar_options_value_is_refused() {
        let bytes = Data::Integer(3).encode().expect("a scalar encodes");
        let error = reference_options(&bytes).expect_err("a scalar is refused");
        assert_eq!(error, "layout options must be a table");
    }

    /// Bytes past the bound are refused before any option is read.
    #[test]
    fn oversize_options_bytes_are_refused() {
        let bytes = vec![1u8; MAX_ENCODED_LEN + 1];
        assert!(reference_options(&bytes).is_err(), "oversize options are refused");
    }

    /// Two references that differ only in options must not share a compiled layout.
    #[test]
    fn selector_key_separates_references_by_options() {
        let reference = |options: BTreeMap<String, LayoutOptionValue>| {
            LayoutSelector::Reference(gnoblin_config::LayoutReference {
                root: LayoutRoot::Shipped,
                module: "gnoblin.layouts.tile".to_owned(),
                hash: "00".to_owned(),
                options,
            })
        };
        let plain = selector_key(&reference(BTreeMap::new()));
        let mut gapped = BTreeMap::new();
        gapped.insert("gap".to_owned(), LayoutOptionValue::Integer(4));
        let with_gap = selector_key(&reference(gapped.clone()));
        gapped.insert("gap".to_owned(), LayoutOptionValue::Integer(8));
        let wider_gap = selector_key(&reference(gapped));
        assert_ne!(plain, with_gap);
        assert_ne!(with_gap, wider_gap);
    }
}
