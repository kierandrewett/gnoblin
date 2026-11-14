//! The `autostart` section as the runtime role reads it: which entries start, and the rule that keeps a second
//! keyring daemon out of a session that serves Gnoblin's own.
//!
//! The runtime role sends the guardian the entries of the document it committed, so the rule lives here, beside
//! the document, rather than in the role.

use crate::Config;

/// The enabled autostart entries of `config`, in name order, each with its argv.
///
/// The schema requires a command on every entry, so an entry without one is refused here
/// rather than started with nothing.
///
/// # Errors
///
/// An entry has no command, or starts another keyring while Gnoblin's is on ([`keyring_conflict`]).
pub fn enabled_autostart(config: &Config) -> Result<Vec<(&str, &[String])>, String> {
    let Some(autostart) = &config.autostart else {
        return Ok(Vec::new());
    };
    let keyring_on = config
        .keyring
        .as_ref()
        .and_then(|keyring| keyring.enabled)
        .unwrap_or(true);
    let mut entries = Vec::new();
    for (name, entry) in &autostart.autostart {
        if !entry.enable.unwrap_or(true) {
            continue;
        }
        match entry.command.as_deref() {
            Some(command) if !command.is_empty() => {
                if let Some(problem) = keyring_conflict(keyring_on, name, command) {
                    return Err(problem);
                }
                entries.push((name.as_str(), command));
            }
            _ => return Err(format!("autostart \"{name}\" has no command")),
        }
    }
    Ok(entries)
}

/// Why an autostart entry must not run while Gnoblin's keyring is on, if it must not.
///
/// The failure this prevents: a keyring daemon started at login registers
/// `org.freedesktop.secrets` first, and every app that asks for a secret reaches it instead
/// of Gnoblin's keyring. Nothing in the session can then tell the user why their apps are
/// not using Gnoblin's. The check matches the basename of each argv word, so
/// `/usr/bin/gnome-keyring-daemon` and `dbus-run-session gnome-keyring-daemon` are caught.
/// It does not parse shell strings: `sh -c "gnome-keyring-daemon --start"` (one word) is
/// missed, and `docs/config/configure/keyring.md` says so.
pub fn keyring_conflict(keyring_on: bool, name: &str, argv: &[String]) -> Option<String> {
    if !keyring_on {
        return None;
    }
    let starts_another_keyring = argv.iter().any(|word| {
        let base = word.rsplit('/').next().unwrap_or(word);
        matches!(base, "gnome-keyring-daemon" | "gnome-keyring")
    });
    starts_another_keyring.then(|| {
        format!(
            "autostart \"{name}\" starts gnome-keyring, which would take org.freedesktop.secrets \
             from Gnoblin's keyring; remove the entry, or set keyring.enabled = false to use \
             gnome-keyring instead"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{Config, enabled_autostart, keyring_conflict};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// The failure this prevents: an autostart entry starts another keyring's daemon while
    /// Gnoblin's keyring is on. The daemon then registers org.freedesktop.secrets first, and
    /// apps that asked for a secret reach it instead of Gnoblin's.
    #[test]
    fn an_entry_that_starts_gnome_keyring_is_refused_while_the_keyring_is_on() {
        let problem = keyring_conflict(true, "keyring", &argv(&["gnome-keyring-daemon", "--start"])).expect("refused");
        assert!(problem.contains("\"keyring\""), "names the entry: {problem}");
        assert!(problem.contains("keyring.enabled"), "names the setting: {problem}");
    }

    #[test]
    fn a_full_path_to_gnome_keyring_is_refused_too() {
        assert!(keyring_conflict(true, "k", &argv(&["/usr/bin/gnome-keyring-daemon"])).is_some());
        assert!(keyring_conflict(true, "k", &argv(&["gnome-keyring"])).is_some());
    }

    #[test]
    fn gnome_keyring_is_allowed_when_gnoblin_keyring_is_off() {
        assert_eq!(
            keyring_conflict(false, "keyring", &argv(&["gnome-keyring-daemon", "--start"])),
            None,
            "the user chose another keyring, so its daemon may run"
        );
    }

    #[test]
    fn other_programs_and_near_names_are_not_refused() {
        assert_eq!(keyring_conflict(true, "bar", &argv(&["waybar"])), None);
        assert_eq!(keyring_conflict(true, "x", &argv(&["gnome-keyring-daemonx"])), None);
        assert_eq!(
            keyring_conflict(true, "x", &argv(&["/opt/gnome-keyring-tools/run"])),
            None
        );
        assert_eq!(
            keyring_conflict(true, "x", &[]),
            None,
            "an empty argv is refused elsewhere"
        );
    }

    /// The gap this closes: a user who wrote the entry gets a load refusal from the runtime
    /// and from `check`, both of which call `enabled_autostart`, while the keyring is on.
    #[test]
    fn autostart_refuses_gnome_keyring_while_the_keyring_is_on() {
        let config =
            Config::from_json_bytes(br#"{"autostart": {"daemon": {"command": ["gnome-keyring-daemon", "--start"]}}}"#)
                .expect("the document is valid");
        let problem = enabled_autostart(&config).expect_err("refused");
        assert!(problem.contains("keyring.enabled"), "{problem}");
    }

    #[test]
    fn autostart_allows_gnome_keyring_when_the_keyring_is_off() {
        let config = Config::from_json_bytes(
            br#"{"keyring": {"enabled": false}, "autostart": {"daemon": {"command": ["gnome-keyring-daemon", "--start"]}}}"#,
        )
        .expect("the document is valid");
        assert_eq!(enabled_autostart(&config).expect("allowed").len(), 1);
    }

    #[test]
    fn a_wrapper_that_starts_gnome_keyring_is_refused() {
        assert!(keyring_conflict(true, "k", &argv(&["dbus-run-session", "gnome-keyring-daemon"])).is_some());
    }

    #[test]
    fn a_disabled_entry_does_not_start() {
        let config = Config::from_json_bytes(
            br#"{"autostart": {"bar": {"command": ["waybar"]}, "off": {"command": ["x"], "enable": false}}}"#,
        )
        .expect("the document is valid");
        let entries = enabled_autostart(&config).expect("allowed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "bar");
    }
}
