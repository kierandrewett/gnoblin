#![allow(unsafe_code)]
//! The object model of `libgnoblin-core` (`gnoblin.model`) against the fixtures the TypeScript library recorded: the order in which the
//! listeners of a window, an output, a collection and the `gnoblin` object are told, how a selection follows its members, what `find`
//! and the focus history answer, the timers, and what an object that is gone still says.
//!
//! Every scenario of `crates/gnoblin-js/tests/oracle/model.rs` is replayed here over the same fake compositor and the same actions, through
//! the object protocol's table and nothing else, and each step's log is compared with the fixture's line for line. What cannot be compared
//! is what only a dynamically typed shell can do: pass a number where a function goes, a record with a key that is not a field. Those
//! calls are refused by the types of the generated SDKs, so the fixture's `refused` entries have no counterpart here; the refusals a
//! native call can make are checked in `refusals`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gnoblin_core_abi::{self as abi, GnoblinStr};
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_object::client::{Api, Reg, closure};
use gnoblin_object::{ClassId, MemberId, Obj, Res, Val};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, text};
use gnoblin_testkit::oracle::{Fixtures, Value, json};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue, TreeLink};

/// The core's registry is one per process, so the tests of this file take turns.
fn process_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn last_error() -> String {
    // SAFETY: with a null buffer and zero capacity the call only returns the length.
    let len = unsafe { gnoblin_core_impl::gnoblin_last_error(std::ptr::null_mut(), 0) };
    let mut buf = vec![0_u8; len + 1];
    // SAFETY: `buf` is writable for `len + 1` bytes.
    let written = unsafe { gnoblin_core_impl::gnoblin_last_error(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(written.min(len));
    String::from_utf8_lossy(&buf).into_owned()
}

struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

/// What the compositor holds before the shell attaches: a 1000 by 600 output `DP-1` and the windows `3` and `4`, as the TypeScript
/// scenarios have it.
fn world(comp: &FakeCompositor, extra_windows: &[(u64, &str, &str)]) {
    comp.add_desktop(1000.0, 600.0);
    {
        let mut borrowed = comp.borrow_mut();
        let screen = borrowed.nodes.get_mut(&20).expect("the screen");
        screen.props.insert("name".to_owned(), text("output-0"));
        screen.props.insert("connector".to_owned(), text("DP-1"));
        screen.props.insert("x".to_owned(), MirrorValue::Double(0.0));
        screen.props.insert("y".to_owned(), MirrorValue::Double(0.0));
    }
    comp.add_window_decorated(3, "one", "app.one", "server");
    comp.add_window_decorated(4, "two", "app.two", "server");
    for (id, title, app) in extra_windows {
        comp.add_window_decorated(*id, title, app, "server");
    }
}

/// A value of the fixture for a number the scenario records.
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// The name a TypeScript listener method has for a signal: `title_changed` is `onTitleChanged`.
fn method_name(signal: &str) -> String {
    let mut name = String::from("on");
    for word in signal.split('_') {
        let mut letters = word.chars();
        if let Some(first) = letters.next() {
            name.extend(first.to_uppercase());
            name.push_str(letters.as_str());
        }
    }
    name
}

/// A shell over the fake compositor: a generation that attached, and the table of `gnoblin.model` to call.
struct Rig {
    ctx: u64,
    api: Api,
    comp: FakeCompositor,
    clock: ManualClock,
    log: Rc<RefCell<Vec<Value>>>,
    gnoblin: Obj,
    fixture: Value,
    step: std::cell::Cell<usize>,
    regs: Vec<Reg>,
    _lock: MutexGuard<'static, ()>,
}

impl Rig {
    fn new(scenario: &str, extra_windows: &[(u64, &str, &str)]) -> Rig {
        let lock = process_lock();
        host::reset_session();
        let comp = FakeCompositor::new();
        world(&comp, extra_windows);
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        let clock = ManualClock::default();
        host::set_clock(Arc::new(clock.clone()));
        let ctx = host::open_generation("model", abi::LANG_NATIVE).expect("a generation opens");
        // SAFETY: `tree_attach` is a function of the core's table.
        assert_eq!(unsafe { (host::api().tree_attach)(ctx, abi::ATTACH_HOLDER) }, abi::E_OK);
        let mut raw: *const abi::GnoblinObjectApi = std::ptr::null();
        // SAFETY: `raw` is a live local the library writes.
        let code = unsafe { (host::api().object_api)(ctx, GnoblinStr::borrowed("gnoblin.model"), &raw mut raw) };
        assert_eq!(code, abi::E_OK, "{}", last_error());
        // SAFETY: the table `object_api` returns is static.
        let api = Api::new(unsafe { &*raw }, ctx, last_error);
        let fixture = Fixtures::new("crates/gnoblin-core-impl/tests/fixtures/model").read_json(scenario);
        let class = api.class("gnoblin").expect("the service class");
        let gnoblin = api.create(class, &[]).expect("the service");
        Rig {
            ctx,
            api,
            comp,
            clock,
            log: Rc::new(RefCell::new(Vec::new())),
            gnoblin,
            fixture,
            step: std::cell::Cell::new(0),
            regs: Vec::new(),
            _lock: lock,
        }
    }

    // ---- the protocol, by name -----------------------------------------------------------------------------------------------

    fn class(&self, name: &str) -> ClassId {
        self.api
            .class(name)
            .unwrap_or_else(|error| panic!("no class {name}: {error}"))
    }

    fn member(&self, class: &str, kind: u32, name: &str) -> MemberId {
        self.api
            .member(self.class(class), kind, name)
            .unwrap_or_else(|error| panic!("no member {class}.{name}: {error}"))
    }

    fn prop(&self, obj: Obj, name: &str) -> Val {
        member_of(self.api, obj, abi::MEMBER_PROPERTY, name)
            .and_then(|member| self.api.get(obj, member))
            .unwrap_or_else(|error| panic!("reading {name}: {error}"))
    }

    fn set(&self, obj: Obj, name: &str, value: &Val) -> Res<()> {
        member_of(self.api, obj, abi::MEMBER_PROPERTY, name).and_then(|member| self.api.set(obj, member, value))
    }

    fn call(&self, obj: Obj, name: &str, args: &[Val]) -> Res<Val> {
        member_of(self.api, obj, abi::MEMBER_METHOD, name).and_then(|member| self.api.call(obj, member, args))
    }

    fn object(&self, obj: Obj, name: &str) -> Obj {
        self.prop(obj, name).object().expect("an object")
    }

    fn clients(&self) -> Obj {
        self.object(self.gnoblin, "clients")
    }

    fn outputs(&self) -> Obj {
        self.object(self.gnoblin, "outputs")
    }

    fn client(&self, id: u64) -> Obj {
        self.call(self.clients(), "get", &[Val::Text(id.to_string())])
            .unwrap_or_else(|error| panic!("no client {id}: {error}"))
            .object()
            .expect("an object")
    }

    fn output(&self, index: i64) -> Obj {
        self.call(self.outputs(), "at", &[Val::Int(index)])
            .expect("an output")
            .object()
            .expect("an object")
    }

    /// The ids of the objects of a list.
    fn ids(&self, list: &Val) -> Vec<String> {
        list.list()
            .expect("a list")
            .iter()
            .map(|item| text_of(self.api, item, "id"))
            .collect()
    }

    // ---- listening -----------------------------------------------------------------------------------------------------------

    /// Listens to `signal` of `obj` and logs `<place>:<method>:<id of the first argument's object, or its name>`.
    fn hear(&mut self, obj: Obj, signal: &str, place: &'static str, label: &'static str) {
        let api = self.api;
        let log = Rc::clone(&self.log);
        // A facade's signal is the collection's with a prefix, and the fixture logs the method of the collection.
        let method = method_name(
            signal
                .strip_prefix("client_")
                .or_else(|| signal.strip_prefix("output_"))
                .unwrap_or(signal),
        );
        let member = self.signal_of(obj, signal);
        let reg = self
            .api
            .listen(obj, member, move |_, args| {
                let who = describe(api, &args[0], label);
                log.borrow_mut().push(json!(format!("{place}:{method}:{who}")));
                Ok(Val::None)
            })
            .expect("a listener");
        self.regs.push(reg);
    }

    fn signal_of(&self, obj: Obj, signal: &str) -> MemberId {
        member_of(self.api, obj, abi::MEMBER_SIGNAL, signal)
            .unwrap_or_else(|error| panic!("no signal {signal}: {error}"))
    }

    /// Listens and logs `<who>:<args.len()>:<id>:<second argument>`, as the listener-order scenario does.
    fn note(&mut self, obj: Obj, signal: &str, who: &'static str) {
        let api = self.api;
        let log = Rc::clone(&self.log);
        let member = self.signal_of(obj, signal);
        let reg = self
            .api
            .listen(obj, member, move |_, args| {
                let second = match &args[1] {
                    Val::Text(text) => text.clone(),
                    Val::Bool(flag) => flag.to_string(),
                    other => format!("{other:?}"),
                };
                log.borrow_mut().push(json!(format!(
                    "{who}:{}:{}:{second}",
                    args.len(),
                    text_of(api, &args[0], "id")
                )));
                Ok(Val::None)
            })
            .expect("a listener");
        self.regs.push(reg);
    }

    /// Ends every listener made with [`Rig::hear`] and [`Rig::note`], twice, as the scenarios' `off(); off();` does.
    fn off(&mut self) {
        for reg in std::mem::take(&mut self.regs) {
            self.api.unlisten(reg).expect("the first end of a listener");
            assert!(
                self.api.unlisten(reg).is_err(),
                "ending a listener twice is harmless and refused"
            );
        }
    }

    // ---- the compositor's side ---------------------------------------------------------------------------------------------

    /// Runs the turns the shell would: the pump, the queued calls, the timers.
    fn settle(&self) {
        for _ in 0..8 {
            // SAFETY: `dispatch` is a function of the core's table.
            unsafe { (host::api().dispatch)(self.ctx, 0) };
        }
    }

    fn open(&self, id: u64, title: &str, app_id: &str) {
        self.comp.create_window(id, title, app_id);
        self.settle();
    }

    fn destroy(&self, id: u64) {
        self.comp.destroy(id);
        self.settle();
    }

    fn put(&self, id: u64, key: &str, value: MirrorValue) {
        self.comp.external_set(id, key, value);
        self.settle();
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
        self.settle();
    }

    fn rec(&self, label: &str, value: Value) {
        self.log.borrow_mut().push(json!({ "label": label, "value": value }));
    }

    /// Ends a step: what the listeners logged since the last one must be what the fixture's step logged, but for the entries the
    /// call names in `skip`: a `rec` whose label only a dynamically typed shell can fill.
    #[track_caller]
    fn step(&self, label: &str, skip: &[&str]) {
        let steps = self.fixture["steps"].as_array().expect("the fixture has steps");
        let expected = &steps[self.step.get()];
        assert_eq!(
            expected["step"],
            json!(label),
            "the steps of the fixture and of the test agree"
        );
        let want: Vec<Value> = expected["log"]
            .as_array()
            .expect("a log")
            .iter()
            .filter(|entry| !skip.iter().any(|name| entry["label"] == json!(name)))
            .cloned()
            .collect();
        let got: Vec<Value> = self.log.borrow_mut().drain(..).collect();
        assert_eq!(Value::Array(got), Value::Array(want), "step `{label}`");
        self.step.set(self.step.get() + 1);
    }

    /// The boot step: the load's own turns.
    #[track_caller]
    fn boot(&self) {
        self.settle();
        self.step("boot", &[]);
    }

    #[track_caller]
    fn finish(&self) {
        let steps = self.fixture["steps"].as_array().expect("steps").len();
        assert_eq!(self.step.get(), steps, "every step of the fixture was replayed");
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        host::close_generation(self.ctx);
        host::reset_session();
    }
}

fn member_of(api: Api, obj: Obj, kind: u32, name: &str) -> Res<MemberId> {
    let class = api.class_of(obj)?;
    api.member(class, kind, name)
}

/// A property of an object value, as text.
fn text_of(api: Api, value: &Val, name: &str) -> String {
    let obj = value.object().expect("an object");
    member_of(api, obj, abi::MEMBER_PROPERTY, name)
        .and_then(|member| api.get(obj, member))
        .map(|value| value.text().expect("text").to_owned())
        .unwrap_or_else(|error| panic!("reading {name}: {error}"))
}

/// How a log line names the object an event is about: a window by its id, an output by its `label`.
fn describe(api: Api, value: &Val, label: &str) -> String {
    match label {
        "name" => text_of(api, value, "name"),
        _ => text_of(api, value, "id"),
    }
}

fn double(value: f64) -> MirrorValue {
    MirrorValue::Double(value)
}

// ---- the scenarios -------------------------------------------------------------------------------------------------------------

const CLIENT_EVENTS: [(&str, &str); 10] = [
    ("title_changed", "onTitleChanged"),
    ("app_id_changed", "onAppIdChanged"),
    ("decoration_changed", "onDecorationChanged"),
    ("focus_changed", "onFocusChanged"),
    ("geometry_changed", "onGeometryChanged"),
    ("pointer_enter", "onPointerEnter"),
    ("pointer_motion", "onPointerMotion"),
    ("pointer_leave", "onPointerLeave"),
    ("pointer_press", "onPointerPress"),
    ("pointer_release", "onPointerRelease"),
];

/// What the compositor does to raise the repeating events of a window.
fn client_cause(rig: &Rig, signal: &str, count: u32) {
    match signal {
        "title_changed" => rig.put(3, "title", text(&format!("t{count}"))),
        "app_id_changed" => rig.put(3, "app_id", text(&format!("a{count}"))),
        "decoration_changed" => rig.put(3, "decoration", text(if count % 2 == 1 { "client" } else { "server" })),
        "focus_changed" => rig.put(3, "focused", MirrorValue::Bool(count % 2 == 1)),
        "geometry_changed" => rig.put(3, "width", double(300.0 + f64::from(count))),
        other => {
            rig.comp
                .pointer(3, other.strip_prefix("pointer_").expect("a pointer edge"), 1.0, 2.0, 0);
            rig.settle();
        }
    }
}

#[test]
fn client_events() {
    let mut rig = Rig::new("model_client_events", &[]);
    rig.boot();
    for (signal, method) in CLIENT_EVENTS {
        let facade = format!("client_{signal}");
        let (client, clients, gnoblin) = (rig.client(3), rig.clients(), rig.gnoblin);
        rig.hear(client, signal, "object", "id");
        rig.hear(clients, signal, "collection", "id");
        rig.hear(gnoblin, &facade, "facade", "id");
        rig.settle();
        rig.step(&format!("{method}_subscribed"), &[]);
        client_cause(&rig, signal, 1);
        rig.step(&format!("{method}_heard"), &[]);
        rig.off();
        rig.settle();
        rig.step(&format!("{method}_unsubscribed"), &[]);
        client_cause(&rig, signal, 2);
        rig.step(&format!("{method}_heard_by_none"), &[]);
    }
    rig.finish();
}

#[test]
fn client_lifecycle() {
    let mut rig = Rig::new("model_client_lifecycle", &[]);
    rig.boot();
    let (three, clients, gnoblin) = (rig.client(3), rig.clients(), rig.gnoblin);
    rig.hear(three, "closed", "object", "id");
    rig.hear(clients, "closed", "collection", "id");
    rig.hear(gnoblin, "client_closed", "facade", "id");
    // `onClientClosed` is told as `onClosed` of the facade; the fixture writes `onClientClosed`.
    let api = rig.api;
    let log = Rc::clone(&rig.log);
    let removed = rig.signal_of(clients, "removed");
    let reg = api
        .listen(clients, removed, move |_, args| {
            let same = matches!(&args[1], Val::Object { .. });
            log.borrow_mut().push(json!(format!(
                "collection:onRemoved:{}:{same}",
                text_of(api, &args[0], "id")
            )));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.regs.push(reg);
    rig.settle();
    rig.step("subscribed_to_closing", &[]);
    rig.destroy(3);
    // The facade's line is `facade:onClientClosed:3` in the fixture.
    let lines: Vec<Value> = rig
        .log
        .borrow()
        .iter()
        .map(|line| {
            json!(
                line.as_str()
                    .unwrap()
                    .replace("facade:onClosed", "facade:onClientClosed")
            )
        })
        .collect();
    *rig.log.borrow_mut() = lines;
    rig.step("window_closed", &[]);
    rig.off();
    rig.destroy(4);
    rig.step("unsubscribed_window_closed", &[]);

    let (clients, gnoblin) = (rig.clients(), rig.gnoblin);
    let api = rig.api;
    let log = Rc::clone(&rig.log);
    let added = rig.signal_of(clients, "added");
    let reg = api
        .listen(clients, added, move |_, args| {
            log.borrow_mut().push(json!(format!(
                "collection:onAdded:{}:{}",
                text_of(api, &args[0], "id"),
                matches!(&args[1], Val::Object { .. })
            )));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.regs.push(reg);
    let log = Rc::clone(&rig.log);
    let created = rig.signal_of(gnoblin, "client_created");
    let reg = api
        .listen(gnoblin, created, move |_, args| {
            log.borrow_mut().push(json!(format!(
                "facade:onClientCreated:{}",
                text_of(api, &args[0], "id")
            )));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.regs.push(reg);
    let log = Rc::clone(&rig.log);
    let ready = rig.signal_of(clients, "ready");
    let reg = api
        .listen(clients, ready, move |_, args| {
            log.borrow_mut()
                .push(json!(format!("collection:onReady:{}", text_of(api, &args[0], "id"))));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.regs.push(reg);
    let log = Rc::clone(&rig.log);
    let ready = rig.signal_of(gnoblin, "client_ready");
    let reg = api
        .listen(gnoblin, ready, move |_, args| {
            log.borrow_mut()
                .push(json!(format!("facade:onClientReady:{}", text_of(api, &args[0], "id"))));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.regs.push(reg);
    rig.settle();
    rig.step("subscribed_to_arrivals", &[]);
    rig.open(100, "new", "app.new");
    rig.step("window_arrived", &[]);
    rig.off();
    rig.open(101, "newer", "app.newer");
    rig.step("unsubscribed_window_arrived", &[]);
    rig.finish();
}

#[test]
fn a_listener_of_created_is_told_of_the_windows_there() {
    let rig = Rig::new("model_created_replays", &[]);
    let (gnoblin, api) = (rig.gnoblin, rig.api);
    let log = Rc::clone(&rig.log);
    let created = rig.signal_of(gnoblin, "client_created");
    let reg = api
        .listen(gnoblin, created, move |_, args| {
            log.borrow_mut()
                .push(json!(format!("created {}", text_of(api, &args[0], "id"))));
            Ok(Val::None)
        })
        .expect("a listener");
    rig.boot();
    rig.open(5, "three", "app.three");
    rig.step("window_arrived", &[]);
    api.unlisten(reg).expect("ends");
    rig.open(6, "four", "app.four");
    rig.step("detached_then_arrived", &[]);
    rig.finish();
}

/// `at` and `get` answer none for a member that is not there (the catalog marks both `optional`), and the member itself, unchanged, for one
/// that is: the generated SDKs read none as `None`, `undefined` and `Option::None`.
#[test]
fn a_lookup_that_finds_nothing_answers_none_and_one_that_finds_answers_the_member() {
    let rig = Rig::new("model_ready_at_boot", &[]);
    rig.settle();
    for (owner, count) in [(rig.clients(), 2), (rig.outputs(), 1)] {
        for index in [count, 99, -count - 1, i64::MIN] {
            let answer = rig
                .call(owner, "at", &[Val::Int(index)])
                .expect("a lookup is not an error");
            assert!(answer.is_none(), "at({index}) found nothing: {answer:?}");
        }
        let first = rig.call(owner, "at", &[Val::Int(0)]).expect("at(0)");
        assert!(first.object().is_ok(), "the first member is an object: {first:?}");
        let last = rig.call(owner, "at", &[Val::Int(-1)]).expect("at(-1)");
        assert!(last.object().is_ok(), "the last member is an object: {last:?}");
    }
    let absent = rig
        .call(rig.clients(), "get", &[Val::Text("no-such".into())])
        .expect("a lookup is not an error");
    assert!(absent.is_none(), "{absent:?}");
    let found = rig
        .call(rig.clients(), "get", &[Val::Text("3".into())])
        .expect("get(3)");
    assert_eq!(rig.client(3), found.object().expect("the window"));
}

#[test]
fn ready_is_told_for_the_windows_there_at_the_start() {
    let rig = Rig::new("model_ready_at_boot", &[]);
    let (three, clients, gnoblin) = (rig.client(3), rig.clients(), rig.gnoblin);
    let api = rig.api;
    for (obj, signal, place) in [
        (three, "ready", "object"),
        (clients, "ready", "collection"),
        (gnoblin, "client_ready", "facade"),
    ] {
        let log = Rc::clone(&rig.log);
        let member = rig.signal_of(obj, signal);
        api.listen(obj, member, move |_, args| {
            log.borrow_mut()
                .push(json!(format!("{place}:{}", text_of(api, &args[0], "id"))));
            Ok(Val::None)
        })
        .expect("a listener");
    }
    rig.boot();
    rig.finish();
}

#[test]
fn output_events() {
    let mut rig = Rig::new("model_output_events", &[]);
    rig.boot();
    let output = rig.output(0);
    rig.rec(
        "id",
        json!(text_of(
            rig.api,
            &Val::Object {
                class: rig.class("output"),
                obj: output
            },
            "id"
        )),
    );
    rig.settle();
    rig.step("output_id", &[]);
    for (signal, method) in [
        ("mode_changed", "onModeChanged"),
        ("scale_changed", "onScaleChanged"),
        ("geometry_changed", "onGeometryChanged"),
        ("applied", "onApplied"),
    ] {
        let (outputs, gnoblin) = (rig.outputs(), rig.gnoblin);
        rig.hear(output, signal, "object", "id");
        rig.hear(outputs, signal, "collection", "id");
        rig.hear(gnoblin, &format!("output_{signal}"), "facade", "id");
        rig.settle();
        rig.step(&format!("{method}_subscribed"), &[]);
        for round in [1_u32, 2] {
            if round == 2 {
                rig.off();
                rig.settle();
                rig.step(&format!("{method}_unsubscribed"), &[]);
            }
            match signal {
                "mode_changed" => rig.put(20, "mode", text(&format!("{}x600@60", 800 + round))),
                "scale_changed" => rig.put(20, "output_scale", double(1.0 + f64::from(round))),
                "geometry_changed" => rig.put(20, "width", double(1000.0 + f64::from(round))),
                _ => {
                    let config = apply_config(&rig, f64::from(round) / 10.0 + 1.0);
                    let operation = rig.call(output, "apply", &[config]).expect("an operation");
                    rig.settle();
                    let token = rig.comp.applies().last().expect("an apply went out").2;
                    rig.comp.answer(token, "", "");
                    rig.settle();
                    assert_eq!(text_of(rig.api, &operation, "error"), "");
                    let status = rig.prop(operation.object().unwrap(), "status");
                    assert!(matches!(status, Val::Enum { .. }), "the status is an enum value");
                }
            }
            rig.step(&format!("{method}_round_{round}"), &[]);
        }
    }
    rig.finish();
}

/// An `output_config` with only a scale.
fn apply_config(rig: &Rig, scale: f64) -> Val {
    let class = rig.class("output_config");
    let names: Vec<String> = rig
        .api
        .catalog()
        .expect("the catalog")
        .iter()
        .filter(|item| item.entry == abi::CATALOG_MEMBER && item.class == class && item.kind == abi::MEMBER_PROPERTY)
        .map(|item| item.name.clone())
        .collect();
    Val::Record {
        class,
        fields: names
            .iter()
            .map(|name| if name == "scale" { Val::Double(scale) } else { Val::None })
            .collect(),
    }
}

#[test]
fn outputs_come_and_go() {
    let mut rig = Rig::new("model_output_hotplug", &[]);
    rig.boot();
    let (outputs, gnoblin) = (rig.outputs(), rig.gnoblin);
    let (api, log) = (rig.api, Rc::clone(&rig.log));
    // The fixture's listeners: `onConnected`, `onAdded`, `onRemoved` and `onDisconnected` of the collection and of the facade, in the
    // order the scenario made them.
    let pairs: [(Obj, &str, &str, &str); 8] = [
        (outputs, "connected", "collection", "onConnected"),
        (gnoblin, "output_connected", "facade", "onOutputConnected"),
        (outputs, "added", "collection", "onAdded"),
        (gnoblin, "output_added", "facade", "onOutputAdded"),
        (outputs, "removed", "collection", "onRemoved"),
        (gnoblin, "output_removed", "facade", "onOutputRemoved"),
        (outputs, "disconnected", "collection", "onDisconnected"),
        (gnoblin, "output_disconnected", "facade", "onOutputDisconnected"),
    ];
    for (obj, signal, place, method) in pairs {
        let log = Rc::clone(&log);
        let member = rig.signal_of(obj, signal);
        let reg = api
            .listen(obj, member, move |_, args| {
                let output = args[0].object().expect("an output");
                let name = text_of(api, &args[0], "name");
                let tail = match (place, method) {
                    ("collection", "onAdded") => {
                        format!(":{}", matches!(&args[1], Val::Object { obj, .. } if *obj == outputs))
                    }
                    ("collection", "onDisconnected") => {
                        let alive = member_of(api, output, abi::MEMBER_PROPERTY, "alive")
                            .and_then(|member| api.get(output, member))
                            .and_then(|value| value.bool())
                            .expect("alive");
                        format!(":{alive}")
                    }
                    _ => String::new(),
                };
                log.borrow_mut().push(json!(format!("{place}:{method}:{name}{tail}")));
                Ok(Val::None)
            })
            .expect("a listener");
        rig.regs.push(reg);
    }
    rig.settle();
    // A listener of `added` is told of the outputs that are there, as the fixture's first step shows.
    rig.step("subscribed", &[]);
    rig.comp
        .create_screen(30, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
    rig.settle();
    rig.step("plugged_in", &[]);
    rig.destroy(30);
    rig.step("unplugged", &[]);
    rig.off();
    rig.comp
        .create_screen(31, "output-2", (1000.0, 0.0, 600.0, 400.0), false);
    rig.settle();
    rig.step("unsubscribed_plugged_in", &[]);
    rig.finish();
}

#[test]
fn listeners_and_subscriptions_are_one_list() {
    let mut rig = Rig::new("model_listener_order", &[]);
    let (three, clients, gnoblin) = (rig.client(3), rig.clients(), rig.gnoblin);
    // `A`, `C` and `D` are listener objects; `B`, `E`, `F` and the rest are subscriptions. All stand in the one list of the signal.
    let api = rig.api;
    let listener_of = |rig: &Rig, class: &str, who: &'static str, signals: &[&'static str]| -> Obj {
        let interface = rig.class(class);
        let members: Vec<MemberId> = signals
            .iter()
            .map(|signal| rig.member(class, abi::MEMBER_METHOD, &format!("on_{signal}")))
            .collect();
        let log = Rc::clone(&rig.log);
        let table: Vec<MemberId> = members.clone();
        let _ = &table;
        api.implement(interface, &members, move |_, args| {
            let second = match &args[1] {
                Val::Text(text) => text.clone(),
                Val::Bool(flag) => flag.to_string(),
                other => format!("{other:?}"),
            };
            log.borrow_mut().push(json!(format!(
                "{who}:{}:{}:{second}",
                args.len(),
                text_of(api, &args[0], "id")
            )));
            Ok(Val::None)
        })
        .expect("a listener object")
    };
    let add = |rig: &Rig, obj: Obj, listener: Obj| {
        rig.call(
            obj,
            "add_listener",
            &[Val::Object {
                class: api.class_of(listener).unwrap(),
                obj: listener,
            }],
        )
        .expect("a listener is added");
    };
    let a = listener_of(&rig, "client_listener", "A", &["title_changed"]);
    add(&rig, three, a);
    rig.note(three, "title_changed", "B");
    let c = listener_of(&rig, "client_listener", "C", &["title_changed"]);
    add(&rig, three, c);
    // `C` also has `onFocusChanged` in the fixture, logged as `c`.
    let lower_c = listener_of(&rig, "client_listener", "c", &["focus_changed"]);
    add(&rig, three, lower_c);
    let d = listener_of(&rig, "client_collection_listener", "D", &["title_changed"]);
    add(&rig, clients, d);
    rig.note(clients, "title_changed", "E");
    rig.note(gnoblin, "client_title_changed", "F");
    let g = listener_of(&rig, "client_collection_listener", "G", &["title_changed"]);
    add(&rig, clients, g);
    rig.note(clients, "focus_changed", "f");
    rig.note(three, "focus_changed", "b");
    rig.boot();
    rig.put(3, "title", text("t1"));
    rig.step("title_one", &[]);
    rig.put(3, "title", text("t2"));
    rig.step("title_two", &[]);
    rig.put(3, "focused", MirrorValue::Bool(true));
    rig.step("focus", &[]);
    rig.finish();
}

#[test]
fn a_listener_that_is_removed_hears_nothing_and_one_added_twice_is_added_once() {
    let rig = Rig::new("model_listener_removal", &[]);
    let three = rig.client(3);
    let api = rig.api;
    let make = |rig: &Rig, who: &'static str| -> Obj {
        let members = vec![rig.member("client_listener", abi::MEMBER_METHOD, "on_title_changed")];
        let log = Rc::clone(&rig.log);
        api.implement(rig.class("client_listener"), &members, move |_, _| {
            log.borrow_mut().push(json!(who));
            Ok(Val::None)
        })
        .expect("a listener object")
    };
    let value = |obj: Obj| Val::Object {
        class: api.class_of(obj).unwrap(),
        obj,
    };
    let gone = make(&rig, "gone");
    let keep = make(&rig, "keep");
    rig.call(three, "add_listener", &[value(gone)]).expect("added");
    let log = Rc::clone(&rig.log);
    let title = rig.signal_of(three, "title_changed");
    let sub = api
        .listen(three, title, move |_, _| {
            log.borrow_mut().push(json!("sub"));
            Ok(Val::None)
        })
        .expect("a subscription");
    rig.call(three, "add_listener", &[value(keep)]).expect("added");
    rig.call(three, "add_listener", &[value(keep)]).expect("added twice");
    rig.boot();
    rig.put(3, "title", text("first"));
    rig.step("first", &[]);
    rig.call(three, "remove_listener", &[value(gone)]).expect("removed");
    api.unlisten(sub).expect("ended");
    rig.put(3, "title", text("second"));
    rig.step("after_removal", &[]);
    rig.finish();
}

#[test]
fn a_listener_that_fails_stops_no_other() {
    let rig = Rig::new("model_listener_errors", &[]);
    let three = rig.client(3);
    let api = rig.api;
    let members = vec![rig.member("client_listener", abi::MEMBER_METHOD, "on_title_changed")];
    let explode = api
        .implement(rig.class("client_listener"), &members, |_, _| {
            Err(gnoblin_object::Fail::err("boom"))
        })
        .expect("a listener object");
    rig.boot();
    rig.step("registration", &["refused"]);
    rig.call(
        three,
        "add_listener",
        &[Val::Object {
            class: api.class_of(explode).unwrap(),
            obj: explode,
        }],
    )
    .expect("added");
    let log = Rc::clone(&rig.log);
    let title = rig.signal_of(three, "title_changed");
    api.listen(three, title, move |_, _| {
        log.borrow_mut().push(json!("after the throw"));
        Ok(Val::None)
    })
    .expect("a subscription");
    rig.put(3, "title", text("x"));
    rig.step("a_throwing_listener", &[]);
    rig.put(3, "title", text("y"));
    rig.step("the_same_failure_again", &[]);
    rig.finish();
}

#[test]
fn collections_follow_their_members() {
    let rig = Rig::new("model_collections", &[]);
    rig.boot();
    let clients = rig.clients();
    let api = rig.api;
    let query = |rig: &Rig, fields: &[(&str, Val)]| -> Val {
        let class = rig.class("client_query");
        let names: Vec<String> = rig
            .api
            .catalog()
            .expect("the catalog")
            .iter()
            .filter(|item| {
                item.entry == abi::CATALOG_MEMBER && item.class == class && item.kind == abi::MEMBER_PROPERTY
            })
            .map(|item| item.name.clone())
            .collect();
        Val::Record {
            class,
            fields: names
                .iter()
                .map(|name| {
                    fields
                        .iter()
                        .find(|(held, _)| held == name)
                        .map_or(Val::None, |(_, value)| value.clone())
                })
                .collect(),
        }
    };
    let where_ = |rig: &Rig, fields: &[(&str, Val)]| -> Obj {
        rig.call(clients, "where", &[query(rig, fields)])
            .expect("a selection")
            .object()
            .expect("an object")
    };
    let foot = where_(&rig, &[("app_id", Val::Text("app.one".into()))]);
    let either = where_(
        &rig,
        &[(
            "any_app_id",
            Val::List(vec![Val::Text("app.one".into()), Val::Text("app.two".into())]),
        )],
    );
    let titled = where_(
        &rig,
        &[("title", Val::Text("two".into())), ("focused", Val::Bool(false))],
    );
    let length = |obj: Obj| rig.prop(obj, "length").uint().expect("a number");
    let members = |obj: Obj| -> Value { json!(rig.ids(&rig.call(obj, "to_list", &[]).expect("a list"))) };
    rig.rec(
        "lengths",
        json!([length(clients), length(foot), length(either), length(titled)]),
    );
    rig.rec(
        "members",
        json!([members(clients), members(foot), members(either), members(titled)]),
    );
    let one = rig.client(3);
    let missing = rig
        .call(clients, "get", &[Val::Text("99".into())])
        .expect("a lookup that finds nothing")
        .is_none();
    let last = text_of(api, &rig.call(clients, "at", &[Val::Int(-1)]).expect("the last"), "id");
    let beyond = rig
        .call(clients, "at", &[Val::Int(5)])
        .expect("a lookup that finds nothing")
        .is_none();
    rig.rec(
        "lookup",
        json!([
            text_of(
                api,
                &Val::Object {
                    class: rig.class("client"),
                    obj: one
                },
                "title"
            ),
            missing,
            last,
            beyond
        ]),
    );
    let all = rig.call(clients, "to_list", &[]).expect("a list");
    let wide = all
        .list()
        .unwrap()
        .iter()
        .filter(|item| rig.prop(item.object().unwrap(), "width").double().unwrap() > 0.0)
        .count();
    let two = all
        .list()
        .unwrap()
        .iter()
        .find(|item| text_of(api, item, "title") == "two")
        .map(|item| text_of(api, item, "id"))
        .expect("a window titled two");
    rig.rec("snapshots", json!([wide, rig.ids(&all), two]));
    rig.settle();
    rig.step("selections", &["refused"]);

    rig.put(3, "app_id", text("app.two"));
    rig.rec(
        "members",
        json!([
            length(foot),
            length(either),
            text_of(
                api,
                &Val::Object {
                    class: rig.class("client"),
                    obj: one
                },
                "app_id"
            )
        ]),
    );
    rig.step("a_property_leaves_a_selection", &[]);
    rig.put(4, "title", text("renamed"));
    rig.rec("titled", json!(length(titled)));
    rig.step("a_property_leaves_another", &[]);
    rig.open(5, "two", "app.one");
    rig.rec("members", json!([members(foot), members(titled)]));
    rig.step("a_window_joins_selections", &[]);

    // A listener of a selection is told of the members it has, and then of what happens to them.
    let selection = foot;
    // Two interface objects tell `added` and `removed` apart by the member the library called.
    let class = "client_collection_listener";
    let on_added = rig.member(class, abi::MEMBER_METHOD, "on_added");
    let on_removed = rig.member(class, abi::MEMBER_METHOD, "on_removed");
    let on_title = rig.member(class, abi::MEMBER_METHOD, "on_title_changed");
    let log = Rc::clone(&rig.log);
    let told = api
        .implement(rig.class(class), &[on_added, on_removed], move |member, args| {
            let id = text_of(api, &args[0], "id");
            let line = if member == on_added {
                format!(
                    "replay:{id}:{}",
                    matches!(&args[1], Val::Object { obj, .. } if *obj == selection)
                )
            } else {
                format!("removed:{id}")
            };
            log.borrow_mut().push(json!(line));
            Ok(Val::None)
        })
        .expect("a listener object");
    let log = Rc::clone(&rig.log);
    let heard = api
        .implement(rig.class(class), &[on_title], move |_, args| {
            log.borrow_mut()
                .push(json!(format!("title:{}", text_of(api, &args[0], "id"))));
            Ok(Val::None)
        })
        .expect("a listener object");
    for listener in [told, heard] {
        rig.call(
            selection,
            "add_listener",
            &[Val::Object {
                class: api.class_of(listener).unwrap(),
                obj: listener,
            }],
        )
        .expect("added");
    }
    rig.settle();
    rig.step("a_collection_listener_is_told_of_the_members", &[]);
    rig.put(5, "title", text("moved"));
    rig.step("title_of_a_member", &[]);
    rig.put(5, "app_id", text("elsewhere"));
    rig.step("a_member_leaves", &[]);
    rig.put(5, "title", text("not heard"));
    rig.step("no_longer_heard", &[]);
    rig.destroy(5);
    rig.step("closed", &[]);

    // `find`: every window, the ones of an application, and no output kind of a window.
    let found = |rig: &Rig, fields: &[(&str, Val)]| -> Vec<String> {
        rig.ids(
            &rig.call(rig.gnoblin, "find_clients", &[query(rig, fields)])
                .expect("windows"),
        )
    };
    let everyone = rig.ids(&rig.call(rig.gnoblin, "find_clients", &[]).expect("windows"));
    let by_app = found(&rig, &[("app_id", Val::Text("app.two".into()))]);
    let by_width = found(&rig, &[("width", Val::Double(400.0))]);
    let outputs = rig
        .call(rig.gnoblin, "find_outputs", &[])
        .expect("outputs")
        .list()
        .unwrap()
        .len();
    rig.rec("find", json!([everyone, by_app, by_width, outputs]));
    rig.step("find", &[]);
    // The snake and camel spellings of a key are one key in the TypeScript; the record has the one name.
    rig.step("find_refusals", &["find_refused"]);
    rig.finish();
}

#[test]
fn focus_history() {
    let rig = Rig::new("model_focus_history", &[(5, "three", "app.three")]);
    rig.boot();
    let focus = rig.object(rig.gnoblin, "focus");
    let history = |rig: &Rig, limit: Option<i64>| -> Res<Vec<String>> {
        let class = rig.class("focus_filter");
        let names: Vec<String> = rig
            .api
            .catalog()
            .expect("the catalog")
            .iter()
            .filter(|item| {
                item.entry == abi::CATALOG_MEMBER && item.class == class && item.kind == abi::MEMBER_PROPERTY
            })
            .map(|item| item.name.clone())
            .collect();
        let filter = Val::Record {
            class,
            fields: names
                .iter()
                .map(|name| match (name.as_str(), limit) {
                    ("limit", Some(limit)) => Val::Int(limit),
                    _ => Val::None,
                })
                .collect(),
        };
        rig.call(focus, "history", &[filter]).map(|list| rig.ids(&list))
    };
    rig.rec("history", json!(history(&rig, None).unwrap()));
    rig.step("nothing_focused", &[]);
    rig.put(4, "focused", MirrorValue::Bool(true));
    rig.rec("history", json!(history(&rig, None).unwrap()));
    rig.step("four_focused", &[]);
    rig.put(4, "focused", MirrorValue::Bool(false));
    rig.put(5, "focused", MirrorValue::Bool(true));
    rig.rec("history", json!(history(&rig, None).unwrap()));
    rig.step("five_focused", &[]);
    rig.put(5, "focused", MirrorValue::Bool(false));
    rig.put(3, "focused", MirrorValue::Bool(true));
    rig.rec("history", json!(history(&rig, None).unwrap()));
    rig.rec("limited", json!(history(&rig, Some(2)).unwrap()));
    rig.step("three_focused", &[]);
    rig.destroy(4);
    rig.rec("history", json!(history(&rig, None).unwrap()));
    rig.step("a_closed_window_leaves_the_history", &[]);
    // The filters of a history are a record: a key that is not a field cannot be written.
    rig.step("refusals", &["refused"]);
    rig.finish();
}

#[test]
fn focus_history_follows_the_seat() {
    let rig = Rig::new("model_focus_history", &[(5, "three", "app.three")]);
    rig.boot();
    let focus = rig.object(rig.gnoblin, "focus");
    let order = |rig: &Rig| rig.ids(&rig.call(focus, "history", &[]).expect("a history"));
    assert_eq!(order(&rig), ["3", "4", "5"]);
    rig.comp.external_set(21, "focus", MirrorValue::Node(5));
    rig.settle();
    assert_eq!(order(&rig), ["5", "3", "4"], "the window the seat names comes first");
    rig.comp.external_set(21, "focus", MirrorValue::Node(4));
    rig.settle();
    assert_eq!(order(&rig), ["4", "5", "3"]);
    rig.destroy(4);
    assert_eq!(order(&rig), ["5", "3"], "a closed window leaves the history");
    let clients = rig.clients();
    rig.comp.external_set(21, "focus", MirrorValue::Node(3));
    rig.settle();
    let focused = rig.prop(clients, "focused").object().expect("the focused window");
    assert_eq!(
        text_of(
            rig.api,
            &Val::Object {
                class: rig.class("client"),
                obj: focused
            },
            "id"
        ),
        "3"
    );
    let class = rig.class("focus_filter");
    let names: Vec<String> = rig
        .api
        .catalog()
        .expect("the catalog")
        .iter()
        .filter(|item| item.entry == abi::CATALOG_MEMBER && item.class == class && item.kind == abi::MEMBER_PROPERTY)
        .map(|item| item.name.clone())
        .collect();
    let limited = |limit: i64| {
        let filter = Val::Record {
            class,
            fields: names
                .iter()
                .map(|name| if name == "limit" { Val::Int(limit) } else { Val::None })
                .collect(),
        };
        rig.call(focus, "history", &[filter])
    };
    for refused in [0, 257, -1] {
        let error = limited(refused).expect_err("a limit outside 1 to 256 is refused");
        assert_eq!(error.code, abi::E_TYPE);
        assert!(
            last_error().contains("limit is an integer from 1 to 256"),
            "{}",
            last_error()
        );
    }
    assert_eq!(rig.ids(&limited(1).unwrap()), ["3"]);
}

#[test]
fn timers_run_once_in_their_time() {
    let rig = Rig::new("model_timers", &[]);
    rig.boot();
    let (api, timers) = (rig.api, rig.object(rig.gnoblin, "timers"));
    let t0 = rig.call(timers, "now", &[]).unwrap().double().unwrap();
    let after = |rig: &Rig, ms: f64, line: &'static str| -> Obj {
        let log = Rc::clone(&rig.log);
        let run = Val::Closure(closure(move |_, _| {
            log.borrow_mut().push(json!(line));
            Ok(Val::None)
        }));
        rig.call(timers, "after", &[Val::Double(ms), run])
            .expect("a timer")
            .object()
            .expect("an object")
    };
    let _ = api;
    after(&rig, 100.0, "after 100");
    let cancelled = after(&rig, 50.0, "after 50 cancelled");
    rig.call(cancelled, "cancel", &[]).expect("cancelled");
    assert!(!rig.prop(cancelled, "pending").bool().unwrap());
    after(&rig, 0.0, "after 0");
    after(&rig, 100.0, "after 100 again");
    rig.rec("now", number(t0));
    rig.settle();
    rig.step("scheduled", &[]);
    rig.advance(60);
    rig.step("sixty", &[]);
    rig.advance(60);
    let t = rig.call(timers, "now", &[]).unwrap().double().unwrap();
    rig.rec("elapsed", number(t - t0));
    rig.step("hundred_and_twenty", &[]);
    rig.finish();
}

#[test]
fn an_object_that_is_gone_keeps_its_last_values_and_refuses_a_write() {
    let rig = Rig::new("model_dead_objects", &[]);
    rig.boot();
    let (c, o) = (rig.client(3), rig.output(0));
    let flags = |rig: &Rig| {
        json!([
            rig.prop(c, "alive").bool().unwrap(),
            rig.prop(o, "alive").bool().unwrap(),
            rig.prop(c, "title").text().unwrap(),
            number(rig.prop(c, "width").double().unwrap()),
        ])
    };
    rig.rec("alive", flags(&rig));
    rig.step("alive", &[]);
    rig.destroy(3);
    let clients = rig.clients();
    rig.rec(
        "dead",
        json!([
            rig.prop(c, "alive").bool().unwrap(),
            rig.prop(c, "title").text().unwrap(),
            number(rig.prop(c, "width").double().unwrap()),
            number(rig.prop(clients, "length").uint().unwrap() as f64),
            rig.call(clients, "get", &[Val::Text("3".into())])
                .expect("a lookup that finds nothing")
                .is_none(),
        ]),
    );
    let refused = rig
        .set(c, "opacity", &Val::Double(0.5))
        .expect_err("a dead window refuses a write");
    assert_eq!(refused.code, abi::E_NO_SUCH_NODE);
    assert!(
        refused.message.contains("Client#3.opacity: the client is gone"),
        "{}",
        refused.message
    );
    rig.step("window_closed", &["write"]);
    rig.destroy(20);
    let outputs = rig.outputs();
    rig.rec(
        "dead",
        json!([
            rig.prop(o, "alive").bool().unwrap(),
            rig.prop(o, "alive").bool().unwrap(),
            number(rig.prop(outputs, "length").uint().unwrap() as f64),
        ]),
    );
    rig.step("output_removed", &[]);
    rig.finish();
}

#[test]
fn a_property_bound_to_a_function_follows_what_it_reads() {
    let rig = Rig::new("model_dead_objects", &[]);
    rig.boot();
    let c = rig.client(3);
    let bindings = rig.object(c, "properties");
    let api = rig.api;
    let width = rig.member("client", abi::MEMBER_PROPERTY, "width");
    let compute = Val::Closure(closure(move |_, _| {
        let value = api.get(c, width)?.double()?;
        Ok(Val::Double(value / 2.0))
    }));
    let binding = rig
        .call(bindings, "bind_double", &[Val::Text("opacity".into()), compute])
        .expect("a binding");
    rig.settle();
    assert_eq!(
        rig.prop(c, "opacity").double().unwrap(),
        200.0,
        "the first run writes half the width"
    );
    rig.put(3, "width", double(100.0));
    rig.settle();
    assert_eq!(
        rig.prop(c, "opacity").double().unwrap(),
        50.0,
        "a change of what it read runs it again"
    );
    let binding = binding.object().unwrap();
    rig.call(binding, "dispose", &[]).expect("disposed");
    rig.put(3, "width", double(40.0));
    assert_eq!(
        rig.prop(c, "opacity").double().unwrap(),
        50.0,
        "a disposed binding is not run"
    );
}

#[test]
fn a_selection_is_refused_a_priority_that_is_not_a_number() {
    let rig = Rig::new("model_collections", &[]);
    let clients = rig.clients();
    let class = rig.class("client_query");
    let names: Vec<String> = rig
        .api
        .catalog()
        .expect("the catalog")
        .iter()
        .filter(|item| item.entry == abi::CATALOG_MEMBER && item.class == class && item.kind == abi::MEMBER_PROPERTY)
        .map(|item| item.name.clone())
        .collect();
    let query = Val::Record {
        class,
        fields: names.iter().map(|_| Val::None).collect(),
    };
    let error = rig
        .call(clients, "where", &[query, Val::Double(f64::NAN)])
        .expect_err("a priority that is not finite is refused");
    assert_eq!(error.code, abi::E_TYPE);
    assert!(
        last_error().contains("where: priority must be a finite number"),
        "{}",
        last_error()
    );
    // The roots cannot be changed, and a window cannot be made by a shell.
    let add = rig.call(
        clients,
        "add",
        &[Val::Object {
            class: rig.class("client"),
            obj: rig.client(3),
        }],
    );
    assert_eq!(add.expect_err("a root is not changeable").code, abi::E_REFUSED);
    assert!(rig.api.create(rig.class("client"), &[]).is_err());
    assert!(rig.api.create(rig.class("output"), &[]).is_err());
}

#[test]
fn a_list_of_a_shell_tells_its_listeners_what_joins_and_leaves() {
    let rig = Rig::new("model_dead_objects", &[]);
    rig.boot();
    let list = rig.api.create(rig.class("client_collection"), &[]).expect("a list");
    assert!(rig.prop(list, "mutable").bool().unwrap());
    let (three, four) = (rig.client(3), rig.client(4));
    let value = |obj: Obj| Val::Object {
        class: rig.class("client"),
        obj,
    };
    let log = Rc::clone(&rig.log);
    let api = rig.api;
    let added = rig.signal_of(list, "added");
    api.listen(list, added, move |_, args| {
        log.borrow_mut()
            .push(json!(format!("added {}", text_of(api, &args[0], "id"))));
        Ok(Val::None)
    })
    .unwrap();
    let log = Rc::clone(&rig.log);
    let removed = rig.signal_of(list, "removed");
    api.listen(list, removed, move |_, args| {
        log.borrow_mut()
            .push(json!(format!("removed {}", text_of(api, &args[0], "id"))));
        Ok(Val::None)
    })
    .unwrap();
    let log = Rc::clone(&rig.log);
    let title = rig.signal_of(list, "title_changed");
    api.listen(list, title, move |_, args| {
        log.borrow_mut()
            .push(json!(format!("title {}", text_of(api, &args[0], "id"))));
        Ok(Val::None)
    })
    .unwrap();
    rig.call(list, "add", &[value(three)]).unwrap();
    rig.call(list, "add", &[value(four)]).unwrap();
    rig.call(list, "swap", &[value(three), value(four)]).unwrap();
    assert_eq!(rig.ids(&rig.call(list, "to_list", &[]).unwrap()), ["4", "3"]);
    rig.settle();
    rig.put(4, "title", text("x"));
    rig.destroy(3);
    assert!(rig.call(list, "remove", &[value(four)]).unwrap().bool().unwrap());
    assert!(!rig.call(list, "remove", &[value(four)]).unwrap().bool().unwrap());
    rig.settle();
    let lines: Vec<String> = rig
        .log
        .borrow()
        .iter()
        .map(|line| line.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(lines, ["added 3", "added 4", "title 4", "removed 3", "removed 4"]);
}
