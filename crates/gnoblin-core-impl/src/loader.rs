//! Loading a native shell module: unique copy, `dlopen`, entry check, `init`, and the retire list.
//!
//! The protocol is `design/modular-framework.md` section 6:
//!
//! 1. Copy the module to a private path and `dlopen` the copy with `RTLD_NOW | RTLD_LOCAL`, then unlink the copy. Opening
//!    the same path twice returns the same handle and shares statics, so a reload must be a different file.
//! 2. Look up `gnoshell_entry_v1` and check the descriptor's ABI major.
//! 3. Make a context for it (staged) and call `init(api, ctx)` under `catch_unwind`.
//! 4. A failed module, and a retired one, is never `dlclose`d: a callback, a thread or a TLS destructor may still reach its
//!    code. The core itself is linked `-z nodelete`.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinShellEntry, GnoblinShellStats};

use crate::guard::{PANICS, last_error, payload_text};
use crate::log;
use crate::registry::{self, GenerationState};
use crate::table::API;

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

/// Handles of libraries that were retired or failed to initialise. Never `dlclose`d.
static RETIRED_LIBS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static COPY_SEQ: AtomicU64 = AtomicU64::new(1);

/// How many module libraries are kept mapped after retirement or a failed load.
pub fn retired_libraries() -> usize {
    RETIRED_LIBS.lock().unwrap_or_else(PoisonError::into_inner).len()
}

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
    /// The private copy could not be made.
    Copy(String),
    /// `dlopen` failed; the loader's own text, which names a missing library.
    Dlopen {
        /// The module path as asked for.
        path: PathBuf,
        /// What `dlerror` said.
        message: String,
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

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Copy(message) => write!(f, "cannot make a private copy of the shell module: {message}"),
            LoadError::Dlopen { path, message } => write!(f, "{}: {message}", path.display()),
            LoadError::NotAShell { path } => write!(
                f,
                "{}: not a gnoblin shell: the library exports no `gnoshell_entry_v1` (was it built with gnoblin-core and #[gnoblin::main]?)",
                path.display()
            ),
            LoadError::AbiMajor { path, built } => write!(
                f,
                "{}: incompatible ABI: this shell was built for gnoblin ABI {built}.x; the installed core is {}.{}",
                path.display(),
                abi::ABI_MAJOR,
                abi::ABI_MINOR
            ),
            LoadError::Init { path, code, message } => {
                write!(
                    f,
                    "{}: init failed ({}): {message}",
                    path.display(),
                    crate::guard::code_name(*code)
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// One loaded generation of a shell: a mapped library, its entry descriptor and its context.
pub struct Module {
    ctx: GnoblinCtx,
    handle: usize,
    entry: &'static GnoblinShellEntry,
    label: String,
    path: PathBuf,
}

impl Module {
    /// The context handle the module's code holds.
    pub fn ctx(&self) -> GnoblinCtx {
        self.ctx
    }

    /// The generation number.
    pub fn generation(&self) -> u64 {
        registry::generation_of(self.ctx).unwrap_or(0)
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
        registry::state_of(self.ctx).unwrap_or(GenerationState::Retired)
    }

    /// Staged to active: from now the module's registrations receive events.
    pub fn activate(&self) -> Result<(), String> {
        registry::activate(self.ctx)
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
        // local; the module's code is mapped for the life of the process.
        unsafe { stats(&raw mut out) };
        Some(out)
    }

    /// Runs queued posts and delivers queued events through the table, as the shell thread does. Returns the number of
    /// callbacks run, or a negative error code.
    pub fn dispatch(&self, timeout_ms: u32) -> i32 {
        // SAFETY: `dispatch` is a function of the core's own table.
        unsafe { (API.dispatch)(self.ctx, timeout_ms) }
    }

    /// Retires the generation: runs its shutdown hooks and the entry's `shutdown`, removes every registration (running
    /// each `drop_user`), and marks it so every later call from its code returns `E_RETIRED`. The library stays mapped.
    /// Returns how many registrations were removed; retiring twice does nothing the second time.
    pub fn retire(&self) -> usize {
        if self.state() == GenerationState::Retired {
            return 0;
        }
        registry::run_shutdown_hooks(self.ctx);
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
        let removed = registry::finish_retire(self.ctx);
        keep_mapped(self.handle);
        log::info!(
            generation = self.generation(),
            "retired generation {} ({}): {removed} registration(s) removed, library NOT dlclosed",
            self.generation(),
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
        let removed = registry::finish_retire(self.ctx);
        keep_mapped(self.handle);
        log::info!(
            generation = self.generation(),
            "abandoned generation {} ({}): {removed} registration(s) removed, library NOT dlclosed",
            self.generation(),
            self.label
        );
        removed
    }

    /// The `dlopen` handle of the module's mapping, for a test that compares mappings.
    pub fn library_handle(&self) -> usize {
        self.handle
    }
}

fn keep_mapped(handle: usize) {
    RETIRED_LIBS.lock().unwrap_or_else(PoisonError::into_inner).push(handle);
}

/// The text of the last `dlerror`.
fn dl_error() -> String {
    // SAFETY: `dlerror` returns null or a NUL-terminated string valid until the next `dl*` call on this thread.
    unsafe {
        let text = dlerror();
        if text.is_null() {
            "unknown dynamic loader error".to_owned()
        } else {
            std::ffi::CStr::from_ptr(text).to_string_lossy().into_owned()
        }
    }
}

/// Makes the private directory copies live in: `$XDG_RUNTIME_DIR/gnoblin/<pid>` (the temp directory when there is none),
/// mode 0700 and owned by this user.
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
fn unique_copy(path: &Path) -> Result<PathBuf, LoadError> {
    let dir = scratch_dir()?;
    let name = path
        .file_name()
        .map_or_else(|| "module".to_owned(), |name| name.to_string_lossy().into_owned());
    let target = dir.join(format!("gen-{}-{name}", COPY_SEQ.fetch_add(1, Ordering::Relaxed)));
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
/// On success the generation is staged: call [`Module::activate`] to start delivering events to it. On failure whatever
/// `init` registered is removed, and the library stays mapped.
pub fn load(path: &Path, copying: Copying, reload_count: u32) -> Result<Module, LoadError> {
    let target = match copying {
        Copying::Unique => unique_copy(path)?,
        Copying::SamePath => path.to_owned(),
    };
    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| LoadError::Copy(format!("{}: the path contains a NUL byte", target.display())))?;
    // SAFETY: `c_path` is a valid NUL-terminated string for the call. `dlopen` runs the library's constructors, which is
    // what loading a shell means.
    let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
    let opened = if handle.is_null() { Err(dl_error()) } else { Ok(handle) };
    if copying == Copying::Unique {
        // The mapping keeps the inode alive; the name is no longer needed.
        let _ = fs::remove_file(&target);
    }
    let handle = opened.map_err(|message| LoadError::Dlopen {
        path: path.to_owned(),
        message,
    })?;
    // SAFETY: `handle` is a live handle from `dlopen`, and the symbol name is a NUL-terminated literal.
    let symbol = unsafe { dlsym(handle, c"gnoshell_entry_v1".as_ptr()) };
    if symbol.is_null() {
        keep_mapped(handle as usize);
        return Err(LoadError::NotAShell { path: path.to_owned() });
    }
    // SAFETY: a native shell's one export is `extern "C" fn() -> *const GnoblinShellEntry` (`gnoshell_entry_v1`).
    let entry_fn =
        unsafe { core::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const GnoblinShellEntry>(symbol) };
    // SAFETY: calling the shell's entry function; it returns a pointer to a static descriptor in its mapping.
    let entry = catch_unwind(AssertUnwindSafe(|| unsafe { entry_fn() })).unwrap_or(core::ptr::null());
    // SAFETY: the descriptor is a `static` in the module's mapping, which is never unmapped, so a shared reference is valid
    // for the rest of the process.
    let Some(entry) = (unsafe { entry.as_ref() }) else {
        keep_mapped(handle as usize);
        return Err(LoadError::NotAShell { path: path.to_owned() });
    };
    let needed = core::mem::offset_of!(GnoblinShellEntry, stats);
    if entry.abi_major != abi::ABI_MAJOR || (entry.size as usize) < needed {
        keep_mapped(handle as usize);
        return Err(LoadError::AbiMajor {
            path: path.to_owned(),
            built: entry.abi_major,
        });
    }
    let label = if entry.label.ptr.is_null() {
        String::new()
    } else {
        // SAFETY: the label is `len` bytes of a string literal in the module's mapping.
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(entry.label.ptr, entry.label.len) }).into_owned()
    };
    let ctx = registry::new_context(&path.display().to_string(), abi::LANG_NATIVE, reload_count, false);
    // SAFETY: calling the shell's `init` with the core's table and the context made for it, on this thread, which becomes
    // the shell thread. Panics cannot unwind out of the shell's `extern "C"` function, but a non-SDK shell may declare it
    // otherwise, so the call is contained.
    let outcome = catch_unwind(AssertUnwindSafe(|| unsafe { (entry.init)(&raw const API, ctx) }));
    let code = match outcome {
        Ok(code) => code,
        Err(payload) => {
            crate::guard::set_error(format!(
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
        let message = last_error();
        registry::finish_retire(ctx);
        keep_mapped(handle as usize);
        return Err(LoadError::Init {
            path: path.to_owned(),
            code,
            message,
        });
    }
    Ok(Module {
        ctx,
        handle: handle as usize,
        entry,
        label,
        path: path.to_owned(),
    })
}
