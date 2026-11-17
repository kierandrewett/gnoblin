//! The relayed link: what a language runtime that keeps its own mirror uses instead of `tree_attach` and the node functions.
//!
//! The core's tree functions (`tree.rs`) give a shell the compositor's tree as nodes, properties and calls over the one mirror the
//! core keeps. The JavaScript runtime predates them and carries a mirror of its own, with the transaction rules and the
//! per-frame builder its tests pin; rewriting it onto the node functions is not part of putting it behind the runtime entry. It
//! drives the compositor's link itself, and these five functions are how: open the link the host provided, send the attach
//! request, send transactions, take what arrived, and a descriptor to wait on. What crosses is bytes
//! (`gnoblin_tree_client::relay`), because no Rust type crosses the ABI. The core keeps no mirror for a relayed shell and
//! delivers it no node events; it still serves its timers, posts, shutdown hooks and `request_reload`.
#![allow(unsafe_code)]

use gnoblin_core_abi::{self as abi, GnoblinBuf, GnoblinCtx};
use gnoblin_tree_client::relay;

use crate::guard::set_code_error;
use crate::memory::lend_bytes;
use crate::registry::check_ctx;
use crate::session::{self, AttachMode};

/// Records a failure of `what` and returns its code.
fn fail(what: &str, (code, text): (i32, String)) -> i32 {
    if text.starts_with(what) {
        set_code_error(code, text);
    } else {
        set_code_error(code, format!("{what}: {text}"));
    }
    code
}

/// `tree_link_open`.
pub(crate) fn open(ctx: GnoblinCtx, mode: u32) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_link_open", true) {
        return code;
    }
    let mode = match mode {
        abi::ATTACH_HOLDER => AttachMode::Holder,
        abi::ATTACH_OBSERVER => AttachMode::Observer,
        other => {
            set_code_error(abi::E_ERR, format!("tree_link_open: {other} is not an attach mode"));
            return abi::E_ERR;
        }
    };
    match session::with(|session| session.open_relayed(mode)) {
        Ok(()) => abi::E_OK,
        Err(error) => fail("tree_link_open", error),
    }
}

/// `tree_link_attach`.
pub(crate) fn attach(ctx: GnoblinCtx, version: u64) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_link_attach", true) {
        return code;
    }
    let sent = session::with(|session| {
        let link = session.relayed()?;
        link.attach(version)
            .map_err(|error| (abi::E_ERR, format!("the compositor's link failed: {error}")))
    });
    match sent {
        Ok(()) => abi::E_OK,
        Err(error) => fail("tree_link_attach", error),
    }
}

/// `tree_link_send`.
///
/// # Safety
///
/// `bytes` must be null or point at `len` readable bytes.
pub(crate) unsafe fn send(ctx: GnoblinCtx, bytes: *const u8, len: usize) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_link_send", true) {
        return code;
    }
    if bytes.is_null() {
        set_code_error(abi::E_ERR, "tree_link_send: a null buffer");
        return abi::E_ERR;
    }
    // SAFETY: the caller guarantees `bytes` points at `len` readable bytes.
    let encoded = unsafe { std::slice::from_raw_parts(bytes, len) };
    let tx = match relay::decode_tx(encoded) {
        Ok(tx) => tx,
        Err(error) => {
            set_code_error(abi::E_ERR, format!("tree_link_send: {error}"));
            return abi::E_ERR;
        }
    };
    let sent = session::with(|session| {
        let link = session.relayed()?;
        link.transact(&tx)
            .map_err(|error| (abi::E_ERR, format!("the compositor's link failed: {error}")))
    });
    match sent {
        Ok(()) => abi::E_OK,
        Err(error) => fail("tree_link_send", error),
    }
}

/// `tree_link_recv`.
///
/// # Safety
///
/// `out` must be null or writable.
pub(crate) unsafe fn recv(ctx: GnoblinCtx, out: *mut GnoblinBuf) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_link_recv", true) {
        return code;
    }
    if out.is_null() {
        set_code_error(abi::E_ERR, "tree_link_recv: a null output pointer");
        return abi::E_ERR;
    }
    let taken = session::with(|session| {
        let link = session.relayed()?;
        link.poll()
            .map_err(|error| (abi::E_ERR, format!("the compositor's link failed: {error}")))
    });
    match taken {
        Ok(messages) => {
            // SAFETY: the caller guarantees `out` is writable.
            unsafe { out.write(lend_bytes(relay::encode_incoming(&messages))) };
            abi::E_OK
        }
        Err(error) => fail("tree_link_recv", error),
    }
}

/// `tree_link_fd`: a duplicate the caller owns, or -1.
pub(crate) fn fd(ctx: GnoblinCtx) -> i32 {
    if check_ctx(ctx, "tree_link_fd", true).is_err() {
        return -1;
    }
    session::with(|session| {
        let link = session.relayed().ok()?;
        let fd = link.wait_fd()?;
        fd.try_clone_to_owned().ok().map(std::os::fd::IntoRawFd::into_raw_fd)
    })
    .unwrap_or(-1)
}
