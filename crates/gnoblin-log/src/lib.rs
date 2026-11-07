//! The logging facade every gnoblin crate depends on.
//!
//! A crate defines its loggers once, with [`logger!`], and logs through the module it names:
//!
//! ```ignore
//! gnoblin_log::logger!(log = "wayland");
//! log::info!("surface {id} mapped", id = 3);
//! ```
//!
//! The component literal is checked at compile time: 1 to 24 bytes of `a-z`, `0-9` and `-`, with
//! no leading or trailing `-`. A sub-area of a crate is its own logger with its own component, so
//! `logger!(wire = "wayland-wire")` gives `wire::info!`. The level macros are `error!`, `warn!`,
//! `info!`, `debug!` and `trace!`, and `enabled!` asks whether a level is on. A disabled call
//! formats nothing.
//!
//! The macros take the arguments of their `tracing` counterparts, including fields and `%`/`?`
//! sigils. Every event carries its component in a `component` field, which [`event`] reads. An
//! event from a crate that does not use these macros has no such field, so it gets the component
//! `ext` and keeps its target. This crate depends on `tracing` alone. The console sink lives in
//! the `console` module, behind the `console` feature; the other sinks follow it.

/// Best-effort scrubbing of secrets from free text. See the module documentation.
pub mod redact;

/// The shape of one logged event, read once and shared by every sink.
pub mod event;

/// Which components log, and at what level: the specification and the process-wide registry.
pub mod filter;

/// The console sink: one line per event on stderr. Needs the `console` feature.
#[cfg(feature = "console")]
pub mod console;

/// The database sink: one SQLite row per event. Needs the `store` feature.
#[cfg(feature = "store")]
pub mod store;

/// The record a logged event becomes, and the sink that stores batches of them. Always built: it
/// has no dependencies, so a storage crate can implement [`record::RecordSink`].
pub mod record;

/// The queue between the logging call and a sink, and the layer that feeds it. Needs the
/// `pipeline` feature.
#[cfg(feature = "pipeline")]
pub mod pipeline;

/// The journald sink: one structured entry per event in the systemd journal. Needs the `journal`
/// feature.
#[cfg(feature = "journal")]
pub mod journal;

/// Installs the process-wide subscriber for a binary. Needs the `init` feature.
#[cfg(feature = "init")]
pub mod init;

/// The `tracing` crate, for the sinks and the composition, which need its subscriber types.
///
/// Crates other than this one must not use it to log; they use the macros.
#[doc(hidden)]
pub use tracing as __tracing;

/// The levels a call can name, for [`enabled!`]: `Level::DEBUG` and the rest.
pub use tracing::Level;

/// The longest component name, in bytes.
const MAX_COMPONENT_LEN: usize = 24;

/// Returns `name` when it is a valid component, and fails compilation when it is not.
///
/// Only [`__emit!`] calls this, in a `static`, so an invalid name is a compile error.
#[doc(hidden)]
pub const fn __checked_component(name: &'static str) -> &'static str {
    if is_valid_component(name) {
        name
    } else {
        panic!("gnoblin_log: a component is 1 to 24 bytes of a-z, 0-9 and '-', with no leading or trailing '-'")
    }
}

/// Whether `name` is a component: 1 to 24 bytes of `a-z`, `0-9` and `-`, neither end a `-`.
const fn is_valid_component(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_COMPONENT_LEN {
        return false;
    }
    if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
            return false;
        }
        index += 1;
    }
    true
}

/// The process role a gnoblin binary runs as.
///
/// The console and the database both record the role, so one log tells the compositor's lines
/// from the guardian's even though both are written by the same binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRole {
    /// The compositor, running in the foreground.
    Compositor,
    /// The guardian, which spawns the other roles and keeps the session alive.
    Guardian,
    /// The runtime, which runs the Lua configuration.
    Runtime,
    /// The services, which host the D-Bus service crates.
    Services,
    /// The xdg-desktop-portal backend.
    Portal,
}

impl ProcessRole {
    /// The role's name as recorded in the log.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Compositor => "compositor",
            Self::Guardian => "guardian",
            Self::Runtime => "runtime",
            Self::Services => "services",
            Self::Portal => "portal",
        }
    }
}

/// Emits one `tracing` event at `$level` for `$component`, if that call site is enabled.
///
/// Every expansion owns a `static` [`filter::Component`], so the enabled check is one relaxed
/// load and a disabled call formats nothing. The event targets `gnoblin::<component>`, so an
/// `EnvFilter` built from the specification selects it, and it carries a `component` field that
/// the sinks read.
///
/// This is an internal entry point; a [`logger!`] module's level macros call it instead.
#[doc(hidden)]
#[macro_export]
macro_rules! __emit {
    ($component:literal, $level:ident, $($rest:tt)+) => {{
        static SITE: $crate::filter::Component =
            $crate::filter::Component::new($crate::__checked_component($component));
        if SITE.enabled($crate::__tracing::Level::$level) {
            $crate::__tracing::event!(
                target: concat!("gnoblin::", $component),
                $crate::__tracing::Level::$level,
                component = $component,
                $($rest)+
            );
        }
    }};
}

/// Defines a logger: a module named `name` whose `error!`, `warn!`, `info!`, `debug!` and
/// `trace!` macros log to `component`, and whose `enabled!` asks whether a level is on.
///
/// ```ignore
/// gnoblin_log::logger!(log = "wayland");
/// log::info!("surface {id} mapped", id = 3);
/// ```
///
/// The visibility goes before the name: `logger!(pub(crate) log = "wayland")`. One crate may
/// define several, such as `log` and `wire`. The component literal is checked when the module is
/// built: a bad id fails the build at this call, with the rule it breaks. The message does not
/// name the id, since a const panic cannot format one; the error points at the literal.
#[macro_export]
macro_rules! logger {
    ($vis:vis $name:ident = $component:literal) => {
        $crate::__logger!(($) $vis $name $component);
    };
}

/// Expands a [`logger!`] into its module. `$d` is a literal `$`, which the nested macros need
/// because they are written inside this one.
#[doc(hidden)]
#[macro_export]
macro_rules! __logger {
    (($d:tt) $vis:vis $name:ident $component:literal) => {
        const _: () = {
            $crate::__checked_component($component);
        };
        $vis mod $name {
            macro_rules! __error {
                ($d($args:tt)+) => { $crate::__emit!($component, ERROR, $d($args)+) };
            }
            macro_rules! __warn {
                ($d($args:tt)+) => { $crate::__emit!($component, WARN, $d($args)+) };
            }
            macro_rules! __info {
                ($d($args:tt)+) => { $crate::__emit!($component, INFO, $d($args)+) };
            }
            macro_rules! __debug {
                ($d($args:tt)+) => { $crate::__emit!($component, DEBUG, $d($args)+) };
            }
            macro_rules! __trace {
                ($d($args:tt)+) => { $crate::__emit!($component, TRACE, $d($args)+) };
            }
            macro_rules! __enabled {
                ($d($level:tt)+) => { $crate::__enabled!($component, $d($level)+) };
            }
            // A logger need not use every level, so the unused re-exports are allowed here.
            #[allow(unused_imports)]
            pub(crate) use {
                __error as error,
                __warn as warn,
                __info as info,
                __debug as debug,
                __trace as trace,
                __enabled as enabled,
            };
        }
    };
}

/// Whether a call at `$level` is enabled for `$component`. A [`logger!`] module's `enabled!`
/// expands to this. Each expansion owns its `static` filter, so the check is one relaxed load.
#[doc(hidden)]
#[macro_export]
macro_rules! __enabled {
    ($component:literal, $level:expr) => {{
        static SITE: $crate::filter::Component =
            $crate::filter::Component::new($crate::__checked_component($component));
        SITE.enabled($level)
    }};
}

#[cfg(test)]
mod tests {
    use super::event::capture::logged;
    use super::*;

    /// One logger per component the tests below check, so each assertion names its own tag.
    mod sample {
        crate::logger!(pub(crate) log = "log");
        crate::logger!(pub(crate) xdg = "xdg-shell");
        crate::logger!(pub(crate) testing = "logger-test");
    }

    #[test]
    fn a_lowercase_name_with_digits_and_inner_dashes_is_valid() {
        for name in ["wayland", "a", "compat-gtk", "x11-2", "a-b-c"] {
            assert!(is_valid_component(name), "{name} should be valid");
        }
    }

    #[test]
    fn a_name_outside_the_grammar_is_invalid() {
        let too_long = "a".repeat(MAX_COMPONENT_LEN + 1);
        for name in ["", "-a", "a-", "A", "a_b", "a b", "a.b", too_long.as_str()] {
            assert!(!is_valid_component(name), "{name:?} should be invalid");
        }
        assert!(is_valid_component(&"a".repeat(MAX_COMPONENT_LEN)));
    }

    #[test]
    fn every_level_macro_tags_the_event_with_its_named_component() {
        let events = logged(|| {
            sample::log::error!("e");
            sample::log::warn!("w");
            sample::log::info!(count = 3, "i {}", 4);
            sample::log::debug!("d");
            sample::log::trace!("t");
        });
        let components: Vec<_> = events.iter().map(|event| event.component.as_str()).collect();
        // `debug` and `trace` are below the default `info` filter, so only three events arrive.
        assert_eq!(components, ["log"; 3]);
    }

    #[test]
    fn a_sub_area_names_its_own_component() {
        let events = logged(|| sample::xdg::warn!("unknown request {}", 7));
        assert_eq!(events[0].component, "xdg-shell");
        assert_eq!(events[0].message, "unknown request 7");
    }

    #[test]
    fn each_role_has_a_distinct_name() {
        let names = [
            ProcessRole::Compositor,
            ProcessRole::Guardian,
            ProcessRole::Runtime,
            ProcessRole::Services,
            ProcessRole::Portal,
        ]
        .map(ProcessRole::name);
        for (index, name) in names.iter().enumerate() {
            assert!(!names[..index].contains(name), "{name} is named twice");
        }
    }

    #[test]
    fn logger_macros_tag_events_with_their_component_and_target() {
        let events = logged(|| {
            sample::testing::info!("hello {}", 1);
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].component, "logger-test");
        assert_eq!(events[0].message, "hello 1");
    }

    /// Two loggers in one module: `log` for the crate, `wire` for a sub-area with its own
    /// component, and `gate`, which only the enabled check uses.
    mod loggers {
        crate::logger!(log = "logger-one");
        crate::logger!(wire = "logger-wire");
        crate::logger!(gate = "logger-gate");

        pub(super) fn emit() {
            log::info!("one {}", 1);
            wire::warn!(frames = 2, "wire {}", 3);
            log::debug!("below the default");
        }

        pub(super) fn gate_checks() -> [bool; 3] {
            [
                gate::enabled!(crate::Level::ERROR),
                gate::enabled!(crate::Level::INFO),
                gate::enabled!(crate::Level::DEBUG),
            ]
        }
    }

    #[test]
    fn logger_events_carry_their_component_and_target() {
        let events = logged(loggers::emit);
        let tags: Vec<_> = events
            .iter()
            .map(|event| (event.component.as_str(), event.message.as_str()))
            .collect();
        assert_eq!(tags, [("logger-one", "one 1"), ("logger-wire", "wire 3")]);
        assert_eq!(events[0].target, "gnoblin::logger-one");
        assert_eq!(events[1].target, "gnoblin::logger-wire");
        assert_eq!(events[1].fields, [("frames".to_owned(), "2".to_owned())]);
    }

    #[test]
    fn enabled_reports_the_default_level_of_a_logger() {
        // The default specification is `info`. Other tests set and reset the global
        // specification, but none changes the default, so this holds whichever runs first.
        assert_eq!(loggers::gate_checks(), [true, true, false]);
    }
}
