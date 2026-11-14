//! `gnoblin.authentication` (`data/js/gnoblin/authentication.ts`, `src/host_auth.rs`) over a fake compositor and a fake
//! slot connection.
//!
//! The page (`docs/shell-api/authentication.md`) says the request lifecycle is the compositor's: offered to the holder of
//! the `auth.polkit` slot, claimed, answered or declined, finished once. These cases are the shell's half of it: the
//! slot is held while a provider is set, the request reaches the provider with what a prompt needs, an answer is the
//! identity or a decline and nothing else, a request settles once, and the failures first: a claim the compositor
//! refuses, no connection, a connection that is lost, a provider that throws, a request that is still open when the
//! generation is retired.
//!
//! The real compositor's slot is exercised by `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gnoblin_js::JsShell;
use gnoblin_model::DeclineReason;
use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
use gnoblin_tree_client::tree_mirror::LinkError;

mod common;
use common::{FakeCompositor, honest_config};

#[derive(Default)]
struct Slots {
    /// Every call the shell made, one line each.
    calls: Vec<String>,
    inbox: Vec<SlotEvent>,
    refuse_claim: Option<String>,
    fail_poll: Option<String>,
}

#[derive(Clone, Default)]
struct FakeSlots(Rc<RefCell<Slots>>);

impl FakeSlots {
    fn offer_on(&self, slot: &str, request: u64, title: &str, message: &str, subject: &str) {
        self.0.borrow_mut().inbox.push(SlotEvent::Offered {
            request,
            deadline: 30_000,
            slot: slot.to_owned(),
            title: title.to_owned(),
            message: message.to_owned(),
            subject: subject.as_bytes().to_vec(),
        });
    }

    fn offer(&self, request: u64, title: &str, message: &str, subject: &str) {
        self.offer_on("auth.polkit", request, title, message, subject);
    }

    fn finish(&self, request: u64, completion: &'static str) {
        self.0
            .borrow_mut()
            .inbox
            .push(SlotEvent::Finished { request, completion });
    }

    fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }
}

impl SlotLink for FakeSlots {
    fn claim_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        let mut slots = self.0.borrow_mut();
        slots.calls.push(format!("claim_slot {slot}"));
        match slots.refuse_claim.clone() {
            Some(reason) => Err(LinkError(reason)),
            None => Ok(()),
        }
    }

    fn release_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.0.borrow_mut().calls.push(format!("release_slot {slot}"));
        Ok(())
    }

    fn claim_request(&mut self, request: u64) -> Result<(), LinkError> {
        self.0.borrow_mut().calls.push(format!("claim_request {request}"));
        Ok(())
    }

    fn answer(&mut self, request: u64, answer: &[u8]) -> Result<(), LinkError> {
        let text = String::from_utf8_lossy(answer).into_owned();
        self.0.borrow_mut().calls.push(format!("answer {request} {text}"));
        Ok(())
    }

    fn decline(&mut self, request: u64, reason: Option<DeclineReason>) -> Result<(), LinkError> {
        let reason = match reason {
            Some(DeclineReason::Locked) => "locked",
            Some(DeclineReason::Denied) => "denied",
            Some(DeclineReason::Unavailable) => "unavailable",
            None => "plain",
        };
        self.0.borrow_mut().calls.push(format!("decline {request} {reason}"));
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<SlotEvent>, LinkError> {
        let mut slots = self.0.borrow_mut();
        if let Some(reason) = slots.fail_poll.take() {
            return Err(LinkError(reason));
        }
        Ok(std::mem::take(&mut slots.inbox))
    }
}

struct Rig {
    shell: JsShell,
    slots: FakeSlots,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }

    fn log(&mut self) -> String {
        self.eval("log.join('|')")
    }
}

const PROVIDER: &str = "import { AuthenticationProvider } from \"gnoblin\";\n\
    class Prompt extends AuthenticationProvider {\n\
        onRequest(request) {\n\
            g.log.push('request ' + request.id + ' ' + request.title + ' / ' + request.message + ' / ' + request.subject);\n\
            g.last = request;\n\
        }\n\
        onFinished(request) {\n\
            g.log.push('finished ' + request.id + ' ' + request.completion);\n\
        }\n\
    }\n\
    g.Prompt = Prompt;\n\
    gnoblin.authentication.setProvider(new Prompt());\n";

/// A shell over a fake compositor and a fake slot connection, with `body` as its `init.ts`. `linked` says whether the
/// shell has a slot connection at all.
fn rig_with(body: &str, linked: bool) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        format!("const g = globalThis as any;\ng.g = g;\ng.log = [];\n{body}\n"),
    )
    .expect("init.ts is written");
    let slots = FakeSlots::default();
    let mut shell = JsShell::new(Box::new(FakeCompositor::new()), honest_config(&root)).expect("the shell boots");
    if linked {
        shell.set_auth_link(Box::new(slots.clone()));
    }
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Rig {
        shell,
        slots,
        _dir: dir,
    }
}

fn rig(body: &str) -> Rig {
    rig_with(body, true)
}

// --- holding the slot --------------------------------------------------------------------------------------------

#[test]
fn the_slot_is_claimed_while_a_provider_is_set_and_given_back_when_it_is_cleared() {
    let mut rig = rig(
        "g.set = () => gnoblin.authentication.setProvider({});\ng.clear = () => gnoblin.authentication.clearProvider();",
    );
    rig.frame();
    assert!(
        rig.slots.calls().is_empty(),
        "with no provider the built-in agent keeps the slot"
    );
    rig.eval("g.set()");
    rig.frame();
    rig.frame();
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit"],
        "claimed once, at the frame after the provider was set"
    );
    rig.eval("g.set()");
    rig.frame();
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit"],
        "a replacement provider keeps the claim"
    );
    rig.eval("g.clear()");
    rig.frame();
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "release_slot auth.polkit"]
    );
    assert_eq!(rig.eval("gnoblin.authentication.provider === null"), "true");
}

#[test]
fn a_claim_the_compositor_refuses_is_reported_once_and_no_request_is_heard() {
    let mut rig = rig(PROVIDER);
    rig.slots.0.borrow_mut().refuse_claim = Some("not-owner".to_owned());
    let first = rig.frame();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(
        first[0].contains("auth.polkit was not claimed") && first[0].contains("not-owner"),
        "{first:?}"
    );
    assert!(rig.frame().is_empty(), "reported once, not retried every frame");
}

#[test]
fn a_shell_with_no_slot_connection_says_so_once() {
    let mut rig = rig_with(PROVIDER, false);
    let first = rig.frame();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("no slot connection for authentication"), "{first:?}");
    assert!(rig.frame().is_empty());
}

// --- a request ---------------------------------------------------------------------------------------------------

#[test]
fn a_request_reaches_the_provider_with_what_a_prompt_needs() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(
        7,
        "Authentication required",
        "Authenticate to run /usr/bin/mount",
        "org.freedesktop.udisks2.mount",
    );
    rig.frame();
    assert_eq!(
        rig.log(),
        "request 7 Authentication required / Authenticate to run /usr/bin/mount / org.freedesktop.udisks2.mount"
    );
    assert_eq!(rig.eval("g.last.open"), "true");
    assert_eq!(rig.eval("g.last.deadline"), "30000");
    assert_eq!(rig.eval("g.last.completion === null"), "true");
    assert_eq!(rig.eval("gnoblin.authentication.requests.length"), "1");
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "claim_request 7"],
        "the shell claims a request as it is offered, and hearing it answers nothing"
    );
}

#[test]
fn allow_answers_with_the_identity_and_nothing_else() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.eval("g.last.allow('unix-user:kieran')");
    rig.frame();
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "claim_request 7", "answer 7 unix-user:kieran"]
    );
    assert_eq!(rig.eval("g.last.open"), "false");
}

#[test]
fn decline_refuses_with_a_reason_that_defaults_to_denied() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(1, "t", "m", "s");
    rig.slots.offer(2, "t", "m", "s");
    rig.slots.offer(3, "t", "m", "s");
    rig.frame();
    rig.eval("gnoblin.authentication.requests[0].decline()");
    rig.eval("gnoblin.authentication.requests[1].decline('locked')");
    rig.eval("gnoblin.authentication.requests[2].decline('unavailable')");
    rig.frame();
    assert_eq!(
        &rig.slots.calls()[1..],
        [
            "claim_request 1",
            "claim_request 2",
            "claim_request 3",
            "decline 1 denied",
            "decline 2 locked",
            "decline 3 unavailable"
        ]
    );
}

#[test]
fn a_request_is_settled_once() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.eval("g.last.allow('unix-user:kieran'); g.last.decline(); g.last.allow('unix-user:root');");
    rig.frame();
    assert_eq!(
        &rig.slots.calls()[1..],
        ["claim_request 7", "answer 7 unix-user:kieran"],
        "the first answer is the answer"
    );
}

#[test]
fn a_provider_may_settle_a_request_in_a_later_frame() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    for _ in 0..3 {
        rig.frame();
    }
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "claim_request 7"],
        "the user is still reading the prompt"
    );
    rig.eval("g.last.allow('unix-user:kieran')");
    rig.frame();
    assert_eq!(
        rig.slots.calls().last().map(String::as_str),
        Some("answer 7 unix-user:kieran")
    );
}

#[test]
fn an_identity_is_a_non_empty_string_of_at_most_256_bytes_and_a_reason_is_one_of_three() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    let attempt = |rig: &mut Rig, call: &str| {
        rig.eval(&format!(
            "(() => {{ try {{ {call}; return 'ok'; }} catch (e) {{ return e.name; }} }})()"
        ))
    };
    assert_eq!(attempt(&mut rig, "g.last.allow('')"), "TypeError");
    assert_eq!(attempt(&mut rig, "g.last.allow(5)"), "TypeError");
    assert_eq!(attempt(&mut rig, "g.last.allow('x'.repeat(257))"), "RangeError");
    assert_eq!(
        attempt(&mut rig, "g.last.allow('\u{e9}'.repeat(129))"),
        "RangeError",
        "bytes, not characters"
    );
    assert_eq!(attempt(&mut rig, "g.last.decline('maybe')"), "TypeError");
    assert_eq!(rig.eval("g.last.open"), "true", "a refused call settles nothing");
    rig.frame();
    assert_eq!(rig.slots.calls(), ["claim_slot auth.polkit", "claim_request 7"]);
}

#[test]
fn a_request_that_ended_cannot_be_answered_and_the_provider_is_told() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.slots.finish(7, "timed-out");
    rig.frame();
    assert!(rig.log().ends_with("finished 7 timed-out"), "{}", rig.log());
    assert_eq!(rig.eval("g.last.open"), "false");
    assert_eq!(rig.eval("g.last.completion"), "timed-out");
    assert_eq!(rig.eval("gnoblin.authentication.requests.length"), "0");
    rig.eval("g.last.allow('unix-user:kieran')");
    rig.frame();
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "claim_request 7"],
        "a late answer sends nothing"
    );
}

#[test]
fn a_request_of_another_slot_is_not_the_providers() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer_on("prompt.secret", 9, "Unlock", "Passphrase", "");
    rig.frame();
    assert_eq!(
        rig.log(),
        "",
        "the provider of authentication hears no passphrase prompt"
    );
}

// --- failures ----------------------------------------------------------------------------------------------------

#[test]
fn a_provider_that_throws_is_reported_once_and_the_request_stays_open_for_its_deadline() {
    let mut rig = rig("gnoblin.authentication.setProvider({ onRequest: () => { throw new Error('prompt broke'); } });");
    rig.frame();
    rig.slots.offer(1, "t", "m", "s");
    rig.slots.offer(2, "t", "m", "s");
    let diagnostics = rig.frame();
    assert_eq!(
        diagnostics.iter().filter(|line| line.contains("prompt broke")).count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(
        rig.slots.calls(),
        ["claim_slot auth.polkit", "claim_request 1", "claim_request 2"],
        "a script that failed answered nothing"
    );
}

#[test]
fn a_lost_connection_is_reported_once_and_open_requests_end_as_lost() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.slots.0.borrow_mut().fail_poll = Some("the socket closed".to_owned());
    let diagnostics = rig.frame();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("the socket closed"), "{diagnostics:?}");
    assert!(rig.log().ends_with("finished 7 lost"), "{}", rig.log());
    assert!(rig.frame().is_empty(), "reported once");
}

#[test]
fn retiring_declines_a_request_still_open_and_releases_the_slot() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.shell.retire().expect("a retire");
    assert_eq!(
        rig.slots.calls(),
        [
            "claim_slot auth.polkit",
            "claim_request 7",
            "decline 7 unavailable",
            "release_slot auth.polkit"
        ],
        "polkit's caller is told now and not at the deadline, and the built-in agent serves again"
    );
}

#[test]
fn a_request_the_provider_settled_is_not_declined_again_at_retire() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.eval("g.last.allow('unix-user:kieran')");
    rig.frame();
    rig.shell.retire().expect("a retire");
    assert_eq!(
        rig.slots.calls(),
        [
            "claim_slot auth.polkit",
            "claim_request 7",
            "answer 7 unix-user:kieran",
            "release_slot auth.polkit"
        ]
    );
}

#[test]
fn a_provider_cleared_with_a_request_open_declines_it() {
    let mut rig = rig(PROVIDER);
    rig.frame();
    rig.slots.offer(7, "t", "m", "s");
    rig.frame();
    rig.eval("gnoblin.authentication.clearProvider()");
    rig.frame();
    assert_eq!(
        &rig.slots.calls()[1..],
        ["claim_request 7", "decline 7 unavailable", "release_slot auth.polkit"]
    );
}
