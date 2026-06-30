//! Scan installed `.desktop` applications (XDG data dirs) — no gio dependency.

use std::collections::HashSet;
use std::path::Path;

#[derive(Clone)]
pub struct App {
    pub name: String,
    pub icon: String,
    /// The .desktop id (filename without extension) — launched via gtk-launch.
    pub id: String,
    /// Lowercased search text: name + generic-name + keywords + comment, so a
    /// query like "browser" finds Firefox.
    pub search: String,
}

/// Parse one `.desktop` file's `[Desktop Entry]` into an `App` (None if it's
/// hidden, NoDisplay, not an Application, or has no name).
fn parse(path: &Path, id: String) -> Option<App> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let locale_order = locale_key_order();
    let (mut name, mut icon) = (None, None);
    let (mut generic, mut keywords, mut comment) = (None, None, None);
    let (mut exec, mut try_exec) = (String::new(), String::new());
    let (mut only_show_in, mut not_show_in) = (String::new(), String::new());
    let (mut no_display, mut hidden, mut is_app, mut dbus_activatable) =
        (false, false, false, false);

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let value = v.trim();
        if set_localized_value(&mut name, key, value, "Name", &locale_order)
            || set_localized_value(&mut icon, key, value, "Icon", &locale_order)
            || set_localized_value(&mut generic, key, value, "GenericName", &locale_order)
            || set_localized_value(&mut keywords, key, value, "Keywords", &locale_order)
            || set_localized_value(&mut comment, key, value, "Comment", &locale_order)
        {
            continue;
        }
        match key {
            "Exec" if exec.is_empty() => exec = unescape_desktop_string(value),
            "TryExec" if try_exec.is_empty() => try_exec = unescape_desktop_string(value),
            "OnlyShowIn" if only_show_in.is_empty() => only_show_in = v.trim().to_string(),
            "NotShowIn" if not_show_in.is_empty() => not_show_in = v.trim().to_string(),
            "NoDisplay" => no_display = v.trim() == "true",
            "Hidden" => hidden = v.trim() == "true",
            "Type" => is_app = v.trim() == "Application",
            "DBusActivatable" => dbus_activatable = v.trim() == "true",
            _ => {}
        }
    }

    let name = name.map(|field| field.value).unwrap_or_default();
    let icon = icon.map(|field| field.value).unwrap_or_default();
    let generic = generic.map(|field| field.value).unwrap_or_default();
    let keywords = keywords.map(|field| field.value).unwrap_or_default();
    let comment = comment.map(|field| field.value).unwrap_or_default();

    if name.is_empty()
        || no_display
        || hidden
        || !is_app
        || (!dbus_activatable && exec.is_empty())
        || !desktop_visible(&only_show_in, &not_show_in)
        || !try_exec_available(&try_exec)
    {
        return None;
    }
    let search = format!("{name} {generic} {keywords} {comment}").to_lowercase();
    Some(App {
        name,
        icon,
        id,
        search,
    })
}

struct LocalizedValue {
    rank: usize,
    value: String,
}

fn set_localized_value(
    field: &mut Option<LocalizedValue>,
    key: &str,
    value: &str,
    base: &str,
    locale_order: &[String],
) -> bool {
    let Some(rank) = localized_key_rank(key, base, locale_order) else {
        return false;
    };
    if field
        .as_ref()
        .map(|current| rank < current.rank)
        .unwrap_or(true)
    {
        *field = Some(LocalizedValue {
            rank,
            value: unescape_desktop_string(value),
        });
    }
    true
}

fn localized_key_rank(key: &str, base: &str, locale_order: &[String]) -> Option<usize> {
    if key == base {
        return Some(locale_order.len());
    }
    let suffix = key.strip_prefix(base)?;
    let locale = suffix.strip_prefix('[')?.strip_suffix(']')?;
    locale_order
        .iter()
        .position(|candidate| candidate == locale)
}

fn locale_key_order() -> Vec<String> {
    let locale = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|v| !v.is_empty()))
        .or_else(|| std::env::var("LANG").ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    locale_candidates(&locale)
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let (locale_without_modifier, modifier) = locale
        .split_once('@')
        .map(|(base, modifier)| (base, Some(modifier)))
        .unwrap_or((locale, None));
    let Some(locale_base) = locale_without_modifier
        .split('.')
        .next()
        .filter(|v| !v.is_empty())
    else {
        return Vec::new();
    };
    let (lang, has_country) = locale_base
        .split_once('_')
        .map(|(lang, _)| (lang, true))
        .unwrap_or((locale_base, false));
    let mut candidates = Vec::new();
    if has_country {
        if let Some(modifier) = modifier {
            candidates.push(format!("{locale_base}@{modifier}"));
        }
        candidates.push(locale_base.to_string());
    }
    if let Some(modifier) = modifier {
        candidates.push(format!("{lang}@{modifier}"));
    }
    candidates.push(lang.to_string());
    candidates.dedup();
    candidates
}

fn unescape_desktop_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn desktop_visible(only_show_in: &str, not_show_in: &str) -> bool {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if !only_show_in.is_empty() {
        return desktop_list_contains(only_show_in, &desktops);
    }
    !desktop_list_contains(not_show_in, &desktops)
}

fn desktop_list_contains(list: &str, desktops: &[String]) -> bool {
    list.split(';')
        .filter(|s| !s.is_empty())
        .any(|entry| desktops.iter().any(|desktop| desktop == entry))
}

fn try_exec_available(try_exec: &str) -> bool {
    if try_exec.is_empty() {
        return true;
    }
    let path = Path::new(try_exec);
    if path.is_absolute() {
        return is_executable(path);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(try_exec))))
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// All launchable apps, de-duplicated by id (earlier dirs win), sorted by name.
pub fn scan() -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for entry in gnoblin_desktop::installed_desktop_entries() {
        if let Some(app) = parse(&entry.path, entry.id) {
            if seen.insert(app.id.clone()) {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVar {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvVar {
        fn set(key: &'static str, value: &Path) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }

        fn set_str(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                std::env::set_var(self.key, old);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gnoblin-launcher-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn scan_uses_xdg_ids_for_nested_desktop_entries() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("nested");
        let data_home = root.join("data");
        let appdir = data_home.join("applications").join("vendor");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("tool.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Nested Tool\n\
             GenericName=Launcher Test\n\
             Exec=foot\n\
             Icon=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let apps = scan();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "vendor-tool");
        assert_eq!(apps[0].name, "Nested Tool");
        assert!(apps[0].search.contains("launcher test"));
    }

    #[test]
    fn scan_filters_try_exec_and_missing_exec_entries() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("try-exec");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let ok_bin = bin_dir.join("ok-tool");
        std::fs::write(&ok_bin, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ok_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(
            appdir.join("ok.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Available Tool\n\
             Exec=foot\n\
             TryExec=ok-tool\n",
        )
        .unwrap();
        std::fs::write(
            appdir.join("missing-tryexec.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Missing Tool\n\
             Exec=foot\n\
             TryExec=definitely-not-installed-gnoblin-test\n",
        )
        .unwrap();
        std::fs::write(
            appdir.join("missing-exec.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=No Exec\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);
        let apps = scan();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "ok");
        assert_eq!(apps[0].name, "Available Tool");
    }

    #[test]
    fn scan_prefers_current_locale_strings() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("locale");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("colour.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Color Tool\n\
             Name[en_GB]=Colour Tool\n\
             GenericName=Network Analyzer\n\
             GenericName[en_GB]=Network Analyser\n\
             Comment=Organize colors\n\
             Comment[en_GB]=Organise colours\n\
             Keywords=color;trash;\n\
             Keywords[en_GB]=colour;wastebasket;\n\
             Icon=color-icon\n\
             Icon[en_GB]=colour-icon\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _lc_all = EnvVar::set_str("LC_ALL", "");
        let _lc_messages = EnvVar::set_str("LC_MESSAGES", "en_GB.UTF-8");
        let apps = scan();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Colour Tool");
        assert_eq!(apps[0].icon, "colour-icon");
        assert!(apps[0].search.contains("network analyser"));
        assert!(apps[0].search.contains("organise colours"));
        assert!(apps[0].search.contains("wastebasket"));
    }

    #[test]
    fn locale_candidates_follow_desktop_entry_fallback_order() {
        assert_eq!(
            locale_candidates("sr_YU.UTF-8@Latn"),
            vec!["sr_YU@Latn", "sr_YU", "sr@Latn", "sr"]
        );
        assert_eq!(locale_candidates("en_GB.UTF-8"), vec!["en_GB", "en"]);
        assert_eq!(locale_candidates("en@shaw"), vec!["en@shaw", "en"]);
        assert_eq!(locale_candidates("C.UTF-8"), vec!["C"]);
    }

    #[test]
    fn scan_decodes_desktop_string_escapes() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("escapes");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("escapes.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Escaped\\sName\n\
             GenericName=Line\\nBreak\n\
             Comment=Tabbed\\tComment\n\
             Keywords=alpha\\;beta;gamma;\n\
             Icon=escaped\\sicon\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let apps = scan();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "Escaped Name");
        assert_eq!(apps[0].icon, "escaped icon");
        assert!(apps[0].search.contains("line\nbreak"));
        assert!(apps[0].search.contains("tabbed\tcomment"));
        assert!(apps[0].search.contains("alpha;beta"));
    }

    #[test]
    fn scan_honors_only_show_in_and_not_show_in() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("desktop-env");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        for (file, name, extra) in [
            ("visible.desktop", "Visible", ""),
            ("gnome-only.desktop", "GNOME Only", "OnlyShowIn=GNOME;\n"),
            ("kde-only.desktop", "KDE Only", "OnlyShowIn=KDE;\n"),
            ("not-gnome.desktop", "Not GNOME", "NotShowIn=GNOME;\n"),
        ] {
            std::fs::write(
                appdir.join(file),
                format!(
                    "[Desktop Entry]\n\
                     Type=Application\n\
                     Name={name}\n\
                     Exec=foot\n\
                     {extra}"
                ),
            )
            .unwrap();
        }

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _current_desktop = EnvVar::set_str("XDG_CURRENT_DESKTOP", "GNOME:Gnoblin");
        let ids: Vec<String> = scan().into_iter().map(|app| app.id).collect();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(ids, vec!["gnome-only", "visible"]);
    }

    #[test]
    fn scan_does_not_fall_through_hidden_or_nodisplay_overrides() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("precedence-overrides");
        let data_home = root.join("home-data");
        let data_dir = root.join("system-data");
        let home_appdir = data_home.join("applications");
        let system_appdir = data_dir.join("applications");
        std::fs::create_dir_all(&home_appdir).unwrap();
        std::fs::create_dir_all(&system_appdir).unwrap();
        for (file, extra) in [
            ("hidden.desktop", "Hidden=true\n"),
            ("nodisplay.desktop", "NoDisplay=true\n"),
        ] {
            std::fs::write(
                home_appdir.join(file),
                format!(
                    "[Desktop Entry]\n\
                     Type=Application\n\
                     Name=Hidden Override\n\
                     Exec=foot\n\
                     {extra}"
                ),
            )
            .unwrap();
            std::fs::write(
                system_appdir.join(file),
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=System Copy\n\
                 Exec=foot\n",
            )
            .unwrap();
        }
        std::fs::write(
            system_appdir.join("visible.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Visible\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &data_dir);
        let ids: Vec<String> = scan().into_iter().map(|app| app.id).collect();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(ids, vec!["visible"]);
    }
}
