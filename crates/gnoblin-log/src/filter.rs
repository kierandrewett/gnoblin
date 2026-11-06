//! Which components log, and at what level.
//!
//! Every [`info_in!`](crate::info_in) call site owns one [`Component`]. The component holds one
//! atomic level, so a disabled call costs a single relaxed load and formats nothing. The level
//! comes from a [`Spec`], the text `GNOBLIN_LOG` or the `logging` configuration supplies:
//!
//! ```text
//! wayland:debug,compositor:info,*:warn
//! ```
//!
//! An entry is either `component:level`, or a bare level that sets the default. `*` is the same
//! default written explicitly. A component with no entry uses the default, which is `info` when
//! the spec names none.
//!
//! The first call from a call site registers its component in a process-wide registry, which
//! [`components`] lists. [`set_spec`] updates every registered component at once, so a spec
//! installed after some components have already logged still applies to them.

use crate::is_valid_component;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};

use tracing::Level;

/// The level a component logs at, or `None` for off.
pub type Setting = Option<Level>;

/// The level a spec uses when it names none: `info`.
const DEFAULT_LEVEL: Setting = Some(Level::INFO);

/// A parsed `GNOBLIN_LOG` or `logging` level specification.
///
/// Build one with [`Spec::parse`]. The default is what an empty specification means: `info` for
/// every component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The level of every component without an entry of its own, from a bare level or `*`.
    default: Setting,
    /// The per-component levels, in the order the specification named them.
    components: Vec<(String, Setting)>,
}

impl Spec {
    /// A specification with no entries: `info` for every component.
    pub const fn new() -> Self {
        Self {
            default: DEFAULT_LEVEL,
            components: Vec::new(),
        }
    }

    /// Parses a comma-separated specification.
    ///
    /// Blank entries are skipped, so an empty string is the default specification. A bad entry
    /// refuses the whole specification and names the entry, so a typo does not silently change
    /// what is logged.
    ///
    /// # Errors
    ///
    /// The first entry that is not a level, not `component:level`, not a valid component name,
    /// or that names the same component twice.
    pub fn parse(text: &str) -> Result<Self, SpecError> {
        let mut spec = Self::new();
        for raw in text.split(',') {
            let entry = raw.trim();
            if entry.is_empty() {
                continue;
            }
            match entry.split_once(':') {
                None => {
                    spec.default = parse_level(entry)
                        .ok_or_else(|| SpecError::new(entry, "expected a level, or component:level"))?;
                }
                Some((name, level)) => {
                    let name = name.trim();
                    let level = parse_level(level.trim()).ok_or_else(|| {
                        SpecError::new(entry, "the level must be off, error, warn, info, debug or trace")
                    })?;
                    if name == "*" {
                        spec.default = level;
                    } else if !is_valid_component(name) {
                        return Err(SpecError::new(
                            entry,
                            "the component must be 1 to 24 bytes of a-z, 0-9 and '-'",
                        ));
                    } else if spec.components.iter().any(|(known, _)| known == name) {
                        return Err(SpecError::new(entry, "the component is named twice"));
                    } else {
                        spec.components.push((name.to_owned(), level));
                    }
                }
            }
        }
        Ok(spec)
    }

    /// The level of `component` under this specification.
    pub fn level_for(&self, component: &str) -> Setting {
        self.components
            .iter()
            .find(|(name, _)| name == component)
            .map_or(self.default, |(_, level)| *level)
    }

    /// The same levels written as `tracing_subscriber` `EnvFilter` directives.
    ///
    /// The directives target `gnoblin::<component>`, which is the target the logger macros emit
    /// to. Foreign `tracing` events match the default, so they follow the same specification.
    pub fn env_filter_directives(&self) -> String {
        let mut directives = vec![level_name(self.default).to_owned()];
        for (name, level) in &self.components {
            directives.push(format!("gnoblin::{name}={}", level_name(*level)));
        }
        directives.join(",")
    }
}

impl Default for Spec {
    fn default() -> Self {
        Self::new()
    }
}

/// A specification entry that was refused, with the entry and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    entry: String,
    reason: &'static str,
}

impl SpecError {
    fn new(entry: &str, reason: &'static str) -> Self {
        Self {
            entry: entry.to_owned(),
            reason,
        }
    }

    /// The entry that was refused, trimmed.
    pub fn entry(&self) -> &str {
        &self.entry
    }
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid logging filter entry {:?}: {}", self.entry, self.reason)
    }
}

impl std::error::Error for SpecError {}

/// Parses a level name, case-insensitively. `off` is `Some(None)`; an unknown name is `None`.
fn parse_level(text: &str) -> Option<Setting> {
    Some(match text.to_ascii_lowercase().as_str() {
        "off" => None,
        "error" => Some(Level::ERROR),
        "warn" => Some(Level::WARN),
        "info" => Some(Level::INFO),
        "debug" => Some(Level::DEBUG),
        "trace" => Some(Level::TRACE),
        _ => return None,
    })
}

/// The lower-case name `EnvFilter` accepts for a setting.
fn level_name(setting: Setting) -> &'static str {
    match setting {
        None => "off",
        Some(Level::ERROR) => "error",
        Some(Level::WARN) => "warn",
        Some(Level::INFO) => "info",
        Some(Level::DEBUG) => "debug",
        Some(_) => "trace",
    }
}

/// The number a level is stored as: a call at `level` is enabled when `level_code(level)` is at
/// most the component's stored code. Off is `0`, so nothing is at or below it.
const fn level_code(setting: Setting) -> u8 {
    match setting {
        None => 0,
        Some(Level::ERROR) => 1,
        Some(Level::WARN) => 2,
        Some(Level::INFO) => 3,
        Some(Level::DEBUG) => 4,
        Some(_) => 5,
    }
}

/// The code of a call's level, in the same scale as [`level_code`].
const fn call_code(level: Level) -> u8 {
    level_code(Some(level))
}

/// One logging call site's component: its name, its level and whether it has registered.
///
/// Each [`info_in!`](crate::info_in) call expands to a `static` of this type, so the level check
/// reads a field of that static and never a lock. The first check registers it.
#[doc(hidden)]
pub struct Component {
    name: &'static str,
    /// The enabled level, as [`level_code`] numbers it.
    level: AtomicU8,
    /// Whether the component is in the registry and its level has been read from the spec.
    registered: AtomicBool,
}

impl Component {
    /// A component named `name`, not yet registered.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            level: AtomicU8::new(0),
            registered: AtomicBool::new(false),
        }
    }

    /// Whether a call at `level` is enabled for this component.
    ///
    /// After the first call this is one acquire load of the registration flag and one relaxed
    /// load of the level.
    #[inline]
    pub fn enabled(&'static self, level: Level) -> bool {
        if !self.registered.load(Ordering::Acquire) {
            self.register();
        }
        call_code(level) <= self.level.load(Ordering::Relaxed)
    }

    /// The component's name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Adds the component to the registry and reads its level from the current spec.
    ///
    /// Holding the registry lock serialises two first calls, so the second sees the registration
    /// the first made and does not add the component twice.
    #[cold]
    fn register(&'static self) {
        let mut registry = lock();
        if self.registered.load(Ordering::Acquire) {
            return;
        }
        self.level
            .store(level_code(registry.spec.level_for(self.name)), Ordering::Relaxed);
        registry.components.push(self);
        self.registered.store(true, Ordering::Release);
    }
}

/// The process-wide registry: the spec in force and every component that has registered.
struct Registry {
    spec: Spec,
    components: Vec<&'static Component>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    spec: Spec::new(),
    components: Vec::new(),
});

/// Locks the registry. A panic while it is held leaves no half-written state, so a poisoned lock
/// is used as it is.
fn lock() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs `spec` for every component, registered or not.
///
/// Components that have already registered get the new level at once. Components that register
/// later read it when they do.
pub fn set_spec(spec: Spec) {
    let mut registry = lock();
    for component in &registry.components {
        component
            .level
            .store(level_code(spec.level_for(component.name)), Ordering::Relaxed);
    }
    registry.spec = spec;
}

/// A registered component, as [`components`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentInfo {
    /// The component's name, as the call gave it.
    pub name: &'static str,
    /// The level now in force, or `None` for off.
    pub level: Setting,
}

/// Every component that has logged, once per name, sorted by name.
///
/// A component is registered on its first call, so one that never ran is not listed. Several call
/// sites may share a name, and they always hold the same level, so one entry stands for them.
pub fn components() -> Vec<ComponentInfo> {
    let registry = lock();
    let mut by_name = BTreeMap::new();
    for component in &registry.components {
        let code = component.level.load(Ordering::Relaxed);
        by_name.entry(component.name).or_insert_with(|| setting_of(code));
    }
    by_name
        .into_iter()
        .map(|(name, level)| ComponentInfo { name, level })
        .collect()
}

/// The setting a stored code stands for. The inverse of [`level_code`].
fn setting_of(code: u8) -> Setting {
    match code {
        0 => None,
        1 => Some(Level::ERROR),
        2 => Some(Level::WARN),
        3 => Some(Level::INFO),
        4 => Some(Level::DEBUG),
        _ => Some(Level::TRACE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Spec {
        Spec::parse(text).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn an_empty_specification_is_info_for_every_component() {
        for text in ["", "   ", " , ,"] {
            let spec = parsed(text);
            assert_eq!(spec.level_for("wayland"), Some(Level::INFO), "{text:?}");
        }
        assert_eq!(Spec::default(), Spec::new());
    }

    #[test]
    fn a_component_entry_sets_only_that_component() {
        let spec = parsed("wayland:debug,compositor:warn");
        assert_eq!(spec.level_for("wayland"), Some(Level::DEBUG));
        assert_eq!(spec.level_for("compositor"), Some(Level::WARN));
        assert_eq!(spec.level_for("other"), Some(Level::INFO));
    }

    #[test]
    fn a_bare_level_and_a_star_both_set_the_default() {
        assert_eq!(parsed("warn").level_for("anything"), Some(Level::WARN));
        assert_eq!(parsed("*:error").level_for("anything"), Some(Level::ERROR));
        let mixed = parsed("wayland:debug,*:warn");
        assert_eq!(mixed.level_for("wayland"), Some(Level::DEBUG));
        assert_eq!(mixed.level_for("other"), Some(Level::WARN));
    }

    #[test]
    fn off_silences_a_component_and_levels_are_case_insensitive() {
        assert_eq!(parsed("noisy:off").level_for("noisy"), None);
        assert_eq!(parsed("wayland:DEBUG").level_for("wayland"), Some(Level::DEBUG));
    }

    #[test]
    fn a_bad_entry_refuses_the_whole_specification_and_names_it() {
        for (text, entry) in [
            ("wayland=debug", "wayland=debug"),
            ("wayland:loud", "wayland:loud"),
            ("Bad_Name:info", "Bad_Name:info"),
            ("wayland:debug,wayland:info", "wayland:info"),
            ("loud", "loud"),
        ] {
            let error = Spec::parse(text).expect_err(text);
            assert_eq!(error.entry(), entry, "{text}");
            assert!(error.to_string().contains(entry), "{error}");
        }
    }

    #[test]
    fn env_filter_directives_target_the_gnoblin_targets() {
        let spec = parsed("wayland:debug,noisy:off,warn");
        assert_eq!(
            spec.env_filter_directives(),
            "warn,gnoblin::wayland=debug,gnoblin::noisy=off"
        );
    }

    #[test]
    fn a_disabled_level_is_refused_and_an_enabled_one_passes() {
        static QUIET: Component = Component::new("filter-test-quiet");
        let component = &QUIET;
        set_spec(parsed("filter-test-quiet:warn"));
        assert!(component.enabled(Level::ERROR));
        assert!(component.enabled(Level::WARN));
        assert!(!component.enabled(Level::INFO));
        assert!(!component.enabled(Level::TRACE));
        set_spec(Spec::new());
    }

    #[test]
    fn a_registered_component_is_listed_once_with_its_level() {
        static LISTED: Component = Component::new("filter-test-listed");
        let component = &LISTED;
        set_spec(parsed("filter-test-listed:debug"));
        assert!(component.enabled(Level::DEBUG));
        let listed: Vec<_> = components()
            .into_iter()
            .filter(|info| info.name == "filter-test-listed")
            .collect();
        assert_eq!(
            listed,
            [ComponentInfo {
                name: "filter-test-listed",
                level: Some(Level::DEBUG)
            }]
        );
        set_spec(Spec::new());
    }

    #[test]
    fn a_new_spec_reaches_components_that_already_registered() {
        static LATE: Component = Component::new("filter-test-late");
        let component = &LATE;
        assert!(component.enabled(Level::INFO));
        set_spec(parsed("filter-test-late:off"));
        assert!(!component.enabled(Level::ERROR));
        set_spec(Spec::new());
    }
}
