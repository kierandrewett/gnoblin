//! `libgnoblin-std.so.1`, found and opened at run time.
//!
//! A script runtime cannot link the library the way a native std shell does: the runtime is loaded for every script, and a
//! script that never imports std must run where the library is not installed (example 17). So the
//! runtime opens it on the first use, with `dlopen`, and reads the table through the library's own export.

use core::ffi::{c_char, c_int, c_void};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::path::PathBuf;

use std::sync::atomic::{AtomicUsize, Ordering};

use gnoblin_core_abi::{GnoblinCtx, GnoblinObjectApi, GnoblinStr};
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, GnoblinStdApi, SONAME};

use crate::{Dyn, StdError, generated};

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
}

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

const RTLD_NOW: c_int = 2;

pub type LastErrorFn = unsafe extern "C" fn(buf: *mut u8, cap: usize) -> usize;
pub type GetApiFn = unsafe extern "C" fn(want_major: u32, built_minor: u32, out: *mut *const GnoblinStdApi) -> i32;

/// The text of the last `dlerror`, if any.
fn dl_error() -> String {
    // SAFETY: `dlerror` returns null or a NUL-terminated string valid until the next `dl*` call, which is copied at once.
    let text = unsafe { dlerror() };
    if text.is_null() {
        return String::new();
    }
    // SAFETY: non-null and NUL terminated, as above.
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}

/// The directory of the library that holds `anchor`: the core, whose siblings the loader would find the std library among.
fn directory_of(anchor: *const c_void) -> Option<PathBuf> {
    let mut info = DlInfo {
        dli_fname: core::ptr::null(),
        dli_fbase: core::ptr::null_mut(),
        dli_sname: core::ptr::null(),
        dli_saddr: core::ptr::null_mut(),
    };
    // SAFETY: `info` is a live local the call fills in; `anchor` is any address.
    if unsafe { dladdr(anchor, &raw mut info) } == 0 || info.dli_fname.is_null() {
        return None;
    }
    // SAFETY: `dli_fname` is a NUL-terminated path the loader owns.
    let path = PathBuf::from(unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned());
    path.parent().map(std::path::Path::to_path_buf)
}

fn open_path(path: &str) -> Option<*mut c_void> {
    let c_path = CString::new(path).ok()?;
    // SAFETY: `c_path` is a live NUL-terminated string.
    let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
    (!handle.is_null()).then_some(handle)
}

fn symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
    let c_name = CString::new(name).map_err(|error| error.to_string())?;
    // SAFETY: `handle` is a live library handle and `c_name` a NUL-terminated string.
    let found = unsafe { dlsym(handle, c_name.as_ptr()) };
    if found.is_null() {
        Err(format!("{SONAME} has no export `{name}`"))
    } else {
        Ok(found)
    }
}

/// The text `last_error` has for the last failure on this thread.
fn text_of(last_error: LastErrorFn) -> Option<String> {
    // SAFETY: with a null buffer and zero capacity the call only returns the length.
    let len = unsafe { last_error(core::ptr::null_mut(), 0) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0_u8; len + 1];
    // SAFETY: `buf` is writable for `len + 1` bytes.
    let written = unsafe { last_error(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(written.min(len));
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The last-error function of the library most recently opened in this process, as an address (zero before one is). The object protocol's
/// client takes a plain function to read the text of a failure, and the library is one per process, so the address is kept here.
static LAST_ERROR: AtomicUsize = AtomicUsize::new(0);

/// The text the installed library left for the last failure on this thread, or the empty string when no library is open.
///
/// This is the function to hand `gnoblin_object::client::Api::new` for the table [`StdLib::object_api`] returns.
pub fn last_error_text() -> String {
    let address = LAST_ERROR.load(Ordering::Acquire);
    if address == 0 {
        return String::new();
    }
    // SAFETY: the address was stored from a `LastErrorFn` of a library that is never unmapped (`-z nodelete`).
    let last_error = unsafe { core::mem::transmute::<usize, LastErrorFn>(address) };
    text_of(last_error).unwrap_or_default()
}

/// The installed std library: its table, and the generations it has attached to.
pub struct StdLib {
    api: &'static GnoblinStdApi,
    last_error: LastErrorFn,
    attached: RefCell<HashSet<GnoblinCtx>>,
}

impl StdLib {
    /// Opens the library.
    ///
    /// `anchor` is the address of any function of the core library the shell runs in: the library is looked for first by its
    /// name through the loader's own search (`LD_LIBRARY_PATH`, the system directories), then beside the core.
    ///
    /// # Errors
    ///
    /// The text a shell reports: the library is not installed (it names the file and where it looked), is of another ABI
    /// major, or is older than this layer needs.
    pub fn load(anchor: *const c_void) -> Result<StdLib, String> {
        let mut looked = vec!["the loader search path".to_owned()];
        let mut handle = open_path(SONAME);
        let first_failure = dl_error();
        if handle.is_none()
            && let Some(directory) = directory_of(anchor)
        {
            let beside = directory.join(SONAME);
            looked.push(directory.display().to_string());
            handle = open_path(&beside.to_string_lossy());
        }
        let Some(handle) = handle else {
            return Err(format!(
                "gnoblin-std is not installed (looked for {SONAME} in {}): {first_failure}",
                looked.join(", ")
            ));
        };
        let get_api = symbol(handle, "gnoblin_std_get_api")?;
        let last_error = symbol(handle, "gnoblin_std_last_error")?;
        // SAFETY: the two exports have the types the std ABI crate declares (`StdGetApiFn`, `StdLastErrorFn`).
        let (get_api, last_error): (GetApiFn, LastErrorFn) = unsafe {
            (
                core::mem::transmute::<*mut c_void, GetApiFn>(get_api),
                core::mem::transmute::<*mut c_void, LastErrorFn>(last_error),
            )
        };
        Self::from_entry_points(get_api, last_error)
    }

    /// The library behind the two entry points it exports, for a process that holds it by other means than `dlopen` (a test that links
    /// the implementation into its own binary, together with the core it is a guest of).
    ///
    /// # Errors
    ///
    /// The library is of another ABI major, or is older than this layer needs.
    pub fn from_entry_points(get_api: GetApiFn, last_error: LastErrorFn) -> Result<StdLib, String> {
        let mut table: *const GnoblinStdApi = core::ptr::null();
        // SAFETY: `get_api` writes one pointer through the pointer it is given, a live local.
        let code = unsafe { get_api(ABI_MAJOR, ABI_MINOR, &raw mut table) };
        // SAFETY: on success the library wrote a pointer to its static table, which is never unmapped (`-z nodelete`).
        let api = unsafe { table.as_ref() };
        match api {
            Some(api) if code >= 0 => {
                LAST_ERROR.store(last_error as usize, Ordering::Release);
                Ok(StdLib {
                    api,
                    last_error,
                    attached: RefCell::new(HashSet::new()),
                })
            }
            _ => Err(format!(
                "cannot use {SONAME}: {}",
                text_of(last_error).unwrap_or_else(|| format!("get_api failed with code {code}"))
            )),
        }
    }

    /// The table.
    pub(crate) fn api(&self) -> &'static GnoblinStdApi {
        self.api
    }

    /// Whether the library's table reaches `end` bytes: the function ending there exists.
    pub(crate) fn has(&self, end: usize) -> bool {
        self.api.size as usize >= end
    }

    /// The names of the table functions this build of the call layer knows.
    pub fn function_names() -> impl Iterator<Item = &'static str> {
        generated::FUNCTIONS.iter().map(|function| function.name)
    }

    /// The text the library left for the last failure on this thread.
    fn last_error_text(&self) -> Option<String> {
        text_of(self.last_error)
    }

    /// Turns a table function's return into the failure it is: a negative code with the library's text.
    pub(crate) fn check(&self, function: &str, code: i32) -> Result<i32, StdError> {
        if code >= 0 {
            return Ok(code);
        }
        Err(StdError::new(
            code,
            self.last_error_text()
                .unwrap_or_else(|| format!("{function} failed with code {code}")),
        ))
    }

    /// Turns a registering function's zero into the failure it is.
    pub(crate) fn check_handle(&self, function: &str, handle: u64) -> Result<u64, StdError> {
        if handle != 0 {
            return Ok(handle);
        }
        Err(StdError::new(
            gnoblin_core_abi::E_ERR,
            self.last_error_text().unwrap_or_else(|| format!("{function} failed")),
        ))
    }

    /// Attaches the library to the generation `ctx`, once.
    fn attach(&self, ctx: GnoblinCtx) -> Result<(), StdError> {
        if self.attached.borrow().contains(&ctx) {
            return Ok(());
        }
        // SAFETY: `open` takes only the generation's context.
        let code = unsafe { (self.api.open)(ctx) };
        self.check("open", code)?;
        self.attached.borrow_mut().insert(ctx);
        Ok(())
    }

    /// The object protocol's table of `namespace` (`gnoblin.std`) for the generation `ctx`, which the library attaches to first. The table is
    /// static; wrap it with `gnoblin_object::client::Api::new(table, ctx, last_error_text)`.
    ///
    /// # Errors
    ///
    /// The library is older than the object protocol, does not serve the namespace, or refuses the generation.
    pub fn object_api(&self, ctx: GnoblinCtx, namespace: &str) -> Result<&'static GnoblinObjectApi, StdError> {
        let end = core::mem::offset_of!(GnoblinStdApi, object_api) + size_of::<usize>();
        if !self.has(end) {
            return Err(StdError::new(
                gnoblin_core_abi::E_UNSUPPORTED,
                format!("the installed {SONAME} is older than the object protocol: its table has no `object_api`"),
            ));
        }
        self.attach(ctx)?;
        let mut table: *const GnoblinObjectApi = core::ptr::null();
        // SAFETY: `namespace` is borrowed for the call and `table` is a live local the library writes.
        let code = unsafe { (self.api.object_api)(ctx, GnoblinStr::borrowed(namespace), &raw mut table) };
        self.check("object_api", code)?;
        // SAFETY: on success the library wrote a pointer to its static table, which is never unmapped.
        unsafe { table.as_ref() }.ok_or_else(|| {
            StdError::new(
                gnoblin_core_abi::E_ERR,
                format!("{SONAME} gave no object table for `{namespace}`"),
            )
        })
    }

    /// Calls the table function `function` for the generation `ctx` with `args`, the way its SDK doc says: the arguments after
    /// the context in order, a record as a map, a list of records as a list, a callback as an object. The result is what the
    /// function writes (one out as itself, several as a list), else its return.
    ///
    /// # Errors
    ///
    /// An argument of the wrong kind (`E_TYPE`), a function the library lacks (`E_UNSUPPORTED`), or the code and text the
    /// library failed with.
    pub fn call(&self, ctx: GnoblinCtx, function: &str, args: &[Dyn]) -> Result<Dyn, StdError> {
        if !generated::FUNCTIONS
            .iter()
            .any(|info| info.name == function && info.standalone)
        {
            self.attach(ctx)?;
        }
        generated::dispatch(self, ctx, function, args)
    }
}
