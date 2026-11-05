//! Model-based test: random gates, calls and declarations, checked against the
//! invariants that hold for every one of them.
//!
//! The generator is a small hand-written linear congruential generator, so the
//! suite needs no dependency and a failure replays from its seed. Each step
//! computes the answer the documented policy implies from the *inputs*, then
//! checks what [`Adapter::serve`] actually did, so the policy is checked against
//! its statement rather than against a copy of its code.
//!
//! Invariants:
//!
//! - a call is served exactly when the mode allows its class, the caller is not
//!   sandboxed where the class is privileged, the lock is not engaged for
//!   capture, and the member is declared supported or partial;
//! - a refused call never reaches [`Adapter::handle`], and a served call reaches
//!   it exactly once;
//! - the rule a refusal reports is the first applicable one in the documented
//!   precedence: mode, then sandbox, then lock, then the declaration;
//! - the same inputs answer identically twice, and matrix renderings are a
//!   function of the declaration and not of the order it was written in.

use std::cell::Cell;

use gnoblin_compat_core::{
    Adapter, Call, CallerContext, Gate, LockState, Mode, OperationClass, Refusal, Rule, Support, SupportMatrix,
    Upstream,
};

const INTERFACE: &str = "org.gnome.Mutter.ScreenCast";

/// Members the generated adapter declares, by index. The trailing `None` is a
/// member the manifest never declares, which must be refused as undeclared.
const DECLARED: [Option<Declared>; 6] = [
    Some(Declared::Supported),
    Some(Declared::Partial),
    Some(Declared::Unsupported),
    Some(Declared::Partial),
    Some(Declared::Supported),
    None,
];

const NAMES: [&str; 6] = ["m0", "m1", "m2", "m3", "m4", "m5"];

/// Sequences per test run and steps per sequence.
const SEQUENCES: u64 = 128;
const STEPS: u32 = 96;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Declared {
    Supported,
    Partial,
    Unsupported,
}

/// A 64-bit linear congruential generator: small, hand-written and identical on
/// every platform, so a failing seed replays exactly.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn mode(&mut self) -> Mode {
        match self.below(3) {
            0 => Mode::Off,
            1 => Mode::Portal,
            _ => Mode::Unmediated,
        }
    }

    fn caller(&mut self) -> CallerContext {
        match self.below(3) {
            0 => CallerContext::UNSANDBOXED,
            1 => CallerContext::flatpak(),
            _ => CallerContext::security_context(),
        }
    }

    fn lock(&mut self) -> LockState {
        match self.below(3) {
            0 => LockState::Unlocked,
            1 => LockState::Locking,
            _ => LockState::Locked,
        }
    }

    fn operation(&mut self) -> OperationClass {
        match self.below(4) {
            0 => OperationClass::Read,
            1 => OperationClass::Write,
            2 => OperationClass::Capture,
            _ => OperationClass::InputInjection,
        }
    }
}

/// One call: a member index and the class the adapter declares for it.
struct ModelCall {
    index: usize,
    operation: OperationClass,
}

impl Call for ModelCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        NAMES[self.index]
    }

    fn operation(&self) -> OperationClass {
        self.operation
    }
}

struct Model {
    matrix: SupportMatrix,
    served: Cell<u32>,
}

impl Model {
    fn new() -> Self {
        let matrix = SupportMatrix::builder()
            .interface(INTERFACE, 4, |members| {
                for (name, declared) in NAMES.iter().zip(DECLARED) {
                    match declared {
                        Some(Declared::Supported) => members.method(*name, Support::Supported),
                        Some(Declared::Partial) => members.method(*name, Support::partial("partially served")),
                        Some(Declared::Unsupported) => members.method(*name, Support::unsupported("never served")),
                        None => {}
                    }
                }
            })
            .build();
        Self {
            matrix,
            served: Cell::new(0),
        }
    }
}

impl Adapter for Model {
    type Call = ModelCall;
    type Reply = &'static str;

    fn name(&self) -> &'static str {
        "model"
    }

    fn upstream(&self) -> Upstream {
        Upstream::new(INTERFACE, 4)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    fn handle(&self, _call: ModelCall) -> Result<Self::Reply, Refusal> {
        self.served.set(self.served.get() + 1);
        Ok("served")
    }
}

/// The rule the documented precedence names for these inputs, which must be the
/// one the frame reports.
fn expected_rule(
    mode: Mode,
    operation: OperationClass,
    caller: CallerContext,
    lock: LockState,
    declared: Option<Declared>,
) -> Option<Rule> {
    match mode {
        Mode::Off => return Some(Rule::ModeOff),
        Mode::Portal => match operation {
            OperationClass::Read => {}
            OperationClass::Write => return Some(Rule::WriteNotEnabled),
            OperationClass::Capture | OperationClass::InputInjection => return Some(Rule::PortalOnly),
        },
        Mode::Unmediated => {}
    }
    if caller.is_sandboxed() {
        match operation {
            OperationClass::Capture => return Some(Rule::SandboxedCapture),
            OperationClass::InputInjection => return Some(Rule::SandboxedInput),
            OperationClass::Read | OperationClass::Write => {}
        }
    }
    if operation == OperationClass::Capture && lock.is_engaged() {
        return Some(Rule::LockedCapture);
    }
    match declared {
        Some(Declared::Supported | Declared::Partial) => None,
        Some(Declared::Unsupported) => Some(Rule::Unsupported),
        None => Some(Rule::Undeclared),
    }
}

#[test]
fn every_gate_answers_as_the_policy_says() {
    for seed in 0..SEQUENCES {
        let mut rng = Rng::new(seed);
        let adapter = Model::new();
        for step in 0..STEPS {
            let mode = rng.mode();
            let caller = rng.caller();
            let lock = rng.lock();
            let operation = rng.operation();
            let index = usize::try_from(rng.below(NAMES.len() as u64)).unwrap_or(0);
            let gate = Gate::new(mode, caller, lock);

            let before = adapter.served.get();
            let result = adapter.serve(ModelCall { index, operation }, gate);
            let expected = expected_rule(mode, operation, caller, lock, DECLARED[index]);

            match (&result, expected) {
                (Ok(reply), None) => {
                    assert_eq!(*reply, "served");
                    assert_eq!(
                        adapter.served.get(),
                        before + 1,
                        "a served call reaches the mapping once"
                    );
                }
                (Err(refusal), Some(rule)) => {
                    assert_eq!(
                        refusal.rule(),
                        rule,
                        "seed {seed} step {step}: {mode}/{caller:?}/{lock}/{operation}"
                    );
                    assert_eq!(refusal.adapter(), "model");
                    assert_eq!(refusal.interface(), Some(INTERFACE));
                    assert_eq!(refusal.member(), Some(NAMES[index]));
                    assert_eq!(refusal.operation(), operation);
                    assert_eq!(adapter.served.get(), before, "a refused call never reaches the mapping");
                }
                (Ok(_), Some(rule)) => panic!("seed {seed} step {step}: served where {rule} was expected"),
                (Err(refusal), None) => {
                    panic!(
                        "seed {seed} step {step}: refused with {} where serving was expected",
                        refusal.rule()
                    )
                }
            }

            // The same inputs answer identically, which is the promise a replayed
            // log line depends on.
            let again = adapter.serve(ModelCall { index, operation }, gate);
            match (&result, &again) {
                (Ok(left), Ok(right)) => assert_eq!(left, right),
                (Err(left), Err(right)) => assert_eq!(left, right),
                _ => panic!("seed {seed} step {step}: the same inputs answered differently"),
            }
        }
    }
}

#[test]
fn a_matrix_renders_the_same_bytes_for_the_same_declaration() {
    let adapter = Model::new();
    let text = adapter.support_matrix().render_text();
    let json = adapter.support_matrix().render_json();
    assert_eq!(
        text.lines().next(),
        Some("interface org.gnome.Mutter.ScreenCast version 4")
    );
    assert!(text.contains("  method m0 supported"));
    assert!(text.contains("  method m1 partial: partially served"));
    assert!(text.contains("  method m2 unsupported: never served"));
    assert!(!text.contains("m5"), "an undeclared member is not in the report");
    assert_eq!(SupportMatrix::from_json(&json).unwrap().render_text(), text);
    assert_eq!(adapter.support_matrix().render_json(), json);
}
