//! [`Std`]: a shell generation's handle on the installed std library, internal to this crate.
//!
//! `Std::open` finds the library's function table (`gnoblin_std_get_api`, which refuses another ABI major) and attaches the library to
//! the generation. It is the one place the SDK meets the library's exports; a shell never holds one, because the classes of
//! [`objects`](crate::objects) are made from `gnoblin.objects()?.std()?` and carry their own handle.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::host::{ErrorKind, Host};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, GnoblinStdApi};

use crate::error::{Error, Result};
use crate::sys;

struct Inner {
    api: &'static GnoblinStdApi,
    ctx: GnoblinCtx,
}

/// A shell generation's handle on `libgnoblin-std`. Cheap to clone and compare; like the host handle it belongs to the shell thread
/// and is neither `Send` nor `Sync`.
#[derive(Clone)]
pub(crate) struct Std {
    inner: Rc<Inner>,
}

impl std::fmt::Debug for Std {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Std").field("ctx", &self.inner.ctx).finish()
    }
}

impl PartialEq for Std {
    fn eq(&self, other: &Std) -> bool {
        self.inner.ctx == other.inner.ctx
    }
}

thread_local! {
    /// The handle `open` made last on this thread: a shell opens the library once and every object finds it here.
    static OPENED: RefCell<Option<Std>> = const { RefCell::new(None) };
}

/// The text of the library's last failure on this thread.
pub(crate) fn last_error_text() -> String {
    // SAFETY: with a null buffer and zero capacity `gnoblin_std_last_error` only returns the length.
    let len = unsafe { sys::gnoblin_std_last_error(core::ptr::null_mut(), 0) };
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u8; len + 1];
    // SAFETY: `buf` is writable for `len + 1` bytes.
    let written = unsafe { sys::gnoblin_std_last_error(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(written.min(len));
    String::from_utf8_lossy(&buf).into_owned()
}

impl Std {
    /// Attaches the std library to `g`'s generation, once: a second call returns the same handle. This is the one place the SDK
    /// meets the library's exports; everything after goes through the table.
    ///
    /// # Errors
    ///
    /// The installed library is of another ABI major (the text names both), is older than this SDK, or cannot attach to the tree.
    pub(crate) fn open(g: &Host) -> Result<Std> {
        let ctx = gnoblin_core::__private::ctx_of(g);
        if let Some(std) = OPENED.with(|opened| opened.borrow().clone())
            && std.inner.ctx == ctx
        {
            return Ok(std);
        }
        let mut table: *const GnoblinStdApi = core::ptr::null();
        // SAFETY: `gnoblin_std_get_api` writes one pointer through the pointer it is given, a live local.
        let rc = unsafe { sys::gnoblin_std_get_api(ABI_MAJOR, ABI_MINOR, &mut table) };
        if rc < 0 {
            return Err(Error::new(ErrorKind::from_code(rc), last_error_text()));
        }
        // SAFETY: on success the library wrote a pointer to its static table, which is never unmapped (the library is linked with
        // `-z nodelete`), so the reference is valid for the rest of the process.
        let api = unsafe { table.as_ref() }
            .ok_or_else(|| Error::new(ErrorKind::Abi, "libgnoblin-std returned no function table"))?;
        if (api.size as usize) < size_of::<GnoblinStdApi>() {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "the installed libgnoblin-std is older than this SDK: its table is {} bytes and the SDK needs {}",
                    api.size,
                    size_of::<GnoblinStdApi>()
                ),
            ));
        }
        let std = Std {
            inner: Rc::new(Inner { api, ctx }),
        };
        // SAFETY: `open` takes only the context handle.
        std.check(unsafe { (api.open)(ctx) })?;
        OPENED.with(|opened| *opened.borrow_mut() = Some(std.clone()));
        Ok(std)
    }

    /// The function table of the installed library.
    pub(crate) fn api(&self) -> &'static GnoblinStdApi {
        self.inner.api
    }

    /// The generation's context handle: the first argument of every table function.
    pub(crate) fn ctx(&self) -> GnoblinCtx {
        self.inner.ctx
    }

    /// Turns a table function's return code into a value: a negative code is the failure, with the library's text.
    pub(crate) fn check(&self, code: i32) -> Result<i32> {
        if code < 0 {
            Err(Error::new(ErrorKind::from_code(code), last_error_text()))
        } else {
            Ok(code)
        }
    }

    /// The ABI version of the installed std library: `(major, minor, patch)`.
    pub(crate) fn installed_abi() -> (u32, u32, u32) {
        let (mut major, mut minor, mut patch) = (0, 0, 0);
        // SAFETY: `gnoblin_std_abi_version` writes three `u32`s through the pointers it is given; they are live locals.
        unsafe { sys::gnoblin_std_abi_version(&mut major, &mut minor, &mut patch) };
        (major, minor, patch)
    }
}
