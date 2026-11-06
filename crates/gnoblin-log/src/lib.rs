//! The logging facade every gnoblin crate depends on.
//!
//! A crate declares its component once, at its crate root:
//!
//! ```ignore
//! gnoblin_log::component!("wayland");
//! ```
//!
//! The name is checked at compile time: 1 to 24 bytes of `a-z`, `0-9` and `-`, with no leading or
//! trailing `-`. The level macros read the declared name, so a crate that logs without declaring
//! one does not compile. Call sites name nothing else:
//!
//! ```ignore
//! gnoblin_log::info!(surfaces = count, "mapped");
//! ```
//!
//! A call that belongs to a sub-area of a crate names its component first:
//!
//! ```ignore
//! gnoblin_log::warn!(component: "xdg-shell", "unknown request {opcode}");
//! ```
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

/// The longest component name, in bytes.
const MAX_COMPONENT_LEN: usize = 24;

/// Declares the component of the crate it appears in.
///
/// Put it once, at the crate root (`lib.rs` or `main.rs`), and not in a module. It expands to a
/// hidden `__GNOBLIN_LOG_COMPONENT` constant that the level macros read.
#[macro_export]
macro_rules! component {
    ($name:literal) => {
        #[allow(dead_code)]
        pub(crate) const __GNOBLIN_LOG_COMPONENT: &str = $crate::__checked_component($name);
    };
}

/// Returns `name` when it is a valid component, and fails compilation when it is not.
///
/// Only [`component!`] calls this, in a constant, so an invalid name is a compile error.
#[doc(hidden)]
pub const fn __checked_component(name: &'static str) -> &'static str {
    if is_valid_component(name) {
        name
    } else {
        panic!(
            "gnoblin_log::component!: a component is 1 to 24 bytes of a-z, 0-9 and '-', with no leading or trailing '-'"
        )
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

/// Logs an event at `ERROR` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // `crate` is the caller's root, where `component!` declares the constant
macro_rules! error {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::error!(component = $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::error!(component = crate::__GNOBLIN_LOG_COMPONENT, $($rest)+)
    };
}

/// Logs an event at `WARN` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // `crate` is the caller's root, where `component!` declares the constant
macro_rules! warn {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::warn!(component = $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::warn!(component = crate::__GNOBLIN_LOG_COMPONENT, $($rest)+)
    };
}

/// Logs an event at `INFO` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // `crate` is the caller's root, where `component!` declares the constant
macro_rules! info {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::info!(component = $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::info!(component = crate::__GNOBLIN_LOG_COMPONENT, $($rest)+)
    };
}

/// Logs an event at `DEBUG` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // `crate` is the caller's root, where `component!` declares the constant
macro_rules! debug {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::debug!(component = $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::debug!(component = crate::__GNOBLIN_LOG_COMPONENT, $($rest)+)
    };
}

/// Logs an event at `TRACE` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
#[allow(clippy::crate_in_macro_def)] // `crate` is the caller's root, where `component!` declares the constant
macro_rules! trace {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::trace!(component = $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::trace!(component = crate::__GNOBLIN_LOG_COMPONENT, $($rest)+)
    };
}

component!("log");

#[cfg(test)]
mod tests {
    use super::event::capture::logged;
    use super::*;

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
    fn every_level_macro_tags_the_event_with_the_declared_component() {
        let events = logged(|| {
            crate::error!("e");
            crate::warn!("w");
            crate::info!(count = 3, "i {}", 4);
            crate::debug!("d");
            crate::trace!("t");
        });
        let components: Vec<_> = events.iter().map(|event| event.component.as_str()).collect();
        assert_eq!(components, ["log"; 5]);
    }

    #[test]
    fn a_component_argument_overrides_the_declared_component() {
        let events = logged(|| crate::warn!(component: "xdg-shell", "unknown request {}", 7));
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
}
