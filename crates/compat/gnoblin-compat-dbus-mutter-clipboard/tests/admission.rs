//! Admission tests: the mode gate, the lock embargo and the matrix, without a bus.

use gnoblin_compat_core::{Call, CallerContext, Gate, LockState, Mode, Rule, Support};
use gnoblin_compat_dbus_mutter_clipboard::{ClipboardAdmission, ClipboardCall, INTERFACE};

const ALL: [ClipboardCall; 6] = [
    ClipboardCall::Enable,
    ClipboardCall::Disable,
    ClipboardCall::SetSelection,
    ClipboardCall::SelectionWrite,
    ClipboardCall::SelectionWriteDone,
    ClipboardCall::SelectionRead,
];

fn gate(mode: Mode, lock: LockState) -> Gate {
    Gate::new(mode, CallerContext::UNSANDBOXED, lock)
}

#[test]
fn the_default_mode_refuses_a_selection_read() {
    let admission = ClipboardAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(ClipboardCall::SelectionRead, gate(Mode::Portal, LockState::Unlocked))
        .expect_err("portal mode must refuse a capture-class read");
    assert_eq!(refusal.rule(), Rule::PortalOnly);
    assert!(
        refusal.to_string().contains("SelectionRead"),
        "the refusal names the member: {refusal}"
    );
}

#[test]
fn the_lock_refuses_a_selection_read() {
    // The embargo is for capture and input, not writes: a read moves contents out.
    let admission = ClipboardAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(ClipboardCall::SelectionRead, gate(Mode::Unmediated, LockState::Locked))
        .expect_err("a locked session must refuse a capture-class read");
    assert_eq!(refusal.rule(), Rule::LockedCapture);
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    let admission = ClipboardAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(ClipboardCall::Enable, gate(Mode::Unmediated, LockState::Unlocked))
        .expect_err("a declared-unsupported member must refuse");
    assert_eq!(refusal.rule(), Rule::Unsupported);
    let text = refusal.to_string();
    assert!(text.contains("Enable"), "the refusal names the member: {text}");
    assert!(
        text.contains("no clipboard IPC object"),
        "the refusal quotes the matrix reason: {text}"
    );
}

#[test]
fn every_call_has_a_declared_member() {
    let admission = ClipboardAdmission::load().expect("the checked-in matrix loads");
    for call in ALL {
        assert_eq!(call.interface(), INTERFACE);
        let support = admission
            .matrix()
            .support(call.interface(), call.member())
            .unwrap_or_else(|| panic!("{} is not declared", call.member()));
        assert!(
            matches!(support, Support::Unsupported { .. }),
            "{} is declared unsupported",
            call.member()
        );
    }
}
