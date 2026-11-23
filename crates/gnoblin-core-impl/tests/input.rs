#![allow(unsafe_code)]
//! The input matcher of `libgnoblin-core` (`gnoblin.model` `input`) against the fixtures the TypeScript library recorded
//! (`fixtures/model/input_{grammar,chords}.json`): the trigger grammar and its refusals, the claims a shell makes and shares, the tiers,
//! priority and decline of a press, a device that is binding-only, and a chord handed from one generation to the next across a reload.
//!
//! The scenarios are replayed over the same fake compositor and the same actions, through the object protocol's table and nothing else. What
//! only a dynamically typed shell can do (a layout class of the standard library, a `KeyBinding` instance) is written as the call of the
//! object protocol it stands for: a layout is a collection a shell made, and a binding with an id and a priority is a `bind` with options.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gnoblin_client::Committed;
use gnoblin_config::Config;
use gnoblin_core_abi::{self as abi, GnoblinStr};
use gnoblin_core_impl::document::{DocumentLink, DocumentLinkProvider};
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_object::client::{Api, closure};
use gnoblin_object::{Fail, MemberId, Obj, Res, Val};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
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

/// The session of the oracle's `World::input`: a 1000 by 800 screen, the windows `3` (focused, at 10,20 and 300 by 200) and `4`, and the seat
/// `seatA` (node 5) with one keyboard, `Macro Pad`.
fn world(comp: &FakeCompositor, second_seat: bool) {
    world_with(comp, second_seat, &["7:Macro Pad:keyboard"]);
}

/// [`world`] with `devices` on the first seat, one `id:name:capabilities` item each.
fn world_with(comp: &FakeCompositor, second_seat: bool, devices_of_seat_a: &[&str]) {
    comp.add_desktop(1000.0, 800.0);
    comp.add_node(
        3,
        ROOT,
        "window",
        vec![
            ("title", text("win1")),
            ("app_id", text("foot")),
            ("focused", MirrorValue::Bool(true)),
            ("x", MirrorValue::Double(10.0)),
            ("y", MirrorValue::Double(20.0)),
            ("width", MirrorValue::Double(300.0)),
            ("height", MirrorValue::Double(200.0)),
        ],
    );
    comp.add_node(
        4,
        ROOT,
        "window",
        vec![
            ("title", text("web")),
            ("app_id", text("firefox")),
            ("x", MirrorValue::Double(400.0)),
            ("y", MirrorValue::Double(20.0)),
            ("width", MirrorValue::Double(300.0)),
            ("height", MirrorValue::Double(200.0)),
        ],
    );
    let seat = |id: u64, name: &str, devices: &[&str]| {
        comp.add_node(
            id,
            ROOT,
            "seat",
            vec![
                ("name", text(name)),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
                ("focus", MirrorValue::Node(3)),
                (
                    "device",
                    MirrorValue::Texts(devices.iter().map(|device| (*device).to_owned()).collect()),
                ),
            ],
        );
    };
    seat(5, "seatA", devices_of_seat_a);
    if second_seat {
        seat(6, "seatB", &[]);
    }
}

type Log = Rc<RefCell<Vec<Value>>>;

/// A shell over the fake compositor: a generation that attached, and the table of `gnoblin.model` to call.
struct Rig {
    ctx: u64,
    api: Api,
    comp: FakeCompositor,
    log: Log,
    input: Obj,
    claim_calls_seen: usize,
    /// Whether this generation made the session and ends it, or is a later one of the same session.
    owner: bool,
    closed: bool,
    _lock: Option<MutexGuard<'static, ()>>,
}

fn api_of(ctx: u64) -> Api {
    let mut raw: *const abi::GnoblinObjectApi = std::ptr::null();
    // SAFETY: `raw` is a live local the library writes.
    let code = unsafe { (host::api().object_api)(ctx, GnoblinStr::borrowed("gnoblin.model"), &raw mut raw) };
    assert_eq!(code, abi::E_OK, "{}", last_error());
    // SAFETY: the table `object_api` returns is static.
    Api::new(unsafe { &*raw }, ctx, last_error)
}

fn member_of(api: Api, obj: Obj, kind: u32, name: &str) -> Res<MemberId> {
    let class = api.class_of(obj)?;
    api.member(class, kind, name)
}

fn gnoblin_input(api: Api) -> Obj {
    let gnoblin = api
        .create(api.class("gnoblin").expect("the service class"), &[])
        .expect("the service");
    let member = member_of(api, gnoblin, abi::MEMBER_PROPERTY, "input").expect("the input property");
    api.get(gnoblin, member)
        .expect("the input manager")
        .object()
        .expect("an object")
}

impl Rig {
    fn new(second_seat: bool) -> Rig {
        Rig::over(|comp| world(comp, second_seat))
    }

    /// A shell over the world `setup` makes in the fake compositor.
    fn over(setup: impl FnOnce(&FakeCompositor)) -> Rig {
        Rig::in_language(abi::LANG_NATIVE, setup)
    }

    /// A shell written in `language` over the world `setup` makes in the fake compositor.
    fn in_language(language: u32, setup: impl FnOnce(&FakeCompositor)) -> Rig {
        let lock = process_lock();
        host::reset_session();
        let comp = FakeCompositor::new();
        setup(&comp);
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        host::set_clock(Arc::new(ManualClock::default()));
        let ctx = host::open_generation("input", language).expect("a generation opens");
        // SAFETY: `tree_attach` is a function of the core's table.
        assert_eq!(unsafe { (host::api().tree_attach)(ctx, abi::ATTACH_HOLDER) }, abi::E_OK);
        let api = api_of(ctx);
        let input = gnoblin_input(api);
        Rig {
            ctx,
            api,
            comp,
            log: Rc::new(RefCell::new(Vec::new())),
            input,
            claim_calls_seen: 0,
            owner: true,
            closed: false,
            _lock: Some(lock),
        }
    }

    /// A generation of the same session that is only staged, as a reload makes while the old one serves.
    fn staged(&self) -> Rig {
        let ctx = host::stage_generation("input-next", abi::LANG_NATIVE);
        let api = api_of(ctx);
        let input = gnoblin_input(api);
        Rig {
            ctx,
            api,
            comp: self.comp.clone(),
            log: Rc::new(RefCell::new(Vec::new())),
            input,
            claim_calls_seen: 0,
            owner: false,
            closed: false,
            _lock: None,
        }
    }

    /// Ends the generation, as a reload does to the one it replaces.
    fn end(&mut self) {
        if !self.closed {
            self.closed = true;
            host::close_generation(self.ctx);
        }
    }

    fn class(&self, name: &str) -> u32 {
        self.api
            .class(name)
            .unwrap_or_else(|error| panic!("no class {name}: {error}"))
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

    fn ok(&self, obj: Obj, name: &str, args: &[Val]) -> Val {
        self.call(obj, name, args)
            .unwrap_or_else(|error| panic!("{name}: {}", error.message))
    }

    fn object(&self, obj: Obj, name: &str) -> Obj {
        self.prop(obj, name)
            .object()
            .unwrap_or_else(|error| panic!("{name} is not an object: {}", error.message))
    }

    fn function(&self, class: &str, name: &str, args: &[Val]) -> Res<Val> {
        let class = self.class(class);
        let member = self.api.member(class, abi::MEMBER_METHOD, name).expect("a function");
        self.api.call_static(class, member, args)
    }

    fn primary(&self) -> Obj {
        self.object(self.input, "primary")
    }

    fn devices(&self, context: Obj) -> Vec<Obj> {
        self.prop(context, "devices")
            .list()
            .expect("a list")
            .iter()
            .map(|device| device.object().expect("a device"))
            .collect()
    }

    /// `binding_options` with the fields the caller names.
    fn options(&self, id: Option<&str>, priority: Option<f64>, only: Option<bool>) -> Val {
        Val::Record {
            class: self.class("binding_options"),
            fields: vec![
                id.map_or(Val::None, |id| Val::Text(id.to_owned())),
                priority.map_or(Val::None, Val::Double),
                Val::None,
                only.map_or(Val::None, Val::Bool),
                Val::None,
            ],
        }
    }

    /// A function for `bind`: it logs `label` and what the event says, and answers `answer` (`None` answers nothing).
    fn action(&self, label: &'static str, answer: Option<bool>) -> Val {
        let log = Rc::clone(&self.log);
        let api = self.api;
        Val::Closure(closure(move |_, args| {
            let fields = args[0].record().expect("an event");
            log.borrow_mut()
                .push(json!({ "label": label, "event": event_summary(api, fields) }));
            Ok(answer.map_or(Val::None, Val::Bool))
        }))
    }

    /// Like [`Rig::action`] for a binding that records only its label, as the fixture's `rec('low')` does.
    fn plain(&self, label: &'static str, answer: Option<bool>) -> Val {
        let log = Rc::clone(&self.log);
        Val::Closure(closure(move |_, _| {
            log.borrow_mut().push(json!({ "label": label }));
            Ok(answer.map_or(Val::None, Val::Bool))
        }))
    }

    fn bind(&self, holder: Obj, trigger: &str, action: Val, options: Val) -> Res<Obj> {
        self.call(holder, "bind", &[Val::Text(trigger.to_owned()), action, options])
            .and_then(|binding| binding.object())
    }

    /// Runs the turns the shell would: the pump, the queued calls.
    fn settle(&self) {
        for _ in 0..8 {
            // SAFETY: `dispatch` is a function of the core's table.
            unsafe { (host::api().dispatch)(self.ctx, 0) };
        }
    }

    /// The claim whose text is `text` (a device and `|` first when it is for one device), pressed as `state`.
    fn chord(&self, claim: &str, state: &str) {
        self.settle();
        let (device, chord) = claim
            .rsplit_once('|')
            .map_or(("", claim), |(device, chord)| (device, chord));
        let wanted = if device.is_empty() {
            chord.to_owned()
        } else {
            format!("{device}\n{chord}")
        };
        let id = self.comp.claim_id(&wanted);
        self.comp.chord(id, chord, state);
        self.settle();
    }

    /// The claim calls the shell made since the last look, as `(id, method, text)`.
    fn claim_calls(&mut self) -> Vec<(i64, String, String)> {
        let calls: Vec<(i64, String, String)> = self
            .comp
            .borrow()
            .claim_calls
            .iter()
            .skip(self.claim_calls_seen)
            .map(|(method, id, _, text)| (*id, method.clone(), text.replace('\n', "|")))
            .collect();
        self.claim_calls_seen += calls.len();
        calls
    }

    fn drain_log(&self) -> Vec<Value> {
        self.log.borrow_mut().drain(..).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.end();
        if self.owner {
            host::reset_session();
        }
    }
}

/// What a closure of a binding logs of its event: the trigger and the id of the window with the focus.
fn event_summary(api: Api, fields: &[Val]) -> Value {
    let id_of = |value: &Val| match value {
        Val::Object { obj, .. } => {
            let member = member_of(api, *obj, abi::MEMBER_PROPERTY, "id").expect("an id");
            json!(api.get(*obj, member).expect("a value").text().expect("text"))
        }
        _ => Value::Null,
    };
    json!([fields[2].text().expect("a trigger"), id_of(&fields[7])])
}

fn fixture(name: &str) -> Value {
    Fixtures::new("crates/gnoblin-core-impl/tests/fixtures/model").read_json(name)
}

fn expected_log(step: &Value) -> Vec<Value> {
    step["log"].as_array().expect("a log").clone()
}

/// The entries of a step's log the replay can compare: the ones that name a label.
fn entry(label: &str, value: Option<Value>) -> Value {
    match value {
        Some(value) => json!({ "label": label, "value": value }),
        None => json!({ "label": label }),
    }
}

// ---- the trigger grammar --------------------------------------------------------------------------------------------------------

/// What the fixture's JSON for a claim says the claim asks the seat for: a chord's text, a gesture's kind and finger count, or nothing.
fn claim_text_of(json_text: &str) -> String {
    let between = |from: &str, to: &str| -> String {
        json_text
            .split_once(from)
            .and_then(|(_, rest)| rest.split_once(to))
            .map(|(found, _)| found.to_owned())
            .unwrap_or_default()
    };
    if json_text.contains("\"kind\":\"chord\"") {
        between("\"text\":\"", "\"")
    } else if json_text.contains("\"kind\":\"gesture\"") {
        format!("{}{}", between("\"gesture\":\"", "\""), between("\"fingers\":", "}"))
    } else {
        String::new()
    }
}

/// The fixture's text for what the library refused: `TypeError: <message> [<token>]`.
fn refusal_text(fail: &Fail) -> String {
    words(fail)
}

/// What a failure says, without the `E_CODE: ` the client puts before it.
fn words(fail: &Fail) -> String {
    match fail.message.split_once(": ") {
        Some((code, rest)) if code.starts_with("E_") => rest.to_owned(),
        _ => fail.message.clone(),
    }
}

#[test]
fn the_trigger_grammar_accepts_and_refuses_what_the_typescript_one_did() {
    let rig = Rig::new(false);
    let fixture = fixture("input_grammar");
    let steps = fixture["steps"].as_array().expect("steps");
    let accepted = steps
        .iter()
        .find(|step| step["step"] == "accepted")
        .expect("the accepted step");
    let entries = accepted["log"].as_array().expect("a log");
    let canonical = entries
        .iter()
        .find(|entry| entry["label"] == "canonical")
        .expect("canonical");
    for row in canonical["value"].as_array().expect("rows") {
        let (given, want_text, want_modifiers) = (
            row[0].as_str().expect("text"),
            row[1].as_str().expect("text"),
            row[2].as_str().expect("text"),
        );
        let parsed = rig
            .function("triggers", "parse", &[Val::Text(given.to_owned()), Val::None])
            .unwrap_or_else(|error| panic!("{given}: {}", error.message));
        let fields = parsed.record().expect("a record");
        assert_eq!(fields[0].text().expect("text"), want_text, "{given}");
        let modifiers: Vec<&str> = fields[2]
            .list()
            .expect("a list")
            .iter()
            .map(|modifier| modifier.text().expect("text"))
            .collect();
        assert_eq!(modifiers.join(","), want_modifiers, "{given}");
        let checked = rig
            .function("triggers", "check", &[Val::Text(given.to_owned()), Val::None])
            .expect("accepted");
        assert_eq!(checked.text().expect("text"), want_text, "{given}");
    }
    let claims = entries.iter().find(|entry| entry["label"] == "claims").expect("claims");
    for row in claims["value"].as_array().expect("rows") {
        let (given, want) = (row[0].as_str().expect("text"), row[1].as_str().expect("text"));
        let claim = rig
            .function("triggers", "claim_text", &[Val::Text(given.to_owned()), Val::None])
            .expect("a claim text");
        let claim = claim.text().expect("text");
        assert_eq!(claim, claim_text_of(want), "{given}");
    }
    let refused = steps
        .iter()
        .find(|step| step["step"] == "refused")
        .expect("the refused step");
    let entries = refused["log"].as_array().expect("a log");
    let refused = entries
        .iter()
        .find(|entry| entry["label"] == "refused")
        .expect("refused");
    for row in refused["value"].as_array().expect("rows") {
        let (given, want) = (row[0].as_str().expect("text"), row[1].as_str().expect("text"));
        let message = match rig.function(
            "triggers",
            "parse",
            &[Val::Text(given.to_owned()), Val::Text("trigger".to_owned())],
        ) {
            Ok(_) => "accepted".to_owned(),
            Err(fail) => refusal_text(&fail),
        };
        let want = want.strip_prefix("TypeError: ").expect("a type error");
        let want = want.rsplit_once(" [").map_or(want, |(message, _)| message);
        assert_eq!(message, want, "{given:?}");
    }
}

// ---- chords, tiers, priority and decline ---------------------------------------------------------------------------------------

#[test]
#[allow(clippy::too_many_lines)]
fn chords_and_tiers_run_as_the_typescript_matcher_did() {
    let mut rig = Rig::new(false);
    let fixture = fixture("input_chords");
    let steps = fixture["steps"].as_array().expect("steps").clone();
    let step = |name: &str| {
        steps
            .iter()
            .find(|step| step["step"] == name)
            .unwrap_or_else(|| panic!("no step {name}"))
            .clone()
    };
    let calls_of = |step: &Value| -> Vec<(i64, String, String)> {
        step["claim_calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .map(|call| {
                        (
                            call["id"].as_i64().expect("an id"),
                            call["method"].as_str().expect("a method").to_owned(),
                            call["text"].as_str().expect("text").to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // The session of the scenario.
    let input = rig.primary();
    rig.bind(input, "Super+Shift+1", rig.action("fired", None), Val::None)
        .expect("bound");
    rig.bind(input, "Super+k", rig.plain("low", None), Val::None)
        .expect("bound");
    let high = rig
        .bind(
            input,
            "Super+k",
            rig.plain("high", None),
            rig.options(Some("high"), Some(5.0), None),
        )
        .expect("bound");
    rig.bind(
        input,
        "Super+a",
        Val::Closure(closure(|_, _| Err(Fail::err("boom")))),
        Val::None,
    )
    .expect("bound");
    rig.bind(input, "Super+b", rig.plain("b", None), Val::None)
        .expect("bound");
    rig.bind(input, "Super+z", Val::None, Val::None).expect("bound");
    rig.bind(
        input,
        "Super+d",
        Val::Closure(closure(|_, _| Ok(Val::Bool(false)))),
        Val::None,
    )
    .expect("bound");
    rig.bind(
        input,
        "Super+d",
        rig.plain("second", None),
        rig.options(Some("second"), Some(-1.0), None),
    )
    .expect("bound");
    let layout = rig
        .api
        .create(rig.class("binding_collection"), &[])
        .expect("a collection");
    rig.bind(
        layout,
        "Super+x",
        rig.plain("layout", None),
        rig.options(Some("layout"), None, None),
    )
    .expect("bound");
    let scope = Val::Closure(closure({
        let api = rig.api;
        move |_, args| {
            let client = args[0].object()?;
            let id = api.get(client, member_of(api, client, abi::MEMBER_PROPERTY, "id")?)?;
            Ok(Val::Bool(id.text()? == "3"))
        }
    }));
    rig.ok(input, "attach_layout", &[Val::Object { class: 0, obj: layout }, scope]);
    rig.bind(input, "Super+x", rig.plain("context", None), Val::None)
        .expect("bound");
    let devices = rig.devices(input);
    assert_eq!(devices.len(), 1);
    rig.bind(
        devices[0],
        "Super+x",
        rig.plain("device declines", Some(false)),
        Val::None,
    )
    .expect("bound");
    let conflict = rig
        .bind(input, "Super+x", rig.plain("never", None), Val::None)
        .expect_err("two bindings of one trigger at one priority");
    rig.settle();

    // boot
    let boot = step("boot");
    assert_eq!(
        rig.claim_calls(),
        calls_of(&boot),
        "the claims of the session, in order"
    );
    assert_eq!(
        rig.comp.claimed(),
        boot["claims_held"]
            .as_array()
            .expect("claims")
            .iter()
            .map(|claim| claim.as_str().expect("text").to_owned())
            .collect::<Vec<_>>()
    );
    assert!(rig.drain_log().is_empty());

    // claimed
    let claimed = step("claimed");
    let want = expected_log(&claimed);
    let got = vec![entry(
        "conflict",
        Some(json!(format!("BindingConflictError: {}", words(&conflict)))),
    )];
    assert_eq!(got, want, "the conflict names both bindings");

    let check = |rig: &mut Rig, name: &str, act: &dyn Fn(&Rig)| {
        let expected = step(name);
        act(rig);
        let got = rig.drain_log();
        let want: Vec<Value> = expected_log(&expected)
            .into_iter()
            .map(|logged| {
                // The fixture's `fired` carries `[trigger, client id]` as its value; the replay logs the same under `event`.
                match logged.get("value") {
                    Some(value) if logged["label"] == "fired" => json!({ "label": "fired", "event": value }),
                    _ => logged,
                }
            })
            .collect();
        assert_eq!(got, want, "step `{name}`");
        assert_eq!(rig.claim_calls(), calls_of(&expected), "the claim calls of `{name}`");
        assert_eq!(
            rig.comp.claimed(),
            expected["claims_held"]
                .as_array()
                .expect("claims")
                .iter()
                .map(|claim| claim.as_str().expect("text").to_owned())
                .collect::<Vec<_>>(),
            "the claims held after `{name}`"
        );
    };
    check(&mut rig, "fires", &|rig| rig.chord("<Shift><Super>1", "press"));
    check(&mut rig, "only_the_press_runs", &|rig| {
        rig.chord("<Shift><Super>1", "repeat");
        rig.chord("<Shift><Super>1", "release");
    });
    check(&mut rig, "higher_priority_first", &|rig| rig.chord("<Super>k", "press"));
    check(&mut rig, "unbind_high", &|rig| {
        rig.ok(high, "unbind", &[]);
        rig.settle();
    });
    check(&mut rig, "lower_priority_after", &|rig| rig.chord("<Super>k", "press"));
    check(&mut rig, "a_throwing_handler", &|rig| {
        rig.chord("<Super>a", "press");
        rig.chord("<Super>b", "press");
    });
    check(&mut rig, "the_same_failure_again", &|rig| {
        rig.chord("<Super>a", "press")
    });
    check(&mut rig, "false_swallows_and_a_returned_false_declines", &|rig| {
        rig.chord("<Super>z", "press");
        rig.chord("<Super>d", "press");
    });
    check(&mut rig, "layout_beats_context", &|rig| rig.chord("<Super>x", "press"));
    check(&mut rig, "device_first_then_decline_falls_through", &|rig| {
        rig.chord("Macro Pad|<Super>x", "press")
    });
    check(&mut rig, "binding_only", &|rig| {
        let device = rig.devices(rig.primary())[0];
        rig.set(device, "binding_only", &Val::Bool(true)).expect("claimed");
        rig.settle();
    });
    check(&mut rig, "binding_only_given_up", &|rig| {
        let device = rig.devices(rig.primary())[0];
        rig.set(device, "binding_only", &Val::Bool(false)).expect("released");
        rig.settle();
    });
}

// ---- the handover of a chord across a reload -------------------------------------------------------------------------------------

#[test]
fn a_chord_is_handed_from_one_generation_to_the_next_without_the_compositor_seeing_it_claimed_twice() {
    let mut first = Rig::new(false);
    first.comp.refuse_duplicate_claims();
    let input = first.primary();
    first
        .bind(input, "Super+k", first.plain("first", None), Val::None)
        .expect("bound");
    first.settle();
    assert_eq!(
        first.claim_calls(),
        vec![(1, "claim".to_owned(), "<Super>k".to_owned())]
    );

    // The new generation binds the same chord while it is only staged: its claim is kept, and not sent.
    let mut second = first.staged();
    let next = second.primary();
    second
        .bind(next, "Super+k", second.plain("second", None), Val::None)
        .expect("bound");
    second.settle();
    assert_eq!(
        second.claim_calls().len(),
        1,
        "the compositor has been told of the first claim only"
    );
    assert!(
        first.comp.borrow().refused_claims.is_empty(),
        "the chord was not claimed twice"
    );
    assert_eq!(first.comp.claimed(), ["<Super>k"]);

    // Until the takeover the old generation has the chord.
    first.chord("<Super>k", "press");
    second.settle();
    assert_eq!(first.drain_log(), vec![json!({ "label": "first" })]);
    assert!(second.drain_log().is_empty(), "a staged generation hears nothing");

    // The takeover releases the old claim and sends the new one in the same transaction, in that order.
    host::take_over(second.ctx).expect("the staged generation takes over");
    first.settle();
    second.settle();
    first.end();
    second.settle();
    let calls = first
        .comp
        .borrow()
        .claim_calls
        .iter()
        .map(|(method, id, _, text)| (method.clone(), *id, text.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        calls,
        vec![
            ("claim".to_owned(), 1, "<Super>k".to_owned()),
            ("unclaim".to_owned(), 1, String::new()),
            ("claim".to_owned(), 2, "<Super>k".to_owned()),
        ],
        "the old claim goes first and the new one follows it"
    );
    assert!(
        first.comp.borrow().refused_claims.is_empty(),
        "the chord was never claimed twice"
    );
    assert_eq!(first.comp.claimed(), ["<Super>k"]);

    // The chord is the new generation's now, and only its.
    second.chord("<Super>k", "press");
    assert_eq!(second.drain_log(), vec![json!({ "label": "second" })]);
    assert!(first.drain_log().is_empty(), "the ended generation hears nothing");
    let claims = second.ok(next, "claims", &[]);
    let claims = claims.list().expect("a list");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].record().expect("a record")[0].int().expect("an id"), 2);
}

// ---- claims, retriggering, refusals -----------------------------------------------------------------------------------------------

#[test]
fn bindings_of_one_trigger_share_a_claim_and_the_last_one_to_go_gives_it_up() {
    let mut rig = Rig::new(false);
    let input = rig.primary();
    let a = rig
        .bind(input, "Super+q", rig.plain("a", None), Val::None)
        .expect("bound");
    let b = rig
        .bind(
            input,
            "super+Q",
            rig.plain("b", None),
            rig.options(None, Some(1.0), None),
        )
        .expect("bound");
    rig.settle();
    assert_eq!(
        rig.claim_calls(),
        vec![(1, "claim".to_owned(), "<Super>q".to_owned())],
        "one claim for two bindings"
    );
    let claims = rig.ok(input, "claims", &[]);
    let claim = claims.list().expect("a list")[0].record().expect("a record").to_vec();
    assert_eq!(claim[1].text().expect("text"), "chord:<Super>q");
    assert_eq!(claim[4].uint().expect("refs"), 2);
    rig.ok(a, "unbind", &[]);
    rig.settle();
    assert!(rig.claim_calls().is_empty(), "a claim another binding stands on stays");
    rig.ok(b, "unbind", &[]);
    rig.settle();
    assert_eq!(rig.claim_calls(), vec![(1, "unclaim".to_owned(), String::new())]);
    assert!(rig.comp.claimed().is_empty());
}

#[test]
fn a_binding_moved_to_another_trigger_moves_its_claim_and_a_refusal_leaves_it_where_it_was() {
    let mut rig = Rig::new(false);
    let input = rig.primary();
    let binding = rig
        .bind(input, "Super+q", rig.plain("q", None), Val::None)
        .expect("bound");
    rig.bind(input, "Super+w", rig.plain("w", None), Val::None)
        .expect("bound");
    rig.settle();
    rig.claim_calls();
    rig.set(binding, "trigger", &Val::Text("Super+e".to_owned()))
        .expect("moved");
    rig.settle();
    assert_eq!(
        rig.claim_calls(),
        vec![
            (3, "claim".to_owned(), "<Super>e".to_owned()),
            (1, "unclaim".to_owned(), String::new())
        ]
    );
    let refused = rig
        .set(binding, "trigger", &Val::Text("Super+w".to_owned()))
        .expect_err("taken at that priority");
    assert!(words(&refused).contains("conflicts with"), "{}", refused.message);
    let bad = rig
        .set(binding, "trigger", &Val::Text("Super+!".to_owned()))
        .expect_err("not a trigger");
    assert!(
        words(&bad).contains("is a character, not a key name"),
        "{}",
        bad.message
    );
    assert_eq!(rig.prop(binding, "trigger").text().expect("text"), "Super+e");
    rig.chord("<Super>e", "press");
    assert_eq!(rig.drain_log(), vec![json!({ "label": "q" })]);
}

#[test]
fn a_wheel_trigger_is_refused_and_leaves_nothing_behind() {
    let mut rig = Rig::new(false);
    let input = rig.primary();
    let refused = rig
        .bind(input, "Super+Wheel", rig.plain("wheel", None), Val::None)
        .expect_err("the compositor has no claim for a wheel");
    assert!(
        words(&refused).contains("wheel triggers cannot be bound yet"),
        "{}",
        refused.message
    );
    assert_eq!(refused.code, abi::E_UNSUPPORTED);
    let bindings = rig.object(input, "bindings");
    assert_eq!(
        rig.prop(bindings, "length").uint().expect("a length"),
        0,
        "a refused binding is not half there"
    );
    assert!(rig.claim_calls().is_empty());
    assert_eq!(rig.ok(input, "describe_bindings", &[]).list().expect("a list").len(), 0);
}

// ---- the shell's own objects --------------------------------------------------------------------------------------------------

#[test]
fn an_object_the_shell_implements_gets_the_press_and_may_decline() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let class = rig.class("key_binding");
    let on_press = rig.api.member(class, abi::MEMBER_METHOD, "on_press").expect("a method");
    let log = Rc::clone(&rig.log);
    let handler = rig
        .api
        .implement(class, &[on_press], move |_, args| {
            log.borrow_mut()
                .push(json!({ "label": "handler", "trigger": args[0].record()?[2].text()? }));
            Ok(Val::Bool(false))
        })
        .expect("an implementation");
    rig.ok(
        input,
        "bind_binding",
        &[
            Val::Text("Super+h".to_owned()),
            Val::Object { class, obj: handler },
            rig.options(Some("h"), Some(2.0), None),
        ],
    );
    rig.bind(input, "Super+h", rig.plain("after", None), Val::None)
        .expect("bound");
    rig.chord("<Super>h", "press");
    assert_eq!(
        rig.drain_log(),
        vec![
            json!({ "label": "handler", "trigger": "Super+h" }),
            json!({ "label": "after" })
        ],
        "the handler runs first at its priority, declines, and the next binding has the press"
    );
}

#[test]
fn a_binding_with_a_scope_matches_only_the_windows_it_accepts() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let binding = rig
        .bind(input, "Super+s", rig.plain("scoped", None), Val::None)
        .expect("bound");
    rig.bind(
        input,
        "Super+s",
        rig.plain("everywhere", None),
        rig.options(Some("rest"), Some(-1.0), None),
    )
    .expect("bound");
    let api = rig.api;
    rig.set(
        binding,
        "scope",
        &Val::Closure(closure(move |_, args| {
            let client = args[0].object()?;
            let id = api.get(client, member_of(api, client, abi::MEMBER_PROPERTY, "id")?)?;
            Ok(Val::Bool(id.text()? == "4"))
        })),
    )
    .expect("a scope");
    rig.chord("<Super>s", "press");
    assert_eq!(
        rig.drain_log(),
        vec![json!({ "label": "everywhere" })],
        "window 3 has the focus, which the scope refuses"
    );
    rig.comp.external_set(5, "focus", MirrorValue::Node(4));
    rig.settle();
    rig.chord("<Super>s", "press");
    assert_eq!(rig.drain_log(), vec![json!({ "label": "scoped" })]);
}

// ---- contexts and devices -------------------------------------------------------------------------------------------------------

#[test]
fn a_context_a_shell_makes_holds_the_devices_of_the_first_seat_and_binds_per_device() {
    let mut rig = Rig::new(false);
    let context_class = rig.class("input_context");
    let name = rig
        .api
        .member(context_class, abi::MEMBER_PROPERTY, "name")
        .expect("a member");
    let pad = rig
        .api
        .create(context_class, &[(name, Val::Text("pad".to_owned()))])
        .expect("a context");
    assert!(!rig.prop(pad, "alive").bool().expect("a flag") || rig.prop(pad, "seat").uint().expect("a seat") == 0);
    assert!(rig.call(pad, "pointer", &[]).is_err() || rig.prop(pad, "seat").uint().expect("a seat") == 0);
    rig.bind(pad, "Super+q", rig.plain("pad", None), Val::None)
        .expect("a context that is in no list collects bindings");
    assert!(rig.claim_calls().is_empty(), "and makes no claim until it is added");
    rig.ok(
        rig.input,
        "add_context",
        &[Val::Object {
            class: context_class,
            obj: pad,
        }],
    );
    rig.settle();
    let seat_context = rig.primary();
    let device = rig.devices(seat_context)[0];
    assert_eq!(
        rig.devices(pad).len(),
        0,
        "the seat's context holds the devices until one is moved"
    );
    rig.set(
        device,
        "context",
        &Val::Object {
            class: context_class,
            obj: pad,
        },
    )
    .expect("moved");
    assert_eq!(rig.devices(pad), [device]);
    assert!(rig.devices(seat_context).is_empty());
    rig.settle();
    assert_eq!(
        rig.claim_calls(),
        vec![(1, "claim".to_owned(), "Macro Pad|<Super>q".to_owned())],
        "a context a shell made registers its bindings for each device it holds"
    );
    rig.chord("Macro Pad|<Super>q", "press");
    assert_eq!(rig.drain_log(), vec![json!({ "label": "pad" })]);
    let duplicate = rig
        .api
        .create(context_class, &[(name, Val::Text("pad".to_owned()))])
        .expect("a context");
    let refused = rig
        .call(
            rig.input,
            "add_context",
            &[Val::Object {
                class: context_class,
                obj: duplicate,
            }],
        )
        .expect_err("the name is taken");
    assert!(words(&refused).contains("is already there"), "{}", refused.message);
    assert!(
        rig.ok(
            rig.input,
            "remove_context",
            &[Val::Object {
                class: context_class,
                obj: pad
            }]
        )
        .bool()
        .expect("a flag")
    );
    rig.settle();
    assert_eq!(rig.devices(seat_context), [device], "the device is the seat's again");
    assert_eq!(
        rig.claim_calls(),
        vec![(1, "unclaim".to_owned(), String::new())],
        "the context's bindings stopped"
    );
    let seat_refused = rig.call(
        rig.input,
        "remove_context",
        &[Val::Object {
            class: context_class,
            obj: seat_context,
        }],
    );
    assert!(seat_refused.is_err(), "a seat's own context cannot be removed");
}

#[test]
fn a_device_that_is_unplugged_is_retired_and_the_manager_says_so() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let device = rig.devices(input)[0];
    assert_eq!(rig.prop(device, "name").text().expect("text"), "Macro Pad");
    let removed = Rc::new(RefCell::new(0));
    let heard = Rc::clone(&removed);
    let member = member_of(rig.api, rig.input, abi::MEMBER_SIGNAL, "device_removed").expect("a signal");
    let _reg = rig
        .api
        .listen(rig.input, member, move |_, _| {
            *heard.borrow_mut() += 1;
            Ok(Val::None)
        })
        .expect("a listener");
    rig.comp.external_set(5, "device", MirrorValue::Texts(Vec::new()));
    rig.settle();
    assert_eq!(*removed.borrow(), 1);
    assert!(!rig.prop(device, "alive").bool().expect("a flag"));
    assert!(rig.devices(input).is_empty());
    let refused = rig
        .set(device, "enabled", &Val::Bool(false))
        .expect_err("the compositor cannot disable a device");
    assert_eq!(refused.code, abi::E_UNSUPPORTED);
}

#[test]
fn a_seat_that_goes_ends_its_context_and_a_second_seat_gets_one() {
    let rig = Rig::new(true);
    let contexts = rig.prop(rig.input, "contexts");
    let names: Vec<String> = contexts
        .list()
        .expect("a list")
        .iter()
        .map(|context| {
            let obj = context.object().expect("a context");
            rig.prop(obj, "name").text().expect("text").to_owned()
        })
        .collect();
    assert_eq!(
        names,
        ["seatA", "seatB", "seat0"],
        "the desktop's own seat is the third"
    );
    let second = contexts.list().expect("a list")[1].object().expect("a context");
    rig.comp.destroy(6);
    rig.settle();
    assert!(
        !rig.prop(second, "alive").bool().expect("a flag"),
        "the seat went, and so did its context"
    );
    assert_eq!(rig.prop(rig.input, "contexts").list().expect("a list").len(), 2);
}

#[test]
fn the_context_reads_the_seat_and_finds_the_window_under_a_point() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let point = |x: f64, y: f64| Val::Record {
        class: rig.class("input_point"),
        fields: vec![Val::Double(x), Val::Double(y)],
    };
    let hit = rig.ok(input, "hit_test", &[point(20.0, 30.0)]);
    let fields = hit.record().expect("a hit");
    let client = fields[0].object().expect("a client");
    assert_eq!(rig.prop(client, "id").text().expect("text"), "3");
    let local = fields[1].record().expect("a point");
    assert_eq!(
        (local[0].double().expect("x"), local[1].double().expect("y")),
        (10.0, 10.0)
    );
    assert!(
        rig.ok(input, "hit_test", &[point(350.0, 30.0)]).is_none(),
        "between the windows there is none"
    );
    let focused = rig.object(input, "focused_client");
    assert_eq!(rig.prop(focused, "id").text().expect("text"), "3");
    let settings = rig.prop(input, "settings");
    assert_eq!(settings.record().expect("settings")[1].text().expect("text"), "click");
    assert!(rig.prop(input, "builtin_motion").bool().expect("a flag"));
    rig.set(input, "builtin_motion", &Val::Bool(false)).expect("written");
    rig.settle();
    assert!(!rig.prop(input, "builtin_motion").bool().expect("a flag"));
    rig.ok(input, "clear_focus", &[]);
    rig.settle();
    assert!(rig.prop(input, "focused_client").is_none());
}

// ---- pointer drags and gestures -----------------------------------------------------------------------------------------------

/// Numbers are compared as numbers: a fixture holds `10` where a replay holds `10.0`.
fn floats(value: &Value) -> Value {
    match value {
        Value::Number(number) => json!(number.as_f64().expect("a finite number")),
        Value::Array(items) => Value::Array(items.iter().map(floats).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(key, item)| (key.clone(), floats(item))).collect()),
        other => other.clone(),
    }
}

/// What the event of a drag or of a gesture said.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    phase: Option<&'static str>,
    position: (f64, f64),
    dx: Option<f64>,
    dy: Option<f64>,
    button: Option<u64>,
    progress: Option<f64>,
    client: Option<String>,
}

type Seens = Rc<RefCell<Vec<Seen>>>;

/// The step an event is: the catalog's `binding_phase`.
fn phase_name(api: Api, value: &Val) -> Option<&'static str> {
    let Val::Enum { class, value } = value else {
        return None;
    };
    ["start", "update", "end", "cancel"].into_iter().find(|name| {
        api.member(*class, abi::MEMBER_VALUE, name)
            .is_ok_and(|member| member == *value)
    })
}

/// A property of an object as text, or none when it cannot be read (an object that is gone).
fn text_of(api: Api, obj: Obj, name: &str) -> Option<String> {
    member_of(api, obj, abi::MEMBER_PROPERTY, name)
        .and_then(|member| api.get(obj, member))
        .ok()
        .and_then(|value| value.text().ok().map(str::to_owned))
}

fn number(api: Api, obj: Obj, name: &str) -> f64 {
    api.get(
        obj,
        member_of(api, obj, abi::MEMBER_PROPERTY, name).expect("a property"),
    )
    .expect("a value")
    .double()
    .expect("a number")
}

fn put(api: Api, obj: Obj, name: &str, value: f64) -> Res<()> {
    api.set(
        obj,
        member_of(api, obj, abi::MEMBER_PROPERTY, name)?,
        &Val::Double(value),
    )
}

fn seen(api: Api, event: &Val) -> Seen {
    let fields = event.record().expect("an event");
    let position = fields[6].record().expect("a point");
    let client = match &fields[7] {
        Val::Object { obj, .. } => text_of(api, *obj, "id"),
        _ => None,
    };
    Seen {
        phase: phase_name(api, &fields[10]),
        position: (position[0].double().expect("x"), position[1].double().expect("y")),
        dx: fields[12].double().ok(),
        dy: fields[13].double().ok(),
        button: fields[11].uint().ok(),
        progress: fields[17].double().ok(),
        client,
    }
}

/// The window an event of a drag or a gesture has under it.
fn client_of(event: &Val) -> Option<Obj> {
    match &event.record().ok()?[7] {
        Val::Object { obj, .. } => Some(*obj),
        _ => None,
    }
}

/// The claim calls a fixture's step records, as `(id, method, text)`.
fn fixture_calls(step: &Value) -> Vec<(i64, String, String)> {
    step["claim_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    (
                        call["id"].as_i64().expect("an id"),
                        call["method"].as_str().expect("a method").to_owned(),
                        call["text"].as_str().expect("text").to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fixture_claims(step: &Value) -> Vec<String> {
    step["claims_held"]
        .as_array()
        .expect("claims")
        .iter()
        .map(|claim| claim.as_str().expect("text").to_owned())
        .collect()
}

/// What the replay did in a step is what the fixture says: the log, the claim calls and the claims held.
#[track_caller]
fn expect_step(rig: &mut Rig, step: &Value) {
    let name = step["step"].as_str().expect("a name");
    let got: Vec<Value> = rig.drain_log().iter().map(floats).collect();
    let want: Vec<Value> = expected_log(step).iter().map(floats).collect();
    assert_eq!(got, want, "the log of step `{name}`");
    assert_eq!(rig.claim_calls(), fixture_calls(step), "the claim calls of `{name}`");
    assert_eq!(
        rig.comp.claimed(),
        fixture_claims(step),
        "the claims held after `{name}`"
    );
}

impl Rig {
    /// The pointer of the seat moves.
    fn pointer_at(&self, x: f64, y: f64) {
        self.settle();
        self.comp.set_pointer(5, x, y);
        self.settle();
    }

    /// A pointer chord (`<Super>Button1`) is pressed at the pointer.
    fn press(&self, chord: &str) {
        self.settle();
        let claim = self.comp.claim_id(chord);
        self.comp.drag(claim, chord, "press");
        self.settle();
    }

    /// A pointer chord is let go.
    fn release(&self, chord: &str) {
        let claim = self.comp.claim_id(chord);
        self.comp.drag(claim, chord, "release");
        self.settle();
    }

    /// A step of a touchpad gesture.
    fn gesture(&self, kind: &str, fingers: i64, phase: &str, (dx, dy): (f64, f64)) {
        self.settle();
        let claim = self.comp.gesture_claim(kind, fingers);
        self.comp.gesture(claim, kind, fingers, phase, (dx, dy), 1.0, 0.0);
        self.settle();
    }

    /// An object the shell implements for `interface`: `run` is called with the name of the hook and its event.
    fn handler(
        &self,
        interface: &str,
        hooks: &[&'static str],
        mut run: impl FnMut(&'static str, &[Val]) -> Res<Val> + 'static,
    ) -> Val {
        let class = self.class(interface);
        let named: Vec<(u32, &'static str)> = hooks
            .iter()
            .map(|name| (self.api.member(class, abi::MEMBER_METHOD, name).expect("a hook"), *name))
            .collect();
        let ids: Vec<u32> = named.iter().map(|(id, _)| *id).collect();
        let obj = self
            .api
            .implement(class, &ids, move |member, args| {
                let name = named
                    .iter()
                    .find(|(id, _)| *id == member)
                    .map(|(_, name)| *name)
                    .expect("a hook of the interface");
                run(name, args)
            })
            .expect("an implementation");
        Val::Object { class, obj }
    }

    /// A function for a drag: it records each event, and declines the start when `declines`.
    fn recorder(&self, store: &Seens, declines: bool) -> Val {
        let (api, store) = (self.api, Rc::clone(store));
        Val::Closure(closure(move |_, args| {
            let event = seen(api, &args[0]);
            let answer = if declines && event.phase == Some("start") {
                Val::Bool(false)
            } else {
                Val::None
            };
            store.borrow_mut().push(event);
            Ok(answer)
        }))
    }

    /// `binding_options` with a threshold.
    fn threshold(&self, id: &str, threshold: f64) -> Val {
        Val::Record {
            class: self.class("binding_options"),
            fields: vec![
                Val::Text(id.to_owned()),
                Val::None,
                Val::None,
                Val::None,
                Val::Double(threshold),
            ],
        }
    }
}

fn phases(seens: &Seens) -> Vec<&'static str> {
    seens.borrow().iter().filter_map(|seen| seen.phase).collect()
}

const BUTTON1: &str = "<Super>Button1";

/// A shell's own `ClientMoveBinding`: the window under the press follows the pointer by the distance it went from the press.
fn move_binding(api: Api) -> Val {
    let moving: Rc<RefCell<Option<(Obj, f64, f64)>>> = Rc::default();
    Val::Closure(closure(move |_, args| {
        let fields = args[0].record()?;
        let (dx, dy) = (fields[12].double().unwrap_or(0.0), fields[13].double().unwrap_or(0.0));
        match phase_name(api, &fields[10]) {
            Some("start") => {
                let Some(client) = client_of(&args[0]) else {
                    return Ok(Val::Bool(false));
                };
                *moving.borrow_mut() = Some((client, number(api, client, "x"), number(api, client, "y")));
                Ok(Val::Bool(true))
            }
            Some("update") => {
                if let Some((client, x, y)) = *moving.borrow() {
                    put(api, client, "x", x + dx)?;
                    put(api, client, "y", y + dy)?;
                }
                Ok(Val::None)
            }
            _ => {
                *moving.borrow_mut() = None;
                Ok(Val::None)
            }
        }
    }))
}

/// A shell's own `ClientResizeBinding`, of the bottom right corner: the size follows the pointer, and not below 100 by 100.
fn resize_binding(api: Api) -> Val {
    let sizing: Rc<RefCell<Option<(Obj, f64, f64)>>> = Rc::default();
    Val::Closure(closure(move |_, args| {
        let fields = args[0].record()?;
        let (dx, dy) = (fields[12].double().unwrap_or(0.0), fields[13].double().unwrap_or(0.0));
        match phase_name(api, &fields[10]) {
            Some("start") => {
                let Some(client) = client_of(&args[0]) else {
                    return Ok(Val::Bool(false));
                };
                *sizing.borrow_mut() = Some((client, number(api, client, "width"), number(api, client, "height")));
                Ok(Val::Bool(true))
            }
            Some("update") => {
                if let Some((client, width, height)) = *sizing.borrow() {
                    put(api, client, "width", (width + dx).max(100.0))?;
                    put(api, client, "height", (height + dy).max(100.0))?;
                }
                Ok(Val::None)
            }
            _ => {
                *sizing.borrow_mut() = None;
                Ok(Val::None)
            }
        }
    }))
}

#[test]
#[allow(clippy::too_many_lines)]
fn drags_and_gestures_run_as_the_typescript_matcher_did() {
    let mut rig = Rig::new(false);
    let fixture = fixture("input_drags");
    let steps = fixture["steps"].as_array().expect("steps").clone();
    let api = rig.api;
    let input = rig.primary();
    let window = rig.object(input, "focused_client");
    let bounds = |rig: &Rig| {
        let value = json!([
            number(api, window, "x"),
            number(api, window, "y"),
            number(api, window, "width"),
            number(api, window, "height")
        ]);
        rig.log.borrow_mut().push(json!({ "label": "bounds", "value": value }));
    };

    // The bindings of the scenario: two directions of a swipe on one claim, then a layout's move and resize on two more.
    let hooks = ["on_begin", "on_update", "on_end", "on_cancel"];
    for (trigger, label) in [("Swipe3Left", "left"), ("Swipe3Right", "right")] {
        let log = Rc::clone(&rig.log);
        let handler = rig.handler("gesture_binding", &hooks, move |hook, args| {
            let progress = seen(api, &args[0]).progress.expect("a gesture has progress");
            let (name, value) = match hook {
                "on_begin" => ("g.start", json!([label, progress])),
                "on_update" => ("g.update", json!([label, progress])),
                "on_end" => ("g.end", json!([label, progress])),
                _ => ("g.cancel", json!([label])),
            };
            log.borrow_mut().push(json!({ "label": name, "value": value }));
            Ok(if hook == "on_begin" { Val::Bool(true) } else { Val::None })
        });
        rig.ok(
            input,
            "bind_gesture",
            &[
                Val::Text(trigger.to_owned()),
                handler,
                rig.options(Some(trigger), None, None),
            ],
        );
    }
    let layout = rig
        .api
        .create(rig.class("binding_collection"), &[])
        .expect("a collection");
    rig.bind(
        layout,
        "Super+MouseLeft+Drag",
        move_binding(api),
        rig.options(Some("move"), None, None),
    )
    .expect("bound");
    rig.bind(
        layout,
        "Super+MouseRight+Drag",
        resize_binding(api),
        rig.options(Some("resize"), None, None),
    )
    .expect("bound");
    rig.ok(
        input,
        "attach_layout",
        &[Val::Object { class: 0, obj: layout }, Val::None],
    );
    rig.settle();

    for step in &steps {
        match step["step"].as_str().expect("a name") {
            "boot" => {}
            "claimed" => rig.log.borrow_mut().push(json!({ "label": "bounds", "value": 0 })),
            "press_moves_nothing" => {
                rig.pointer_at(50.0, 60.0);
                rig.press(BUTTON1);
                bounds(&rig);
            }
            "within_the_threshold" => {
                rig.pointer_at(52.0, 61.0);
                bounds(&rig);
            }
            "moved_by_the_delta" => {
                rig.pointer_at(80.0, 100.0);
                bounds(&rig);
            }
            "a_released_drag_follows_nothing" => {
                rig.release(BUTTON1);
                rig.pointer_at(400.0, 400.0);
                bounds(&rig);
            }
            "resize_top_left" => {
                rig.pointer_at(20.0, 30.0);
                rig.press("<Super>Button3");
                rig.pointer_at(60.0, 70.0);
                bounds(&rig);
            }
            "resize_held_at_the_minimum" => {
                rig.pointer_at(350.0, 230.0);
                bounds(&rig);
            }
            "resize_released" => rig.release("<Super>Button3"),
            "a_drag_over_no_client_is_declined" => {
                rig.pointer_at(600.0, 600.0);
                rig.press(BUTTON1);
                rig.pointer_at(650.0, 650.0);
                rig.release(BUTTON1);
                bounds(&rig);
            }
            "probe_bound" => {
                let log = Rc::clone(&rig.log);
                let updates = Rc::new(std::cell::Cell::new(0_u32));
                let probe = rig.handler(
                    "pointer_drag_binding",
                    &["on_start", "on_update", "on_end", "on_cancel"],
                    move |hook, args| {
                        let event = seen(api, &args[0]);
                        let entry = match hook {
                            "on_start" => {
                                updates.set(0);
                                json!({ "label": "start", "value": ["p", event.client, event.position.0, event.position.1] })
                            }
                            "on_update" => {
                                updates.set(updates.get() + 1);
                                json!({ "label": "update", "value": ["p", event.position.0, event.position.1] })
                            }
                            "on_end" => json!({ "label": "end", "value": ["p", updates.get()] }),
                            _ => json!({ "label": "cancel", "value": ["p", updates.get()] }),
                        };
                        log.borrow_mut().push(entry);
                        Ok(if hook == "on_start" { Val::Bool(true) } else { Val::None })
                    },
                );
                rig.ok(
                    input,
                    "bind_drag",
                    &[Val::Text("Super+MouseMiddle+Drag".to_owned()), probe, Val::None],
                );
                rig.settle();
            }
            "probe_started" => {
                rig.pointer_at(50.0, 60.0);
                rig.press("<Super>Button2");
                rig.pointer_at(70.0, 80.0);
            }
            "window_dies_mid_drag" => {
                rig.comp.destroy(3);
                rig.settle();
                rig.pointer_at(90.0, 100.0);
                rig.release("<Super>Button2");
            }
            "undecided" => {
                rig.gesture("swipe", 3, "begin", (0.0, 0.0));
                rig.gesture("swipe", 3, "update", (-4.0, 1.0));
            }
            "left_swipe" => {
                rig.gesture("swipe", 3, "update", (-20.0, 2.0));
                rig.gesture("swipe", 3, "update", (-100.0, 5.0));
                rig.gesture("swipe", 3, "end", (-120.0, 5.0));
            }
            "right_swipe" => {
                rig.gesture("swipe", 3, "begin", (0.0, 0.0));
                rig.gesture("swipe", 3, "update", (30.0, 0.0));
                rig.gesture("swipe", 3, "end", (40.0, 0.0));
            }
            "cancelled_swipe" => {
                rig.gesture("swipe", 3, "begin", (0.0, 0.0));
                rig.gesture("swipe", 3, "update", (-50.0, 0.0));
                rig.gesture("swipe", 3, "cancel", (-50.0, 0.0));
            }
            other => panic!("the replay does not know the step `{other}`"),
        }
        expect_step(&mut rig, step);
    }
}

// ---- the three scenarios of the tiling fixtures that drag or gesture ---------------------------------------------------------

/// The world of `tests/tiling.rs`: one window, `40`, and the seat. What the scenario's shell does with a layout is not the core's; the
/// core's part is the bindings of the shell's own, which write numbers into window 40.
fn tiling_world(comp: &FakeCompositor) {
    comp.add_desktop(1000.0, 800.0);
    comp.add_node(
        40,
        ROOT,
        "window",
        vec![
            ("title", text("A")),
            ("app_id", text("app")),
            ("focused", MirrorValue::Bool(true)),
            ("x", MirrorValue::Double(0.0)),
            ("y", MirrorValue::Double(0.0)),
            ("width", MirrorValue::Double(200.0)),
            ("height", MirrorValue::Double(200.0)),
        ],
    );
    comp.add_node(
        5,
        ROOT,
        "seat",
        vec![
            ("name", text("seat0")),
            ("pointer_x", MirrorValue::Double(0.0)),
            ("pointer_y", MirrorValue::Double(0.0)),
            ("focus", MirrorValue::Node(40)),
            ("device", MirrorValue::Texts(Vec::new())),
        ],
    );
}

fn bump(api: Api, window: Obj, name: &str, by: f64) -> Res<()> {
    put(api, window, name, number(api, window, name) + by)
}

/// The bindings of `shell-tiling`'s probes: two drags of `Super+Button2` (`A`, of priority 5, declines a window titled `B`; `B` takes what is
/// left), and gestures that write numbers into window 40. A drag moves the window it pressed: `A` along x, `B` along y.
fn bind_tiling_probes(rig: &Rig, first: Obj) {
    let api = rig.api;
    let input = rig.primary();
    let hooks = ["on_begin", "on_update", "on_end", "on_cancel"];
    // (trigger, the property it counts the starts in, the sign and offset of its x, what its end and its cancel add to y)
    let gestures = [
        ("Swipe3Left", "width", 1.0, 0.0, 10.0, 1000.0),
        ("Swipe3Right", "height", -1.0, 5000.0, 20.0, 2000.0),
        ("Swipe4", "width", 1.0, 10000.0, 30.0, 3000.0),
    ];
    for (trigger, starts, sign, offset, end_by, cancel_by) in gestures {
        let handler = rig.handler("gesture_binding", &hooks, move |hook, args| {
            let progress = seen(api, &args[0]).progress.expect("a gesture has progress");
            let follow = || put(api, first, "x", offset + sign * (progress * 1000.0).round());
            match hook {
                "on_begin" => {
                    bump(api, first, starts, 1.0)?;
                    return Ok(Val::Bool(true));
                }
                "on_update" => follow()?,
                "on_end" => {
                    follow()?;
                    bump(api, first, "y", end_by)?;
                }
                _ => bump(api, first, "y", cancel_by)?,
            }
            Ok(Val::None)
        });
        rig.ok(
            input,
            "bind_gesture",
            &[
                Val::Text(trigger.to_owned()),
                handler,
                rig.options(Some(trigger), None, None),
            ],
        );
    }
    for (label, priority) in [("B", 0.0), ("A", 5.0)] {
        let handler = rig.handler(
            "pointer_drag_binding",
            &["on_start", "on_update", "on_end", "on_cancel"],
            move |hook, args| {
                let event = seen(api, &args[0]);
                let Some(client) = client_of(&args[0]) else {
                    return Ok(if hook == "on_start" {
                        Val::Bool(false)
                    } else {
                        Val::None
                    });
                };
                match hook {
                    "on_start" => {
                        let title = text_of(api, client, "title").unwrap_or_default();
                        return Ok(Val::Bool(label == "B" || title != "B"));
                    }
                    "on_update" if label == "A" => put(api, client, "x", event.position.0)?,
                    "on_update" => put(api, client, "y", event.position.1)?,
                    "on_end" => bump(api, client, "width", 1.0)?,
                    _ => {}
                }
                Ok(Val::None)
            },
        );
        rig.ok(
            input,
            "bind_drag",
            &[
                Val::Text("Super+MouseMiddle+Drag".to_owned()),
                handler,
                rig.options(Some(label), Some(priority), None),
            ],
        );
    }
    rig.settle();
}

fn reference_fixture(name: &str) -> Value {
    Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference").read_json(name)
}

/// `Super+Button2` as the compositor spells a claim.
fn claim_text(chord: &str) -> String {
    let (modifiers, button) = chord.rsplit_once('+').expect("a pointer chord");
    format!("<{}>{button}", modifiers.replace('+', "><"))
}

fn point(text: &str) -> (f64, f64) {
    let (x, y) = text.split_once(',').expect("a point");
    (x.parse().expect("x"), y.parse().expect("y"))
}

/// Plays the script of the tiling fixture `name` to the bindings of the probes, over the fake compositor, and holds what the windows do to
/// the rectangles of the fixture.
#[track_caller]
fn replay_tiling(name: &str) {
    let rig = Rig::over(tiling_world);
    let fixture = reference_fixture(name);
    let first = rig.object(rig.primary(), "focused_client");
    bind_tiling_probes(&rig, first);
    let script = fixture["script"].as_str().expect("a script");
    let steps = fixture["steps"].as_array().expect("steps");
    let mut played = 0;
    let mut windows = vec![40_u64];
    for line in script.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let id = |at: usize| words.get(at).and_then(|word| word.parse::<u64>().ok()).unwrap_or(0);
        let real = |at: usize| words.get(at).and_then(|word| word.parse::<f64>().ok()).unwrap_or(0.0);
        match words.as_slice() {
            [] => {}
            // The layout chord belongs to the scenario's shell, not to the matcher.
            ["chord", _] => {}
            ["focus", ..] => {
                for window in &windows {
                    rig.comp
                        .external_set(*window, "focused", MirrorValue::Bool(*window == id(1)));
                }
                rig.settle();
            }
            ["create", _, title, ..] => {
                rig.comp.create_window(id(1), title, "app");
                windows.push(id(1));
                rig.settle();
            }
            ["destroy", ..] => {
                rig.comp.destroy(id(1));
                windows.retain(|window| *window != id(1));
                rig.settle();
            }
            ["place", ..] => {
                for (key, value) in [("x", real(2)), ("y", real(3)), ("width", real(4)), ("height", real(5))] {
                    rig.comp.external_set(id(1), key, MirrorValue::Double(value));
                }
                rig.settle();
            }
            ["seat", key, value] => {
                rig.comp
                    .external_set(5, key, MirrorValue::Double(value.parse().expect("a number")));
                rig.settle();
            }
            ["drag", chord, from, rest @ ..] if !rest.is_empty() => {
                let (last, path) = rest.split_last().expect("how the drag ends");
                let chord = claim_text(chord);
                let claim = rig.comp.claim_id(&chord);
                let from = point(from);
                rig.comp.pointer_chord(claim, &chord, "start", from, from);
                rig.settle();
                let mut at = from;
                for step in path {
                    at = point(step);
                    rig.comp.pointer_chord(claim, &chord, "update", from, at);
                    rig.settle();
                }
                rig.comp.pointer_chord(claim, &chord, last, from, at);
                rig.settle();
            }
            ["gesture", kind, fingers, phase, ..] => {
                let fingers: i64 = fingers.parse().expect("fingers");
                let claim = rig.comp.gesture_claim(kind, fingers);
                rig.comp
                    .gesture(claim, kind, fingers, phase, (real(4), real(5)), real(6), real(7));
                rig.settle();
            }
            ["step", step, ids @ ..] => {
                let want = &steps[played];
                played += 1;
                assert_eq!(want["step"], *step);
                for (at, id) in ids.iter().enumerate() {
                    let id: u64 = id.parse().expect("an id");
                    let expected = &want["windows"][at];
                    assert_eq!(expected["id"], id);
                    let got = rig.comp.exists(id).then(|| {
                        [
                            rig.comp.number(id, "x"),
                            rig.comp.number(id, "y"),
                            rig.comp.number(id, "width"),
                            rig.comp.number(id, "height"),
                        ]
                    });
                    let wanted = expected["rect"].as_array().map(|rect| {
                        let mut numbers = [0.0; 4];
                        for (slot, value) in numbers.iter_mut().zip(rect) {
                            *slot = value.as_f64().expect("a number");
                        }
                        numbers
                    });
                    assert_eq!(got, wanted, "window {id} at step `{step}` of `{name}`");
                }
            }
            other => panic!("the replay does not know the line {other:?}"),
        }
    }
    assert_eq!(played, steps.len(), "every step of `{name}` was compared");
}

#[test]
fn two_bindings_on_one_drag_trigger_are_asked_by_priority_and_a_decline_leaves_the_drag_to_the_next() {
    replay_tiling("tiling_binding_priority");
}

#[test]
fn a_directed_gesture_starts_when_its_direction_is_known_and_only_the_matching_binding_runs() {
    replay_tiling("tiling_directed_gesture");
}

#[test]
fn a_gesture_is_cancelled_once_when_the_window_under_it_closes() {
    replay_tiling("tiling_gesture_cancel_on_close");
}

// ---- the closure path, from a function ---------------------------------------------------------------------------------------

#[test]
fn a_function_bound_to_a_drag_hears_each_step_and_the_cancel_when_its_window_closes() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let heard: Seens = Rc::default();
    rig.bind(input, "Super+MouseLeft+Drag", rig.recorder(&heard, false), Val::None)
        .expect("a drag is bound with a function");
    rig.settle();

    // Under the threshold nothing starts. Past it the start says where the press was, and the update where the pointer is.
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.pointer_at(52.0, 61.0);
    assert!(heard.borrow().is_empty());
    rig.pointer_at(80.0, 100.0);
    rig.pointer_at(90.0, 110.0);
    rig.release(BUTTON1);
    let steps = heard.borrow().clone();
    assert_eq!(
        steps,
        vec![
            Seen {
                phase: Some("start"),
                position: (50.0, 60.0),
                dx: Some(0.0),
                dy: Some(0.0),
                button: Some(1),
                progress: None,
                client: Some("3".to_owned()),
            },
            Seen {
                phase: Some("update"),
                position: (80.0, 100.0),
                dx: Some(30.0),
                dy: Some(40.0),
                button: Some(1),
                progress: None,
                client: Some("3".to_owned()),
            },
            Seen {
                phase: Some("update"),
                position: (90.0, 110.0),
                dx: Some(40.0),
                dy: Some(50.0),
                button: Some(1),
                progress: None,
                client: Some("3".to_owned()),
            },
            Seen {
                phase: Some("end"),
                position: (90.0, 110.0),
                dx: Some(40.0),
                dy: Some(50.0),
                button: Some(1),
                progress: None,
                client: Some("3".to_owned()),
            },
        ]
    );

    // A window that closes under a drag in flight cancels it once, and nothing after reaches the function.
    heard.borrow_mut().clear();
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.pointer_at(80.0, 100.0);
    assert_eq!(phases(&heard), ["start", "update"]);
    rig.comp.destroy(3);
    rig.settle();
    assert_eq!(phases(&heard), ["start", "update", "cancel"]);
    rig.pointer_at(120.0, 140.0);
    rig.release(BUTTON1);
    assert_eq!(phases(&heard), ["start", "update", "cancel"], "the end never arrives");
}

#[test]
fn a_drag_function_that_declines_leaves_the_drag_to_the_next_binding_and_a_throw_loses_it() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let high: Seens = Rc::default();
    let low: Seens = Rc::default();
    rig.bind(
        input,
        "Super+MouseLeft+Drag",
        rig.recorder(&high, true),
        rig.options(Some("high"), Some(5.0), None),
    )
    .expect("bound");
    let bound = rig
        .bind(
            input,
            "Super+MouseLeft+Drag",
            rig.recorder(&low, false),
            rig.options(Some("low"), None, None),
        )
        .expect("bound");
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.pointer_at(80.0, 100.0);
    rig.release(BUTTON1);
    assert_eq!(phases(&high), ["start"], "the first binding is asked once and declines");
    assert_eq!(
        phases(&low),
        ["start", "update", "end"],
        "the next binding has the drag"
    );

    // A binding that throws in an update has lost the drag and is told the cancel.
    rig.ok(bound, "unbind", &[]);
    let thrown: Seens = Rc::default();
    let (api, store) = (rig.api, Rc::clone(&thrown));
    rig.bind(
        input,
        "Super+MouseLeft+Drag",
        Val::Closure(closure(move |_, args| {
            let event = seen(api, &args[0]);
            let fail = event.phase == Some("update");
            store.borrow_mut().push(event);
            if fail { Err(Fail::err("boom")) } else { Ok(Val::None) }
        })),
        rig.options(Some("thrower"), Some(9.0), None),
    )
    .expect("bound");
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.pointer_at(80.0, 100.0);
    rig.pointer_at(90.0, 110.0);
    rig.release(BUTTON1);
    assert_eq!(phases(&thrown), ["start", "update", "cancel"]);
}

#[test]
fn a_press_that_ends_before_the_threshold_is_a_click_and_one_that_went_too_far_for_every_drag_is_none() {
    let mut rig = Rig::new(false);
    let input = rig.primary();
    let clicks: Seens = Rc::default();
    rig.bind(input, "Super+MouseLeft", rig.recorder(&clicks, false), Val::None)
        .expect("a click is bound");
    let drags: Seens = Rc::default();
    let drag = rig
        .bind(
            input,
            "Super+MouseLeft+Drag",
            rig.recorder(&drags, true),
            rig.threshold("drag", 20.0),
        )
        .expect("bound");
    assert_eq!(rig.prop(drag, "threshold").double().expect("a number"), 20.0);
    rig.settle();
    assert_eq!(
        rig.claim_calls().len(),
        1,
        "a click and a drag of one button stand on one claim"
    );

    // No travel: a click, and a click has no phase.
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.release(BUTTON1);
    assert_eq!(clicks.borrow().len(), 1);
    assert_eq!(clicks.borrow()[0].phase, None);
    assert_eq!(clicks.borrow()[0].button, Some(1));
    assert!(drags.borrow().is_empty());

    // Under the drag's threshold of 20: still a click.
    rig.press(BUTTON1);
    rig.pointer_at(60.0, 60.0);
    rig.release(BUTTON1);
    assert_eq!(clicks.borrow().len(), 2, "10 pixels is not a drag of this binding");

    // Past it the drag is offered, declines (the recorder declines), and what went so far is not a click either.
    rig.press(BUTTON1);
    rig.pointer_at(100.0, 100.0);
    rig.release(BUTTON1);
    assert_eq!(phases(&drags), ["start"]);
    assert_eq!(clicks.borrow().len(), 2);

    // The threshold of a binding is its property, and a distance.
    rig.set(drag, "threshold", &Val::Double(0.0)).expect("a distance");
    let refused = rig
        .set(drag, "threshold", &Val::Double(-1.0))
        .expect_err("not a distance");
    assert!(
        words(&refused).contains("threshold must be a finite number of at least 0"),
        "{}",
        refused.message
    );
}

#[test]
fn removing_the_binding_that_runs_a_drag_cancels_it() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let heard: Seens = Rc::default();
    let bound = rig
        .bind(input, "Super+MouseLeft+Drag", rig.recorder(&heard, false), Val::None)
        .expect("bound");
    rig.pointer_at(50.0, 60.0);
    rig.press(BUTTON1);
    rig.pointer_at(80.0, 100.0);
    assert_eq!(phases(&heard), ["start", "update"]);
    rig.ok(bound, "unbind", &[]);
    rig.settle();
    assert_eq!(phases(&heard), ["start", "update", "cancel"]);
    rig.pointer_at(90.0, 110.0);
    rig.settle();
    assert_eq!(
        phases(&heard),
        ["start", "update", "cancel"],
        "the claim went with the binding"
    );
}

#[test]
fn a_function_bound_to_a_gesture_runs_once_at_the_end_when_the_gesture_went_far_enough() {
    let rig = Rig::new(false);
    let input = rig.primary();
    let heard: Seens = Rc::default();
    rig.bind(input, "Swipe3Left", rig.recorder(&heard, false), Val::None)
        .expect("a gesture is bound with a function");
    // 30 of 400 units is 0.075, under the 0.12 a swipe needs.
    rig.gesture("swipe", 3, "begin", (0.0, 0.0));
    rig.gesture("swipe", 3, "update", (-30.0, 0.0));
    rig.gesture("swipe", 3, "end", (-30.0, 0.0));
    assert!(heard.borrow().is_empty(), "a short swipe runs nothing");
    // 60 units is 0.15. A cancelled gesture runs nothing either.
    rig.gesture("swipe", 3, "begin", (0.0, 0.0));
    rig.gesture("swipe", 3, "update", (-60.0, 0.0));
    rig.gesture("swipe", 3, "cancel", (-60.0, 0.0));
    assert!(heard.borrow().is_empty());
    rig.gesture("swipe", 3, "begin", (0.0, 0.0));
    rig.gesture("swipe", 3, "update", (-60.0, 0.0));
    rig.gesture("swipe", 3, "end", (-60.0, 0.0));
    let steps = heard.borrow().clone();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].phase, Some("end"));
    assert_eq!(steps[0].progress, Some(0.15));
}

#[test]
fn a_drag_and_a_gesture_need_the_object_of_their_kind_and_a_trigger_of_it() {
    let mut rig = Rig::new(false);
    let input = rig.primary();
    let drag = |rig: &Rig| rig.handler("pointer_drag_binding", &["on_start"], |_, _| Ok(Val::Bool(true)));
    let gesture = |rig: &Rig| rig.handler("gesture_binding", &["on_begin"], |_, _| Ok(Val::Bool(true)));
    let press = {
        let class = rig.class("key_binding");
        let on_press = rig.api.member(class, abi::MEMBER_METHOD, "on_press").expect("a method");
        let obj = rig
            .api
            .implement(class, &[on_press], |_, _| Ok(Val::None))
            .expect("an implementation");
        Val::Object { class, obj }
    };
    let refused = |rig: &Rig, call: &str, trigger: &str, handler: Val| {
        words(
            &rig.call(input, call, &[Val::Text(trigger.to_owned()), handler, Val::None])
                .expect_err("refused"),
        )
    };
    assert!(refused(&rig, "bind_drag", "Super+a", drag(&rig)).contains("is not a drag"));
    assert!(refused(&rig, "bind_drag", "Super+MouseLeft", drag(&rig)).contains("is not a drag"));
    assert!(refused(&rig, "bind_gesture", "Super+MouseLeft+Drag", gesture(&rig)).contains("is not a gesture"));
    assert!(
        refused(&rig, "bind_binding", "Super+MouseLeft+Drag", press.clone())
            .contains("bind a pointer_drag_binding with bind_drag")
    );
    assert!(refused(&rig, "bind_binding", "Swipe3Left", press).contains("bind a gesture_binding with bind_gesture"));
    // An object that leaves out the method that tells the binding is there is no drag binding.
    let silent = rig.handler("pointer_drag_binding", &["on_start", "on_update"], |_, _| Ok(Val::None));
    rig.ok(
        input,
        "bind_drag",
        &[Val::Text("Super+MouseLeft+Drag".to_owned()), silent, Val::None],
    );
    // A threshold is for a drag.
    let key = rig
        .call(
            input,
            "bind",
            &[Val::Text("Super+a".to_owned()), Val::None, rig.threshold("key", 3.0)],
        )
        .expect_err("a key has no threshold");
    assert!(words(&key).contains("threshold is for a drag"), "{}", key.message);
    rig.settle();
    assert!(
        rig.claim_calls().len() == 1,
        "the refusals left no claim behind: the one drag binding that was accepted has the only claim"
    );
}

// ---- a drag in flight at a reload -----------------------------------------------------------------------------------------------

#[test]
fn a_drag_and_a_gesture_in_flight_end_with_a_cancel_when_the_generation_that_runs_them_is_replaced() {
    let mut first = Rig::new(false);
    first.comp.refuse_duplicate_claims();
    let input = first.primary();
    let dragged: Seens = Rc::default();
    first
        .bind(
            input,
            "Super+MouseLeft+Drag",
            first.recorder(&dragged, false),
            Val::None,
        )
        .expect("bound");
    let swiped = Rc::clone(&first.log);
    let api = first.api;
    let gesture = first.handler(
        "gesture_binding",
        &["on_begin", "on_update", "on_end", "on_cancel"],
        move |hook, args| {
            swiped
                .borrow_mut()
                .push(json!({ "hook": hook, "phase": seen(api, &args[0]).phase }));
            Ok(if hook == "on_begin" { Val::Bool(true) } else { Val::None })
        },
    );
    first.ok(
        input,
        "bind_gesture",
        &[Val::Text("Swipe3".to_owned()), gesture, Val::None],
    );
    first.settle();

    // A drag and a gesture are under way.
    first.pointer_at(50.0, 60.0);
    first.press(BUTTON1);
    first.pointer_at(80.0, 100.0);
    first.gesture("swipe", 3, "begin", (0.0, 0.0));
    first.gesture("swipe", 3, "update", (-60.0, 0.0));
    assert_eq!(phases(&dragged), ["start", "update"]);
    assert_eq!(first.drain_log().len(), 2, "the gesture began and was updated");

    // The next generation binds the same while it is staged, and takes over. The old one is told while it ends.
    let second = first.staged();
    let again: Seens = Rc::default();
    let next = second.primary();
    second
        .bind(next, "Super+MouseLeft+Drag", second.recorder(&again, false), Val::None)
        .expect("bound");
    second.settle();
    host::take_over(second.ctx).expect("the staged generation takes over");
    first.settle();
    second.settle();
    assert_eq!(
        phases(&dragged),
        ["start", "update"],
        "the takeover alone ends nothing: the old generation still serves"
    );
    first.end();
    second.settle();
    assert_eq!(
        phases(&dragged),
        ["start", "update", "cancel"],
        "the drag ended once, with the cancel of its hooks"
    );
    assert_eq!(
        first.drain_log(),
        vec![json!({ "hook": "on_cancel", "phase": "cancel" })],
        "the gesture ended once"
    );
    assert!(
        again.borrow().is_empty(),
        "the new generation hears nothing of what the old one began"
    );

    // The new generation has the button: its own drag runs from its start.
    second.pointer_at(50.0, 60.0);
    second.press(BUTTON1);
    second.pointer_at(80.0, 100.0);
    assert_eq!(phases(&again), ["start", "update"]);
    assert_eq!(
        phases(&dragged),
        ["start", "update", "cancel"],
        "and the old one hears no more"
    );
}

// ---- the device classes and their settings --------------------------------------------------------------------------------------------

/// The device list of the seat: a keyboard, two mice, a touchpad, a touch screen, a tablet and a lid switch.
const DEVICES: [&str; 7] = [
    "1:USB Keyboard:keyboard",
    "2:Logitech MX Master:pointer",
    "3:Touchpad:pointer,gesture",
    "4:Screen:touch",
    "5:Wacom Pen:tablet-tool",
    "6:Lid Switch:switch",
    "8:Trackball:pointer",
];

fn devices_rig() -> Rig {
    Rig::over(|comp| world_with(comp, false, &DEVICES))
}

fn device(rig: &Rig, id: &str) -> Obj {
    rig.ok(rig.input, "find_device", &[Val::Text(id.to_owned())])
        .object()
        .expect("a device")
}

fn profile(rig: &Rig, name: &str) -> Val {
    let class = rig.class("acceleration_profile");
    Val::Enum {
        class,
        value: rig.api.member(class, abi::MEMBER_VALUE, name).expect("a profile"),
    }
}

#[test]
fn a_seats_devices_are_the_class_their_capabilities_name() {
    let rig = devices_rig();
    let primary = rig.primary();
    let listed: Vec<(String, u32)> = rig
        .devices(primary)
        .into_iter()
        .map(|device| {
            (
                rig.prop(device, "name").text().expect("text").to_owned(),
                rig.api.class_of(device).expect("a class"),
            )
        })
        .collect();
    let class = |name: &str| rig.class(name);
    assert_eq!(
        listed,
        vec![
            ("USB Keyboard".to_owned(), class("keyboard")),
            ("Logitech MX Master".to_owned(), class("mouse")),
            ("Touchpad".to_owned(), class("touchpad")),
            ("Screen".to_owned(), class("touchscreen")),
            ("Wacom Pen".to_owned(), class("tablet")),
            ("Lid Switch".to_owned(), class("device")),
            ("Trackball".to_owned(), class("mouse")),
        ]
    );
    assert_eq!(rig.prop(rig.input, "devices").list().expect("a list").len(), 7);
    assert_eq!(
        rig.prop(device(&rig, "2"), "name").text().expect("text"),
        "Logitech MX Master"
    );
    assert!(
        rig.ok(rig.input, "find_device", &[Val::Text("99".to_owned())])
            .is_none()
    );
    assert!(
        rig.ok(rig.input, "find_context", &[Val::Text("seatA".to_owned())])
            .object()
            .is_ok()
    );
    assert!(
        rig.ok(rig.input, "find_context", &[Val::Text("none".to_owned())])
            .is_none()
    );
}

#[test]
fn device_settings_make_the_input_section_of_the_document_and_belong_to_the_kind() {
    let rig = devices_rig();
    assert_eq!(rig.prop(rig.input, "document").text().expect("text"), "{}");
    let (mouse, other_mouse, keyboard, touchpad) = (
        device(&rig, "2"),
        device(&rig, "8"),
        device(&rig, "1"),
        device(&rig, "3"),
    );
    rig.set(mouse, "sensitivity", &Val::Double(0.5)).expect("written");
    rig.set(mouse, "acceleration", &profile(&rig, "flat")).expect("written");
    rig.set(mouse, "natural_scroll", &Val::Bool(true)).expect("written");
    rig.set(keyboard, "repeat_rate", &Val::Double(40.0)).expect("written");
    rig.set(keyboard, "repeat_delay", &Val::Uint(250)).expect("written");
    rig.set(touchpad, "natural_scroll", &Val::Bool(true)).expect("written");
    rig.set(touchpad, "tap_to_click", &Val::Bool(true)).expect("written");
    rig.set(touchpad, "sensitivity", &Val::Double(-0.25)).expect("written");
    assert_eq!(
        rig.prop(rig.input, "document").text().expect("text"),
        r#"{"input":{"mouse":{"speed":0.5,"accel-profile":"flat","natural-scroll":true},"keyboard":{"repeat-interval":25,"delay":250},"touchpad":{"natural-scroll":true,"tap-to-click":true,"speed":-0.25}}}"#
    );
    assert_eq!(rig.prop(rig.input, "document_revision").uint().expect("a number"), 8);

    // A setting is the kind's, not the device's: another mouse reads it, and hears it change.
    assert_eq!(rig.prop(other_mouse, "sensitivity").double().expect("a number"), 0.5);
    assert_eq!(
        rig.prop(other_mouse, "acceleration").enum_value().expect("a profile"),
        profile(&rig, "flat").enum_value().expect("a profile")
    );
    let heard = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&heard);
    let member = member_of(rig.api, other_mouse, abi::MEMBER_PROPERTY, "sensitivity").expect("a property");
    let _reg = rig
        .api
        .listen(other_mouse, member, move |_, _| {
            *counter.borrow_mut() += 1;
            Ok(Val::None)
        })
        .expect("a listener");
    rig.set(mouse, "sensitivity", &Val::Double(0.25)).expect("written");
    rig.settle();
    assert_eq!(*heard.borrow(), 1);
    assert_eq!(rig.prop(rig.input, "document_revision").uint().expect("a number"), 9);

    // Writing the same value is no change, and none gives a setting back to the system's.
    rig.set(mouse, "sensitivity", &Val::Double(0.25)).expect("written");
    assert_eq!(rig.prop(rig.input, "document_revision").uint().expect("a number"), 9);
    for name in ["sensitivity", "acceleration", "natural_scroll"] {
        rig.ok(mouse, "reset", &[Val::Text(name.to_owned())]);
    }
    assert!(
        rig.call(mouse, "reset", &[Val::Text("repeat_rate".to_owned())])
            .is_err(),
        "a mouse has no repeat rate"
    );
    assert!(rig.prop(mouse, "sensitivity").is_none());
    assert_eq!(
        rig.prop(rig.input, "document").text().expect("text"),
        r#"{"input":{"keyboard":{"repeat-interval":25,"delay":250},"touchpad":{"natural-scroll":true,"tap-to-click":true,"speed":-0.25}}}"#
    );
}

#[test]
fn a_bad_device_setting_is_refused_where_it_is_written_and_leaves_the_setting() {
    let rig = devices_rig();
    let (mouse, keyboard) = (device(&rig, "2"), device(&rig, "1"));
    let refusal = |result: Res<()>| words(&result.expect_err("refused"));
    assert_eq!(
        refusal(rig.set(mouse, "sensitivity", &Val::Double(2.0))),
        "Mouse#2.sensitivity: must be a number from -1 to 1, got 2"
    );
    assert_eq!(
        refusal(rig.set(keyboard, "repeat_rate", &Val::Double(0.0))),
        "Keyboard#1.repeat_rate: must be a number of repeats per second from 0.1 to 1000, got 0"
    );
    assert_eq!(
        refusal(rig.set(keyboard, "repeat_delay", &Val::Uint(0))),
        "Keyboard#1.repeat_delay: must be a whole number of milliseconds from 1 to 10000, got 0"
    );
    let disabled = rig
        .set(mouse, "enabled", &Val::Bool(false))
        .expect_err("the compositor cannot disable one device");
    assert_eq!(disabled.code, abi::E_UNSUPPORTED);
    assert!(
        words(&disabled).starts_with("Mouse#2.enabled: the compositor cannot disable one device"),
        "{}",
        disabled.message
    );
    assert!(rig.prop(mouse, "sensitivity").is_none());
    assert!(rig.prop(keyboard, "repeat_rate").is_none());
    assert_eq!(rig.prop(rig.input, "document").text().expect("text"), "{}");
    assert_eq!(rig.prop(rig.input, "document_revision").uint().expect("a number"), 0);
}

// ---- the document a shell without a runtime of its own commits ------------------------------------------------------------------------

#[derive(Default)]
struct DocumentState {
    /// What `config.current` answers: the document in force.
    current: Option<String>,
    /// Each document the compositor was asked to commit and took.
    commits: Vec<String>,
    /// How many times the shell host opened its connection to the compositor.
    opens: usize,
    /// Whether the compositor refuses the commits it is sent.
    refuse: bool,
}

/// The compositor's configuration document as the shell host's connection sees it: `current` answers the last document it was given, and
/// what it is sent is kept and counted.
#[derive(Clone, Default)]
struct Document(Arc<Mutex<DocumentState>>);

impl Document {
    fn holding(current: &str) -> Document {
        let document = Document::default();
        document.state().current = Some(current.to_owned());
        document
    }

    fn state(&self) -> MutexGuard<'_, DocumentState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn commits(&self) -> usize {
        self.state().commits.len()
    }

    fn opens(&self) -> usize {
        self.state().opens
    }

    /// The document in force says `expected`, whatever the order of its keys.
    #[track_caller]
    fn assert_holds(&self, expected: &str) {
        let current = self.state().current.clone().expect("a document is in force");
        assert_eq!(
            document_of(&current),
            document_of(expected),
            "the document in force: {current}"
        );
    }
}

fn document_of(text: &str) -> gnoblin_config::Value {
    Config::from_json_bytes(text.as_bytes())
        .expect("a valid configuration")
        .document()
        .clone()
}

impl DocumentLinkProvider for Document {
    fn open(&self) -> Result<Box<dyn DocumentLink>, String> {
        self.state().opens += 1;
        Ok(Box::new(self.clone()))
    }
}

impl DocumentLink for Document {
    fn current(&mut self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.state().current.clone().map(String::into_bytes))
    }

    fn commit(&mut self, document: &[u8]) -> Result<Committed, String> {
        let mut state = self.state();
        if state.refuse {
            return Err("config-rejected: the compositor refused it".to_owned());
        }
        let text = String::from_utf8(document.to_vec()).expect("a document is text");
        state.commits.push(text.clone());
        state.current = Some(text);
        Ok(Committed {
            revision: state.commits.len() as u64,
            notices: Vec::new(),
        })
    }
}

/// A shell written in `language` over the devices of the seat, whose host commits to `document`.
fn rig_with_document(language: u32, document: &Document) -> Rig {
    let rig = Rig::in_language(language, |comp| world_with(comp, false, &DEVICES));
    host::set_document_link_provider(Box::new(document.clone()));
    rig
}

/// `gnoblin.compositor`, the settings of the compositor itself, in the rig's generation.
fn compositor_of(rig: &Rig) -> Obj {
    let gnoblin = rig.api.create(rig.class("gnoblin"), &[]).expect("the service");
    rig.object(gnoblin, "compositor")
}

/// A setting of a keyboard, one of a mouse and one of the compositor, which are one of each section the ledger writes.
fn write_settings(rig: &Rig) {
    rig.set(device(rig, "1"), "repeat_delay", &Val::Uint(250))
        .expect("written");
    rig.set(device(rig, "2"), "sensitivity", &Val::Double(0.5))
        .expect("written");
    rig.set(compositor_of(rig), "user_shaders", &Val::Bool(false))
        .expect("written");
}

#[test]
fn the_settings_a_shell_wrote_are_committed_over_the_document_in_force_with_no_call_of_its_own() {
    for language in [abi::LANG_NATIVE, abi::LANG_PYTHON] {
        let document =
            Document::holding(r#"{"appearance":{"reduced-motion":true},"input":{"mouse":{"accel-profile":"flat"}}}"#);
        let rig = rig_with_document(language, &document);
        write_settings(&rig);
        assert_eq!(
            document.commits(),
            0,
            "a write sends nothing: the host commits after the turn that made it"
        );

        host::commit_document_changes(rig.ctx);
        assert_eq!(document.commits(), 1, "language {language}");
        document.assert_holds(
            r#"{"appearance":{"reduced-motion":true},"input":{"mouse":{"accel-profile":"flat","speed":0.5},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#,
        );

        // A turn that wrote nothing commits nothing.
        host::commit_document_changes(rig.ctx);
        assert_eq!(document.commits(), 1);
    }
}

#[test]
fn the_writes_of_one_turn_are_one_commit_and_a_later_write_is_the_next() {
    let document = Document::holding("{}");
    let rig = rig_with_document(abi::LANG_NATIVE, &document);
    write_settings(&rig);
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 1, "three writes, one commit");

    rig.set(device(&rig, "2"), "sensitivity", &Val::Double(0.25))
        .expect("written");
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 2);
    document.assert_holds(
        r#"{"input":{"mouse":{"speed":0.25},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#,
    );

    // The same value is no write: the ledger did not change.
    rig.set(device(&rig, "2"), "sensitivity", &Val::Double(0.25))
        .expect("written");
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 2);

    // A setting the shell gives back leaves the document as the shell found it.
    rig.ok(device(&rig, "2"), "reset", &[Val::Text("sensitivity".to_owned())]);
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 3);
    document.assert_holds(r#"{"input":{"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#);
}

#[test]
fn a_shell_that_wrote_nothing_commits_nothing_and_does_not_reach_the_compositor() {
    let document = Document::holding(r#"{"cursor":{"size":24}}"#);
    let rig = rig_with_document(abi::LANG_NATIVE, &document);
    host::commit_document_changes(rig.ctx);
    // Reading a setting is not writing one.
    assert!(rig.prop(device(&rig, "2"), "sensitivity").is_none());
    assert!(rig.prop(compositor_of(&rig), "user_shaders").bool().expect("a flag"));
    rig.settle();
    host::commit_document_changes(rig.ctx);
    assert_eq!((document.commits(), document.opens()), (0, 0));
    document.assert_holds(r#"{"cursor":{"size":24}}"#);
}

#[test]
fn a_write_of_what_the_document_already_says_is_not_committed() {
    let document = Document::holding(r#"{"compositor":{"user-shaders":false},"cursor":{"size":24}}"#);
    let rig = rig_with_document(abi::LANG_NATIVE, &document);
    rig.set(compositor_of(&rig), "user_shaders", &Val::Bool(false))
        .expect("written");
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 0, "the compositor holds it already");
    assert_eq!(document.opens(), 1, "the shell host had to look");
}

#[test]
fn a_reload_takes_back_what_its_new_generation_does_not_write_and_a_staged_generation_commits_nothing() {
    let document = Document::holding(r#"{"cursor":{"size":24}}"#);
    let mut first = rig_with_document(abi::LANG_NATIVE, &document);
    write_settings(&first);
    host::commit_document_changes(first.ctx);
    assert_eq!(document.commits(), 1);

    // The next generation writes while it is staged: nothing is committed for it until it serves.
    let second = first.staged();
    second
        .set(compositor_of(&second), "user_shaders", &Val::Bool(true))
        .expect("written");
    host::commit_document_changes(second.ctx);
    assert_eq!(
        document.commits(),
        1,
        "a generation that is only staged commits nothing"
    );
    host::take_over(second.ctx).expect("the staged generation takes over");
    first.end();
    host::commit_document_changes(second.ctx);
    assert_eq!(document.commits(), 2);
    document.assert_holds(r#"{"cursor":{"size":24},"compositor":{"user-shaders":true}}"#);

    // A generation that writes nothing takes back everything the one before it set, and nothing that was not the shell's.
    let third = second.staged();
    host::take_over(third.ctx).expect("the staged generation takes over");
    second.settle();
    host::commit_document_changes(third.ctx);
    assert_eq!(document.commits(), 3);
    document.assert_holds(r#"{"cursor":{"size":24}}"#);
}

#[test]
fn the_javascript_runtime_commits_its_own_document_so_the_core_leaves_its_generation_alone() {
    for language in [abi::LANG_JS, abi::LANG_TS] {
        let document = Document::holding(r#"{"cursor":{"size":24}}"#);
        let rig = rig_with_document(language, &document);
        write_settings(&rig);
        host::commit_document_changes(rig.ctx);
        assert_eq!(
            (document.commits(), document.opens()),
            (0, 0),
            "a second commit would be a second revision, and the ledger alone would take the script's sections out"
        );
        // The runtime reads the ledger it commits from the input manager.
        let ledger = rig.prop(rig.input, "document").text().expect("text").to_owned();
        assert!(ledger.contains(r#""user-shaders":false"#), "{ledger}");
    }
}

#[test]
fn a_commit_the_compositor_refuses_leaves_the_shell_running_and_the_next_write_commits_again() {
    let document = Document::holding(r#"{"cursor":{"size":24}}"#);
    let rig = rig_with_document(abi::LANG_NATIVE, &document);
    document.state().refuse = true;
    write_settings(&rig);
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 0);
    document.assert_holds(r#"{"cursor":{"size":24}}"#);
    // The refusal is not retried every turn: the shell host looks again when the shell writes again.
    host::commit_document_changes(rig.ctx);
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.opens(), 1);

    document.state().refuse = false;
    rig.set(device(&rig, "2"), "sensitivity", &Val::Double(0.25))
        .expect("written");
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 1);
    document.assert_holds(
        r#"{"cursor":{"size":24},"input":{"mouse":{"speed":0.25},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#,
    );
}

#[test]
fn a_core_with_no_connection_for_the_document_keeps_what_the_shell_wrote_readable() {
    let rig = devices_rig();
    write_settings(&rig);
    host::commit_document_changes(rig.ctx);
    assert!(!rig.prop(compositor_of(&rig), "user_shaders").bool().expect("a flag"));
    assert_eq!(
        rig.prop(rig.input, "document").text().expect("text"),
        r#"{"input":{"keyboard":{"delay":250},"mouse":{"speed":0.5}},"compositor":{"user-shaders":false}}"#
    );
}

#[test]
fn a_commit_that_fails_is_tried_again_after_a_wait_and_not_on_every_turn() {
    let document = Document::holding(r#"{"cursor":{"size":24}}"#);
    let rig = rig_with_document(abi::LANG_NATIVE, &document);
    let clock = ManualClock::default();
    host::set_clock(Arc::new(clock.clone()));
    document.state().refuse = true;
    write_settings(&rig);
    host::commit_document_changes(rig.ctx);
    assert_eq!((document.commits(), document.opens()), (0, 1));

    // The turns inside the wait ask nothing of the compositor.
    for _ in 0..5 {
        host::commit_document_changes(rig.ctx);
    }
    assert_eq!(document.opens(), 1, "no retry storm");

    // The compositor takes it now, and the wait is over: the same write is committed, without a new write from the shell.
    document.state().refuse = false;
    clock.advance(Duration::from_millis(1_100));
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 1);
    document.assert_holds(
        r#"{"cursor":{"size":24},"input":{"mouse":{"speed":0.5},"keyboard":{"delay":250}},"compositor":{"user-shaders":false}}"#,
    );

    // Confirmed: nothing more is sent, however long the shell runs.
    clock.advance(Duration::from_secs(120));
    host::commit_document_changes(rig.ctx);
    assert_eq!(document.commits(), 1);
}
