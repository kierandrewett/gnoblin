//! The relayed link: the view of the core's tree that a language runtime with a mirror of its own uses instead of the node functions.
//!
//! The core's tree functions (`tree.rs`) give a shell the compositor's tree as nodes, properties and calls over the one mirror the
//! core keeps. The JavaScript engine predates them and carries a mirror of its own, with the transaction rules and the per-frame
//! builder its tests pin. It does not hold the compositor's link, and the compositor admits one holder: the core does (`hub.rs`), and
//! these five functions give the engine a view of it. `tree_link_open` makes the core attach as `tree_attach` does, `tree_link_attach`
//! answers the engine's attach from the core's mirror (a snapshot of it, then every message that arrives), `tree_link_send` puts the
//! engine's transactions on the one link and `tree_link_recv` takes what arrived for it, with the replies of its own transactions. The
//! std library, which works on the core's mirror, and the engine therefore see one tree. What crosses is bytes
//! (`gnoblin_tree_client::relay`), because no Rust type crosses the ABI.
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
pub(crate) fn attach(ctx: GnoblinCtx, _version: u64) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_link_attach", true) {
        return code;
    }
    match session::with(session::Session::attach_view) {
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
    let sent = session::with(|session| session.view()).and_then(|hub| {
        hub.borrow_mut()
            .send_view(&tx)
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
    let taken = session::with(|session| session.view()).and_then(|hub| {
        hub.borrow_mut()
            .take_view()
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
    let Some(hub) = session::with(|session| session.hub()) else {
        return -1;
    };
    let hub = hub.borrow();
    hub.wait_fd()
        .and_then(|fd| fd.try_clone_to_owned().ok())
        .map_or(-1, std::os::fd::IntoRawFd::into_raw_fd)
}
