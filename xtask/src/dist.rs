//! `cargo xtask dist`: install the Gnoblin session into a prefix, or remove what an earlier install
//! put there.
//!
//! `install` copies the binaries the build wrote to `target/<profile>/` and the data files under
//! `data/` and the portal crate's `data/` into `<destdir><prefix>`, in the layout the binaries read:
//! `bin/gnoblin`, `libexec/xdg-desktop-portal-gnoblin`, `libexec/gnoblin-xr` when it was built, and so on. Files
//! that name an absolute path (the Wayland session entry and the
//! portal's D-Bus service) carry an `@PREFIX@` placeholder, replaced with the prefix as the file is
//! written.
//!
//! The layout is the one `cargo xtask dist-check` checks, with `--libdir` (`lib`, `lib64` or a multiarch
//! directory) naming where libraries go: the core library `libgnoblin_core_impl.so` is installed as
//! `<libdir>/libgnoblin-core.so.<major>.<minor>.<patch>` with the SONAME link and the linker link, the generated header
//! `include/gnoblin/core.h` and `<libdir>/pkgconfig/gnoblin-core.pc` go with it, `share/gnoblin/js/` holds the stock JavaScript
//! library as data, and the directories shell packages install into (`share/gnoblin/shells`, `<libdir>/gnoblin/shells`)
//! are made. All of these need the core library to have been built, except the JavaScript data.
//!
//! Every installed path is listed in `share/gnoblin/install-manifest` as `<digest>  <path>`, relative to the prefix: a
//! file's sha256, `symlink:<target>` for a link, `dir` for a directory the install made. An install replaces only entries
//! the manifest lists and that still hold what it wrote. Anything else already at a destination is refused before the first
//! write (an existing directory where the install wants one is adopted). `uninstall` removes the listed entries that are
//! unchanged, keeps modified ones and reports them, and removes a listed directory only while it is empty.
//!
//! Exit codes: 0 success, 1 a refused or failed install or uninstall, 2 a usage error.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

/// The token a template file uses where the install prefix goes.
const PLACEHOLDER: &str = "@PREFIX@";
/// Where the install manifest lives, relative to the prefix.
const MANIFEST: &str = "share/gnoblin/install-manifest";
const DEFAULT_PROFILE: &str = "release";
const DEFAULT_LIBDIR: &str = "lib";
/// The token a destination uses where the library directory goes.
const LIBDIR_TOKEN: &str = "@LIBDIR@";
/// The manifest records a symlink as this prefix followed by its target, where a file has its sha256.
const LINK_PREFIX: &str = "symlink:";
/// What cargo names the JavaScript runtime's `cdylib` (`libgnoblin_runtime_js.so`); dist installs it as
/// `libgnoblin-runtime-js.so`, the name the loader looks for.
pub(crate) const RUNTIME_JS_BUILT_NAME: &str = "libgnoblin_runtime_js.so";

/// What cargo names the Python runtime's `cdylib` (`libgnoblin_runtime_python.so`); dist installs it as
/// `libgnoblin-runtime-python.so`, the name the loader looks for a `.py` shell's runtime under.
pub(crate) const RUNTIME_PYTHON_BUILT_NAME: &str = "libgnoblin_runtime_python.so";

/// What cargo names the core's `cdylib` (`libgnoblin_core_impl.so`); dist installs it under the SONAME's name.
const CORE_BUILT_NAME: &str = "libgnoblin_core_impl.so";

/// What cargo names the std library's `cdylib` (`libgnoblin_std_impl.so`); dist installs it under the SONAME's name too.
const STD_BUILT_NAME: &str = "libgnoblin_std_impl.so";

/// What the manifest records for a directory the install made, where a file has its sha256: the directory is ours while it
/// stays empty, and is removed with the install only then (another package may install into it).
const DIR_DIGEST: &str = "dir";

pub const USAGE: &str = "\
usage: cargo xtask dist install --prefix PREFIX [--destdir DIR] [--profile NAME] [--libdir NAME]
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
    /// A program the build may not have written: installed from `target/<profile>/<name>` when it is
    /// there and skipped when it is not. An optional extension has its own workspace, so building the
    /// session alone must still install.
    BuiltIfPresent { name: &'static str, to: &'static str },
    /// A shared library the build wrote: installed as `<libdir>/<stem>.so.<major>.<minor>.<patch>` with the SONAME link
    /// `<stem>.so.<major>` and the linker link `<stem>.so`, when `target/<profile>/<name>` is there.
    BuiltLibrary {
        name: &'static str,
        stem: &'static str,
        major: u32,
        minor: u32,
        patch: u32,
    },
    /// A development file made from the ABI crate (the C header, the pkg-config file), installed when `requires_built`
    /// was built, so a tree never describes a library it does not contain.
    Generated {
        to: &'static str,
        kind: Generated,
        requires_built: &'static str,
    },
    /// Every regular file under a repository directory, installed under `to` with the same relative paths, as plain data.
    Tree { from: &'static str, to: &'static str },
    /// The files `gnoblin-sdkgen` renders for one language, installed under `to` with the paths of the library they belong to. No
    /// generated file is committed, so a tree made from the repository gets these from the generator, as it gets the C header.
    Sdk { to: &'static str, set: SdkSet },
    /// An empty directory the layout needs (a place other packages install into), made when `requires_built` was built.
    Dir {
        to: &'static str,
        requires_built: &'static str,
    },
}

/// The language an [`InstallItem::Sdk`] installs the generated files of.
#[derive(Clone, Copy, Debug)]
enum SdkSet {
    /// `gnoblin-core/generated/*` and `gnoblin-std/generated/*`: the modules the runtime also embeds, installed so a `tsconfig.json`
    /// that maps the packages to the installed library finds every module an editor opens.
    TypeScript,
    /// `gnoblin/_generated/*` and `gnoblin/std.py(i)`: likewise for the Python package, whose stubs import them.
    Python,
}

/// What a generated development file is.
#[derive(Clone, Copy, Debug)]
enum Generated {
    /// `include/gnoblin/core.h`.
    CoreHeader,
    /// `<libdir>/pkgconfig/gnoblin-core.pc`.
    CorePkgConfig,
    /// `include/gnoblin/std.h`.
    StdHeader,
    /// `<libdir>/pkgconfig/gnoblin-std.pc`.
    StdPkgConfig,
    /// `share/gnoblin/types/gnoblin.d.ts`: the globals of a TypeScript shell (`gnoblin`, `console`), as `gnoblinctl types init` writes it.
    TypesGlobals,
    /// `share/gnoblin/types/<package>/index.ts`: the package entry as an editor reads it, the native library's
    /// (`gnoblin_js_types::package_entry`, which `gnoblin-js`'s `editor_library_in` also uses for `types init`).
    TypesIndex(&'static str),
    /// `share/gnoblin/types/tsconfig.json`: the configuration that maps the two packages to the installed library and loads the globals.
    TypesConfig,
}

/// What the session installs. The GeoClue authorisation file (`packaging/geoclue/`)
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
    // The XR extension, from its own workspace (`extensions/gnoblin-xr`). `just dist` builds it into
    // the same target directory; the guardian finds it in `libexec` and starts it only when the user
    // sets `GNOBLIN_XR=1`.
    InstallItem::BuiltIfPresent {
        name: "gnoblin-xr",
        to: "libexec/gnoblin-xr",
    },
    InstallItem::File {
        from: InstallSource::Built("libpam_gnoblin_keyring.so"),
        to: "lib/security/pam_gnoblin_keyring.so",
        mode: FileMode::Program,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("packaging/pam.d/gnoblin-lock"),
        to: "lib/pam.d/gnoblin-lock",
        mode: FileMode::Data,
        template: false,
    },
    InstallItem::File {
        from: InstallSource::Repo("packaging/session/gnoblin.desktop"),
        to: "share/wayland-sessions/gnoblin.desktop",
        mode: FileMode::Data,
        template: true,
    },
    InstallItem::File {
        from: InstallSource::Repo("packaging/session/systemd-user/gnoblin-session.target"),
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
        from: InstallSource::Repo("packaging/gnoblin-portals.conf"),
        to: "share/xdg-desktop-portal/gnoblin-portals.conf",
        mode: FileMode::Data,
        template: false,
    },
    // The core library, when the build wrote it: the versioned file, the SONAME link and the linker link, then the
    // development files that describe it. An install of a build without the core ships none of these. The build names
    // the file after the crate (`libgnoblin_core_impl.so`); the install gives it the SONAME's name.
    InstallItem::BuiltLibrary {
        name: CORE_BUILT_NAME,
        stem: "libgnoblin-core",
        major: gnoblin_core_abi::ABI_MAJOR,
        minor: gnoblin_core_abi::ABI_MINOR,
        patch: gnoblin_core_abi::ABI_PATCH,
    },
    // The JavaScript and TypeScript runtime, when the build wrote it: the loader opens it by the file name
    // `libgnoblin-runtime-js.so` in the core's library directory when a `.js`, `.mjs`, `.ts` or `.mts` shell is selected. It is
    // optional: a build, and a package, without it still runs native shells.
    InstallItem::BuiltIfPresent {
        name: RUNTIME_JS_BUILT_NAME,
        to: "@LIBDIR@/gnoblin/libgnoblin-runtime-js.so",
    },
    // The Python runtime, the same way: optional, opened when a `.py` shell is selected. It links the system libpython, so a package of
    // it depends on the distribution's Python of the version it was built against.
    InstallItem::BuiltIfPresent {
        name: RUNTIME_PYTHON_BUILT_NAME,
        to: "@LIBDIR@/gnoblin/libgnoblin-runtime-python.so",
    },
    InstallItem::Generated {
        to: "include/gnoblin/core.h",
        kind: Generated::CoreHeader,
        requires_built: CORE_BUILT_NAME,
    },
    InstallItem::Generated {
        to: "@LIBDIR@/pkgconfig/gnoblin-core.pc",
        kind: Generated::CorePkgConfig,
        requires_built: CORE_BUILT_NAME,
    },
    // The std library, the same way: the versioned file with its SONAME and linker links, its header (which includes core.h) and its
    // pkg-config file (which requires gnoblin-core). Only when the build wrote it: a tree without it is a tree whose shells use the core
    // alone, which is a supported install (example 17).
    InstallItem::BuiltLibrary {
        name: STD_BUILT_NAME,
        stem: "libgnoblin-std",
        major: gnoblin_std_abi::ABI_MAJOR,
        minor: gnoblin_std_abi::ABI_MINOR,
        patch: gnoblin_std_abi::ABI_PATCH,
    },
    InstallItem::Generated {
        to: "include/gnoblin/std.h",
        kind: Generated::StdHeader,
        requires_built: STD_BUILT_NAME,
    },
    InstallItem::Generated {
        to: "@LIBDIR@/pkgconfig/gnoblin-std.pc",
        kind: Generated::StdPkgConfig,
        requires_built: STD_BUILT_NAME,
    },
    // The stock JavaScript library as installed data (`share/gnoblin/js/<package>/`): the JavaScript runtime holds no copy and reads
    // `gnoblin-core` and `gnoblin-std` from here, so a prefix without the std
    // directory is a prefix whose scripts get the error of example 17 when they import it.
    InstallItem::Tree {
        from: "data/js/gnoblin-core",
        to: "share/gnoblin/js/gnoblin-core",
    },
    InstallItem::Tree {
        from: "data/js/gnoblin-std",
        to: "share/gnoblin/js/gnoblin-std",
    },
    // The generated modules of both packages (`generated/*`), beside the sources that import them. The runtime serves its own copy,
    // embedded when it was built; these are what an editor opens when the installed `tsconfig.json` maps a package to this directory.
    InstallItem::Sdk {
        to: "share/gnoblin/js",
        set: SdkSet::TypeScript,
    },
    // The declarations of a TypeScript shell (section 11: `share/gnoblin/types/**`). The two packages are declared by their own sources, which
    // the generated bindings of `gen-sdks` (`gnoblin-std/generated/*`) are part of, so this directory holds what is not a source of the
    // library: the globals, the two entry modules as the runtime builds them (the stock index plus its host module's exports), and the
    // configuration that maps the package names to those and their submodules to `../js`. A shell's `tsconfig.json` extends it.
    InstallItem::Generated {
        to: "share/gnoblin/types/gnoblin.d.ts",
        kind: Generated::TypesGlobals,
        requires_built: CORE_BUILT_NAME,
    },
    InstallItem::Generated {
        to: "share/gnoblin/types/gnoblin-core/index.ts",
        kind: Generated::TypesIndex("gnoblin-core"),
        requires_built: CORE_BUILT_NAME,
    },
    InstallItem::Generated {
        to: "share/gnoblin/types/gnoblin-std/index.ts",
        kind: Generated::TypesIndex("gnoblin-std"),
        requires_built: CORE_BUILT_NAME,
    },
    InstallItem::Generated {
        to: "share/gnoblin/types/tsconfig.json",
        kind: Generated::TypesConfig,
        requires_built: CORE_BUILT_NAME,
    },
    // The starter is the packaged default shell (section 7.2, tier 7): what `gnoblin` runs on a fresh install, and what `gnoblinctl
    // config init` writes.
    InstallItem::File {
        from: InstallSource::Repo("shells/default/shell.ts"),
        to: "share/gnoblin/default-shell/shell.ts",
        mode: FileMode::Data,
        template: false,
    },
    // The Python package a `.py` shell imports (`from gnoblin import gnoblin`) is embedded in the runtime and installed as no source. What
    // is installed is its declarations, for editors and type checkers: `share/gnoblin/types/python/`, a stub package with `py.typed`.
    // The hand-written stubs are the crate's own (checked against what the runtime reflects), and the generated ones come from the
    // generator below.
    InstallItem::Tree {
        from: "crates/gnoblin-runtime-python/stubs",
        to: "share/gnoblin/types/python",
    },
    InstallItem::Sdk {
        to: "share/gnoblin/types/python",
        set: SdkSet::Python,
    },
    // The two places a shell package installs into: launcher files under `share`, the modules they name under the core's
    // library directory (`module` in a launcher file is relative to it; `$libdir/gnoblin/` is also where the runtime
    // modules go). The install makes them so discovery finds a directory, empty, on a fresh install.
    InstallItem::Dir {
        to: "share/gnoblin/shells",
        requires_built: CORE_BUILT_NAME,
    },
    InstallItem::Dir {
        to: "@LIBDIR@/gnoblin/shells",
        requires_built: CORE_BUILT_NAME,
    },
];

/// What an installed path holds.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PlannedContent {
    /// A file, read when the install runs.
    File(PathBuf),
    /// Bytes generated while planning.
    Bytes(Vec<u8>),
    /// A symlink with this relative target.
    Link(String),
    /// A directory (an empty one, or one a file below it creates).
    Dir,
}

/// One path the install would write, with its content resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Planned {
    to: String,
    content: PlannedContent,
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
    /// A development file could not be generated from the ABI crate.
    Generate(String),
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
            Self::Generate(message) => write!(f, "cannot generate a development file: {message}"),
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
pub(crate) fn install_root(destdir: &Path, prefix: &str) -> PathBuf {
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
        if let Some(target) = digest.strip_prefix(LINK_PREFIX) {
            if target.is_empty() || target.starts_with('/') || target.split('/').any(|part| part == "..") {
                return Err(bad(
                    "a symlink target must be a relative path that stays inside the prefix",
                ));
            }
        } else if digest != DIR_DIGEST && !is_sha256(digest) {
            return Err(bad("the digest is not a lower-case sha256, `dir` or a symlink target"));
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
    /// A directory that is not empty: another package installed into it, so it stays.
    Shared,
}

fn removal(current: Option<&str>, recorded: &str) -> Removal {
    match current {
        None => Removal::Missing,
        Some(now) if now == recorded => Removal::Remove,
        Some(_) => Removal::Modified,
    }
}

/// Removes the recorded entry `rel` when it is unchanged, and prunes the directories that leaves empty.
fn remove_recorded(root: &Path, rel: &str, digest: &str) -> Result<Removal, DistError> {
    let path = root.join(rel);
    let outcome = removal(current_digest(&path)?.as_deref(), digest);
    if outcome != Removal::Remove {
        return Ok(outcome);
    }
    if digest == DIR_DIGEST {
        if fs::remove_dir(&path).is_err() {
            return Ok(Removal::Shared);
        }
    } else {
        fs::remove_file(&path).map_err(io_at(&path))?;
    }
    prune_dirs(root, &path);
    Ok(Removal::Remove)
}

/// The recorded entries in the order they can be removed: files first, then directories, deepest first.
fn removal_order(manifest: &Manifest) -> Vec<(&String, &String)> {
    let files = manifest.iter().filter(|(_, digest)| digest.as_str() != DIR_DIGEST);
    let dirs = manifest
        .iter()
        .rev()
        .filter(|(_, digest)| digest.as_str() == DIR_DIGEST);
    files.chain(dirs).collect()
}

/// What the manifest records for a symlink: its target, not the content it resolves to, so a link repointed elsewhere is a
/// modification and a dangling link is still a link.
fn link_digest(target: &str) -> String {
    format!("{LINK_PREFIX}{target}")
}

/// The digest of what is at `path` (a file's sha256, `symlink:<target>` for a link, `dir` for a directory), or `None` if
/// nothing is there.
fn current_digest(path: &Path) -> Result<Option<String>, DistError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = fs::read_link(path).map_err(io_at(path))?;
            Ok(Some(link_digest(&target.to_string_lossy())))
        }
        Ok(meta) if meta.is_dir() => Ok(Some(DIR_DIGEST.to_owned())),
        Ok(_) => match fs::read(path) {
            Ok(bytes) => Ok(Some(sha256_hex(&bytes))),
            Err(error) => Err(DistError::Io(path.to_owned(), error)),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DistError::Io(path.to_owned(), error)),
    }
}

/// What differs between an installed tree and the manifest its install recorded: one line per listed path that is missing
/// or no longer holds what the install wrote, and the number of paths checked. `dist-check` runs it, so a tree that was
/// edited, or only half copied, is not mistaken for the layout of section 11.
pub(crate) fn manifest_problems(root: &Path) -> Result<(Vec<String>, usize), String> {
    let path = root.join(MANIFEST);
    if !path.is_file() {
        return Ok((
            vec![format!(
                "{} does not exist: the tree was not made by `dist install`",
                path.display()
            )],
            0,
        ));
    }
    let manifest = read_manifest(&path).map_err(|error| error.to_string())?;
    let mut problems = Vec::new();
    for (rel, digest) in &manifest {
        match current_digest(&root.join(rel)).map_err(|error| error.to_string())? {
            None => problems.push(format!("{rel} is listed in the manifest and missing")),
            Some(now) if now != *digest => problems.push(format!("{rel} no longer holds what the install wrote")),
            Some(_) => {}
        }
    }
    Ok((problems, manifest.len()))
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

/// Makes a symlink at `dest` pointing at `target`, replacing a link already there, through a sibling temporary link so a
/// reader never finds the name missing.
pub(crate) fn write_link(dest: &Path, target: &str) -> Result<(), DistError> {
    let parent = dest
        .parent()
        .ok_or_else(|| DistError::Unsupported(dest.to_owned(), "has no parent directory"))?;
    fs::create_dir_all(parent).map_err(io_at(parent))?;
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".gnoblin-dist");
    let temp = parent.join(name);
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(DistError::Io(temp, error)),
    }
    symlink(target, &temp).map_err(io_at(&temp))?;
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

/// Expands the install table into one entry per file or link. Built files are not read here (they are read when installed);
/// generated files are made now. `libdir` replaces `@LIBDIR@` in a destination.
fn plan(repo: &Path, built: &Path, libdir: &str) -> Result<Vec<Planned>, DistError> {
    let mut planned = Vec::new();
    for item in ITEMS {
        match *item {
            InstallItem::File {
                from,
                to,
                mode,
                template,
            } => planned.push(Planned {
                to: expand_libdir(to, libdir),
                content: PlannedContent::File(origin_path(repo, built, from)),
                mode,
                template,
            }),
            InstallItem::BuiltIfPresent { name, to } => {
                let source = built.join(name);
                if source.is_file() {
                    planned.push(Planned {
                        to: expand_libdir(to, libdir),
                        content: PlannedContent::File(source),
                        mode: FileMode::Program,
                        template: false,
                    });
                }
            }
            InstallItem::BuiltLibrary {
                name,
                stem,
                major,
                minor,
                patch,
            } => {
                let source = built.join(name);
                if source.is_file() {
                    let versioned = format!("{stem}.so.{major}.{minor}.{patch}");
                    let soname = format!("{stem}.so.{major}");
                    let linker = format!("{stem}.so");
                    let dir = |file: &str| format!("{libdir}/{file}");
                    planned.push(Planned {
                        to: dir(&versioned),
                        content: PlannedContent::File(source),
                        mode: FileMode::Program,
                        template: false,
                    });
                    // The SONAME link is what the dynamic loader resolves a NEEDED entry through; the linker name is what
                    // `-lgnoblin-core` finds, and points at the SONAME link so a build records the SONAME, not the file.
                    planned.push(Planned {
                        to: dir(&soname),
                        content: PlannedContent::Link(versioned),
                        mode: FileMode::Program,
                        template: false,
                    });
                    planned.push(Planned {
                        to: dir(&linker),
                        content: PlannedContent::Link(soname),
                        mode: FileMode::Program,
                        template: false,
                    });
                }
            }
            InstallItem::Generated {
                to,
                kind,
                requires_built,
            } => {
                if built.join(requires_built).is_file() {
                    let (bytes, template) = match kind {
                        Generated::CoreHeader => (
                            crate::abi_c::render_header(&gnoblin_core_abi::reflect::reflection())
                                .map_err(DistError::Generate)?
                                .into_bytes(),
                            false,
                        ),
                        Generated::CorePkgConfig => (core_pkg_config(libdir).into_bytes(), true),
                        Generated::StdHeader => (
                            crate::abi_c::render_header_with(&gnoblin_std_abi::reflection(), &crate::abi_c::STD_HEADER)
                                .map_err(DistError::Generate)?
                                .into_bytes(),
                            false,
                        ),
                        Generated::StdPkgConfig => (std_pkg_config(libdir).into_bytes(), true),
                        Generated::TypesGlobals => (gnoblin_js_types::render_dts().into_bytes(), false),
                        Generated::TypesIndex(package) => {
                            let entry = gnoblin_js_types::package_entry(package).ok_or_else(|| {
                                DistError::Generate(format!("`{package}` is not a package of the library"))
                            })?;
                            (entry.as_bytes().to_vec(), false)
                        }
                        Generated::TypesConfig => (gnoblin_js_types::render_installed_tsconfig().into_bytes(), false),
                    };
                    planned.push(Planned {
                        to: expand_libdir(to, libdir),
                        content: PlannedContent::Bytes(bytes),
                        mode: FileMode::Data,
                        template,
                    });
                }
            }
            InstallItem::Tree { from, to } => {
                let source = repo.join(from);
                let mut files = Vec::new();
                collect_files(&source, &mut files)?;
                for file in files {
                    let relative = file.strip_prefix(&source).map_err(|_| {
                        DistError::Unsupported(file.clone(), "is not under the directory the install table names")
                    })?;
                    let relative = relative
                        .to_str()
                        .ok_or_else(|| DistError::Unsupported(file.clone(), "has a name that is not UTF-8"))?;
                    planned.push(Planned {
                        to: format!("{}/{relative}", expand_libdir(to, libdir)),
                        content: PlannedContent::File(file),
                        mode: FileMode::Data,
                        template: false,
                    });
                }
            }
            InstallItem::Sdk { to, set } => {
                // `object-types` is the declarations of the object classes: `.d.ts` under `js/` and `.pyi` under `python/`. The classes
                // themselves (`object-js`, `object-py`) are embedded in the runtimes and installed nowhere.
                let (targets, prefix): (&[&str], &str) = match set {
                    SdkSet::TypeScript => (&["typescript", "object-types"], "js/"),
                    SdkSet::Python => (&["python", "object-types"], "python/"),
                };
                let spec = gnoblin_sdkgen::SdkSpec::load(repo).map_err(DistError::Generate)?;
                for target in targets {
                    let rendered = gnoblin_sdkgen::render(&spec, Some(target)).map_err(DistError::Generate)?;
                    let files = gnoblin_sdkgen::files_of(&rendered);
                    // A Python module with a stub is read from the stub: its source is embedded in the runtime and is not installed.
                    let stubbed: BTreeSet<String> = files
                        .iter()
                        .filter_map(|(path, _)| path.strip_suffix(".pyi").map(str::to_owned))
                        .collect();
                    for (path, text) in files {
                        if path.strip_suffix(".py").is_some_and(|module| stubbed.contains(module)) {
                            continue;
                        }
                        let Some(relative) = path.strip_prefix(prefix) else {
                            continue;
                        };
                        planned.push(Planned {
                            to: format!("{}/{relative}", expand_libdir(to, libdir)),
                            content: PlannedContent::Bytes(text.into_bytes()),
                            mode: FileMode::Data,
                            template: false,
                        });
                    }
                }
            }
            InstallItem::Dir { to, requires_built } => {
                if built.join(requires_built).is_file() {
                    planned.push(Planned {
                        to: expand_libdir(to, libdir),
                        content: PlannedContent::Dir,
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

/// The regular files under `dir`, in name order. A symlink or anything else is refused: a tree is plain data.
fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), DistError> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(io_at(dir))?
        .map(|entry| entry.map(|entry| entry.path()).map_err(io_at(dir)))
        .collect::<Result<_, _>>()?;
    entries.sort();
    for path in entries {
        let kind = fs::symlink_metadata(&path).map_err(io_at(&path))?.file_type();
        if kind.is_dir() {
            // Python's bytecode cache, which a run of a source tree leaves beside the package, is not part of it.
            if path.file_name().is_some_and(|name| name == "__pycache__") {
                continue;
            }
            collect_files(&path, files)?;
        } else if kind.is_file() {
            files.push(path);
        } else {
            return Err(DistError::Unsupported(
                path,
                "is not a regular file or a directory, and a data tree holds only those",
            ));
        }
    }
    Ok(())
}

/// Writes what an install gives an editor or a type checker, with nothing built, under `dest`: the stock TypeScript library and its generated declarations
/// (`share/gnoblin/js`), the globals and the `tsconfig.json` that maps the package names to them (`share/gnoblin/types`), and the Python stub package
/// (`share/gnoblin/types/python`). It runs the install's own plan, so the files are the ones `dist install` would write, byte for byte. `docs-examples`
/// type-checks the examples of the documentation against this tree.
pub(crate) fn write_editor_tree(repo: &Path, dest: &Path) -> Result<(), DistError> {
    // The plan writes a generated file only when the library it describes was built, so the tree is planned against a build that names both.
    let built = dest.join(".planned-build");
    fs::create_dir_all(&built).map_err(io_at(&built))?;
    for name in [CORE_BUILT_NAME, STD_BUILT_NAME] {
        fs::write(built.join(name), b"").map_err(io_at(&built))?;
    }
    let planned = plan(repo, &built, DEFAULT_LIBDIR)?;
    fs::remove_dir_all(&built).map_err(io_at(&built))?;
    for entry in planned {
        if !(entry.to.starts_with("share/gnoblin/js/") || entry.to.starts_with("share/gnoblin/types/")) {
            continue;
        }
        let to = dest.join(&entry.to);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(io_at(parent))?;
        }
        match entry.content {
            PlannedContent::File(from) => {
                fs::copy(&from, &to).map_err(io_at(&from))?;
            }
            PlannedContent::Bytes(bytes) => fs::write(&to, bytes).map_err(io_at(&to))?,
            PlannedContent::Link(_) | PlannedContent::Dir => {}
        }
    }
    Ok(())
}

fn expand_libdir(to: &str, libdir: &str) -> String {
    to.replace(LIBDIR_TOKEN, libdir)
}

/// `gnoblin-core.pc`, for the development package. `@PREFIX@` is replaced with the install prefix when it is written, so
/// a relocated or staged install still names its own location; a consumer can override it with
/// `pkg-config --define-variable=prefix=...`.
fn core_pkg_config(libdir: &str) -> String {
    pkg_config(
        libdir,
        "gnoblin-core",
        "The Gnoblin compositor core library: the shell ABI",
        (
            gnoblin_core_abi::ABI_MAJOR,
            gnoblin_core_abi::ABI_MINOR,
            gnoblin_core_abi::ABI_PATCH,
        ),
        None,
    )
}

/// `gnoblin-std.pc`: the std library builds on the core's, so a consumer that asks for it gets both.
fn std_pkg_config(libdir: &str) -> String {
    pkg_config(
        libdir,
        "gnoblin-std",
        "The Gnoblin desktop library above the core: clients, collections, workspaces and layouts",
        (
            gnoblin_std_abi::ABI_MAJOR,
            gnoblin_std_abi::ABI_MINOR,
            gnoblin_std_abi::ABI_PATCH,
        ),
        Some("gnoblin-core"),
    )
}

/// A pkg-config file for one of the libraries, with `requires` as its dependency on the other.
fn pkg_config(libdir: &str, name: &str, description: &str, version: (u32, u32, u32), requires: Option<&str>) -> String {
    let requires = requires.map_or_else(String::new, |module| format!("Requires: {module}\n"));
    format!(
        "prefix={PLACEHOLDER}\n\
         libdir=${{prefix}}/{libdir}\n\
         includedir=${{prefix}}/include\n\
         \n\
         Name: {name}\n\
         Description: {description}\n\
         Version: {}.{}.{}\n\
         {requires}\
         Libs: -L${{libdir}} -l{name}\n\
         Cflags: -I${{includedir}}\n",
        version.0, version.1, version.2,
    )
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
    /// Where libraries go under the prefix: `lib`, `lib64`, or a multiarch directory such as `lib/x86_64-linux-gnu`.
    libdir: String,
}

/// The library directory must be a plain relative path: it is joined to the prefix and written into the `.pc` file.
fn validate_libdir(libdir: &str) -> Result<(), String> {
    let reason = if libdir.is_empty() {
        "is empty"
    } else if libdir.starts_with('/') {
        "must be relative to the prefix"
    } else if libdir.ends_with('/') {
        "must not end with a slash"
    } else if libdir.split('/').any(|part| part.is_empty() || part == "..") {
        "must not contain empty or `..` components"
    } else if libdir
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\\' | '%' | '$' | '@'))
    {
        "must not contain whitespace, control characters, quotes, backslashes, `%`, `$` or `@`"
    } else {
        return Ok(());
    };
    Err(format!("libdir `{libdir}` {reason}"))
}

/// Parses `install|uninstall --prefix P [--destdir D] [--profile NAME] [--libdir NAME]`. The target directory comes
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
    let mut libdir = DEFAULT_LIBDIR.to_owned();
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| format!("`{flag}` needs a value"));
        match flag.as_str() {
            "--prefix" => prefix = Some(value()?),
            "--destdir" => destdir = PathBuf::from(value()?),
            "--profile" => profile = value()?,
            "--libdir" => libdir = value()?,
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    let prefix = prefix.ok_or_else(|| "`--prefix` is required".to_owned())?;
    validate_libdir(&libdir)?;
    Ok(DistOptions {
        action,
        prefix,
        destdir,
        profile,
        target_dir,
        libdir,
    })
}

/// What an entry writes, with its template already rendered.
enum Payload {
    Bytes(Vec<u8>),
    Link(String),
    Dir,
}

/// Renders a template entry's text with the prefix and digests what will be written.
fn prepare(raw: Vec<u8>, origin: &Path, entry: &Planned, prefix: &str) -> Result<(Payload, String), DistError> {
    let contents = if entry.template {
        let text = String::from_utf8(raw)
            .map_err(|_| DistError::Unsupported(origin.to_owned(), "is a template but not UTF-8 text"))?;
        render(&text, prefix)?.into_bytes()
    } else {
        raw
    };
    let digest = sha256_hex(&contents);
    Ok((Payload::Bytes(contents), digest))
}

fn install(repo: &Path, options: &DistOptions) -> Result<(), DistError> {
    validate_prefix(&options.prefix)?;
    let root = install_root(&options.destdir, &options.prefix);
    let manifest_path = root.join(MANIFEST);
    let recorded = read_manifest(&manifest_path)?;
    let planned = plan(repo, &options.target_dir.join(&options.profile), &options.libdir)?;

    // Preflight: every source is read and every destination checked before the first write, so a
    // refusal leaves the tree as it was.
    let mut staged = Vec::with_capacity(planned.len());
    for entry in &planned {
        let (payload, digest) = match &entry.content {
            PlannedContent::Link(target) => (Payload::Link(target.clone()), link_digest(target)),
            PlannedContent::File(source) => {
                let raw = fs::read(source).map_err(io_at(source))?;
                prepare(raw, source, entry, &options.prefix)?
            }
            PlannedContent::Bytes(bytes) => prepare(bytes.clone(), Path::new(&entry.to), entry, &options.prefix)?,
            PlannedContent::Dir => (Payload::Dir, DIR_DIGEST.to_owned()),
        };
        let dest = root.join(&entry.to);
        let current = current_digest(&dest)?;
        if matches!(payload, Payload::Dir) {
            // A directory another package made is adopted; a file or link in its place is not ours to replace.
            if current.as_deref().is_some_and(|now| now != DIR_DIGEST) {
                return Err(DistError::Foreign(dest));
            }
        } else {
            match destination(current.as_deref(), recorded.get(&entry.to).map(String::as_str)) {
                Destination::Create | Destination::Replace => {}
                Destination::Foreign => return Err(DistError::Foreign(dest)),
                Destination::Modified => return Err(DistError::Modified(dest)),
            }
        }
        staged.push((entry, dest, payload, digest));
    }

    // The manifest starts as the old record, and each file written is added as it lands. If a write
    // fails, the manifest still describes what is on disk, so `uninstall` can remove it.
    let mut manifest = recorded.clone();
    let mut failure = None;
    for (entry, dest, payload, digest) in &staged {
        let written = match payload {
            Payload::Bytes(contents) => write_file(dest, contents, entry.mode),
            Payload::Link(target) => write_link(dest, target),
            Payload::Dir => fs::create_dir_all(dest).map_err(io_at(dest)),
        };
        if let Err(error) = written {
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
        for (rel, digest) in removal_order(&recorded) {
            if wanted.contains(rel.as_str()) {
                continue;
            }
            match remove_recorded(&root, rel, digest)? {
                Removal::Remove => {
                    manifest.remove(rel);
                    retired.push(rel.clone());
                }
                Removal::Modified => kept.push(rel.clone()),
                Removal::Missing | Removal::Shared => {
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
    for (rel, digest) in removal_order(&recorded) {
        match remove_recorded(&root, rel, digest)? {
            Removal::Remove => removed += 1,
            Removal::Modified => {
                println!("[dist] kept {rel}: changed since the install");
                kept.insert(rel.clone(), digest.clone());
            }
            Removal::Missing => println!("[dist] missing {rel}: already removed"),
            Removal::Shared => println!("[dist] left {rel}: not empty"),
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
    use crate::test_fixtures::{good_core, launcher, scratch};
    use gnoblin_core_abi as abi;

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
        entries.insert("share/gnoblin/x.txt".to_owned(), sha256_hex(b"two"));
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
        let planned = plan(&repo_root(), Path::new("target/release"), "lib").unwrap();
        let mut seen = BTreeSet::new();
        for entry in &planned {
            assert!(seen.insert(entry.to.clone()), "{} listed twice", entry.to);
        }
        for expected in [
            "bin/gnoblin",
            "bin/gnoblinctl",
            "libexec/xdg-desktop-portal-gnoblin",
            "lib/security/pam_gnoblin_keyring.so",
            "lib/pam.d/gnoblin-lock",
            "share/xdg-desktop-portal/gnoblin-portals.conf",
        ] {
            assert!(seen.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn the_xr_extension_is_installed_only_when_it_was_built() {
        let built = std::env::temp_dir().join(format!("gnoblin-dist-xr-{}", std::process::id()));
        fs::create_dir_all(&built).unwrap();
        let without = plan(&repo_root(), &built, "lib").unwrap();
        assert!(
            without.iter().all(|entry| entry.to != "libexec/gnoblin-xr"),
            "a build without the extension installs without it"
        );

        fs::write(built.join("gnoblin-xr"), b"binary").unwrap();
        let with = plan(&repo_root(), &built, "lib").unwrap();
        let entry = with
            .iter()
            .find(|entry| entry.to == "libexec/gnoblin-xr")
            .expect("the built extension is installed");
        assert_eq!(entry.content, PlannedContent::File(built.join("gnoblin-xr")));
        assert_eq!(entry.mode, FileMode::Program);
        assert!(!entry.template);
        fs::remove_dir_all(&built).unwrap();
    }

    #[test]
    fn every_repository_source_exists_and_every_template_carries_the_placeholder() {
        let planned = plan(&repo_root(), Path::new("target/release"), "lib").unwrap();
        for entry in &planned {
            let PlannedContent::File(source) = &entry.content else {
                continue;
            };
            if source.starts_with("target") {
                continue;
            }
            assert!(source.is_file(), "missing source {}", source.display());
            if entry.template {
                let text = fs::read_to_string(source).unwrap();
                assert!(text.contains(PLACEHOLDER), "{} has no placeholder", source.display());
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
        assert_eq!(parsed.libdir, DEFAULT_LIBDIR);
        assert_eq!(parsed.destdir, PathBuf::from("/tmp/s"));
        assert_eq!(parsed.target_dir, target);
    }

    #[test]
    fn the_libdir_is_a_plain_relative_directory_name() {
        let target = PathBuf::from("target");
        let parsed = parse(
            &args(&["install", "--prefix", "/usr", "--libdir", "lib/x86_64-linux-gnu"]),
            target.clone(),
        )
        .unwrap();
        assert_eq!(parsed.libdir, "lib/x86_64-linux-gnu");
        assert_eq!(
            parse(
                &args(&["install", "--prefix", "/usr", "--libdir", "lib64"]),
                target.clone()
            )
            .unwrap()
            .libdir,
            "lib64"
        );
        for bad in ["", "/usr/lib", "../lib", "lib/../x", "lib/", "a b", "lib\\x"] {
            assert!(
                parse(&args(&["install", "--prefix", "/usr", "--libdir", bad]), target.clone()).is_err(),
                "accepted libdir `{bad}`"
            );
        }
    }

    /// A build directory with the programs the install table needs, as plain bytes, plus the core library when `core` is
    /// given (a path to a file to copy as `libgnoblin_core_impl.so`).
    fn fake_build(dir: &Path, launcher: Option<&Path>, core: Option<&Path>) -> PathBuf {
        let built = dir.join("target/release");
        fs::create_dir_all(&built).unwrap();
        for name in ["gnoblinctl", "xdg-desktop-portal-gnoblin", "libpam_gnoblin_keyring.so"] {
            fs::write(built.join(name), format!("fake {name}")).unwrap();
        }
        match launcher {
            Some(launcher) => {
                fs::copy(launcher, built.join("gnoblin")).unwrap();
            }
            None => fs::write(built.join("gnoblin"), b"fake launcher").unwrap(),
        }
        if let Some(core) = core {
            fs::copy(core, built.join(CORE_BUILT_NAME)).unwrap();
        }
        dir.join("target")
    }

    fn options(destdir: &Path, target: &Path, libdir: &str) -> DistOptions {
        DistOptions {
            action: DistAction::Install,
            prefix: "/usr".to_owned(),
            destdir: destdir.to_owned(),
            profile: DEFAULT_PROFILE.to_owned(),
            target_dir: target.to_owned(),
            libdir: libdir.to_owned(),
        }
    }

    #[test]
    fn the_javascript_runtime_is_installed_under_the_name_the_loader_looks_for_only_when_it_was_built() {
        let dir = scratch("dist-plan-runtime");
        let built = fake_build(&dir, None, None);
        let without = plan(&repo_root(), &built.join("release"), "lib64").unwrap();
        assert!(without.iter().all(|entry| !entry.to.contains("runtime-js")));
        fs::write(built.join("release").join(RUNTIME_JS_BUILT_NAME), b"fake runtime").unwrap();
        let with = plan(&repo_root(), &built.join("release"), "lib64").unwrap();
        let runtime = with
            .iter()
            .find(|entry| entry.to == "lib64/gnoblin/libgnoblin-runtime-js.so")
            .expect("the runtime is planned");
        assert_eq!(runtime.mode, FileMode::Program);
        assert!(matches!(runtime.content, PlannedContent::File(_)));
    }

    #[test]
    fn the_editor_tree_is_what_an_install_writes_for_a_type_checker_with_nothing_built() {
        let dir = scratch("dist-editor-tree");
        write_editor_tree(&repo_root(), &dir).unwrap();
        for file in [
            "share/gnoblin/types/tsconfig.json",
            "share/gnoblin/types/gnoblin.d.ts",
            "share/gnoblin/types/gnoblin-core/index.ts",
            "share/gnoblin/js/gnoblin-core/generated/objects.d.ts",
            "share/gnoblin/types/python/gnoblin/py.typed",
            "share/gnoblin/types/python/gnoblin/objects/model.pyi",
        ] {
            assert!(dir.join(file).is_file(), "{file} is written");
        }
        // Nothing that needs a build: no program, no library, no header, and the scratch build the plan was made against is gone.
        assert!(!dir.join("bin").exists() && !dir.join("include").exists() && !dir.join(".planned-build").exists());
    }

    #[test]
    fn the_python_runtime_and_its_package_are_installed_where_the_loader_and_the_runtime_look() {
        let dir = scratch("dist-plan-runtime-python");
        let built = fake_build(&dir, None, None);
        let without = plan(&repo_root(), &built.join("release"), "lib64").unwrap();
        assert!(without.iter().all(|entry| !entry.to.contains("runtime-python")));
        fs::write(built.join("release").join(RUNTIME_PYTHON_BUILT_NAME), b"fake runtime").unwrap();
        let with = plan(&repo_root(), &built.join("release"), "lib64").unwrap();
        let runtime = with
            .iter()
            .find(|entry| entry.to == "lib64/gnoblin/libgnoblin-runtime-python.so")
            .expect("the runtime is planned");
        assert_eq!(runtime.mode, FileMode::Program);
        // Nothing of the package is installed as source: the runtime serves it from itself. The declarations an editor reads are the
        // hand-written stubs of the crate, copied, and the generated ones, which come from the generator.
        for wanted in [
            "share/gnoblin/types/python/gnoblin/__init__.pyi",
            "share/gnoblin/types/python/gnoblin/_base.pyi",
            "share/gnoblin/types/python/gnoblin/api.pyi",
            "share/gnoblin/types/python/gnoblin/errors.pyi",
            "share/gnoblin/types/python/gnoblin/events.pyi",
            "share/gnoblin/types/python/gnoblin/py.typed",
            "share/gnoblin/types/python/_gnoblin_core.pyi",
        ] {
            let entry = with
                .iter()
                .find(|entry| entry.to == wanted)
                .unwrap_or_else(|| panic!("{wanted} is installed"));
            assert!(matches!(entry.content, PlannedContent::File(_)), "{wanted} is copied");
        }
        for wanted in [
            "share/gnoblin/types/python/gnoblin/_generated/classes.pyi",
            "share/gnoblin/types/python/gnoblin/_generated/verbs.py",
            "share/gnoblin/types/python/gnoblin/std.pyi",
            "share/gnoblin/types/python/gnoblin/objects/std.pyi",
            "share/gnoblin/types/python/gnoblin/objects/__init__.pyi",
        ] {
            let entry = with
                .iter()
                .find(|entry| entry.to == wanted)
                .unwrap_or_else(|| panic!("{wanted} is installed"));
            assert!(
                matches!(entry.content, PlannedContent::Bytes(_)),
                "{wanted} is generated, not copied"
            );
        }
        // A module with a stub is installed as the stub only, and none of the package is installed as source.
        for absent in [
            "share/gnoblin/types/python/gnoblin/std.py",
            "share/gnoblin/types/python/gnoblin/_generated/classes.py",
            "share/gnoblin/types/python/gnoblin/__init__.py",
            "share/gnoblin/types/python/gnoblin/_loader.py",
        ] {
            assert!(with.iter().all(|entry| entry.to != absent), "{absent} is not installed");
        }
        assert!(
            with.iter().all(|entry| !entry.to.starts_with("share/gnoblin/python/")),
            "the package is embedded: no directory of it is installed"
        );
    }

    #[test]
    fn the_core_library_and_its_development_files_are_planned_only_when_it_was_built() {
        let dir = scratch("dist-plan-core");
        let without = plan(&repo_root(), &fake_build(&dir, None, None).join("release"), "lib64").unwrap();
        assert!(
            without
                .iter()
                .all(|entry| !entry.to.contains("libgnoblin-core") && !entry.to.starts_with("include/")),
            "an install without the core ships no core files"
        );

        let core_dir = dir.join("fixture-core-src");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let with = plan(
            &repo_root(),
            &fake_build(&dir, None, Some(&core)).join("release"),
            "lib64",
        )
        .unwrap();
        let version = format!("{}.{}.{}", abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH);
        let find = |to: &str| {
            with.iter()
                .find(|entry| entry.to == to)
                .unwrap_or_else(|| panic!("{to} is not planned"))
        };
        let file = find(&format!("lib64/libgnoblin-core.so.{version}"));
        assert!(matches!(file.content, PlannedContent::File(_)));
        assert_eq!(file.mode, FileMode::Program);
        assert_eq!(
            find("lib64/libgnoblin-core.so.1").content,
            PlannedContent::Link(format!("libgnoblin-core.so.{version}"))
        );
        assert_eq!(
            find("lib64/libgnoblin-core.so").content,
            PlannedContent::Link("libgnoblin-core.so.1".to_owned())
        );
        assert!(matches!(
            find("include/gnoblin/core.h").content,
            PlannedContent::Bytes(_)
        ));
        let pc = find("lib64/pkgconfig/gnoblin-core.pc");
        assert!(pc.template, "the .pc file carries the prefix");
        let PlannedContent::Bytes(bytes) = &pc.content else {
            panic!("the .pc file is generated")
        };
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("libdir=${prefix}/lib64"), "{text}");
        assert!(text.contains(&format!("Version: {version}")), "{text}");
        assert!(text.contains("prefix=@PREFIX@"), "{text}");
    }

    #[test]
    fn the_manifest_records_a_symlink_by_its_target_and_refuses_one_that_escapes() {
        let good = sha256_hex(b"x");
        let text = format!("{good}  lib/a.so.1.0.0\nsymlink:a.so.1.0.0  lib/a.so.1\n");
        let manifest = parse_manifest(Path::new("m"), &text).unwrap();
        assert_eq!(manifest["lib/a.so.1"], "symlink:a.so.1.0.0");
        assert_eq!(
            parse_manifest(Path::new("m"), &format_manifest(&manifest)).unwrap(),
            manifest
        );
        for bad in [
            "symlink:  lib/a",
            "symlink:/etc/passwd  lib/a",
            "symlink:../x  lib/a",
            "symlink:a/../b  lib/a",
        ] {
            assert!(
                parse_manifest(Path::new("m"), &format!("{bad}\n")).is_err(),
                "accepted {bad:?}"
            );
        }
    }

    #[test]
    fn a_built_core_installs_with_its_links_and_the_installed_tree_passes_dist_check() {
        let dir = scratch("dist-install-core");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let launcher = launcher(&dir, "launcher", &core_dir, "$ORIGIN/../lib64");
        let target = fake_build(&dir, Some(&launcher), Some(&core));
        let destdir = dir.join("dest");

        install(&repo_root(), &options(&destdir, &target, "lib64")).unwrap();

        let root = install_root(&destdir, "/usr");
        let version = format!("{}.{}.{}", abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH);
        assert!(root.join(format!("lib64/libgnoblin-core.so.{version}")).is_file());
        assert_eq!(
            fs::read_link(root.join("lib64/libgnoblin-core.so.1")).unwrap(),
            Path::new(&format!("libgnoblin-core.so.{version}"))
        );
        assert_eq!(
            fs::read_link(root.join("lib64/libgnoblin-core.so")).unwrap(),
            Path::new("libgnoblin-core.so.1")
        );
        let pc = fs::read_to_string(root.join("lib64/pkgconfig/gnoblin-core.pc")).unwrap();
        assert!(pc.contains("prefix=/usr\n"), "the prefix is substituted: {pc}");
        let manifest = read_manifest(&root.join(MANIFEST)).unwrap();
        assert_eq!(manifest["lib64/libgnoblin-core.so"], "symlink:libgnoblin-core.so.1");
        assert!(manifest.contains_key("include/gnoblin/core.h"));

        // The whole point of the install: what it wrote passes the dynamic-linking checks, including a C program built
        // through pkg-config and run against the installed library.
        let failures = crate::dist_check::check_installed(&destdir, "/usr", "lib64").unwrap();
        assert_eq!(
            failures, 0,
            "dist-check found problems in the installed tree (printed above)"
        );
    }

    #[test]
    fn a_reinstall_replaces_links_and_a_foreign_link_is_refused_before_anything_is_written() {
        let dir = scratch("dist-reinstall-core");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let target = fake_build(&dir, None, Some(&core));
        let destdir = dir.join("dest");
        install(&repo_root(), &options(&destdir, &target, "lib")).unwrap();
        install(&repo_root(), &options(&destdir, &target, "lib")).unwrap();

        let root = install_root(&destdir, "/usr");
        let link = root.join("lib/libgnoblin-core.so");
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("libgnoblin-core.so.1"));

        // Someone repointed the linker name: the install notices it is no longer what it wrote.
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("elsewhere.so", &link).unwrap();
        let error = install(&repo_root(), &options(&destdir, &target, "lib")).unwrap_err();
        assert!(matches!(error, DistError::Modified(_)), "{error}");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            Path::new("elsewhere.so"),
            "the refused install changed nothing"
        );

        // A link where the manifest knows nothing is foreign.
        let fresh = dir.join("fresh");
        fs::create_dir_all(fresh.join("usr/lib")).unwrap();
        std::os::unix::fs::symlink("elsewhere.so", fresh.join("usr/lib/libgnoblin-core.so")).unwrap();
        let error = install(&repo_root(), &options(&fresh, &target, "lib")).unwrap_err();
        assert!(matches!(error, DistError::Foreign(_)), "{error}");
    }

    #[test]
    fn uninstall_removes_the_links_with_the_files_and_prunes_the_directories() {
        let dir = scratch("dist-uninstall-core");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let target = fake_build(&dir, None, Some(&core));
        let destdir = dir.join("dest");
        install(&repo_root(), &options(&destdir, &target, "lib")).unwrap();
        let root = install_root(&destdir, "/usr");
        assert!(fs::symlink_metadata(root.join("lib/libgnoblin-core.so")).is_ok());

        let mut remove = options(&destdir, &target, "lib");
        remove.action = DistAction::Uninstall;
        uninstall(&remove).unwrap();
        assert!(
            fs::symlink_metadata(root.join("lib/libgnoblin-core.so")).is_err(),
            "the link is gone"
        );
        assert!(!root.join("lib/pkgconfig").exists(), "the empty directory is pruned");
        assert!(!root.join(MANIFEST).exists());
    }

    #[test]
    fn the_stock_javascript_library_and_the_shell_directories_are_part_of_a_core_install() {
        let dir = scratch("dist-plan-layout");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let without = plan(&repo_root(), &fake_build(&dir, None, None).join("release"), "lib64").unwrap();
        assert!(
            without.iter().all(|entry| entry.content != PlannedContent::Dir),
            "a build without the core makes no shell directory"
        );
        let with = plan(
            &repo_root(),
            &fake_build(&dir, None, Some(&core)).join("release"),
            "lib64",
        )
        .unwrap();
        for directory in ["share/gnoblin/shells", "lib64/gnoblin/shells"] {
            assert!(
                with.iter()
                    .any(|entry| entry.to == directory && entry.content == PlannedContent::Dir),
                "{directory} is not planned"
            );
        }
        // Every file of both stock packages is installed under the same relative path, as plain data.
        for package in ["gnoblin-core", "gnoblin-std"] {
            let source = repo_root().join("data/js").join(package);
            let mut files = Vec::new();
            collect_files(&source, &mut files).unwrap();
            assert!(!files.is_empty(), "{package} has no files");
            for file in files {
                let relative = file.strip_prefix(&source).unwrap().to_str().unwrap().to_owned();
                let entry = with
                    .iter()
                    .find(|entry| entry.to == format!("share/gnoblin/js/{package}/{relative}"))
                    .unwrap_or_else(|| panic!("{package}/{relative} is not planned"));
                assert_eq!(entry.content, PlannedContent::File(file));
                assert_eq!(entry.mode, FileMode::Data);
                assert!(!entry.template);
            }
        }
        // The generated modules come from the generator, not from a committed file, and land beside the sources that import them.
        for generated in [
            "share/gnoblin/js/gnoblin-core/generated/abi.ts",
            "share/gnoblin/js/gnoblin-core/generated/schema.ts",
            "share/gnoblin/js/gnoblin-std/generated/classes.ts",
            "share/gnoblin/js/gnoblin-std/generated/records.ts",
        ] {
            let entry = with
                .iter()
                .find(|entry| entry.to == generated)
                .unwrap_or_else(|| panic!("{generated} is not planned"));
            assert!(matches!(entry.content, PlannedContent::Bytes(_)), "{generated}");
            assert_eq!(entry.mode, FileMode::Data);
        }
        // The starter is installed as the packaged default shell, which discovery runs on a fresh install (tier 7).
        let shell = with
            .iter()
            .find(|entry| entry.to == "share/gnoblin/default-shell/shell.ts")
            .expect("the packaged default shell is planned");
        assert_eq!(
            shell.content,
            PlannedContent::File(repo_root().join("shells/default/shell.ts"))
        );
        assert_eq!(shell.mode, FileMode::Data);
    }

    #[test]
    fn a_data_tree_refuses_a_symlink() {
        let dir = scratch("dist-tree-link");
        fs::write(dir.join("real"), b"x").unwrap();
        std::os::unix::fs::symlink("real", dir.join("alias")).unwrap();
        let mut files = Vec::new();
        let error = collect_files(&dir, &mut files).unwrap_err();
        assert!(matches!(error, DistError::Unsupported(..)), "{error}");
    }

    #[test]
    fn the_manifest_records_a_directory_and_the_removal_order_puts_directories_last() {
        let good = sha256_hex(b"x");
        let text = format!("dir  lib/gnoblin/shells\n{good}  lib/a.so\ndir  lib/gnoblin\n");
        let manifest = parse_manifest(Path::new("m"), &text).unwrap();
        assert_eq!(manifest["lib/gnoblin/shells"], DIR_DIGEST);
        assert_eq!(
            removal_order(&manifest)
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            ["lib/a.so", "lib/gnoblin/shells", "lib/gnoblin"],
            "files first, then directories deepest first"
        );
        assert!(parse_manifest(Path::new("m"), "directory  lib/x\n").is_err());
    }

    #[test]
    fn the_core_install_makes_the_layout_and_uninstall_keeps_a_directory_another_package_filled() {
        let dir = scratch("dist-install-layout");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let target = fake_build(&dir, None, Some(&core));
        let destdir = dir.join("dest");
        // A shell package installed first: its directory is adopted, not refused, and not removed with ours.
        let root = install_root(&destdir, "/usr");
        fs::create_dir_all(root.join("share/gnoblin/shells")).unwrap();
        fs::write(root.join("share/gnoblin/shells/tiler.toml"), b"module = \"x\"\n").unwrap();

        install(&repo_root(), &options(&destdir, &target, "lib64")).unwrap();
        let manifest = read_manifest(&root.join(MANIFEST)).unwrap();
        assert_eq!(manifest["share/gnoblin/shells"], DIR_DIGEST);
        assert_eq!(manifest["lib64/gnoblin/shells"], DIR_DIGEST);
        assert!(root.join("lib64/gnoblin/shells").is_dir());
        assert!(root.join("share/gnoblin/js/gnoblin-core/index.ts").is_file());
        assert!(root.join("share/gnoblin/js/gnoblin-std/index.ts").is_file());
        assert!(root.join("share/gnoblin/types/gnoblin.d.ts").is_file());
        let config = fs::read_to_string(root.join("share/gnoblin/types/tsconfig.json")).unwrap();
        assert!(
            config.contains("./gnoblin-std/index.ts") && config.contains("../js/gnoblin-std/*"),
            "{config}"
        );
        let index = fs::read_to_string(root.join("share/gnoblin/types/gnoblin-std/index.ts")).unwrap();
        assert!(
            index.ends_with("export * from \"gnoblin-std/generated/objects\";\n"),
            "{index}"
        );
        assert_eq!(manifest_problems(&root).unwrap().0, Vec::<String>::new());

        let mut remove = options(&destdir, &target, "lib64");
        remove.action = DistAction::Uninstall;
        uninstall(&remove).unwrap();
        assert!(
            !root.join("lib64/gnoblin").exists(),
            "the empty module directory went with the install"
        );
        assert!(
            root.join("share/gnoblin/shells/tiler.toml").is_file(),
            "the other package's launcher file stays"
        );
        assert!(!root.join("share/gnoblin/js").exists());
        assert!(!root.join(MANIFEST).exists());

        // A file where a directory belongs is not ours to replace.
        let fresh = dir.join("fresh");
        fs::create_dir_all(fresh.join("usr/share/gnoblin")).unwrap();
        fs::write(fresh.join("usr/share/gnoblin/shells"), b"not a directory").unwrap();
        let error = install(&repo_root(), &options(&fresh, &target, "lib64")).unwrap_err();
        assert!(matches!(error, DistError::Foreign(_)), "{error}");
    }

    #[test]
    fn a_tree_that_drifted_from_its_manifest_is_reported() {
        let dir = scratch("dist-manifest-drift");
        let core_dir = dir.join("fixture-core");
        fs::create_dir_all(&core_dir).unwrap();
        let core = good_core(&core_dir);
        let target = fake_build(&dir, None, Some(&core));
        let destdir = dir.join("dest");
        install(&repo_root(), &options(&destdir, &target, "lib")).unwrap();
        let root = install_root(&destdir, "/usr");

        fs::write(root.join("share/gnoblin/js/gnoblin-core/index.ts"), b"// edited\n").unwrap();
        fs::remove_file(root.join("include/gnoblin/core.h")).unwrap();
        let (problems, listed) = manifest_problems(&root).unwrap();
        assert!(listed > 10, "{listed}");
        assert!(
            problems
                .iter()
                .any(|problem| problem.starts_with("share/gnoblin/js/gnoblin-core/index.ts no longer")),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .any(|problem| problem.starts_with("include/gnoblin/core.h is listed")),
            "{problems:?}"
        );
        let (problems, _) = manifest_problems(&dir.join("nothing")).unwrap();
        assert!(problems[0].contains("not made by `dist install`"), "{problems:?}");
    }
}
