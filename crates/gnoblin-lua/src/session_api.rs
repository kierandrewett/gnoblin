//! `gnoblin.version()`: the build identity the runtime was installed with.
//!
//! The identity comes from the installed `version.ini` sidecar, which the packaging writes
//! beside the compositor. It is read on each call, so a reinstall is seen without a restart.
//! The record is immutable: its fields are reachable by name and by `pairs`, and a write or a
//! new field is refused.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use mlua::{Error, Function, Lua, MultiValue, Table, Value};

/// The value of a field the sidecar does not supply.
const UNKNOWN: &str = "unknown";

/// The `[version]` section of a `version.ini` sidecar: its non-empty values by key.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct VersionSidecar(BTreeMap<String, String>);

impl VersionSidecar {
    /// The value of `key`, or `None` when the sidecar leaves it empty or absent.
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str).filter(|value| !value.is_empty())
    }
}

/// Parse the `[version]` section of a `version.ini` text. Other sections are ignored, and so
/// are blank lines and `#` or `;` comments.
pub(crate) fn parse_identity(text: &str) -> VersionSidecar {
    let mut values = BTreeMap::new();
    let mut in_version = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_version = section.trim() == "version";
            continue;
        }
        if !in_version {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    VersionSidecar(values)
}

/// The sidecar paths to try, in order: the override, the executable's installation prefix,
/// then the system data directories. The first file that loads wins.
pub(crate) fn candidate_paths(
    override_path: Option<&OsStr>,
    executable: Option<&Path>,
    data_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_path.filter(|path| !path.is_empty()) {
        candidates.push(PathBuf::from(path));
    }
    if let Some(prefix) = executable.and_then(Path::parent).and_then(Path::parent) {
        candidates.push(prefix.join("share/gnoblin/version.ini"));
    }
    candidates.extend(data_dirs.iter().map(|dir| dir.join("gnoblin/version.ini")));
    candidates
}

/// The first candidate that can be read, parsed. A missing file is an empty identity.
fn load_identity(candidates: &[PathBuf]) -> VersionSidecar {
    candidates
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| parse_identity(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// The XDG system data directories, as GLib reads them.
fn system_data_dirs() -> Vec<PathBuf> {
    let raw = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share/:/usr/share/".into());
    raw.to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// A remote with its credentials removed: a URL keeps its scheme, host, port and path, and
/// loses its userinfo, query and fragment. SSH's scp-like spelling has no scheme, so anything
/// before its first `@` is dropped.
pub(crate) fn sanitize_git_remote(remote: &str) -> String {
    if remote.is_empty() {
        return UNKNOWN.to_owned();
    }
    if let Some((scheme, rest)) = split_scheme(remote) {
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let host_and_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        let after = &rest[authority_end..];
        let path = &after[..after.find(['?', '#']).unwrap_or(after.len())];
        return format!("{scheme}://{host_and_port}{path}");
    }
    remote.split_once('@').map_or(remote, |(_, rest)| rest).to_owned()
}

/// The scheme and the text after `://`, when `remote` has a syntactically valid scheme.
fn split_scheme(remote: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = remote.split_once("://")?;
    let mut chars = scheme.chars();
    let starts_alphabetic = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let valid = starts_alphabetic && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some((scheme, rest))
}

/// The eight record fields, in the order the documentation lists them.
fn record_values(identity: &VersionSidecar, lua_version: &str, api: &str) -> [(&'static str, String); 8] {
    let or_unknown = |key: &str| identity.get(key).unwrap_or(UNKNOWN).to_owned();
    [
        ("gnoblin", or_unknown("gnoblin")),
        ("gnome", or_unknown("gnome")),
        ("mutter", or_unknown("mutter")),
        ("lua", lua_version.to_owned()),
        ("api", api.to_owned()),
        (
            "git_remote",
            sanitize_git_remote(identity.get("git_remote").unwrap_or("")),
        ),
        ("git_sha", or_unknown("git_sha")),
        ("build_id", or_unknown("build_id")),
    ]
}

/// An immutable record: reads go to a private table, `pairs` walks it, and a write or a new
/// field raises an error.
fn read_only_record(lua: &Lua, next: &Function, values: [(&'static str, String); 8]) -> mlua::Result<Table> {
    let fields = lua.create_table()?;
    for (key, value) in values {
        fields.raw_set(key, value)?;
    }
    let meta = lua.create_table()?;
    meta.raw_set("__index", fields.clone())?;
    meta.raw_set(
        "__newindex",
        lua.create_function(|_, _: MultiValue| -> mlua::Result<()> {
            Err(Error::runtime("gnoblin.version() returns an immutable record"))
        })?,
    )?;
    let next = next.clone();
    meta.raw_set(
        "__pairs",
        lua.create_function(move |_, _: MultiValue| Ok((next.clone(), fields.clone(), Value::Nil)))?,
    )?;
    let record = lua.create_table()?;
    record.set_metatable(Some(meta))?;
    Ok(record)
}

/// Install `gnoblin.version` on the `gnoblin` table.
pub(crate) fn install(lua: &Lua, gnoblin: &Table) -> mlua::Result<()> {
    // Captured now, before a script runs, so a script that reassigns these globals cannot
    // change what the record reports.
    let lua_version: String = lua.globals().get("_VERSION")?;
    let next: Function = lua.globals().get("next")?;
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    let api = format!("{major}.{minor}.{patch}");
    let version = lua.create_function(move |lua, args: MultiValue| {
        if !args.is_empty() {
            return Err(Error::runtime("gnoblin.version takes no arguments"));
        }
        let candidates = candidate_paths(
            std::env::var_os("GNOBLIN_VERSION_METADATA_FILE").as_deref(),
            std::env::current_exe().ok().as_deref(),
            &system_data_dirs(),
        );
        let identity = load_identity(&candidates);
        read_only_record(lua, &next, record_values(&identity, &lua_version, &api))
    })?;
    gnoblin.set("version", version)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use super::{candidate_paths, parse_identity, sanitize_git_remote};

    #[test]
    fn a_remote_loses_its_credentials_and_query() {
        assert_eq!(
            sanitize_git_remote("https://user:token@git.example.org:8443/a/b.git?x=1#f"),
            "https://git.example.org:8443/a/b.git"
        );
        assert_eq!(sanitize_git_remote("ssh://git@host/repo.git"), "ssh://host/repo.git");
        assert_eq!(sanitize_git_remote("git@host:owner/repo.git"), "host:owner/repo.git");
        assert_eq!(sanitize_git_remote(""), "unknown");
        assert_eq!(sanitize_git_remote("/local/path"), "/local/path");
    }

    #[test]
    fn only_the_version_section_is_read() {
        let identity = parse_identity(
            "[other]\ngnoblin=0\n[version]\n# comment\ngnoblin = 1.2.3\ngit_sha=abc\n[more]\ngnome=49\n",
        );
        assert_eq!(identity.get("gnoblin"), Some("1.2.3"));
        assert_eq!(identity.get("git_sha"), Some("abc"));
        assert_eq!(identity.get("gnome"), None);
    }

    #[test]
    fn candidates_are_ordered_override_prefix_then_data_dirs() {
        let candidates = candidate_paths(
            Some(OsStr::new("/override/version.ini")),
            Some(Path::new("/usr/bin/gnoblin")),
            &[PathBuf::from("/usr/share")],
        );
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/override/version.ini"),
                PathBuf::from("/usr/share/gnoblin/version.ini"),
                PathBuf::from("/usr/share/gnoblin/version.ini"),
            ]
        );
    }
}
