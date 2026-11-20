//! [`Std`]: a shell generation's handle on the installed std library.
//!
//! `Std::open` finds the library's function table (`gnoblin_std_get_api`, which refuses another ABI major) and attaches the library to
//! the generation. Every object the SDK makes (a [`Client`](crate::Client), a [`Workspace`](crate::Workspace), a
//! [`Collection`](crate::Collection)) holds one, so it can call the library and so a handle that outlives its generation fails with
//! `Retired` instead of acting.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::{ErrorKind, Gnoblin};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, GnoblinStdApi, GnoblinStdStats};

use crate::error::{Error, Result};
use crate::sys;

struct Inner {
    g: Gnoblin,
    api: &'static GnoblinStdApi,
    ctx: GnoblinCtx,
}

/// A shell generation's handle on `libgnoblin-std`. Cheap to clone and compare; like [`Gnoblin`] it belongs to the shell thread
/// and is neither `Send` nor `Sync`.
#[derive(Clone)]
pub struct Std {
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

/// Counters the library keeps for a generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Workspaces that exist.
    pub workspaces: u64,
    /// Clients the library knows.
    pub clients: u64,
    /// Collections that exist.
    pub collections: u64,
    /// Listeners the library holds.
    pub registrations: u64,
    /// Layout passes that ran.
    pub layout_passes: u64,
    /// Layout `compute` calls that ran.
    pub computes: u64,
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
    pub fn open(g: &Gnoblin) -> Result<Std> {
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
            inner: Rc::new(Inner { g: g.clone(), api, ctx }),
        };
        // SAFETY: `open` takes only the context handle.
        std.check(unsafe { (api.open)(ctx) })?;
        OPENED.with(|opened| *opened.borrow_mut() = Some(std.clone()));
        Ok(std)
    }

    /// The std handle of the generation that is running on this thread: for objects made without a `&Gnoblin` to hand, such as
    /// [`Workspace::new`](crate::Workspace::new). Opens the library on first use.
    ///
    /// # Errors
    ///
    /// Called outside a shell's entry (no generation has started on this thread), or as for [`Std::open`].
    pub fn current() -> Result<Std> {
        match gnoblin_core::__private::current_gnoblin() {
            Some(g) => Std::open(g),
            None => Err(Error::new(
                ErrorKind::Failed,
                "no shell generation is running on this thread: make std objects inside the shell's entry function, or use `Std::open(g)`",
            )),
        }
    }

    /// The core handle of this generation.
    pub fn gnoblin(&self) -> &Gnoblin {
        &self.inner.g
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

    /// The failure of a function that returns a handle (zero is the failure) and so carries no code: the library's text starts with
    /// the symbolic name of the code (`E_REFUSED: ...`).
    pub(crate) fn failure(&self) -> Error {
        let text = last_error_text();
        let kind = ErrorKind::from_symbol(text.split_once(':').map_or("", |(name, _)| name));
        Error::new(kind, text)
    }

    /// Fills the library's counters for this generation.
    ///
    /// # Errors
    ///
    /// The generation has retired.
    pub fn stats(&self) -> Result<Stats> {
        let mut raw = GnoblinStdStats {
            size: size_of::<GnoblinStdStats>() as u32,
            reserved: 0,
            workspaces: 0,
            clients: 0,
            collections: 0,
            registrations: 0,
            layout_passes: 0,
            computes: 0,
        };
        // SAFETY: `stats` fills the struct it is given up to the `size` the caller set; `raw` is a live local.
        self.check(unsafe { (self.api().stats)(self.ctx(), &mut raw) })?;
        Ok(Stats {
            workspaces: raw.workspaces,
            clients: raw.clients,
            collections: raw.collections,
            registrations: raw.registrations,
            layout_passes: raw.layout_passes,
            computes: raw.computes,
        })
    }

    /// The ABI version of the installed std library: `(major, minor, patch)`.
    pub fn installed_abi() -> (u32, u32, u32) {
        let (mut major, mut minor, mut patch) = (0, 0, 0);
        // SAFETY: `gnoblin_std_abi_version` writes three `u32`s through the pointers it is given; they are live locals.
        unsafe { sys::gnoblin_std_abi_version(&mut major, &mut minor, &mut patch) };
        (major, minor, patch)
    }
}
