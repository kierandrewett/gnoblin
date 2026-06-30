use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

fn applications_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = crate::xdg::data_home() {
        dirs.push(d.join("applications"));
    }
    for d in crate::xdg::data_dirs() {
        dirs.push(d.join("applications"));
    }
    dirs
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopEntryFile {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopExec {
    program: String,
    args: Vec<String>,
    terminal: bool,
    working_dir: Option<PathBuf>,
}

struct DesktopExecMetadata {
    name: Option<String>,
    icon: Option<String>,
    desktop_file: String,
}

#[derive(Clone, Debug, Default)]
struct DesktopActionDraft {
    name: Option<String>,
    icon: Option<String>,
    exec: Option<String>,
    terminal: bool,
    working_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopActionExec {
    action: DesktopAction,
    exec: DesktopExec,
}

fn desktop_id_for_path(applications_dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(applications_dir).ok()?;
    let mut parts = Vec::new();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_string_lossy().to_string());
    }
    let id = parts.join("-");
    id.strip_suffix(".desktop").map(|id| id.to_string())
}

fn collect_desktop_entries(
    applications_dir: &Path,
    dir: &Path,
    entries: &mut Vec<DesktopEntryFile>,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read_dir.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_desktop_entries(applications_dir, &path, entries);
        } else if path.extension().map(|e| e == "desktop").unwrap_or(false) {
            if let Some(id) = desktop_id_for_path(applications_dir, &path) {
                entries.push(DesktopEntryFile { id, path });
            }
        }
    }
}

/// Installed desktop entries in XDG data-dir precedence order. IDs are returned
/// without the `.desktop` suffix, matching the argument accepted by `gtk-launch`.
pub fn installed_desktop_entries() -> Vec<DesktopEntryFile> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for dir in applications_dirs() {
        let mut dir_entries = Vec::new();
        collect_desktop_entries(&dir, &dir, &mut dir_entries);
        for entry in dir_entries {
            if seen.insert(entry.id.clone()) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn desktop_id_tail(id: &str) -> &str {
    let dot = id.rfind('.');
    let dash = id.rfind('-');
    match (dot, dash) {
        (Some(a), Some(b)) => &id[a.max(b) + 1..],
        (Some(i), None) | (None, Some(i)) => &id[i + 1..],
        (None, None) => id,
    }
}

fn desktop_id_match_rank(candidate: &str, want: &str) -> Option<usize> {
    if candidate.rsplit('.').next() == Some(want) {
        return Some(0);
    }
    if desktop_id_tail(candidate) == want {
        return Some(1);
    }
    if candidate.ends_with(&format!(".{want}")) {
        return Some(2);
    }
    if candidate.ends_with(&format!("-{want}")) {
        return Some(3);
    }
    None
}

/// Resolve a user-facing/dock favourite id to the installed `.desktop` id that
/// `gtk-launch` expects. Exact ids win; otherwise tolerate legacy short ids like
/// `firefox` for an installed `org.mozilla.firefox.desktop`.
pub fn resolve_desktop_id(id: &str) -> Option<String> {
    let id = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    if id.is_empty() {
        return None;
    }
    let ids: Vec<String> = installed_desktop_entries()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    if ids.iter().any(|candidate| candidate == id) {
        return Some(id.to_string());
    }
    let want = id.to_lowercase();
    ids.into_iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let c = candidate.to_lowercase();
            desktop_id_match_rank(&c, &want).map(|rank| (rank, index, candidate))
        })
        .min_by_key(|(rank, index, _)| (*rank, *index))
        .map(|(_, _, candidate)| candidate)
}

/// Launch an installed desktop application detached from the shell client.
/// `gtk-launch` exits quickly but still needs reaping; otherwise repeated dock
/// clicks leave zombies until the dock/launcher exits.
pub fn launch_desktop_app(id: &str) {
    let requested = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    if requested.is_empty() {
        return;
    }
    let launch_id = match resolve_desktop_id(requested) {
        Some(resolved) => {
            if resolved != requested {
                eprintln!("gnoblin: desktop app '{requested}' resolved to '{resolved}'");
            }
            resolved
        }
        None => {
            eprintln!("gnoblin: desktop app '{requested}' not found in XDG application dirs");
            requested.to_string()
        }
    };
    if let Some(entry) = installed_desktop_entry(&launch_id) {
        if desktop_entry_dbus_activatable(&entry.path) {
            if let Some(exec) = desktop_exec(&entry.path) {
                if spawn_desktop_exec_command(&launch_id, exec) {
                    return;
                }
            }
        }
    }
    let fallback_id = launch_id.clone();
    match Command::new("gtk-launch")
        .arg(&launch_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    eprintln!("gnoblin: gtk-launch '{launch_id}' exited with {status}");
                    spawn_desktop_exec_fallback(&fallback_id);
                }
                Err(e) => {
                    eprintln!("gnoblin: gtk-launch '{launch_id}' wait failed: {e}");
                    spawn_desktop_exec_fallback(&fallback_id);
                }
            });
        }
        Err(e) => {
            eprintln!("gnoblin: failed to spawn gtk-launch '{launch_id}': {e}");
            spawn_desktop_exec_fallback(&fallback_id);
        }
    }
}

pub fn desktop_actions(id: &str) -> Vec<DesktopAction> {
    desktop_action_execs(id)
        .into_iter()
        .map(|a| a.action)
        .collect()
}

pub fn launch_desktop_action(id: &str, action_id: &str) -> bool {
    let Some(action) = desktop_action_execs(id)
        .into_iter()
        .find(|a| a.action.id == action_id)
    else {
        eprintln!("gnoblin: desktop action '{action_id}' not found for '{id}'");
        return false;
    };
    spawn_desktop_exec_command(id, action.exec)
}

fn desktop_action_execs(id: &str) -> Vec<DesktopActionExec> {
    let requested = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    let Some(launch_id) = resolve_desktop_id(requested) else {
        return Vec::new();
    };
    let Some(entry) = installed_desktop_entry(&launch_id) else {
        return Vec::new();
    };
    parse_desktop_actions(&entry.path)
}

fn parse_desktop_actions(path: &Path) -> Vec<DesktopActionExec> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut action_order: Vec<String> = Vec::new();
    let mut action_seen = HashSet::new();
    let mut drafts: HashMap<String, DesktopActionDraft> = HashMap::new();
    let mut current_action: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_action = line
                .strip_prefix("[Desktop Action ")
                .and_then(|s| s.strip_suffix(']'))
                .map(str::to_string);
            if let Some(id) = &current_action {
                if action_seen.insert(id.clone()) {
                    action_order.push(id.clone());
                }
                drafts.entry(id.clone()).or_default();
            }
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if current_action.is_none() {
            if key == "Actions" {
                for id in value.split(';').map(str::trim).filter(|id| !id.is_empty()) {
                    if action_seen.insert(id.to_string()) {
                        action_order.push(id.to_string());
                    }
                    drafts.entry(id.to_string()).or_default();
                }
            }
            continue;
        }

        let Some(id) = current_action.as_ref().cloned() else {
            continue;
        };
        let draft = drafts.entry(id).or_default();
        match key {
            "Name" if draft.name.is_none() => {
                draft.name = Some(unescape_desktop_string(value));
            }
            "Icon" if draft.icon.is_none() => {
                draft.icon = Some(unescape_desktop_string(value));
            }
            "Exec" if draft.exec.is_none() => {
                draft.exec = Some(value.to_string());
            }
            "Terminal" => draft.terminal = value == "true",
            "Path" if draft.working_dir.is_none() => {
                let path = unescape_desktop_string(value);
                if !path.is_empty() {
                    draft.working_dir = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }

    action_order
        .into_iter()
        .filter_map(|id| {
            let draft = drafts.remove(&id)?;
            let name = draft.name?;
            let exec = parse_desktop_exec(
                draft.exec.as_deref()?,
                &DesktopExecMetadata {
                    name: Some(name.clone()),
                    icon: draft.icon,
                    desktop_file: path.display().to_string(),
                },
                draft.terminal,
                draft.working_dir,
            )?;
            Some(DesktopActionExec {
                action: DesktopAction { id, name },
                exec,
            })
        })
        .collect()
}

fn installed_desktop_entry(id: &str) -> Option<DesktopEntryFile> {
    installed_desktop_entries()
        .into_iter()
        .find(|entry| entry.id == id)
}

fn desktop_entry_dbus_activatable(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "DBusActivatable" {
            return value.trim() == "true";
        }
    }
    false
}

fn desktop_exec(path: &Path) -> Option<DesktopExec> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut exec = None;
    let mut name = None;
    let mut icon = None;
    let mut terminal = false;
    let mut working_dir = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Exec" if exec.is_none() => exec = Some(value.trim().to_string()),
            "Name" if name.is_none() => name = Some(unescape_desktop_string(value.trim())),
            "Icon" if icon.is_none() => icon = Some(unescape_desktop_string(value.trim())),
            "Terminal" => terminal = value.trim() == "true",
            "Path" if working_dir.is_none() => {
                let path = unescape_desktop_string(value.trim());
                if !path.is_empty() {
                    working_dir = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }
    parse_desktop_exec(
        &exec?,
        &DesktopExecMetadata {
            name,
            icon,
            desktop_file: path.display().to_string(),
        },
        terminal,
        working_dir,
    )
}

fn parse_desktop_exec(
    value: &str,
    metadata: &DesktopExecMetadata,
    terminal: bool,
    working_dir: Option<PathBuf>,
) -> Option<DesktopExec> {
    let value = unescape_desktop_string(value);
    let argv = split_desktop_exec_words(&value)?;
    let mut argv = argv
        .into_iter()
        .flat_map(|arg| expand_desktop_exec_field_codes(arg, metadata));
    let program = argv.next()?.trim().to_string();
    if program.is_empty() {
        return None;
    }
    Some(DesktopExec {
        program,
        args: argv.collect(),
        terminal,
        working_dir,
    })
}

fn split_desktop_exec_words(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = value.chars();
    let mut in_quotes = false;
    let mut in_word = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                in_word = true;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    match next {
                        '"' | '`' | '$' | '\\' => word.push(next),
                        _ => {
                            word.push('\\');
                            word.push(next);
                        }
                    }
                } else {
                    word.push('\\');
                }
                in_word = true;
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            _ => {
                word.push(ch);
                in_word = true;
            }
        }
    }

    if in_quotes {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
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

fn expand_desktop_exec_field_codes(arg: String, metadata: &DesktopExecMetadata) -> Vec<String> {
    if arg == "%i" {
        return metadata
            .icon
            .as_ref()
            .map(|icon| vec!["--icon".to_string(), icon.clone()])
            .unwrap_or_default();
    }

    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    let mut removed_field_code = false;
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('i') => {
                if let Some(icon) = &metadata.icon {
                    out.push_str(icon);
                } else {
                    removed_field_code = true;
                }
            }
            Some('c') => {
                if let Some(name) = &metadata.name {
                    out.push_str(name);
                } else {
                    removed_field_code = true;
                }
            }
            Some('k') => out.push_str(&metadata.desktop_file),
            Some('f' | 'F' | 'u' | 'U' | 'v' | 'm') => {
                removed_field_code = true;
            }
            Some(other) if other.is_ascii_alphabetic() => {
                removed_field_code = true;
            }
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    if out.is_empty() && removed_field_code {
        Vec::new()
    } else {
        vec![out]
    }
}

fn spawn_desktop_exec_fallback(id: &str) -> bool {
    let Some(entry) = installed_desktop_entry(id) else {
        eprintln!("gnoblin: no desktop file found for Exec fallback '{id}'");
        return false;
    };
    let Some(exec) = desktop_exec(&entry.path) else {
        eprintln!("gnoblin: desktop file '{id}' has no Exec fallback");
        return false;
    };
    spawn_desktop_exec_command(id, exec)
}

fn spawn_desktop_exec_command(id: &str, exec: DesktopExec) -> bool {
    let exec = if exec.terminal {
        match terminal_command_for_exec(&exec) {
            Some(exec) => exec,
            None => {
                eprintln!("gnoblin: desktop file '{id}' requires a terminal, but no terminal emulator was found");
                return false;
            }
        }
    } else {
        exec
    };
    let mut command = Command::new(&exec.program);
    command
        .args(&exec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_dir) = &exec.working_dir {
        command.current_dir(working_dir);
    }
    match command.spawn() {
        Ok(mut child) => {
            let id = id.to_string();
            std::thread::spawn(move || match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("gnoblin: Exec fallback '{id}' exited with {status}"),
                Err(e) => eprintln!("gnoblin: Exec fallback '{id}' wait failed: {e}"),
            });
            true
        }
        Err(e) => {
            eprintln!("gnoblin: failed to spawn Exec fallback '{id}': {e}");
            false
        }
    }
}

fn terminal_command_for_exec(exec: &DesktopExec) -> Option<DesktopExec> {
    if let Some(custom) = std::env::var("GNOBLIN_TERMINAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        let mut words = split_desktop_exec_words(&custom)?;
        if !words.is_empty() {
            let program = words.remove(0);
            if command_available(&program) {
                words.push(exec.program.clone());
                words.extend(exec.args.clone());
                return Some(DesktopExec {
                    program,
                    args: words,
                    terminal: false,
                    working_dir: exec.working_dir.clone(),
                });
            }
        }
    }

    for (program, prefix) in [
        ("ptyxis", &["--new-window", "--"][..]),
        ("foot", &[][..]),
        ("alacritty", &["-e"][..]),
        ("kitty", &[][..]),
        ("gnome-terminal", &["--"][..]),
        ("kgx", &["--"][..]),
        ("konsole", &["-e"][..]),
        ("xterm", &["-e"][..]),
    ] {
        if !command_available(program) {
            continue;
        }
        let mut args: Vec<String> = prefix.iter().map(|arg| (*arg).to_string()).collect();
        args.push(exec.program.clone());
        args.extend(exec.args.clone());
        return Some(DesktopExec {
            program: program.to_string(),
            args,
            terminal: false,
            working_dir: exec.working_dir.clone(),
        });
    }

    None
}

fn command_available(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return is_executable_file(path);
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| is_executable_file(&dir.join(program)))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod desktop_entry_tests {
    use super::*;
    use crate::test_support::{env_lock, temp_root, EnvVar};
    use std::path::{Path, PathBuf};

    fn write_executable(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn expected_desktop_exec(program: String, args: Vec<String>) -> DesktopExec {
        DesktopExec {
            program,
            args,
            terminal: false,
            working_dir: None,
        }
    }

    #[test]
    fn applications_dirs_use_default_data_dirs_when_env_is_empty() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("xdg-data-dirs-empty");
        let data_home = root.join("data");
        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set_str("XDG_DATA_DIRS", "");

        let dirs = applications_dirs();

        assert_eq!(dirs[0], data_home.join("applications"));
        assert!(dirs.contains(&PathBuf::from("/usr/local/share/applications")));
        assert!(dirs.contains(&PathBuf::from("/usr/share/applications")));
    }

    #[test]
    fn installed_entries_use_xdg_ids_for_nested_desktop_files() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("nested");
        let data_home = root.join("data");
        let appdir = data_home.join("applications").join("RPCS3");
        std::fs::create_dir_all(&appdir).unwrap();
        let desktop_path = appdir.join("RPCS3.desktop");
        std::fs::write(
            &desktop_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=RPCS3\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let entries = installed_desktop_entries();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            entries,
            vec![DesktopEntryFile {
                id: "RPCS3-RPCS3".to_string(),
                path: desktop_path
            }]
        );
    }

    #[test]
    fn installed_entries_use_first_xdg_precedence_match_per_id() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("precedence");
        let data_home = root.join("home-data");
        let data_dir = root.join("system-data");
        let home_appdir = data_home.join("applications");
        let system_appdir = data_dir.join("applications");
        std::fs::create_dir_all(&home_appdir).unwrap();
        std::fs::create_dir_all(&system_appdir).unwrap();
        let home_path = home_appdir.join("same.desktop");
        let system_path = system_appdir.join("same.desktop");
        std::fs::write(
            &home_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Home Override\n\
             Hidden=true\n",
        )
        .unwrap();
        std::fs::write(
            &system_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=System Copy\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &data_dir);
        let entries = installed_desktop_entries();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            entries,
            vec![DesktopEntryFile {
                id: "same".to_string(),
                path: home_path
            }]
        );
    }

    #[test]
    fn desktop_actions_parse_ordered_action_sections() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("desktop-actions");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("firefox.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Firefox\n\
             Exec=firefox %u\n\
             Actions=new-window;private-window;profile-manager;\n\
             \n\
             [Desktop Action private-window]\n\
             Name=New Private Window\n\
             Exec=firefox --private-window\n\
             \n\
             [Desktop Action new-window]\n\
             Name=New Window\n\
             Exec=firefox --new-window %u\n\
             \n\
             [Desktop Action profile-manager]\n\
             Name=Open the Profile Manager\n\
             Exec=firefox --ProfileManager\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        let actions = desktop_actions("firefox");
        assert_eq!(
            actions,
            vec![
                DesktopAction {
                    id: "new-window".to_string(),
                    name: "New Window".to_string()
                },
                DesktopAction {
                    id: "private-window".to_string(),
                    name: "New Private Window".to_string()
                },
                DesktopAction {
                    id: "profile-manager".to_string(),
                    name: "Open the Profile Manager".to_string()
                },
            ]
        );

        let execs = desktop_action_execs("firefox");
        assert_eq!(
            execs[0].exec,
            expected_desktop_exec("firefox".to_string(), vec!["--new-window".to_string()])
        );
        assert_eq!(
            execs[2].exec,
            expected_desktop_exec("firefox".to_string(), vec!["--ProfileManager".to_string()])
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_accepts_nested_exact_and_tail_ids() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-nested");
        let data_home = root.join("data");
        let appdir = data_home.join("applications").join("RPCS3");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("RPCS3.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=RPCS3\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("RPCS3-RPCS3.desktop"),
            Some("RPCS3-RPCS3".to_string())
        );
        assert_eq!(resolve_desktop_id("RPCS3"), Some("RPCS3-RPCS3".to_string()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_accepts_reverse_dns_tail_ids() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-reverse-dns");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("org.mozilla.firefox.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Firefox\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("firefox"),
            Some("org.mozilla.firefox".to_string())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_prefers_exact_final_segment_over_longer_suffix() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-ambiguous-tail");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("com.example.More-Settings.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=More Settings\n\
             Exec=foot\n",
        )
        .unwrap();
        std::fs::write(
            appdir.join("org.gnome.Settings.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Settings\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("settings"),
            Some("org.gnome.Settings".to_string())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn desktop_exec_argv_parses_quoting_and_field_codes_without_shell() {
        let root = temp_root("desktop-exec-command");
        std::fs::create_dir_all(&root).unwrap();
        let app_path = root.join("Arg Recorder");
        let desktop_path = root.join("settings.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings\n\
                 Exec=\"{}\" \"\\$HOME\" \"two words\" --percent=%% --url=%U %F\n\
                 DBusActivatable=true\n",
                app_path.display()
            ),
        )
        .unwrap();

        assert_eq!(
            desktop_exec(&desktop_path),
            Some(expected_desktop_exec(
                app_path.to_string_lossy().to_string(),
                vec![
                    "$HOME".to_string(),
                    "two words".to_string(),
                    "--percent=%".to_string(),
                    "--url=".to_string(),
                ],
            ))
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn desktop_exec_expands_icon_name_and_desktop_file_field_codes() {
        let root = temp_root("desktop-exec-metadata");
        std::fs::create_dir_all(&root).unwrap();
        let app_path = root.join("Arg Recorder");
        let desktop_path = root.join("settings.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings App\n\
                 Icon=org.example.Settings\n\
                 Exec=\"{}\" %i --name=%c --desktop=%k\n",
                app_path.display()
            ),
        )
        .unwrap();

        assert_eq!(
            desktop_exec(&desktop_path),
            Some(expected_desktop_exec(
                app_path.to_string_lossy().to_string(),
                vec![
                    "--icon".to_string(),
                    "org.example.Settings".to_string(),
                    "--name=Settings App".to_string(),
                    format!("--desktop={}", desktop_path.display()),
                ],
            ))
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_falls_back_to_exec_when_gtk_launch_fails() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("gtk-launch-fallback");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let marker = root.join("launched");
        let recorder = bin_dir.join("record-launch");
        write_executable(&recorder, "#!/bin/sh\nprintf launched > \"$1\"\n");
        let desktop_path = appdir.join("fake.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Fake App\n\
                 Exec=\"{}\" \"{}\"\n",
                recorder.display(),
                marker.display()
            ),
        )
        .unwrap();
        let gtk_launch = bin_dir.join("gtk-launch");
        write_executable(&gtk_launch, "#!/bin/sh\nexit 1\n");

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("fake");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !marker.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&marker).ok().as_deref(),
            Some("launched")
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_uses_exec_for_dbus_activatable_entries() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("dbus-activatable-exec");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let launched = root.join("launched");
        let gtk_launch_called = root.join("gtk-launch-called");
        let recorder = bin_dir.join("record-dbus-launch");
        write_executable(&recorder, "#!/bin/sh\nprintf launched > \"$1\"\n");
        std::fs::write(
            appdir.join("org.example.Settings.desktop"),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings\n\
                 Exec=\"{}\" \"{}\"\n\
                 DBusActivatable=true\n",
                recorder.display(),
                launched.display()
            ),
        )
        .unwrap();
        let gtk_launch = bin_dir.join("gtk-launch");
        write_executable(
            &gtk_launch,
            &format!(
                "#!/bin/sh\nprintf called > {}\nexit 0\n",
                gtk_launch_called.display()
            ),
        );

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("org.example.Settings");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !launched.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&launched).ok().as_deref(),
            Some("launched")
        );
        assert!(!gtk_launch_called.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_wraps_terminal_exec_fallback_in_terminal() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("terminal-exec-fallback");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        let workdir = root.join("work dir");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let args_marker = root.join("terminal-args");
        let cwd_marker = root.join("terminal-cwd");
        let app_path = bin_dir.join("terminal-app");
        write_executable(&app_path, "#!/bin/sh\nexit 0\n");
        write_executable(
            &bin_dir.join("ptyxis"),
            &format!(
                "#!/bin/sh\npwd > '{}'\nprintf '%s\\n' \"$@\" > '{}'\n",
                cwd_marker.display(),
                args_marker.display()
            ),
        );
        write_executable(&bin_dir.join("gtk-launch"), "#!/bin/sh\nexit 1\n");
        std::fs::write(
            appdir.join("terminal.desktop"),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Terminal App\n\
                 Exec=\"{}\" \"two words\"\n\
                 Path={}\n\
                 Terminal=true\n",
                app_path.display(),
                workdir.display()
            ),
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("terminal");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !args_marker.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&cwd_marker).ok().as_deref(),
            Some(format!("{}\n", workdir.display()).as_str())
        );
        assert_eq!(
            std::fs::read_to_string(&args_marker).ok().as_deref(),
            Some(format!("--new-window\n--\n{}\ntwo words\n", app_path.display()).as_str())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}
