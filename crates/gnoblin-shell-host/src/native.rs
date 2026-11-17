//! Loading a native shell module: unique copy, `dlopen`, entry check, `init`, and the bounded retire list.
//!
//! The protocol is `design/modular-framework.md` section 6:
//!
//! 1. Refuse the load when the retire list is full ([`LoadError::TooManyRetained`]): retired native code is never unmapped,
//!    so the number kept is bounded, and past the bound the user restarts.
//! 2. Copy the module to a private path and `dlopen` the copy with `RTLD_NOW | RTLD_LOCAL`, then unlink the copy. Opening
//!    the same path twice returns the same handle and shares statics, so a reload must be a different file.
//! 3. Look up `gnoshell_entry_v1` and check the descriptor's ABI major.
//! 4. Make a generation for it (staged) and call `init(api, ctx)` under `catch_unwind`.
//! 5. A failed module, and a retired one, is never `dlclose`d by default: a callback, a thread or a TLS destructor may still
//!    reach its code. The one exception is [`NativeModule::try_unload`], which is off unless the policy allows it and the
//!    module proves (`gnoshell_unload_safe`) that nothing of it is still running.
//!
//! This is the one loader. The core library instantiates it over its own function table; a test instantiates it over a
//! table of its own.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gnoblin_core_abi::{self as abi, GnoblinApi, GnoblinCtx, GnoblinShellEntry, GnoblinShellStats};

use crate::generation::{GenerationHost, GenerationId, GenerationInfo, GenerationState};
use crate::guard::{PANICS, code_name, last_error, payload_text, set_error};
use crate::handle::{KIND_CTX, encode};
use crate::log;
use crate::retire::{Retained, RetainedStats, RetainedWhy, RetireList, RoomError};

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

/// The optional export of a module that can say it is safe to unload: no thread, callback or TLS destructor of its own is
/// still running. `extern "C" fn() -> u32`, non-zero for yes.
const UNLOAD_SAFE_SYMBOL: &CStr = c"gnoshell_unload_safe";

/// Whether a module is opened from a private copy, or from its own path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copying {
    /// The loader's normal mode: a private copy per load, so two generations never share a mapping.
    Unique,
    /// Open the path itself. Only for a test that shows what happens when a reload does not copy.
    SamePath,
}

/// Why a module did not load.
#[derive(Debug)]
pub enum LoadError {
    /// The retire list is full: too many retired libraries are still mapped.
    TooManyRetained {
        /// Libraries retained.
        retained: usize,
        /// The limit.
        max: usize,
    },
    /// The module file does not exist. A build that failed, or has not finished, leaves none.
    Missing {
        /// The module path as asked for.
        path: PathBuf,
    },
    /// The private copy could not be made.
    Copy(String),
    /// `dlopen` failed for a reason the loader has no better words for: a truncated or corrupt file, a missing
    /// dependency. `message` is the dynamic loader's own text, with the private copy's path replaced by the module's.
    Dlopen {
        /// The module path as asked for.
        path: PathBuf,
        /// What `dlerror` said.
        message: String,
    },
    /// The shell references a symbol the installed core does not export: it was built against a newer core.
    UndefinedSymbol {
        /// The module path as asked for.
        path: PathBuf,
        /// The symbol the core lacks.
        symbol: String,
    },
    /// The shell was linked against another core major (`NEEDED libgnoblin-core.so.2`), which is not installed.
    OtherCore {
        /// The module path as asked for.
        path: PathBuf,
        /// The library file name the shell asks for.
        needed: String,
        /// The ABI major that file name carries.
        major: u32,
    },
    /// The file loaded but exports no `gnoshell_entry_v1`.
    NotAShell {
        /// The module path as asked for.
        path: PathBuf,
    },
    /// The module was built for another ABI major.
    AbiMajor {
        /// The module path as asked for.
        path: PathBuf,
        /// The major the module was built for.
        built: u32,
        /// The minor the module was built against.
        built_minor: u32,
    },
    /// The module's `init` returned a failure.
    Init {
        /// The module path as asked for.
        path: PathBuf,
        /// The error code `init` returned.
        code: i32,
        /// The text the shell or the core left in the last-error slot.
        message: String,
    },
}

impl From<RoomError> for LoadError {
    fn from(error: RoomError) -> Self {
        Self::TooManyRetained {
            retained: error.retained,
            max: error.max,
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::TooManyRetained { retained, max } => write!(
                f,
                "{retained} retired shell libraries are still mapped (the limit is {max}). Retired native code is never \
                 unloaded, so restart gnoblin to reload the shell again"
            ),
            LoadError::Missing { path } => write!(
                f,
                "{}: the shell library does not exist (a build that failed or has not finished leaves no file)",
                path.display()
            ),
            LoadError::Copy(message) => write!(f, "cannot make a private copy of the shell module: {message}"),
            LoadError::Dlopen { path, message } => {
                write!(f, "{}: cannot load the shell library: {message}", path.display())
            }
            LoadError::UndefinedSymbol { path, symbol } => write!(
                f,
                "{}: incompatible ABI: the shell needs the symbol `{symbol}`, which the installed core (ABI {}.{}) does not \
                 provide; it was built against a newer libgnoblin-core, so install that one or rebuild the shell",
                path.display(),
                abi::ABI_MAJOR,
                abi::ABI_MINOR
            ),
            LoadError::OtherCore { path, needed, major } => write!(
                f,
                "{}: incompatible ABI: this shell was built for gnoblin ABI {major}.x (it needs `{needed}`); the installed \
                 core is {}.{} (`{}`)",
                path.display(),
                abi::ABI_MAJOR,
                abi::ABI_MINOR,
                abi::SONAME
            ),
            LoadError::NotAShell { path } => write!(
                f,
                "{}: not a gnoblin shell: the library exports no `gnoshell_entry_v1` (was it built with gnoblin-core and #[gnoblin::main]?)",
                path.display()
            ),
            LoadError::AbiMajor {
                path,
                built,
                built_minor,
            } => write!(
                f,
                "{}: incompatible ABI: this shell was built for gnoblin ABI {built}.{built_minor}; the installed core is {}.{}",
                path.display(),
                abi::ABI_MAJOR,
                abi::ABI_MINOR
            ),
            LoadError::Init { path, code, message } => {
                write!(f, "{}: init failed ({}): {message}", path.display(), code_name(*code))
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// Why [`NativeModule::try_unload`] left the library mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnloadRefused {
    /// The policy does not allow unloading (the default).
    NotAllowed,
    /// The generation has not retired.
    NotRetired,
    /// Registrations or posts of the generation still exist, so a callback may still reach its code.
    StillHeld(usize),
    /// The module does not export `gnoshell_unload_safe`, or it answered no: a thread or a TLS destructor may remain.
    NotProven,
}

impl fmt::Display for UnloadRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAllowed => write!(f, "unloading retired shell libraries is not enabled"),
            Self::NotRetired => write!(f, "the generation has not retired"),
            Self::StillHeld(count) => write!(f, "{count} registration(s) of the generation still exist"),
            Self::NotProven => write!(
                f,
                "the module does not report (`gnoshell_unload_safe`) that it has no threads, callbacks or TLS destructors"
            ),
        }
    }
}

/// Loads native shell modules into a [`GenerationHost`], keeps the libraries it cannot unmap on a bounded list, and hands each
/// generation back as a [`NativeModule`].
///
/// It lives in a `static` next to the host and the function table it serves; [`NativeLoader::new`] is `const`.
pub struct NativeLoader<E: 'static> {
    host: &'static GenerationHost<E>,
    api: &'static GnoblinApi,
    retained: RetireList,
    copy_seq: AtomicU64,
}

impl<E: 'static> NativeLoader<E> {
    /// A loader whose generations live in `host` and whose modules are initialised with `api`.
    pub const fn new(host: &'static GenerationHost<E>, api: &'static GnoblinApi) -> Self {
        Self {
            host,
            api,
            retained: RetireList::new(),
            copy_seq: AtomicU64::new(1),
        }
    }

    /// The host the generations live in.
    pub fn host(&self) -> &'static GenerationHost<E> {
        self.host
    }

    /// The retire list: its policy, its numbers, its entries.
    pub fn retire_list(&self) -> &RetireList {
        &self.retained
    }

    /// How many libraries are kept mapped after retirement, abandonment or a failed load.
    pub fn retained_libraries(&self) -> usize {
        self.retained.stats().count
    }

    /// The numbers `gnoblinctl status` shows: how many libraries are retained, how much address space they hold, and the
    /// limit past which a reload is refused.
    pub fn retained_stats(&self) -> RetainedStats {
        self.retained.stats()
    }

    fn keep_mapped(
        &self,
        handle: usize,
        path: &Path,
        label: &str,
        why: RetainedWhy,
        mapped_bytes: u64,
    ) -> RetainedStats {
        self.retained.retain(Retained {
            handle,
            path: path.to_owned(),
            label: label.to_owned(),
            why,
            mapped_bytes,
        })
    }

    /// Makes the private directory copies live in: `$XDG_RUNTIME_DIR/gnoblin/<pid>` (the temp directory when there is
    /// none), mode 0700 and owned by this user.
    fn scratch_dir() -> Result<PathBuf, LoadError> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("gnoblin").join(std::process::id().to_string());
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|error| LoadError::Copy(format!("{}: {error}", dir.display())))?;
        let meta = fs::metadata(&dir).map_err(|error| LoadError::Copy(format!("{}: {error}", dir.display())))?;
        if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
            return Err(LoadError::Copy(format!(
                "{} is not private to this user (owner {}, mode {:o}); refusing to load code from it",
                dir.display(),
                meta.uid(),
                meta.mode() & 0o777
            )));
        }
        Ok(dir)
    }

    /// Copies `path` to a fresh private file. `create_new` is `O_CREAT | O_EXCL`, so a planted file or symlink is an error.
    fn unique_copy(&self, path: &Path) -> Result<PathBuf, LoadError> {
        let dir = Self::scratch_dir()?;
        let name = path
            .file_name()
            .map_or_else(|| "module".to_owned(), |name| name.to_string_lossy().into_owned());
        let target = dir.join(format!("gen-{}-{name}", self.copy_seq.fetch_add(1, Ordering::Relaxed)));
        let copy = || -> io::Result<()> {
            let mut source = File::open(path)?;
            let mut destination = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&target)?;
            io::copy(&mut source, &mut destination)?;
            Ok(())
        };
        copy().map_err(|error| LoadError::Copy(format!("{} -> {}: {error}", path.display(), target.display())))?;
        Ok(target)
    }

    /// Loads a native shell module into a staged generation and runs its `init`.
    ///
    /// On success the generation is staged: call [`NativeModule::activate`] to start delivering events to it. On failure
    /// whatever `init` registered is removed, and the library stays mapped (and counted).
    ///
    /// # Errors
    ///
    /// [`LoadError`]: the retire list is full, the copy or `dlopen` failed, the file is not a shell or was built for
    /// another ABI major, or `init` failed.
    pub fn load(&'static self, path: &Path, copying: Copying, reload_count: u32) -> Result<NativeModule<E>, LoadError> {
        self.retained.check_room()?;
        if matches!(fs::metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound) {
            return Err(LoadError::Missing { path: path.to_owned() });
        }
        let target = match copying {
            Copying::Unique => self.unique_copy(path)?,
            Copying::SamePath => path.to_owned(),
        };
        let mapped_bytes = fs::metadata(&target).map_or(0, |meta| meta.len());
        let c_path = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| LoadError::Copy(format!("{}: the path contains a NUL byte", target.display())))?;
        // SAFETY: `c_path` is a valid NUL-terminated string for the call. `dlopen` runs the library's constructors, which
        // is what loading a shell means.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        let opened = if handle.is_null() { Err(dl_error()) } else { Ok(handle) };
        if copying == Copying::Unique {
            // The mapping keeps the inode alive; the name is no longer needed.
            let _ = fs::remove_file(&target);
        }
        let handle = opened.map_err(|message| classify_dlopen(path, &target, &message))?;
        // SAFETY: `handle` is a live handle from `dlopen`, and the symbol name is a NUL-terminated literal.
        let symbol = unsafe { dlsym(handle, c"gnoshell_entry_v1".as_ptr()) };
        if symbol.is_null() {
            self.keep_mapped(handle as usize, path, "", RetainedWhy::Failed, mapped_bytes);
            return Err(LoadError::NotAShell { path: path.to_owned() });
        }
        // SAFETY: a native shell's one export is `extern "C" fn() -> *const GnoblinShellEntry` (`gnoshell_entry_v1`).
        let entry_fn =
            unsafe { core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const GnoblinShellEntry>(symbol) };
        // SAFETY: calling the shell's entry function; it returns a pointer to a static descriptor in its mapping.
        let entry = catch_unwind(AssertUnwindSafe(|| unsafe { entry_fn() })).unwrap_or(core::ptr::null());
        // SAFETY: the descriptor is a `static` in the module's mapping, which stays mapped for as long as the entry is
        // used (it is unmapped only by `try_unload`, which consumes the module), so a shared reference is valid.
        let Some(entry) = (unsafe { entry.as_ref() }) else {
            self.keep_mapped(handle as usize, path, "", RetainedWhy::Failed, mapped_bytes);
            return Err(LoadError::NotAShell { path: path.to_owned() });
        };
        let needed = core::mem::offset_of!(GnoblinShellEntry, stats);
        if entry.abi_major != abi::ABI_MAJOR || (entry.size as usize) < needed {
            self.keep_mapped(handle as usize, path, "", RetainedWhy::Failed, mapped_bytes);
            return Err(LoadError::AbiMajor {
                path: path.to_owned(),
                built: entry.abi_major,
                built_minor: entry.abi_minor_built,
            });
        }
        let label = if entry.label.ptr.is_null() {
            String::new()
        } else {
            // SAFETY: the label is `len` bytes of a string literal in the module's mapping.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(entry.label.ptr, entry.label.len) })
                .into_owned()
        };
        let generation = self.host.new_generation(GenerationInfo {
            module_path: path.display().to_string(),
            state_dir: String::new(),
            language: abi::LANG_NATIVE,
            reload_count,
            isolate: false,
        });
        let ctx = encode(KIND_CTX, generation);
        // SAFETY: calling the shell's `init` with the core's table and the context made for it, on this thread, which
        // becomes the shell thread. Panics cannot unwind out of the shell's `extern "C"` function, but a non-SDK shell may
        // declare it otherwise, so the call is contained.
        // The text a failing `init` leaves must be its own, never an earlier failure's.
        set_error(String::new());
        let outcome = catch_unwind(AssertUnwindSafe(|| unsafe {
            (entry.init)(core::ptr::from_ref(self.api), ctx)
        }));
        let code = match outcome {
            Ok(code) => code,
            Err(payload) => {
                set_error(format!(
                    "init unwound across the ABI: {}",
                    payload_text(payload.as_ref())
                ));
                abi::E_PANIC
            }
        };
        if code != abi::E_OK {
            if code == abi::E_PANIC {
                PANICS.fetch_add(1, Ordering::Relaxed);
            }
            let mut message = last_error();
            if message.is_empty() {
                message = format!("init returned {} without saying why", code_name(code));
            }
            self.host.finish_retire(generation);
            self.keep_mapped(handle as usize, path, &label, RetainedWhy::Failed, mapped_bytes);
            return Err(LoadError::Init {
                path: path.to_owned(),
                code,
                message,
            });
        }
        Ok(NativeModule {
            loader: self,
            generation,
            ctx,
            handle: handle as usize,
            entry,
            label,
            path: path.to_owned(),
            mapped_bytes,
            ended: AtomicBool::new(false),
        })
    }
}

/// One loaded generation of a shell: a mapped library, its entry descriptor and its context.
pub struct NativeModule<E: 'static> {
    loader: &'static NativeLoader<E>,
    generation: GenerationId,
    ctx: GnoblinCtx,
    handle: usize,
    entry: &'static GnoblinShellEntry,
    label: String,
    path: PathBuf,
    mapped_bytes: u64,
    /// Whether the entry's `shutdown` and the shutdown hooks have run.
    ended: AtomicBool,
}

impl<E: 'static> NativeModule<E> {
    /// The context handle the module's code holds.
    pub fn ctx(&self) -> GnoblinCtx {
        self.ctx
    }

    /// The generation number.
    pub fn generation(&self) -> GenerationId {
        self.generation
    }

    /// The label the shell gave its entry descriptor (its crate name).
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The path the module was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The ABI minor the module says it was built against.
    pub fn built_minor(&self) -> u32 {
        self.entry.abi_minor_built
    }

    /// Where the generation is in its life.
    pub fn state(&self) -> GenerationState {
        self.loader
            .host
            .state_of(self.generation)
            .unwrap_or(GenerationState::Retired)
    }

    /// Staged to active: from now the module's registrations receive events.
    ///
    /// # Errors
    ///
    /// The generation is not staged.
    pub fn activate(&self) -> Result<(), String> {
        self.loader.host.activate(self.generation)
    }

    /// The counters the shell keeps about itself, when it provides them.
    pub fn shell_stats(&self) -> Option<GnoblinShellStats> {
        let end = core::mem::offset_of!(GnoblinShellEntry, stats) + size_of::<usize>();
        if !abi::table_has(self.entry.size, end) {
            return None;
        }
        let stats = self.entry.stats?;
        let mut out = GnoblinShellStats {
            registered: 0,
            calls: 0,
            drops: 0,
        };
        // SAFETY: the entry's `stats` function writes one `GnoblinShellStats` through the pointer it is given, a live
        // local; the module's code is mapped.
        unsafe { stats(&raw mut out) };
        Some(out)
    }

    /// Runs queued posts and delivers queued events through the table, as the shell thread does. Returns the number of
    /// callbacks run, or a negative error code.
    pub fn dispatch(&self, timeout_ms: u32) -> i32 {
        // SAFETY: `dispatch` is a function of the table this module was initialised with.
        unsafe { (self.loader.api.dispatch)(self.ctx, timeout_ms) }
    }

    /// Ends the generation's own code: runs its shutdown hooks and then the entry's `shutdown`, while its registrations and
    /// claims are still in place. A reload's commit calls this before the new generation takes over, so the old code
    /// has finished (and may have written state for its successor) before anything of it is removed; [`retire`](Self::retire)
    /// then removes what it registered. Ending twice does nothing the second time, and ending a retired generation does
    /// nothing.
    pub fn end(&self) {
        if self.state() == GenerationState::Retired || self.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        self.loader.host.run_shutdown_hooks(self.generation);
        // SAFETY: `shutdown` is the entry's function, in code that is still mapped, called with the context it was made for.
        let shutdown = catch_unwind(AssertUnwindSafe(|| unsafe { (self.entry.shutdown)(self.ctx) }));
        if let Err(payload) = shutdown {
            PANICS.fetch_add(1, Ordering::Relaxed);
            log::error!(
                "shutdown of {} panicked: {}",
                self.label,
                payload_text(payload.as_ref())
            );
        }
    }

    /// Retires the generation: [`end`](Self::end)s it if it has not ended, removes every registration (running each
    /// `drop_user`), and marks it so every later call from its code returns `E_RETIRED`. The library stays mapped,
    /// is put on the retire list, and the retirement is logged with the running count. Returns how many registrations
    /// were removed; retiring twice does nothing the second time.
    pub fn retire(&self) -> usize {
        if self.state() == GenerationState::Retired {
            return 0;
        }
        self.end();
        let removed = self.loader.host.finish_retire(self.generation);
        let stats = self.keep_mapped(RetainedWhy::Retired);
        log::info!(
            generation = self.generation,
            "retired generation {} ({}): {removed} registration(s) removed, library NOT dlclosed; {stats}",
            self.generation,
            self.label
        );
        removed
    }

    /// Discards a generation that never became active (a failed or abandoned reload): its registrations are removed
    /// without any callback running, and its calls are refused from now on. Abandoning twice does nothing the second time.
    pub fn abandon(&self) -> usize {
        if self.state() == GenerationState::Retired {
            return 0;
        }
        let removed = self.loader.host.finish_retire(self.generation);
        let stats = self.keep_mapped(RetainedWhy::Abandoned);
        log::info!(
            generation = self.generation,
            "abandoned generation {} ({}): {removed} registration(s) removed, library NOT dlclosed; {stats}",
            self.generation,
            self.label
        );
        removed
    }

    fn keep_mapped(&self, why: RetainedWhy) -> RetainedStats {
        self.loader
            .keep_mapped(self.handle, &self.path, &self.label, why, self.mapped_bytes)
    }

    /// The `dlopen` handle of the module's mapping, for a test that compares mappings.
    pub fn library_handle(&self) -> usize {
        self.handle
    }

    /// Unmaps the library of a retired generation, when it is provably unreachable. Off by default: the policy must allow
    /// it ([`RetirePolicy::allow_unload`](crate::retire::RetirePolicy::allow_unload)), and the module must say
    /// (`gnoshell_unload_safe`) that it runs no thread, and holds no callback or TLS destructor, that could still reach
    /// its code. The generation must have retired and hold no registration or post.
    ///
    /// The module is consumed: its entry descriptor lives in the library being unmapped. On refusal it is handed back,
    /// still mapped, still on the retire list.
    ///
    /// # Errors
    ///
    /// The module and the reason it stays mapped.
    pub fn try_unload(self) -> Result<RetainedStats, (Self, UnloadRefused)> {
        let refuse = |module: Self, why: UnloadRefused| Err((module, why));
        if !self.loader.retained.policy().allow_unload {
            return refuse(self, UnloadRefused::NotAllowed);
        }
        if self.state() != GenerationState::Retired {
            return refuse(self, UnloadRefused::NotRetired);
        }
        let held = self.loader.host.held_by(self.generation);
        if held > 0 {
            return refuse(self, UnloadRefused::StillHeld(held));
        }
        if !self.reports_unload_safe() {
            return refuse(self, UnloadRefused::NotProven);
        }
        let Some(retained) = self.loader.retained.release(self.handle) else {
            // Not on the list: it never retired through this module (or was already given back).
            return refuse(self, UnloadRefused::NotRetired);
        };
        // SAFETY: the handle is a live `dlopen` handle of this module, the only one (a unique copy), and the generation
        // holds no registration, post, thread or TLS destructor by the module's own report. The module is consumed here,
        // so its entry descriptor is not used again.
        let rc = unsafe { dlclose(self.handle as *mut c_void) };
        let stats = self.loader.retained.stats();
        log::info!(
            generation = self.generation,
            "unloaded generation {} ({}): dlclose returned {rc}; {stats}",
            self.generation,
            retained.label
        );
        Ok(stats)
    }

    /// Whether the module exports `gnoshell_unload_safe` and it answers yes.
    fn reports_unload_safe(&self) -> bool {
        // SAFETY: `self.handle` is a live handle from `dlopen`, and the symbol name is a NUL-terminated literal.
        let symbol = unsafe { dlsym(self.handle as *mut c_void, UNLOAD_SAFE_SYMBOL.as_ptr()) };
        if symbol.is_null() {
            return false;
        }
        // SAFETY: the export is `extern "C" fn() -> u32` by the loader's contract (`UNLOAD_SAFE_SYMBOL`).
        let probe = unsafe { core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> u32>(symbol) };
        // SAFETY: calling an export of a library that is still mapped.
        catch_unwind(AssertUnwindSafe(|| unsafe { probe() })).is_ok_and(|answer| answer != 0)
    }
}

/// The text of the last `dlerror`.
fn dl_error() -> String {
    // SAFETY: `dlerror` returns null or a NUL-terminated string valid until the next `dl*` call on this thread.
    unsafe {
        let text = dlerror();
        if text.is_null() {
            "unknown dynamic loader error".to_owned()
        } else {
            CStr::from_ptr(text).to_string_lossy().into_owned()
        }
    }
}

/// Turns the dynamic loader's complaint about the module into the error that tells the user what to do. A symbol the core
/// lacks and a core of another major are ABI incompatibilities and say so, naming what is missing; anything else keeps the
/// loader's own words, with the private copy's name replaced by the module's (the copy is gone, the user never saw it).
fn classify_dlopen(path: &Path, copy: &Path, message: &str) -> LoadError {
    if let Some(rest) = message.split("undefined symbol: ").nth(1) {
        let symbol = rest
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or(rest);
        return LoadError::UndefinedSymbol {
            path: path.to_owned(),
            symbol: symbol.to_owned(),
        };
    }
    if let Some(before) = message
        .split(": cannot open shared object file")
        .next()
        .filter(|b| *b != message)
    {
        let needed = before.rsplit(char::is_whitespace).next().unwrap_or(before);
        let major = needed
            .strip_prefix("libgnoblin-core.so.")
            .and_then(|digits| digits.parse::<u32>().ok());
        if let Some(major) = major.filter(|major| *major != abi::ABI_MAJOR) {
            return LoadError::OtherCore {
                path: path.to_owned(),
                needed: needed.to_owned(),
                major,
            };
        }
    }
    let named = message.replace(&copy.display().to_string(), &path.display().to_string());
    let prefix = format!("{}: ", path.display());
    LoadError::Dlopen {
        path: path.to_owned(),
        message: named.strip_prefix(&prefix).unwrap_or(&named).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(message: &str) -> String {
        classify_dlopen(
            Path::new("/home/u/shell/libmine.so"),
            Path::new("/run/user/1000/gnoblin/42/gen-3-libmine.so"),
            message,
        )
        .to_string()
    }

    #[test]
    fn an_undefined_core_symbol_is_an_incompatible_abi_naming_it_and_the_installed_core() {
        let text = classify("/run/user/1000/gnoblin/42/gen-3-libmine.so: undefined symbol: gnoblin_future_call");
        assert_eq!(
            text,
            "/home/u/shell/libmine.so: incompatible ABI: the shell needs the symbol `gnoblin_future_call`, which the \
             installed core (ABI 1.0) does not provide; it was built against a newer libgnoblin-core, so install that \
             one or rebuild the shell"
        );
    }

    #[test]
    fn a_shell_linked_to_another_core_major_names_both_versions() {
        let text = classify("libgnoblin-core.so.2: cannot open shared object file: No such file or directory");
        assert_eq!(
            text,
            "/home/u/shell/libmine.so: incompatible ABI: this shell was built for gnoblin ABI 2.x (it needs \
             `libgnoblin-core.so.2`); the installed core is 1.0 (`libgnoblin-core.so.1`)"
        );
    }

    #[test]
    fn a_missing_dependency_that_is_not_the_core_keeps_the_loaders_words_and_the_modules_name() {
        let text = classify("libother.so.9: cannot open shared object file: No such file or directory");
        assert_eq!(
            text,
            "/home/u/shell/libmine.so: cannot load the shell library: libother.so.9: cannot open shared object file: \
             No such file or directory"
        );
        let truncated = classify("/run/user/1000/gnoblin/42/gen-3-libmine.so: file too short");
        assert_eq!(
            truncated,
            "/home/u/shell/libmine.so: cannot load the shell library: file too short"
        );
    }

    #[test]
    fn the_core_of_this_major_not_being_found_is_not_called_an_abi_mismatch() {
        let text = classify("libgnoblin-core.so.1: cannot open shared object file: No such file or directory");
        assert!(text.contains("cannot load the shell library"), "{text}");
        assert!(!text.contains("incompatible ABI"), "{text}");
    }
}
