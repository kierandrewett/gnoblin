//! Admission tests: the mode gate, the lock embargo and the matrix, without a bus.

use gnoblin_compat_core::{Call, CallerContext, Gate, LockState, Mode, Rule, Support};
use gnoblin_compat_mutter_remotedesktop::{
    INTERFACE, RemoteDesktopCall, SESSION_INTERFACE, compat::RemoteDesktopAdmission,
};

fn gate(mode: Mode, lock: LockState) -> Gate {
    Gate::new(mode, CallerContext::UNSANDBOXED, lock)
}

#[test]
fn the_default_mode_refuses_input_injection() {
    let admission = RemoteDesktopAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(
            RemoteDesktopCall::NotifyKeyboardKeycode,
            gate(Mode::Portal, LockState::Unlocked),
        )
        .expect_err("portal mode must refuse input injection");
    assert_eq!(refusal.rule(), Rule::PortalOnly);
}

#[test]
fn the_lock_refuses_input_injection() {
    let admission = RemoteDesktopAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(
            RemoteDesktopCall::NotifyPointerMotionRelative,
            gate(Mode::Unmediated, LockState::Locked),
        )
        .expect_err("a locked session must refuse input injection");
    assert_eq!(refusal.rule(), Rule::LockedCapture);
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    let admission = RemoteDesktopAdmission::load().expect("the checked-in matrix loads");
    let refusal = admission
        .admit(RemoteDesktopCall::Start, gate(Mode::Unmediated, LockState::Unlocked))
        .expect_err("a declared-unsupported member must refuse");
    assert_eq!(refusal.rule(), Rule::Unsupported);
    let text = refusal.to_string();
    assert!(text.contains("Start"), "the refusal names the member: {text}");
}

#[test]
fn every_call_has_a_declared_member() {
    let admission = RemoteDesktopAdmission::load().expect("the checked-in matrix loads");
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

const ALL: [RemoteDesktopCall; 29] = [
    RemoteDesktopCall::CreateSession,
    RemoteDesktopCall::SupportedDeviceTypes,
    RemoteDesktopCall::Version,
    RemoteDesktopCall::SessionId,
    RemoteDesktopCall::Start,
    RemoteDesktopCall::Stop,
    RemoteDesktopCall::NotifyKeyboardKeycode,
    RemoteDesktopCall::NotifyKeyboardKeysym,
    RemoteDesktopCall::NotifyPointerButton,
    RemoteDesktopCall::NotifyPointerAxis,
    RemoteDesktopCall::NotifyPointerAxisDiscrete,
    RemoteDesktopCall::NotifyPointerMotionRelative,
    RemoteDesktopCall::NotifyPointerMotionAbsolute,
    RemoteDesktopCall::NotifyTouchDown,
    RemoteDesktopCall::NotifyTouchMotion,
    RemoteDesktopCall::NotifyTouchUp,
    RemoteDesktopCall::EnableClipboard,
    RemoteDesktopCall::DisableClipboard,
    RemoteDesktopCall::SetSelection,
    RemoteDesktopCall::SelectionWrite,
    RemoteDesktopCall::SelectionWriteDone,
    RemoteDesktopCall::SelectionRead,
    RemoteDesktopCall::CapsLockState,
    RemoteDesktopCall::NumLockState,
    RemoteDesktopCall::ConnectToEIS,
    RemoteDesktopCall::KeymapCapabilities,
    RemoteDesktopCall::SetKeymap,
    RemoteDesktopCall::SetKeymapLayoutIndex,
    RemoteDesktopCall::CurrentKeymap,
];
