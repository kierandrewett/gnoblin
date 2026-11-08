//! `cargo xtask dist`: install the Gnoblin session into a prefix, or remove what an earlier install
//! put there.
//!
//! `install` copies the binaries the build wrote to `target/<profile>/` and the data files under
//! `data/` and the portal crate's `data/` into `<destdir><prefix>`, in the layout the binaries read:
//! `bin/gnoblin`, `libexec/xdg-desktop-portal-gnoblin`, `share/gnoblin/lua` and so on. Files that name
//! an absolute path (the Wayland session entry and the portal's D-Bus service) carry an `@PREFIX@`
//! placeholder, replaced with the prefix as the file is written.
//!
//! Every installed file is listed in `share/gnoblin/install-manifest` as `<sha256>  <path>`, relative
//! to the prefix. An install replaces only files the manifest lists and that still hold the content it
//! wrote. Anything else already at a destination is refused before the first write. `uninstall`
//! removes the listed files that are unchanged, keeps modified ones and reports them.
//!
//! Exit codes: 0 success, 1 a refused or failed install or uninstall, 2 a usage error.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

/// The token a template file uses where the install prefix goes.
const PLACEHOLDER: &str = "@PREFIX@";
/// Where the install manifest lives, relative to the prefix.
const MANIFEST: &str = "share/gnoblin/install-manifest";
const DEFAULT_PROFILE: &str = "release";

pub const USAGE: &str = "\
usage: cargo xtask dist install --prefix PREFIX [--destdir DIR] [--profile NAME]
       cargo xtask dist uninstall --prefix PREFIX [--destdir DIR]";

/// The permission bits an installed file gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileMode {
    Program,
    Data,
}

impl FileMode {
    const fn bits(self) -> u32 {
        match self {
            Self::Program => 0o755,
            Self::Data => 0o644,
        }
    }
}

/// Where an installed file comes from.
#[derive(Clone, Copy, Debug)]
enum InstallSource {
    /// A file in the repository, relative to its root.
    Repo(&'static str),
    /// A file the build wrote into `target/<profile>/`.
    Built(&'static str),
}

/// One line of the install table.
#[derive(Clone, Copy, Debug)]
enum InstallItem {
    /// One file, installed at `to` relative to the prefix.
    File {
        from: InstallSource,
        to: &'static str,
        mode: FileMode,
        template: bool,
    },
    /// Every file under a repository directory, kept in the same relative layout under `to`.
    Tree { from: &'static str, to: &'static str },
}

/// What the session installs. The optional packages (ananicy rules, the GeoClue authorisation file)
/// and the man pages are not in this table: their install paths belong to the distribution.
const ITEMS: &[InstallItem] = &[
    InstallItem::File {
        from: InstallSource::Built("gnoblin"),
        to: "bin/gnoblin",
        mode: FileMode::Program,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Built("gnoblinctl"),
        to: "bin/gnoblinctl",
        mode: FileMode::Program,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Built("xdg-desktop-portal-gnoblin"),
        to: "libexec/xdg-desktop-portal-gnoblin",
        mode: FileMode::Program,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Built("libpam_gnoblin_keyring.so"),
        to: "lib/security/pam_gnoblin_keyring.so",
        mode: FileMode::Program,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("data/session/gnoblin.desktop"),
        to: "share/wayland-sessions/gnoblin.desktop",
        mode: FileMode::Data,
        template: true,
    },
    InstallItem::File {
        from: InstallSource::Repo("data/session/systemd-user/gnoblin-session.target"),
        to: "lib/systemd/user/gnoblin-session.target",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo(
            "crates/xdg-desktop-portal-gnoblin/data/org.freedesktop.impl.portal.desktop.gnoblin.service",
        ),
        to: "share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service",
        mode: FileMode::Data,
        template: true,
    },
    InstallItem::File {
        from: InstallSource::Repo("crates/xdg-desktop-portal-gnoblin/data/gnoblin.portal"),
        to: "share/xdg-desktop-portal/portals/gnoblin.portal",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("data/gnoblin-portals.conf"),
        to: "share/xdg-desktop-portal/gnoblin-portals.conf",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("data/init.lua.example"),
        to: "share/gnoblin/init.lua.example",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("data/shaders/tint.frag"),
        to: "share/gnoblin/shaders/tint.frag",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::Tree {
        from: "data/default-config",
        to: "share/gnoblin/default-config",
    },
    InstallItem::Tree {
        from: "data/lua",
        to: "share/gnoblin/lua",
    },
];

/// One file the install would write, with its source resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Planned {
    to: String,
    source: PathBuf,
    mode: FileMode,
    template: bool,
}

/// The install manifest: installed path (relative to the prefix) to its sha256 digest.
type Manifest = BTreeMap<String, String>;

#[derive(Debug)]
pub enum DistError {
    /// The prefix is not a plain absolute path a desktop entry can carry.
    Prefix(String, &'static str),
    /// A destination already holds a file the manifest does not list.
    Foreign(PathBuf),
    /// A destination the manifest lists has changed since the install wrote it.
    Modified(PathBuf),
    /// `uninstall` found no manifest under the prefix.
    NoManifest(PathBuf),
    /// A manifest line that is not `<sha256>  <relative path>`.
    Manifest {
        path: PathBuf,
        line: usize,
        reason: &'static str,
    },
    /// Two install entries name the same destination.
    Duplicate(String),
    /// A source the install table names is not a regular file or directory, or is not text.
    Unsupported(PathBuf, &'static str),
    Io(PathBuf, io::Error),
}

impl fmt::Display for DistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prefix(prefix, reason) => write!(f, "prefix `{prefix}` {reason}"),
            Self::Foreign(path) => write!(
                f,
                "{} exists and the install manifest does not list it; remove it or choose another prefix",
                path.display()
            ),
            Self::Modified(path) => write!(
                f,
                "{} was changed after the last install; remove it, then install again",
                path.display()
            ),
            Self::NoManifest(path) => write!(f, "no install manifest at {}", path.display()),
            Self::Manifest { path, line, reason } => {
                write!(f, "{}:{line}: {reason}", path.display())
            }
            Self::Duplicate(to) => write!(f, "the install table names {to} twice"),
            Self::Unsupported(path, reason) => write!(f, "{} {reason}", path.display()),
            Self::Io(path, error) => write!(f, "{}: {error}", path.display()),
        }
    }
}

/// Maps an I/O error on `path` into a [`DistError`].
fn io_at(path: &Path) -> impl FnOnce(io::Error) -> DistError + '_ {
    move |error| DistError::Io(path.to_owned(), error)
}

/// The prefix must be an absolute path with no trailing slash, `..` component, whitespace, quotes,
/// backslash, `%` or `$`: a desktop entry's `Exec` line and a D-Bus service file both carry it.
fn validate_prefix(prefix: &str) -> Result<(), DistError> {
    let reason = if !prefix.starts_with('/') {
        "must be an absolute path"
    } else if prefix.len() > 1 && prefix.ends_with('/') {
        "must not end with a slash"
    } else if prefix.split('/').any(|part| part == "..") {
        "must not contain `..`"
    } else if prefix
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\\' | '%' | '$'))
    {
        "must not contain whitespace, control characters, quotes, backslashes, `%` or `$`"
    } else {
        return Ok(());
    };
    Err(DistError::Prefix(prefix.to_owned(), reason))
}

/// Replaces every `@PREFIX@` in a template with `prefix`, after checking the prefix.
fn render(template: &str, prefix: &str) -> Result<String, DistError> {
    validate_prefix(prefix)?;
    Ok(template.replace(PLACEHOLDER, prefix))
}

/// The directory an install writes under: `destdir` followed by the prefix, so a staged tree keeps
/// the absolute layout of the target system.
fn install_root(destdir: &Path, prefix: &str) -> PathBuf {
    let mut root = destdir.as_os_str().to_owned();
    root.push(prefix);
    PathBuf::from(root)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn format_manifest(entries: &Manifest) -> String {
    entries
        .iter()
        .map(|(path, digest)| format!("{digest}  {path}\n"))
        .collect()
}

fn parse_manifest(path: &Path, text: &str) -> Result<Manifest, DistError> {
    let mut entries = Manifest::new();
    for (index, line) in text.lines().enumerate() {
        let bad = |reason: &'static str| DistError::Manifest {
            path: path.to_owned(),
            line: index + 1,
            reason,
        };
        let Some((digest, rel)) = line.split_once("  ") else {
            return Err(bad("expected `<sha256>  <path>`"));
        };
        if !is_sha256(digest) {
            return Err(bad("the digest is not a lower-case sha256"));
        }
        if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|part| part == "..") {
            return Err(bad("the path must be relative and stay inside the prefix"));
        }
        if entries.insert(rel.to_owned(), digest.to_owned()).is_some() {
            return Err(bad("the path is listed twice"));
        }
    }
    Ok(entries)
}

/// What an install may do with a destination, given the digest of the file now there (if any) and
/// the digest the manifest recorded for it.
#[derive(Debug, PartialEq, Eq)]
enum Destination {
    Create,
    Replace,
    Foreign,
    Modified,
}

fn destination(current: Option<&str>, recorded: Option<&str>) -> Destination {
    match (current, recorded) {
        (None, _) => Destination::Create,
        (Some(now), Some(then)) if now == then => Destination::Replace,
        (Some(_), Some(_)) => Destination::Modified,
        (Some(_), None) => Destination::Foreign,
    }
}

/// What `uninstall`, or an install retiring a file it no longer ships, may do with a recorded file.
#[derive(Debug, PartialEq, Eq)]
enum Removal {
    Remove,
    Modified,
    Missing,
}

fn removal(current: Option<&str>, recorded: &str) -> Removal {
    match current {
        None => Removal::Missing,
        Some(now) if now == recorded => Removal::Remove,
        Some(_) => Removal::Modified,
    }
}

/// The digest of the file at `path`, or `None` if nothing is there.
fn current_digest(path: &Path) -> Result<Option<String>, DistError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(sha256_hex(&bytes))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DistError::Io(path.to_owned(), error)),
    }
}

fn read_manifest(path: &Path) -> Result<Manifest, DistError> {
    match fs::read_to_string(path) {
        Ok(text) => parse_manifest(path, &text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Manifest::new()),
        Err(error) => Err(DistError::Io(path.to_owned(), error)),
    }
}

/// Writes `contents` to `dest` through a sibling temporary file, so a reader never sees half a file.
fn write_file(dest: &Path, contents: &[u8], mode: FileMode) -> Result<(), DistError> {
    let parent = dest
        .parent()
        .ok_or_else(|| DistError::Unsupported(dest.to_owned(), "has no parent directory"))?;
    fs::create_dir_all(parent).map_err(io_at(parent))?;
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".gnoblin-dist");
    let temp = parent.join(name);
    fs::write(&temp, contents).map_err(io_at(&temp))?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(mode.bits())).map_err(io_at(&temp))?;
    fs::rename(&temp, dest).map_err(io_at(dest))
}

/// Removes the directories that `removed` left empty, stopping below `root`. A directory that still
/// holds files stops the walk, and so does any error: the directory is then not ours to remove.
fn prune_dirs(root: &Path, removed: &Path) {
    let mut dir = removed.parent();
    while let Some(current) = dir {
        if current == root || !current.starts_with(root) || fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

fn origin_path(repo: &Path, built: &Path, origin: InstallSource) -> PathBuf {
    match origin {
        InstallSource::Repo(path) => repo.join(path),
        InstallSource::Built(name) => built.join(name),
    }
}

/// Lists every file under `dir`, sorted, with its path relative to the directory `prefix` names.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<(), DistError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(io_at(dir))? {
        entries.push(entry.map_err(io_at(dir))?);
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DistError::Unsupported(path.clone(), "is not named in UTF-8"))?;
        let meta = fs::metadata(&path).map_err(io_at(&path))?;
        if meta.is_dir() {
            walk(&path, &format!("{prefix}{name}/"), out)?;
        } else if meta.is_file() {
            out.push((format!("{prefix}{name}"), path));
        } else {
            return Err(DistError::Unsupported(path, "is not a regular file or directory"));
        }
    }
    Ok(())
}

/// Expands the install table into one entry per file. Nothing is read here except directory
/// listings; the sources of built files are read when they are installed.
fn plan(repo: &Path, built: &Path) -> Result<Vec<Planned>, DistError> {
    let mut planned = Vec::new();
    for item in ITEMS {
        match *item {
            InstallItem::File {
                from,
                to,
                mode,
                template,
            } => planned.push(Planned {
                to: to.to_owned(),
                source: origin_path(repo, built, from),
                mode,
                template,
            }),
            InstallItem::Tree { from, to } => {
                let mut files = Vec::new();
                walk(&repo.join(from), "", &mut files)?;
                for (rel, source) in files {
                    planned.push(Planned {
                        to: format!("{to}/{rel}"),
                        source,
                        mode: FileMode::Data,
                        template: false,
                    });
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    for entry in &planned {
        if entry.to == MANIFEST || !seen.insert(entry.to.as_str()) {
            return Err(DistError::Duplicate(entry.to.clone()));
        }
    }
    Ok(planned)
}

#[derive(Debug, PartialEq, Eq)]
enum DistAction {
    Install,
    Uninstall,
}

#[derive(Debug, PartialEq, Eq)]
struct DistOptions {
    action: DistAction,
    prefix: String,
    destdir: PathBuf,
    profile: String,
    target_dir: PathBuf,
}

/// Parses `install|uninstall --prefix P [--destdir D] [--profile NAME]`. The target directory comes
/// from `CARGO_TARGET_DIR`, as cargo itself reads it, and defaults to `target`.
fn parse(args: &[String], target_dir: PathBuf) -> Result<DistOptions, String> {
    let (action, flags) = match args.split_first() {
        Some((verb, flags)) if verb == "install" => (DistAction::Install, flags),
        Some((verb, flags)) if verb == "uninstall" => (DistAction::Uninstall, flags),
        Some((verb, _)) => return Err(format!("unknown dist action `{verb}`")),
        None => return Err("expected `install` or `uninstall`".to_owned()),
    };
    let mut prefix = None;
    let mut destdir = PathBuf::new();
    let mut profile = DEFAULT_PROFILE.to_owned();
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| format!("`{flag}` needs a value"));
        match flag.as_str() {
            "--prefix" => prefix = Some(value()?),
            "--destdir" => destdir = PathBuf::from(value()?),
            "--profile" => profile = value()?,
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    let prefix = prefix.ok_or_else(|| "`--prefix` is required".to_owned())?;
    Ok(DistOptions {
        action,
        prefix,
        destdir,
        profile,
        target_dir,
    })
}

fn install(repo: &Path, options: &DistOptions) -> Result<(), DistError> {
    validate_prefix(&options.prefix)?;
    let root = install_root(&options.destdir, &options.prefix);
    let manifest_path = root.join(MANIFEST);
    let recorded = read_manifest(&manifest_path)?;
    let planned = plan(repo, &options.target_dir.join(&options.profile))?;

    // Preflight: every source is read and every destination checked before the first write, so a
    // refusal leaves the tree as it was.
    let mut staged = Vec::with_capacity(planned.len());
    for entry in &planned {
        let raw = fs::read(&entry.source).map_err(io_at(&entry.source))?;
        let contents = if entry.template {
            let text = String::from_utf8(raw)
                .map_err(|_| DistError::Unsupported(entry.source.clone(), "is a template but not UTF-8 text"))?;
            render(&text, &options.prefix)?.into_bytes()
        } else {
            raw
        };
        let digest = sha256_hex(&contents);
        let dest = root.join(&entry.to);
        let current = current_digest(&dest)?;
        match destination(current.as_deref(), recorded.get(&entry.to).map(String::as_str)) {
            Destination::Create | Destination::Replace => {}
            Destination::Foreign => return Err(DistError::Foreign(dest)),
            Destination::Modified => return Err(DistError::Modified(dest)),
        }
        staged.push((entry, dest, contents, digest));
    }

    // The manifest starts as the old record, and each file written is added as it lands. If a write
    // fails, the manifest still describes what is on disk, so `uninstall` can remove it.
    let mut manifest = recorded.clone();
    let mut failure = None;
    for (entry, dest, contents, digest) in &staged {
        if let Err(error) = write_file(dest, contents, entry.mode) {
            failure = Some(error);
            break;
        }
        manifest.insert(entry.to.clone(), digest.clone());
    }

    // Files the old manifest lists but this build no longer ships are retired, unless they were
    // changed since the install, in which case they stay listed for the user to decide.
    let wanted: BTreeSet<&str> = planned.iter().map(|entry| entry.to.as_str()).collect();
    let mut retired = Vec::new();
    let mut kept = Vec::new();
    if failure.is_none() {
        for (rel, digest) in &recorded {
            if wanted.contains(rel.as_str()) {
                continue;
            }
            let path = root.join(rel);
            match removal(current_digest(&path)?.as_deref(), digest) {
                Removal::Remove => {
                    fs::remove_file(&path).map_err(io_at(&path))?;
                    prune_dirs(&root, &path);
                    manifest.remove(rel);
                    retired.push(rel.clone());
                }
                Removal::Modified => kept.push(rel.clone()),
                Removal::Missing => {
                    manifest.remove(rel);
                }
            }
        }
    }

    write_file(&manifest_path, format_manifest(&manifest).as_bytes(), FileMode::Data)?;
    if let Some(error) = failure {
        return Err(error);
    }
    for rel in &retired {
        println!("[dist] retired {rel}: no longer shipped");
    }
    for rel in &kept {
        println!("[dist] kept {rel}: changed since the last install and no longer shipped");
    }
    println!("[dist] installed {} files under {}", staged.len(), root.display());
    Ok(())
}

fn uninstall(options: &DistOptions) -> Result<(), DistError> {
    validate_prefix(&options.prefix)?;
    let root = install_root(&options.destdir, &options.prefix);
    let manifest_path = root.join(MANIFEST);
    let text = match fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(DistError::NoManifest(manifest_path));
        }
        Err(error) => return Err(DistError::Io(manifest_path, error)),
    };
    let recorded = parse_manifest(&manifest_path, &text)?;

    let mut kept = Manifest::new();
    let mut removed = 0_usize;
    for (rel, digest) in &recorded {
        let path = root.join(rel);
        match removal(current_digest(&path)?.as_deref(), digest) {
            Removal::Remove => {
                fs::remove_file(&path).map_err(io_at(&path))?;
                prune_dirs(&root, &path);
                removed += 1;
            }
            Removal::Modified => {
                println!("[dist] kept {rel}: changed since the install");
                kept.insert(rel.clone(), digest.clone());
            }
            Removal::Missing => println!("[dist] missing {rel}: already removed"),
        }
    }
    if kept.is_empty() {
        fs::remove_file(&manifest_path).map_err(io_at(&manifest_path))?;
        prune_dirs(&root, &manifest_path);
    } else {
        write_file(&manifest_path, format_manifest(&kept).as_bytes(), FileMode::Data)?;
    }
    println!("[dist] removed {removed} files from {}", root.display());
    Ok(())
}

/// Entry point for `cargo xtask dist <args>`. The repository is the current directory, as for every
/// other xtask subcommand.
pub fn run(args: &[String]) -> ExitCode {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    let options = match parse(args, target_dir) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("[dist] {message}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match options.action {
        DistAction::Install => install(Path::new("."), &options),
        DistAction::Uninstall => uninstall(&options),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[dist] {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn render_replaces_every_placeholder_and_only_those() {
        let rendered = render("Exec=@PREFIX@/bin/gnoblin\n# @PREFIX@ again\n", "/opt/gnoblin").unwrap();
        assert_eq!(rendered, "Exec=/opt/gnoblin/bin/gnoblin\n# /opt/gnoblin again\n");
    }

    #[test]
    fn render_refuses_a_prefix_a_desktop_entry_cannot_carry() {
        for prefix in [
            "usr",
            "/usr/",
            "/opt/my gnoblin",
            "/opt/../x",
            "/a\"b",
            "/a%b",
            "/a$b",
            "/a\\b",
        ] {
            assert!(render("@PREFIX@", prefix).is_err(), "accepted {prefix}");
        }
        assert!(render("@PREFIX@", "/").is_ok());
        assert!(render("@PREFIX@", "/usr/local").is_ok());
    }

    #[test]
    fn install_root_keeps_the_prefix_under_the_destdir() {
        assert_eq!(
            install_root(Path::new("/tmp/stage"), "/usr/local"),
            PathBuf::from("/tmp/stage/usr/local")
        );
        assert_eq!(install_root(Path::new(""), "/usr"), PathBuf::from("/usr"));
    }

    #[test]
    fn sha256_matches_the_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_round_trips() {
        let mut entries = Manifest::new();
        entries.insert("bin/gnoblin".to_owned(), sha256_hex(b"one"));
        entries.insert("share/gnoblin/lua/x.lua".to_owned(), sha256_hex(b"two"));
        let text = format_manifest(&entries);
        assert_eq!(parse_manifest(Path::new("m"), &text).unwrap(), entries);
    }

    #[test]
    fn manifest_rejects_lines_an_uninstall_could_not_trust() {
        let good = sha256_hex(b"x");
        let cases = [
            format!("{good} bin/gnoblin\n"),
            format!("{good}  /etc/passwd\n"),
            format!("{good}  ../escape\n"),
            format!("{good}  \n"),
            "ABC  bin/gnoblin\n".to_owned(),
            format!("{good}  bin/a\n{good}  bin/a\n"),
        ];
        for text in cases {
            assert!(parse_manifest(Path::new("m"), &text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn an_install_never_overwrites_a_file_it_did_not_write() {
        assert_eq!(destination(None, None), Destination::Create);
        assert_eq!(destination(None, Some("aa")), Destination::Create);
        assert_eq!(destination(Some("aa"), Some("aa")), Destination::Replace);
        assert_eq!(destination(Some("bb"), Some("aa")), Destination::Modified);
        assert_eq!(destination(Some("aa"), None), Destination::Foreign);
    }

    #[test]
    fn an_uninstall_removes_only_what_is_unchanged() {
        assert_eq!(removal(None, "aa"), Removal::Missing);
        assert_eq!(removal(Some("aa"), "aa"), Removal::Remove);
        assert_eq!(removal(Some("bb"), "aa"), Removal::Modified);
    }

    #[test]
    fn the_install_table_names_each_destination_once() {
        let planned = plan(&repo_root(), Path::new("target/release")).unwrap();
        let mut seen = BTreeSet::new();
        for entry in &planned {
            assert!(seen.insert(entry.to.clone()), "{} listed twice", entry.to);
        }
        for expected in [
            "bin/gnoblin",
            "bin/gnoblinctl",
            "libexec/xdg-desktop-portal-gnoblin",
            "lib/security/pam_gnoblin_keyring.so",
            "share/gnoblin/default-config/init.lua",
            "share/gnoblin/lua/gnoblin/layouts/dwindle.lua",
            "share/xdg-desktop-portal/gnoblin-portals.conf",
        ] {
            assert!(seen.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn every_repository_source_exists_and_every_template_carries_the_placeholder() {
        let planned = plan(&repo_root(), Path::new("target/release")).unwrap();
        for entry in planned.iter().filter(|entry| !entry.source.starts_with("target")) {
            assert!(entry.source.is_file(), "missing source {}", entry.source.display());
            if entry.template {
                let text = fs::read_to_string(&entry.source).unwrap();
                assert!(
                    text.contains(PLACEHOLDER),
                    "{} has no placeholder",
                    entry.source.display()
                );
            }
        }
    }

    #[test]
    fn parse_requires_a_prefix_and_an_action() {
        let target = PathBuf::from("target");
        assert!(parse(&args(&["install"]), target.clone()).is_err());
        assert!(parse(&args(&["remove", "--prefix", "/usr"]), target.clone()).is_err());
        assert!(parse(&args(&["install", "--prefix"]), target.clone()).is_err());
        assert!(parse(&args(&["install", "--prefix", "/usr", "--frob"]), target.clone()).is_err());
        let parsed = parse(
            &args(&["uninstall", "--prefix", "/usr/local", "--destdir", "/tmp/s"]),
            target.clone(),
        )
        .unwrap();
        assert_eq!(parsed.action, DistAction::Uninstall);
        assert_eq!(parsed.profile, DEFAULT_PROFILE);
        assert_eq!(parsed.destdir, PathBuf::from("/tmp/s"));
        assert_eq!(parsed.target_dir, target);
    }
}
