//! The bytecode cache: what a module compiles to, kept between starts.
//!
//! Every start of the runtime and every reload used to strip the TypeScript of the stock library (52 modules) with
//! oxc and compile the result with QuickJS before the first line of the user's config ran. Both steps are pure
//! functions of the source text, so the result is kept: a module's QuickJS bytecode is written once under the user's
//! cache directory and loaded instead of compiled on every later start. The stock modules and the user's own files
//! (`shell.ts` and what it imports) are cached the same way.
//!
//! # Where and under what key
//!
//! Files live in `$XDG_CACHE_HOME/gnoblin/js/qjs-<toolchain>/` (`$HOME/.cache` without the variable), one file per
//! module named by the SHA-256 of everything its bytecode depends on: the toolchain, the module's name (QuickJS keeps
//! it in the bytecode, so the same text under another path is another entry), whether the text was stripped, and the
//! text itself. A changed source is a different file, so nothing is ever invalidated by hand. The toolchain is the
//! engine's own version string plus the versions of the oxc crates that strip the types (read from `Cargo.lock` by
//! `build.rs`) and of this crate, so a directory is never read by a build that differs in any of them; a build
//! that cannot name its toolchain does not cache at all. `GNOBLIN_JS_CACHE=0` turns the cache off, and deleting the
//! directory clears it.
//!
//! # Failure modes first
//!
//! 1. **Anything is wrong with a file.** It is missing, short, from another format, fails its checksum, or QuickJS
//!    refuses it: the module is compiled from source as if there were no cache, the bad file is removed, and a good
//!    one takes its place. A cache can only make a start faster, never make it fail.
//! 2. **The cache cannot be written.** No directory, a full disk, a read-only home: the write is dropped and logged
//!    at debug level, and the start goes on. Nothing is ever written in place: a file is made under a temporary name
//!    and renamed, so a crash or a second process leaves either the whole file or none.
//! 3. **Bytecode from an unknown source.** QuickJS does not verify the bytecode it is given, so a file is trusted
//!    only when its checksum matches what was written. The directory is made private to the user (mode 0700, files
//!    0600); a process that can write there can already edit the config, so the cache adds no authority.
//! 4. **The cache grows.** Each distinct text of a user's config is a new entry. The directory holds at most
//!    [`MAX_ENTRIES`] files, the oldest are removed first, and the directories of other toolchains are removed when
//!    this one is first written.
//!
//! # Why there is one `unsafe`
//!
//! QuickJS does not check the bytecode it is given, and `rquickjs` marks loading it `unsafe`. The call is in
//! `engine::load`, the crate's only `unsafe`, and the SAFETY comment there says what makes it sound: the checksum
//! and the toolchain-named directory above. `Cargo.toml` lowers the crate's `unsafe_code` from `forbid` to `deny`
//! for that one module.

use std::fmt::Write as _;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use gnoblin_config::Language;
use rquickjs::{Ctx, Module, WriteOptions};
use sha2::{Digest as _, Sha256};

gnoblin_log::logger!(log = "js");

/// The variable that turns the cache off when it is `0`.
pub const CACHE_VARIABLE: &str = "GNOBLIN_JS_CACHE";

/// The most files the cache directory keeps. The stock library is about fifty; the rest is room for the versions of
/// a config that a person edits.
pub const MAX_ENTRIES: usize = 512;

/// A temporary file older than this is a crashed writer's and is removed.
const STALE_TEMPORARY: Duration = Duration::from_secs(3600);

/// The start of every cache file; the last byte before the NUL is the format version.
const MAGIC: &[u8; 8] = b"GNJSBC\x01\0";
/// The magic, the payload's length (u64, little endian) and the payload's SHA-256.
const HEADER_LEN: usize = 8 + 8 + 32;

/// What the build knows of the toolchain that makes bytecode: the versions of the oxc crates (`build.rs` reads them
/// from `Cargo.lock`) and of the engine. Empty when `Cargo.lock` could not be read, which turns the cache off.
const BUILD_TOOLCHAIN: &str = env!("GNOBLIN_JS_TOOLCHAIN");

/// The 32 bytes that name one cache entry.
pub(crate) type Digest = [u8; 32];

/// How the cache has been used since it was made, for a test and for the numbers in the design document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JsCacheStats {
    /// Modules loaded from bytecode.
    pub hits: u64,
    /// Modules that had no usable bytecode and were compiled from source.
    pub misses: u64,
    /// Bytecode files written.
    pub writes: u64,
    /// Files that were there and were refused: a bad checksum, a short file, bytecode QuickJS would not read.
    pub rejected: u64,
}

#[derive(Debug, Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    writes: AtomicU64,
    rejected: AtomicU64,
}

#[derive(Debug)]
struct CacheState {
    /// The directory of this toolchain's files.
    dir: PathBuf,
    /// The directory the toolchains' directories are in, whose other children are pruned.
    parent: PathBuf,
    counters: Counters,
    /// Whether the directory exists and the stale toolchains are gone, so a start does that once.
    prepared: Mutex<bool>,
}

/// A bytecode cache, or the absence of one. Cheap to clone; a clone shares the counters.
#[derive(Clone, Debug, Default)]
pub struct JsBytecodeCache {
    inner: Option<Arc<CacheState>>,
}

/// The toolchain's name, or `None` when the build cannot name it. The build names the engine (`rquickjs-sys`, which
/// fixes the QuickJS that is compiled in) and the oxc crates that strip the types.
fn toolchain() -> Option<&'static str> {
    static KEY: LazyLock<Option<String>> = LazyLock::new(|| {
        if BUILD_TOOLCHAIN.is_empty() {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(MAGIC);
        hash.update(BUILD_TOOLCHAIN.as_bytes());
        hash.update([0]);
        hash.update(env!("CARGO_PKG_VERSION").as_bytes());
        hash.update([0]);
        hash.update(usize::BITS.to_le_bytes());
        hash.update(if cfg!(target_endian = "big") { [1] } else { [0] });
        Some(hex(&hash.finalize()[..8]))
    });
    KEY.as_deref()
}

/// The one place bytecode enters the engine.
#[allow(unsafe_code)]
mod engine {
    use rquickjs::{Ctx, Module};

    /// Declares the module `bytecode` holds.
    pub(super) fn load<'js>(ctx: &Ctx<'js>, bytecode: &[u8]) -> rquickjs::Result<Module<'js>> {
        // SAFETY: `Module::load` is unsafe because QuickJS does not validate bytecode. The caller is
        // `JsBytecodeCache::declare`, which passes only a payload whose SHA-256 matches the one written beside it by
        // this crate, from a file in a directory named for the toolchain that wrote it (`toolchain`): bytecode this
        // engine version produced from a module this crate compiled. The engine's own version check on top rejects a
        // format it does not read. The bytes are copied by the engine (`JS_READ_OBJ_ROM_DATA` is 0 in the vendored
        // QuickJS), so `bytecode` need not outlive the call.
        unsafe { Module::load(ctx.clone(), bytecode) }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

impl JsBytecodeCache {
    /// No cache: every module is compiled from source, as before the cache existed.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// A cache whose files live under `root`, in the directory of this toolchain. `disabled` when the build cannot
    /// name its toolchain.
    pub fn in_dir(root: impl Into<PathBuf>) -> Self {
        let Some(toolchain) = toolchain() else {
            return Self::disabled();
        };
        let parent: PathBuf = root.into();
        Self {
            inner: Some(Arc::new(CacheState {
                dir: parent.join(format!("qjs-{toolchain}")),
                parent,
                counters: Counters::default(),
                prepared: Mutex::new(false),
            })),
        }
    }

    /// The cache the user's environment asks for: `$XDG_CACHE_HOME/gnoblin/js` (or `$HOME/.cache/gnoblin/js`), off
    /// when `GNOBLIN_JS_CACHE` is `0` or when there is no place to put it. Only an absolute `XDG_CACHE_HOME` counts,
    /// as the XDG specification says.
    pub fn from_environment(env: &dyn Fn(&str) -> Option<String>) -> Self {
        if env(CACHE_VARIABLE).as_deref() == Some("0") {
            return Self::disabled();
        }
        let absolute = |name: &str| {
            env(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && !path.as_os_str().is_empty())
        };
        let Some(base) = absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache"))) else {
            return Self::disabled();
        };
        Self::in_dir(base.join("gnoblin").join("js"))
    }

    /// Whether modules are cached at all.
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// The directory the files of this toolchain live in, or `None` for a disabled cache.
    pub fn dir(&self) -> Option<&Path> {
        self.inner.as_ref().map(|inner| inner.dir.as_path())
    }

    /// What the cache has done so far.
    pub fn stats(&self) -> JsCacheStats {
        self.inner
            .as_ref()
            .map_or_else(JsCacheStats::default, |inner| JsCacheStats {
                hits: inner.counters.hits.load(Ordering::Relaxed),
                misses: inner.counters.misses.load(Ordering::Relaxed),
                writes: inner.counters.writes.load(Ordering::Relaxed),
                rejected: inner.counters.rejected.load(Ordering::Relaxed),
            })
    }

    /// The name of the entry for a module `name` made from `text` of `kind`, or `None` without a cache.
    pub(crate) fn digest(&self, kind: Language, name: &str, text: &str) -> Option<Digest> {
        self.inner.as_ref()?;
        let mut hash = Sha256::new();
        hash.update(toolchain()?.as_bytes());
        let tag = match kind {
            Language::TypeScript => "ts",
            Language::JavaScript => "js",
        };
        for part in [tag.as_bytes(), name.as_bytes()] {
            hash.update([0]);
            hash.update(part);
        }
        hash.update([0]);
        hash.update(text.as_bytes());
        Some(hash.finalize().into())
    }

    /// Declares module `name` from the bytecode cached under `digest`, or returns `None` when there is none that
    /// QuickJS accepts. A rejected file is removed.
    pub(crate) fn declare<'js>(&self, ctx: &Ctx<'js>, digest: &Digest, name: &str) -> Option<Module<'js>> {
        let inner = self.inner.as_ref()?;
        let Some(payload) = inner.read(digest, name) else {
            inner.counters.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        match engine::load(ctx, &payload) {
            Ok(module) => {
                inner.counters.hits.fetch_add(1, Ordering::Relaxed);
                Some(module)
            }
            Err(error) => {
                // QuickJS left its complaint pending on the context; the module is about to be compiled from
                // source, and a stale exception would be thrown at whatever runs next.
                drop(ctx.catch());
                log::warn!(area = "bytecode", %error, "cached bytecode for {name} was refused by the engine; compiling it");
                inner.counters.rejected.fetch_add(1, Ordering::Relaxed);
                inner.counters.misses.fetch_add(1, Ordering::Relaxed);
                inner.discard(digest);
                None
            }
        }
    }

    /// Writes the bytecode of `module`, declared from the source `digest` names. Failure is logged and dropped.
    pub(crate) fn store(&self, digest: &Digest, name: &str, module: &Module<'_>) {
        let Some(inner) = self.inner.as_ref() else {
            return;
        };
        let bytes = match module.write(WriteOptions::default()) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::debug!(area = "bytecode", %error, "the bytecode of {name} could not be serialised");
                return;
            }
        };
        match inner.write(digest, &bytes) {
            Ok(()) => {
                inner.counters.writes.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                log::debug!(area = "bytecode", %error, "the bytecode of {name} was not cached in {}", inner.dir.display());
            }
        }
    }
}

impl CacheState {
    fn path(&self, digest: &Digest) -> PathBuf {
        self.dir.join(format!("{}.bc", hex(digest)))
    }

    /// The verified bytecode for `digest`, read from its file. `None` for a miss, and for a file that must not be
    /// trusted, which is removed.
    fn read(&self, digest: &Digest, name: &str) -> Option<Vec<u8>> {
        let path = self.path(digest);
        let mut file = File::open(&path).ok()?;
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return None;
        }
        if checked_payload(&bytes).is_none() {
            log::warn!(
                area = "bytecode",
                "{} is damaged or from another format; compiling {name}",
                path.display()
            );
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            self.discard(digest);
            return None;
        }
        bytes.drain(..HEADER_LEN);
        Some(bytes)
    }

    /// Removes the file for `digest`.
    fn discard(&self, digest: &Digest) {
        let _ = fs::remove_file(self.path(digest));
    }

    /// Writes `payload` as the file for `digest`, whole or not at all.
    fn write(&self, digest: &Digest, payload: &[u8]) -> std::io::Result<()> {
        self.prepare()?;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let temporary = self.dir.join(format!(
            "{}.{}.{}.tmp",
            hex(digest),
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(MAGIC)?;
            file.write_all(&u64::try_from(payload.len()).unwrap_or(u64::MAX).to_le_bytes())?;
            file.write_all(&Sha256::digest(payload))?;
            file.write_all(payload)
        })();
        if let Err(error) = written.and_then(|()| fs::rename(&temporary, self.path(digest))) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        self.prune();
        Ok(())
    }

    /// Makes the directory (private to the user) and removes the directories of other toolchains, once.
    fn prepare(&self) -> std::io::Result<()> {
        let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
        if *prepared {
            return Ok(());
        }
        DirBuilder::new().recursive(true).mode(0o700).create(&self.dir)?;
        if let Ok(entries) = fs::read_dir(&self.parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ours = path == self.dir;
                let toolchain_dir = entry.file_name().to_string_lossy().starts_with("qjs-");
                if !ours && toolchain_dir && path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }
        *prepared = true;
        Ok(())
    }

    /// Keeps the directory to [`MAX_ENTRIES`] files, oldest first out, and removes a crashed writer's leftovers.
    fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if path.extension().is_some_and(|extension| extension == "tmp") {
                let age = SystemTime::now().duration_since(modified).unwrap_or_default();
                if age > STALE_TEMPORARY {
                    let _ = fs::remove_file(&path);
                }
            } else {
                files.push((modified, path));
            }
        }
        if files.len() <= MAX_ENTRIES {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - MAX_ENTRIES] {
            let _ = fs::remove_file(path);
        }
    }
}

/// The payload of a cache file, when the header names this format and the payload is whole and matches its checksum.
fn checked_payload(bytes: &[u8]) -> Option<&[u8]> {
    let (header, payload) = bytes.split_at_checked(HEADER_LEN)?;
    if &header[..8] != MAGIC {
        return None;
    }
    let length = u64::from_le_bytes(header[8..16].try_into().ok()?);
    if usize::try_from(length).ok()? != payload.len() {
        return None;
    }
    (header[16..] == Sha256::digest(payload)[..]).then_some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_in(dir: &Path) -> JsBytecodeCache {
        let cache = JsBytecodeCache::in_dir(dir);
        assert!(cache.is_enabled(), "the test build names its toolchain");
        cache
    }

    fn file_for(cache: &JsBytecodeCache, kind: Language, name: &str, text: &str) -> PathBuf {
        let digest = cache
            .digest(kind, name, text)
            .expect("an enabled cache names its entries");
        cache.inner.as_ref().expect("enabled").path(&digest)
    }

    #[test]
    fn a_digest_depends_on_the_kind_the_name_and_the_text() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let cache = cache_in(dir.path());
        let base = cache.digest(Language::TypeScript, "a", "x").expect("a digest");
        assert_eq!(Some(base), cache.digest(Language::TypeScript, "a", "x"));
        for other in [
            cache.digest(Language::JavaScript, "a", "x"),
            cache.digest(Language::TypeScript, "b", "x"),
            cache.digest(Language::TypeScript, "a", "y"),
        ] {
            assert_ne!(Some(base), other);
        }
        // The seams between the parts cannot be moved: "a" + "bc" is not "ab" + "c".
        assert_ne!(
            cache.digest(Language::TypeScript, "a", "bc"),
            cache.digest(Language::TypeScript, "ab", "c")
        );
    }

    #[test]
    fn a_disabled_cache_names_nothing() {
        let cache = JsBytecodeCache::disabled();
        assert!(!cache.is_enabled());
        assert_eq!(cache.digest(Language::TypeScript, "a", "x"), None);
        assert_eq!(cache.stats(), JsCacheStats::default());
    }

    #[test]
    fn the_environment_decides_where_the_cache_is_and_whether_it_is() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(known, _)| *known == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        let xdg = JsBytecodeCache::from_environment(&env(&[("XDG_CACHE_HOME", "/c"), ("HOME", "/h")]));
        assert!(xdg.dir().expect("enabled").starts_with("/c/gnoblin/js"));
        let home = JsBytecodeCache::from_environment(&env(&[("HOME", "/h")]));
        assert!(home.dir().expect("enabled").starts_with("/h/.cache/gnoblin/js"));
        let relative = JsBytecodeCache::from_environment(&env(&[("XDG_CACHE_HOME", "c"), ("HOME", "/h")]));
        assert!(
            relative.dir().expect("enabled").starts_with("/h/.cache/gnoblin/js"),
            "a relative XDG_CACHE_HOME is ignored"
        );
        assert!(
            !JsBytecodeCache::from_environment(&env(&[])).is_enabled(),
            "no place, no cache"
        );
        let off = JsBytecodeCache::from_environment(&env(&[("HOME", "/h"), (CACHE_VARIABLE, "0")]));
        assert!(!off.is_enabled(), "GNOBLIN_JS_CACHE=0 turns it off");
    }

    #[test]
    fn a_file_is_taken_only_when_it_is_whole_and_matches_its_checksum() {
        let payload = b"bytecode";
        let mut file = Vec::new();
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        file.extend_from_slice(&Sha256::digest(payload));
        file.extend_from_slice(payload);
        assert_eq!(checked_payload(&file), Some(&payload[..]));

        let mut flipped = file.clone();
        *flipped.last_mut().expect("a payload") ^= 1;
        assert_eq!(checked_payload(&flipped), None, "a flipped bit");
        assert_eq!(checked_payload(&file[..file.len() - 1]), None, "a short file");
        let mut longer = file.clone();
        longer.push(0);
        assert_eq!(checked_payload(&longer), None, "a long file");
        let mut other = file.clone();
        other[6] = 2;
        assert_eq!(checked_payload(&other), None, "another format version");
        assert_eq!(checked_payload(&[]), None, "an empty file");
        assert_eq!(checked_payload(&file[..HEADER_LEN - 1]), None, "a short header");
    }

    #[test]
    fn writing_leaves_one_private_file_and_no_temporary() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let cache = cache_in(dir.path());
        let digest = cache
            .digest(Language::JavaScript, "m", "export const a = 1;")
            .expect("a digest");
        let inner = cache.inner.as_ref().expect("enabled");
        inner.write(&digest, b"payload").expect("the write succeeds");
        let names: Vec<String> = fs::read_dir(&inner.dir)
            .expect("the directory exists")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![format!("{}.bc", hex(&digest))]);
        let mode = |path: &Path| {
            std::os::unix::fs::PermissionsExt::mode(&fs::metadata(path).expect("metadata").permissions()) & 0o777
        };
        assert_eq!(mode(&inner.dir), 0o700);
        assert_eq!(mode(&inner.path(&digest)), 0o600);
        assert_eq!(
            checked_payload(&fs::read(inner.path(&digest)).expect("readable")),
            Some(&b"payload"[..])
        );
    }

    #[test]
    fn a_new_toolchain_removes_the_directories_of_the_old_ones() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let stale = dir.path().join("qjs-0123456789abcdef");
        fs::create_dir_all(&stale).expect("a stale directory");
        fs::write(stale.join("x.bc"), b"old").expect("a stale file");
        let unrelated = dir.path().join("notes");
        fs::create_dir_all(&unrelated).expect("an unrelated directory");
        let cache = cache_in(dir.path());
        let digest = cache.digest(Language::JavaScript, "m", "1").expect("a digest");
        cache
            .inner
            .as_ref()
            .expect("enabled")
            .write(&digest, b"p")
            .expect("the write succeeds");
        assert!(!stale.exists(), "another toolchain's directory goes");
        assert!(unrelated.exists(), "what is not ours stays");
        assert!(cache.dir().expect("enabled").is_dir());
    }

    #[test]
    fn the_directory_keeps_at_most_the_newest_entries() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let cache = cache_in(dir.path());
        let inner = cache.inner.as_ref().expect("enabled");
        for index in 0..MAX_ENTRIES + 5 {
            let digest = cache
                .digest(Language::JavaScript, "m", &index.to_string())
                .expect("a digest");
            inner.write(&digest, b"p").expect("the write succeeds");
        }
        let count = fs::read_dir(&inner.dir).expect("the directory exists").count();
        assert_eq!(count, MAX_ENTRIES);
        let _ = file_for(&cache, Language::JavaScript, "m", "0");
    }
}
