//! The libpam boundary. This module is the only one allowed `unsafe`: it reads PAM items, stashes
//! the auth-stage password for the session stage, and forwards to `handoff`. Every entry point
//! returns a code that cannot grant or deny a login, so the module is never the deciding check.
#![allow(unsafe_code)]

use std::ffi::{CStr, OsStr};
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use zeroize::{Zeroize, Zeroizing};

use crate::handoff;

/// An opaque libpam handle. PAM owns it; the module only passes it back.
pub enum PamHandle {}

const PAM_SUCCESS: c_int = 0;
const PAM_IGNORE: c_int = 25;

const PAM_AUTHTOK: c_int = 6;
const PAM_OLDAUTHTOK: c_int = 7;

const PAM_UPDATE_AUTHTOK: c_int = 0x2000;
const PAM_PRELIM_CHECK: c_int = 0x4000;

/// The module-data key under which the auth stage leaves the password for the session stage.
const STASH_KEY: &CStr = c"gnoblin-keyring-authtok";
const RUNTIME_DIR_VAR: &CStr = c"XDG_RUNTIME_DIR";

unsafe extern "C" {
    fn pam_get_item(pamh: *const PamHandle, item_type: c_int, item: *mut *const c_void) -> c_int;
    fn pam_set_data(
        pamh: *mut PamHandle,
        module_data_name: *const c_char,
        data: *mut c_void,
        cleanup: Option<unsafe extern "C" fn(*mut PamHandle, *mut c_void, c_int)>,
    ) -> c_int;
    fn pam_get_data(pamh: *const PamHandle, module_data_name: *const c_char, data: *mut *const c_void) -> c_int;
    fn pam_getenv(pamh: *mut PamHandle, name: *const c_char) -> *const c_char;
}

/// Copy a PAM item into a buffer that is wiped on drop. PAM keeps its own copy and the module
/// does not free it.
///
/// # Safety
///
/// `pamh` must be the live handle PAM passed to the entry point that calls this.
unsafe fn item_bytes(pamh: *const PamHandle, item: c_int) -> Option<Zeroizing<Vec<u8>>> {
    let mut raw: *const c_void = ptr::null();
    // SAFETY: `pamh` is live, and `raw` is a valid out-pointer for the item value.
    let rc = unsafe { pam_get_item(pamh, item, &mut raw) };
    if rc != PAM_SUCCESS || raw.is_null() {
        return None;
    }
    // SAFETY: PAM stores these items as NUL-terminated strings, valid until the handle ends.
    let bytes = unsafe { CStr::from_ptr(raw.cast::<c_char>()) }.to_bytes();
    Some(Zeroizing::new(bytes.to_vec()))
}

/// The runtime dir the service listens under: `XDG_RUNTIME_DIR` when PAM has it (pam_systemd sets
/// it in the session stage), otherwise `/run/user/<uid>`, which is where systemd puts it.
///
/// # Safety
///
/// `pamh` must be the live handle PAM passed to the entry point that calls this.
unsafe fn runtime_dir(pamh: *mut PamHandle) -> PathBuf {
    // SAFETY: `pamh` is live, and the name is a valid NUL-terminated string.
    let value = unsafe { pam_getenv(pamh, RUNTIME_DIR_VAR.as_ptr()) };
    if !value.is_null() {
        // SAFETY: pam_getenv returns a NUL-terminated string owned by PAM.
        let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
        if !bytes.is_empty() {
            return PathBuf::from(OsStr::from_bytes(bytes));
        }
    }
    let uid = rustix::process::getuid().as_raw();
    PathBuf::from(format!("/run/user/{uid}"))
}

/// Frees a stashed password when PAM ends the handle or replaces the stash.
///
/// # Safety
///
/// `data` must be null or a pointer made by `Box::into_raw` on a `Zeroizing<Vec<u8>>`.
unsafe extern "C" fn free_stash(_pamh: *mut PamHandle, data: *mut c_void, _status: c_int) {
    if !data.is_null() {
        // SAFETY: per the contract above, `data` came from `Box::into_raw` in
        // `pam_sm_authenticate` and is freed exactly once, here.
        drop(unsafe { Box::from_raw(data.cast::<Zeroizing<Vec<u8>>>()) });
    }
}

/// Auth stage: keep the login password for the session stage. Always `PAM_IGNORE`, so the
/// module can never be the check that lets a login through.
///
/// # Safety
///
/// Called by libpam with a live handle and valid argument pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(
    pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    // SAFETY: `pamh` is live for this call.
    if let Some(password) = unsafe { item_bytes(pamh, PAM_AUTHTOK) } {
        let stash = Box::into_raw(Box::new(password)).cast::<c_void>();
        // SAFETY: `pamh` is live; the stash is owned by PAM from here and freed by `free_stash`.
        let rc = unsafe { pam_set_data(pamh, STASH_KEY.as_ptr(), stash, Some(free_stash)) };
        if rc != PAM_SUCCESS {
            // PAM did not take ownership, so free the stash here rather than leak the password.
            // SAFETY: `stash` came from `Box::into_raw` above and is not referenced elsewhere.
            unsafe { free_stash(pamh, stash, rc) };
        }
    }
    PAM_IGNORE
}

/// Account and credential stages have nothing to do.
///
/// # Safety
///
/// Called by libpam with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

/// Account stage has nothing to do.
///
/// # Safety
///
/// Called by libpam with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_acct_mgmt(
    _pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

/// Session stage: hand the stashed password to the keyring service, then wipe it. Always
/// `PAM_SUCCESS`, because a missing or refused hand-off must not fail the login; the service then
/// prompts once instead.
///
/// # Safety
///
/// Called by libpam with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_open_session(
    pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    let mut stash: *const c_void = ptr::null();
    // SAFETY: `pamh` is live; `stash` is a valid out-pointer.
    let rc = unsafe { pam_get_data(pamh, STASH_KEY.as_ptr(), &mut stash) };
    if rc != PAM_SUCCESS || stash.is_null() {
        // No password (fingerprint or autologin): nothing to hand off.
        return PAM_SUCCESS;
    }
    // SAFETY: the stash was made by `pam_sm_authenticate` and stays live until the handle ends.
    // It is only used here, on the thread libpam called us on, before it is wiped below.
    let password = unsafe { &mut *stash.cast_mut().cast::<Zeroizing<Vec<u8>>>() };
    // SAFETY: `pamh` is live for this call.
    let dir = unsafe { runtime_dir(pamh) };
    // The service prompts once if the hand-off fails, so the error is not the login's problem.
    let _ = handoff::unlock(&dir, password);
    password.zeroize();
    PAM_SUCCESS
}

/// Session close has nothing to do; the stash is already wiped.
///
/// # Safety
///
/// Called by libpam with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_close_session(
    _pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_SUCCESS
}

/// Password change: re-key the store from the old password to the new one, once, in the update
/// phase. Always `PAM_SUCCESS`. If the service is not running, the store stays locked with the
/// old password, as `design/keyring.md` describes.
///
/// # Safety
///
/// Called by libpam with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_chauthtok(
    pamh: *mut PamHandle,
    flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    if flags & PAM_PRELIM_CHECK != 0 || flags & PAM_UPDATE_AUTHTOK == 0 {
        return PAM_SUCCESS;
    }
    // SAFETY: `pamh` is live for this call.
    let old = unsafe { item_bytes(pamh, PAM_OLDAUTHTOK) };
    // SAFETY: as above.
    let new = unsafe { item_bytes(pamh, PAM_AUTHTOK) };
    if let (Some(old), Some(new)) = (old, new) {
        // SAFETY: as above.
        let dir = unsafe { runtime_dir(pamh) };
        // As with unlock, a failed re-key is not the password change's problem.
        let _ = handoff::change(&dir, &old, &new);
    }
    PAM_SUCCESS
}
