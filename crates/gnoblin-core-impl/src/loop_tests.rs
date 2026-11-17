//! In-process tests of the two table functions that serve a shell with a loop of its own: `wake_fd`, and the verbs (`call`,
//! `subscribe`). They call the table as a shell would, from Rust test code, with `extern "C"` callbacks.
//!
//! The descriptor is tested the way it is used: a loop that sleeps in `poll` on it and calls `dispatch(ctx, 0)` when it is readable,
//! with other threads posting to it. The verbs are tested over a scripted link, which is what the compositor edge's owner-socket
//! link is to the core: the real one is proved against a real session (`crates/gnoblin/tests/native_shell_verbs.rs`).
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_core_abi::{self as abi, GnoblinAnswer, GnoblinCtx, GnoblinEvent, GnoblinStr, GnoblinValue};
use gnoblin_model::{Fields, Value};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;

use crate::host;
use crate::tests::{Gen, api, serial};
use crate::verbs::{self, VerbCall, VerbFailure, VerbLink, VerbLinkProvider};
use crate::{catalog, registry};

// ---- the wake descriptor ------------------------------------------------------------------------------------------------

fn wake_fd_of(generation: &Gen) -> OwnedFd {
    // SAFETY: `wake_fd` is a function of the core's table.
    let rc = unsafe { (api().wake_fd)(generation.ctx) };
    assert!(rc >= 0, "wake_fd failed with {rc}");
    // SAFETY: a non-negative result is a duplicate the caller owns.
    unsafe { OwnedFd::from_raw_fd(rc) }
}

fn readable(fd: &OwnedFd, wait: Duration) -> bool {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap(),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0)
}

fn dispatch(generation: &Gen) -> i32 {
    // SAFETY: `dispatch` is a function of the core's table.
    unsafe { (api().dispatch)(generation.ctx, 0) }
}

static POSTED: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn bump(_user: *mut c_void) {
    POSTED.fetch_add(1, Ordering::SeqCst);
}

fn post_from(ctx: GnoblinCtx) {
    // SAFETY: `post` is a function of the core's table, callable from any thread; `bump` takes no `user`.
    let rc = unsafe { (api().post)(ctx, bump, core::ptr::null_mut(), None) };
    assert_eq!(rc, abi::E_OK);
}

/// A generation whose queue is empty and whose descriptor is quiet, with its descriptor.
fn quiet() -> (Gen, OwnedFd) {
    let generation = Gen::active();
    let fd = wake_fd_of(&generation);
    // The set is the process's: what an earlier test left in it is drained by a dispatch, whose end disarms its timer.
    dispatch(&generation);
    assert!(
        !readable(&fd, Duration::from_millis(20)),
        "quiet before anything is queued"
    );
    (generation, fd)
}

#[test]
fn a_post_from_another_thread_wakes_the_descriptor_and_dispatch_quietens_it() {
    let _serial = serial();
    let (generation, fd) = quiet();
    let before = POSTED.load(Ordering::SeqCst);
    let ctx = generation.ctx;
    let poster = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        post_from(ctx);
    });
    assert!(
        readable(&fd, Duration::from_secs(5)),
        "a post from another thread wakes a loop that sleeps on the descriptor"
    );
    poster.join().unwrap();
    assert_eq!(dispatch(&generation), 1, "dispatch(0) runs the post");
    assert_eq!(POSTED.load(Ordering::SeqCst), before + 1);
    assert!(!readable(&fd, Duration::from_millis(20)), "and drains the descriptor");
}

#[test]
fn a_published_signal_wakes_the_descriptor() {
    let _serial = serial();
    let (generation, fd) = quiet();
    let seen = Arc::new(AtomicUsize::new(0));
    unsafe extern "C" fn counted(user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
        // SAFETY: `user` is the `AtomicUsize` the test keeps alive.
        unsafe { &*user.cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
        abi::E_OK
    }
    let class = catalog::class_id("lock").unwrap();
    let signal = catalog::signal_id(class, "requested").unwrap();
    // SAFETY: `on_signal` is a function of the core's table; `user` outlives the registration (the generation retires first).
    let reg = unsafe {
        (api().on_signal)(
            generation.ctx,
            0,
            class,
            signal,
            counted,
            Arc::as_ptr(&seen).cast_mut().cast(),
            None,
        )
    };
    assert_ne!(reg, 0);
    let publisher = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(30));
        host::publish_named("lock", "requested", 0, vec![]).unwrap();
    });
    assert!(readable(&fd, Duration::from_secs(5)), "a published signal wakes it");
    publisher.join().unwrap();
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(!readable(&fd, Duration::from_millis(20)));
}

#[test]
fn a_timer_wakes_the_descriptor_when_it_falls_due_and_not_before() {
    let _serial = serial();
    let (generation, fd) = quiet();
    static FIRED: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn fired(_user: *mut c_void) -> i32 {
        FIRED.fetch_add(1, Ordering::SeqCst);
        abi::E_OK
    }
    let started = Instant::now();
    // SAFETY: `timer_add` is a function of the core's table; the callback needs no `user`.
    let reg = unsafe { (api().timer_add)(generation.ctx, 60, 0, fired, core::ptr::null_mut(), None) };
    assert_ne!(reg, 0);
    assert!(
        !readable(&fd, Duration::from_millis(20)),
        "a timer that is not due does not wake the loop"
    );
    assert!(
        readable(&fd, Duration::from_secs(5)),
        "the timer wakes the loop when it falls due"
    );
    assert!(started.elapsed() >= Duration::from_millis(55), "and not before");
    assert_eq!(dispatch(&generation), 1, "dispatch(0) runs the timer");
    assert_eq!(FIRED.load(Ordering::SeqCst), 1);
    assert!(
        !readable(&fd, Duration::from_millis(100)),
        "a one-shot leaves the descriptor quiet"
    );
}

#[test]
fn a_poll_loop_serves_posts_from_four_threads_without_spinning() {
    let _serial = serial();
    let (generation, fd) = quiet();
    let before = POSTED.load(Ordering::SeqCst);
    let ctx = generation.ctx;
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(move || {
                for _ in 0..25 {
                    post_from(ctx);
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        })
        .collect();
    let (mut wakes, mut ran) = (0_usize, 0_usize);
    let deadline = Instant::now() + Duration::from_secs(20);
    while ran < 100 && Instant::now() < deadline {
        if readable(&fd, Duration::from_millis(500)) {
            wakes += 1;
            let n = dispatch(&generation);
            assert!(n >= 0, "dispatch failed with {n}");
            ran += usize::try_from(n).unwrap();
        }
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(ran, 100, "every post ran");
    assert_eq!(POSTED.load(Ordering::SeqCst), before + 100);
    assert!(
        wakes <= 100 + 1,
        "a wake does not repeat once drained: {wakes} wakes for 100 posts"
    );
    // A post that arrived while the last dispatch ran signalled again and was served by that dispatch: one spurious wake is
    // allowed, and one dispatch that runs nothing puts the loop to sleep.
    if readable(&fd, Duration::from_millis(20)) {
        assert_eq!(dispatch(&generation), 0, "a spurious wake runs nothing");
    }
    assert!(
        !readable(&fd, Duration::from_millis(50)),
        "the loop is quiet at the end: it sleeps"
    );
}

#[test]
fn a_retired_generation_is_refused_a_descriptor_and_a_live_one_may_ask_from_any_thread() {
    let _serial = serial();
    let generation = Gen::active();
    let ctx = generation.ctx;
    let from_another_thread = std::thread::spawn(move || {
        // SAFETY: `wake_fd` is a function of the core's table, callable from any thread.
        let rc = unsafe { (api().wake_fd)(ctx) };
        assert!(rc >= 0, "any thread may ask: {rc}");
        // SAFETY: a non-negative result is a descriptor the caller owns.
        drop(unsafe { OwnedFd::from_raw_fd(rc) });
    });
    from_another_thread.join().unwrap();
    registry::finish_retire(ctx);
    // SAFETY: as above.
    assert_eq!(unsafe { (api().wake_fd)(ctx) }, abi::E_RETIRED);
}

// ---- verbs --------------------------------------------------------------------------------------------------------------

/// How a scripted link answers a call: nothing, a value, or a refusal.
type Reply = Arc<dyn Fn(&VerbCall) -> Option<Result<Option<Value>, VerbFailure>> + Send + Sync>;

/// What the scripted link saw, and how it answers.
#[derive(Clone)]
struct Script {
    calls: Arc<Mutex<Vec<VerbCall>>>,
    /// SeenAnswers a call from a thread of its own, as the owner connection's thread does.
    reply: Reply,
    /// A link that has gone away refuses every call.
    gone: Arc<AtomicBool>,
}

impl Script {
    fn new(reply: impl Fn(&VerbCall) -> Option<Result<Option<Value>, VerbFailure>> + Send + Sync + 'static) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply: Arc::new(reply),
            gone: Arc::new(AtomicBool::new(false)),
        }
    }

    fn seen(&self) -> Vec<VerbCall> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn install(&self) {
        host::set_verb_link_provider(Box::new(self.clone()));
    }
}

impl VerbLinkProvider for Script {
    fn open(&self) -> Result<Box<dyn VerbLink>, String> {
        Ok(Box::new(self.clone()))
    }
}

impl VerbLink for Script {
    fn call(&mut self, call: VerbCall) -> Result<(), String> {
        if self.gone.load(Ordering::SeqCst) {
            return Err("the owner connection is gone".to_owned());
        }
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        if let Some(outcome) = (self.reply)(&call) {
            std::thread::spawn(move || verbs::answered(call.token, outcome));
        }
        Ok(())
    }
}

/// What an answer callback saw.
#[derive(Default)]
struct SeenAnswers {
    runs: AtomicUsize,
    drops: AtomicUsize,
    code: AtomicI32,
    result: Mutex<String>,
    error: Mutex<String>,
}

fn answers_of<'a>(user: *mut c_void) -> &'a SeenAnswers {
    // SAFETY: every test passes a pointer to an `SeenAnswers` that outlives the registration.
    unsafe { &*user.cast::<SeenAnswers>() }
}

unsafe extern "C" fn record_answer(user: *mut c_void, answer: *const GnoblinAnswer) -> i32 {
    let seen = answers_of(user);
    // SAFETY: the core passes a live answer; its strings are valid for the call.
    let answer = unsafe { &*answer };
    let text = |raw: &GnoblinStr| {
        if raw.ptr.is_null() {
            String::new()
        } else {
            // SAFETY: the core lends `len` bytes at `ptr` for the call.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) }).into_owned()
        }
    };
    seen.code.store(answer.code, Ordering::SeqCst);
    *seen.result.lock().unwrap() = text(&answer.result);
    *seen.error.lock().unwrap() = text(&answer.error);
    seen.runs.fetch_add(1, Ordering::SeqCst);
    abi::E_OK
}

unsafe extern "C" fn count_answer_drop(user: *mut c_void) {
    answers_of(user).drops.fetch_add(1, Ordering::SeqCst);
}

fn text_value(text: &str) -> GnoblinValue {
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind: abi::VALUE_TEXT,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: text.as_ptr().cast(),
        len: text.len(),
    }
}

fn verb(class: &str, method: &str) -> (u32, u32) {
    let class = catalog::class_id(class).unwrap_or_else(|| panic!("no class {class}"));
    let method = catalog::member(class, abi::MEMBER_METHOD, method).unwrap().id;
    (class, method)
}

/// Calls `class.method` with `args`, wanting the answer in `answers`; returns the table's code and the pending handle.
fn call(generation: &Gen, class: &str, method: &str, args: &[GnoblinValue], answers: &SeenAnswers) -> (i32, u64) {
    let (class, method) = verb(class, method);
    let mut pending = 0;
    // SAFETY: `call` is a function of the core's table; `args` and `pending` are live locals, and `answers` outlives the
    // registration (the generation retires first).
    let rc = unsafe {
        (api().call)(
            generation.ctx,
            0,
            class,
            method,
            args.as_ptr(),
            args.len(),
            Some(record_answer),
            core::ptr::from_ref(answers).cast_mut().cast(),
            Some(count_answer_drop),
            &raw mut pending,
        )
    };
    (rc, pending)
}

fn wait_dispatch(generation: &Gen) -> i32 {
    // SAFETY: `dispatch` is a function of the core's table.
    unsafe { (api().dispatch)(generation.ctx, 5000) }
}

#[test]
fn a_call_goes_out_by_the_schemas_names_and_its_answer_arrives_on_the_shell_thread() {
    let _serial = serial();
    let script = Script::new(|_| Some(Ok(None)));
    script.install();
    let generation = Gen::active();
    let answers = SeenAnswers::default();
    let (rc, pending) = call(
        &generation,
        "gnoblin.idle",
        "inhibit",
        &[text_value("the-test")],
        &answers,
    );
    assert_eq!(rc, abi::E_OK);
    assert_ne!(pending, 0, "a call with a callback has a handle to cancel it by");
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].class, "idle");
    assert_eq!(seen[0].method, "inhibit");
    assert_eq!(seen[0].object, 0, "no object named: the class's one");
    assert_eq!(seen[0].args.get("holder"), Some(&Value::Text("the-test".to_owned())));
    assert_eq!(
        answers.runs.load(Ordering::SeqCst),
        0,
        "the answer is delivered by dispatch, not at the call"
    );
    assert_eq!(wait_dispatch(&generation), 1);
    assert_eq!(answers.runs.load(Ordering::SeqCst), 1);
    assert_eq!(answers.code.load(Ordering::SeqCst), abi::E_OK);
    assert_eq!(*answers.result.lock().unwrap(), "", "a bare done has no value");
    assert_eq!(
        answers.drops.load(Ordering::SeqCst),
        1,
        "the answer is once: the registration goes with it"
    );
}

#[test]
fn a_refusal_is_the_compositors_code_and_a_value_is_json() {
    let _serial = serial();
    let script = Script::new(|call| {
        Some(match call.method.as_str() {
            "suspend" => Err(VerbFailure {
                code: "unsupported".to_owned(),
            }),
            _ => Ok(Some(Value::Text("a \"b\"".to_owned()))),
        })
    });
    script.install();
    let generation = Gen::active();
    let refused = SeenAnswers::default();
    assert_eq!(
        call(&generation, "gnoblin.session", "suspend", &[], &refused).0,
        abi::E_OK
    );
    assert_eq!(wait_dispatch(&generation), 1);
    assert_eq!(refused.code.load(Ordering::SeqCst), abi::E_REFUSED);
    assert_eq!(*refused.error.lock().unwrap(), "unsupported");
    let valued = SeenAnswers::default();
    assert_eq!(
        call(&generation, "gnoblin.session", "logout", &[], &valued).0,
        abi::E_OK
    );
    assert_eq!(wait_dispatch(&generation), 1);
    assert_eq!(valued.code.load(Ordering::SeqCst), abi::E_OK);
    assert_eq!(*valued.result.lock().unwrap(), "\"a \\\"b\\\"\"");
}

#[test]
fn a_call_that_is_refused_before_it_goes_out_has_not_adopted_the_callback() {
    let _serial = serial();
    let script = Script::new(|_| None);
    script.install();
    let generation = Gen::active();
    let answers = SeenAnswers::default();
    // Too many arguments, too few, and the wrong kind.
    assert_eq!(
        call(&generation, "gnoblin.session", "logout", &[text_value("x")], &answers).0,
        abi::E_TYPE
    );
    assert_eq!(
        call(&generation, "gnoblin.idle", "inhibit", &[], &answers).0,
        abi::E_TYPE
    );
    let number = GnoblinValue {
        kind: abi::VALUE_UINT,
        bits: 3,
        ..text_value("")
    };
    assert_eq!(
        call(&generation, "gnoblin.idle", "inhibit", &[number], &answers).0,
        abi::E_TYPE
    );
    // A class of the tree is `write_call`'s.
    let (rc, _) = {
        let class = catalog::class_id("window").unwrap();
        let mut pending = 0;
        // SAFETY: as in `call` above.
        let rc = unsafe {
            (api().call)(
                generation.ctx,
                0,
                class,
                1,
                core::ptr::null(),
                0,
                Some(record_answer),
                core::ptr::from_ref(&answers).cast_mut().cast(),
                Some(count_answer_drop),
                &raw mut pending,
            )
        };
        (rc, pending)
    };
    assert_eq!(rc, abi::E_NO_SUCH_MEMBER);
    assert!(script.seen().is_empty(), "nothing went out");
    assert_eq!(
        answers.drops.load(Ordering::SeqCst),
        0,
        "the core adopted nothing, so it owes no drop"
    );
    // A link that has gone fails the call itself.
    script.gone.store(true, Ordering::SeqCst);
    assert_eq!(
        call(&generation, "gnoblin.session", "logout", &[], &answers).0,
        abi::E_ERR
    );
    assert_eq!(answers.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn a_core_with_no_owner_connection_says_so_and_adopts_nothing() {
    let _serial = serial();
    // The session of a fresh thread has no provider: run the call on one.
    let outcome = std::thread::spawn(|| {
        let generation = Gen::active();
        let answers = SeenAnswers::default();
        let rc = call(&generation, "gnoblin.session", "logout", &[], &answers).0;
        let text = crate::guard::last_error();
        let (class, signal) = (
            catalog::class_id("gnoblin.session").unwrap(),
            catalog::signal_id(catalog::class_id("gnoblin.session").unwrap(), "bell").unwrap(),
        );
        unsafe extern "C" fn nothing(_user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
            abi::E_OK
        }
        // SAFETY: `subscribe` is a function of the core's table.
        let reg = unsafe { (api().subscribe)(generation.ctx, class, signal, nothing, core::ptr::null_mut(), None) };
        (rc, text, reg, answers.drops.load(Ordering::SeqCst))
    })
    .join()
    .unwrap();
    assert_eq!(outcome.0, abi::E_ERR);
    assert!(outcome.1.contains("owner connection"), "{}", outcome.1);
    assert_eq!(
        outcome.2, 0,
        "a subscription that can never be served is refused, not silent"
    );
    assert_eq!(outcome.3, 0);
}

#[test]
fn cancelling_a_pending_call_drops_its_callback_and_a_retired_generation_hears_no_answer() {
    let _serial = serial();
    // The link never answers by itself: the test answers, late.
    let script = Script::new(|_| None);
    script.install();
    let generation = Gen::active();
    let cancelled = SeenAnswers::default();
    let (rc, pending) = call(&generation, "gnoblin.idle", "inhibit", &[text_value("a")], &cancelled);
    assert_eq!(rc, abi::E_OK);
    // SAFETY: `cancel` is a function of the core's table.
    assert_eq!(unsafe { (api().cancel)(pending) }, abi::E_OK);
    assert_eq!(
        cancelled.drops.load(Ordering::SeqCst),
        1,
        "cancel drops the callback at once"
    );
    verbs::answered(script.seen()[0].token, Ok(None));
    assert_eq!(dispatch(&generation), 0, "the late answer reaches nobody");
    assert_eq!(cancelled.runs.load(Ordering::SeqCst), 0);

    let retired = SeenAnswers::default();
    let (rc, _) = call(&generation, "gnoblin.idle", "inhibit", &[text_value("b")], &retired);
    assert_eq!(rc, abi::E_OK);
    let token = script.seen()[1].token;
    registry::finish_retire(generation.ctx);
    assert_eq!(
        retired.drops.load(Ordering::SeqCst),
        1,
        "retiring drops what was pending"
    );
    verbs::answered(token, Ok(None));
    assert_eq!(
        retired.runs.load(Ordering::SeqCst),
        0,
        "and the callback is never invoked"
    );
}

#[test]
fn a_subscribed_signal_reaches_the_shell_with_its_payload_in_the_schemas_field_order() {
    let _serial = serial();
    let script = Script::new(|_| None);
    script.install();
    let generation = Gen::active();
    let class = catalog::class_id("gnoblin.session").unwrap();
    let signal = catalog::signal_id(class, "lock_state_changed").unwrap();
    // The catalog names the fields, in order, so a payload is typed by name.
    let member = catalog::member(class, abi::MEMBER_SIGNAL, "lock_state_changed").unwrap();
    // SAFETY: `arg_names` is static text the core owns.
    let names = unsafe { std::slice::from_raw_parts(member.arg_names.ptr, member.arg_names.len) };
    assert_eq!(std::str::from_utf8(names).unwrap(), "state revision");

    static NODE: AtomicU64 = AtomicU64::new(0);
    static REVISION: AtomicU64 = AtomicU64::new(0);
    static STATE: Mutex<String> = Mutex::new(String::new());
    unsafe extern "C" fn record(_user: *mut c_void, event: *const GnoblinEvent) -> i32 {
        // SAFETY: the core passes a live event with `n_values` values.
        let event = unsafe { &*event };
        assert_eq!(event.n_values, 2);
        // SAFETY: as above.
        let values = unsafe { std::slice::from_raw_parts(event.values, 2) };
        NODE.store(event.node, Ordering::SeqCst);
        assert_eq!(values[0].kind, abi::VALUE_TEXT);
        // SAFETY: a text value's pointer and length are valid for the callback.
        let state = unsafe { std::slice::from_raw_parts(values[0].ptr.cast::<u8>(), values[0].len) };
        *STATE.lock().unwrap() = String::from_utf8_lossy(state).into_owned();
        REVISION.store(values[1].bits, Ordering::SeqCst);
        abi::E_OK
    }
    // SAFETY: `subscribe` is a function of the core's table.
    let reg = unsafe { (api().subscribe)(generation.ctx, class, signal, record, core::ptr::null_mut(), None) };
    assert_ne!(reg, 0);
    let fields: Fields = [
        ("revision".to_owned(), Value::Uint(9)),
        ("state".to_owned(), Value::Text("locked".to_owned())),
    ]
    .into_iter()
    .collect();
    // From a thread that is not the shell's, as the owner connection's is.
    std::thread::spawn(move || verbs::signalled("session", "lock_state_changed", 4, &fields))
        .join()
        .unwrap();
    assert_eq!(wait_dispatch(&generation), 1);
    assert_eq!(NODE.load(Ordering::SeqCst), 4, "the object that raised it");
    assert_eq!(*STATE.lock().unwrap(), "locked");
    assert_eq!(REVISION.load(Ordering::SeqCst), 9);
    // A signal this core's catalog does not know is dropped, not delivered as another.
    verbs::signalled("session", "no_such_signal", 4, &Fields::new());
    verbs::signalled("no_such_class", "bell", 4, &Fields::new());
    // SAFETY: `dispatch` is a function of the core's table.
    assert_eq!(unsafe { (api().dispatch)(generation.ctx, 0) }, 0);
}
