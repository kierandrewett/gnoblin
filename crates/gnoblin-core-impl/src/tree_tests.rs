//! The tree, claim, timer, process and state functions of the table, called the way a shell calls them, over the fake compositor
//! of `gnoblin-testkit`.
//!
//! Everything runs on the test's own thread, which is the shell thread: the session is thread-local, the provider hands the core a
//! link to the fake, and the tests drive the fake from the same thread between `dispatch` calls. The host is process-wide, so
//! these take the same lock as the registry tests.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_core_abi::{
    self as abi, GnoblinAnswer, GnoblinBuf, GnoblinClaim, GnoblinClaimEvent, GnoblinCtx, GnoblinNodeEvent,
    GnoblinNodes, GnoblinPending, GnoblinProcessExit, GnoblinSpawn, GnoblinStr, GnoblinValue,
};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue, TreeLink};

use crate::host::{self, AttachMode, TreeLinkProvider};
use crate::tests::{Gen, api, serial};
use crate::{catalog, registry};

const WINDOW: u64 = 40;
const SEAT: u64 = 21;

struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

/// A shell over the fake: an active generation that has attached, a window and a seat in the tree, and a clock the test moves.
struct Rig {
    generation: Gen,
    comp: FakeCompositor,
    clock: ManualClock,
}

fn rig() -> Rig {
    host::reset_session();
    let comp = FakeCompositor::new();
    comp.add_window(WINDOW, "Terminal", "foot");
    comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
    host::set_tree_link_provider(Box::new(Provider(comp.clone())));
    let clock = ManualClock::default();
    host::set_clock(Arc::new(clock.clone()));
    let generation = Gen::active();
    // SAFETY: `tree_attach` is a function of the core's table.
    assert_eq!(
        unsafe { (api().tree_attach)(generation.ctx, abi::ATTACH_HOLDER) },
        abi::E_OK
    );
    Rig {
        generation,
        comp,
        clock,
    }
}

impl Rig {
    fn ctx(&self) -> GnoblinCtx {
        self.generation.ctx
    }

    fn dispatch(&self) -> i32 {
        // SAFETY: `dispatch` is a function of the core's table.
        unsafe { (api().dispatch)(self.ctx(), 0) }
    }

    /// Dispatches until `done` or two seconds pass: a call that must reach the compositor and come back crosses two turns.
    fn dispatch_until(&self, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "the condition never held");
            self.dispatch();
            thread::sleep(Duration::from_millis(1));
        }
    }
}

fn class(name: &str) -> u32 {
    catalog::class_id(name).unwrap_or_else(|| panic!("no class {name}"))
}

fn property(class_name: &str, name: &str) -> u32 {
    catalog::property_id(class(class_name), name).unwrap_or_else(|| panic!("no property {class_name}.{name}"))
}

fn method(class_name: &str, name: &str) -> u32 {
    let class = catalog::class(class(class_name)).unwrap();
    class
        .methods
        .iter()
        .position(|m| m.name == name)
        .unwrap_or_else(|| panic!("no method {name}")) as u32
        + 1
}

fn blank_value() -> GnoblinValue {
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind: 0,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: core::ptr::null(),
        len: 0,
    }
}

fn double(value: f64) -> GnoblinValue {
    GnoblinValue {
        kind: abi::VALUE_DOUBLE,
        bits: value.to_bits(),
        ..blank_value()
    }
}

fn int(value: i64) -> GnoblinValue {
    GnoblinValue {
        kind: abi::VALUE_INT,
        bits: value as u64,
        ..blank_value()
    }
}

fn string(value: &str) -> GnoblinValue {
    GnoblinValue {
        kind: abi::VALUE_TEXT,
        ptr: value.as_ptr().cast(),
        len: value.len(),
        ..blank_value()
    }
}

fn get(ctx: GnoblinCtx, node: u64, class_name: &str, name: &str) -> (i32, GnoblinValue) {
    let mut out = blank_value();
    // SAFETY: `node_get` is a function of the core's table and `out` is a live local.
    let rc = unsafe { (api().node_get)(ctx, node, property(class_name, name), &raw mut out) };
    (rc, out)
}

fn text_of(value: &mut GnoblinValue) -> String {
    assert_eq!(value.kind, abi::VALUE_TEXT);
    let text = if value.len == 0 {
        String::new()
    } else {
        // SAFETY: the core lent `len` bytes at `ptr`.
        String::from_utf8(unsafe { std::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }.to_vec()).unwrap()
    };
    // SAFETY: `value_free` is a function of the core's table, given a value the core lent.
    assert_eq!(unsafe { (api().value_free)(value) }, abi::E_OK);
    text
}

fn double_of(value: &GnoblinValue) -> f64 {
    assert_eq!(value.kind, abi::VALUE_DOUBLE);
    f64::from_bits(value.bits)
}

fn set(ctx: GnoblinCtx, node: u64, class_name: &str, name: &str, value: &GnoblinValue, seconds: f64) -> i32 {
    // SAFETY: `write_set` is a function of the core's table and `value` is a live local.
    unsafe { (api().write_set)(ctx, node, property(class_name, name), value, seconds) }
}

fn last_error() -> String {
    crate::guard::last_error()
}

/// What a test callback saw, one line each.
struct Seen {
    lines: Mutex<Vec<String>>,
    drops: AtomicUsize,
    ctx: GnoblinCtx,
}

impl Seen {
    fn new(ctx: GnoblinCtx) -> Seen {
        Seen {
            lines: Mutex::new(Vec::new()),
            drops: AtomicUsize::new(0),
            ctx,
        }
    }

    fn user(&self) -> *mut c_void {
        (self as *const Seen).cast_mut().cast()
    }

    fn push(&self, line: String) {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).push(line);
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

fn seen_of<'a>(user: *mut c_void) -> &'a Seen {
    // SAFETY: every test passes a `Seen` that outlives the registration (it is dropped after the generation retires).
    unsafe { &*user.cast::<Seen>() }
}

unsafe extern "C" fn drop_seen(user: *mut c_void) {
    seen_of(user).drops.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn see_node(user: *mut c_void, event: *const GnoblinNodeEvent) -> i32 {
    let seen = seen_of(user);
    // SAFETY: the core passes a live event.
    let event = unsafe { &*event };
    let mut line = format!("node kind={} node={} class={}", event.kind, event.node, event.class);
    if event.kind == abi::NODE_CREATED {
        // The batch that made the node has been applied: its title reads as it will stay.
        let (rc, mut title) = get(seen.ctx, event.node, "window", "title");
        assert_eq!(rc, abi::E_OK);
        line.push_str(&format!(" title={}", text_of(&mut title)));
    }
    if event.kind == abi::NODE_CHANGED {
        // SAFETY: a change carries its new value.
        let value = unsafe { &*event.value };
        let shown = match value.kind {
            abi::VALUE_TEXT => String::from_utf8(
                // SAFETY: the value points at `len` bytes for the call.
                unsafe { std::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }.to_vec(),
            )
            .unwrap(),
            abi::VALUE_DOUBLE => f64::from_bits(value.bits).to_string(),
            other => format!("kind {other}"),
        };
        line.push_str(&format!(" property={} value={shown}", event.property));
    }
    seen.push(line);
    abi::E_OK
}

unsafe extern "C" fn see_answer(user: *mut c_void, answer: *const GnoblinAnswer) -> i32 {
    let seen = seen_of(user);
    // SAFETY: the core passes a live answer whose strings are valid for the call.
    let answer = unsafe { &*answer };
    let part = |value: &GnoblinStr| {
        if value.len == 0 {
            String::new()
        } else {
            // SAFETY: the core lends `len` bytes at `ptr`.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(value.ptr, value.len) }).into_owned()
        }
    };
    seen.push(format!(
        "answer code={} result={} error={}",
        answer.code,
        part(&answer.result),
        part(&answer.error)
    ));
    abi::E_OK
}

unsafe extern "C" fn see_claim(user: *mut c_void, event: *const GnoblinClaimEvent) -> i32 {
    let seen = seen_of(user);
    // SAFETY: the core passes a live event whose strings are valid for the call.
    let event = unsafe { &*event };
    let part = |value: &GnoblinStr| {
        if value.len == 0 {
            String::new()
        } else {
            // SAFETY: the core lends `len` bytes at `ptr`.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(value.ptr, value.len) }).into_owned()
        }
    };
    seen.push(format!(
        "claim kind={} id={} state={} text={} dx={}",
        event.kind,
        event.claim,
        part(&event.state),
        part(&event.text),
        event.dx
    ));
    abi::E_OK
}

unsafe extern "C" fn see_timer(user: *mut c_void) -> i32 {
    seen_of(user).push("timer".to_owned());
    abi::E_OK
}

unsafe extern "C" fn see_exit(user: *mut c_void, exit: *const GnoblinProcessExit) -> i32 {
    // SAFETY: the core passes a live exit whose strings are valid for the call.
    let exit = unsafe { &*exit };
    let stdout = if exit.stdout.len == 0 {
        String::new()
    } else {
        // SAFETY: the core lends `len` bytes at `ptr`.
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(exit.stdout.ptr, exit.stdout.len) }).into_owned()
    };
    seen_of(user).push(format!(
        "exit has_code={} code={} stdout={}",
        exit.has_code,
        exit.code,
        stdout.trim()
    ));
    abi::E_OK
}

// ---- reads and writes -------------------------------------------------------------------------------------------------------

#[test]
fn a_shell_reads_the_tree_and_a_property_that_was_never_set_reads_as_its_default() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    // SAFETY (the whole test): the functions called are the core's table's, and every pointer is a live local.
    unsafe {
        assert_eq!((api().node_exists)(ctx, WINDOW), 1);
        assert_eq!((api().node_exists)(ctx, 999), 0);
        assert_eq!((api().node_class)(ctx, WINDOW), class("window"));
        assert_eq!((api().node_class)(ctx, 999), 0);
        assert!(last_error().starts_with("E_NO_SUCH_NODE"), "{}", last_error());

        let mut children = GnoblinNodes {
            ptr: core::ptr::null(),
            len: 0,
        };
        assert_eq!((api().node_children)(ctx, ROOT, &raw mut children), abi::E_OK);
        assert_eq!(std::slice::from_raw_parts(children.ptr, children.len), [WINDOW, SEAT]);
        let mut parent = 7;
        assert_eq!((api().node_parent)(ctx, WINDOW, &raw mut parent), abi::E_OK);
        assert_eq!(parent, ROOT);

        let (rc, mut title) = get(ctx, WINDOW, "window", "title");
        assert_eq!(rc, abi::E_OK);
        assert_eq!(text_of(&mut title), "Terminal");
        // The schema's default, not zero: a window is fully opaque until something says otherwise.
        let (rc, opacity) = get(ctx, WINDOW, "window", "opacity");
        assert_eq!(rc, abi::E_OK);
        assert_eq!(double_of(&opacity), 1.0);
    }
}

#[test]
fn a_write_is_visible_to_a_read_at_once_and_leaves_as_one_transaction_at_the_end_of_the_turn() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    assert_eq!(set(ctx, WINDOW, "window", "x", &double(10.0), 0.0), abi::E_OK);
    assert_eq!(
        set(ctx, WINDOW, "window", "x", &int(20), 0.0),
        abi::E_OK,
        "an integer widens to a double"
    );
    assert_eq!(set(ctx, WINDOW, "window", "y", &double(5.0), 0.0), abi::E_OK);
    let (_, x) = get(ctx, WINDOW, "window", "x");
    assert_eq!(double_of(&x), 20.0, "read through the tick");
    assert_eq!(
        rig.comp.number(WINDOW, "x"),
        0.0,
        "the compositor has heard nothing yet"
    );

    rig.dispatch();
    assert_eq!(rig.comp.sent_count(), 1, "one transaction for the turn");
    assert_eq!(rig.comp.number(WINDOW, "x"), 20.0, "the last write of a property wins");
    assert_eq!(rig.comp.number(WINDOW, "y"), 5.0);
    rig.dispatch();
    assert_eq!(rig.comp.sent_count(), 1, "nothing more to send");
}

#[test]
fn a_write_the_schema_forbids_is_refused_before_it_is_sent_with_the_codes_the_abi_names() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    assert_eq!(
        set(ctx, WINDOW, "window", "title", &string("x"), 0.0),
        abi::E_REFUSED,
        "title is read-only"
    );
    assert!(last_error().contains("read-only"), "{}", last_error());
    assert_eq!(set(ctx, WINDOW, "window", "x", &string("wide"), 0.0), abi::E_TYPE);
    assert_eq!(set(ctx, 999, "window", "x", &double(1.0), 0.0), abi::E_NO_SUCH_NODE);
    // `x` is animated: a duration is accepted, and the transaction carries it.
    assert_eq!(set(ctx, WINDOW, "window", "x", &double(40.0), 0.25), abi::E_OK);
    rig.dispatch();
    assert_eq!(rig.comp.sent_count(), 1);
    assert_eq!(rig.comp.timed_sets(), 1);
    // A property that is not animated refuses a duration.
    assert_eq!(set(ctx, WINDOW, "window", "app_id", &string("a"), 0.25), abi::E_REFUSED);
    // Nothing the refused writes asked for was sent.
    rig.dispatch();
    assert_eq!(rig.comp.sent_count(), 1);
}

#[test]
fn a_node_made_by_the_shell_lands_in_the_compositor_and_can_be_destroyed() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let mut node = 0;
    // SAFETY: the functions called are the core's table's, and `node` is a live local.
    unsafe {
        assert_eq!((api().write_create)(ctx, class("canvas"), &raw mut node), abi::E_OK);
        assert_ne!(node, 0);
        assert_eq!(
            (api().node_class)(ctx, node),
            class("canvas"),
            "readable before it is sent"
        );
        assert_eq!((api().write_insert)(ctx, ROOT, node, -1), abi::E_OK);
        rig.dispatch();
        assert_eq!(rig.comp.children(ROOT).last(), Some(&node));
        rig.dispatch();
        assert_eq!((api().write_destroy)(ctx, node), abi::E_OK);
        assert_eq!((api().node_exists)(ctx, node), 0, "gone from reads at once");
        rig.dispatch();
        assert!(!rig.comp.children(ROOT).contains(&node));
        // A class outside the tree cannot be made.
        assert_eq!(
            (api().write_create)(ctx, class("gnoblin.session"), &raw mut node),
            abi::E_NO_SUCH_MEMBER
        );
    }
}

#[test]
fn a_stale_refusal_is_sent_again_and_the_write_lands() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    rig.comp.stale(1, None, WINDOW, "title");
    assert_eq!(set(ctx, WINDOW, "window", "x", &double(33.0), 0.0), abi::E_OK);
    rig.dispatch();
    assert_eq!(rig.comp.number(WINDOW, "x"), 0.0, "the first attempt was refused");
    rig.dispatch_until(|| rig.comp.number(WINDOW, "x") == 33.0);
    assert_eq!(rig.comp.sent_count(), 2, "sent once more on the next base");
}

// ---- events -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_window_appearing_and_a_title_changing_reach_the_callbacks_with_the_batch_applied() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    // SAFETY: the functions called are the core's table's; `seen` outlives the registrations.
    unsafe {
        let created = (api().on_node_created)(ctx, class("window"), see_node, seen.user(), Some(drop_seen));
        let changed = (api().on_change)(
            ctx,
            0,
            class("window"),
            property("window", "title"),
            see_node,
            seen.user(),
            Some(drop_seen),
        );
        let gone = (api().on_node_destroyed)(ctx, class("window"), see_node, seen.user(), Some(drop_seen));
        assert!(created != 0 && changed != 0 && gone != 0);
    }
    rig.comp.create_window(41, "Files", "nautilus");
    assert_eq!(rig.dispatch(), 1);
    rig.comp.external_set(41, "title", text("Files (2)"));
    rig.comp.external_set(41, "app_id", text("not watched"));
    assert_eq!(rig.dispatch(), 1, "only the title is watched");
    rig.comp.destroy(41);
    assert_eq!(rig.dispatch(), 1);
    let window = class("window");
    let title = property("window", "title");
    assert_eq!(
        seen.lines(),
        [
            format!("node kind=1 node=41 class={window} title=Files"),
            format!("node kind=3 node=41 class={window} property={title} value=Files (2)"),
            format!("node kind=2 node=41 class={window}"),
        ]
    );
}

#[test]
fn an_input_event_for_a_node_is_a_signal_of_the_tree() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let tree = class("tree");
    let event = catalog::signal_id(tree, "event").unwrap();
    // SAFETY: as above, with a signal callback.
    unsafe extern "C" fn see_event(user: *mut c_void, event: *const abi::GnoblinEvent) -> i32 {
        // SAFETY: the core passes a live event with `n_values` values.
        let event = unsafe { &*event };
        let values = unsafe { std::slice::from_raw_parts(event.values, event.n_values) };
        let kind = unsafe { std::slice::from_raw_parts(values[3].ptr.cast::<u8>(), values[3].len) };
        seen_of(user).push(format!(
            "event node={} kind={}",
            event.node,
            String::from_utf8_lossy(kind)
        ));
        abi::E_OK
    }
    // SAFETY: `on_signal` is a function of the core's table.
    let reg = unsafe { (api().on_signal)(ctx, WINDOW, tree, event, see_event, seen.user(), Some(drop_seen)) };
    assert_ne!(reg, 0);
    rig.comp.pointer(WINDOW, "press", 4.0, 5.0, 1);
    rig.comp.pointer(41, "press", 4.0, 5.0, 1);
    assert_eq!(rig.dispatch(), 1, "only the node asked for");
    assert_eq!(seen.lines(), ["event node=40 kind=press"]);
}

// ---- calls and flush --------------------------------------------------------------------------------------------------------

#[test]
fn a_call_is_answered_once_by_its_token_and_the_registration_goes_with_the_answer() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let mut pending: GnoblinPending = 0;
    // SAFETY: the functions called are the core's table's; every pointer is a live local.
    unsafe {
        assert_eq!(
            (api().write_call)(
                ctx,
                WINDOW,
                method("window", "close"),
                core::ptr::null(),
                0,
                Some(see_answer),
                seen.user(),
                Some(drop_seen),
                &raw mut pending,
            ),
            abi::E_OK
        );
        assert_ne!(pending, 0);
        rig.dispatch();
        assert_eq!(rig.comp.calls(), [(WINDOW, "close".to_owned())]);
        // The compositor answers under the token the core allocated: the first call of this shell.
        rig.comp.answer(1, "closed", "");
        assert_eq!(rig.dispatch(), 1);
        assert_eq!(seen.lines(), ["answer code=0 result=closed error="]);
        assert_eq!(
            seen.drops.load(Ordering::SeqCst),
            1,
            "a one-shot callback is dropped after it ran"
        );
        rig.comp.answer(1, "again", "");
        assert_eq!(rig.dispatch(), 0, "it comes once");
        assert_eq!((api().cancel)(pending), abi::E_STALE, "a spent pending call is stale");
    }
}

#[test]
fn a_call_that_the_compositor_refuses_is_answered_failed_and_a_cancelled_one_is_never_answered() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let mut refused: GnoblinPending = 0;
    let mut cancelled: GnoblinPending = 0;
    // SAFETY: as above.
    unsafe {
        let close = method("window", "close");
        let kill = method("window", "kill");
        let call = |m: u32, out: *mut GnoblinPending| {
            (api().write_call)(
                ctx,
                WINDOW,
                m,
                core::ptr::null(),
                0,
                Some(see_answer),
                seen.user(),
                Some(drop_seen),
                out,
            )
        };
        assert_eq!(call(close, &raw mut refused), abi::E_OK);
        assert_eq!(call(kill, &raw mut cancelled), abi::E_OK);
        assert_eq!((api().cancel)(cancelled), abi::E_OK);
        assert_eq!(
            seen.drops.load(Ordering::SeqCst),
            1,
            "cancelling drops the callback's user now"
        );
        rig.comp.fault(gnoblin_testkit::compositor::Fault::Refuse {
            code: "bad-value",
            failed_op: 0,
        });
        rig.dispatch();
        rig.dispatch_until(|| !seen.lines().is_empty());
        let lines = seen.lines();
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].starts_with(&format!("answer code={}", abi::E_REFUSED)),
            "{lines:?}"
        );
        assert!(lines[0].contains("bad-value"), "{lines:?}");
    }
}

#[test]
fn flush_tells_when_everything_sent_has_been_answered() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    assert_eq!(set(ctx, WINDOW, "window", "x", &double(9.0), 0.0), abi::E_OK);
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api().flush)(
                ctx,
                Some(see_answer),
                seen.user(),
                Some(drop_seen),
                core::ptr::null_mut()
            ),
            abi::E_OK
        );
    }
    assert_eq!(rig.comp.sent_count(), 1, "flush sends at once");
    rig.dispatch_until(|| !seen.lines().is_empty());
    let lines = seen.lines();
    assert!(lines[0].starts_with("answer code=0 result="), "{lines:?}");
    assert_eq!(rig.comp.number(WINDOW, "x"), 9.0);
}

// ---- input claims -----------------------------------------------------------------------------------------------------------

fn claim_spec(kind: u32, subject: &'static str, device: &'static str, fingers: i64, flags: u32) -> GnoblinClaim {
    GnoblinClaim {
        size: size_of::<GnoblinClaim>() as u32,
        kind,
        flags,
        reserved: 0,
        fingers,
        subject: GnoblinStr::borrowed(subject),
        device: GnoblinStr::borrowed(device),
        node: 0,
    }
}

#[test]
fn a_chord_claim_goes_to_the_seat_and_its_events_reach_the_callback_until_it_is_given_up() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let spec = claim_spec(abi::CLAIM_CHORD, "<Super>Return", "", 0, abi::CLAIM_FLAG_GRAB);
    // SAFETY: the functions called are the core's table's; `spec` and `seen` are live.
    let reg = unsafe {
        (api().claim)(
            ctx,
            SEAT,
            &raw const spec,
            Some(see_claim),
            seen.user(),
            Some(drop_seen),
        )
    };
    assert_ne!(reg, 0, "{}", last_error());
    rig.dispatch();
    assert_eq!(rig.comp.claimed(), ["<Super>Return"], "the claim is a call on the seat");
    let id = rig.comp.claim_id("<Super>Return");

    rig.comp.chord(id, "<Super>Return", "press");
    assert_eq!(rig.dispatch(), 1);
    rig.comp.chord(id + 100, "<Super>Return", "press");
    assert_eq!(rig.dispatch(), 0, "another claim's event is not ours");
    assert_eq!(
        seen.lines(),
        [format!("claim kind=1 id={id} state=press text=<Super>Return dx=0")]
    );

    // SAFETY: `remove` is a function of the core's table.
    assert_eq!(unsafe { (api().remove)(reg) }, abi::E_OK);
    assert_eq!(seen.drops.load(Ordering::SeqCst), 1);
    rig.dispatch();
    assert!(
        rig.comp.claimed().is_empty(),
        "giving the registration up gives the claim up"
    );
}

#[test]
fn gestures_pointer_chords_devices_and_grabs_are_claims_too_and_a_bad_one_is_refused() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let mut specs = vec![
        claim_spec(abi::CLAIM_GESTURE, "swipe", "", 3, 0),
        claim_spec(abi::CLAIM_CHORD, "<Super>Button1", "mouse0", 0, 0),
        claim_spec(abi::CLAIM_DEVICE, "", "kbd0", 0, 0),
        claim_spec(abi::CLAIM_POINTER, "", "pad0", 0, 0),
    ];
    let mut grab = claim_spec(abi::CLAIM_GRAB, "keyboard", "", 0, 0);
    grab.node = WINDOW;
    specs.push(grab);
    for spec in &specs {
        // SAFETY: as above.
        let reg = unsafe { (api().claim)(ctx, SEAT, spec, Some(see_claim), seen.user(), Some(drop_seen)) };
        assert_ne!(reg, 0, "{}", last_error());
    }
    rig.dispatch();
    assert_eq!(
        rig.comp.claimed(),
        ["swipe", "mouse0|<Super>Button1", "kbd0", "pad0", "keyboard"]
            .map(str::to_owned)
            .to_vec()
    );
    let gesture = rig.comp.gesture_claim("swipe", 3);
    rig.comp.gesture(gesture, "swipe", 3, "update", (4.0, 0.0), 1.0, 0.0);
    let chord = rig.comp.claim_id("mouse0\n<Super>Button1");
    rig.comp
        .pointer_chord(chord, "<Super>Button1", "start", (1.0, 1.0), (1.0, 1.0));
    rig.comp.pointer_button(rig.comp.claim_id("pad0"), 272, "press");
    assert_eq!(rig.dispatch(), 3);
    let lines = seen.lines();
    assert!(
        lines[0].starts_with(&format!("claim kind=2 id={gesture} state=update text=swipe dx=4")),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(&format!("claim kind=3 id={chord} state=start")),
        "{lines:?}"
    );
    assert!(
        lines[2].contains("kind=4") && lines[2].contains("state=press"),
        "{lines:?}"
    );

    // What the schema or the seat would refuse is refused here, with nothing sent.
    let bad = [
        claim_spec(abi::CLAIM_GESTURE, "swipe", "", 9, 0),
        claim_spec(abi::CLAIM_GESTURE, "wave", "", 3, 0),
        claim_spec(abi::CLAIM_DEVICE, "", "", 0, 0),
        claim_spec(abi::CLAIM_GRAB, "hand", "", 0, 0),
        claim_spec(abi::CLAIM_CHORD, "", "", 0, 0),
        claim_spec(99, "x", "", 0, 0),
    ];
    for spec in &bad {
        // SAFETY: as above.
        assert_eq!(
            unsafe { (api().claim)(ctx, SEAT, spec, None, core::ptr::null_mut(), None) },
            0
        );
    }
    let on_window = claim_spec(abi::CLAIM_CHORD, "a", "", 0, 0);
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api().claim)(ctx, WINDOW, &raw const on_window, None, core::ptr::null_mut(), None) },
        0
    );
    assert!(last_error().contains("not a seat"), "{}", last_error());
}

#[test]
fn a_claim_dies_with_its_generation_and_a_reload_hands_the_chord_over() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let spec = claim_spec(abi::CLAIM_CHORD, "<Super>q", "", 0, 0);
    // SAFETY: as above.
    unsafe {
        (api().claim)(
            ctx,
            SEAT,
            &raw const spec,
            Some(see_claim),
            seen.user(),
            Some(drop_seen),
        )
    };
    rig.dispatch();
    assert_eq!(rig.comp.claimed(), ["<Super>q"]);

    // The next generation is staged and claims the same chord: nothing is sent until it takes over, and then the old claim is
    // released and the new one made in the same transaction, so the compositor never sees the chord claimed twice.
    rig.comp.refuse_duplicate_claims();
    let next = Gen::staged();
    let next_seen = Seen::new(next.ctx);
    // SAFETY: as above.
    unsafe {
        (api().claim)(
            next.ctx,
            SEAT,
            &raw const spec,
            Some(see_claim),
            next_seen.user(),
            Some(drop_seen),
        )
    };
    rig.dispatch();
    assert_eq!(rig.comp.claimed(), ["<Super>q"], "the staged generation's claim waits");
    registry::activate(next.ctx).unwrap();
    // SAFETY: `dispatch` is a function of the core's table.
    unsafe { (api().dispatch)(next.ctx, 0) };
    assert_eq!(rig.comp.claimed(), ["<Super>q"], "released and made again, once");
    assert!(
        rig.comp.borrow().refused_claims.is_empty(),
        "{:?}",
        rig.comp.borrow().refused_claims
    );
    assert_eq!(
        rig.comp.borrow().claim_calls.len(),
        3,
        "claim, unclaim and the new claim"
    );
    // The old generation retires: its registration goes (its callback is dropped) and its chord is not released a second time.
    registry::finish_retire(ctx);
    assert_eq!(seen.drops.load(Ordering::SeqCst), 1);
    // SAFETY: as above.
    unsafe { (api().dispatch)(next.ctx, 0) };
    assert_eq!(rig.comp.claimed(), ["<Super>q"], "the new generation still holds it");
    registry::finish_retire(next.ctx);
}

// ---- timers -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_timer_fires_when_its_time_comes_a_repeating_one_again_and_a_removed_one_never() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let once = Seen::new(ctx);
    let again = Seen::new(ctx);
    let removed = Seen::new(ctx);
    // SAFETY: the functions called are the core's table's; the probes outlive the registrations.
    let (_once_reg, _again_reg, removed_reg) = unsafe {
        (
            (api().timer_add)(ctx, 100, 0, see_timer, once.user(), Some(drop_seen)),
            (api().timer_add)(ctx, 40, 1, see_timer, again.user(), Some(drop_seen)),
            (api().timer_add)(ctx, 10, 0, see_timer, removed.user(), Some(drop_seen)),
        )
    };
    assert!(_once_reg != 0 && _again_reg != 0 && removed_reg != 0);
    // SAFETY: `remove` is a function of the core's table.
    assert_eq!(unsafe { (api().remove)(removed_reg) }, abi::E_OK);
    assert_eq!(removed.drops.load(Ordering::SeqCst), 1);

    assert_eq!(rig.dispatch(), 0, "nothing is due at the start");
    rig.clock.advance(Duration::from_millis(40));
    assert_eq!(rig.dispatch(), 1);
    assert_eq!(again.lines(), ["timer"]);
    rig.clock.advance(Duration::from_millis(60));
    assert_eq!(rig.dispatch(), 2, "the one-shot and the repeating one");
    assert_eq!(once.lines(), ["timer"]);
    assert_eq!(
        once.drops.load(Ordering::SeqCst),
        1,
        "a one-shot's callback is dropped after it ran"
    );
    assert_eq!(again.lines().len(), 2);
    assert_eq!(again.drops.load(Ordering::SeqCst), 0);
    rig.clock.advance(Duration::from_secs(10));
    assert_eq!(
        rig.dispatch(),
        1,
        "only the repeating timer is left, and it fires once, not for every period it missed"
    );
    assert!(removed.lines().is_empty());
}

#[test]
fn a_dispatch_that_waits_wakes_for_a_timer_that_falls_due() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    // SAFETY: as above.
    unsafe { (api().timer_add)(ctx, 5, 0, see_timer, seen.user(), Some(drop_seen)) };
    let clock = rig.clock.clone();
    let mover = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        clock.advance(Duration::from_millis(50));
    });
    // SAFETY: `dispatch` is a function of the core's table.
    let called = unsafe { (api().dispatch)(ctx, 10_000) };
    assert_eq!(called, 1, "returned as soon as the timer was due, not at the deadline");
    mover.join().unwrap();
}

// ---- processes and state ----------------------------------------------------------------------------------------------------

fn argv<'a>(words: &'a [&'a str]) -> Vec<GnoblinStr> {
    words.iter().map(|word| GnoblinStr::borrowed(word)).collect()
}

#[test]
fn a_program_the_shell_starts_reports_how_it_ended_and_one_that_is_killed_says_so() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let words = argv(&["sh", "-c", "echo hello; exit 3"]);
    let options = GnoblinSpawn {
        size: size_of::<GnoblinSpawn>() as u32,
        stdout: abi::STREAM_CAPTURE,
        stderr: abi::STREAM_DISCARD,
        reserved: 0,
        cwd: GnoblinStr::EMPTY,
        env: core::ptr::null(),
        n_env: 0,
        stdin: GnoblinStr {
            ptr: core::ptr::null(),
            len: 0,
        },
        output_limit: 0,
        timeout_ms: 0.0,
    };
    let mut process = 0;
    // SAFETY: the functions called are the core's table's; every pointer is a live local.
    unsafe {
        assert_eq!(
            (api().spawn)(
                ctx,
                words.as_ptr(),
                words.len(),
                &raw const options,
                Some(see_exit),
                seen.user(),
                Some(drop_seen),
                &raw mut process
            ),
            abi::E_OK,
            "{}",
            last_error()
        );
    }
    assert_ne!(process, 0);
    rig.dispatch_until(|| !seen.lines().is_empty());
    assert_eq!(seen.lines(), ["exit has_code=1 code=3 stdout=hello"]);
    assert_eq!(seen.drops.load(Ordering::SeqCst), 1);
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api().proc_kill)(ctx, process, b"term".as_ptr(), 4),
            abi::E_STALE,
            "it has ended"
        );
        let sleeper = argv(&["sleep", "30"]);
        let mut running = 0;
        assert_eq!(
            (api().spawn)(
                ctx,
                sleeper.as_ptr(),
                2,
                core::ptr::null(),
                None,
                core::ptr::null_mut(),
                None,
                &raw mut running
            ),
            abi::E_OK
        );
        assert_eq!((api().proc_kill)(ctx, running, b"nonsense".as_ptr(), 8), abi::E_ERR);
        assert_eq!((api().proc_kill)(ctx, running, b"kill".as_ptr(), 4), abi::E_OK);
        let missing = argv(&["/no/such/program"]);
        assert_eq!(
            (api().spawn)(
                ctx,
                missing.as_ptr(),
                1,
                core::ptr::null(),
                None,
                core::ptr::null_mut(),
                None,
                core::ptr::null_mut()
            ),
            abi::E_ERR
        );
        assert!(last_error().contains("cannot start"), "{}", last_error());
    }
}

#[test]
fn a_program_a_staged_generation_starts_waits_for_the_takeover_and_one_it_abandons_never_runs() {
    let _serial = serial();
    host::reset_session();
    let marker = std::env::temp_dir().join(format!("gnoblin-core-spawn-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let command = format!("touch {}", marker.display());
    let words = argv(&["sh", "-c", &command]);
    let staged = Gen::staged();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api().spawn)(
                staged.ctx,
                words.as_ptr(),
                3,
                core::ptr::null(),
                None,
                core::ptr::null_mut(),
                None,
                core::ptr::null_mut()
            ),
            abi::E_OK
        );
    }
    thread::sleep(Duration::from_millis(150));
    assert!(!marker.exists(), "a staged generation's program has not started");
    registry::finish_retire(staged.ctx);
    thread::sleep(Duration::from_millis(150));
    assert!(!marker.exists(), "an abandoned generation's program never starts");

    let next = Gen::staged();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api().spawn)(
                next.ctx,
                words.as_ptr(),
                3,
                core::ptr::null(),
                None,
                core::ptr::null_mut(),
                None,
                core::ptr::null_mut()
            ),
            abi::E_OK
        );
    }
    registry::activate(next.ctx).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "the program started at the takeover");
        thread::sleep(Duration::from_millis(5));
    }
    let _ = std::fs::remove_file(&marker);
    registry::finish_retire(next.ctx);
}

#[test]
fn state_kept_by_one_generation_is_read_by_the_next_and_is_bounded() {
    let _serial = serial();
    host::reset_session();
    let first = Gen::active();
    let mut out = GnoblinBuf {
        ptr: core::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    let mut version = 0;
    // SAFETY: the functions called are the core's table's; every pointer is a live local.
    unsafe {
        assert_eq!(
            (api().state_get)(first.ctx, b"layout".as_ptr(), 6, &raw mut out, &raw mut version),
            abi::E_NO_SUCH_MEMBER
        );
        assert_eq!(
            (api().state_put)(first.ctx, b"layout".as_ptr(), 6, b"tiles=3".as_ptr(), 7, 2),
            abi::E_OK
        );
    }
    registry::finish_retire(first.ctx);
    let second = Gen::active();
    // SAFETY: as above.
    unsafe {
        assert_eq!(
            (api().state_get)(second.ctx, b"layout".as_ptr(), 6, &raw mut out, &raw mut version),
            abi::E_OK
        );
        assert_eq!(std::slice::from_raw_parts(out.ptr, out.len), b"tiles=3");
        assert_eq!(version, 2);
        assert_eq!((api().buf_free)(&raw mut out), abi::E_OK);
        let too_big = vec![0u8; abi::STATE_MAX_BYTES as usize];
        assert_eq!(
            (api().state_put)(second.ctx, b"big".as_ptr(), 3, too_big.as_ptr(), too_big.len(), 1),
            abi::E_ERR
        );
        assert!(last_error().contains("at most"), "{}", last_error());
    }
}

// ---- the lifetime rules, for the new families -------------------------------------------------------------------------------

#[test]
fn every_registering_function_is_owned_by_its_generation_and_dropped_exactly_once_when_it_retires() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let seen = Seen::new(ctx);
    let spec = claim_spec(abi::CLAIM_CHORD, "<Super>x", "", 0, 0);
    // SAFETY: the functions called are the core's table's; `seen` outlives the registrations.
    unsafe {
        (api().on_node_created)(ctx, 0, see_node, seen.user(), Some(drop_seen));
        (api().on_change)(ctx, WINDOW, class("window"), 0, see_node, seen.user(), Some(drop_seen));
        (api().on_node_destroyed)(ctx, 0, see_node, seen.user(), Some(drop_seen));
        (api().claim)(
            ctx,
            SEAT,
            &raw const spec,
            Some(see_claim),
            seen.user(),
            Some(drop_seen),
        );
        (api().timer_add)(ctx, 10, 1, see_timer, seen.user(), Some(drop_seen));
        let mut pending = 0;
        (api().write_call)(
            ctx,
            WINDOW,
            method("window", "close"),
            core::ptr::null(),
            0,
            Some(see_answer),
            seen.user(),
            Some(drop_seen),
            &raw mut pending,
        );
        (api().flush)(
            ctx,
            Some(see_answer),
            seen.user(),
            Some(drop_seen),
            core::ptr::null_mut(),
        );
    }
    rig.dispatch();
    rig.dispatch();
    // The flush was answered, and a one-shot is dropped after it ran: the six others are still registered.
    assert_eq!(seen.drops.load(Ordering::SeqCst), 1);
    registry::finish_retire(ctx);
    assert_eq!(
        seen.drops.load(Ordering::SeqCst),
        7,
        "one `drop_user` for each registration, once"
    );
    // SAFETY: as above: every function of the new families refuses a retired generation and does nothing.
    unsafe {
        assert_eq!((api().node_exists)(ctx, WINDOW), abi::E_RETIRED);
        assert_eq!(set(ctx, WINDOW, "window", "x", &double(1.0), 0.0), abi::E_RETIRED);
        assert_eq!((api().tree_attach)(ctx, abi::ATTACH_HOLDER), abi::E_RETIRED);
        assert_eq!((api().timer_add)(ctx, 1, 0, see_timer, seen.user(), Some(drop_seen)), 0);
        assert!(last_error().starts_with("E_RETIRED"), "{}", last_error());
        assert_eq!(
            (api().spawn)(
                ctx,
                argv(&["true"]).as_ptr(),
                1,
                core::ptr::null(),
                None,
                core::ptr::null_mut(),
                None,
                core::ptr::null_mut()
            ),
            abi::E_RETIRED
        );
        assert_eq!(
            (api().state_put)(ctx, b"k".as_ptr(), 1, b"v".as_ptr(), 1, 1),
            abi::E_RETIRED
        );
    }
    assert_eq!(
        seen.drops.load(Ordering::SeqCst),
        7,
        "a refused registration is not adopted, so nothing more is dropped"
    );
}

#[test]
fn a_shell_thread_function_called_from_another_thread_is_refused_and_does_nothing() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let outcome = thread::spawn(move || {
        // SAFETY: the functions called are the core's table's; the nil pointers are never read on this path.
        unsafe {
            let attach = (api().tree_attach)(ctx, abi::ATTACH_HOLDER);
            let exists = (api().node_exists)(ctx, 40);
            let timer = (api().timer_add)(ctx, 1, 0, see_timer, core::ptr::null_mut(), None);
            let one = double(1.0);
            let write = (api().write_set)(ctx, 40, 1, &raw const one, 0.0);
            (attach, exists, timer, write)
        }
    })
    .join()
    .unwrap();
    assert_eq!(outcome, (abi::E_THREAD, abi::E_THREAD, 0, abi::E_THREAD));
    // `log` and `post` are the any-thread functions, and still work.
    let (post, log) = thread::spawn(move || {
        // SAFETY: as above.
        unsafe {
            (
                (api().log)(ctx, abi::LOG_INFO, b"from elsewhere".as_ptr(), 14),
                (api().report_panic)(ctx, b"p".as_ptr(), 1),
            )
        }
    })
    .join()
    .unwrap();
    assert_eq!((post, log), (abi::E_OK, abi::E_OK));
}

#[test]
fn a_tree_function_before_the_shell_has_attached_says_so() {
    let _serial = serial();
    host::reset_session();
    let generation = Gen::active();
    // SAFETY: as above.
    unsafe {
        assert_eq!((api().node_exists)(generation.ctx, 1), abi::E_ERR);
        assert!(last_error().contains("tree_attach"), "{}", last_error());
        // Without a provider the attach itself fails, and says why.
        assert_eq!((api().tree_attach)(generation.ctx, abi::ATTACH_HOLDER), abi::E_ERR);
        assert!(last_error().contains("no link"), "{}", last_error());
        assert_eq!((api().tree_attach)(generation.ctx, 99), abi::E_ERR);
    }
}

#[test]
fn a_catalog_class_has_a_name_and_a_value_the_abi_does_not_carry_is_unsupported() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let mut name = GnoblinStr::EMPTY;
    // SAFETY: as above.
    unsafe {
        assert_eq!((api().class_name)(ctx, class("window"), &raw mut name), abi::E_OK);
        assert_eq!(std::slice::from_raw_parts(name.ptr, name.len), b"window");
        assert_eq!(
            (api().class_name)(ctx, class("gnoblin.session"), &raw mut name),
            abi::E_OK
        );
        assert_eq!(std::slice::from_raw_parts(name.ptr, name.len), b"gnoblin.session");
        assert_eq!((api().class_name)(ctx, 0, &raw mut name), abi::E_NO_SUCH_MEMBER);
    }
    // A value held as effects reads as unsupported, not as something else.
    let effects = property("window", "effects");
    rig.comp
        .external_set(WINDOW, "effects", MirrorValue::Effects(Vec::new()));
    rig.dispatch();
    let mut out = blank_value();
    // SAFETY: as above.
    assert_eq!(
        unsafe { (api().node_get)(ctx, WINDOW, effects, &raw mut out) },
        abi::E_UNSUPPORTED
    );
}
