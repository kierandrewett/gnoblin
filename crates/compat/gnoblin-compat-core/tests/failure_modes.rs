//! One test per failure mode in the crate documentation.
//!
//! Every test goes through the public frame only: a call type, an adapter and
//! [`Adapter::serve`]. A test here fails when the frame could let a call past a
//! refusal it promised, which is the property each failure-mode entry names.

use std::cell::Cell;

use gnoblin_compat_core::{
    Adapter, Call, CallerContext, Gate, Kind, LockState, MatrixError, Mode, OperationClass, Refusal, Rule, Support,
    SupportMatrix, Upstream, allow,
};

const SCREENCAST: &str = "org.gnome.Mutter.ScreenCast";
const IDLE: &str = "org.gnome.Mutter.IdleMonitor";

/// One call the mock adapter can be asked for, one variant per member.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Request {
    CreateSession,
    Start,
    Stop,
    InjectKey,
    Version,
    Undeclared,
    Foreign,
}

impl Call for Request {
    fn interface(&self) -> &'static str {
        match self {
            Self::Foreign => IDLE,
            _ => SCREENCAST,
        }
    }

    fn member(&self) -> &'static str {
        match self {
            Self::CreateSession => "CreateSession",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::InjectKey => "InjectKey",
            Self::Version => "Version",
            Self::Undeclared => "Undeclared",
            Self::Foreign => "GetIdletime",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            Self::CreateSession => OperationClass::Write,
            Self::Start | Self::Stop => OperationClass::Capture,
            Self::InjectKey => OperationClass::InputInjection,
            Self::Version | Self::Undeclared | Self::Foreign => OperationClass::Read,
        }
    }
}

/// A capture adapter that counts the calls that reached its mapping.
struct Mock {
    name: &'static str,
    matrix: SupportMatrix,
    served: Cell<u32>,
}

impl Mock {
    fn screencast() -> Self {
        Self::named("screencast", screencast_matrix())
    }

    fn named(name: &'static str, matrix: SupportMatrix) -> Self {
        Self {
            name,
            matrix,
            served: Cell::new(0),
        }
    }
}

impl Adapter for Mock {
    type Call = Request;
    type Reply = &'static str;

    fn name(&self) -> &'static str {
        self.name
    }

    fn upstream(&self) -> Upstream {
        Upstream::new(SCREENCAST, 4)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    fn handle(&self, _call: Request) -> Result<Self::Reply, Refusal> {
        self.served.set(self.served.get() + 1);
        Ok("served")
    }
}

/// An adapter whose mapping refuses a call the gate admitted.
struct Refusing {
    matrix: SupportMatrix,
}

impl Refusing {
    fn new() -> Self {
        let matrix = SupportMatrix::builder()
            .interface(SCREENCAST, 4, |members| {
                members.property("Version", Support::Supported);
            })
            .build();
        Self { matrix }
    }
}

impl Adapter for Refusing {
    type Call = Request;
    type Reply = &'static str;

    fn name(&self) -> &'static str {
        "refusing"
    }

    fn upstream(&self) -> Upstream {
        Upstream::new(SCREENCAST, 4)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    fn handle(&self, _call: Request) -> Result<Self::Reply, Refusal> {
        Err(Refusal::new(
            "refusing",
            OperationClass::Read,
            Rule::Adapter,
            "the mapping has no session to answer from",
        ))
    }
}

fn screencast_matrix() -> SupportMatrix {
    SupportMatrix::builder()
        .interface(SCREENCAST, 4, |members| {
            members.method("CreateSession", Support::Supported);
            members.method("Start", Support::partial("recording starts with the session"));
            members.method("Stop", Support::unsupported("streams end with their session"));
            members.method("InjectKey", Support::Supported);
            members.property("Version", Support::Supported);
        })
        .build()
}

fn unlocked(mode: Mode) -> Gate {
    Gate::new(mode, CallerContext::UNSANDBOXED, LockState::Unlocked)
}

/// The frame refuses a capture while the cover is presented, and the mapping is
/// never reached: this is the lock embargo the plan calls out for `compat/`.
#[test]
fn capture_is_refused_while_locked() {
    let adapter = Mock::screencast();
    let gate = Gate::new(Mode::Unmediated, CallerContext::UNSANDBOXED, LockState::Locked);
    let refusal = adapter.serve(Request::Start, gate).unwrap_err();
    assert_eq!(refusal.rule(), Rule::LockedCapture);
    assert_eq!(refusal.adapter(), "screencast");
    assert_eq!(refusal.interface(), Some(SCREENCAST));
    assert_eq!(refusal.member(), Some("Start"));
    assert_eq!(refusal.operation(), OperationClass::Capture);
    assert_eq!(adapter.served.get(), 0, "a refused call must not reach the mapping");
}

/// The transition before the cover is presented refuses capture too, because a
/// stream opened then would outlive the moment the screen is covered.
#[test]
fn capture_during_the_lock_transition_is_refused() {
    let adapter = Mock::screencast();
    let gate = Gate::new(Mode::Unmediated, CallerContext::UNSANDBOXED, LockState::Locking);
    let refusal = adapter.serve(Request::Start, gate).unwrap_err();
    assert_eq!(refusal.rule(), Rule::LockedCapture);
    assert_eq!(adapter.served.get(), 0);
}

/// A sandboxed caller never captures, in any mode: the refusal is the sandbox's
/// even where the mode alone would have admitted the call.
#[test]
fn a_sandboxed_caller_never_reaches_capture() {
    for caller in [CallerContext::flatpak(), CallerContext::security_context()] {
        for lock in [LockState::Unlocked, LockState::Locking, LockState::Locked] {
            for (mode, expected) in [
                (Mode::Off, Rule::ModeOff),
                (Mode::Portal, Rule::PortalOnly),
                (Mode::Unmediated, Rule::SandboxedCapture),
            ] {
                let adapter = Mock::screencast();
                let refusal = adapter
                    .serve(Request::Start, Gate::new(mode, caller, lock))
                    .unwrap_err();
                assert_eq!(refusal.rule(), expected, "mode {mode}, caller {caller}, lock {lock}");
                assert_eq!(adapter.served.get(), 0, "mode {mode}, caller {caller}, lock {lock}");
            }
        }
    }
}

/// Input injection is refused a sandboxed caller in every mode as well, and in
/// `unmediated` that is the sandbox rule rather than the mode's.
#[test]
fn a_sandboxed_caller_never_reaches_input_injection() {
    for caller in [CallerContext::flatpak(), CallerContext::security_context()] {
        for lock in [LockState::Unlocked, LockState::Locking, LockState::Locked] {
            for (mode, expected) in [
                (Mode::Off, Rule::ModeOff),
                (Mode::Portal, Rule::PortalOnly),
                (Mode::Unmediated, Rule::SandboxedInput),
            ] {
                let adapter = Mock::screencast();
                let refusal = adapter
                    .serve(Request::InjectKey, Gate::new(mode, caller, lock))
                    .unwrap_err();
                assert_eq!(refusal.rule(), expected, "mode {mode}, caller {caller}, lock {lock}");
                assert_eq!(adapter.served.get(), 0, "mode {mode}, caller {caller}, lock {lock}");
            }
        }
    }
}

/// An unsupported member is refused with the reason the manifest gives, so the
/// caller learns what is missing instead of depending on a silent no-op.
#[test]
fn an_unsupported_member_is_refused_with_its_reason() {
    let adapter = Mock::screencast();
    let refusal = adapter.serve(Request::Stop, unlocked(Mode::Unmediated)).unwrap_err();
    assert_eq!(refusal.rule(), Rule::Unsupported);
    assert_eq!(refusal.member(), Some("Stop"));
    assert_eq!(refusal.detail(), "streams end with their session");
    assert!(refusal.to_string().contains("unsupported"));
    assert_eq!(adapter.served.get(), 0);
}

/// A member the manifest never mentions is refused as well: absent is not the
/// same as supported.
#[test]
fn an_undeclared_member_is_refused() {
    let adapter = Mock::screencast();
    let refusal = adapter
        .serve(Request::Undeclared, unlocked(Mode::Unmediated))
        .unwrap_err();
    assert_eq!(refusal.rule(), Rule::Undeclared);
    assert_eq!(refusal.member(), Some("Undeclared"));
    assert!(refusal.detail().contains("Undeclared"));
    assert_eq!(adapter.served.get(), 0);
}

/// A matrix only speaks for the interfaces it declares, so a call on another
/// interface is undeclared rather than guessed from a same-named member.
#[test]
fn a_matrix_for_another_interface_is_not_consulted() {
    let matrix = SupportMatrix::builder()
        .interface(SCREENCAST, 4, |members| {
            members.method("GetIdletime", Support::Supported);
        })
        .build();
    let adapter = Mock::named("screencast", matrix);
    let refusal = adapter.serve(Request::Foreign, unlocked(Mode::Unmediated)).unwrap_err();
    assert_eq!(refusal.rule(), Rule::Undeclared);
    assert_eq!(refusal.interface(), Some(IDLE));
    assert!(refusal.detail().contains(IDLE));
    assert_eq!(adapter.served.get(), 0);
}

/// The default mode is D13's `portal`, so a state change is refused out of the
/// box and a user has to enable it.
#[test]
fn the_default_mode_refuses_writes() {
    assert_eq!(Mode::default(), Mode::Portal);
    assert_eq!(Gate::default().mode(), Mode::Portal);
    assert_eq!(Gate::default().caller(), CallerContext::UNSANDBOXED);
    assert_eq!(Gate::default().lock(), LockState::Unlocked);
    let adapter = Mock::screencast();
    let refusal = adapter.serve(Request::CreateSession, Gate::default()).unwrap_err();
    assert_eq!(refusal.rule(), Rule::WriteNotEnabled);
    assert_eq!(refusal.operation(), OperationClass::Write);
    assert_eq!(adapter.served.get(), 0);
}

/// `off` refuses every class, reads included, because a disabled adapter owns no
/// name at all and must not answer as if it did.
#[test]
fn the_off_mode_refuses_reads_too() {
    for request in [
        Request::Version,
        Request::CreateSession,
        Request::Start,
        Request::InjectKey,
    ] {
        let adapter = Mock::screencast();
        let refusal = adapter.serve(request, unlocked(Mode::Off)).unwrap_err();
        assert_eq!(refusal.rule(), Rule::ModeOff, "{request:?}");
        assert_eq!(adapter.served.get(), 0);
    }
}

/// The default mode keeps capture and input away from unsandboxed callers too:
/// they go through the portal, which is not this adapter.
#[test]
fn portal_mode_refuses_capture_and_input_even_unsandboxed() {
    for request in [Request::Start, Request::InjectKey] {
        let adapter = Mock::screencast();
        let refusal = adapter.serve(request, unlocked(Mode::Portal)).unwrap_err();
        assert_eq!(refusal.rule(), Rule::PortalOnly, "{request:?}");
        assert_eq!(adapter.served.get(), 0);
    }
}

/// `unmediated` is the mode that serves what the plan permits: reads, writes,
/// capture and input, for an unsandboxed caller, while the session is unlocked.
#[test]
fn unmediated_serves_what_the_plan_allows() {
    for request in [
        Request::Version,
        Request::CreateSession,
        Request::Start,
        Request::InjectKey,
    ] {
        let adapter = Mock::screencast();
        assert_eq!(adapter.serve(request, unlocked(Mode::Unmediated)).unwrap(), "served");
        assert_eq!(adapter.served.get(), 1, "{request:?}");
    }
}

/// `allow` is the whole gate in one pure function: it answers from its five
/// inputs and knows no interface or member, which is why it names neither.
#[test]
fn the_decision_function_answers_from_its_inputs_alone() {
    let allowed = allow(
        "idle",
        OperationClass::Write,
        CallerContext::UNSANDBOXED,
        Mode::Unmediated,
        LockState::Unlocked,
    );
    assert!(allowed.is_allowed());
    assert!(allowed.refusal().is_none());

    let refused = allow(
        "idle",
        OperationClass::Capture,
        CallerContext::UNSANDBOXED,
        Mode::Unmediated,
        LockState::Locked,
    );
    assert!(!refused.is_allowed());
    let refusal = refused.refusal().unwrap();
    assert_eq!(refusal.adapter(), "idle");
    assert_eq!(refusal.rule(), Rule::LockedCapture);
    assert_eq!(refusal.interface(), None);
    assert_eq!(refusal.member(), None);
    assert_eq!(
        refused,
        allow(
            "idle",
            OperationClass::Capture,
            CallerContext::UNSANDBOXED,
            Mode::Unmediated,
            LockState::Locked
        ),
        "the same inputs must answer the same way"
    );
}

/// An adapter still refuses after the gate admits a call, and that refusal is
/// the adapter's own rule, not a silent success.
#[test]
fn an_adapter_may_refuse_an_admitted_call() {
    let adapter = Refusing::new();
    let refusal = adapter.serve(Request::Version, unlocked(Mode::Unmediated)).unwrap_err();
    assert_eq!(refusal.rule(), Rule::Adapter);
    assert_eq!(refusal.adapter(), "refusing");
    assert!(refusal.detail().contains("no session"));
}

/// A refusal reads as one line that names the adapter, the member, the operation
/// and the rule, so a log or a bug report identifies all four.
#[test]
fn a_refusal_names_the_adapter_operation_and_rule() {
    let adapter = Mock::screencast();
    let gate = Gate::new(Mode::Unmediated, CallerContext::UNSANDBOXED, LockState::Locked);
    let text = adapter.serve(Request::Start, gate).unwrap_err().to_string();
    assert!(text.contains("screencast"), "{text}");
    assert!(text.contains(&format!("{SCREENCAST}.Start")), "{text}");
    assert!(text.contains("capture"), "{text}");
    assert!(text.contains("locked-capture"), "{text}");
    assert!(text.contains("refused"), "{text}");
}

/// Two adapters that both serve `Start` stay distinguishable: each refusal names
/// its adapter and interface, and each report carries its own declaration.
#[test]
fn two_adapters_disagree_visibly() {
    let partial = Mock::named("screencast", screencast_matrix());
    let refusing = Mock::named(
        "screencast-legacy",
        SupportMatrix::builder()
            .interface(SCREENCAST, 3, |members| {
                members.method(
                    "Start",
                    Support::unsupported("the legacy adapter never started a session here"),
                );
            })
            .build(),
    );

    assert_eq!(
        partial.serve(Request::Start, unlocked(Mode::Unmediated)).unwrap(),
        "served"
    );
    let refusal = refusing.serve(Request::Start, unlocked(Mode::Unmediated)).unwrap_err();
    assert_eq!(refusal.rule(), Rule::Unsupported);
    assert_eq!(refusal.adapter(), "screencast-legacy");
    assert_eq!(refusing.served.get(), 0);

    let partial_report = partial.support_matrix().render_text();
    let refusing_report = refusing.support_matrix().render_text();
    assert_ne!(partial_report, refusing_report);
    assert!(partial_report.contains(&format!("interface {SCREENCAST} version 4")));
    assert!(refusing_report.contains(&format!("interface {SCREENCAST} version 3")));
}

/// A manifest declares what the frame can honour, and nothing else is accepted
/// as if it did.
#[test]
fn a_matrix_round_trips_through_its_manifest() {
    let matrix = screencast_matrix();
    let text = matrix.render_json();
    assert_eq!(SupportMatrix::from_json(&text).unwrap(), matrix);
    assert!(text.ends_with("}\n"), "a manifest ends with a newline");
}

/// Both renderings and the builder agree with the frame's checked-in fixture, so
/// `cargo xtask compat-report` and an adapter's manifest cannot drift apart.
#[test]
fn the_renderings_match_the_frame_fixture() {
    let text = include_str!("../fixtures/matrix.json");
    let matrix = SupportMatrix::from_json(text).unwrap();
    assert_eq!(matrix.render_json(), text);
    assert_eq!(matrix.render_text(), include_str!("../fixtures/matrix.txt"));
    assert_eq!(matrix, fixture_matrix());
}

/// Report order is the frame's, not the declaration's, so a reordered manifest
/// cannot produce a diff that says nothing changed.
#[test]
fn the_matrix_ordering_is_stable() {
    let forwards = SupportMatrix::builder()
        .interface(SCREENCAST, 4, |members| {
            members.method("CreateSession", Support::Supported);
            members.property("Version", Support::Supported);
            members.method("Start", Support::partial("recording starts with the session"));
        })
        .interface(IDLE, 1, |members| {
            members.method("GetIdletime", Support::Supported);
        })
        .build();
    let backwards = SupportMatrix::builder()
        .interface(IDLE, 1, |members| {
            members.method("GetIdletime", Support::Supported);
        })
        .interface(SCREENCAST, 4, |members| {
            members.method("Start", Support::partial("recording starts with the session"));
            members.property("Version", Support::Supported);
            members.method("CreateSession", Support::Supported);
        })
        .build();
    assert_eq!(forwards.render_text(), backwards.render_text());
    assert_eq!(forwards.render_json(), backwards.render_json());
    assert!(
        forwards
            .render_text()
            .starts_with(&format!("interface {IDLE} version 1\n")),
        "interfaces are ordered by name, not by declaration order"
    );
}

/// An empty declaration serves nothing, which is what "not written yet" must
/// mean rather than answering plausibly.
#[test]
fn an_adapter_with_no_declared_members_refuses_everything() {
    let adapter = Mock::named("empty", SupportMatrix::new());
    assert!(adapter.support_matrix().is_empty());
    for request in [
        Request::Version,
        Request::CreateSession,
        Request::Start,
        Request::InjectKey,
    ] {
        let refusal = adapter.serve(request, unlocked(Mode::Unmediated)).unwrap_err();
        assert_eq!(refusal.rule(), Rule::Undeclared, "{request:?}");
    }
    assert_eq!(adapter.served.get(), 0);
    assert_eq!(SupportMatrix::new().render_json(), "{\n  \"interfaces\": []\n}\n");
    assert_eq!(SupportMatrix::new().render_text(), "");
}

/// The configured value is parsed exactly, so a typo is a configuration problem
/// rather than a mode nobody chose.
#[test]
fn the_modes_parse_from_their_configuration_value() {
    assert_eq!(Mode::parse("off"), Some(Mode::Off));
    assert_eq!(Mode::parse("portal"), Some(Mode::Portal));
    assert_eq!(Mode::parse("unmediated"), Some(Mode::Unmediated));
    assert_eq!(Mode::parse("on"), None);
    assert_eq!(Mode::parse("Portal"), None);
    assert_eq!(Mode::Off.to_string(), "off");
}

#[test]
fn a_manifest_with_an_unknown_support_word_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"mostly"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[0].support");
    assert!(error.message().contains("mostly"), "{}", error.message());
}

#[test]
fn a_partial_member_without_a_note_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"partial"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[0].note");
}

#[test]
fn an_unsupported_member_without_a_reason_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"unsupported"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[0].reason");
}

#[test]
fn a_supported_member_with_an_explanation_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"supported","reason":"why not"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[0]");
    assert!(error.message().contains("neither a note nor a reason"));
}

#[test]
fn an_unknown_key_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"supported","supoort":"supported"}
        ]}]}"#,
    );
    assert!(error.message().contains("supoort"), "{}", error.message());
}

#[test]
fn an_unknown_member_kind_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"attribute","name":"M","support":"supported"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[0].kind");
}

#[test]
fn a_repeated_member_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[{"name":"i","version":1,"members":[
            {"kind":"method","name":"M","support":"supported"},
            {"kind":"method","name":"M","support":"supported"}
        ]}]}"#,
    );
    assert_eq!(error.path(), "interfaces[0].members[1]");
    assert!(error.message().contains("already declared"));
}

#[test]
fn a_repeated_interface_is_rejected() {
    let error = manifest_error(
        r#"{"interfaces":[
            {"name":"i","version":1,"members":[]},
            {"name":"i","version":2,"members":[]}
        ]}"#,
    );
    assert_eq!(error.path(), "interfaces[1]");
    assert!(error.message().contains("interfaces[0]"));
}

#[test]
fn a_manifest_that_is_not_an_object_is_rejected() {
    let error = manifest_error("[]");
    assert_eq!(error.path(), "<document>");
    assert!(error.message().contains("expected an object"));
}

fn manifest_error(text: &str) -> MatrixError {
    SupportMatrix::from_json(text).unwrap_err()
}

/// The fixture declaration, built the way an adapter builds it, so the builder
/// and the checked-in manifest are proved equal rather than assumed equal.
fn fixture_matrix() -> SupportMatrix {
    SupportMatrix::builder()
        .interface("org.gnome.Mutter.DisplayConfig", 3, |members| {
            members.method(
                "ApplyConfiguration",
                Support::unsupported("the legacy variant is not served; callers use ApplyMonitorsConfig"),
            );
            members.method(
                "ApplyMonitorsConfig",
                Support::partial("applies monitors and logical monitors, but not a layout mode change"),
            );
            members.method("GetCurrentState", Support::Supported);
            members.signal("MonitorsChanged", Support::Supported);
        })
        .interface(IDLE, 1, |members| {
            members.method("AddIdleWatch", Support::Supported);
            members.method("GetIdletime", Support::Supported);
            members.method("RemoveWatch", Support::Supported);
            members.signal("WatchFired", Support::Supported);
        })
        .build()
}

#[test]
fn a_member_kind_reads_as_its_manifest_spelling() {
    assert_eq!(Kind::parse("method"), Some(Kind::Method));
    assert_eq!(Kind::parse("property"), Some(Kind::Property));
    assert_eq!(Kind::parse("signal"), Some(Kind::Signal));
    assert_eq!(Kind::parse("Method"), None);
}
