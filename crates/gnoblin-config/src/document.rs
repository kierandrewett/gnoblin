//! The configuration document: the typed sections built from a value tree.
//!
//! The Lua host evaluates `init.lua`, layers the files it loaded, and hands the
//! result here as a [`Value`]. This crate never evaluates Lua and never reads the
//! config file: it owns what a setting *means*, so a misspelled key, an
//! out-of-range number or a deprecated spelling is caught once, with a path and
//! an accepted-values hint, before any subsystem acts on it.
//!
//! Two properties are deliberate.
//!
//! *All problems at once.* [`Config::from_value`] collects every problem and
//! returns them in one [`Report`]; a user fixes a document, not a document per
//! reload.
//!
//! *One stored spelling.* Parsing rewrites a deprecated spelling to its current
//! one in the tree it was given, so [`Config::document`] is the canonical
//! accepted document. The last-good store writes that tree, which is why a
//! document saved by this version never needs the "an older Gnoblin wrote this"
//! check the C core performs on load.
//!
//! Six sections are *carried*: `extensions`, `keybindings`, `portals`,
//! `autostart`, `window-rules` and `animations`. Their values are callbacks and rule
//! declarations, which the Lua host and the window-management crate own; this
//! layer keeps them in the document, rewrites nothing inside them, and validates
//! nothing inside them. Carrying them is not the same as accepting them: a
//! consumer that cannot use a rule still refuses it, and that refusal happens
//! where the meaning is known.

use std::collections::BTreeMap;

use crate::engine::{Parser, removed, section_table, unknown_keys};
use crate::generated::{
    Appearance, Auth, CaptureSettings, Compat, Compositor, Cursor, FrameRenderers, Gesture, Input, InputSources,
    KeyringConfig, LayerShell, Layers, Layout, Location, Logging, Monitors, Prompts, Protocols, Shortcuts,
    VisualAccessibility, WindowManagement, Workspace, Xwayland, parse_appearance, parse_auth, parse_capture_settings,
    parse_compat, parse_compositor, parse_cursor, parse_frame_renderers, parse_input, parse_input_sources,
    parse_keyring_config, parse_layer_shell, parse_layers, parse_layout, parse_location, parse_logging, parse_monitors,
    parse_prompts, parse_protocols, parse_shortcuts, parse_touchpad_gestures, parse_visual_accessibility,
    parse_window_management, parse_workspaces, parse_xwayland,
};
use crate::model::Deprecation;
use crate::path::Path;
use crate::problem::{Fault, Problem, Report};
use crate::value::Value;

/// Every top-level key the document accepts, generated from the schema.
///
/// The generated list is compared against the recorded inventory in
/// `tests/parity/contracts/config-keys.json` by `tests/contract.rs`. `shell` is
/// absent on purpose: it is refused with an explanation instead of an
/// unknown-key hint.
pub use crate::generated::TOP_LEVEL_KEYS;

/// Sections this layer keeps but does not interpret, generated from the schema.
pub use crate::generated::CARRIED_SECTIONS;

/// A validated configuration document.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `appearance`: the colour scheme and accent every toolkit is told about.
    pub appearance: Option<Appearance>,
    /// `compositor`: what the compositor draws itself and how.
    pub compositor: Option<Compositor>,
    /// `cursor`: the cursor theme and size.
    pub cursor: Option<Cursor>,
    /// `frame-renderers`: the named frame renderer commands.
    pub frame_renderers: Option<FrameRenderers>,
    /// `accessibility`: the visual accessibility section, a magnifier and a colour filter.
    pub accessibility: Option<VisualAccessibility>,
    /// `input`: pointer, keyboard and tablet settings.
    pub input: Option<Input>,
    /// `input-sources`: the switchable layouts and input methods.
    pub input_sources: Option<InputSources>,
    /// `layer-shell`: how layer surfaces affect focus.
    pub layer_shell: Option<LayerShell>,
    /// `layers`: the per-layer overrides, keyed by full layer name.
    pub layers: Option<Layers>,
    /// `layout`: which layout engine arranges each workspace's windows.
    pub layout: Option<Layout>,
    /// `location`: the GeoClue policy.
    pub location: Option<Location>,
    /// `monitors`: preferences that apply to connected monitors.
    pub monitors: Option<Monitors>,
    /// `prompts`: whether Gnoblin serves passphrase prompts.
    pub prompts: Option<Prompts>,
    /// `keyring`: which program provides the Secret Service on the session bus.
    pub keyring: Option<KeyringConfig>,
    /// `compat`: which compatibility adapters Gnoblin serves.
    pub compat: Option<Compat>,
    /// `capture`: how captures treat secure layers.
    pub capture: Option<CaptureSettings>,
    /// `logging`: what is logged, where the database is and how long it is kept.
    pub logging: Option<Logging>,
    /// `auth`: whether Gnoblin is the polkit agent.
    pub auth: Option<Auth>,
    /// `protocols`: the protocol toggles.
    pub protocols: Option<Protocols>,
    /// `shortcuts`: the keyboard shortcuts that run a command, by name.
    pub shortcuts: Option<Shortcuts>,
    /// `touchpad-gestures`: the gesture bindings, in declaration order.
    pub touchpad_gestures: Vec<Gesture>,
    /// `window-management`: stacking, focus and placement policy.
    pub window_management: Option<WindowManagement>,
    /// `workspaces`: the declared workspaces, in declaration order.
    pub workspaces: Vec<Workspace>,
    /// `xwayland`: how the X11 server behaves.
    pub xwayland: Option<Xwayland>,
    /// The accepted document, with deprecated spellings already resolved.
    document: Value,
    /// The deprecated spellings this document used, in the order they appeared.
    deprecations: Vec<Deprecation>,
}

impl Config {
    /// Decode and validate a document the runtime committed as JSON bytes.
    ///
    /// The runtime validates a document before it commits it, so a refusal here means the
    /// bytes were not that document; the message names the decode or validation failure.
    ///
    /// # Errors
    ///
    /// A message when the bytes are not JSON, are not a configuration tree, or fail
    /// validation.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Config, String> {
        let json: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let value = Value::from_json(&json).ok_or_else(|| "the document is not a configuration tree".to_owned())?;
        Config::from_value(value).map_err(|report| report.to_string())
    }

    /// Validate a document tree, reporting every problem at once.
    ///
    /// The tree is taken by value and rewritten in place where a deprecated
    /// spelling is accepted, so the config that comes back carries the canonical
    /// spellings and [`Config::document`] can be stored as-is.
    pub fn from_value(mut document: Value) -> Result<Config, Report> {
        // A tree with a non-finite number or a branch past the depth limit is
        // unusable before any section is read, and reading the sections first
        // would bury the one problem that explains everything else (an infinite
        // `speed` would also be reported as an out-of-range value).
        let tree_problems = document.tree_problems();
        if !tree_problems.is_empty() {
            return Err(Report::new(tree_problems));
        }
        let mut out = Parser::new();
        let mut config = Config::empty();
        match &mut document {
            Value::Map(entries) => {
                let root = Path::root();
                removed(
                    entries,
                    "shell",
                    &root,
                    &mut out,
                    "configure the shell UI in the shell client, and use gnoblin.animation and window rules for compositor animations",
                );
                unknown_keys(entries, &root, &mut out, TOP_LEVEL_KEYS);
                config.appearance = section_table(entries, "appearance", &root, &mut out)
                    .map(|table| parse_appearance(table, &root.key("appearance"), &mut out));
                config.compositor = section_table(entries, "compositor", &root, &mut out)
                    .map(|table| parse_compositor(table, &root.key("compositor"), &mut out));
                config.cursor = section_table(entries, "cursor", &root, &mut out)
                    .map(|table| parse_cursor(table, &root.key("cursor"), &mut out));
                config.frame_renderers = section_table(entries, "frame-renderers", &root, &mut out)
                    .map(|table| parse_frame_renderers(table, &root.key("frame-renderers"), &mut out));
                config.input = section_table(entries, "input", &root, &mut out)
                    .map(|table| parse_input(table, &root.key("input"), &mut out));
                config.input_sources = section_table(entries, "input-sources", &root, &mut out)
                    .map(|table| parse_input_sources(table, &root.key("input-sources"), &mut out));
                config.accessibility = section_table(entries, "accessibility", &root, &mut out)
                    .map(|table| parse_visual_accessibility(table, &root.key("accessibility"), &mut out));
                config.layer_shell = section_table(entries, "layer-shell", &root, &mut out)
                    .map(|table| parse_layer_shell(table, &root.key("layer-shell"), &mut out));
                config.layers = section_table(entries, "layers", &root, &mut out)
                    .map(|table| parse_layers(table, &root.key("layers"), &mut out));
                config.layout = section_table(entries, "layout", &root, &mut out)
                    .map(|table| parse_layout(table, &root.key("layout"), &mut out));
                config.location = section_table(entries, "location", &root, &mut out)
                    .map(|table| parse_location(table, &root.key("location"), &mut out));
                config.monitors = section_table(entries, "monitors", &root, &mut out)
                    .map(|table| parse_monitors(table, &root.key("monitors"), &mut out));
                config.prompts = section_table(entries, "prompts", &root, &mut out)
                    .map(|table| parse_prompts(table, &root.key("prompts"), &mut out));
                config.keyring = section_table(entries, "keyring", &root, &mut out)
                    .map(|table| parse_keyring_config(table, &root.key("keyring"), &mut out));
                config.compat = section_table(entries, "compat", &root, &mut out)
                    .map(|table| parse_compat(table, &root.key("compat"), &mut out));
                config.capture = section_table(entries, "capture", &root, &mut out)
                    .map(|table| parse_capture_settings(table, &root.key("capture"), &mut out));
                config.logging = section_table(entries, "logging", &root, &mut out)
                    .map(|table| parse_logging(table, &root.key("logging"), &mut out));
                config.auth = section_table(entries, "auth", &root, &mut out)
                    .map(|table| parse_auth(table, &root.key("auth"), &mut out));
                config.protocols = section_table(entries, "protocols", &root, &mut out)
                    .map(|table| parse_protocols(table, &root.key("protocols"), &mut out));
                config.shortcuts = section_table(entries, "shortcuts", &root, &mut out)
                    .map(|table| parse_shortcuts(table, &root.key("shortcuts"), &mut out));
                config.window_management = section_table(entries, "window-management", &root, &mut out)
                    .map(|table| parse_window_management(table, &root.key("window-management"), &mut out));
                config.xwayland = section_table(entries, "xwayland", &root, &mut out)
                    .map(|table| parse_xwayland(table, &root.key("xwayland"), &mut out));
                config.touchpad_gestures = match entries.get_mut("touchpad-gestures") {
                    Some(value) => parse_touchpad_gestures(value, &root.key("touchpad-gestures"), &mut out).gestures,
                    None => Vec::new(),
                };
                config.workspaces = match entries.get_mut("workspaces") {
                    Some(value) => parse_workspaces(value, &root.key("workspaces"), &mut out).workspaces,
                    None => Vec::new(),
                };
                // A declared workspace list fixes the count, so a document that
                // also asks for dynamic workspaces contradicts itself. The
                // problem is anchored on the declaration, which is the newer of
                // the two settings and the one a salvage can drop.
                if entries.contains_key("workspaces")
                    && config
                        .window_management
                        .as_ref()
                        .is_some_and(|management| management.dynamic_workspaces.is_some())
                {
                    out.problem(
                        &root.key("workspaces"),
                        Fault::Conflict {
                            other: root.key("window-management").key("dynamic-workspaces"),
                        },
                    );
                }
            }
            other => out.problem(
                &Path::root(),
                Fault::WrongType {
                    expected: "a table",
                    found: other.kind(),
                },
            ),
        }
        let (problems, deprecations) = out.finish();
        if problems.is_empty() {
            config.document = document;
            config.deprecations = deprecations;
            Ok(config)
        } else {
            Err(Report::new(problems))
        }
    }

    /// The accepted document, with deprecated spellings resolved.
    ///
    /// This is what the object model publishes and what the last-good store
    /// writes, so a caller never has to reconstruct the document from the typed
    /// sections.
    pub fn document(&self) -> &Value {
        &self.document
    }

    /// The deprecated spellings this document used.
    ///
    /// Empty for a document read back from the last-good store, because storing
    /// resolves them.
    pub fn deprecations(&self) -> &[Deprecation] {
        &self.deprecations
    }

    /// The configuration with no sections set, used as the starting point while
    /// a document is read and as the result of a salvage that kept nothing.
    pub(crate) fn empty() -> Config {
        Config {
            appearance: None,
            compositor: None,
            cursor: None,
            accessibility: None,
            frame_renderers: None,
            input: None,
            input_sources: None,
            layer_shell: None,
            layers: None,
            layout: None,
            location: None,
            monitors: None,
            prompts: None,
            keyring: None,
            compat: None,
            capture: None,
            logging: None,
            auth: None,
            protocols: None,
            shortcuts: None,
            touchpad_gestures: Vec::new(),
            window_management: None,
            workspaces: Vec::new(),
            xwayland: None,
            document: Value::Map(BTreeMap::new()),
            deprecations: Vec::new(),
        }
    }

    /// Whether the document set no section at all.
    pub fn is_empty(&self) -> bool {
        self.document.as_table().is_some_and(BTreeMap::is_empty)
    }
}

/// The result of keeping what could be kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Salvaged {
    /// The accepted document, with the sections that held a problem removed.
    pub config: Config,
    /// The problems that were reported for the removed sections, or `None` when
    /// nothing had to be removed.
    pub report: Option<Report>,
}

/// The most sections one salvage will drop before giving up.
///
/// A document cannot need more than one round per top-level key, so this is a
/// safety net against a validation rule that keeps inventing problems, not a
/// budget a real document can spend.
const MAX_SALVAGE_ROUNDS: usize = 32;

/// Keep the sections that validate and drop the ones that do not.
///
/// Login is forgiving in a way a reload is not: a broken section must not cost
/// the user their cursor theme or their keyboard layout. The section is the unit
/// because a section is what a merge replaces: keeping half of a section would
/// leave a mix of two files' values that neither file asked for.
///
/// Dropping is reported, never silent: every problem from every round is in the
/// [`Salvaged::report`], so the recovery panel can name what was ignored.
pub fn salvage(mut document: Value) -> Salvaged {
    let mut problems: Vec<Problem> = Vec::new();
    for _ in 0..MAX_SALVAGE_ROUNDS {
        match Config::from_value(document.clone()) {
            Ok(config) => {
                let report = (!problems.is_empty()).then(|| Report::new(problems));
                return Salvaged { config, report };
            }
            Err(report) => {
                let sections = report.sections();
                problems.extend(report.problems().iter().cloned());
                if sections.is_empty() {
                    // Nothing inside the document could be blamed, so nothing
                    // can be kept: the caller falls back to its own defaults.
                    break;
                }
                if let Value::Map(entries) = &mut document {
                    for section in sections {
                        entries.remove(section);
                    }
                }
            }
        }
    }
    Salvaged {
        config: Config::empty(),
        report: Some(Report::new(problems)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_level_key_list_is_sorted_and_has_no_duplicates() {
        let mut sorted: Vec<&str> = TOP_LEVEL_KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), TOP_LEVEL_KEYS.len(), "a key is listed twice");
        assert!(
            TOP_LEVEL_KEYS.windows(2).all(|pair| pair[0] < pair[1]),
            "the key list is the accepted-values hint, so it is kept sorted"
        );
        assert!(!TOP_LEVEL_KEYS.contains(&"shell"), "shell is refused, not accepted");
        for section in CARRIED_SECTIONS {
            assert!(TOP_LEVEL_KEYS.contains(section), "{section} is carried but not known");
        }
    }

    #[test]
    fn an_empty_document_is_valid_and_empty() {
        let config = Config::from_value(Value::Map(BTreeMap::new())).expect("empty document");
        assert!(config.is_empty());
        assert!(config.deprecations().is_empty());
    }

    #[test]
    fn a_root_that_is_not_a_table_is_one_problem() {
        let report = Config::from_value(Value::List(vec![])).expect_err("a list is not a document");
        assert_eq!(report.problems().len(), 1);
        assert_eq!(report.problems()[0].path, Path::root());
    }
}
