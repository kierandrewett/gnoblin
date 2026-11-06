//! The `xwayland` configuration section, and what one start needs from it.
//!
//! The section is validated by `gnoblin-config`, which owns the ranges and the
//! problems; this module turns an accepted section into the start parameters and
//! re-checks the one rule that a caller outside the document could break. That is
//! deliberate: `scaling-factor` says "0, or a finite number from 0.5", and a caller
//! that computed a scale from an output must not be able to hand the compositor a
//! `0.3` that rounds to zero and turns every X11 window into a one-pixel stub.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gnoblin_config::{Xwayland, XwaylandExtension};
use gnoblin_types::Scale;

use gnoblin_backend::x11::Launch;

/// The X11 settings a session starts with.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Settings {
    /// Whether X11 clients may take keyboard grabs.
    pub allow_grabs: bool,
    /// Whether clients whose byte order differs from the server's are allowed.
    pub allow_byte_swapped_clients: bool,
    /// The classes or instances allowed to take a grab, in declaration order.
    ///
    /// Empty allows every client, which is the configuration default. Only read when
    /// [`Settings::allow_grabs`] is true.
    pub grab_access_rules: Vec<String>,
    /// The Xwayland extensions to disable.
    pub disable_extensions: Vec<XwaylandExtension>,
    /// The fixed client scale, or [`None`] to follow the output.
    pub scale: Option<Scale>,
    /// The `XDG_RUNTIME_DIR` the `Xwayland` process is started with, or [`None`] to
    /// inherit the compositor's own.
    pub runtime_dir: Option<PathBuf>,
}

/// Why a setting a caller supplied is refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsError {
    /// A scale was neither automatic (`0`) nor at least `0.5`.
    ScaleBelowMinimum,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScaleBelowMinimum => formatter.write_str("a fixed scale is 0.5 or more; 0 means automatic"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl Settings {
    /// The settings an accepted `xwayland` section states.
    ///
    /// A field the document did not set is the section's default, so a document that
    /// says nothing at all produces [`Settings::default`].
    #[must_use]
    pub fn from_config(section: &Xwayland) -> Self {
        let defaults = Settings::default();
        Self {
            allow_grabs: section.allow_grabs.unwrap_or(defaults.allow_grabs),
            allow_byte_swapped_clients: section
                .allow_byte_swapped_clients
                .unwrap_or(defaults.allow_byte_swapped_clients),
            grab_access_rules: section.grab_access_rules.clone().unwrap_or(defaults.grab_access_rules),
            disable_extensions: section
                .disable_extensions
                .clone()
                .unwrap_or(defaults.disable_extensions),
            scale: match section.scaling_factor {
                Some(factor) => Self::scale_from_factor(factor).unwrap_or(defaults.scale),
                None => defaults.scale,
            },
            runtime_dir: defaults.runtime_dir,
        }
    }

    /// The scale a `scaling-factor` value means.
    ///
    /// `0` is automatic scaling and answers [`None`]; `0.5` and up is a fixed scale.
    /// Anything else is refused rather than rounded, because rounding it is how a
    /// scale of `0.3` becomes a scale of zero.
    ///
    /// # Errors
    ///
    /// [`SettingsError::ScaleBelowMinimum`] for a value that is neither zero nor at
    /// least `0.5`, and for one that is not finite.
    pub fn scale_from_factor(factor: f64) -> Result<Option<Scale>, SettingsError> {
        if factor == 0.0 {
            return Ok(None);
        }
        if !factor.is_finite() || factor < 0.5 {
            return Err(SettingsError::ScaleBelowMinimum);
        }
        // A scale is a reduced fraction; a factor with more decimal places than the
        // configuration permits is truncated to the thousandth, the precision the
        // resolver uses for a fractional scale.
        let thousandths = (factor * 1000.0).round();
        if thousandths > f64::from(u32::MAX) {
            return Err(SettingsError::ScaleBelowMinimum);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let thousandths = thousandths as u32;
        Ok(Some(Scale::new(thousandths, 1000)))
    }

    /// Whether a class or instance name may take a grab.
    ///
    /// A client with no name has nothing to match, so an empty rule list is the only
    /// configuration that lets it through.
    #[must_use]
    pub fn grab_allowed(&self, class: &str, instance: &str) -> bool {
        grab_permitted(self.allow_grabs, &self.grab_access_rules, class, instance)
    }
}

/// Whether a client named `class` or `instance` may take a keyboard grab.
///
/// This is the rule `grab_access_rules` documents. Nothing is allowed unless `allow_grabs` is
/// set. Each rule is a glob over `*` (any run of characters) and `?` (any one character),
/// matched case-sensitively against the class and the instance. A rule prefixed with `!` denies.
/// A deny match wins over any allow match. An empty rule list allows every client, because the
/// document then removes only user rules and Mutter's compiled rules still apply. A name that is
/// empty matches no rule.
#[must_use]
pub fn grab_permitted(allow_grabs: bool, rules: &[String], class: &str, instance: &str) -> bool {
    if !allow_grabs {
        return false;
    }
    let named = |text: &str| !text.is_empty();
    let matches = |pattern: &str| {
        (named(class) && glob_matches(pattern, class)) || (named(instance) && glob_matches(pattern, instance))
    };
    if rules.iter().any(|rule| rule.strip_prefix('!').is_some_and(matches)) {
        return false;
    }
    rules.is_empty() || rules.iter().any(|rule| !rule.starts_with('!') && matches(rule))
}

/// Whether `text` matches `pattern`, where `*` matches any run of characters and `?` one.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    // The pattern position after the last `*` seen, and the text position it was tried at.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p + 1, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            // Let the last `*` absorb one more character and retry from there.
            p = star_p;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod grab_rule_tests {
    use super::grab_permitted;

    fn rules(list: &[&str]) -> Vec<String> {
        list.iter().map(|rule| (*rule).to_owned()).collect()
    }

    #[test]
    fn an_exact_name_matches_its_class_or_instance() {
        assert!(grab_permitted(true, &rules(&["Gnome-terminal"]), "Gnome-terminal", "x"));
        assert!(grab_permitted(true, &rules(&["gnome"]), "Other", "gnome"));
        assert!(!grab_permitted(true, &rules(&["gnome"]), "Gnome-terminal", "x"));
    }

    #[test]
    fn a_star_matches_any_run_of_characters() {
        assert!(grab_permitted(true, &rules(&["Gnome-*"]), "Gnome-terminal", "x"));
        assert!(grab_permitted(true, &rules(&["*term*"]), "Gnome-terminal", "x"));
        assert!(!grab_permitted(true, &rules(&["Gnome-*"]), "Kate", "x"));
    }

    #[test]
    fn a_question_mark_matches_exactly_one_character() {
        assert!(grab_permitted(true, &rules(&["app?"]), "app1", "x"));
        assert!(!grab_permitted(true, &rules(&["app?"]), "app", "x"));
        assert!(!grab_permitted(true, &rules(&["app?"]), "app12", "x"));
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(!grab_permitted(true, &rules(&["gnome"]), "Gnome", "x"));
    }

    #[test]
    fn a_deny_rule_beats_an_allow_rule() {
        let list = rules(&["*", "!blocked-*"]);
        assert!(grab_permitted(true, &list, "fine", "x"));
        assert!(!grab_permitted(true, &list, "blocked-app", "x"));
        assert!(!grab_permitted(true, &rules(&["blocked", "!blocked"]), "blocked", "x"));
    }

    #[test]
    fn a_deny_with_a_wildcard_denies_every_match() {
        assert!(!grab_permitted(true, &rules(&["*", "!?"]), "a", ""));
        assert!(grab_permitted(true, &rules(&["*", "!?"]), "ab", ""));
    }

    #[test]
    fn an_empty_rule_list_allows_every_client() {
        assert!(grab_permitted(true, &[], "anything", "anything"));
    }

    #[test]
    fn allow_grabs_false_denies_everything() {
        assert!(!grab_permitted(false, &rules(&["*"]), "anything", "anything"));
        assert!(!grab_permitted(
            false,
            &rules(&["Gnome-terminal"]),
            "Gnome-terminal",
            "x"
        ));
    }
}

/// What one `Xwayland` binary supports, read from its own help text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Whether the binary knows the `-byteswappedclients` switch. Servers before the
    /// switch existed reject it as an unknown option and stop before they listen.
    pub byte_order_switch: bool,
}

impl Capabilities {
    /// Asks the binary for its switches with `-help`, which prints the usage and exits
    /// without starting a server.
    ///
    /// A binary that cannot be run answers "no switches": the conservative answer,
    /// because every switch that depends on it is then refused with a reason rather
    /// than passed to a server that will reject it.
    #[must_use]
    pub fn of(binary: &Path) -> Self {
        let Ok(output) = std::process::Command::new(binary)
            .arg("-help")
            .stdin(std::process::Stdio::null())
            .output()
        else {
            return Self::default();
        };
        // The usage goes to stderr on some servers and stdout on others.
        let text = [output.stdout, output.stderr].concat();
        Self {
            byte_order_switch: String::from_utf8_lossy(&text).contains("byteswappedclients"),
        }
    }
}

impl Settings {
    /// The Xwayland arguments the settings add, for a server with `capabilities`.
    ///
    /// Only the switches that map to a setting are here. Everything else Smithay's
    /// `XWayland::spawn` already passes: the display, `-rootless`, `-terminate`, the
    /// window manager socket and the readiness pipe.
    ///
    /// # Errors
    ///
    /// Refusing byte-swapped clients needs the switch. A server without it would accept
    /// them, so the session is refused with the reason instead of starting with a weaker
    /// policy than the one configured.
    pub fn arguments(&self, capabilities: Capabilities) -> Result<Vec<String>, String> {
        let mut arguments = Vec::new();
        // Xwayland accepts a client whose byte order differs from the server's unless
        // told otherwise. Allowing them is the server's default, so it needs no switch;
        // refusing them needs the switch, and the setting defaults to refusing.
        match (self.allow_byte_swapped_clients, capabilities.byte_order_switch) {
            (true, true) => arguments.push("+byteswappedclients".to_owned()),
            (false, true) => arguments.push("-byteswappedclients".to_owned()),
            (true, false) => {}
            (false, false) => {
                return Err(
                    "Xwayland is not started: this Xwayland has no -byteswappedclients switch, \
                    so byte-swapped clients cannot be refused; install a newer Xwayland or allow \
                    byte-swapped clients"
                        .to_owned(),
                );
            }
        }
        for extension in &self.disable_extensions {
            arguments.push("-extension".to_owned());
            arguments.push(match extension {
                XwaylandExtension::Security => "SECURITY".to_owned(),
                XwaylandExtension::Xtest => "XTEST".to_owned(),
            });
        }
        Ok(arguments)
    }

    /// The start parameters for one attempt against a server with `capabilities`.
    ///
    /// # Errors
    ///
    /// The reason from [`Settings::arguments`] when the configured policy cannot be
    /// enforced by this server.
    pub fn launch(&self, display: Option<u32>, capabilities: Capabilities) -> Result<Launch, String> {
        let runtime_dir = match &self.runtime_dir {
            Some(dir) => Some((
                "XDG_RUNTIME_DIR".to_owned(),
                dir.to_str()
                    .ok_or_else(|| format!("the runtime directory {} is not UTF-8", dir.display()))?
                    .to_owned(),
            )),
            None => None,
        };
        Ok(Launch {
            display,
            args: self.arguments(capabilities)?,
            env: runtime_dir.into_iter().collect(),
            open_abstract_socket: false,
            scale: self.scale,
        })
    }
}

/// The `Xwayland` binary, if it is on `PATH`.
///
/// The search is the caller's `PATH` value rather than the process environment, so a
/// start path can be tested without changing the environment of the test process and
/// a supervised session explores the `PATH` it was handed.
///
/// # Errors
///
/// The reason to log and to disable the session with: no `PATH`, no `Xwayland` entry
/// on it, or no executable file at the entry. The sentence is complete, because it is
/// what a person sees when Xwayland is off.
pub fn probe(path: Option<&OsStr>) -> Result<PathBuf, String> {
    let Some(path) = path else {
        return Err("Xwayland is not started: PATH is not set".to_owned());
    };
    for directory in std::env::split_paths(path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let candidate = directory.join("Xwayland");
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Xwayland is not started: no Xwayland binary on PATH ({})",
        Path::new(OsStr::new(path)).display()
    ))
}

/// Whether a path is a file any user bit allows to execute.
///
/// This is the check the kernel applies to `execve`, minus the answer for the current
/// user: a binary that is present but not executable is the same absence to the
/// caller, and the message says which one it was.
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsString;

    /// A scale below `0.5` is refused instead of rounded to zero: the failure the
    /// configuration range exists for, reached through the typed entry point.
    #[test]
    fn a_scale_below_the_minimum_is_refused() {
        for factor in [0.3, 0.49, 0.1, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Settings::scale_from_factor(factor),
                Err(SettingsError::ScaleBelowMinimum),
                "{factor} should be refused"
            );
        }
    }

    /// Zero is automatic and half is the smallest fixed scale, so the two ends of the
    /// rule are accepted with distinct meanings.
    #[test]
    fn zero_is_automatic_and_a_half_is_a_fixed_scale() {
        assert_eq!(Settings::scale_from_factor(0.0), Ok(None));
        assert_eq!(Settings::scale_from_factor(0.5), Ok(Some(Scale::new(1, 2))));
        assert_eq!(Settings::scale_from_factor(2.0), Ok(Some(Scale::integral(2))));
    }

    /// A client with no name matches nothing when a rule list exists, and every client
    /// matches when there is none. The alternative, matching an empty name against
    /// every rule, would let a nameless client take a grab the document refused it.
    #[test]
    fn an_empty_class_matches_only_an_empty_rule_list() {
        let mut settings = Settings {
            allow_grabs: true,
            ..Settings::default()
        };
        assert!(settings.grab_allowed("", ""));

        settings.grab_access_rules = vec!["Gnome-terminal".to_owned()];
        assert!(!settings.grab_allowed("", ""));
        assert!(settings.grab_allowed("Gnome-terminal", "gnome-terminal"));
        assert!(!settings.grab_allowed("Gimp", "gimp"));
    }

    /// Grabs are off by default, so a rule list cannot allow one on its own.
    #[test]
    fn no_rules_allow_nothing_when_grabs_are_off() {
        let settings = Settings {
            allow_grabs: false,
            grab_access_rules: vec!["Gnome-terminal".to_owned()],
            ..Settings::default()
        };
        assert!(!settings.grab_allowed("Gnome-terminal", "gnome-terminal"));
    }

    /// The byte-order switch is stated in both directions when the server has it, and
    /// a disabled extension becomes its `-extension` pair in declaration order.
    #[test]
    fn the_settings_become_arguments() {
        let modern = Capabilities {
            byte_order_switch: true,
        };
        let refusing = Settings {
            allow_byte_swapped_clients: false,
            disable_extensions: vec![XwaylandExtension::Security, XwaylandExtension::Xtest],
            ..Settings::default()
        };
        assert_eq!(
            refusing.arguments(modern),
            Ok(
                vec!["-byteswappedclients", "-extension", "SECURITY", "-extension", "XTEST"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            )
        );

        let allowed = Settings {
            allow_byte_swapped_clients: true,
            ..Settings::default()
        };
        assert_eq!(allowed.arguments(modern), Ok(vec!["+byteswappedclients".to_owned()]));
    }

    /// A server without the switch cannot refuse byte-swapped clients, so refusing is a
    /// reason to disable the session rather than a silent start that accepts them. Allowing
    /// them needs no switch at all, because accepting is the server's default.
    #[test]
    fn an_old_server_cannot_refuse_byte_swapped_clients() {
        let old = Capabilities::default();
        let refusing = Settings::default();
        let reason = refusing.arguments(old).expect_err("refusal needs the switch");
        assert!(reason.contains("-byteswappedclients"), "{reason}");
        assert!(refusing.launch(None, old).is_err());

        let allowed = Settings {
            allow_byte_swapped_clients: true,
            ..Settings::default()
        };
        assert_eq!(allowed.arguments(old), Ok(Vec::new()));
        assert!(allowed.launch(None, old).is_ok());
    }

    /// The capabilities are read from the binary's own help text, so a binary that
    /// prints the switch has it and one that prints nothing has none.
    #[cfg(unix)]
    #[test]
    fn capabilities_come_from_the_help_text() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("gnoblin-xwayland-caps-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        let modern = directory.join("modern");
        std::fs::write(&modern, "#!/bin/sh\necho '+byteswappedclients    Allow clients' >&2\n").expect("a script");
        std::fs::set_permissions(&modern, std::fs::Permissions::from_mode(0o755)).expect("an executable mode");
        let old = directory.join("old");
        std::fs::write(&old, "#!/bin/sh\nexit 0\n").expect("a script");
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).expect("an executable mode");

        assert!(Capabilities::of(&modern).byte_order_switch);
        assert!(!Capabilities::of(&old).byte_order_switch);
        assert!(!Capabilities::of(&directory.join("missing")).byte_order_switch);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// An empty `PATH`, or one without the binary, disables the session with a reason
    /// instead of a panic; a real directory with an executable is found.
    #[test]
    fn the_binary_is_probed_on_the_given_path() {
        assert!(probe(None).unwrap_err().contains("PATH is not set"));

        let empty = probe(Some(OsStr::new(""))).unwrap_err();
        assert!(empty.contains("no Xwayland binary"), "{empty}");

        let directory = std::env::temp_dir().join(format!("gnoblin-xwayland-probe-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        use std::os::unix::fs::PermissionsExt;
        let binary = directory.join("Xwayland");
        std::fs::write(&binary, b"#!/bin/true\n").expect("a placeholder binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("an executable mode");

        let path: OsString = directory.clone().into_os_string();
        assert_eq!(probe(Some(path.as_os_str())), Ok(binary.clone()));

        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o644)).expect("a plain mode");
        assert!(
            probe(Some(path.as_os_str())).is_err(),
            "a non-executable entry is not the binary"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }
}
