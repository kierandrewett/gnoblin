//! The libpam boundary. This module is the only one allowed `unsafe`.
//!
//! libpam is opened with `dlopen` on each check rather than linked, so the crate builds without
//! libpam's development package, and a machine without PAM gets a `Failed` answer instead of a
//! failed build or a failed launch.
//!
//! PAM frees the replies it receives from the conversation function, so the C copies of the
//! password are handed over and freed by libpam: they are not wiped. The Rust copies made here are
//! wiped on drop.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::size_of;
use std::ptr;

use libloading::Library;
use zeroize::Zeroizing;

const LIBRARY: &str = "libpam.so.0";

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// `struct pam_message` from `<security/_pam_types.h>`.
#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

/// `struct pam_response` from `<security/_pam_types.h>`.
#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConversationFn =
    unsafe extern "C" fn(c_int, *const *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int;

/// `struct pam_conv` from `<security/_pam_types.h>`.
#[repr(C)]
struct PamConv {
    conv: Option<ConversationFn>,
    appdata_ptr: *mut c_void,
}

type StartFn = unsafe extern "C" fn(*const c_char, *const c_char, *const PamConv, *mut *mut c_void) -> c_int;

/// `pam_authenticate` and `pam_acct_mgmt`: both take the handle and the flags.
type StageFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

type EndFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

/// What the conversation function answers with. PAM gets a pointer to this for the duration of
/// one `pam_start`..`pam_end` run.
struct Answers {
    user: Vec<u8>,
    password: Zeroizing<Vec<u8>>,
}

/// Run `pam_authenticate`, then `pam_acct_mgmt` if that passed, against `service`.
///
/// Returns the PAM return code of the last stage that ran, or an error message when libpam or one
/// of its entry points cannot be used. The message never contains the password.
pub(crate) fn authenticate(service: &CStr, user: &CStr, password: &[u8]) -> Result<c_int, String> {
    // SAFETY: loading the system libpam runs its initialisers; no caller-supplied code is loaded.
    let library = unsafe { Library::new(LIBRARY) }.map_err(|error| format!("cannot load {LIBRARY}: {error}"))?;
    // SAFETY: each symbol is a documented libpam entry point with the signature declared above.
    let start: StartFn = *unsafe { library.get::<StartFn>(b"pam_start\0") }.map_err(symbol_error)?;
    let auth_stage: StageFn = *unsafe { library.get::<StageFn>(b"pam_authenticate\0") }.map_err(symbol_error)?;
    let account_stage: StageFn = *unsafe { library.get::<StageFn>(b"pam_acct_mgmt\0") }.map_err(symbol_error)?;
    let end: EndFn = *unsafe { library.get::<EndFn>(b"pam_end\0") }.map_err(symbol_error)?;

    let mut answers = Answers {
        user: user.to_bytes().to_vec(),
        password: Zeroizing::new(password.to_vec()),
    };
    let conv = PamConv {
        conv: Some(conversation),
        appdata_ptr: (&raw mut answers).cast(),
    };
    let mut handle: *mut c_void = ptr::null_mut();

    // SAFETY: service and user are NUL-terminated and outlive the call; `conv` and `answers` stay
    // alive until `pam_end` below; `handle` is a valid out pointer.
    let code = unsafe { start(service.as_ptr(), user.as_ptr(), &raw const conv, &raw mut handle) };
    if code != PAM_SUCCESS {
        return Ok(code);
    }

    // SAFETY: `handle` came from a successful `pam_start` and is ended exactly once, below.
    let mut code = unsafe { auth_stage(handle, 0) };
    if code == PAM_SUCCESS {
        // SAFETY: as above; the handle is still live until `pam_end`.
        code = unsafe { account_stage(handle, 0) };
    }
    // SAFETY: last use of `handle`; the conversation data it pointed at is still alive.
    unsafe { end(handle, code) };
    Ok(code)
}

fn symbol_error(error: libloading::Error) -> String {
    format!("cannot use {LIBRARY}: {error}")
}

/// Answer PAM's prompts: the password for echo-off prompts, the user name for echo-on prompts.
/// Informational and error messages get no reply.
///
/// # Safety
///
/// Called only by libpam, with `messages` pointing at `count` message pointers, `responses` a valid
/// out pointer, and `appdata` the `Answers` passed to `pam_start`.
unsafe extern "C" fn conversation(
    count: c_int,
    messages: *const *const PamMessage,
    responses: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if count < 0 || messages.is_null() || responses.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    let count = count as usize;
    // SAFETY: calloc returns zeroed memory for `count` responses, or null. PAM frees the array.
    let replies = unsafe { calloc(count, size_of::<PamResponse>()) }.cast::<PamResponse>();
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    // SAFETY: `appdata` is the `Answers` that `authenticate` keeps alive until `pam_end`.
    let answers = unsafe { &*appdata.cast::<Answers>() };

    for index in 0..count {
        // SAFETY: `index` is below `count`, the length of the array libpam passed.
        let message = unsafe { *messages.add(index) };
        if message.is_null() {
            free_replies(replies, count);
            return PAM_CONV_ERR;
        }
        // SAFETY: libpam keeps each message alive for the duration of this call.
        let style = unsafe { (*message).msg_style };
        let answer: &[u8] = match style {
            PAM_PROMPT_ECHO_OFF => answers.password.as_slice(),
            PAM_PROMPT_ECHO_ON => answers.user.as_slice(),
            _ => continue,
        };
        let copy = copy_c_string(answer);
        if copy.is_null() {
            free_replies(replies, count);
            return PAM_BUF_ERR;
        }
        // SAFETY: `replies` has `count` zeroed entries and `index` is below `count`.
        unsafe { (*replies.add(index)).resp = copy };
    }

    // SAFETY: `responses` is the out pointer libpam supplied, checked non-null above.
    unsafe { *responses = replies };
    PAM_SUCCESS
}

/// A calloc'd NUL-terminated copy of `bytes`, or null when allocation fails.
fn copy_c_string(bytes: &[u8]) -> *mut c_char {
    // SAFETY: calloc zeroes `bytes.len() + 1` bytes, so the copy is NUL-terminated.
    let copy = unsafe { calloc(bytes.len() + 1, 1) }.cast::<c_char>();
    if !copy.is_null() {
        // SAFETY: `copy` has room for `bytes.len()` bytes and is a fresh allocation, so it cannot
        // overlap `bytes`.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), copy.cast::<u8>(), bytes.len()) };
    }
    copy
}

/// Free a reply array and every string in it. Entries not yet filled are zeroed, so their null
/// strings are skipped by `free`.
fn free_replies(replies: *mut PamResponse, count: usize) {
    for index in 0..count {
        // SAFETY: `index` is below `count`, the length of the array `replies` points at.
        unsafe { free((*replies.add(index)).resp.cast()) };
    }
    // SAFETY: `replies` came from calloc and is freed once, here.
    unsafe { free(replies.cast()) };
}
