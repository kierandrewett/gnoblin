//! The session environment, expressed as data the caller supplies and applies.
//!
//! The guardian never touches its own process environment and never calls
//! systemd or D-Bus. It decides *which* variables the session must have, in
//! which order, and when they become valid; the caller applies them to its own
//! environment, to the user manager (`systemctl --user set-environment`) and to
//! the D-Bus activation environment (`UpdateActivationEnvironment`), exactly as
//! `src/session/gnoblin-runtime-environment.c` does today.

/// Environment changes for one target: what to set, and what to unset.
///
/// Both lists keep the order the rules declared, so an exported environment is
/// reproducible; `unset` is applied after `set`, so a rule may set a variable and
/// a later rule clear it deliberately.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Environment {
    /// Variables to set, in rule order.
    pub set: Vec<(String, String)>,
    /// Variables to remove, in rule order.
    pub unset: Vec<String>,
}

impl Environment {
    /// An empty set of changes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `key` to `value`, replacing an earlier value for the same key.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        let key = key.into();
        let value = value.into();
        match self.set.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, existing)) => *existing = value,
            None => self.set.push((key, value)),
        }
        self
    }

    /// Remove `key` from the environment.
    pub fn unset(&mut self, key: impl Into<String>) -> &mut Self {
        let key = key.into();
        if !self.unset.contains(&key) {
            self.unset.push(key);
        }
        self
    }

    /// The value this environment gives `key`, ignoring removals.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.set
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, value)| value.as_str())
    }
}

/// Where an exported environment belongs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvTarget {
    /// The guardian's own environment, inherited by every child it spawns, and
    /// mirrored into the user manager's environment so units started later see
    /// the same session.
    Session,
    /// The D-Bus activation environment, so an application activated over D-Bus
    /// finds the display it belongs to.
    Activation,
}

/// The parts of the guardian's own environment that the session rules are built
/// from. Passing them in keeps the rules pure data: the caller reads its
/// environment, the guardian decides what the session gets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InheritedEnvironment<'a> {
    /// The installation prefix the guardian runs from, usually the parent of
    /// its own `bin` directory.
    pub prefix: &'a str,
    /// The inherited `PATH`.
    pub path: &'a str,
    /// The library search path the session needs, already composed: it depends
    /// on the private Mutter and C++ runtime directories the packaging chooses.
    pub library_path: &'a str,
    /// The inherited `XDG_DATA_DIRS`.
    pub data_dirs: &'a str,
}

/// Which variables the session owns, in which order, and when they become valid.
///
/// `session` is applied before the first child starts. `activation` describes the
/// D-Bus activation environment, which must not claim a display before the
/// compositor has published one, so it starts with the stale variables removed
/// and gains the display later. `forwarded` names the variables the compositor
/// publishes in its ready report; every child spawned afterwards inherits them,
/// and a `required` one that is missing means the session cannot work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentRules {
    /// Changes applied to the session before the first child starts.
    pub session: Environment,
    /// Changes applied to the D-Bus activation environment.
    pub activation: Environment,
    /// Variables the compositor publishes and later children inherit, in the
    /// order they appear in a child's environment.
    pub forwarded: Vec<String>,
    /// The subset of `forwarded` the session cannot work without.
    pub required: Vec<String>,
    /// A variable set to `1` in every child while the session runs in safe mode,
    /// so the composition can run embedded defaults instead of the user's files.
    pub safe_mode_key: String,
}

impl EnvironmentRules {
    /// The rules the C session applies today, rebuilt from the inherited values.
    #[must_use]
    pub fn standard(inherited: &InheritedEnvironment<'_>) -> Self {
        let prefix = inherited.prefix.trim_end_matches('/');
        let mut session = Environment::new();
        session
            .set("PATH", format!("{prefix}/bin:{}", inherited.path))
            .set("LD_LIBRARY_PATH", inherited.library_path)
            .set("XDG_DATA_DIRS", format!("{prefix}/share:{}", inherited.data_dirs))
            .set("XDG_CURRENT_DESKTOP", "Gnoblin")
            .set("XDG_SESSION_DESKTOP", "gnoblin")
            .set("GNOBLIN_PREFIX", prefix);
        for stale in [
            // A login that exported these would point applications at another
            // display, or at a Xwayland authority this session does not own.
            "XAUTHORITY",
            "GNOME_SETUP_DISPLAY",
            "GSETTINGS_SCHEMA_DIR",
            "GI_TYPELIB_PATH",
            "GNOME_SHELL_SESSION_MODE",
            // The privileged display is published by this session's own ready report, so a value
            // a login exported names a socket from another session and must not reach children.
            "GNOBLIN_PRIVILEGED_DISPLAY",
        ] {
            session.unset(stale);
        }

        let mut activation = Environment::new();
        for stale in ["WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", "GNOME_SETUP_DISPLAY"] {
            activation.unset(stale);
        }
        activation
            .set("XDG_CURRENT_DESKTOP", "Gnoblin")
            .set("XDG_SESSION_DESKTOP", "gnoblin");

        Self {
            session,
            activation,
            forwarded: ["WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", "GNOBLIN_PRIVILEGED_DISPLAY"]
                .map(String::from)
                .to_vec(),
            required: ["WAYLAND_DISPLAY"].map(String::from).to_vec(),
            safe_mode_key: "GNOBLIN_SAFE_MODE".to_owned(),
        }
    }
}
