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

use gnoblin_core_abi::{self as abi, GnoblinStr};
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
    seat(5, "seatA", &["7:Macro Pad:keyboard"]);
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
        let lock = process_lock();
        host::reset_session();
        let comp = FakeCompositor::new();
        world(&comp, second_seat);
        host::set_tree_link_provider(Box::new(Provider(comp.clone())));
        host::set_clock(Arc::new(ManualClock::default()));
        let ctx = host::open_generation("input", abi::LANG_NATIVE).expect("a generation opens");
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
