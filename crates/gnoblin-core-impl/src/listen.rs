//! Registrations for the events a shell thread produces itself: changes of the tree, input claims, and timers.
//!
//! Each is a [`Subscription`] over the host's registrations, so it is owned by the generation that made it, receives nothing
//! while the generation is only staged, and is removed with it. What each one also owns in the session (a claim the compositor
//! holds, a timer) is released by the subscription's `cleanup`, which runs with the registration whatever ends it: `remove`, the
//! generation retiring, or a one-shot firing.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinClaim, GnoblinClaimEvent, GnoblinClaimFn, GnoblinCtx, GnoblinDropFn, GnoblinNodeEvent,
    GnoblinNodeFn, GnoblinReg, GnoblinStr, GnoblinTimerFn,
};
use gnoblin_shell_host::generation::{CallbackOutcome, Release};
use gnoblin_shell_host::handle::{KIND_REG, encode};

use crate::catalog;
use crate::events::ShellEvent;
use crate::guard::set_code_error;
use crate::registry::{self, Subscription, check_ctx, is_active};
use crate::session;
use crate::values::{RawScope, read_text};

// ---- changes of the tree ----------------------------------------------------------------------------------------------

/// Which node events a registration wants.
pub(crate) struct NodeFilter {
    /// `NODE_CREATED`, `NODE_DESTROYED` or `NODE_CHANGED`.
    pub kind: u32,
    /// One node, or zero for every node of the class.
    pub node: u64,
    /// A class, or zero for every class.
    pub class: u32,
    /// For a change: one property of the class, or zero for every property.
    pub property: u32,
}

/// `on_change`, `on_node_created` and `on_node_destroyed`.
pub(crate) fn on_node(
    ctx: GnoblinCtx,
    what: &str,
    filter: NodeFilter,
    cb: GnoblinNodeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    if check_ctx(ctx, what, true).is_err() {
        return 0;
    }
    if filter.class != 0 && catalog::class(filter.class).is_none() {
        set_code_error(
            abi::E_NO_SUCH_MEMBER,
            format!("{what}: no class with id {}", filter.class),
        );
        return 0;
    }
    if filter.property != 0 && (filter.class == 0 || catalog::property_def(filter.class, filter.property).is_none()) {
        set_code_error(
            abi::E_NO_SUCH_MEMBER,
            format!(
                "{what}: property {} is not a property of class {} (a property needs its class)",
                filter.property, filter.class
            ),
        );
        return 0;
    }
    let user_word = user as usize;
    let NodeFilter {
        kind,
        node,
        class,
        property,
    } = filter;
    let subscription = Subscription {
        matches: Box::new(move |event: &ShellEvent| {
            matches!(event, ShellEvent::Node(notice)
                if notice.kind == kind
                    && (node == 0 || notice.node == node)
                    && (class == 0 || notice.class == class)
                    && (property == 0 || notice.property == property))
        }),
        call: Arc::new(move |event: &ShellEvent| {
            let ShellEvent::Node(notice) = event else {
                return CallbackOutcome::Handled;
            };
            let mut scope = RawScope::default();
            let value = notice.value.as_ref().map(|value| scope.raw(value));
            let raw = GnoblinNodeEvent {
                size: size_of::<GnoblinNodeEvent>() as u32,
                kind: notice.kind,
                class: notice.class,
                property: notice.property,
                node: notice.node,
                parent: notice.parent,
                value: value.as_ref().map_or(core::ptr::null(), core::ptr::from_ref),
            };
            // SAFETY: `cb` and `user_word` came from one registering call; `raw` and the value it points at are live locals,
            // valid for the whole call, as the ABI promises the callback.
            let rc = unsafe { cb(user_word as *mut c_void, &raw const raw) };
            registry::outcome_of(rc)
        }),
        cleanup: None,
    };
    match registry::subscribe(ctx, what, subscription, user, drop_user) {
        Ok(id) => encode(KIND_REG, id),
        Err(_) => 0,
    }
}

// ---- input claims -----------------------------------------------------------------------------------------------------

/// The seat call a claim kind makes: its method, its second integer and its text.
fn claim_call(
    spec: &ClaimSpec<'_>,
    subject: &str,
    device: &str,
    what: &str,
) -> Result<(&'static str, Option<i64>, String), (i32, String)> {
    let scoped = |text: &str| {
        if device.is_empty() {
            text.to_owned()
        } else {
            format!("{device}\n{text}")
        }
    };
    if device.contains('\n') {
        return Err((abi::E_ERR, format!("{what}: a device name is one line")));
    }
    match spec.kind {
        abi::CLAIM_CHORD => {
            if subject.is_empty() {
                return Err((abi::E_ERR, format!("{what}: a chord claim needs a chord")));
            }
            Ok((
                "claim",
                Some(i64::from(spec.flags & abi::CLAIM_FLAG_GRAB != 0)),
                scoped(subject),
            ))
        }
        abi::CLAIM_GESTURE => {
            if !matches!(subject, "swipe" | "pinch") {
                return Err((
                    abi::E_ERR,
                    format!("{what}: a gesture is `swipe` or `pinch`, not `{subject}`"),
                ));
            }
            if !(2..=5).contains(&spec.fingers) {
                return Err((
                    abi::E_ERR,
                    format!("{what}: a gesture takes 2 to 5 fingers, not {}", spec.fingers),
                ));
            }
            Ok(("claim_gesture", Some(spec.fingers), scoped(subject)))
        }
        abi::CLAIM_DEVICE | abi::CLAIM_POINTER => {
            if device.is_empty() {
                return Err((abi::E_ERR, format!("{what}: this claim needs a device name")));
            }
            let method = if spec.kind == abi::CLAIM_DEVICE {
                "claim_device"
            } else {
                "claim_pointer"
            };
            Ok((method, None, device.to_owned()))
        }
        abi::CLAIM_GRAB => {
            if !matches!(subject, "pointer" | "keyboard" | "both") {
                return Err((
                    abi::E_ERR,
                    format!("{what}: a grab's scope is `pointer`, `keyboard` or `both`, not `{subject}`"),
                ));
            }
            let node = i64::try_from(spec.node)
                .map_err(|_| (abi::E_ERR, format!("{what}: the grab's node id is out of range")))?;
            Ok(("grab", Some(node), subject.to_owned()))
        }
        other => Err((abi::E_UNSUPPORTED, format!("{what}: {other} is not a claim kind"))),
    }
}

/// What a claim asks of a seat: the fields of [`GnoblinClaim`] with the text borrowed.
pub(crate) struct ClaimSpec<'a> {
    pub kind: u32,
    pub flags: u32,
    pub fingers: i64,
    pub subject: &'a str,
    pub device: &'a str,
    pub node: u64,
}

/// What a claim does with each event it hears.
pub(crate) type ClaimHandler = Arc<dyn Fn(&crate::events::ClaimNotice) -> CallbackOutcome + Send + Sync>;

/// Claims `spec` on `seat` for the generation of `ctx` and routes the events of the claim to `handler`. Answers the claim's id and the
/// registration that owns it: removing the registration gives the claim up, and so does the generation's end. A claim of a staged
/// generation is kept and sent when it takes over.
///
/// # Errors
///
/// The ABI code and the words of a claim that cannot be made: a bad spec, a node that is not a seat, a generation that is gone.
pub(crate) fn claim_with(
    ctx: GnoblinCtx,
    seat: u64,
    spec: &ClaimSpec<'_>,
    handler: Option<ClaimHandler>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> Result<(i64, u64), (i32, String)> {
    let what = "claim";
    let owner = registry::owner_of(ctx, what).map_err(|code| (code, format!("{what}: the generation is gone")))?;
    let call = claim_call(spec, spec.subject, spec.device, what)?;
    let class = session::with(|session| session.tree_ref().map(|tree| tree.class_of(seat)))
        .map_err(|(code, text)| (code, format!("claim: {text}")))?;
    if class.as_deref() != Some("seat") {
        return Err((
            abi::E_NO_SUCH_NODE,
            format!("claim: node {seat} is not a seat of the tree"),
        ));
    }
    let id = session::with(session::Session::next_claim_id);
    let active = is_active(owner);
    session::with(|session| session.add_claim(id, seat, owner, call, active))?;
    let has_handler = handler.is_some();
    let cleanup: Release = Box::new(move || session::with(|session| session.release_claim(id)));
    let subscription = Subscription {
        matches: Box::new(move |event: &ShellEvent| {
            has_handler && matches!(event, ShellEvent::Claim(notice) if notice.claim == id)
        }),
        call: Arc::new(move |event: &ShellEvent| match (event, &handler) {
            (ShellEvent::Claim(notice), Some(handler)) => handler(notice),
            _ => CallbackOutcome::Handled,
        }),
        cleanup: Some(cleanup),
    };
    match registry::subscribe(ctx, what, subscription, user, drop_user) {
        Ok(reg) => Ok((id, reg)),
        Err(code) => {
            // The claim was made before the registration; with no registration to own it, it goes.
            session::with(|session| session.release_claim(id));
            Err((code, "claim: the generation cannot hold a registration".to_owned()))
        }
    }
}

/// `claim`.
///
/// # Safety
///
/// `spec` must be null or point at a `GnoblinClaim` valid for the call, with the strings it holds.
pub(crate) unsafe fn claim(
    ctx: GnoblinCtx,
    seat: u64,
    spec: *const GnoblinClaim,
    cb: Option<GnoblinClaimFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    let what = "claim";
    if registry::owner_of(ctx, what).is_err() {
        return 0;
    }
    // SAFETY: the caller guarantees `spec` is null or valid for the call.
    let Some(spec) = (unsafe { spec.as_ref() }) else {
        set_code_error(abi::E_ERR, "claim: a null claim");
        return 0;
    };
    if (spec.size as usize) < size_of::<GnoblinClaim>() {
        set_code_error(
            abi::E_ABI,
            format!(
                "claim: a claim of {} bytes is shorter than this core's ({})",
                spec.size,
                size_of::<GnoblinClaim>()
            ),
        );
        return 0;
    }
    // SAFETY: the caller guarantees the strings of `spec` are valid for the call.
    let (subject, device) = unsafe {
        match (
            read_text(spec.subject.ptr, spec.subject.len, "claim subject"),
            read_text(spec.device.ptr, spec.device.len, "claim device"),
        ) {
            (Ok(subject), Ok(device)) => (subject, device),
            (Err(error), _) | (_, Err(error)) => {
                set_code_error(error.code(), error.message());
                return 0;
            }
        }
    };
    let user_word = user as usize;
    let handler: Option<ClaimHandler> = cb.map(|cb| -> ClaimHandler {
        Arc::new(move |notice: &crate::events::ClaimNotice| {
            let raw = GnoblinClaimEvent {
                size: size_of::<GnoblinClaimEvent>() as u32,
                kind: notice.kind,
                claim: notice.claim,
                state: GnoblinStr::borrowed(&notice.state),
                text: GnoblinStr::borrowed(&notice.text),
                fingers: notice.fingers,
                button: notice.button,
                x: notice.x,
                y: notice.y,
                dx: notice.dx,
                dy: notice.dy,
                scale: notice.scale,
                rotation: notice.rotation,
                time: notice.time,
            };
            // SAFETY: `cb` and `user_word` came from one `claim` call; `raw` and the strings it points at are live for the call.
            let rc = unsafe { cb(user_word as *mut c_void, &raw const raw) };
            registry::outcome_of(rc)
        })
    });
    let spec = ClaimSpec {
        kind: spec.kind,
        flags: spec.flags,
        fingers: spec.fingers,
        subject: &subject,
        device: &device,
        node: spec.node,
    };
    match claim_with(ctx, seat, &spec, handler, user, drop_user) {
        Ok((_, reg)) => encode(KIND_REG, reg),
        Err((code, text)) => {
            set_code_error(code, text);
            0
        }
    }
}

// ---- timers -----------------------------------------------------------------------------------------------------------

/// `timer_add`.
pub(crate) fn timer_add(
    ctx: GnoblinCtx,
    ms: u32,
    repeat: u32,
    cb: GnoblinTimerFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    let Ok(owner) = registry::owner_of(ctx, "timer_add") else {
        return 0;
    };
    let repeating = repeat != 0;
    let timer = session::with(|session| session.add_timer(owner, ms, repeating));
    let reg = Arc::new(AtomicU64::new(0));
    let user_word = user as usize;
    let cleanup: Release = Box::new(move || session::with(|session| session.remove_timer(timer)));
    let own = Arc::clone(&reg);
    let subscription = Subscription {
        matches: Box::new(move |event: &ShellEvent| matches!(event, ShellEvent::Timer(fired) if *fired == timer)),
        call: Arc::new(move |_event: &ShellEvent| {
            // SAFETY: `cb` and `user_word` came from one `timer_add` call; the host calls it on the shell thread and does not free
            // `user` before `drop_user`.
            let rc = unsafe { cb(user_word as *mut c_void) };
            if !repeating {
                // A one-shot is done: its registration goes, and `drop_user` with it, once this callback returns.
                registry::remove_id(own.load(Ordering::Acquire));
            }
            registry::outcome_of(rc)
        }),
        cleanup: Some(cleanup),
    };
    match registry::subscribe(ctx, "timer_add", subscription, user, drop_user) {
        Ok(id) => {
            reg.store(id, Ordering::Release);
            registry::sync_wake();
            encode(KIND_REG, id)
        }
        Err(_) => {
            session::with(|session| session.remove_timer(timer));
            0
        }
    }
}
