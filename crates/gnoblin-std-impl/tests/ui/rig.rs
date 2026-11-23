//! The rig the toolkit's replay tests share: the real core and the real `libgnoblin-std` in this process, over the fake compositor, with a
//! shell's side written as Rust over the object protocol (`Api`), and the recorder that turns what happened into the shape of an
//! oracle fixture.
//!
//! A scenario of `crates/gnoblin-js/tests/oracle/ui.rs` is a TypeScript shell that the TypeScript toolkit ran; the replays here do what
//! it did through the native classes and compare what they find with the fixture's:
//!
//! - each step's `log` (what the scenario recorded with `rec`) and the lines the toolkit reported (`diagnostics`);
//! - the shell's nodes, whole, after every step: the fixture's writes are folded into the state they leave (a node's class, its parent,
//!   its children in order, its properties), and the native run must leave the same;
//! - the snapshots `trees` and `paints`, with the paint lists' bytes.
//!
//! What a step wrote on the way is not compared. The TypeScript toolkit asked the compositor to measure text and drew an estimate
//! until the answer came, and drew again; the native toolkit measures at once and draws once. The measurer of this rig gives the
//! estimate for a text until `answer_measures` and the fixed rule after, as the oracle's compositor did, so the states agree at every
//! step; the writes between them are what O9 of the design removes.
#![allow(dead_code, unsafe_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinStr};
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_object::client::{Api, closure};
use gnoblin_object::{Obj, Val};
use gnoblin_paint::decode;
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, GnoblinStdApi};
use gnoblin_std_impl::ui_testing::{self, Measurer};
use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_testkit::oracle::{Fixtures, Map, Value, Writes, hex, json, number};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue, TreeLink};

/// The compositor's buttons.
pub const LEFT: u64 = 272;
pub const RIGHT: u64 = 273;
pub const MIDDLE: u64 = 274;

/// What the core opens when the shell attaches: a clone of the fake compositor.
struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

fn process_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn last_error() -> String {
    // SAFETY: with a null buffer and zero capacity the call only returns the length.
    let len = unsafe { gnoblin_std_impl::gnoblin_std_last_error(std::ptr::null_mut(), 0) };
    let mut buf = vec![0_u8; len + 1];
    // SAFETY: `buf` is writable for `len + 1` bytes.
    let written = unsafe { gnoblin_std_impl::gnoblin_std_last_error(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(written.min(len));
    String::from_utf8_lossy(&buf).into_owned()
}

// ---- text, as the oracle's compositor measured it ------------------------------------------------------------------------------

/// What the toolkit estimates a string at: half the size a character, a line and a half.
fn estimate(text: &str, size: f64) -> (u32, u32, u32) {
    let mut widest = 0.0_f64;
    let mut lines = 0.0_f64;
    for line in text.split('\n') {
        widest = widest.max(((line.chars().count() as f64) * size / 2.0).floor());
        lines += 1.0;
    }
    (
        widest as u32,
        (lines * (size * 3.0 / 2.0).floor()) as u32,
        ((size * 11.0) / 10.0).floor() as u32,
    )
}

/// The oracle's measurer: a text is estimated until `answer_measures` and then a font with a fixed advance measures it: 0.6 of the
/// size a glyph, 1.2 a line, 0.9 to the baseline, each rounded up.
#[derive(Default)]
pub struct Fixed {
    answered: RefCell<HashSet<String>>,
    asked: RefCell<Vec<String>>,
}

impl Fixed {
    fn key(font: &str, size: f64, text: &str) -> String {
        format!("{font}\u{1}{size}\u{1}{text}")
    }

    /// The compositor has answered nothing yet for what is made from now on: the TypeScript toolkit asked again for each text a new
    /// widget showed, and drew its estimate until the answer came. Widgets that measured already keep what they measured.
    pub fn unanswer(&self) {
        self.answered.borrow_mut().clear();
    }

    /// The compositor answers every measurement asked so far.
    pub fn answer(&self) {
        let asked = std::mem::take(&mut *self.asked.borrow_mut());
        self.answered.borrow_mut().extend(asked);
        ui_testing::measurements_changed();
    }
}

impl Measurer for Fixed {
    fn measure(&self, font: &str, size: f64, text: &str) -> Result<(u32, u32, u32), &'static str> {
        let key = Fixed::key(font, size, text);
        if !self.answered.borrow().contains(&key) {
            let mut asked = self.asked.borrow_mut();
            if !asked.contains(&key) {
                asked.push(key);
            }
            return Ok(estimate(text, size));
        }
        let widest = text.lines().map(|line| line.chars().count()).max().unwrap_or(0) as f64;
        let lines = text.lines().count().max(1) as f64;
        Ok((
            (widest * size * 0.6).ceil() as u32,
            (lines * size * 1.2).ceil() as u32,
            (size * 0.9).ceil() as u32,
        ))
    }
}

// ---- objects --------------------------------------------------------------------------------------------------------------------

/// An object of the toolkit, by what a shell holds: the protocol, its class and its handle.
#[derive(Clone, Copy)]
pub struct W {
    pub api: Api,
    pub class: u32,
    pub obj: Obj,
}

pub fn d(value: f64) -> Val {
    Val::Double(value)
}

pub fn t(value: &str) -> Val {
    Val::Text(value.to_owned())
}

pub fn b(value: bool) -> Val {
    Val::Bool(value)
}

/// A value of an enum, by its place (counted from one).
pub fn en(value: u32) -> Val {
    Val::Enum { class: 0, value }
}

pub const ROW: u32 = 1;
pub const COLUMN: u32 = 2;

/// The core's `Client` of the window `node` over the core's object protocol `core`: `gnoblin.clients.get(id)`.
pub fn client_of(core: Api, node: u64) -> Val {
    let service = core.class("gnoblin").expect("the core's root class");
    let root = core.create(service, &[]).expect("the core's root object");
    let clients = core
        .member(service, abi::MEMBER_PROPERTY, "clients")
        .and_then(|member| core.get(root, member))
        .and_then(|list| list.object())
        .expect("the core's list of clients");
    let class = core.class_of(clients).expect("the class of the list");
    let get = core.member(class, abi::MEMBER_METHOD, "get").expect("the list's `get`");
    match core.call(clients, get, &[Val::Text(node.to_string())]) {
        Ok(found @ Val::Object { .. }) => found,
        other => panic!("the core has no client {node}: {other:?}"),
    }
}

/// A number as a script's JSON shows it: a whole one without a fraction.
pub fn js(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

impl W {
    fn member(&self, kind: u32, name: &str) -> u32 {
        self.api
            .member(self.class, kind, name)
            .unwrap_or_else(|fail| panic!("class {} has no {name}: {fail}", self.class))
    }

    pub fn set(&self, name: &str, value: Val) {
        let member = self.member(abi::MEMBER_PROPERTY, name);
        self.api
            .set(self.obj, member, &value)
            .unwrap_or_else(|fail| panic!("set {name}: {fail}"));
    }

    pub fn try_set(&self, name: &str, value: Val) -> Result<(), gnoblin_object::Fail> {
        let member = self.member(abi::MEMBER_PROPERTY, name);
        self.api.set(self.obj, member, &value)
    }

    pub fn get(&self, name: &str) -> Val {
        let member = self.member(abi::MEMBER_PROPERTY, name);
        self.api
            .get(self.obj, member)
            .unwrap_or_else(|fail| panic!("get {name}: {fail}"))
    }

    pub fn call(&self, name: &str, args: &[Val]) -> Val {
        self.try_call(name, args)
            .unwrap_or_else(|fail| panic!("call {name}: {fail}"))
    }

    pub fn try_call(&self, name: &str, args: &[Val]) -> Result<Val, gnoblin_object::Fail> {
        let member = self.member(abi::MEMBER_METHOD, name);
        self.api.call(self.obj, member, args)
    }

    pub fn num(&self, name: &str) -> f64 {
        self.get(name).double().unwrap_or_else(|fail| panic!("{name}: {fail}"))
    }

    pub fn text(&self, name: &str) -> String {
        self.get(name)
            .text()
            .unwrap_or_else(|fail| panic!("{name}: {fail}"))
            .to_owned()
    }

    pub fn flag(&self, name: &str) -> bool {
        self.get(name).bool().unwrap_or_else(|fail| panic!("{name}: {fail}"))
    }

    pub fn node(&self) -> u64 {
        self.get("node").uint().expect("a node id")
    }

    /// The object as a value to pass to a call.
    pub fn val(&self) -> Val {
        Val::Object {
            class: self.class,
            obj: self.obj,
        }
    }

    /// `add`, which answers the canvas so calls chain, as the toolkit's did.
    pub fn add(&self, child: W) -> W {
        self.call("add", &[child.val()]);
        *self
    }

    pub fn mount_top(&self) -> W {
        self.call("mount_top", &[]);
        *self
    }

    pub fn focus(&self) {
        self.call("focus", &[]);
    }

    /// An object a property holds.
    pub fn obj(&self, name: &str) -> W {
        match self.get(name) {
            Val::Object { class, obj } => W {
                api: self.api,
                class,
                obj,
            },
            other => panic!("{name} is {other:?}, not an object"),
        }
    }

    /// The children, in order.
    pub fn kids(&self) -> Vec<W> {
        self.get("children")
            .list()
            .expect("a list of canvases")
            .iter()
            .map(|kid| match kid {
                Val::Object { class, obj } => W {
                    api: self.api,
                    class: *class,
                    obj: *obj,
                },
                other => panic!("a child that is {other:?}"),
            })
            .collect()
    }

    pub fn kid(&self, index: usize) -> W {
        self.kids()[index]
    }

    /// Ends the object and what it holds, as a shell's `destroy` does.
    pub fn destroy(&self) {
        self.api.destroy(self.obj).expect("the object ends");
    }
}

/// A node of a snapshot of a subtree: its id, class, parent and properties.
type Snapshot = (u64, String, u64, Vec<(String, MirrorValue)>);

/// A node of the shell's, as the compositor holds it: its id, the name the recorder gave it, class, parent, children and properties.
type Held = (u64, String, String, u64, Vec<u64>, Vec<(String, MirrorValue)>);

// ---- the rig --------------------------------------------------------------------------------------------------------------------

/// A place where the fixture holds a value that is an artifact of how the TypeScript toolkit measured text, and the value the native
/// toolkit holds instead (see [`Rig::allow`]).
struct Allowance {
    step: &'static str,
    node: &'static str,
    key: &'static str,
    fixture: f64,
    native: f64,
}

/// A place where the fixture holds a value of a node of the compositor's own that only the TypeScript engine's bookkeeping explains
/// (see [`Rig::allow_shared`]).
struct SharedAllowance {
    step: &'static str,
    node: u64,
    key: &'static str,
    fixture: Value,
    native: Value,
}

/// The shell's side and the recorder.
pub struct Rig {
    pub api: Api,
    /// The object protocol of `gnoblin.model`, the core's: the windows and what binds to them.
    pub core: Api,
    /// The table of the library, for what the TypeScript library reached through it and the object protocol does not serve yet (a
    /// transition, the desktop's workspaces).
    pub std: &'static GnoblinStdApi,
    /// Lines a shell's own runtime would report (a closure that threw): the replay's closures push them, as the TypeScript runtime
    /// reported its own.
    pub reports: Rc<RefCell<Vec<String>>>,
    pub ctx: GnoblinCtx,
    pub comp: FakeCompositor,
    pub clock: ManualClock,
    pub fixed: Rc<Fixed>,
    pub log: Rc<RefCell<Vec<Value>>>,
    writes: Writes,
    mark: usize,
    name: &'static str,
    steps: Vec<Value>,
    diagnostics: Vec<String>,
    expected: Vec<Value>,
    fold: Fold,
    allowances: Vec<Allowance>,
    /// How far a number of a step's log may be from the fixture's (zero: equal), for scenarios that compute numbers.
    tolerance: f64,
    /// Values of the compositor's own nodes that the fixture holds because of the TypeScript engine's own copy of a window's state.
    shared_allowances: Vec<SharedAllowance>,
    /// The labels of the log entries the next step does not compare, for what only a dynamically typed shell can say.
    skipped: Vec<&'static str>,
    /// Nodes the replay made to probe what the fixture's script held in its own objects: they are not compared.
    probes: Vec<u64>,
    forgotten: Vec<(&'static str, Vec<&'static str>)>,
    _lock: MutexGuard<'static, ()>,
}

/// The world a rig's compositor starts with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A desktop and windows of 800 by 600, as the oracle's scene scenarios have them.
    Sized,
    /// A desktop and windows as the fake compositor makes them (400 by 300), as the oracle's style scenarios have them.
    Plain,
    /// The session of `tests/common/policy.rs`: a screen, a seat, a lock and windows of 300 by 200 at (10 n, 20 n).
    Policy,
}

impl Rig {
    /// A generation of the core with the std library and a desktop of `width` by `height` and no window.
    pub fn new(name: &'static str, width: f64, height: f64) -> Rig {
        Rig::open(name, width, height, false, 0, Shape::Sized)
    }

    /// As [`Rig::new`] with `count` server-decorated windows of 800 by 600 (ids 3 on, titled `win1`, app id `foot`), as the oracle's
    /// scene scenarios have them.
    pub fn with_windows(name: &'static str, width: f64, height: f64, count: u64) -> Rig {
        Rig::open(name, width, height, false, count, Shape::Sized)
    }

    /// As [`Rig::with_windows`], the windows as the fake compositor makes them (400 by 300), as the oracle's style scenarios have them.
    pub fn with_plain_windows(name: &'static str, width: f64, height: f64, count: u64) -> Rig {
        Rig::open(name, width, height, false, count, Shape::Plain)
    }

    /// The oracle's policy session with `count` windows (ids 3 on): the world of the transition scenarios.
    pub fn with_policy_windows(name: &'static str, count: u64) -> Rig {
        Rig::open(name, 1000.0, 800.0, false, count, Shape::Policy)
    }

    /// A generation with no fixture behind it: a test of what the toolkit does on its own terms.
    pub fn scratch(width: f64, height: f64) -> Rig {
        Rig::open("scratch", width, height, true, 0, Shape::Sized)
    }

    /// A generation over the policy session (a screen and a seat) that records nothing: for a test that is not held to a fixture.
    pub fn scratch_policy(count: u64) -> Rig {
        Rig::open("scratch", 1000.0, 800.0, true, count, Shape::Policy)
    }

    /// As [`Rig::scratch`] with `count` windows, as [`Rig::with_windows`] makes them.
    pub fn scratch_with_windows(width: f64, height: f64, count: u64) -> Rig {
        Rig::open("scratch", width, height, true, count, Shape::Sized)
    }

    fn open(name: &'static str, width: f64, height: f64, scratch: bool, windows: u64, shape: Shape) -> Rig {
        let lock = process_lock();
        // SAFETY: the tests of this file run one at a time under `process_lock`, and nothing else reads the environment meanwhile.
        unsafe { std::env::set_var("TZ", "UTC") };
        host::reset_session();
        let comp = FakeCompositor::new();
        if shape == Shape::Policy {
            comp.add_screen(20, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
            comp.add_node(
                21,
                gnoblin_testkit::compositor::ROOT,
                "seat",
                vec![
                    ("name", gnoblin_testkit::compositor::text("seat0")),
                    ("pointer_x", MirrorValue::Double(0.0)),
                    ("pointer_y", MirrorValue::Double(0.0)),
                ],
            );
            comp.add_node(23, gnoblin_testkit::compositor::ROOT, "lock", Vec::new());
            for i in 1..=windows {
                comp.add_node(
                    2 + i,
                    gnoblin_testkit::compositor::ROOT,
                    "window",
                    vec![
                        ("title", gnoblin_testkit::compositor::text(&format!("win{i}"))),
                        ("app_id", gnoblin_testkit::compositor::text("foot")),
                        ("decoration", gnoblin_testkit::compositor::text("server")),
                        ("x", MirrorValue::Double(10.0 * i as f64)),
                        ("y", MirrorValue::Double(20.0 * i as f64)),
                        ("width", MirrorValue::Double(300.0)),
                        ("height", MirrorValue::Double(200.0)),
                    ],
                );
            }
        } else {
            comp.add_desktop(width, height);
            for n in 0..windows {
                comp.add_window_decorated(3 + n, &format!("win{}", n + 1), "foot", "server");
                if shape == Shape::Sized
                    && let Some(node) = comp.borrow_mut().nodes.get_mut(&(3 + n))
                {
                    node.props.insert("width".to_owned(), MirrorValue::Double(800.0));
                    node.props.insert("height".to_owned(), MirrorValue::Double(600.0));
                }
            }
        }
        comp.borrow_mut().refuse_every = 0;
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        let clock = ManualClock::default();
        host::set_clock(Arc::new(clock.clone()));
        let fixed = Rc::new(Fixed::default());
        ui_testing::set_measurer(Rc::clone(&fixed) as Rc<dyn Measurer>);
        let _ = ui_testing::take_diagnostics();
        let ctx = host::open_generation(name, abi::LANG_NATIVE).expect("a generation opens");
        let mut table: *const GnoblinStdApi = std::ptr::null();
        // SAFETY: `table` is a live local the library writes.
        let code = unsafe { gnoblin_std_impl::gnoblin_std_get_api(ABI_MAJOR, ABI_MINOR, &raw mut table) };
        assert_eq!(code, abi::E_OK);
        // SAFETY: the library's table is static.
        let table = unsafe { &*table };
        let mut raw: *const abi::GnoblinObjectApi = std::ptr::null();
        // SAFETY: `raw` is a live local the library writes.
        let code = unsafe { (table.object_api)(ctx, GnoblinStr::borrowed("gnoblin.std"), &raw mut raw) };
        assert_eq!(code, abi::E_OK, "{}", last_error());
        // SAFETY: the table `object_api` returns is static.
        let api = Api::new(unsafe { &*raw }, ctx, last_error);
        let mut core_raw: *const abi::GnoblinObjectApi = std::ptr::null();
        // SAFETY: `core_raw` is a live local the core writes.
        let code = unsafe { (host::api().object_api)(ctx, GnoblinStr::borrowed("gnoblin.model"), &raw mut core_raw) };
        assert_eq!(code, abi::E_OK, "{}", last_error());
        // SAFETY: the table `object_api` returns is static.
        let core = Api::new(unsafe { &*core_raw }, ctx, last_error);
        Rig {
            api,
            core,
            std: table,
            reports: Rc::new(RefCell::new(Vec::new())),
            ctx,
            comp,
            clock,
            fixed,
            log: Rc::new(RefCell::new(Vec::new())),
            writes: Writes::with_bytes(render_bytes),
            mark: 0,
            name,
            steps: Vec::new(),
            diagnostics: Vec::new(),
            expected: if scratch {
                Vec::new()
            } else {
                Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference").read_json(name)["steps"]
                    .as_array()
                    .expect("the fixture's steps")
                    .clone()
            },
            fold: Fold::default(),
            allowances: Vec::new(),
            tolerance: 0.0,
            shared_allowances: Vec::new(),
            skipped: Vec::new(),
            probes: Vec::new(),
            forgotten: Vec::new(),
            _lock: lock,
        }
    }

    /// The core's `Client` of the window `node`, as the value a shell passes to a member of the std library that takes a window. The
    /// handle carries a reference the rig never releases, so the client stays held for as long as the generation lives, as a shell's
    /// variable would hold it.
    pub fn client(&self, node: u64) -> Val {
        client_of(self.core, node)
    }

    /// One turn of the core: what is posted runs, the compositor's events are heard, the timers that are due fire.
    pub fn dispatch(&self) {
        // SAFETY: the core's table is static and `dispatch` takes plain values.
        unsafe { (host::api().dispatch)(self.ctx, 0) };
    }

    /// Runs turns until nothing is left to say: six, as the scenarios' frames were.
    pub fn settle(&mut self) {
        for _ in 0..6 {
            self.dispatch();
        }
        self.diagnostics.extend(ui_testing::take_diagnostics());
        self.diagnostics.extend(self.reports.borrow_mut().drain(..));
    }

    /// The lines the toolkit reported since the last call.
    pub fn reported(&mut self) -> Vec<String> {
        self.diagnostics.extend(ui_testing::take_diagnostics());
        self.diagnostics.extend(self.reports.borrow_mut().drain(..));
        std::mem::take(&mut self.diagnostics)
    }

    /// Moves the shell's clock on and settles.
    pub fn advance(&mut self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
        self.settle();
    }

    /// The compositor has answered nothing for the widgets made from now on (see [`Fixed::unanswer`]).
    pub fn unanswer_measures(&self) {
        self.fixed.unanswer();
    }

    /// The compositor answers every measurement asked.
    pub fn answer_measures(&mut self) {
        self.fixed.answer();
        self.settle();
    }

    /// Makes an object of `class` with `inits`.
    pub fn make(&self, class: &str, inits: &[(&str, Val)]) -> W {
        let class_id = self.api.class(class).expect("the class");
        let mut ids = Vec::new();
        for (name, value) in inits {
            let member = self
                .api
                .member(class_id, abi::MEMBER_PROPERTY, name)
                .unwrap_or_else(|fail| panic!("{class} has no {name}: {fail}"));
            ids.push((member, value.clone()));
        }
        let obj = self
            .api
            .create(class_id, &ids)
            .unwrap_or_else(|fail| panic!("a {class}: {fail}"));
        W {
            api: self.api,
            class: class_id,
            obj,
        }
    }

    /// An object of the interface `class` that the shell implements: the members it defines, by name, run `run` with their name. The
    /// library calls them from its turn.
    pub fn implement(
        &self,
        class: &str,
        members: &[&str],
        mut run: impl FnMut(&str, &[Val]) -> gnoblin_object::Res<Val> + 'static,
    ) -> Val {
        let class_id = self.api.class(class).expect("the interface");
        let ids: Vec<(u32, String)> = members
            .iter()
            .map(|name| {
                (
                    self.api
                        .member(class_id, abi::MEMBER_METHOD, name)
                        .unwrap_or_else(|fail| panic!("{class} has no {name}: {fail}")),
                    (*name).to_owned(),
                )
            })
            .collect();
        let defined: Vec<u32> = ids.iter().map(|(id, _)| *id).collect();
        let obj = self
            .api
            .implement(class_id, &defined, move |member, args| {
                let name = ids
                    .iter()
                    .find(|(id, _)| *id == member)
                    .map(|(_, name)| name.clone())
                    .expect("a member the shell defined");
                run(&name, args)
            })
            .expect("the shell implements the interface");
        Val::Object { class: class_id, obj }
    }

    /// A function of a class.
    pub fn function(&self, class: &str, name: &str, args: &[Val]) -> Val {
        let class = self.api.class(class).expect("the class");
        let member = self
            .api
            .member(class, abi::MEMBER_METHOD, name)
            .unwrap_or_else(|fail| panic!("{name}: {fail}"));
        self.api
            .call_static(class, member, args)
            .unwrap_or_else(|fail| panic!("{name}: {fail}"))
    }

    /// Says that at step `step` the fixture holds `fixture` for `key` of the compositor's node `node`, because the TypeScript engine
    /// wrote it back from a copy of its own (the rollback of a failed transition callback restores the engine's last copy of a
    /// window's state, which had not heard of the earlier writes), and that the native run holds `native`: what the failed callback
    /// left alone. The fixture's value must be exactly what is named, and from this step on the fixture's state holds the native
    /// value until a write of the fixture's own sets it again.
    pub fn allow_shared(&mut self, step: &'static str, node: u64, key: &'static str, fixture: Value, native: Value) {
        self.shared_allowances.push(SharedAllowance {
            step,
            node,
            key,
            fixture,
            native,
        });
    }

    /// The compositor changes a property of a node of its own, as a scenario's action does: what the shell's writes left of it is no
    /// longer what the compositor holds, and the fixture's state takes the new value.
    pub fn external(&mut self, id: u64, key: &str, value: MirrorValue) {
        let held = self.writes.value(&value);
        self.comp.external_set(id, key, value);
        self.fold.shared.insert(format!("{id}.{key}"), held);
    }

    /// The compositor destroys a node of its own: the shell's writes to it are gone with it.
    pub fn external_destroy(&mut self, id: u64) {
        self.comp.destroy(id);
        let prefix = format!("{id}.");
        self.fold.shared.retain(|key, _| !key.starts_with(&prefix));
    }

    /// The numbers of the next steps' logs may differ from the fixture's by `tolerance`: the scenario computes them.
    pub fn tolerate(&mut self, tolerance: f64) {
        self.tolerance = tolerance;
    }

    /// The next step does not compare the log entries with these labels: only a dynamically typed shell can fill them, and the
    /// test says why where it skips them.
    pub fn skip(&mut self, labels: &[&'static str]) {
        self.skipped.extend_from_slice(labels);
    }

    /// The node `node` is the replay's own probe of what the fixture's script held in an object of its own: it is not compared.
    pub fn probe(&mut self, node: u64) {
        self.probes.push(node);
    }

    /// The scenario started services the library does not have natively yet (the window decorations, the lock screen): the first `made`
    /// nodes it made, the nodes made in the steps `steps`, and the properties of the compositor's own nodes it set (`shared`: node and
    /// key) belong to the TypeScript run and to nothing the replay does. The fixture is read without them: their writes are dropped, and
    /// the nodes after them are numbered from the first, as the replay numbers its own.
    pub fn foreign(&mut self, made: usize, steps: &[&str], shared: &[(u64, &str)]) {
        let number = |name: &Value| {
            name.as_str()
                .and_then(|name| name.strip_prefix('n'))
                .and_then(|digits| digits.parse::<usize>().ok())
        };
        // Which of the fixture's nodes are foreign: the first `made`, and every one a listed step made.
        let mut foreign: Vec<usize> = (1..=made).collect();
        let mut seen = made;
        for step in &self.expected {
            let listed = steps.iter().any(|label| step["step"] == json!(label));
            for batch in step["writes"].as_array().into_iter().flatten() {
                for op in batch.as_array().into_iter().flatten() {
                    if op["op"] == "new"
                        && let Some(at) = number(&op["node"])
                    {
                        seen = seen.max(at);
                        if listed && at > made {
                            foreign.push(at);
                        }
                    }
                }
            }
        }
        let is_foreign = |name: &Value| number(name).is_some_and(|at| foreign.contains(&at));
        let foreign_op = |op: &Value| {
            is_foreign(&op["node"])
                || is_foreign(&op["parent"])
                || is_foreign(&op["value"]["node"])
                || (op["op"] == "set"
                    && shared
                        .iter()
                        .any(|(node, key)| op["node"] == json!(node) && op["key"] == json!(key)))
        };
        for step in &mut self.expected {
            if let Some(batches) = step["writes"].as_array_mut() {
                for batch in batches.iter_mut() {
                    if let Some(ops) = batch.as_array_mut() {
                        ops.retain(|op| !foreign_op(op));
                    }
                }
                batches.retain(|batch| batch.as_array().is_some_and(|ops| !ops.is_empty()));
            }
            renumber(step, &foreign);
        }
    }

    /// Records `value` under `label` in the step being made.
    pub fn rec(&self, label: &str, value: Value) {
        self.log.borrow_mut().push(json!({ "label": label, "value": value }));
    }

    // ---- the recorder ------------------------------------------------------------------------------------------------------

    /// Says that at step `step` the fixture holds `fixture` for `key` of the node `node` (the compositor's `nK` for the K-th canvas the
    /// shell made) because of how the TypeScript toolkit measured text, and that the native toolkit holds `native`. Nothing else may
    /// differ: the fixture's value must be exactly what is named, the native value is what the run must leave, and from this step on
    /// the fixture's state is taken to hold the native value until a write of the fixture's own sets it again.
    ///
    /// The one case is the entry's third hidden text. It asks the compositor to measure the empty text, is shown the old answer for it
    /// (it measured the empty text earlier) and not asked again, and then the answer to an older question reaches it, which it takes
    /// for the answer to the current one: the TypeScript toolkit draws an estimate for a step before it asks again. The native
    /// toolkit measures at once and has no old question to be answered.
    pub fn allow(&mut self, step: &'static str, node: &'static str, key: &'static str, fixture: f64, native: f64) {
        self.allowances.push(Allowance {
            step,
            node,
            key,
            fixture,
            native,
        });
    }

    /// Ends a step: what the shell logged and reported since the last one, and the shell's nodes as they stand, held against the
    /// fixture's: the log, what the toolkit reported, and the state the fixture's writes leave.
    pub fn step(&mut self, label: &str) {
        let index = self.steps.len();
        let want = self
            .expected
            .get(index)
            .unwrap_or_else(|| panic!("`{}` has no step number {index} (`{label}`)", self.name))
            .clone();
        assert_eq!(
            want["step"], label,
            "step number {index} of `{}` is `{}` in the fixture",
            self.name, want["step"]
        );
        let batches = self.comp.sent_since(self.mark);
        self.mark = self.comp.sent_count();
        let writes: Vec<Value> = batches
            .iter()
            .filter(|batch| !batch.is_empty())
            .map(|batch| Value::Array(self.writes.ops(batch)))
            .collect();
        let log = Value::Array(std::mem::take(&mut *self.log.borrow_mut()));
        let diagnostics = json!(std::mem::take(&mut self.diagnostics));
        let skipped = std::mem::take(&mut self.skipped);
        let wanted_log = Value::Array(
            want["log"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| !skipped.iter().any(|name| entry["label"] == json!(name)))
                .cloned()
                .collect(),
        );
        if self.tolerance > 0.0 {
            if let Some(difference) = close(&log, &wanted_log, self.tolerance) {
                panic!(
                    "step `{label}` of `{}`: the log: {difference}\nnative {log}\nfixture {wanted_log}",
                    self.name
                );
            }
        } else {
            assert_eq!(log, wanted_log, "step `{label}` of `{}`: the log", self.name);
        }
        compare_diagnostics(&diagnostics, &want["diagnostics"], label, self.name);
        self.fold.apply(&want["writes"]);
        for allowance in self.allowances.iter().filter(|held| held.step == label) {
            self.fold.override_number(allowance);
        }
        for allowance in self.shared_allowances.iter().filter(|held| held.step == label) {
            let key = format!("{}.{}", allowance.node, allowance.key);
            assert_eq!(
                self.fold.shared.get(&key),
                Some(&allowance.fixture),
                "the fixture's {key} is not what was allowed for"
            );
            self.fold.shared.insert(key, allowance.native.clone());
        }
        for (_, names) in self.forgotten.iter().filter(|(held, _)| *held == label) {
            for name in names {
                self.fold.nodes.remove(*name);
            }
        }
        let shared = self.fold.shared_keys();
        let actual = Value::Object(self.state(&shared).into_iter().collect());
        let wanted = Value::Object(self.fold.state().into_iter().collect());
        let same = if self.tolerance > 0.0 {
            close(&actual, &wanted, self.tolerance).is_none()
        } else {
            actual == wanted
        };
        assert!(
            same,
            "step `{label}` of `{}`: the shell's nodes differ:\n{}",
            self.name,
            differences(&actual, &wanted)
        );
        let mut step = Map::new();
        step.insert("step".to_owned(), json!(label));
        step.insert("writes".to_owned(), Value::Array(writes));
        self.steps.push(Value::Object(step));
    }

    fn attach(&mut self, key: &str, item: Value) {
        let step = self
            .steps
            .last_mut()
            .and_then(Value::as_object_mut)
            .expect("a snapshot follows a step");
        step.entry(key.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("a list")
            .push(item);
    }

    /// A snapshot of the subtree under the node `root`.
    pub fn tree(&mut self, label: &str, root: u64) {
        let nodes: Vec<Snapshot> = {
            let borrowed = self.comp.borrow();
            borrowed
                .subtree(root)
                .into_iter()
                .filter_map(|id| {
                    borrowed.nodes.get(&id).map(|node| {
                        (
                            id,
                            node.class.clone(),
                            node.parent,
                            node.props
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect(),
                        )
                    })
                })
                .collect()
        };
        let listed: Vec<Value> = nodes
            .iter()
            .map(|(id, class, parent, props)| {
                let mut map = Map::new();
                for (key, value) in props {
                    map.insert(key.clone(), self.writes.value(value));
                }
                json!({
                    "node": self.writes.node(*id),
                    "class": class,
                    "parent": self.writes.node(*parent),
                    "props": map,
                })
            })
            .collect();
        let item = json!({ "tree": label, "root": self.writes.node(root), "nodes": listed });
        self.attach("trees", item);
    }

    /// The paint lists of every canvas under the node `root`, in tree order, as the oracle holds them.
    pub fn paint(&mut self, label: &str, root: u64) {
        let canvases: Vec<(u64, f64, f64, f64, f64, Vec<u8>)> = {
            let borrowed = self.comp.borrow();
            borrowed
                .subtree(root)
                .into_iter()
                .filter_map(|id| {
                    let node = borrowed.nodes.get(&id)?;
                    let content = match node.props.get("content") {
                        Some(MirrorValue::Bytes(bytes)) => bytes.clone(),
                        _ => return None,
                    };
                    let double = |key: &str| match node.props.get(key) {
                        Some(MirrorValue::Double(value)) => *value,
                        _ => 0.0,
                    };
                    Some((id, double("x"), double("y"), double("width"), double("height"), content))
                })
                .collect()
        };
        let mut file = Vec::new();
        let mut listed = Vec::new();
        for (id, x, y, width, height, content) in &canvases {
            file.extend_from_slice(&(content.len() as u32).to_le_bytes());
            file.extend_from_slice(content);
            listed.push(json!({
                "node": self.writes.node(*id),
                "rect": [x, y, width, height],
                "bytes": content.len(),
                "paint": render_bytes(content)["paint"].clone(),
            }));
        }
        let name = format!("{}.{label}", self.name);
        let committed = Fixtures::new("crates/gnoblin-js/tests/fixtures/paint").read_bytes(&name);
        assert_eq!(
            hex(&file),
            hex(&committed),
            "the paint lists of `{name}` are not the bytes the TypeScript toolkit wrote"
        );
        let item = json!({ "paint": label, "file": format!("{name}.bin"), "canvases": listed });
        self.attach("paints", item);
    }

    /// The shell's nodes, whole, as the compositor holds them now: class, parent, children and properties by the names the recorder
    /// gave them. `shared` names the properties of the compositor's own nodes that the fixture's writes touched.
    fn state(&mut self, shared: &[(String, String)]) -> BTreeMap<String, Value> {
        let probes = self.probes.clone();
        let made: Vec<(u64, String)> = self
            .writes
            .names()
            .into_iter()
            .filter(|(id, _)| !probes.contains(id))
            .collect();
        let mut out = BTreeMap::new();
        let nodes: Vec<Held> = {
            let borrowed = self.comp.borrow();
            made.iter()
                .filter_map(|(id, name)| {
                    borrowed.nodes.get(id).map(|node| {
                        (
                            *id,
                            name.clone(),
                            node.class.clone(),
                            node.parent,
                            node.children.clone(),
                            node.props
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect(),
                        )
                    })
                })
                .collect()
        };
        for (_, name, class, parent, children, props) in nodes {
            let mut map = Map::new();
            for (key, value) in &props {
                map.insert(key.clone(), self.writes.value(value));
            }
            let parent = self.writes.node(parent);
            let children: Vec<Value> = children.iter().map(|id| self.writes.node(*id)).collect();
            out.insert(
                name,
                json!({ "class": class, "parent": parent, "children": children, "props": map }),
            );
        }
        for (node, key) in shared {
            let id: u64 = node.parse().expect("a compositor node");
            let value = self.comp.value(id, key).map(|value| self.writes.value(&value));
            out.insert(format!("{node}.{key}"), value.unwrap_or(Value::Null));
        }
        out
    }

    /// At step `step` the fixture still holds the shell's canvases `names`, which the compositor took with the window it destroyed: the
    /// TypeScript oracle's record of writes has no way to say so, and the native run does not hold them.
    pub fn forget(&mut self, step: &'static str, names: &[&'static str]) {
        self.forgotten.push((step, names.to_vec()));
    }

    /// The replay stops after the fixture's step `label`: what the fixture holds of the steps that follow is not replayed, and the test
    /// says why where it calls this.
    pub fn stop_after(&mut self, label: &str) {
        let at = self
            .expected
            .iter()
            .position(|step| step["step"] == label)
            .unwrap_or_else(|| panic!("`{}` has no step `{label}`", self.name));
        self.expected.truncate(at + 1);
    }

    /// Compares the snapshots of every step with the fixture's, and that no step is missing.
    pub fn finish(mut self) {
        assert_eq!(
            self.steps.len(),
            self.expected.len(),
            "`{}` ran {} steps and the fixture has {}",
            self.name,
            self.steps.len(),
            self.expected.len()
        );
        for (step, want) in self.steps.iter().zip(&mut self.expected) {
            let label = want["step"].as_str().unwrap_or("").to_owned();
            for allowance in self.allowances.iter().filter(|held| held.step == label) {
                override_rect(want, allowance);
            }
            if self.tolerance > 0.0 {
                if let Some(difference) = close(&step["trees"], &want["trees"], self.tolerance) {
                    panic!("step `{label}` of `{}`: the trees: {difference}", self.name);
                }
            } else {
                assert_eq!(
                    step["trees"], want["trees"],
                    "step `{label}` of `{}`: the trees",
                    self.name
                );
            }
            assert_eq!(
                step["paints"], want["paints"],
                "step `{label}` of `{}`: the paints",
                self.name
            );
        }
    }
}

impl Drop for Rig {
    /// The generation ends while the core is still there to be told: its objects go, and the nodes they hold.
    fn drop(&mut self) {
        host::close_generation(self.ctx);
    }
}

/// Where two states differ, node by node and property by property: `native` is what the run left, `fixture` what the fixture's writes
/// leave.
fn differences(native: &Value, fixture: &Value) -> String {
    let mut out = String::new();
    let (Some(native), Some(fixture)) = (native.as_object(), fixture.as_object()) else {
        return "not objects".to_owned();
    };
    for (name, want) in fixture {
        match native.get(name) {
            None => out.push_str(&format!(
                "  {name}: the native run has no such node (fixture: {want})\n"
            )),
            Some(got) if got == want => {}
            Some(got) => {
                for part in ["class", "parent", "children"] {
                    if got[part] != want[part] {
                        out.push_str(&format!(
                            "  {name}.{part}: native {} fixture {}\n",
                            got[part], want[part]
                        ));
                    }
                }
                let (gp, wp) = (got["props"].as_object(), want["props"].as_object());
                if let (Some(gp), Some(wp)) = (gp, wp) {
                    for (key, value) in wp {
                        if gp.get(key) != Some(value) {
                            out.push_str(&format!("  {name}.{key}: native {:?} fixture {value}\n", gp.get(key)));
                        }
                    }
                    for key in gp.keys() {
                        if !wp.contains_key(key) {
                            out.push_str(&format!("  {name}.{key}: native {} fixture none\n", gp[key]));
                        }
                    }
                } else if got != want {
                    out.push_str(&format!("  {name}: native {got} fixture {want}\n"));
                }
            }
        }
    }
    for name in native.keys() {
        if !fixture.contains_key(name) {
            out.push_str(&format!("  {name}: the fixture has no such node\n"));
        }
    }
    out
}

/// A paint list as its ops (for a person) and its bytes (the proof).
fn render_bytes(bytes: &[u8]) -> Value {
    match decode(bytes) {
        Ok(list) => json!({
            "paint": list.ops.iter().map(|op| format!("{op:?}")).collect::<Vec<_>>(),
            "bytes": hex(bytes),
        }),
        Err(_) => json!({ "bytes": hex(bytes) }),
    }
}

/// What the toolkit reported and what the TypeScript toolkit reported are the same lines in two languages: they agree in how many
/// there were and in what went wrong, the part after the last `: `. A shell's closure that fails is the one exception: the library
/// learns only that it did (`the callback failed`), and the language's own words are the runtime's to log.
fn compare_diagnostics(actual: &Value, expected: &Value, label: &str, name: &str) {
    // The fixture's lines have a node number's digits taken out (`#rgb` reads `#Nrgb`): so do these.
    let tail = |line: &Value, number: bool| {
        let line = line.as_str().unwrap_or("");
        let line = if number {
            unnumber(line).replace('#', "#N")
        } else {
            line.to_owned()
        };
        line.rsplit(": ").next().unwrap_or(&line).to_owned()
    };
    let got: Vec<String> = actual
        .as_array()
        .map(|lines| lines.iter().map(|line| tail(line, true)).collect())
        .unwrap_or_default();
    let want: Vec<String> = expected
        .as_array()
        .map(|lines| lines.iter().map(|line| tail(line, false)).collect())
        .unwrap_or_default();
    assert_eq!(
        got.len(),
        want.len(),
        "step `{label}` of `{name}`: what the toolkit reported: {got:?} against {want:?}"
    );
    for (got, want) in got.iter().zip(&want) {
        assert!(
            got == want || got == "the callback failed",
            "step `{label}` of `{name}`: the toolkit reported `{got}`, the TypeScript toolkit `{want}`"
        );
    }
}

// ---- folding writes into a state --------------------------------------------------------------------------------------------

/// The state a run of writes leaves: the nodes a shell made (class, parent, children, properties), and the properties of the
/// compositor's own nodes that it set.
#[derive(Default)]
struct Fold {
    nodes: BTreeMap<String, Value>,
    shared: BTreeMap<String, Value>,
}

fn key_of(node: &Value) -> String {
    match node {
        Value::String(name) => name.clone(),
        other => other.to_string(),
    }
}

/// The rectangle of a canvas in the snapshot of paint lists, with `key` replaced.
fn override_rect(step: &mut Value, allowance: &Allowance) {
    let at = match allowance.key {
        "x" => 0,
        "y" => 1,
        "width" => 2,
        _ => 3,
    };
    for paint in step["paints"].as_array_mut().into_iter().flatten() {
        for canvas in paint["canvases"].as_array_mut().into_iter().flatten() {
            if canvas["node"] == allowance.node {
                assert_eq!(
                    canvas["rect"][at],
                    number(allowance.fixture),
                    "the fixture's {} of {} is not what was allowed for",
                    allowance.key,
                    allowance.node
                );
                canvas["rect"][at] = number(allowance.native);
            }
        }
    }
}

impl Fold {
    /// Replaces a number the fixture holds with the one the native toolkit does (an [`Allowance`]).
    fn override_number(&mut self, allowance: &Allowance) {
        let entry = self
            .nodes
            .get_mut(allowance.node)
            .unwrap_or_else(|| panic!("no node {} to allow for", allowance.node));
        assert_eq!(
            entry["props"][allowance.key],
            number(allowance.fixture),
            "the fixture's {} of {} is not what was allowed for",
            allowance.key,
            allowance.node
        );
        entry["props"][allowance.key] = number(allowance.native);
    }

    fn apply(&mut self, steps_writes: &Value) {
        for batch in steps_writes.as_array().into_iter().flatten() {
            for op in batch.as_array().into_iter().flatten() {
                let node = key_of(&op["node"]);
                match op["op"].as_str().unwrap_or("") {
                    "new" => {
                        self.nodes.insert(
                            node,
                            json!({ "class": op["class"], "parent": 0, "children": [], "props": {} }),
                        );
                    }
                    "insert" => self.insert(&node, &op["parent"], op["index"].as_i64().unwrap_or(-1)),
                    "set" => {
                        let key = op["key"].as_str().unwrap_or("").to_owned();
                        match self.nodes.get_mut(&node) {
                            Some(entry) => {
                                entry["props"][&key] = op["value"].clone();
                            }
                            None => {
                                self.shared.insert(format!("{node}.{key}"), op["value"].clone());
                            }
                        }
                    }
                    "destroy" => self.destroy(&node),
                    _ => {}
                }
            }
        }
    }

    fn detach(&mut self, node: &str) {
        let parent = self.nodes.get(node).map(|entry| key_of(&entry["parent"]));
        if let Some(parent) = parent
            && let Some(entry) = self.nodes.get_mut(&parent)
            && let Some(children) = entry["children"].as_array_mut()
        {
            children.retain(|child| key_of(child) != node);
        }
    }

    fn insert(&mut self, node: &str, parent: &Value, index: i64) {
        self.detach(node);
        let named = key_of(parent);
        if let Some(entry) = self.nodes.get_mut(node) {
            entry["parent"] = parent.clone();
        }
        if let Some(entry) = self.nodes.get_mut(&named)
            && let Some(children) = entry["children"].as_array_mut()
        {
            let at = usize::try_from(index).map_or(children.len(), |at| at.min(children.len()));
            children.insert(at, json!(node));
        }
    }

    fn destroy(&mut self, node: &str) {
        self.detach(node);
        let mut doomed = vec![node.to_owned()];
        while let Some(next) = doomed.pop() {
            if let Some(entry) = self.nodes.remove(&next) {
                doomed.extend(entry["children"].as_array().into_iter().flatten().map(key_of));
            }
        }
    }

    /// The compositor's nodes' properties the writes touched, as `node.key`.
    fn shared_keys(&self) -> Vec<(String, String)> {
        self.shared
            .keys()
            .filter_map(|key| key.split_once('.').map(|(node, key)| (node.to_owned(), key.to_owned())))
            .collect()
    }

    fn state(&self) -> BTreeMap<String, Value> {
        let mut out = self.nodes.clone();
        for (key, value) in &self.shared {
            out.insert(key.clone(), value.clone());
        }
        out
    }
}

/// Renames the nodes `n<K>` of a fixture's value to the number they have without the foreign ones before them.
fn renumber(value: &mut Value, foreign: &[usize]) {
    match value {
        Value::String(name) => {
            if let Some(at) = name.strip_prefix('n').and_then(|digits| digits.parse::<usize>().ok())
                && !foreign.contains(&at)
            {
                *name = format!("n{}", at - foreign.iter().filter(|held| **held < at).count());
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| renumber(item, foreign)),
        Value::Object(map) => map.values_mut().for_each(|item| renumber(item, foreign)),
        _ => {}
    }
}

/// A record as the closure of a handler gets it: its fields.
pub fn fields(args: &[Val]) -> Vec<Val> {
    match &args[0] {
        Val::Record { fields, .. } => fields.clone(),
        other => panic!("a record, got {other:?}"),
    }
}

/// A closure value for a callback property.
pub fn callback(run: impl FnMut(&[Val]) -> gnoblin_object::Res<Val> + 'static) -> Val {
    let mut run = run;
    Val::Closure(closure(move |_, args| run(args)))
}

/// A number as the oracle's writes show it.
pub fn double(value: f64) -> Value {
    number(value)
}

/// Strips the digits of a node number out of a line: `Camera2D#12` is `Camera2D#`.
pub fn unnumber(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut after_hash = false;
    for ch in line.chars() {
        if after_hash && ch.is_ascii_digit() {
            continue;
        }
        after_hash = ch == '#';
        out.push(ch);
    }
    out
}

/// A line the way the recorder keeps it: the digits of a node number are `N`.
pub fn recorded(line: &str) -> String {
    unnumber(line).replace('#', "#N")
}

/// Where two values differ by more than `tolerance`, or none: numbers are close, a missing number (`null`) is one that is not a
/// number, and everything else is equal.
pub fn close(native: &Value, fixture: &Value, tolerance: f64) -> Option<String> {
    match (native, fixture) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64()?, b.as_f64()?);
            ((a - b).abs() > tolerance).then(|| format!("{a} against {b}"))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{} items against {}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(at, (a, b))| close(a, b, tolerance).map(|why| format!("[{at}]: {why}")))
        }
        (Value::Object(a), Value::Object(b)) => {
            if let Some(key) = a
                .keys()
                .find(|key| !b.contains_key(*key))
                .or_else(|| b.keys().find(|key| !a.contains_key(*key)))
            {
                return Some(format!("the key `{key}` is in one and not the other"));
            }
            a.iter()
                .find_map(|(key, a)| close(a, &b[key], tolerance).map(|why| format!(".{key}: {why}")))
        }
        (a, b) => (a != b).then(|| format!("{a} against {b}")),
    }
}
