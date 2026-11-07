//! Admission tests: the mode gate, the lock embargo and the matrix, without a bus.

use gnoblin_compat_core::{Call, CallerContext, Gate, LockState, Mode, Rule, Support};
use gnoblin_compat_dbus_mutter_inputcapture::{
    INTERFACE, InputCaptureCall, SESSION_INTERFACE, compat::InputCaptureAdmission,
};

fn gate(mode: Mode, lock: LockState) -> Gate {
    Gate::new(mode, CallerContext::UNSANDBOXED, lock)
}

#[test]
fn the_default_mode_refuses_input_injection() {
    let admission = InputCaptureAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(InputCaptureCall::ConnectToEIS, gate(Mode::Portal, LockState::Unlocked))
        .expect_err("portal mode must refuse input injection");
    assert_eq!(refusal.rule(), Rule::PortalOnly);
}

#[test]
fn the_lock_refuses_input_injection() {
    let admission = InputCaptureAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(
            InputCaptureCall::ConnectToEIS,
            gate(Mode::Unmediated, LockState::Locked),
        )
        .expect_err("a locked session must refuse input injection");
    assert_eq!(refusal.rule(), Rule::LockedCapture);
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    let admission = InputCaptureAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(InputCaptureCall::Enable, gate(Mode::Unmediated, LockState::Unlocked))
        .expect_err("a declared-unsupported member must refuse");
    assert_eq!(refusal.rule(), Rule::Unsupported);
    let text = refusal.to_string();
    assert!(text.contains("Enable"), "the refusal names the member: {text}");
}

#[test]
fn every_call_has_a_declared_member() {
    let admission = InputCaptureAdmission::load().expect("the checked-in matrix loads");
    for call in ALL {
        let interface = call.interface();
        assert!(interface == INTERFACE || interface == SESSION_INTERFACE);
        let support = admission
            .matrix()
            .support(interface, call.member())
            .unwrap_or_else(|| panic!("{} is not declared", call.member()));
        assert!(
            matches!(support, Support::Unsupported { .. }),
            "{} is declared unsupported",
            call.member()
        );
    }
}

const ALL: [InputCaptureCall; 10] = [
    InputCaptureCall::CreateSession,
    InputCaptureCall::SupportedCapabilities,
    InputCaptureCall::GetZones,
    InputCaptureCall::AddBarrier,
    InputCaptureCall::ClearBarriers,
    InputCaptureCall::Enable,
    InputCaptureCall::Disable,
    InputCaptureCall::ConnectToEIS,
    InputCaptureCall::Release,
    InputCaptureCall::Close,
];
