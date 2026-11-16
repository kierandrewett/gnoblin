//! In-process tests of the registry, the table and the exports.
//!
//! These call the function table the way a shell would, from Rust test code, with `extern "C"` callbacks. The registry is
//! one per process, so the tests take a lock and each retires its contexts when it ends; the dlopen-based scenarios live in
//! `tests/scenarios.rs`.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;

use gnoblin_core_abi::{
    self as abi, GnoblinApi, GnoblinBuf, GnoblinCtx, GnoblinCtxInfo, GnoblinEvent, GnoblinMember, GnoblinStats,
};

use crate::guard::last_error;
use crate::host::{self, GenerationState, OwnedValue, SignalDelivery};
use crate::{catalog, registry};

/// The registry is process-wide; one test at a time.
pub(crate) fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A context that retires itself when the test ends, even by panic.
pub(crate) struct Gen {
    pub(crate) ctx: GnoblinCtx,
}

impl Gen {
    pub(crate) fn staged() -> Gen {
        Gen {
            ctx: registry::new_context("tests", abi::LANG_NATIVE, 0, false),
        }
    }

    pub(crate) fn active() -> Gen {
        let generation = Gen::staged();
        registry::activate(generation.ctx).unwrap();
        generation
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        registry::finish_retire(self.ctx);
    }
}

pub(crate) fn api() -> &'static GnoblinApi {
    host::api()
}

/// What a test callback records.
#[derive(Default)]
struct TestProbe {
    calls: AtomicUsize,
    drops: AtomicUsize,
    /// The registration handle, for a callback that removes itself or another.
    reg: AtomicU64,
    other: AtomicU64,
    /// Drops observed from inside the callback, right after it removed its own registration.
    drops_seen_in_callback: AtomicUsize,
    /// The first payload field of the last event, as a uint.
    epoch: AtomicU64,
    node: AtomicU64,
}

fn probe_of<'a>(user: *mut c_void) -> &'a TestProbe {
    // SAFETY: every test passes a pointer to a `TestProbe` that outlives the registration (it is dropped after the context
    // retires).
    unsafe { &*user.cast::<TestProbe>() }
}

unsafe extern "C" fn count(user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
    probe_of(user).calls.fetch_add(1, Ordering::SeqCst);
    abi::E_OK
}

unsafe extern "C" fn record_payload(user: *mut c_void, event: *const GnoblinEvent) -> i32 {
    let probe = probe_of(user);
    // SAFETY: the core passes a live event.
    let event = unsafe { &*event };
    probe.calls.fetch_add(1, Ordering::SeqCst);
    probe.node.store(event.node, Ordering::SeqCst);
    if event.n_values > 0 {
        // SAFETY: `values` points at `n_values` values for the duration of the callback.
        let first = unsafe { &*event.values };
        probe.epoch.store(first.bits, Ordering::SeqCst);
    }
    abi::E_OK
}

unsafe extern "C" fn remove_self(user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
    let probe = probe_of(user);
    probe.calls.fetch_add(1, Ordering::SeqCst);
    // SAFETY: `remove` is a function of the core's table.
    let rc = unsafe { (api().remove)(probe.reg.load(Ordering::SeqCst)) };
    assert_eq!(rc, abi::E_OK);
    probe
        .drops_seen_in_callback
        .store(probe.drops.load(Ordering::SeqCst), Ordering::SeqCst);
    abi::E_OK
}

unsafe extern "C" fn remove_other(user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
    let probe = probe_of(user);
    probe.calls.fetch_add(1, Ordering::SeqCst);
    // SAFETY: `remove` is a function of the core's table.
    let rc = unsafe { (api().remove)(probe.other.load(Ordering::SeqCst)) };
    assert_eq!(rc, abi::E_OK);
    abi::E_OK
}

unsafe extern "C" fn report_panic_code(user: *mut c_void, _event: *const GnoblinEvent) -> i32 {
    probe_of(user).calls.fetch_add(1, Ordering::SeqCst);
    abi::E_PANIC
}

unsafe extern "C" fn count_drop(user: *mut c_void) {
    probe_of(user).drops.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn nothing(_user: *mut c_void) {}

fn ids(class: &str, signal: &str) -> (u32, u32) {
    let class = catalog::class_id(class).unwrap();
    (class, catalog::signal_id(class, signal).unwrap())
}

fn on(generation: &Gen, class: &str, signal: &str, cb: abi::GnoblinEventFn, probe: &TestProbe) -> u64 {
    let (class, signal) = ids(class, signal);
    let user = (probe as *const TestProbe).cast_mut().cast::<c_void>();
    // SAFETY: `on_signal` is a function of the core's table; `user` stays valid until the probe's drop runs.
    unsafe { (api().on_signal)(generation.ctx, 0, class, signal, cb, user, Some(count_drop)) }
}

fn publish(class: &str, signal: &str, values: Vec<OwnedValue>) {
    host::publish_named(class, signal, 0, values).unwrap();
}

fn dispatch(generation: &Gen) -> i32 {
    // SAFETY: `dispatch` is a function of the core's table.
    unsafe { (api().dispatch)(generation.ctx, 0) }
}

#[test]
fn an_event_reaches_a_registration_of_an_active_context() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    assert_ne!(on(&generation, "lock", "requested", count, &probe), 0);
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    // An event for another signal is consumed and reaches nobody.
    publish("tree", "attached", vec![]);
    assert_eq!(dispatch(&generation), 0);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn the_payload_and_node_reach_the_callback() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    assert_ne!(on(&generation, "tree", "attached", record_payload, &probe), 0);
    host::publish_named("tree", "attached", 42, vec![OwnedValue::Uint(7), OwnedValue::Node(3)]).unwrap();
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(probe.epoch.load(Ordering::SeqCst), 7);
    assert_eq!(probe.node.load(Ordering::SeqCst), 42);
}

#[test]
fn a_node_filter_selects_one_node() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    let (class, signal) = ids("tree", "attached");
    let user = (&raw const probe).cast_mut().cast::<c_void>();
    // SAFETY: as for `on`.
    let reg = unsafe { (api().on_signal)(generation.ctx, 9, class, signal, count, user, Some(count_drop)) };
    assert_ne!(reg, 0);
    host::publish_named("tree", "attached", 8, vec![]).unwrap();
    host::publish_named("tree", "attached", 9, vec![]).unwrap();
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_staged_context_receives_nothing_until_it_is_activated() {
    let _serial = serial();
    let active = Gen::active();
    let staged = Gen::staged();
    let probe = TestProbe::default();
    assert_ne!(on(&staged, "lock", "requested", count, &probe), 0);
    publish("lock", "requested", vec![]);
    assert_eq!(
        dispatch(&active),
        0,
        "the event is consumed by the active generation, which has no registration"
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    registry::activate(staged.ctx).unwrap();
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&staged), 1);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn drop_user_runs_exactly_once_per_registration() {
    let _serial = serial();
    let probe = TestProbe::default();
    {
        let generation = Gen::active();
        let first = on(&generation, "lock", "requested", count, &probe);
        let _second = on(&generation, "lock", "requested", count, &probe);
        let _third = on(&generation, "tree", "attached", count, &probe);
        // SAFETY: `remove` is a function of the core's table.
        assert_eq!(unsafe { (api().remove)(first) }, abi::E_OK);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1, "remove drops now");
        // SAFETY: as above; a second remove of the same handle is refused, not a second drop.
        assert_eq!(unsafe { (api().remove)(first) }, abi::E_STALE);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    }
    assert_eq!(
        probe.drops.load(Ordering::SeqCst),
        3,
        "retiring the context drops the other two"
    );
}

#[test]
fn removing_the_running_registration_defers_its_drop_until_the_callback_returns() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    let reg = on(&generation, "lock", "requested", remove_self, &probe);
    probe.reg.store(reg, Ordering::SeqCst);
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(
        probe.drops_seen_in_callback.load(Ordering::SeqCst),
        0,
        "the closure is still alive inside its own callback"
    );
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1, "and dropped once it returned");
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 0, "a removed registration is not called again");
}

#[test]
fn a_registration_removed_by_an_earlier_callback_is_not_called_for_the_same_event() {
    let _serial = serial();
    let generation = Gen::active();
    let first = TestProbe::default();
    let second = TestProbe::default();
    let first_reg = on(&generation, "lock", "requested", remove_other, &first);
    let second_reg = on(&generation, "lock", "requested", count, &second);
    assert_ne!(first_reg, 0);
    assert_ne!(second_reg, 0);
    first.other.store(second_reg, Ordering::SeqCst);
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(first.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    assert_eq!(second.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn callbacks_of_one_event_run_in_registration_order() {
    let _serial = serial();
    let generation = Gen::active();
    static ORDER: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    unsafe extern "C" fn first(_: *mut c_void, _: *const GnoblinEvent) -> i32 {
        ORDER.lock().unwrap().push(1);
        abi::E_OK
    }
    unsafe extern "C" fn second(_: *mut c_void, _: *const GnoblinEvent) -> i32 {
        ORDER.lock().unwrap().push(2);
        abi::E_OK
    }
    ORDER.lock().unwrap().clear();
    let probe = TestProbe::default();
    assert_ne!(on(&generation, "lock", "requested", first, &probe), 0);
    assert_ne!(on(&generation, "lock", "requested", second, &probe), 0);
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 2);
    assert_eq!(*ORDER.lock().unwrap(), [1, 2]);
}

#[test]
fn a_retired_context_is_refused_and_every_refusal_is_counted() {
    let _serial = serial();
    let live = Gen::active();
    let retired = Gen::active();
    let probe = TestProbe::default();
    let reg = on(&retired, "lock", "requested", count, &probe);
    assert_ne!(reg, 0);
    let stats = |ctx| {
        let mut out = GnoblinStats {
            size: size_of::<GnoblinStats>() as u32,
            reserved: 0,
            registrations: 0,
            callbacks: 0,
            panics: 0,
            retired_generations: 0,
            stale_calls: 0,
        };
        // SAFETY: `stats` fills the struct it is given.
        assert_eq!(unsafe { (api().stats)(ctx, &raw mut out) }, abi::E_OK);
        out
    };
    let before = stats(live.ctx);
    assert_eq!(registry::finish_retire(retired.ctx), 1);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(registry::state_of(retired.ctx), Some(GenerationState::Retired));

    let (class, signal) = ids("lock", "requested");
    let user = (&raw const probe).cast_mut().cast::<c_void>();
    // SAFETY: as for `on`.
    let again = unsafe { (api().on_signal)(retired.ctx, 0, class, signal, count, user, Some(count_drop)) };
    assert_eq!(again, 0);
    assert!(last_error().starts_with("E_RETIRED: generation "), "{}", last_error());
    assert_eq!(
        probe.drops.load(Ordering::SeqCst),
        1,
        "a refused registration is not adopted, so not dropped either"
    );
    // SAFETY: `dispatch` is a function of the core's table.
    assert_eq!(unsafe { (api().dispatch)(retired.ctx, 0) }, abi::E_RETIRED);
    // SAFETY: `request_reload` is a function of the core's table.
    assert_eq!(unsafe { (api().request_reload)(retired.ctx) }, abi::E_RETIRED);
    assert!(
        !host::take_reload_request(),
        "a retired generation cannot ask for a reload"
    );
    assert_eq!(stats(live.ctx).stale_calls - before.stale_calls, 3);
    assert_eq!(stats(live.ctx).retired_generations - before.retired_generations, 1);
}

#[test]
fn a_callback_that_reports_a_panic_is_counted_and_the_next_event_is_still_delivered() {
    let _serial = serial();
    let generation = Gen::active();
    let boom = TestProbe::default();
    let after = TestProbe::default();
    assert_ne!(on(&generation, "tree", "attached", report_panic_code, &boom), 0);
    assert_ne!(on(&generation, "lock", "requested", count, &after), 0);
    let panics_before = crate::guard::PANICS.load(Ordering::SeqCst);
    publish("tree", "attached", vec![]);
    publish("lock", "requested", vec![]);
    assert_eq!(dispatch(&generation), 2);
    assert_eq!(boom.calls.load(Ordering::SeqCst), 1);
    assert_eq!(after.calls.load(Ordering::SeqCst), 1);
    assert!(crate::guard::PANICS.load(Ordering::SeqCst) > panics_before);
}

#[test]
fn shell_thread_functions_refuse_another_thread_and_any_thread_functions_work() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    let reg = on(&generation, "lock", "requested", count, &probe);
    let ctx = generation.ctx;
    let outcome = thread::spawn(move || {
        // SAFETY: functions of the core's table, called with live handles.
        unsafe {
            let dispatch = (api().dispatch)(ctx, 0);
            let remove = (api().remove)(reg);
            let (class, signal) = ids("lock", "requested");
            let register = (api().on_signal)(ctx, 0, class, signal, count, std::ptr::null_mut(), None);
            let thread_error = last_error();
            let log = (api().log)(ctx, abi::LOG_INFO, b"from another thread".as_ptr(), 19);
            let mut out = std::mem::zeroed::<GnoblinStats>();
            out.size = size_of::<GnoblinStats>() as u32;
            let stats = (api().stats)(ctx, &raw mut out);
            (dispatch, remove, register, thread_error, log, stats)
        }
    })
    .join()
    .unwrap();
    assert_eq!(outcome.0, abi::E_THREAD);
    assert_eq!(outcome.1, abi::E_THREAD);
    assert_eq!(outcome.2, 0);
    assert!(outcome.3.starts_with("E_THREAD: "), "{}", outcome.3);
    assert_eq!(outcome.4, abi::E_OK, "log is callable from any thread");
    assert_eq!(outcome.5, abi::E_OK, "stats is callable from any thread");
    assert_eq!(probe.drops.load(Ordering::SeqCst), 0, "the refused remove did not drop");
}

#[test]
fn work_posted_from_another_thread_runs_on_the_shell_thread_at_dispatch() {
    let _serial = serial();
    let generation = Gen::active();
    static RAN_ON: Mutex<Option<thread::ThreadId>> = Mutex::new(None);
    unsafe extern "C" fn work(_user: *mut c_void) {
        *RAN_ON.lock().unwrap() = Some(thread::current().id());
    }
    let ctx = generation.ctx;
    thread::spawn(move || {
        // SAFETY: `post` is a function of the core's table; the work function ignores its user pointer.
        assert_eq!(
            unsafe { (api().post)(ctx, work, std::ptr::null_mut(), None) },
            abi::E_OK
        );
    })
    .join()
    .unwrap();
    assert!(RAN_ON.lock().unwrap().is_none(), "posting does not run the work");
    assert_eq!(dispatch(&generation), 1);
    assert_eq!(*RAN_ON.lock().unwrap(), Some(thread::current().id()));
}

#[test]
fn posted_work_is_dropped_unrun_when_the_context_retires_first() {
    let _serial = serial();
    let probe = TestProbe::default();
    {
        let generation = Gen::active();
        let user = (&raw const probe).cast_mut().cast::<c_void>();
        // SAFETY: `post` is a function of the core's table; `user` outlives the context.
        assert_eq!(
            unsafe { (api().post)(generation.ctx, nothing, user, Some(count_drop)) },
            abi::E_OK
        );
    }
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn an_unknown_signal_is_refused_and_the_user_pointer_is_not_adopted() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    let class = catalog::class_id("lock").unwrap();
    let user = (&raw const probe).cast_mut().cast::<c_void>();
    // SAFETY: as for `on`.
    let reg = unsafe { (api().on_signal)(generation.ctx, 0, class, 9_999, count, user, Some(count_drop)) };
    assert_eq!(reg, 0);
    assert!(last_error().starts_with("E_NO_SUCH_MEMBER: "), "{}", last_error());
    drop(generation);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn shutdown_hooks_run_once_before_the_registrations_are_removed() {
    let _serial = serial();
    let generation = Gen::active();
    let probe = TestProbe::default();
    static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    unsafe extern "C" fn hook(user: *mut c_void) {
        let probe = probe_of(user);
        ORDER.lock().unwrap().push(if probe.drops.load(Ordering::SeqCst) == 0 {
            "hook before drop"
        } else {
            "hook after drop"
        });
    }
    ORDER.lock().unwrap().clear();
    let user = (&raw const probe).cast_mut().cast::<c_void>();
    // SAFETY: `on_shutdown` is a function of the core's table; `user` outlives the context.
    let reg = unsafe { (api().on_shutdown)(generation.ctx, hook, user, Some(count_drop)) };
    assert_ne!(reg, 0);
    registry::run_shutdown_hooks(generation.ctx);
    registry::run_shutdown_hooks(generation.ctx);
    assert_eq!(*ORDER.lock().unwrap(), ["hook before drop"], "a hook runs once");
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1, "and its drop_user runs after it");
}

#[test]
fn the_mailbox_is_bounded() {
    let _serial = serial();
    let generation = Gen::active();
    assert_eq!(dispatch(&generation), 0);
    for _ in 0..(65_536 + 10) {
        host::publish(SignalDelivery {
            class: 1,
            signal: 1,
            node: 0,
            values: Vec::new(),
        });
    }
    assert_eq!(host::queued_events(), 65_536);
    assert_eq!(dispatch(&generation), 0);
    assert_eq!(host::queued_events(), 0);
}

#[test]
fn ctx_info_follows_the_size_convention() {
    let _serial = serial();
    let generation = Gen::active();
    let mut full = unsafe { std::mem::zeroed::<GnoblinCtxInfo>() };
    full.size = size_of::<GnoblinCtxInfo>() as u32;
    // SAFETY: `ctx_info` fills the struct it is given.
    assert_eq!(unsafe { (api().ctx_info)(generation.ctx, &raw mut full) }, abi::E_OK);
    assert_eq!(full.language, abi::LANG_NATIVE);
    assert_eq!(full.generation, registry::generation_of(generation.ctx).unwrap());
    assert_eq!(full.module_path.len, "tests".len());

    // A caller built against a shorter layout (only the first 16 bytes) gets those and nothing past them.
    let mut short = unsafe { std::mem::zeroed::<GnoblinCtxInfo>() };
    short.size = 16;
    short.module_path.len = 0xdead;
    // SAFETY: as above; the core writes at most 16 bytes.
    assert_eq!(unsafe { (api().ctx_info)(generation.ctx, &raw mut short) }, abi::E_OK);
    assert_eq!(short.size, 16);
    assert_eq!(short.generation, full.generation);
    assert_eq!(
        short.module_path.len, 0xdead,
        "bytes past the caller's size are untouched"
    );

    // A struct too small to carry its own size is an ABI error.
    let mut tiny = unsafe { std::mem::zeroed::<GnoblinCtxInfo>() };
    tiny.size = 2;
    // SAFETY: as above.
    assert_eq!(unsafe { (api().ctx_info)(generation.ctx, &raw mut tiny) }, abi::E_ABI);
}

#[test]
fn the_catalog_answers_by_name_through_the_table() {
    let _serial = serial();
    let generation = Gen::active();
    let name = "window";
    // SAFETY: `catalog_class` reads `len` bytes at `name`.
    let class = unsafe { (api().catalog_class)(generation.ctx, name.as_ptr(), name.len()) };
    assert_ne!(class, 0);
    let missing = "no_such_class";
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api().catalog_class)(generation.ctx, missing.as_ptr(), missing.len()) },
        0
    );
    assert!(last_error().starts_with("E_NO_SUCH_MEMBER: "), "{}", last_error());

    let title = "title";
    let mut member = unsafe { std::mem::zeroed::<GnoblinMember>() };
    member.size = size_of::<GnoblinMember>() as u32;
    // SAFETY: as above; `catalog_member` fills the struct it is given.
    let rc = unsafe {
        (api().catalog_member)(
            generation.ctx,
            class,
            abi::MEMBER_PROPERTY,
            title.as_ptr(),
            title.len(),
            &raw mut member,
        )
    };
    assert_eq!(rc, abi::E_OK);
    assert_eq!(member.value_kind, abi::VALUE_TEXT);

    unsafe extern "C" fn collect(user: *mut c_void, member: *const GnoblinMember) -> i32 {
        // SAFETY: `user` is the `Vec` below and `member` is live for the call.
        unsafe { (*user.cast::<Vec<u32>>()).push((*member).id) };
        abi::E_OK
    }
    let mut ids = Vec::<u32>::new();
    // SAFETY: the callback only runs during the call.
    let rc = unsafe {
        (api().catalog_each)(
            generation.ctx,
            class,
            abi::MEMBER_PROPERTY,
            collect,
            (&raw mut ids).cast(),
        )
    };
    assert_eq!(rc, abi::E_OK);
    assert!(ids.len() > 5);
    assert_eq!(ids[0], 1);
}

#[test]
fn a_handle_from_nowhere_is_a_stale_context() {
    let _serial = serial();
    let mut out = unsafe { std::mem::zeroed::<GnoblinStats>() };
    out.size = size_of::<GnoblinStats>() as u32;
    // SAFETY: `stats` checks the handle before it writes anything.
    assert_eq!(unsafe { (api().stats)(0x1234, &raw mut out) }, abi::E_STALE);
}

#[test]
fn get_api_refuses_another_major_naming_both_versions_and_accepts_a_newer_minor() {
    let _serial = serial();
    let mut table: *const GnoblinApi = std::ptr::null();
    // SAFETY: `gnoblin_get_api` writes one pointer through `out`.
    let rc = unsafe { crate::gnoblin_get_api(2, 0, &raw mut table) };
    assert_eq!(rc, abi::E_ABI);
    assert!(table.is_null());
    let text = last_error();
    assert!(text.contains("ABI 2.0"), "{text}");
    assert!(
        text.contains(&format!("installed core is {}.{}", abi::ABI_MAJOR, abi::ABI_MINOR)),
        "{text}"
    );

    // SAFETY: as above.
    let rc = unsafe { crate::gnoblin_get_api(abi::ABI_MAJOR, 99, &raw mut table) };
    assert_eq!(rc, abi::E_OK);
    assert!(!table.is_null());
    // SAFETY: the pointer is the core's static table.
    let table = unsafe { &*table };
    assert_eq!(table.size as usize, size_of::<GnoblinApi>());
    assert_eq!(table.major, abi::ABI_MAJOR);
}

#[test]
fn last_error_reports_the_full_length_and_truncates_to_the_buffer() {
    let _serial = serial();
    crate::guard::set_error("a fairly long error text");
    // SAFETY: `gnoblin_last_error` writes at most `cap` bytes into the buffer.
    unsafe {
        assert_eq!(crate::gnoblin_last_error(std::ptr::null_mut(), 0), 24);
        let mut small = [0xffu8; 8];
        assert_eq!(crate::gnoblin_last_error(small.as_mut_ptr(), small.len()), 24);
        assert_eq!(small, *b"a fairl\0");
        let mut large = [0xffu8; 64];
        assert_eq!(crate::gnoblin_last_error(large.as_mut_ptr(), large.len()), 24);
        assert_eq!(&large[..25], b"a fairly long error text\0");
    }
}

#[test]
fn abi_version_and_catalog_hash_are_reported() {
    let (mut major, mut minor, mut patch) = (0, 0, 99);
    // SAFETY: three live locals.
    unsafe { crate::gnoblin_abi_version(&raw mut major, &raw mut minor, &raw mut patch) };
    assert_eq!((major, minor, patch), (abi::ABI_MAJOR, abi::ABI_MINOR, abi::ABI_PATCH));
    // SAFETY: null pointers are allowed and ignored.
    unsafe { crate::gnoblin_abi_version(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
    assert_eq!(crate::gnoblin_catalog_hash(), catalog::CATALOG_HASH);
    assert_eq!(host::catalog_hash(), catalog::CATALOG_HASH);
}

#[test]
fn the_function_table_matches_the_abi_crate() {
    let table = host::api();
    assert_eq!(table.size as usize, size_of::<GnoblinApi>());
    assert_eq!((table.major, table.minor), (abi::ABI_MAJOR, abi::ABI_MINOR));
    assert_eq!(table.reserved, 0);
}

#[test]
fn buf_free_through_the_table_refuses_what_the_core_did_not_lend() {
    let _serial = serial();
    let mut lent = host::lend_bytes(b"data".to_vec());
    // SAFETY: `buf_free` takes a buffer the core lent.
    assert_eq!(unsafe { (api().buf_free)(&raw mut lent) }, abi::E_OK);
    assert!(lent.ptr.is_null());
    let mut local = [0u8; 4];
    let mut forged = GnoblinBuf {
        ptr: local.as_mut_ptr(),
        len: 4,
        cap: 4,
    };
    // SAFETY: the core checks the address against what it lent before it frees.
    assert_eq!(unsafe { (api().buf_free)(&raw mut forged) }, abi::E_STALE);
    // SAFETY: null is allowed.
    assert_eq!(unsafe { (api().buf_free)(std::ptr::null_mut()) }, abi::E_OK);
}

#[test]
fn the_registry_is_empty_between_tests() {
    let _serial = serial();
    assert_eq!(registry::live_registrations(), 0, "a test leaked a registration");
}
