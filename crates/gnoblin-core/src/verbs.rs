//! Verbs and signals of the compositor's `gnoblin` namespace, and the descriptor of a shell that owns its event loop.
//!
//! The tree's nodes are read and written through [`Node`](crate::host::Node). The `gnoblin` namespace is what is not a node: the
//! session (`gnoblin.session`: log out, power off, the lock's state, the bell), the idle watch (`gnoblin.idle`), the outputs.
//! A **verb** is one call on such an object: [`Host::call_verb`] sends it and [`Host::call_verb_with`] also takes the
//! compositor's answer, on the shell thread, when the operation finishes. A **signal** is [`Host::subscribe`]: the callback runs
//! on the shell thread for every time an object of the class raises it, with the payload in the schema's field order (the
//! generated `fields` constants of each class name the positions: `gnoblin::session::fields::bell::COUNT`).
//!
//! An answer's bytes are the operation's value as JSON text (empty for a bare "done"); a refusal is the compositor's error code
//! (`denied`, `unsupported`, `busy`, `unavailable`).
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::os::fd::{FromRawFd as _, OwnedFd};

use gnoblin_core_abi::{self as abi, GnoblinPending};

use crate::error::{CoreError, Result, check};
use crate::gnoblin::{CallbackSlot, Host, Poster, Subscription, drop_slot, event_tramp};
use crate::owned::Value;
use crate::tree::{Answer, PendingCall, answer_tramp, with_raws};
use crate::value::{MemberKind, SignalEvent};

/// What `call_verb_inner` is given to run when the answer arrives.
type OnAnswer = Box<dyn FnOnce(Answer)>;

impl Host {
    /// Calls `method` of the one object of `class` (`gnoblin.session`, `gnoblin.idle`) on the compositor and does not wait for the
    /// answer. `args` are checked against the method's parameters, which are sent by their schema names.
    pub fn call_verb(&self, class: &str, method: &str, args: &[Value]) -> Result<()> {
        self.call_verb_inner(0, class, method, args, None).map(|_| ())
    }

    /// Calls `method` of the one object of `class` and runs `on_answer` once, on the shell thread, when the compositor answers
    /// (the operation's value as JSON in [`Answer::result`], or the compositor's error code in [`Answer::error`]). Cancel the wait
    /// with [`PendingCall::cancel`].
    pub fn call_verb_with<F>(&self, class: &str, method: &str, args: &[Value], on_answer: F) -> Result<PendingCall>
    where
        F: FnOnce(Answer) + 'static,
    {
        self.call_verb_inner(0, class, method, args, Some(Box::new(on_answer)))?
            .ok_or_else(|| {
                CoreError::new(
                    crate::host::ErrorKind::Failed,
                    "the core gave no handle for the pending call",
                )
            })
    }

    /// [`Host::call_verb_with`] on the object the compositor numbers `object`, for a class that has several (an output).
    pub fn call_verb_on<F>(
        &self,
        object: u64,
        class: &str,
        method: &str,
        args: &[Value],
        on_answer: F,
    ) -> Result<PendingCall>
    where
        F: FnOnce(Answer) + 'static,
    {
        self.call_verb_inner(object, class, method, args, Some(Box::new(on_answer)))?
            .ok_or_else(|| {
                CoreError::new(
                    crate::host::ErrorKind::Failed,
                    "the core gave no handle for the pending call",
                )
            })
    }

    fn call_verb_inner(
        &self,
        object: u64,
        class: &str,
        method: &str,
        args: &[Value],
        on_answer: Option<OnAnswer>,
    ) -> Result<Option<PendingCall>> {
        let class = self.class(class)?;
        let member = self.member(class, MemberKind::Method, method)?;
        let slot = on_answer.map(|f| Box::into_raw(Box::new(CallbackSlot { f: Some(f) })));
        let (cb, user, drop_user): (Option<abi::GnoblinAnswerFn>, *mut c_void, Option<abi::GnoblinDropFn>) = match slot
        {
            Some(slot) => (
                Some(answer_tramp),
                slot.cast(),
                Some(drop_slot::<CallbackSlot<Option<OnAnswer>>>),
            ),
            None => (None, core::ptr::null_mut(), None),
        };
        let mut pending: GnoblinPending = 0;
        let mut raws = Vec::with_capacity(args.len());
        let rc = with_raws(args, &mut raws, &mut |raws| {
            // SAFETY: `call` reads the arguments for the call; `user` and `drop_user` are the slot's, which the core adopts on
            // success.
            unsafe {
                (self.api().call)(
                    self.ctx(),
                    object,
                    class.0,
                    member.id,
                    raws.as_ptr(),
                    raws.len(),
                    cb,
                    user,
                    drop_user,
                    &mut pending,
                )
            }
        });
        if rc < abi::E_OK {
            if let Some(slot) = slot {
                // SAFETY: the core refused the call, so it has not adopted the slot and nothing else owns the box.
                drop(unsafe { Box::from_raw(slot) });
            }
            return Err(CoreError::from_code(rc));
        }
        if slot.is_some() {
            crate::gnoblin::REGISTERED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok((pending != 0).then(|| PendingCall::new(self.clone(), pending)))
    }

    /// Calls `f` every time an object of the compositor's `gnoblin` namespace raises `signal` of `class`
    /// (`gnoblin.session`, `bell`), on the shell thread, with the payload in the schema's field order. The first call connects to
    /// the compositor's owner socket, so a session that has none fails here and not silently later.
    pub fn subscribe<F>(&self, class: &str, signal: &str, f: F) -> Result<Subscription>
    where
        F: FnMut(&SignalEvent<'_>) + 'static,
    {
        let class = self.class(class)?;
        let signal = self.member(class, MemberKind::Signal, signal)?;
        let slot = Box::into_raw(Box::new(CallbackSlot { f }));
        // SAFETY: as for `on_signal`: `slot` is a leaked box the core hands back to `event_tramp::<F>` and
        // `drop_slot::<CallbackSlot<F>>`.
        let reg = unsafe {
            (self.api().subscribe)(
                self.ctx(),
                class.0,
                signal.id,
                event_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<CallbackSlot<F>>),
            )
        };
        self.adopt(reg, slot)
    }

    /// A descriptor that turns readable when [`Host::dispatch`] has something to do, for a shell that owns its event loop
    /// (tokio, calloop, GLib): poll it beside the loop's own sources and call `dispatch(0)` when it is readable, which drains it.
    ///
    /// It is level triggered and carries no data; one wake can be spurious, and then `dispatch(0)` returns `0`. It turns readable
    /// when a post or an event is queued (from any thread), when a timer is due and when the compositor's link has bytes to
    /// read. The descriptor is the caller's: it is closed when dropped.
    pub fn wake_fd(&self) -> Result<OwnedFd> {
        wake_fd(self.api(), self.ctx())
    }
}

impl Poster {
    /// The same descriptor as [`Host::wake_fd`], for a thread that is not the shell's: an event loop of its own may be started
    /// anywhere that holds a poster.
    pub fn wake_fd(&self) -> Result<OwnedFd> {
        wake_fd(self.api(), self.ctx())
    }
}

fn wake_fd(api: &'static abi::GnoblinApi, ctx: abi::GnoblinCtx) -> Result<OwnedFd> {
    // SAFETY: `wake_fd` takes only the context handle and may be called from any thread.
    let fd = unsafe { (api.wake_fd)(ctx) };
    check(fd)?;
    // SAFETY: the core returned a duplicate of its descriptor that nobody else owns: a non-negative result is a descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
