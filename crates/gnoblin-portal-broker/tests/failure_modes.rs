//! Failure-mode tests for the D19 negotiation, queue and reply-validation rules.

use gnoblin_portal_broker::{
    CancelReason, Deviation, Effect, Event, InterfaceContract, Negotiation, PortalError, Provider, Queue, Refusal,
    Request, Stage, Verdict, validate_reply,
};
use gnoblin_portal_ui::{Grant, Interface};
use gnoblin_types::{RequestId, Tick};

const TIMEOUT_MS: u64 = 5_000;
const UI_TIMEOUT_MS: u64 = 60_000;
const MINUTE_MS: u64 = 60_000;

fn tick(raw: u64) -> Tick {
    Tick::new(raw)
}

fn request(raw: u64) -> RequestId {
    RequestId::new(raw)
}

fn remote(interface: Interface, now: Tick) -> Negotiation {
    Negotiation::start(request(1), interface, Provider::Remote, now, TIMEOUT_MS, UI_TIMEOUT_MS).0
}

fn finished(effects: &[Effect]) -> Option<&Verdict> {
    effects.iter().find_map(|effect| match effect {
        Effect::Finish(verdict) => Some(verdict),
        _ => None,
    })
}

#[test]
fn provider_timeout_falls_back_only_for_idempotent_interfaces() {
    // An idempotent interface with a UI step falls back; a non-idempotent one
    // cancels with Timeout, because a second call could repeat the side effect.
    let interface = Interface::FileChooser;
    let mut negotiation = remote(interface, tick(0));
    let effects = negotiation.handle(Event::Tick, tick(TIMEOUT_MS));
    if interface.idempotent() {
        assert!(effects.contains(&Effect::CancelProvider));
        assert!(effects.iter().any(|effect| matches!(effect, Effect::OpenUi { .. })));
        assert_eq!(negotiation.stage(), Stage::AwaitingUi);
    } else {
        assert_eq!(finished(&effects), Some(&Verdict::Cancelled(CancelReason::Timeout)));
    }
}

#[test]
fn provider_timeout_never_falls_back_when_not_idempotent() {
    let interface = Interface::ScreenCast;
    if interface.idempotent() {
        return;
    }
    let mut negotiation = remote(interface, tick(0));
    let effects = negotiation.handle(Event::Tick, tick(TIMEOUT_MS));
    assert_eq!(finished(&effects), Some(&Verdict::Cancelled(CancelReason::Timeout)));
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::OpenUi { .. })));
}

#[test]
fn absent_provider_falls_back_once_and_never_twice() {
    let mut negotiation = remote(Interface::FileChooser, tick(0));
    let first = negotiation.handle(Event::ProviderUnavailable, tick(1));
    assert!(first.iter().any(|effect| matches!(effect, Effect::OpenUi { .. })));
    // A late provider verdict after fallback is ignored.
    let late = negotiation.handle(Event::ProviderVerdict(Verdict::Declined), tick(2));
    assert!(late.is_empty());
    assert_eq!(negotiation.stage(), Stage::AwaitingUi);
}

#[test]
fn declined_provider_falls_back_and_declined_ui_is_a_cancel() {
    let mut negotiation = remote(Interface::FileChooser, tick(0));
    let effects = negotiation.handle(Event::ProviderVerdict(Verdict::Declined), tick(1));
    assert!(effects.iter().any(|effect| matches!(effect, Effect::OpenUi { .. })));
    let effects = negotiation.handle(Event::UiVerdict(Verdict::Declined), tick(2));
    assert_eq!(finished(&effects), Some(&Verdict::Cancelled(CancelReason::UiFailed)));
}

#[test]
fn requester_gone_is_terminal_and_stops_the_provider() {
    let mut negotiation = remote(Interface::FileChooser, tick(0));
    let effects = negotiation.handle(Event::RequesterGone, tick(1));
    assert!(effects.contains(&Effect::CancelProvider));
    assert_eq!(
        finished(&effects),
        Some(&Verdict::Cancelled(CancelReason::RequesterGone))
    );
    assert_eq!(negotiation.stage(), Stage::Finished);
}

#[test]
fn first_terminal_outcome_wins() {
    let mut negotiation = remote(Interface::FileChooser, tick(0));
    let first = negotiation.handle(Event::ProviderVerdict(Verdict::Handled(Grant::Allowed)), tick(1));
    assert_eq!(finished(&first), Some(&Verdict::Handled(Grant::Allowed)));
    // Requester-gone or a timeout arriving after the terminal outcome changes nothing.
    assert!(negotiation.handle(Event::RequesterGone, tick(2)).is_empty());
    assert!(negotiation.handle(Event::Tick, tick(TIMEOUT_MS * 10)).is_empty());
    assert!(
        negotiation
            .handle(Event::ProviderVerdict(Verdict::Error(PortalError::Busy)), tick(3))
            .is_empty()
    );
}

#[test]
fn interface_without_ui_step_ends_not_supported_on_fallback() {
    let interface = Interface::ALL.into_iter().find(|interface| !interface.has_ui_step());
    if let Some(interface) = interface {
        let mut negotiation = remote(interface, tick(0));
        let effects = negotiation.handle(Event::ProviderUnavailable, tick(1));
        assert_eq!(finished(&effects), Some(&Verdict::Error(PortalError::NotSupported)));
    }
}

#[test]
fn queue_caps_total_and_per_app_and_rate_window() {
    let mut queue = Queue::new();
    // Per-app cap is 2: the third request from one app is busy.
    assert!(queue.admit(request(1), "app.a", tick(0), false).is_ok());
    assert!(queue.admit(request(2), "app.a", tick(0), false).is_ok());
    assert_eq!(queue.admit(request(3), "app.a", tick(0), false), Err(Refusal::Busy));
    // Total cap is 8: fill with other apps, the ninth is busy.
    for (index, app) in ["b", "c", "d", "e", "f", "g"].into_iter().enumerate() {
        let raw = 10 + index as u64;
        assert!(queue.admit(request(raw), app, tick(0), false).is_ok());
    }
    assert_eq!(queue.len(), 8);
    assert_eq!(queue.admit(request(99), "h", tick(0), false), Err(Refusal::Busy));
    // Completing one frees a slot.
    assert!(queue.complete(request(1)));
    assert!(!queue.complete(request(1)));
    assert!(queue.admit(request(99), "h", tick(0), false).is_ok());
}

#[test]
fn rate_window_is_five_per_minute_per_app() {
    let mut queue = Queue::new();
    for index in 0..5 {
        let raw = index + 1;
        assert!(
            queue
                .admit(request(raw), "rate.app", tick(index * 1_000), false)
                .is_ok()
        );
        assert!(queue.complete(request(raw)));
    }
    assert_eq!(
        queue.admit(request(6), "rate.app", tick(5_000), false),
        Err(Refusal::Busy)
    );
    // Once the first admission leaves the window, the app may request again.
    assert!(queue.admit(request(7), "rate.app", tick(MINUTE_MS), false).is_ok());
}

#[test]
fn refused_requests_cost_no_rate_budget() {
    let mut queue = Queue::new();
    assert_eq!(
        queue.admit(request(1), "locked.app", tick(0), true),
        Err(Refusal::Locked)
    );
    for index in 0..5 {
        let raw = index + 2;
        assert!(queue.admit(request(raw), "locked.app", tick(index), false).is_ok());
        assert!(queue.complete(request(raw)));
    }
}

#[test]
fn lock_drops_every_queued_request() {
    let mut queue = Queue::new();
    assert!(queue.admit(request(1), "a", tick(0), false).is_ok());
    assert!(queue.admit(request(2), "b", tick(0), false).is_ok());
    let dropped = queue.lock();
    assert_eq!(dropped, vec![request(1), request(2)]);
    assert!(queue.is_empty());
    assert_eq!(queue.admit(request(3), "a", tick(0), true), Err(Refusal::Locked));
}

#[test]
fn session_lock_closes_a_running_negotiation_with_locked() {
    let mut negotiation = remote(Interface::FileChooser, tick(0));
    let effects = negotiation.handle(Event::Locked, tick(1));
    assert_eq!(finished(&effects), Some(&Verdict::Error(PortalError::Locked)));
}

#[test]
fn reply_cannot_grant_more_than_requested() {
    let single = Request::FileChooser {
        multiple: false,
        filter_count: 1,
    };
    let two = Grant::Files {
        paths: vec!["/a".to_owned(), "/b".to_owned()],
        filter: 0,
    };
    assert_eq!(validate_reply(&single, &two), Err(Deviation::TooManyPaths));

    let relative = Grant::Files {
        paths: vec!["a".to_owned()],
        filter: 0,
    };
    assert_eq!(validate_reply(&single, &relative), Err(Deviation::BadPath));

    let print = Request::Print {
        max_copies: 2,
        first_page: 1,
        last_page: 5,
    };
    let too_many = Grant::Print {
        copies: 3,
        first_page: 1,
        last_page: 5,
    };
    assert_eq!(validate_reply(&print, &too_many), Err(Deviation::CopiesOutOfRange));
    let outside = Grant::Print {
        copies: 1,
        first_page: 1,
        last_page: 9,
    };
    assert_eq!(validate_reply(&print, &outside), Err(Deviation::PagesOutOfRange));

    let cast = Request::ScreenCast {
        cursor_modes: 0b001,
        multiple: false,
    };
    let hidden_cursor = Grant::Capture {
        cursor_mode: 2,
        sources: 1,
    };
    assert_eq!(
        validate_reply(&cast, &hidden_cursor),
        Err(Deviation::CursorModeNotAllowed)
    );

    let apps = Request::AppChooser {
        offered: vec!["org.a.Editor.desktop".to_owned()],
    };
    let not_offered = Grant::App {
        desktop_file_id: "org.evil.desktop".to_owned(),
        remember: false,
    };
    assert_eq!(validate_reply(&apps, &not_offered), Err(Deviation::NotOffered));

    let devices = Request::RemoteDesktop { device_types: 0b01 };
    let extra = Grant::Devices { device_types: 0b11 };
    assert_eq!(validate_reply(&devices, &extra), Err(Deviation::DevicesNotRequested));

    assert_eq!(validate_reply(&Request::Consent, &Grant::Allowed), Ok(()));
    assert_eq!(validate_reply(&Request::Consent, &two), Err(Deviation::WrongKind));
}
