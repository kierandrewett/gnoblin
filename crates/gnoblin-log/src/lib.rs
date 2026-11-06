//! The logging facade every gnoblin crate depends on.
//!
//! Crates log through the macros here and never name `tracing` themselves. Each macro tags its
//! event with the component of the crate that expands it, worked out at compile time from the
//! package name, so a call site names nothing:
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
//! sigils. This crate depends on `tracing` alone. The sinks live in `gnoblin-log-console` and
//! `gnoblin-log-store`, and the composition that installs them in `gnoblin-log-init`.

/// The `tracing` crate, for the sinks and the composition, which need its subscriber types.
///
/// Crates other than this one must not use it to log; they use the macros.
#[doc(hidden)]
pub use tracing as __tracing;

/// The name an event carries as its component.
///
/// A crate's component is its package name without the `gnoblin-` prefix, so `gnoblin-wayland`
/// logs as `wayland`, and the portal keeps its name `portal`. This table is the one place those
/// names are decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component(&'static str);

impl Component {
    /// The component of the package named `package`.
    pub const fn for_package(package: &'static str) -> Self {
        Self(component_name(package))
    }

    /// The name as logged.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// The prefix every gnoblin package shares, which components leave out.
const PREFIX: &[u8] = b"gnoblin-";

const fn component_name(package: &'static str) -> &'static str {
    if bytes_equal(package, "xdg-desktop-portal-gnoblin") {
        "portal"
    } else {
        strip_prefix(package)
    }
}

/// Whether two strings hold the same bytes; `==` on `str` is not usable in a const fn.
const fn bytes_equal(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

const fn strip_prefix(package: &'static str) -> &'static str {
    let bytes = package.as_bytes();
    if bytes.len() <= PREFIX.len() {
        return package;
    }
    let mut index = 0;
    while index < PREFIX.len() {
        if bytes[index] != PREFIX[index] {
            return package;
        }
        index += 1;
    }
    match core::str::from_utf8(bytes.split_at(PREFIX.len()).1) {
        Ok(rest) => rest,
        Err(_) => package,
    }
}

/// The process role a gnoblin binary runs as.
///
/// The console and the database both record the role, so one log tells the compositor's lines
/// from the guardian's even though both are written by the same binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
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

impl Role {
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
macro_rules! error {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::error!(target: $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::error!(
            target: $crate::Component::for_package(env!("CARGO_PKG_NAME")).as_str(),
            $($rest)+
        )
    };
}

/// Logs an event at `WARN` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
macro_rules! warn {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::warn!(target: $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::warn!(
            target: $crate::Component::for_package(env!("CARGO_PKG_NAME")).as_str(),
            $($rest)+
        )
    };
}

/// Logs an event at `INFO` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
macro_rules! info {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::info!(target: $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::info!(
            target: $crate::Component::for_package(env!("CARGO_PKG_NAME")).as_str(),
            $($rest)+
        )
    };
}

/// Logs an event at `DEBUG` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
macro_rules! debug {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::debug!(target: $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::debug!(
            target: $crate::Component::for_package(env!("CARGO_PKG_NAME")).as_str(),
            $($rest)+
        )
    };
}

/// Logs an event at `TRACE` level, tagged with the crate's component.
///
/// `component: "name"` as the first argument tags it with `name` instead.
#[macro_export]
macro_rules! trace {
    (component: $component:expr, $($rest:tt)+) => {
        $crate::__tracing::trace!(target: $component, $($rest)+)
    };
    ($($rest:tt)+) => {
        $crate::__tracing::trace!(
            target: $crate::Component::for_package(env!("CARGO_PKG_NAME")).as_str(),
            $($rest)+
        )
    };
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::*;

    /// Records the target of every event it is given.
    #[derive(Clone, Default)]
    struct Targets(Arc<Mutex<Vec<String>>>);

    impl Subscriber for Targets {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, event: &Event<'_>) {
            self.0
                .lock()
                .expect("the capture lock")
                .push(event.metadata().target().to_owned());
        }

        fn enter(&self, _: &Id) {}

        fn exit(&self, _: &Id) {}
    }

    /// The targets of the events `run` logs.
    fn targets_of(run: impl FnOnce()) -> Vec<String> {
        let targets = Targets::default();
        tracing::subscriber::with_default(targets.clone(), run);
        targets.0.lock().expect("the capture lock").clone()
    }

    #[test]
    fn the_package_name_without_the_prefix_is_the_component() {
        assert_eq!(Component::for_package("gnoblin-wayland").as_str(), "wayland");
        assert_eq!(Component::for_package("gnoblin-log").as_str(), "log");
    }

    #[test]
    fn the_portal_keeps_its_name() {
        assert_eq!(Component::for_package("xdg-desktop-portal-gnoblin").as_str(), "portal");
    }

    #[test]
    fn a_package_without_the_prefix_is_its_own_component() {
        assert_eq!(Component::for_package("gnoblin").as_str(), "gnoblin");
        assert_eq!(Component::for_package("tracing").as_str(), "tracing");
        assert_eq!(Component::for_package("gnoblin-").as_str(), "gnoblin-");
    }

    #[test]
    fn every_level_macro_tags_the_event_with_the_crate_component() {
        let targets = targets_of(|| {
            crate::error!("e");
            crate::warn!("w");
            crate::info!(count = 3, "i {}", 4);
            crate::debug!("d");
            crate::trace!("t");
        });
        assert_eq!(targets, ["log"; 5]);
    }

    #[test]
    fn a_component_argument_overrides_the_crate_component() {
        let targets = targets_of(|| crate::warn!(component: "xdg-shell", "unknown request {}", 7));
        assert_eq!(targets, ["xdg-shell"]);
    }

    #[test]
    fn each_role_has_a_distinct_name() {
        let names = [
            Role::Compositor,
            Role::Guardian,
            Role::Runtime,
            Role::Services,
            Role::Portal,
        ]
        .map(Role::name);
        for (index, name) in names.iter().enumerate() {
            assert!(!names[..index].contains(name), "{name} is named twice");
        }
    }
}
