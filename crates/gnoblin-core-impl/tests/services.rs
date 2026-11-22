#![allow(unsafe_code)]
//! The services of `gnoblin.model` that talk to the session: `session` and `runtime`, `processes`, `idle`, `launches`, `dialogs` and
//! `authentication`, `session_lock`, `virtual_output` and `insets`, through the object protocol's table and nothing else.
//!
//! The compositor's owner connection and its dialog slot are fakes the test scripts: `FakeOwner` records the calls and requests a shell makes
//! and holds the properties it publishes, and `FakeSlot` offers requests and records the verdicts. They stand for the connections the
//! TypeScript tests (`gnoblin-js/tests/{host_api,host_idle,host_launches,lock,xr,error_codes}.rs`) faked with `JsSessionLink`, `JsIdleLink`
//! and `SlotLink`. Programs are real: a process test starts `/bin/sh`.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_core_abi::{self as abi, GnoblinStr};
use gnoblin_core_impl::host::{self, AttachMode, TreeLinkProvider};
use gnoblin_core_impl::slots::{AfterGrace, SlotLinkProvider, SlotTimings};
use gnoblin_core_impl::verbs::{self, OwnerRequest, VerbCall, VerbLink, VerbLinkProvider};
use gnoblin_model::Value;
use gnoblin_object::client::{Api, closure};
use gnoblin_object::{ClassId, Fail, MemberId, Obj, Res, Val};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, text};
use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
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

struct TreeProvider(FakeCompositor);

impl TreeLinkProvider for TreeProvider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

// ---- the fake owner connection ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct OwnerState {
    calls: Vec<VerbCall>,
    requests: Vec<(u64, OwnerRequest)>,
    props: BTreeMap<(String, String), Value>,
    /// A call that cannot go out: the connection is gone.
    refuse: bool,
}

#[derive(Clone, Default)]
struct Owner(Arc<Mutex<OwnerState>>);

impl Owner {
    fn state(&self) -> MutexGuard<'_, OwnerState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, class: &str, name: &str, value: Value) {
        self.state().props.insert((class.to_owned(), name.to_owned()), value);
        verbs::property_changed(class, name);
    }

    fn idle(&self, idle_ms: u64, inhibitors: u64) {
        self.state()
            .props
            .insert(("idle".to_owned(), "idle_ms".to_owned()), Value::Uint(idle_ms));
        self.state()
            .props
            .insert(("idle".to_owned(), "inhibitors".to_owned()), Value::Uint(inhibitors));
        verbs::property_changed("idle", "idle_ms");
    }

    fn calls(&self, method: &str) -> Vec<VerbCall> {
        self.state()
            .calls
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }
}

impl VerbLink for Owner {
    fn call(&mut self, call: VerbCall) -> Result<(), String> {
        let mut state = self.state();
        if state.refuse {
            return Err("the owner connection is gone".to_owned());
        }
        state.calls.push(call);
        Ok(())
    }

    fn property(&self, class: &str, name: &str) -> Option<Value> {
        self.state().props.get(&(class.to_owned(), name.to_owned())).cloned()
    }

    fn request(&mut self, token: u64, request: OwnerRequest) -> Result<(), String> {
        let mut state = self.state();
        if state.refuse {
            return Err("the owner connection is gone".to_owned());
        }
        state.requests.push((token, request));
        Ok(())
    }
}

impl VerbLinkProvider for Owner {
    fn open(&self) -> Result<Box<dyn VerbLink>, String> {
        Ok(Box::new(self.clone()))
    }
}

// ---- the fake slot connection --------------------------------------------------------------------------------------------

#[derive(Default)]
struct SlotState {
    events: VecDeque<SlotEvent>,
    claimed: Vec<String>,
    released: Vec<String>,
    answers: Vec<(u64, Vec<u8>)>,
    /// Whether the compositor has a shell holding the slot now.
    live: bool,
    /// The requests the compositor's built-in dialog took because no shell held the slot: they wait for the user.
    builtin: Vec<u64>,
}

#[derive(Clone, Default)]
struct Slot(Arc<Mutex<SlotState>>);

impl Slot {
    fn state(&self) -> MutexGuard<'_, SlotState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn offer(&self, request: u64, title: &str, message: &str, subject: &[u8]) {
        let mut state = self.state();
        // The compositor offers a request to the shell that holds the slot, and shows its own dialog when none does.
        if !state.live {
            state.builtin.push(request);
            return;
        }
        state.events.push_back(SlotEvent::Offered {
            request,
            deadline: 250,
            slot: "dialogs.observe".to_owned(),
            title: title.to_owned(),
            message: message.to_owned(),
            subject: subject.to_vec(),
        });
    }

    /// A request that ends in the same poll that offered it: the compositor's window closed before the shell could answer.
    fn offer_and_finish(&self, request: u64, title: &str) {
        let mut state = self.state();
        state.events.push_back(SlotEvent::Offered {
            request,
            deadline: 250,
            slot: "dialogs.observe".to_owned(),
            title: title.to_owned(),
            message: String::new(),
            subject: b"app".to_vec(),
        });
        state.events.push_back(SlotEvent::Finished {
            request,
            completion: "ended",
        });
    }

    fn answered(&self, request: u64) -> Option<String> {
        self.state()
            .answers
            .iter()
            .find(|(held, _)| *held == request)
            .map(|(_, answer)| String::from_utf8_lossy(answer).into_owned())
    }

    fn verdicts(&self, request: u64) -> usize {
        self.state().answers.iter().filter(|(held, _)| *held == request).count()
    }
}

struct SlotLinkOf(Slot);

impl SlotLink for SlotLinkOf {
    fn claim_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.0.state().claimed.push(slot.to_owned());
        self.0.state().live = true;
        Ok(())
    }

    fn release_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.0.state().released.push(slot.to_owned());
        self.0.state().live = false;
        Ok(())
    }

    fn claim_request(&mut self, _request: u64) -> Result<(), LinkError> {
        Ok(())
    }

    fn answer(&mut self, request: u64, answer: &[u8]) -> Result<(), LinkError> {
        self.0.state().answers.push((request, answer.to_vec()));
        Ok(())
    }

    fn decline(&mut self, _request: u64, _reason: Option<gnoblin_model::DeclineReason>) -> Result<(), LinkError> {
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<SlotEvent>, LinkError> {
        Ok(self.0.state().events.drain(..).collect())
    }
}

impl SlotLinkProvider for Slot {
    fn open(&self) -> Result<Box<dyn SlotLink>, String> {
        Ok(Box::new(SlotLinkOf(self.clone())))
    }
}

// ---- the rig ------------------------------------------------------------------------------------------------------------

struct Rig {
    ctx: u64,
    api: Api,
    comp: FakeCompositor,
    clock: ManualClock,
    owner: Owner,
    slot: Slot,
    _lock: MutexGuard<'static, ()>,
}

fn api_of(ctx: u64) -> Api {
    let mut raw: *const abi::GnoblinObjectApi = std::ptr::null();
    // SAFETY: `raw` is a live local the library writes.
    let code = unsafe { (host::api().object_api)(ctx, GnoblinStr::borrowed("gnoblin.model"), &raw mut raw) };
    assert_eq!(code, abi::E_OK, "{}", last_error());
    // SAFETY: the table `object_api` returns is static.
    Api::new(unsafe { &*raw }, ctx, last_error)
}

impl Rig {
    /// A shell over the fake compositor with the fake owner connection and slot, and a generation that attached.
    fn new() -> Rig {
        Rig::with(true, true)
    }

    fn with(owner_link: bool, slot_link: bool) -> Rig {
        let lock = process_lock();
        host::reset_session();
        let comp = FakeCompositor::new();
        comp.add_desktop(1000.0, 600.0);
        host::set_tree_link_provider(Box::new(TreeProvider(comp.clone())));
        let clock = ManualClock::default();
        host::set_clock(Arc::new(clock.clone()));
        let owner = Owner::default();
        if owner_link {
            host::set_verb_link_provider(Box::new(owner.clone()));
        }
        let slot = Slot::default();
        if slot_link {
            host::set_slot_link_provider(Arc::new(slot.clone()));
        }
        host::set_slot_timings(SlotTimings {
            hold: Duration::from_millis(80),
            grace: Duration::from_millis(900),
            after_grace: gnoblin_core_impl::slots::AFTER_GRACE,
        });
        let ctx = host::open_generation("services", abi::LANG_NATIVE).expect("a generation opens");
        // SAFETY: `tree_attach` is a function of the core's table.
        assert_eq!(unsafe { (host::api().tree_attach)(ctx, abi::ATTACH_HOLDER) }, abi::E_OK);
        let api = api_of(ctx);
        Rig {
            ctx,
            api,
            comp,
            clock,
            owner,
            slot,
            _lock: lock,
        }
    }

    fn class(&self, name: &str) -> ClassId {
        self.api
            .class(name)
            .unwrap_or_else(|error| panic!("no class {name}: {error}"))
    }

    /// The service `name`, made on first use.
    fn service(&self, name: &str) -> Obj {
        self.api
            .create(self.class(name), &[])
            .unwrap_or_else(|error| panic!("the service {name}: {error}"))
    }

    fn member(&self, obj: Obj, kind: u32, name: &str) -> MemberId {
        let class = self.api.class_of(obj).expect("the object's class");
        self.api
            .member(class, kind, name)
            .unwrap_or_else(|error| panic!("no member {name}: {error}"))
    }

    fn prop(&self, obj: Obj, name: &str) -> Val {
        let member = self.member(obj, abi::MEMBER_PROPERTY, name);
        self.api
            .get(obj, member)
            .unwrap_or_else(|error| panic!("reading {name}: {error}"))
    }

    fn try_call(&self, obj: Obj, name: &str, args: &[Val]) -> Res<Val> {
        let class = self.api.class_of(obj)?;
        let member = self.api.member(class, abi::MEMBER_METHOD, name)?;
        self.api.call(obj, member, args)
    }

    fn call(&self, obj: Obj, name: &str, args: &[Val]) -> Val {
        self.try_call(obj, name, args)
            .unwrap_or_else(|error| panic!("calling {name}: {error}"))
    }

    fn object(&self, value: Val) -> Obj {
        value.object().expect("an object")
    }

    fn flag(&self, obj: Obj, name: &str) -> bool {
        self.prop(obj, name).bool().expect("a boolean")
    }

    fn text_of(&self, obj: Obj, name: &str) -> String {
        self.prop(obj, name).text().expect("text").to_owned()
    }

    /// Whether the enum value `value` is the member `name`.
    fn is(&self, value: &Val, name: &str) -> bool {
        let Val::Enum { class, value } = value else {
            panic!("an enum value, got {value:?}");
        };
        self.api
            .member(*class, abi::MEMBER_VALUE, name)
            .is_ok_and(|member| member == *value)
    }

    /// The status of an operation: `pending`, `succeeded` or `failed`.
    fn status(&self, op: Obj) -> String {
        let value = self.prop(op, "status");
        ["pending", "succeeded", "failed"]
            .into_iter()
            .find(|name| self.is(&value, name))
            .expect("a status")
            .to_owned()
    }

    /// Listens to `signal` of `obj`, logging `<label>` and the debug form of each argument that is text, a number or a flag.
    fn hear(&self, obj: Obj, signal: &str, log: &Rc<RefCell<Vec<String>>>, label: &'static str) {
        let member = self.member(obj, abi::MEMBER_SIGNAL, signal);
        let log = Rc::clone(log);
        let reg = self
            .api
            .listen(obj, member, move |_, args| {
                let words: Vec<String> = args
                    .iter()
                    .map(|arg| match arg {
                        Val::Text(text) => text.clone(),
                        Val::Uint(value) => value.to_string(),
                        Val::Int(value) => value.to_string(),
                        Val::Bool(flag) => flag.to_string(),
                        Val::Object { .. } => "object".to_owned(),
                        Val::Enum { .. } => "enum".to_owned(),
                        Val::Record { .. } => "record".to_owned(),
                        other => format!("{other:?}"),
                    })
                    .collect();
                log.borrow_mut().push(format!("{label}:{}", words.join(",")));
                Ok(Val::None)
            })
            .expect("a listener");
        let _ = reg;
    }

    /// Runs the turns the shell would.
    fn settle(&self) {
        for _ in 0..6 {
            // SAFETY: `dispatch` is a function of the core's table.
            unsafe { (host::api().dispatch)(self.ctx, 0) };
        }
    }

    /// Turns until `done`, for what comes from another thread.
    #[track_caller]
    fn until(&self, what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            // SAFETY: `dispatch` is a function of the core's table.
            unsafe { (host::api().dispatch)(self.ctx, 5) };
        }
        self.settle();
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
        self.settle();
    }

    fn open(&self, id: u64, title: &str, app_id: &str) {
        self.comp.create_window(id, title, app_id);
        self.settle();
    }

    fn texts(items: &[&str]) -> Val {
        Val::List(items.iter().map(|item| Val::Text((*item).to_owned())).collect())
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        host::reset_session();
    }
}

// ---- session and runtime ---------------------------------------------------------------------------------------------------

#[test]
fn session_status_omits_the_lock_state_until_the_session_publishes_one() {
    let rig = Rig::new();
    let session = rig.service("session");
    let status = rig.prop(session, "status").record().expect("a record").to_vec();
    assert!(rig.is(&status[0], "running"));
    assert_eq!(status[1].bool(), Ok(false));
    assert_eq!(status[2].uint(), Ok(0));
    assert!(status[3].is_none());
}

#[test]
fn session_status_reports_the_published_lock_state_and_revision() {
    let rig = Rig::new();
    let session = rig.service("session");
    rig.owner.publish("session", "lock_available", Value::Bool(true));
    rig.owner.publish("session", "lock_revision", Value::Uint(4));
    rig.owner
        .publish("session", "lock_state", Value::Text("locked".to_owned()));
    let status = rig.prop(session, "status").record().expect("a record").to_vec();
    assert_eq!(status[1].bool(), Ok(true));
    assert_eq!(status[2].uint(), Ok(4));
    assert!(rig.is(&status[3], "locked"));
}

#[test]
fn a_shell_with_no_owner_connection_reports_unavailable_and_refuses_every_verb() {
    let rig = Rig::with(false, false);
    let session = rig.service("session");
    let status = rig.prop(session, "status").record().expect("a record").to_vec();
    assert!(rig.is(&status[0], "unavailable"));
    assert_eq!(status[1].bool(), Ok(false));
    for verb in ["logout", "poweroff", "reboot", "suspend", "hibernate", "lock"] {
        let op = rig.object(rig.call(session, verb, &[]));
        assert_eq!(rig.status(op), "failed", "{verb}");
        assert_eq!(rig.text_of(op, "error_code"), "unavailable", "{verb}");
    }
}

#[test]
fn each_verb_calls_the_session_and_its_operation_settles_with_the_compositors_answer() {
    let rig = Rig::new();
    let session = rig.service("session");
    for verb in ["logout", "poweroff", "reboot", "suspend", "hibernate"] {
        let op = rig.object(rig.call(session, verb, &[]));
        assert_eq!(rig.status(op), "pending", "{verb}");
        let call = rig
            .owner
            .calls(verb)
            .pop()
            .unwrap_or_else(|| panic!("{verb} was not called"));
        assert_eq!(call.class, "session");
        verbs::answered(call.token, Ok(None));
        rig.settle();
        assert_eq!(rig.status(op), "succeeded", "{verb}");
    }
}

#[test]
fn a_refused_verb_fails_the_operation_with_the_typed_code_and_the_message() {
    let rig = Rig::new();
    let session = rig.service("session");
    let op = rig.object(rig.call(session, "suspend", &[]));
    let token = rig.owner.calls("suspend")[0].token;
    verbs::answered(
        token,
        Err(verbs::VerbFailure {
            code: "timed-out".to_owned(),
        }),
    );
    rig.settle();
    assert_eq!(rig.status(op), "failed");
    assert_eq!(rig.text_of(op, "error_code"), "timed_out");
    assert!(rig.text_of(op, "error").contains("timed-out"));
}

#[test]
fn a_link_that_refuses_to_begin_fails_the_operation_unavailable_at_once() {
    let rig = Rig::new();
    let session = rig.service("session");
    rig.call(session, "logout", &[]);
    rig.owner.state().refuse = true;
    let op = rig.object(rig.call(session, "poweroff", &[]));
    assert_eq!(rig.status(op), "failed");
    assert_eq!(rig.text_of(op, "error_code"), "unavailable");
}

#[test]
fn lock_and_reload_config_are_requests_of_their_own_and_a_reload_keeps_its_result() {
    let rig = Rig::new();
    let session = rig.service("session");
    let lock = rig.object(rig.call(session, "lock", &[]));
    let (token, request) = rig.owner.state().requests[0];
    assert_eq!(request, OwnerRequest::Lock);
    verbs::answered_text(token, Ok(String::new()));
    rig.settle();
    assert_eq!(rig.status(lock), "succeeded");

    let runtime = rig.service("runtime");
    assert!(rig.prop(runtime, "last_reload").is_none());
    let reload = rig.object(rig.call(runtime, "reload_config", &[]));
    let (token, request) = rig.owner.state().requests[1];
    assert_eq!(request, OwnerRequest::ReloadConfig);
    verbs::answered_text(token, Ok("7\nrestart required".to_owned()));
    rig.settle();
    assert_eq!(rig.status(reload), "succeeded");
    let result = rig.prop(runtime, "last_reload").record().expect("a record").to_vec();
    assert_eq!(result[0].uint(), Ok(7));
    assert_eq!(result[2].list().expect("a list").len(), 1);

    // A refusal carries the compositor's code and words.
    let refused = rig.object(rig.call(runtime, "reload_config", &[]));
    let (token, _) = rig.owner.state().requests[2];
    verbs::answered_text(
        token,
        Err((
            "config-rejected".to_owned(),
            "the document does not validate".to_owned(),
        )),
    );
    rig.settle();
    assert_eq!(rig.text_of(refused, "error_code"), "config_rejected");
    assert_eq!(rig.text_of(refused, "error"), "the document does not validate");
}

#[test]
fn the_sessions_and_the_configurations_signals_reach_their_listeners_in_order() {
    let rig = Rig::new();
    let session = rig.service("session");
    let runtime = rig.service("runtime");
    let log = Rc::new(RefCell::new(Vec::new()));
    for signal in ["ready", "bell", "reload_requested", "lock_state_changed"] {
        rig.hear(session, signal, &log, signal);
    }
    for signal in ["config_reloaded", "config_reload_failed"] {
        rig.hear(runtime, signal, &log, signal);
    }
    host::publish_named("gnoblin.session", "ready", 0, vec![]).expect("ready");
    host::publish_named("gnoblin.session", "bell", 0, vec![host::OwnedValue::Uint(3)]).expect("bell");
    host::publish_named(
        "gnoblin.session",
        "lock_state_changed",
        0,
        vec![host::OwnedValue::Text("covering".to_owned()), host::OwnedValue::Uint(9)],
    )
    .expect("lock");
    host::publish_named(
        "gnoblin.config",
        "reload_failed",
        0,
        vec![
            host::OwnedValue::Text("/c.ts".to_owned()),
            host::OwnedValue::Text("bad".to_owned()),
            host::OwnedValue::Uint(1),
            host::OwnedValue::Uint(5),
        ],
    )
    .expect("failed");
    rig.settle();
    assert_eq!(
        *log.borrow(),
        vec![
            "ready:object".to_owned(),
            "bell:object,3".to_owned(),
            "lock_state_changed:object,enum,9".to_owned(),
            "config_reload_failed:object,record".to_owned(),
        ]
    );
}

#[test]
fn version_is_the_build_identity_with_unknown_for_what_the_build_lacks() {
    let rig = Rig::new();
    let gnoblin = rig.service("gnoblin");
    let version = rig.prop(gnoblin, "version").record().expect("a record").to_vec();
    assert_eq!(version.len(), 8);
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    assert_eq!(version[4].text(), Ok(format!("{major}.{minor}.{patch}").as_str()));
    assert_eq!(version[3].text(), Ok("unknown"), "a native shell has no script engine");
}

// ---- processes -----------------------------------------------------------------------------------------------------------

fn capture() -> Val {
    // The record `spawn_options`: cwd, env, stdin, stdout, stderr, output_limit, timeout.
    Val::None
}

fn options(rig: &Rig, fields: &[(&str, Val)]) -> Val {
    let class = rig.class("spawn_options");
    let names = ["cwd", "env", "stdin", "stdout", "stderr", "output_limit", "timeout"];
    let mut values = vec![Val::None; names.len()];
    for (name, value) in fields {
        let at = names.iter().position(|held| held == name).expect("a field");
        values[at] = value.clone();
    }
    let _ = capture();
    Val::Record { class, fields: values }
}

fn stream(rig: &Rig, name: &str) -> Val {
    let class = rig.class("stream_mode");
    Val::Enum {
        class,
        value: rig.api.member(class, abi::MEMBER_VALUE, name).expect("a stream mode"),
    }
}

#[test]
fn a_spawned_process_has_a_handle_and_its_end_reaches_wait_and_the_listeners() {
    let rig = Rig::new();
    let processes = rig.service("processes");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.hear(processes, "exited", &log, "table");
    let opts = options(&rig, &[("stdout", stream(&rig, "capture"))]);
    let process = rig.object(rig.call(processes, "shell", &[Val::Text("printf hi; exit 3".to_owned()), opts]));
    assert!(rig.flag(process, "alive"));
    assert!(rig.prop(process, "id").uint().expect("an id") > 0);
    let wait = rig.object(rig.call(process, "wait", &[]));
    assert_eq!(
        rig.object(rig.call(process, "wait", &[])),
        wait,
        "every call returns the same operation"
    );
    assert_eq!(rig.status(wait), "pending");
    rig.hear(process, "exited", &log, "own");
    rig.until("the process to end", || !rig.flag(process, "alive"));
    assert_eq!(rig.status(wait), "succeeded");
    assert!(rig.is(&rig.prop(process, "status"), "exited"));
    assert_eq!(rig.prop(process, "exit_code").int(), Ok(3));
    assert_eq!(rig.prop(process, "pid").uint(), Ok(0));
    let exit = rig.prop(process, "exit").record().expect("a record").to_vec();
    assert_eq!(exit[3].text(), Ok("hi"), "captured standard output");
    assert!(exit[4].is_none(), "standard error was not captured");
    assert_eq!(*log.borrow(), vec!["own:object".to_owned(), "table:object".to_owned()]);
    // A process that has ended is not listed and the signal is refused `not_found`.
    assert!(rig.call(processes, "list", &[]).list().expect("a list").is_empty());
    assert!(rig.call(processes, "by_id", &[Val::Uint(1)]).is_none());
    let kill = rig.object(rig.call(process, "kill", &[]));
    assert_eq!(rig.status(kill), "failed");
    assert_eq!(rig.text_of(kill, "error_code"), "not_found");
}

#[test]
fn kill_ends_the_group_and_the_signal_is_named() {
    let rig = Rig::new();
    let processes = rig.service("processes");
    let process = rig.object(rig.call(processes, "spawn", &[Rig::texts(&["/bin/sleep", "30"])]));
    let running = rig.call(processes, "list", &[]);
    assert_eq!(running.list().expect("a list").len(), 1, "a running process is listed");
    let id = rig.prop(process, "id");
    assert!(rig.call(processes, "by_id", &[id]).object().is_ok());
    assert!(rig.prop(process, "pid").uint().expect("a pid") > 0);
    let kill = rig.object(rig.call(process, "kill", &[]));
    assert_eq!(rig.status(kill), "succeeded");
    rig.until("the process to end", || !rig.flag(process, "alive"));
    assert_eq!(rig.text_of(process, "signal"), "term");
    assert!(rig.prop(process, "exit_code").is_none());
}

#[test]
fn a_timeout_ends_the_process_and_reports_it() {
    let rig = Rig::new();
    let processes = rig.service("processes");
    let opts = options(&rig, &[("timeout", Val::Double(100.0))]);
    let process = rig.object(rig.call(processes, "spawn", &[Rig::texts(&["/bin/sleep", "30"]), opts]));
    rig.until("the timeout", || !rig.flag(process, "alive"));
    assert!(rig.flag(process, "timed_out"));
}

#[test]
fn spawn_refuses_a_program_that_cannot_start_and_a_bad_command_or_option() {
    let rig = Rig::new();
    let processes = rig.service("processes");
    let missing = rig
        .try_call(processes, "spawn", &[Rig::texts(&["/definitely/not/a/program"])])
        .expect_err("a program that is not there");
    assert!(missing.message.contains("gnoblin.spawn"), "{}", missing.message);
    let empty = rig
        .try_call(processes, "spawn", &[Val::List(vec![])])
        .expect_err("an empty command");
    assert_eq!(empty.code, abi::E_TYPE);
    let blank = rig
        .try_call(processes, "shell", &[Val::Text(String::new())])
        .expect_err("an empty command line");
    assert_eq!(blank.code, abi::E_TYPE);
    let bad_env = options(&rig, &[("env", Rig::texts(&["NOEQUALS"]))]);
    let refused = rig
        .try_call(processes, "spawn", &[Rig::texts(&["/bin/true"]), bad_env])
        .expect_err("a bad environment entry");
    assert_eq!(refused.code, abi::E_TYPE);
    assert!(refused.message.contains("NAME=value"));
}

#[test]
fn spawn_options_reach_the_program() {
    let rig = Rig::new();
    let gnoblin = rig.service("gnoblin");
    let opts = options(
        &rig,
        &[
            ("env", Rig::texts(&["GREETING=hello"])),
            ("stdin", Val::Text("from stdin".to_owned())),
            ("stdout", stream(&rig, "capture")),
        ],
    );
    let process = rig.object(rig.call(
        gnoblin,
        "shell",
        &[Val::Text("printf '%s %s' \"$GREETING\" \"$(cat)\"".to_owned()), opts],
    ));
    rig.until("the process to end", || !rig.flag(process, "alive"));
    let exit = rig.prop(process, "exit").record().expect("a record").to_vec();
    assert_eq!(exit[3].text(), Ok("hello from stdin"));
}

// ---- idle ----------------------------------------------------------------------------------------------------------------

/// Makes a listener of the interface `idle_listener` that logs `<label>:<method>`.
fn idle_listener(rig: &Rig, log: &Rc<RefCell<Vec<String>>>, label: &'static str) -> Val {
    let class = rig.class("idle_listener");
    let methods: Vec<(MemberId, &'static str)> = ["on_idle", "on_resume", "on_closed"]
        .iter()
        .map(|name| {
            (
                rig.api.member(class, abi::MEMBER_METHOD, name).expect("a method"),
                *name,
            )
        })
        .collect();
    let ids: Vec<MemberId> = methods.iter().map(|(id, _)| *id).collect();
    let log = Rc::clone(log);
    let obj = rig
        .api
        .implement(class, &ids, move |member, args| {
            let name = methods
                .iter()
                .find(|(id, _)| *id == member)
                .map_or("?", |(_, name)| *name);
            let extra = args
                .get(1)
                .and_then(|arg| arg.text().ok())
                .map_or(String::new(), |reason| format!(":{reason}"));
            log.borrow_mut().push(format!("{label}:{name}{extra}"));
            Ok(Val::None)
        })
        .expect("a listener");
    Val::Object { class, obj }
}

#[test]
fn a_watch_is_not_idle_when_it_starts_and_hears_idle_once_then_resume_and_idle_again() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.owner.idle(0, 0);
    let watch = rig.object(rig.call(idle, "watch", &[Val::Double(1000.0), idle_listener(&rig, &log, "w")]));
    assert!(!rig.flag(watch, "is_idle"));
    assert!(rig.flag(watch, "alive"));
    assert_eq!(rig.text_of(watch, "id"), "idle-1");
    rig.settle();
    assert!(log.borrow().is_empty());
    rig.owner.idle(1500, 0);
    rig.settle();
    rig.owner.idle(2500, 0);
    rig.settle();
    assert_eq!(*log.borrow(), vec!["w:on_idle".to_owned()]);
    assert!(rig.flag(watch, "is_idle"));
    rig.owner.idle(10, 0);
    rig.settle();
    rig.owner.idle(1200, 0);
    rig.settle();
    assert_eq!(
        *log.borrow(),
        vec!["w:on_idle".to_owned(), "w:on_resume".to_owned(), "w:on_idle".to_owned()]
    );
}

#[test]
fn each_watch_has_its_own_time_and_one_made_while_idle_hears_it_at_once() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.owner.idle(0, 0);
    rig.call(
        idle,
        "watch",
        &[Val::Double(1000.0), idle_listener(&rig, &log, "short")],
    );
    rig.call(idle, "watch", &[Val::Double(5000.0), idle_listener(&rig, &log, "long")]);
    rig.owner.idle(2000, 0);
    rig.settle();
    assert_eq!(*log.borrow(), vec!["short:on_idle".to_owned()]);
    rig.call(idle, "watch", &[Val::Double(1500.0), idle_listener(&rig, &log, "late")]);
    rig.settle();
    assert_eq!(log.borrow().last().map(String::as_str), Some("late:on_idle"));
    let reading = rig.prop(idle, "reading").record().expect("a reading").to_vec();
    assert_eq!(reading[0].uint(), Ok(2000));
    assert!(rig.call(idle, "watch", &[Val::Double(10.0)]).object().is_ok());
    assert_eq!(rig.prop(idle, "watches").list().expect("a list").len(), 4);
}

#[test]
fn an_inhibitor_delays_idle_and_one_that_appears_while_idle_resumes_the_watch() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.owner.idle(0, 0);
    rig.call(idle, "watch", &[Val::Double(1000.0), idle_listener(&rig, &log, "w")]);
    rig.owner.idle(5000, 1);
    rig.settle();
    assert!(log.borrow().is_empty(), "a hold delays idle");
    rig.owner.idle(5000, 0);
    rig.settle();
    rig.owner.idle(6000, 1);
    rig.settle();
    assert_eq!(*log.borrow(), vec!["w:on_idle".to_owned(), "w:on_resume".to_owned()]);
}

#[test]
fn a_hold_is_sent_once_per_holder_given_back_with_release_and_a_refused_one_is_not_a_hold() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    rig.owner.idle(0, 0);
    let hold = rig.object(rig.call(idle, "inhibit", &[Val::Text("movie".to_owned())]));
    assert_eq!(
        rig.object(rig.call(idle, "inhibit", &[Val::Text("movie".to_owned())])),
        hold
    );
    assert_eq!(rig.owner.calls("inhibit").len(), 1);
    assert!(rig.flag(hold, "alive"));
    assert_eq!(rig.text_of(hold, "holder"), "movie");
    rig.call(hold, "release", &[]);
    rig.call(hold, "release", &[]);
    assert_eq!(rig.owner.calls("uninhibit").len(), 1);
    assert!(!rig.flag(hold, "alive"));

    let refused = rig.object(rig.call(idle, "inhibit", &[Val::Text("too-many".to_owned())]));
    let token = rig.owner.calls("inhibit").last().expect("a call").token;
    verbs::answered(
        token,
        Err(verbs::VerbFailure {
            code: "too-many-inhibitors".to_owned(),
        }),
    );
    rig.settle();
    assert!(!rig.flag(refused, "alive"));
    assert!(rig.try_call(idle, "inhibit", &[Val::Text(String::new())]).is_err());
}

#[test]
fn cancel_ends_the_watch_and_tells_closed_once() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.owner.idle(0, 0);
    let watch = rig.object(rig.call(idle, "watch", &[Val::Double(1000.0), idle_listener(&rig, &log, "w")]));
    rig.call(watch, "cancel", &[]);
    rig.call(watch, "cancel", &[]);
    rig.owner.idle(9000, 0);
    rig.settle();
    assert_eq!(*log.borrow(), vec!["w:on_closed:cancelled".to_owned()]);
    assert!(!rig.flag(watch, "alive"));
    assert!(rig.prop(idle, "watches").list().expect("a list").is_empty());
}

#[test]
fn the_time_is_above_zero_and_at_most_a_day_and_a_generation_has_256_watches() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    for bad in [0.0, -1.0, 86_400_001.0, f64::NAN] {
        assert!(rig.try_call(idle, "watch", &[Val::Double(bad)]).is_err(), "{bad}");
    }
    for _ in 0..256 {
        rig.call(idle, "watch", &[Val::Double(1000.0)]);
    }
    let refused = rig
        .try_call(idle, "watch", &[Val::Double(1000.0)])
        .expect_err("the 257th");
    assert!(refused.message.contains("256"));
}

#[test]
fn a_lost_connection_ends_the_holds_and_resumes_the_watches() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.owner.idle(0, 0);
    rig.call(idle, "watch", &[Val::Double(1000.0), idle_listener(&rig, &log, "w")]);
    let hold = rig.object(rig.call(idle, "inhibit", &[Val::Text("h".to_owned())]));
    rig.owner.idle(2000, 0);
    rig.settle();
    verbs::owner_lost("the compositor went away");
    rig.settle();
    assert_eq!(*log.borrow(), vec!["w:on_idle".to_owned(), "w:on_resume".to_owned()]);
    assert!(!rig.flag(hold, "alive"));
    assert!(rig.prop(idle, "reading").is_none());
}

#[test]
fn a_shell_with_no_idle_connection_has_watches_that_never_fire_and_holds_that_are_not_alive() {
    let rig = Rig::with(false, false);
    let idle = rig.service("idle");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.call(idle, "watch", &[Val::Double(10.0), idle_listener(&rig, &log, "w")]);
    rig.settle();
    assert!(log.borrow().is_empty());
    let hold = rig.object(rig.call(idle, "inhibit", &[Val::Text("h".to_owned())]));
    assert!(!rig.flag(hold, "alive"));
}

#[test]
fn ending_the_generation_gives_every_hold_back() {
    let rig = Rig::new();
    let idle = rig.service("idle");
    rig.owner.idle(0, 0);
    rig.call(idle, "inhibit", &[Val::Text("a".to_owned())]);
    rig.call(idle, "inhibit", &[Val::Text("b".to_owned())]);
    host::close_generation(rig.ctx);
    let given: Vec<String> = rig
        .owner
        .calls("uninhibit")
        .iter()
        .filter_map(|call| match call.args.get("holder") {
            Some(Value::Text(holder)) => Some(holder.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(given, vec!["a".to_owned(), "b".to_owned()]);
}

// ---- launches -------------------------------------------------------------------------------------------------------------

/// A listener of `launch_listener` that logs `<method>[:<reason>]`.
fn launch_listener(rig: &Rig, log: &Rc<RefCell<Vec<String>>>) -> Val {
    let class = rig.class("launch_listener");
    let methods: Vec<(MemberId, &'static str)> = ["on_changed", "on_started", "on_timed_out", "on_closed"]
        .iter()
        .map(|name| {
            (
                rig.api.member(class, abi::MEMBER_METHOD, name).expect("a method"),
                *name,
            )
        })
        .collect();
    let ids: Vec<MemberId> = methods.iter().map(|(id, _)| *id).collect();
    let log = Rc::clone(log);
    let obj = rig
        .api
        .implement(class, &ids, move |member, args| {
            let name = methods
                .iter()
                .find(|(id, _)| *id == member)
                .map_or("?", |(_, name)| *name);
            let extra = match args.get(1) {
                Some(Val::Text(reason)) => format!(":{reason}"),
                Some(Val::List(fields)) => format!(":{}", fields.len()),
                _ => String::new(),
            };
            log.borrow_mut().push(format!("{name}{extra}"));
            Ok(Val::None)
        })
        .expect("a listener");
    Val::Object { class, obj }
}

fn launch_options(rig: &Rig, timeout: Option<i64>, listener: Option<Val>) -> Val {
    Val::Record {
        class: rig.class("launch_options"),
        fields: vec![timeout.map_or(Val::None, Val::Int), listener.unwrap_or(Val::None)],
    }
}

#[test]
fn a_launch_is_pending_at_once_and_a_window_of_the_application_starts_it_with_closed_last() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let log = Rc::new(RefCell::new(Vec::new()));
    let launch = rig.object(rig.call(
        launches,
        "begin",
        &[
            Val::Text("t1".to_owned()),
            Val::Text("Org.Example.App.desktop".to_owned()),
            launch_options(&rig, None, Some(launch_listener(&rig, &log))),
        ],
    ));
    assert!(rig.is(&rig.prop(launch, "state"), "pending"));
    assert!(rig.flag(launch, "alive"));
    assert_eq!(rig.prop(launch, "revision").uint(), Ok(0));
    assert_eq!(rig.prop(launch, "timeout_ms").uint(), Ok(3000));
    assert!(rig.flag(launches, "pending"));
    rig.open(5, "App", "org.example.app");
    assert!(rig.is(&rig.prop(launch, "state"), "started"));
    assert_eq!(rig.prop(launch, "revision").uint(), Ok(1));
    assert_eq!(
        *log.borrow(),
        vec![
            "on_changed:3".to_owned(),
            "on_started".to_owned(),
            "on_closed:started".to_owned()
        ]
    );
    assert!(!rig.flag(launches, "pending"));
}

#[test]
fn a_window_that_existed_before_the_launch_does_not_start_it_and_a_later_focus_or_app_id_does() {
    let rig = Rig::new();
    rig.open(3, "old", "app.old");
    let launches = rig.service("launches");
    let early = rig.object(rig.call(
        launches,
        "begin",
        &[Val::Text("a".to_owned()), Val::Text("app.old".to_owned())],
    ));
    rig.settle();
    assert!(rig.flag(early, "alive"), "a window that was there already answers none");
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert!(!rig.flag(early, "alive"), "a window that takes focus answers");
    let named = rig.object(rig.call(
        launches,
        "begin",
        &[Val::Text("b".to_owned()), Val::Text("app.new".to_owned())],
    ));
    rig.comp.external_set(3, "app_id", text("APP.NEW"));
    rig.settle();
    assert!(
        !rig.flag(named, "alive"),
        "a window that names its application later answers"
    );
}

#[test]
fn a_window_starts_only_the_oldest_pending_launch_it_matches() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let first = rig.object(rig.call(
        launches,
        "begin",
        &[Val::Text("a".to_owned()), Val::Text("app".to_owned())],
    ));
    let second = rig.object(rig.call(
        launches,
        "begin",
        &[Val::Text("b".to_owned()), Val::Text("app".to_owned())],
    ));
    rig.open(5, "w", "app");
    assert!(!rig.flag(first, "alive"));
    assert!(rig.flag(second, "alive"));
}

#[test]
fn the_deadline_times_the_launch_out_and_a_later_window_changes_nothing() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let log = Rc::new(RefCell::new(Vec::new()));
    let launch = rig.object(rig.call(
        launches,
        "begin",
        &[
            Val::Text("a".to_owned()),
            Val::Text("app".to_owned()),
            launch_options(&rig, Some(500), Some(launch_listener(&rig, &log))),
        ],
    ));
    rig.advance(499);
    assert!(rig.flag(launch, "alive"));
    rig.advance(2);
    assert!(rig.is(&rig.prop(launch, "state"), "timed_out"));
    assert_eq!(
        *log.borrow(),
        vec![
            "on_changed:3".to_owned(),
            "on_timed_out".to_owned(),
            "on_closed:timed_out".to_owned()
        ]
    );
    rig.open(5, "w", "app");
    assert!(rig.is(&rig.prop(launch, "state"), "timed_out"));
}

#[test]
fn finish_ends_a_pending_launch_once_and_a_token_in_use_replaces_the_earlier_launch() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let log = Rc::new(RefCell::new(Vec::new()));
    let first = rig.object(rig.call(
        launches,
        "begin",
        &[
            Val::Text("t".to_owned()),
            Val::Text("app".to_owned()),
            launch_options(&rig, None, Some(launch_listener(&rig, &log))),
        ],
    ));
    let second = rig.object(rig.call(
        launches,
        "begin",
        &[Val::Text("t".to_owned()), Val::Text("app".to_owned())],
    ));
    rig.settle();
    assert!(!rig.flag(first, "alive"));
    assert_eq!(log.borrow().last().map(String::as_str), Some("on_closed:replaced"));
    rig.call(second, "finish", &[]);
    rig.call(second, "finish", &[]);
    assert!(rig.is(&rig.prop(second, "state"), "ended"));
    assert_eq!(rig.prop(second, "revision").uint(), Ok(1));
    assert_eq!(rig.call(launches, "list", &[]).list().expect("a list").len(), 1);
}

#[test]
fn the_registry_holds_64_launches_drops_the_oldest_finished_and_ends_the_new_one_when_all_are_pending() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let begin =
        |token: String| rig.object(rig.call(launches, "begin", &[Val::Text(token), Val::Text("app".to_owned())]));
    let objects: Vec<Obj> = (0..64).map(|index| begin(format!("t{index}"))).collect();
    let overflow = begin("extra".to_owned());
    assert!(!rig.flag(overflow, "alive"), "all pending: the new launch ends at once");
    assert_eq!(rig.call(launches, "list", &[]).list().expect("a list").len(), 64);
    rig.call(objects[3], "finish", &[]);
    let kept = begin("again".to_owned());
    assert!(rig.flag(kept, "alive"));
    let held: Vec<String> = rig
        .call(launches, "list", &[])
        .list()
        .expect("a list")
        .iter()
        .map(|launch| rig.text_of(launch.object().expect("an object"), "token"))
        .collect();
    assert_eq!(held.len(), 64);
    assert!(
        !held.contains(&"t3".to_owned()),
        "the oldest finished launch was dropped"
    );
}

#[test]
fn begin_takes_a_token_up_to_128_characters_an_application_up_to_512_and_a_timeout_of_100_to_10000() {
    let rig = Rig::new();
    let launches = rig.service("launches");
    let begin = |token: &str, app: &str, timeout: Option<i64>| {
        rig.try_call(
            launches,
            "begin",
            &[
                Val::Text(token.to_owned()),
                Val::Text(app.to_owned()),
                launch_options(&rig, timeout, None),
            ],
        )
    };
    assert!(begin("", "a", None).is_err());
    assert!(begin(&"t".repeat(129), "a", None).is_err());
    assert!(begin(&"t".repeat(128), "a", None).is_ok());
    assert!(begin("x", "", None).is_err());
    assert!(begin("y", &"a".repeat(513), None).is_err());
    assert!(begin("z", &"a".repeat(512), None).is_ok());
    assert!(begin("w", "a", Some(99)).is_err());
    assert!(begin("w", "a", Some(10_001)).is_err());
    assert!(begin("w", "a", Some(100)).is_ok());
}

// ---- dialogs and authentication, and the reload gap ---------------------------------------------------------------------------

/// A listener of `dialog_listener` that declines the requests whose subject is in `decline`, and logs what it was shown.
fn dialog_listener(rig: &Rig, log: &Rc<RefCell<Vec<String>>>, decline: &'static [&'static str]) -> Val {
    let class = rig.class("dialog_listener");
    let consent = rig
        .api
        .member(class, abi::MEMBER_METHOD, "on_consent_request")
        .expect("a method");
    let secret = rig
        .api
        .member(class, abi::MEMBER_METHOD, "on_secret_request")
        .expect("a method");
    let api = rig.api;
    let log = Rc::clone(log);
    let obj = rig
        .api
        .implement(class, &[consent, secret], move |member, args| {
            let request = args[0].object()?;
            let class = api.class_of(request)?;
            let subject = api.get(request, api.member(class, abi::MEMBER_PROPERTY, "subject")?)?;
            let subject = subject.text()?.to_owned();
            log.borrow_mut().push(format!(
                "{}:{subject}",
                if member == consent { "consent" } else { "secret" }
            ));
            if decline.contains(&subject.as_str()) {
                api.call(
                    request,
                    api.member(class, abi::MEMBER_METHOD, "decline")?,
                    &[Val::Text("policy".to_owned())],
                )?;
            }
            Ok(Val::None)
        })
        .expect("a listener");
    Val::Object { class, obj }
}

fn authentication_listener(rig: &Rig, log: &Rc<RefCell<Vec<String>>>) -> Val {
    let class = rig.class("authentication_listener");
    let method = rig
        .api
        .member(class, abi::MEMBER_METHOD, "on_authentication_request")
        .expect("a method");
    let api = rig.api;
    let log = Rc::clone(log);
    let obj = rig
        .api
        .implement(class, &[method], move |_, args| {
            let request = args[0].object()?;
            let class = api.class_of(request)?;
            let get = |name: &str| -> Res<Val> { api.get(request, api.member(class, abi::MEMBER_PROPERTY, name)?) };
            let identities: Vec<String> = get("identities")?
                .list()?
                .iter()
                .map(|item| item.text().map(str::to_owned))
                .collect::<Res<_>>()?;
            log.borrow_mut().push(format!(
                "auth:{}:{}:{}:{}",
                get("action_id")?.text()?,
                get("subject")?.text()?,
                get("app_id")?.text()?,
                identities.join("+")
            ));
            api.call(request, api.member(class, abi::MEMBER_METHOD, "decline")?, &[])?;
            Ok(Val::None)
        })
        .expect("a listener");
    Val::Object { class, obj }
}

#[test]
fn a_listener_that_declines_refuses_the_request_and_one_that_does_nothing_passes_it() {
    let rig = Rig::new();
    let dialogs = rig.service("dialogs");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &["Screenshot"])]);
    rig.until("the slot claim", || {
        rig.slot.state().claimed == vec!["dialogs.observe".to_owned()]
    });
    rig.slot.offer(1, "consent:Screenshot", "", b"org.example.App");
    rig.slot.offer(2, "consent:ScreenCast", "", b"org.example.App");
    rig.slot.offer(3, "secret", "Unlock the vault", b"org.example.Vault");
    rig.until("the verdicts", || {
        (1..=3).all(|request| rig.slot.answered(request).is_some())
    });
    assert_eq!(rig.slot.answered(1).as_deref(), Some("deny"));
    assert_eq!(rig.slot.answered(2).as_deref(), Some("pass"));
    assert_eq!(rig.slot.answered(3).as_deref(), Some("pass"));
    assert_eq!(
        *log.borrow(),
        vec![
            "consent:Screenshot".to_owned(),
            "consent:ScreenCast".to_owned(),
            "secret:Unlock the vault".to_owned()
        ]
    );
}

#[test]
fn a_request_that_ended_in_its_window_is_not_answered_and_an_unknown_kind_passes() {
    let rig = Rig::new();
    let dialogs = rig.service("dialogs");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &["Screenshot"])]);
    rig.until("the slot claim", || !rig.slot.state().claimed.is_empty());
    rig.slot.offer_and_finish(1, "consent:Screenshot");
    // The connection publishes both events before the shell takes a turn, as a window that closed before the shell looked.
    std::thread::sleep(Duration::from_millis(40));
    rig.slot.offer(2, "something-new", "", b"");
    rig.until("the unknown kind to pass", || rig.slot.answered(2).is_some());
    assert_eq!(rig.slot.answered(2).as_deref(), Some("pass"));
    assert_eq!(
        rig.slot.verdicts(1),
        0,
        "the window ended first: nothing to refuse: {:?}",
        rig.slot.state().answers
    );
}

#[test]
fn an_authentication_is_described_to_its_listener_and_declined_by_policy() {
    let rig = Rig::new();
    let authentication = rig.service("authentication");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.call(authentication, "add_listener", &[authentication_listener(&rig, &log)]);
    rig.until("the slot claim", || !rig.slot.state().claimed.is_empty());
    rig.slot.offer(
        7,
        "auth",
        "Authentication is required",
        br#"{"actionId":"org.freedesktop.policykit.exec","requester":"pkexec","identities":["unix-user:root","unix-user:kieran"]}"#,
    );
    rig.until("the verdict", || rig.slot.answered(7).is_some());
    assert_eq!(rig.slot.answered(7).as_deref(), Some("deny"));
    assert_eq!(
        *log.borrow(),
        vec!["auth:org.freedesktop.policykit.exec:pkexec::unix-user:root+unix-user:kieran".to_owned()]
    );
}

#[test]
fn the_slot_is_claimed_while_a_listener_could_use_it_and_released_when_none_can() {
    let rig = Rig::new();
    let dialogs = rig.service("dialogs");
    let log = Rc::new(RefCell::new(Vec::new()));
    assert!(rig.slot.state().claimed.is_empty(), "no listener, no claim");
    let listener = dialog_listener(&rig, &log, &[]);
    rig.call(dialogs, "add_listener", std::slice::from_ref(&listener));
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    rig.call(dialogs, "remove_listener", &[listener]);
    rig.until("the release", || !rig.slot.state().released.is_empty());
}

/// A generation that observes the slot: its own handle on the table and its own listener log.
struct Generation {
    ctx: u64,
    api: Api,
}

impl Generation {
    fn start(label: &str) -> Generation {
        let ctx = host::open_generation(label, abi::LANG_NATIVE).expect("a generation opens");
        Generation { ctx, api: api_of(ctx) }
    }
}

/// The rig's calls, over another generation's table.
fn on(rig: &Rig, generation: &Generation) -> Rig2 {
    let _ = rig;
    Rig2 { api: generation.api }
}

struct Rig2 {
    api: Api,
}

impl Rig2 {
    fn add_dialog_listener(&self, log: &Rc<RefCell<Vec<String>>>, decline: &'static [&'static str]) {
        let dialogs = self
            .api
            .create(self.api.class("dialogs").expect("class"), &[])
            .expect("dialogs");
        let class = self.api.class("dialog_listener").expect("class");
        let consent = self
            .api
            .member(class, abi::MEMBER_METHOD, "on_consent_request")
            .expect("a method");
        let api = self.api;
        let log = Rc::clone(log);
        let listener = self
            .api
            .implement(class, &[consent], move |_, args| {
                let request = args[0].object()?;
                let class = api.class_of(request)?;
                let subject = api.get(request, api.member(class, abi::MEMBER_PROPERTY, "subject")?)?;
                let subject = subject.text()?.to_owned();
                log.borrow_mut().push(subject.clone());
                if decline.contains(&subject.as_str()) {
                    api.call(request, api.member(class, abi::MEMBER_METHOD, "decline")?, &[])?;
                }
                Ok(Val::None)
            })
            .expect("a listener");
        let add = self
            .api
            .member(
                self.api.class_of(dialogs).expect("class"),
                abi::MEMBER_METHOD,
                "add_listener",
            )
            .expect("a method");
        self.api
            .call(dialogs, add, &[Val::Object { class, obj: listener }])
            .expect("a listener is added");
    }
}

#[test]
fn a_request_in_the_reload_gap_is_refused_never_served_by_the_built_in_dialog() {
    let rig = Rig::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &[])]);
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    // The generation ends and nothing has taken over: the gap.
    host::close_generation(rig.ctx);
    rig.slot.offer(11, "consent:Screenshot", "", b"app");
    let start = Instant::now();
    let mut waited = 0;
    while rig.slot.answered(11).is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the request was never answered"
        );
        std::thread::sleep(Duration::from_millis(5));
        waited += 1;
    }
    assert!(waited > 0);
    assert_eq!(
        rig.slot.answered(11).as_deref(),
        Some("deny"),
        "held, then refused: never passed"
    );
    assert_eq!(rig.slot.verdicts(11), 1);
    assert!(
        rig.slot
            .state()
            .answers
            .iter()
            .all(|(_, answer)| answer.as_slice() != b"pass"),
        "nothing was served unfiltered"
    );
    // Past the grace the claim stays, by default: see `after_the_grace...` for the switch.
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        rig.slot.state().released.is_empty(),
        "the default keeps the claim: the built-in dialog does not serve"
    );
}

/// What the slot connection does once the policy shell has really gone and the grace has passed, with the switch (`slots::AFTER_GRACE`)
/// set to `BuiltIn`: the claim is given back, so the compositor's built-in dialog takes every request. It is the compositor's
/// dialog, so it waits for the user: the core answers nothing for these requests and approves none, and every verdict it ever sent was `deny`
/// or `pass`, the two words that exist.
#[test]
fn with_the_switch_on_the_built_in_dialog_serves_after_the_grace_and_waits_for_the_user() {
    let rig = Rig::new();
    host::set_slot_timings(SlotTimings {
        hold: Duration::from_millis(80),
        grace: Duration::from_millis(900),
        after_grace: AfterGrace::BuiltIn,
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &["Screenshot"])]);
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    host::close_generation(rig.ctx);
    let end = Instant::now() + Duration::from_secs(5);
    while rig.slot.state().released.is_empty() {
        assert!(Instant::now() < end, "the unobserved claim was never given back");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!rig.slot.state().live, "no shell holds the slot any more");
    rig.slot.offer(51, "consent:Screenshot", "", b"app");
    std::thread::sleep(Duration::from_millis(250));
    let state = rig.slot.state();
    assert_eq!(state.builtin, vec![51], "the compositor's own dialog took the request");
    assert!(
        state.answers.iter().all(|(request, _)| *request != 51),
        "the core answered nothing for it"
    );
    assert!(
        state
            .answers
            .iter()
            .all(|(_, answer)| answer.as_slice() == b"deny" || answer.as_slice() == b"pass"),
        "no verdict but `deny` and `pass` exists: {:?}",
        state.answers
    );
    assert!(
        log.borrow().is_empty(),
        "the policy listener of the ended generation heard nothing"
    );
}

/// The default (`slots::AFTER_GRACE` is `Deny`): the claim stays and every request is refused until a policy shell runs again.
#[test]
fn by_default_after_the_grace_the_claim_stays_and_every_request_is_refused_until_a_shell_returns() {
    assert_eq!(gnoblin_core_impl::slots::AFTER_GRACE, AfterGrace::Deny);
    let rig = Rig::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &[])]);
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    host::close_generation(rig.ctx);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        rig.slot.state().released.is_empty(),
        "the claim was kept past the grace"
    );
    rig.slot.offer(61, "consent:Screenshot", "", b"app");
    let end = Instant::now() + Duration::from_secs(5);
    while rig.slot.answered(61).is_none() {
        assert!(Instant::now() < end, "the request was never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(rig.slot.answered(61).as_deref(), Some("deny"));
    assert!(
        rig.slot.state().builtin.is_empty(),
        "the built-in dialog served nothing"
    );
    // A shell that comes back takes the slot and answers from its own listeners again.
    let next = Generation::start("back");
    let log = Rc::new(RefCell::new(Vec::new()));
    on(&rig, &next).add_dialog_listener(&log, &[]);
    rig.slot.offer(62, "consent:Other", "", b"app");
    let end = Instant::now() + Duration::from_secs(5);
    while rig.slot.answered(62).is_none() {
        assert!(Instant::now() < end, "the returning shell's request was never answered");
        // SAFETY: `dispatch` is a function of the core's table.
        unsafe { (host::api().dispatch)(next.ctx, 5) };
    }
    assert_eq!(rig.slot.answered(62).as_deref(), Some("pass"));
    // The request the gap refused was shown to the returning shell late, and its verdict is moot: only the refusal was ever sent.
    assert_eq!(rig.slot.verdicts(61), 1);
    assert_eq!(rig.slot.answered(61).as_deref(), Some("deny"));
    assert!(log.borrow().contains(&"Other".to_owned()));
    host::close_generation(next.ctx);
}

/// A session in which no shell ever installed a dialog policy holds no claim: the compositor's built-in dialog serves every request as it
/// always has, and the core answers nothing and refuses nothing.
#[test]
fn a_session_that_never_installed_a_policy_leaves_every_request_to_the_built_in_dialog() {
    let rig = Rig::new();
    rig.settle();
    std::thread::sleep(Duration::from_millis(1200));
    rig.slot.offer(71, "consent:Screenshot", "", b"app");
    std::thread::sleep(Duration::from_millis(300));
    let state = rig.slot.state();
    assert!(state.claimed.is_empty(), "no policy, no claim");
    assert_eq!(state.builtin, vec![71]);
    assert!(state.answers.is_empty(), "the core answered nothing");
}

#[test]
fn a_successful_reload_hands_the_claim_over_and_only_the_new_generation_answers() {
    let rig = Rig::new();
    let old_log = Rc::new(RefCell::new(Vec::new()));
    let new_log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(
        dialogs,
        "add_listener",
        &[dialog_listener(&rig, &old_log, &["Screenshot"])],
    );
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    // The new generation takes the slot with a listener of its own, then the old one ends.
    let next = Generation::start("next");
    on(&rig, &next).add_dialog_listener(&new_log, &["ScreenCast"]);
    host::close_generation(rig.ctx);
    // The claim was kept through the hand-over: it is held once, and was not given back.
    assert_eq!(rig.slot.state().claimed.len(), 1);
    assert!(rig.slot.state().released.is_empty());
    rig.slot.offer(21, "consent:ScreenCast", "", b"app");
    rig.slot.offer(22, "consent:Screenshot", "", b"app");
    let end = Instant::now() + Duration::from_secs(5);
    while rig.slot.answered(21).is_none() || rig.slot.answered(22).is_none() {
        assert!(Instant::now() < end, "the requests were not answered");
        // SAFETY: `dispatch` is a function of the core's table.
        unsafe { (host::api().dispatch)(next.ctx, 5) };
    }
    assert_eq!(
        rig.slot.answered(21).as_deref(),
        Some("deny"),
        "the new generation's policy"
    );
    assert_eq!(rig.slot.answered(22).as_deref(), Some("pass"), "the old policy is gone");
    assert_eq!(
        *new_log.borrow(),
        vec!["ScreenCast".to_owned(), "Screenshot".to_owned()]
    );
    assert!(old_log.borrow().is_empty(), "the old generation heard nothing");
    host::close_generation(next.ctx);
}

#[test]
fn a_failed_reload_leaves_the_previous_provider_in_place() {
    let rig = Rig::new();
    let old_log = Rc::new(RefCell::new(Vec::new()));
    let new_log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(
        dialogs,
        "add_listener",
        &[dialog_listener(&rig, &old_log, &["Screenshot"])],
    );
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    // The replacement asks for the slot, then fails: it is retired without ever replacing the old one.
    let failed = Generation::start("failed");
    on(&rig, &failed).add_dialog_listener(&new_log, &[]);
    host::close_generation(failed.ctx);
    assert!(
        rig.slot.state().released.is_empty(),
        "a failed replacement does not give the claim back"
    );
    rig.slot.offer(31, "consent:Screenshot", "", b"app");
    rig.until("the previous provider's verdict", || rig.slot.answered(31).is_some());
    assert_eq!(
        rig.slot.answered(31).as_deref(),
        Some("deny"),
        "the previous provider still answers"
    );
    assert_eq!(*old_log.borrow(), vec!["consent:Screenshot".to_owned()]);
    // And the gap did not open: a request after the failed replacement is still the old provider's.
    rig.slot.offer(32, "consent:Other", "", b"app");
    rig.until("the next verdict", || rig.slot.answered(32).is_some());
    assert_eq!(rig.slot.answered(32).as_deref(), Some("pass"));
    assert_eq!(old_log.borrow().len(), 2);
}

#[test]
fn a_request_shown_to_a_generation_that_ends_is_refused_not_passed() {
    let rig = Rig::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let dialogs = rig.service("dialogs");
    rig.call(dialogs, "add_listener", &[dialog_listener(&rig, &log, &[])]);
    rig.until("the claim", || !rig.slot.state().claimed.is_empty());
    rig.slot.offer(41, "consent:Screenshot", "", b"app");
    // The poll of the slot brings the request in, and the generation ends before the verdict is sent.
    let end = Instant::now() + Duration::from_secs(5);
    while !rig.slot.state().events.is_empty() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
    host::close_generation(rig.ctx);
    let end = Instant::now() + Duration::from_secs(5);
    while rig.slot.answered(41).is_none() {
        assert!(Instant::now() < end, "the request was never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(rig.slot.answered(41).as_deref(), Some("pass"));
}

// ---- the session lock ----------------------------------------------------------------------------------------------------

#[test]
fn the_lock_state_the_node_and_the_screen_are_read_from_the_tree() {
    let rig = Rig::new();
    rig.settle();
    let lock = rig.service("session_lock");
    assert!(rig.is(&rig.prop(lock, "state"), "not-locked"));
    assert_eq!(rig.prop(lock, "node").uint(), Ok(22));
    let screen = rig.prop(lock, "screen").record().expect("a size").to_vec();
    assert_eq!(screen[0].double(), Ok(1000.0));
    assert_eq!(screen[1].double(), Ok(600.0));
    rig.comp.external_set(22, "state", text("locked"));
    rig.settle();
    assert!(rig.is(&rig.prop(lock, "state"), "locked"));
}

#[test]
fn engage_authenticate_and_heartbeat_are_calls_on_the_lock_node_and_an_answer_is_told_once() {
    let rig = Rig::new();
    rig.settle();
    let lock = rig.service("session_lock");
    let log = Rc::new(RefCell::new(Vec::new()));
    let member = rig.member(lock, abi::MEMBER_SIGNAL, "answered");
    let api = rig.api;
    let heard = Rc::clone(&log);
    let reg = rig
        .api
        .listen(lock, member, move |_, args| {
            let answer = args[0].record()?.to_vec();
            let _ = &api;
            heard.borrow_mut().push(format!(
                "{}:{}:{}",
                answer[0].uint()?,
                answer[1].text()?,
                answer[2].text()?
            ));
            Ok(Val::None)
        })
        .expect("a listener");
    let _ = reg;
    rig.call(lock, "engage", &[Val::Bool(true)]);
    rig.settle();
    assert_eq!(rig.comp.flag(22, "engaged"), Some(true));
    let token = rig
        .call(lock, "authenticate", &[Val::Secret("hunter2".to_owned())])
        .uint()
        .expect("a token");
    rig.settle();
    assert!(
        rig.comp
            .calls()
            .iter()
            .any(|(node, method)| *node == 22 && method == "authenticate")
    );
    // The answer to another call is not the check's.
    rig.comp.answer(token + 1000, "ok", "");
    rig.settle();
    assert!(log.borrow().is_empty());
    rig.comp.answer(token, "ok", "");
    rig.settle();
    assert_eq!(*log.borrow(), vec![format!("{token}:ok:")]);
    rig.comp.answer(token, "ok", "");
    rig.settle();
    assert_eq!(log.borrow().len(), 1, "an answer is told once");
    // Heartbeats go at most twice a second.
    rig.call(lock, "heartbeat", &[]);
    rig.call(lock, "heartbeat", &[]);
    rig.settle();
    let beats = || {
        rig.comp
            .calls()
            .iter()
            .filter(|(_, method)| method == "heartbeat")
            .count()
    };
    assert_eq!(beats(), 1);
    rig.advance(600);
    rig.call(lock, "heartbeat", &[]);
    rig.settle();
    assert_eq!(beats(), 2);
}

#[test]
fn a_check_the_compositor_never_answers_is_answered_failed_after_thirty_seconds() {
    let rig = Rig::new();
    rig.settle();
    let lock = rig.service("session_lock");
    let log = Rc::new(RefCell::new(Vec::new()));
    rig.hear(lock, "answered", &log, "answered");
    let token = rig
        .call(lock, "authenticate", &[Val::Text("pw".to_owned())])
        .uint()
        .expect("a token");
    rig.settle();
    rig.advance(29_000);
    assert!(log.borrow().is_empty());
    rig.advance(1_500);
    assert_eq!(log.borrow().len(), 1, "{token}");
}

// ---- virtual outputs and insets ---------------------------------------------------------------------------------------------

#[test]
fn a_virtual_output_is_checked_when_it_is_made_and_the_compositor_refuses_to_create_it() {
    let rig = Rig::new();
    let class = rig.class("virtual_output");
    let member = |name: &str| rig.api.member(class, abi::MEMBER_PROPERTY, name).expect("a property");
    let make = |name: Val, width: Val, height: Val, scale: Val| {
        rig.api.create(
            class,
            &[
                (member("name"), name),
                (member("width"), width),
                (member("height"), height),
                (member("scale"), scale),
            ],
        )
    };
    let hud = make(
        Val::Text("XR-HUD".to_owned()),
        Val::Uint(1600),
        Val::Uint(900),
        Val::Double(2.0),
    )
    .expect("a description");
    assert_eq!(rig.prop(hud, "width").uint(), Ok(1600));
    assert_eq!(rig.prop(hud, "scale").double(), Ok(2.0));
    assert!(rig.prop(hud, "attachment").is_none());
    for (name, width, height, scale) in [
        ("", 1, 1, 1.0),
        ("A", 0, 1, 1.0),
        ("A", 1, 0, 1.0),
        ("A", 1, 1, 0.0),
        ("A", 1, 1, f64::NAN),
    ] {
        let refused = make(
            Val::Text(name.to_owned()),
            Val::Uint(width),
            Val::Uint(height),
            Val::Double(scale),
        )
        .expect_err("refused");
        assert_eq!(refused.code, abi::E_TYPE, "{name} {width} {height} {scale}");
        assert!(refused.message.contains("VirtualOutput:"), "{}", refused.message);
    }
    let gnoblin = rig.service("gnoblin");
    let outputs = rig.object(rig.prop(gnoblin, "outputs"));
    let refused = rig
        .try_call(outputs, "add", &[Val::Object { class, obj: hud }])
        .expect_err("unsupported");
    assert_eq!(refused.code, abi::E_UNSUPPORTED);
}

#[test]
fn insets_grow_and_shrink_rectangles_and_compare() {
    let rig = Rig::new();
    let insets = rig.class("insets");
    let call = |name: &str, args: &[Val]| -> Val {
        let member = rig.api.member(insets, abi::MEMBER_METHOD, name).expect("a function");
        rig.api
            .call_static(insets, member, args)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let inset = |top: f64, right: f64, bottom: f64, left: f64| Val::Record {
        class: rig.class("frame_inset"),
        fields: vec![
            Val::Double(top),
            Val::Double(right),
            Val::Double(bottom),
            Val::Double(left),
        ],
    };
    let rect = |x: f64, y: f64, width: f64, height: f64| Val::Record {
        class: rig.class("rect"),
        fields: vec![Val::Double(x), Val::Double(y), Val::Double(width), Val::Double(height)],
    };
    let numbers = |value: &Val| -> Vec<f64> {
        value
            .record()
            .expect("a record")
            .iter()
            .map(|field| field.double().expect("a number"))
            .collect()
    };
    assert_eq!(numbers(&call("none", &[])), vec![0.0; 4]);
    assert_eq!(call("is_none", &[inset(0.0, 0.0, 0.0, 0.0)]).bool(), Ok(true));
    assert_eq!(call("is_none", &[inset(1.0, 0.0, 0.0, 0.0)]).bool(), Ok(false));
    assert_eq!(
        call("same", &[inset(1.0, 2.0, 3.0, 4.0), inset(1.0, 2.0, 3.0, 4.0)]).bool(),
        Ok(true)
    );
    assert_eq!(
        call("same", &[inset(1.0, 2.0, 3.0, 4.0), inset(1.0, 2.0, 3.0, 5.0)]).bool(),
        Ok(false)
    );
    assert_eq!(
        numbers(&call(
            "grow",
            &[rect(10.0, 10.0, 100.0, 50.0), inset(5.0, 6.0, 7.0, 8.0)]
        )),
        vec![2.0, 5.0, 114.0, 62.0]
    );
    assert_eq!(
        numbers(&call(
            "shrink",
            &[rect(2.0, 5.0, 114.0, 62.0), inset(5.0, 6.0, 7.0, 8.0)]
        )),
        vec![10.0, 10.0, 100.0, 50.0]
    );
    assert_eq!(
        numbers(&call("shrink", &[rect(0.0, 0.0, 4.0, 4.0), inset(9.0, 9.0, 9.0, 9.0)])),
        vec![9.0, 9.0, 0.0, 0.0]
    );
    let _ = (closure(|_, _| Ok(Val::None)), Fail::err("unused"));
}
