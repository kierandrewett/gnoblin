//! The dconf layer GTK reads its own settings from (D15's `gtk` target).
//!
//! GTK is not told its fonts, icon theme, cursor or button layout by any protocol: it
//! reads GSettings keys under `org.gnome.desktop.interface` and
//! `org.gnome.desktop.wm.preferences`, and those keys come from dconf. D15's answer is a
//! dconf *layer* Gnoblin writes and GTK reads, so the desktop's configuration is the only
//! source and nothing a toolkit writes is read back.
//!
//! # Keys, then text, then a database
//!
//! The adapter is three steps so each one can be tested alone:
//!
//! 1. [`publish`] is a pure function of one [`DesktopSettings`] value: the same value
//!    always produces the same actions.
//! 2. [`keyfile`] renders those actions as a dconf keyfile, which is a text format:
//!    `[org/gnome/desktop/interface]` groups and `key=value` lines with GVariant values.
//! 3. [`apply`] writes the keyfile into the session's own dconf profile directory and, if
//!    the `dconf` binary is installed, compiles it with `dconf compile`. **This is the
//!    step that is conditional.** There is no libdconf dependency and no `unsafe` FFI
//!    here; when `dconf` is absent the keyfile is written and reported as
//!    [`Written::uncompiled`], and the caller decides whether that is good enough for the
//!    session (a dconf keyfile in a profile's directory is not a database until it is
//!    compiled).
//!
//! # Why a keyfile and not the user's database
//!
//! Writing `~/.config/dconf/user` directly would make this adapter a second writer of the
//! database every GNOME application uses, and a crash mid-write is a corrupt database for
//! the whole session. A keyfile under a Gnoblin-owned profile directory, compiled into its
//! own `gnoblin` database, is one file this adapter owns. `DCONF_PROFILE` decides which
//! profile dconf reads, and setting it is the session's job, not this adapter's.
//!
//! # Failure modes first
//!
//! 1. **A toolkit's own change is read back.** [`publish`] takes one value and reads
//!    nothing, and [`apply`] overwrites its own file, so a database another process
//!    changed has no effect on what is published:
//!    `a_changed_database_is_never_read_back`.
//! 2. **The target is off and the values still arrive.** [`apply`] with
//!    [`Switch::Off`] writes nothing and answers [`Written::Skipped`]:
//!    `a_switched_off_target_writes_nothing`.
//! 3. **A font name with a space or a quote breaks the keyfile.** Values are quoted and
//!    escaped as GVariant strings, and a name that contains both round-trips through
//!    [`keyfile`] unchanged: `a_font_name_survives_the_keyfile`.
//! 4. **The keyfile is written but GTK sees nothing, silently.** [`Written`] says whether
//!    the file was compiled, and the caller logs or refuses accordingly:
//!    `an_uncompiled_keyfile_is_reported`.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use gnoblin_compat_core::{DesktopSettings, Switch};

gnoblin_log::logger!(log = "compat-dconf");

/// The checked-in declaration, compiled in so the file a reviewer approved is the one
/// `cargo xtask compat-report` prints and this build honours.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// The interface schema's dconf group.
pub const INTERFACE: &str = "org/gnome/desktop/interface";

/// The window manager schema's dconf group.
pub const WINDOW_PREFERENCES: &str = "org/gnome/desktop/wm/preferences";

/// One key the layer writes.
///
/// `KeyWrite` rather than `Action`: the name says what it is (one key and the value it
/// takes) and does not collide with the many other things called an action in this tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyWrite {
    /// The schema's path, as a dconf group: `org/gnome/desktop/interface`.
    pub group: &'static str,
    /// The key within the schema.
    pub key: &'static str,
    /// The value, in the type the schema declares.
    pub value: DconfValue,
}

/// A dconf value, in the three types these keys use.
///
/// A string is quoted and escaped by [`keyfile`]; an integer and a boolean are their
/// text. Keeping the type here rather than a rendered string means a keyfile writer
/// cannot forget a quote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DconfValue {
    /// A `string` key, such as a font name.
    Text(String),
    /// An `int32` key, such as a cursor size.
    Integer(i32),
    /// A `boolean` key.
    Boolean(bool),
}

impl DconfValue {
    /// The value as a dconf keyfile writes it: GVariant text.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Text(text) => {
                let mut rendered = String::with_capacity(text.len() + 2);
                rendered.push('\'');
                for character in text.chars() {
                    // GVariant's single-quoted strings escape the quote and the
                    // backslash, which is what a font name with an apostrophe needs.
                    if character == '\'' || character == '\\' {
                        rendered.push('\\');
                    }
                    rendered.push(character);
                }
                rendered.push('\'');
                rendered
            }
            Self::Integer(value) => value.to_string(),
            Self::Boolean(value) => {
                if *value {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                }
            }
        }
    }
}

/// The keys Gnoblin writes into the layer.
///
/// Every key is one GTK actually reads for one of the facts D15 names: fonts, icon
/// theme, cursor theme and size, button layout, colour scheme and accent colour.
#[must_use]
pub fn publish(settings: &DesktopSettings) -> Vec<KeyWrite> {
    let mut writes = vec![
        KeyWrite {
            group: INTERFACE,
            key: "font-name",
            value: DconfValue::Text(settings.font.clone()),
        },
        KeyWrite {
            group: INTERFACE,
            key: "monospace-font-name",
            value: DconfValue::Text(settings.monospace_font.clone()),
        },
        KeyWrite {
            group: INTERFACE,
            key: "icon-theme",
            value: DconfValue::Text(settings.icon_theme.clone()),
        },
        KeyWrite {
            group: INTERFACE,
            key: "cursor-theme",
            value: DconfValue::Text(settings.cursor_theme.clone()),
        },
        KeyWrite {
            group: INTERFACE,
            key: "cursor-size",
            value: DconfValue::Integer(i32::try_from(settings.cursor_size).unwrap_or(i32::MAX)),
        },
        KeyWrite {
            group: INTERFACE,
            key: "color-scheme",
            value: DconfValue::Text(settings.colour_scheme.as_str().to_owned()),
        },
        KeyWrite {
            group: WINDOW_PREFERENCES,
            key: "titlebar-font",
            value: DconfValue::Text(settings.titlebar_font.clone()),
        },
        KeyWrite {
            group: WINDOW_PREFERENCES,
            key: "button-layout",
            value: DconfValue::Text(settings.button_layout.clone()),
        },
    ];
    if let Some(accent) = settings.accent {
        writes.push(KeyWrite {
            group: INTERFACE,
            key: "accent-color",
            value: DconfValue::Text(accent.name().to_owned()),
        });
    }
    writes
}

/// The actions as the dconf keyfile `dconf compile` reads.
///
/// Groups appear once and in the order of their first action, and keys appear in the
/// order [`publish`] emits them, so the same value always renders the same bytes.
#[must_use]
pub fn keyfile(actions: &[KeyWrite]) -> String {
    let mut out = String::new();
    let mut group: Option<&str> = None;
    for action in actions {
        if group != Some(action.group) {
            if group.is_some() {
                out.push('\n');
            }
            out.push('[');
            out.push_str(action.group);
            out.push_str("]\n");
            group = Some(action.group);
        }
        out.push_str(action.key);
        out.push('=');
        out.push_str(&action.value.render());
        out.push('\n');
    }
    out
}

/// What [`apply`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Written {
    /// The target is off, so nothing was written and nothing was read.
    Skipped,
    /// The keyfile was written, and whether the database beside it was compiled.
    Keyfile {
        /// Where the keyfile is.
        path: PathBuf,
        /// Whether `dconf compile` produced a database from it.
        compiled: bool,
        /// Why it did not, when it did not.
        reason: Option<String>,
    },
}

impl Written {
    /// Whether a database was produced, which is what GTK can actually read.
    #[must_use]
    pub fn compiled(&self) -> bool {
        matches!(self, Self::Keyfile { compiled: true, .. })
    }
}

/// Writes the layer into `directory`, and compiles it when `dconf` is installed.
///
/// `directory` is the session's dconf profile directory, not the user's
/// `~/.config/dconf`: the caller decides where a Gnoblin-owned profile lives and sets
/// `DCONF_PROFILE` to it.
///
/// # Errors
///
/// The I/O error when the keyfile cannot be written. A missing `dconf` binary is not an
/// error: it is [`Written::Keyfile`] with `compiled: false` and the reason, because a
/// session on a system without dconf still has its settings written down for the next
/// run.
pub fn apply(actions: &[KeyWrite], switch: Switch, directory: &Path) -> std::io::Result<Written> {
    if !switch.is_on() {
        log::debug!("[dconf] the target is off, so nothing is written");
        return Ok(Written::Skipped);
    }
    // `dconf compile` reads a directory of keyfiles, not one file, so the keyfile goes in its
    // own subdirectory and the database is compiled beside it.
    let keyfiles = directory.join("keyfiles");
    std::fs::create_dir_all(&keyfiles)?;
    let path = keyfiles.join("gnoblin");
    std::fs::write(&path, keyfile(actions))?;
    let database = directory.join("gnoblin.db");
    let result = Command::new("dconf")
        .arg("compile")
        .arg(&database)
        .arg(&keyfiles)
        .output();
    let written = match result {
        Ok(output) if output.status.success() => Written::Keyfile {
            path,
            compiled: true,
            reason: None,
        },
        Ok(output) => Written::Keyfile {
            path,
            compiled: false,
            reason: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        },
        Err(error) => Written::Keyfile {
            path,
            compiled: false,
            reason: Some(format!("`dconf` is not installed: {error}")),
        },
    };
    if !written.compiled() {
        log::debug!("[dconf] the layer is written but not compiled");
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are GNOME's own, which is what a session that configures nothing
    /// publishes: a tool that read the layer would see no change from a stock session.
    #[test]
    fn the_defaults_are_gnomes_own() {
        let actions = publish(&DesktopSettings::default());
        let find = |key: &str| {
            actions
                .iter()
                .find(|action| action.key == key)
                .map(|action| action.value.clone())
        };
        assert_eq!(find("font-name"), Some(DconfValue::Text("Cantarell 11".to_owned())));
        assert_eq!(find("icon-theme"), Some(DconfValue::Text("Adwaita".to_owned())));
        assert_eq!(find("cursor-size"), Some(DconfValue::Integer(24)));
        assert_eq!(
            find("button-layout"),
            Some(DconfValue::Text("appmenu:minimize,maximize,close".to_owned()))
        );
        assert_eq!(find("color-scheme"), Some(DconfValue::Text("default".to_owned())));
    }

    /// The keyfile is the format `dconf compile` reads, and the same value renders the
    /// same bytes.
    #[test]
    fn the_keyfile_is_grouped_by_schema() {
        let actions = publish(&DesktopSettings::default());
        let text = keyfile(&actions);
        assert!(text.starts_with("[org/gnome/desktop/interface]\n"), "{text}");
        assert_eq!(text.matches("[org/gnome/desktop/interface]").count(), 1);
        assert_eq!(text.matches("[org/gnome/desktop/wm/preferences]").count(), 1);
        assert!(text.contains("font-name='Cantarell 11'\n"), "{text}");
        assert!(text.contains("cursor-size=24\n"), "{text}");
        assert_eq!(text, keyfile(&publish(&DesktopSettings::default())));
    }

    /// A font name with a quote and a backslash round-trips, so the keyfile cannot be
    /// broken by a name a person typed.
    #[test]
    fn a_font_name_survives_the_keyfile() {
        let settings = DesktopSettings {
            font: "Odd'Name \\ 12".to_owned(),
            colour_scheme: gnoblin_compat_core::ColourScheme::PreferDark,
            ..DesktopSettings::default()
        };
        let text = keyfile(&publish(&settings));
        assert!(
            text.contains("font-name='Odd\\'Name \\\\ 12'"),
            "the value is quoted and escaped: {text}"
        );
        assert!(text.contains("color-scheme='prefer-dark'"));
    }

    /// A target that is off writes nothing at all, whichever directory it is given.
    #[test]
    fn a_switched_off_target_writes_nothing() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let actions = publish(&DesktopSettings::default());
        let written = apply(&actions, Switch::Off, &directory.path().join("gnoblin-dconf")).expect("no error");
        assert_eq!(written, Written::Skipped);
        assert!(
            !directory.path().join("gnoblin-dconf").exists(),
            "an off target does not even create its directory"
        );
    }

    /// Another process changing the database cannot change what is published, because
    /// nothing here reads a database.
    #[test]
    fn a_changed_database_is_never_read_back() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let profile = directory.path().join("gnoblin-dconf");
        std::fs::create_dir_all(&profile).expect("the profile directory");
        // What a user's own `gsettings set` would have written.
        let theirs = profile.join("gnoblin.db");
        std::fs::write(&theirs, b"not a database, and not read").expect("the file");
        let settings = DesktopSettings::default();
        let before = publish(&settings);
        std::fs::write(&theirs, b"still not a database").expect("the file");
        let after = publish(&settings);
        assert_eq!(before, after);
        assert_eq!(keyfile(&before), keyfile(&after));
    }

    /// The keyfile is written and the compile answer is reported rather than assumed.
    #[test]
    fn an_uncompiled_keyfile_is_reported() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let profile = directory.path().join("gnoblin-dconf");
        let written =
            apply(&publish(&DesktopSettings::default()), Switch::On, &profile).expect("the keyfile is written");
        let Written::Keyfile { path, compiled, reason } = written else {
            panic!("an on target writes a keyfile");
        };
        assert!(path.exists(), "the keyfile is there even without dconf");
        let database = profile.join("gnoblin.db");
        assert_eq!(
            compiled,
            database.exists(),
            "`compiled` follows the database, not a hope"
        );
        assert_eq!(compiled, reason.is_none(), "a reason is given exactly when it failed");
        if compiled {
            // `dconf` is installed on this machine, so the compiled form is the one that
            // matters: an empty database would mean the keyfile was rejected.
            let size = std::fs::metadata(&database).expect("the database").len();
            assert!(size > 0, "a compiled database is not empty");
        } else {
            println!(
                "[dconf] not compiled: {} (the keyfile path is tested regardless)",
                reason.expect("a reason")
            );
        }
    }
    /// The checked-in declaration is what `cargo xtask compat-report` prints, and it is
    /// parsed here so a key the crate writes and the manifest does not declare is a test
    /// failure rather than a report nobody reads.
    #[test]
    fn the_manifest_declares_every_key_the_layer_writes() {
        let matrix = gnoblin_compat_core::SupportMatrix::from_json(crate::MATRIX).expect("the checked-in manifest");
        for action in publish(&DesktopSettings::default()) {
            assert!(
                matrix.support(action.group, action.key).is_some(),
                "{} {} is written but not declared",
                action.group,
                action.key
            );
        }
    }
}
