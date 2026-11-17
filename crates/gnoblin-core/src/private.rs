//! What `#[gnoblin::main]` expands to call. Not public API: the names here change without notice.
//!
//! A native shell exports one symbol, `gnoshell_entry_v1`, which returns a static [`GnoblinShellEntry`]. The macro emits
//! that symbol and the static; the descriptor's function pointers are the ones in this module, so the macro itself holds
//! no logic and no `unsafe`. `init` is where a shell generation starts: it checks the installed core's ABI major, installs
//! the panic hook that reports to the core, and calls the author's function.
#![allow(unsafe_code)]

use std::fmt::Display;
use std::panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

pub use gnoblin_core_abi as abi;
use gnoblin_core_abi::{GnoblinApi, GnoblinCtx, GnoblinShellStats};
pub use gnoblin_core_abi::{GnoblinShellEntry, GnoblinStr};

use crate::gnoblin::{CALLS, DROPS, Gnoblin, LogLevel, REGISTERED};
use crate::sys;

/// The shell's entry function, recorded by `gnoshell_entry_v1` before the core calls `init`.
type UserFn = fn(&'static Gnoblin) -> Result<(), String>;

static USER: OnceLock<UserFn> = OnceLock::new();

thread_local! {
    /// The handle of the generation whose entry function ran last on this thread.
    static CURRENT: std::cell::Cell<Option<&'static Gnoblin>> = const { std::cell::Cell::new(None) };
}

/// The handle of the generation that last started on this thread, for libraries a shell uses that have no `&Gnoblin` to hand
/// (`gnoblin-std`'s `Workspace::new("main")`). A shell has one thread, and a reload starts the new generation on it, so this is
/// the generation that is running; `None` outside a shell's entry.
pub fn current_gnoblin() -> Option<&'static Gnoblin> {
    CURRENT.with(std::cell::Cell::get)
}

/// Records the author's function. Called by the generated `gnoshell_entry_v1`; a second call keeps the first.
pub fn set_user(user: UserFn) {
    let _ = USER.set(user);
}

/// The raw context handle of `g`'s generation: the first argument of every function in `libgnoblin-std`'s table, which
/// the std SDK passes on behalf of the shell.
pub fn ctx_of(g: &Gnoblin) -> GnoblinCtx {
    g.ctx()
}

/// The installed core's function table, as `g` holds it: for `libgnoblin-std`, which registers providers and sets settings
/// through functions the SDK has no wrapper for.
pub fn api_of(g: &Gnoblin) -> &'static GnoblinApi {
    g.api()
}

/// The last error text on this thread, as the core wrote it (`E_NAME: what`).
pub fn last_error() -> String {
    crate::error::last_error_text()
}

/// A handle on the generation `ctx`, made by `libgnoblin-std` (which runs inside the shell's process and is given only the
/// handle) to talk to the installed core as that generation: the same table, the same context, the same calls a shell makes.
///
/// # Errors
///
/// The installed core refuses the ABI major this crate was built for.
pub fn gnoblin_for(ctx: GnoblinCtx) -> crate::Result<Gnoblin> {
    let mut table: *const GnoblinApi = core::ptr::null();
    // SAFETY: `gnoblin_get_api` writes one pointer through the pointer it is given, a live local.
    let rc = unsafe { sys::gnoblin_get_api(abi::ABI_MAJOR, abi::ABI_MINOR, &mut table) };
    crate::error::check(rc)?;
    // SAFETY: on success the core wrote a pointer to its static table, which is never unmapped (the core is linked with
    // `-z nodelete`), so the reference is valid for the rest of the process.
    let api = unsafe { table.as_ref() }.ok_or_else(|| crate::CoreError::from_code(abi::E_ABI))?;
    Ok(Gnoblin::new(api, ctx))
}

/// What an entry function may return: nothing, or a `Result` whose error is shown to the user.
pub trait IntoInit {
    /// The outcome as the text of a failure, if any.
    fn into_init(self) -> Result<(), String>;
}

impl IntoInit for () {
    fn into_init(self) -> Result<(), String> {
        Ok(())
    }
}

impl<T, E: Display> IntoInit for Result<T, E> {
    fn into_init(self) -> Result<(), String> {
        self.map(drop).map_err(|error| error.to_string())
    }
}

/// The descriptor a shell exports, built at compile time.
pub const fn entry(label: GnoblinStr) -> GnoblinShellEntry {
    GnoblinShellEntry {
        size: size_of::<GnoblinShellEntry>() as u32,
        abi_major: abi::ABI_MAJOR,
        abi_minor_built: abi::ABI_MINOR,
        flags: 0,
        init,
        shutdown,
        stats: Some(shell_stats),
        label,
    }
}

/// Starts one generation: checks the ABI, installs the panic hook, calls the shell's function.
///
/// # Safety
///
/// Called by the core's loader with a context it made for this generation, on the thread that becomes the shell thread.
/// `api` is ignored in favour of the table `gnoblin_get_api` returns, so the version check is the NEEDED core's.
pub unsafe extern "C" fn init(_api: *const GnoblinApi, ctx: GnoblinCtx) -> i32 {
    let mut table: *const GnoblinApi = core::ptr::null();
    // SAFETY: `gnoblin_get_api` writes one pointer through the pointer it is given, a live local.
    let rc = unsafe { sys::gnoblin_get_api(abi::ABI_MAJOR, abi::ABI_MINOR, &mut table) };
    if rc != abi::E_OK {
        return rc;
    }
    // SAFETY: on success the core wrote a pointer to its static table, which is never unmapped (the core is linked with
    // `-z nodelete`), so the reference is valid for the rest of the process.
    let Some(api) = (unsafe { table.as_ref() }) else {
        return abi::E_ABI;
    };
    install_panic_hook(api, ctx);
    // The handle is leaked: sixteen bytes per generation, so the author's closures may capture `g` for as long as the shell runs
    // (`g.bind("Super+Return", || g.spawn("foot"))`). The core never unmaps a generation's code (D16), so what the box costs is
    // what the library already costs, and a handle kept past its generation fails with `Retired` instead of acting.
    let g: &'static Gnoblin = Box::leak(Box::new(Gnoblin::new(api, ctx)));
    CURRENT.with(|current| current.set(Some(g)));
    let Some(user) = USER.get() else {
        g.log_at(LogLevel::CoreError, "the shell's entry function was not recorded");
        return abi::E_LOAD;
    };
    match catch_unwind(AssertUnwindSafe(|| user(g))) {
        Ok(Ok(())) => abi::E_OK,
        Ok(Err(message)) => {
            // SAFETY: `set_last_error` reads `len` bytes at `msg` for the call.
            let _ = unsafe { (api.set_last_error)(ctx, message.as_ptr(), message.len()) };
            g.log_at(LogLevel::CoreError, &format!("shell init error: {message}"));
            abi::E_LOAD
        }
        // The panic hook has already reported the message to the core.
        Err(_) => abi::E_PANIC,
    }
}

/// The shell's retirement hook. The core runs registered shutdown hooks itself, so there is nothing left for the
/// entry's own `shutdown` to do.
pub extern "C" fn shutdown(_ctx: GnoblinCtx) {}

/// Fills the counters the lifetime tests read.
///
/// # Safety
///
/// `out` must be null or point at a writable `GnoblinShellStats`.
pub unsafe extern "C" fn shell_stats(out: *mut GnoblinShellStats) {
    // SAFETY: the caller guarantees `out` is null or writable.
    if let Some(out) = unsafe { out.as_mut() } {
        *out = GnoblinShellStats {
            registered: REGISTERED.load(Ordering::Relaxed),
            calls: CALLS.load(Ordering::Relaxed),
            drops: DROPS.load(Ordering::Relaxed),
        };
    }
}

/// Routes panics in this shell's libstd to the core's log with the generation. The shell is a separate copy of libstd,
/// so without this a panic would print only the default message on stderr and the core would not know which generation
/// it came from.
fn install_panic_hook(api: &'static GnoblinApi, ctx: GnoblinCtx) {
    std::panic::set_hook(Box::new(move |info| {
        let text = describe(info);
        // SAFETY: `report_panic` reads `len` bytes at `msg` for the call and may be called from any thread.
        let _ = unsafe { (api.report_panic)(ctx, text.as_ptr(), text.len()) };
    }));
}

fn describe(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with a non-text payload".to_owned());
    match info.location() {
        Some(location) => format!(
            "{message} at {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        ),
        None => message,
    }
}
