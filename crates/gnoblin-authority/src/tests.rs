//! One test per failure mode in the crate documentation.
//!
//! Each drives [`Authority::apply`] and its queries only, with ticks the test
//! supplies, so nothing here depends on a clock or on a process table.

use crate::{
    Answer, AppId, Authority, Capability, Claimant, ClientId, Consent, ContextRequest, Decision, Denial, Effect, Event,
    Gesture, GestureId, LaunchOrigin, Limits, RequestId, RequesterId, SessionLeader, SurfaceId, Tick, TokenId,
};

const LEADER: SessionLeader = SessionLeader::new(100);
const OTHER_LEADER: SessionLeader = SessionLeader::new(200);
const CLIENT: ClientId = ClientId::new(1);
const OTHER_CLIENT: ClientId = ClientId::new(2);
const SHELL: ClientId = ClientId::new(9);
const SURFACE: SurfaceId = SurfaceId::new(1);
const OTHER_SURFACE: SurfaceId = SurfaceId::new(2);
const TOKEN: TokenId = TokenId::new(0x5eed);
const REQUESTER: RequesterId = RequesterId::new(7);
const REQUEST: RequestId = RequestId::new(1);
const GESTURE: GestureId = GestureId::new(3);
const NOW: Tick = Tick::new(1_000);
const LIFETIME: u64 = 100;

/// A session with one launch recorded for [`LEADER`].
fn launched() -> Authority {
    let mut authority = Authority::new();
    assert_eq!(
        authority.apply(
            NOW,
            Event::Launch {
                origin: LaunchOrigin {
                    leader: LEADER,
                    app_id: None,
                },
                token: TOKEN,
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::IssueToken {
            token: TOKEN,
            leader: LEADER,
            app_id: None,
            expires_at: NOW.after(LIFETIME),
        }]
    );
    authority
}

/// A claim for focus at [`SURFACE`], optionally with an app id.
fn claim(token: Option<TokenId>, app_id: Option<&str>) -> Claimant {
    Claimant {
        client: CLIENT,
        leader: LEADER,
        surface: SURFACE,
        app_id: app_id.map(AppId::new),
        token,
    }
}

fn activation(reason: Denial) -> Effect {
    Effect::RefuseActivation {
        client: CLIENT,
        surface: SURFACE,
        reason,
    }
}

fn gesture(lifetime: u64) -> Event {
    Event::GestureBegan {
        gesture: Gesture {
            id: GESTURE,
            client: CLIENT,
            surface: SURFACE,
            kind: crate::GestureKind::Pointer { button: 1 },
        },
        lifetime,
    }
}

fn move_request(client: ClientId, surface: SurfaceId) -> Event {
    Event::ContextRequest {
        gesture: GESTURE,
        client,
        surface,
        request: ContextRequest::Move,
    }
}

/// A session with a `portal.ui` provider and one capture prompt waiting.
fn prompting() -> Authority {
    let mut authority = Authority::new();
    authority.apply(NOW, Event::PortalUiClaimed { client: SHELL });
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentRequested {
                request: REQUEST,
                requester: REQUESTER,
                capability: Capability::Capture,
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::Prompt {
            request: REQUEST,
            capability: Capability::Capture,
            requester: REQUESTER,
            provider: SHELL,
        }]
    );
    authority
}

fn allow(authority: &mut Authority) {
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentAnswered {
                request: REQUEST,
                from: SHELL,
                answer: Answer::Allow,
            }
        ),
        vec![Effect::Answer {
            request: REQUEST,
            decision: Decision::Allowed,
        }]
    );
}

#[test]
fn a_token_is_accepted_once() {
    let mut authority = launched();
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![Effect::Activate {
            client: CLIENT,
            surface: SURFACE,
        }]
    );
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![activation(Denial::ReusedToken)]
    );
}

#[test]
fn a_context_does_not_outlive_its_gesture() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(LIFETIME));
    assert_eq!(
        authority.apply(NOW, Event::GestureEnded { gesture: GESTURE }),
        vec![Effect::EndContext { gesture: GESTURE }]
    );
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::StaleContext,
        }]
    );
}

#[test]
fn a_child_with_another_leader_inherits_nothing() {
    let mut authority = launched();
    // A child that left the launch's session with `setsid` has its own leader.
    let child = Claimant {
        leader: OTHER_LEADER,
        ..claim(Some(TOKEN), None)
    };
    assert_eq!(
        authority.apply(NOW, Event::Activate { claim: child }),
        vec![activation(Denial::ForeignLaunch)]
    );
    // The token was not spent by the attempt, so the launch can still use it.
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![Effect::Activate {
            client: CLIENT,
            surface: SURFACE,
        }]
    );
}

#[test]
fn an_app_id_never_grants_focus() {
    let mut authority = launched();
    // Pretending to be the launcher's application, with no token.
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(None, Some("org.example.trusted")),
            }
        ),
        vec![activation(Denial::NoAuthority)]
    );
    // The token is still unredeemed.
    assert!(authority.token_expiry(TOKEN).is_some());
}

#[test]
fn a_session_leader_alone_never_grants_focus() {
    let mut authority = launched();
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(None, None)
            }
        ),
        vec![activation(Denial::NoAuthority)]
    );
}

#[test]
fn an_expired_token_is_refused() {
    let mut authority = launched();
    assert_eq!(
        authority.apply(
            NOW.after(LIFETIME),
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![activation(Denial::ExpiredToken)]
    );
    // Expiry is final: the token is gone, so a later attempt cannot revive it.
    assert_eq!(authority.token_expiry(TOKEN), None);
}

#[test]
fn a_token_exists_only_in_the_session_that_minted_it() {
    let mut authority = launched();
    let mut other = Authority::new();
    other.apply(
        NOW,
        Event::Launch {
            origin: LaunchOrigin {
                leader: OTHER_LEADER,
                app_id: None,
            },
            token: TokenId::new(0xfeed),
            lifetime: LIFETIME,
        },
    );
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![Effect::Activate {
            client: CLIENT,
            surface: SURFACE,
        }]
    );
    assert_eq!(
        other.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![activation(Denial::UnknownToken)]
    );
}

#[test]
fn a_covered_session_refuses_activation() {
    let mut authority = launched();
    assert_eq!(
        authority.apply(NOW, Event::LockChanged { active: true }),
        vec![Effect::Capture { allowed: false }]
    );
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![activation(Denial::SessionLocked)]
    );
    authority.apply(NOW, Event::LockChanged { active: false });
    // Refusing while covered does not burn the token.
    assert_eq!(
        authority.apply(
            NOW,
            Event::Activate {
                claim: claim(Some(TOKEN), None)
            }
        ),
        vec![Effect::Activate {
            client: CLIENT,
            surface: SURFACE,
        }]
    );
}

#[test]
fn a_trusted_gesture_authorises_focus() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(LIFETIME));
    // Focus needs no token when a gesture the user just made authorises it,
    // which is the other half of the plan's "a valid token or a trusted
    // context" rule.
    assert_eq!(
        authority.apply(
            NOW,
            Event::ContextRequest {
                gesture: GESTURE,
                client: CLIENT,
                surface: SURFACE,
                request: ContextRequest::Focus,
            }
        ),
        vec![Effect::Authorise {
            gesture: GESTURE,
            client: CLIENT,
            surface: SURFACE,
            request: ContextRequest::Focus,
        }]
    );
}

#[test]
fn a_context_is_single_use() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(LIFETIME));
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::Authorise {
            gesture: GESTURE,
            client: CLIENT,
            surface: SURFACE,
            request: ContextRequest::Move,
        }]
    );
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::ConsumedContext,
        }]
    );
}

#[test]
fn an_expired_context_is_refused() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(10));
    assert_eq!(
        authority.apply(NOW.after(10), move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::ExpiredContext,
        }]
    );
}

#[test]
fn a_context_is_bound_to_its_target() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(LIFETIME));
    assert_eq!(
        authority.apply(NOW, move_request(OTHER_CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: OTHER_CLIENT,
            request: ContextRequest::Move,
            reason: Denial::WrongTarget,
        }]
    );
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, OTHER_SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::WrongTarget,
        }]
    );
    // A wrong target does not consume the context the gesture really owns.
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::Authorise {
            gesture: GESTURE,
            client: CLIENT,
            surface: SURFACE,
            request: ContextRequest::Move,
        }]
    );
}

#[test]
fn locking_ends_live_contexts() {
    let mut authority = Authority::new();
    authority.apply(NOW, gesture(LIFETIME));
    assert_eq!(
        authority.apply(NOW, Event::LockChanged { active: true }),
        vec![
            Effect::Capture { allowed: false },
            Effect::EndContext { gesture: GESTURE },
        ]
    );
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::SessionLocked,
        }]
    );
    // Unlocking restores nothing: the context is gone for good.
    authority.apply(NOW, Event::LockChanged { active: false });
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::StaleContext,
        }]
    );
}

#[test]
fn a_gesture_during_lock_authorises_nothing() {
    let mut authority = Authority::new();
    authority.apply(NOW, Event::LockChanged { active: true });
    assert_eq!(authority.apply(NOW, gesture(LIFETIME)), Vec::new());
    authority.apply(NOW, Event::LockChanged { active: false });
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::StaleContext,
        }]
    );
}

#[test]
fn a_context_request_without_a_gesture_is_refused() {
    let mut authority = Authority::new();
    assert_eq!(
        authority.apply(NOW, move_request(CLIENT, SURFACE)),
        vec![Effect::RefuseContext {
            gesture: GESTURE,
            client: CLIENT,
            request: ContextRequest::Move,
            reason: Denial::StaleContext,
        }]
    );
}

#[test]
fn capture_fails_closed_without_a_portal_ui_provider() {
    let mut authority = Authority::new();
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentRequested {
                request: REQUEST,
                requester: REQUESTER,
                capability: Capability::Capture,
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::Answer {
            request: REQUEST,
            decision: Decision::Denied(Denial::NoPortalUi),
        }]
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoGrant)
    );
}

#[test]
fn a_grant_stops_while_the_session_is_locked() {
    let mut authority = prompting();
    allow(&mut authority);
    assert_eq!(authority.gate(NOW, REQUESTER, Capability::Capture), Decision::Allowed);
    assert_eq!(
        authority.apply(NOW, Event::LockChanged { active: true }),
        vec![Effect::Capture { allowed: false }]
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::Locked)
    );
    // The consent still stands, so the stream resumes when the cover goes.
    authority.apply(NOW, Event::LockChanged { active: false });
    assert_eq!(authority.gate(NOW, REQUESTER, Capability::Capture), Decision::Allowed);
}

#[test]
fn consent_pending_grants_nothing() {
    let authority = prompting();
    assert_eq!(
        authority.consent(REQUESTER, Capability::Capture),
        Some(Consent::Pending)
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoConsent)
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::InputInjection),
        Decision::Denied(Denial::NoGrant)
    );
}

#[test]
fn a_refused_consent_stays_refused() {
    let mut authority = prompting();
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentAnswered {
                request: REQUEST,
                from: SHELL,
                answer: Answer::Deny,
            }
        ),
        vec![Effect::Answer {
            request: REQUEST,
            decision: Decision::Denied(Denial::Refused),
        }]
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::Refused)
    );
    // Asking again while the refusal stands does not re-prompt the user.
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentRequested {
                request: RequestId::new(2),
                requester: REQUESTER,
                capability: Capability::Capture,
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::Answer {
            request: RequestId::new(2),
            decision: Decision::Denied(Denial::Refused),
        }]
    );
    // It does end with its lifetime.
    assert_eq!(
        authority.gate(NOW.after(LIFETIME), REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoGrant)
    );
}

#[test]
fn only_the_provider_answers() {
    let mut authority = prompting();
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentAnswered {
                request: REQUEST,
                from: CLIENT,
                answer: Answer::Allow,
            }
        ),
        vec![Effect::Ignored {
            request: REQUEST,
            from: CLIENT,
        }]
    );
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoConsent)
    );
    // The real provider can still answer.
    allow(&mut authority);
    assert_eq!(authority.gate(NOW, REQUESTER, Capability::Capture), Decision::Allowed);
}

#[test]
fn a_grant_ends_with_its_requester() {
    let mut authority = prompting();
    allow(&mut authority);
    assert_eq!(
        authority.apply(NOW, Event::RequesterGone { requester: REQUESTER }),
        vec![Effect::EndGrants { requester: REQUESTER }]
    );
    assert_eq!(authority.consent(REQUESTER, Capability::Capture), None);
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoGrant)
    );
}

#[test]
fn a_token_value_is_issued_once() {
    let mut authority = launched();
    assert_eq!(
        authority.apply(
            NOW,
            Event::Launch {
                origin: LaunchOrigin {
                    leader: OTHER_LEADER,
                    app_id: None,
                },
                token: TOKEN,
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::RefuseLaunch {
            token: TOKEN,
            reason: Denial::DuplicateToken,
        }]
    );
}

#[test]
fn token_capacity_fails_closed() {
    let mut authority = Authority::with_limits(Limits {
        tokens: 1,
        ..Limits::default()
    });
    authority.apply(
        NOW,
        Event::Launch {
            origin: LaunchOrigin {
                leader: LEADER,
                app_id: None,
            },
            token: TOKEN,
            lifetime: LIFETIME,
        },
    );
    assert_eq!(
        authority.apply(
            NOW,
            Event::Launch {
                origin: LaunchOrigin {
                    leader: OTHER_LEADER,
                    app_id: None,
                },
                token: TokenId::new(0xfeed),
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::RefuseLaunch {
            token: TokenId::new(0xfeed),
            reason: Denial::AtCapacity,
        }]
    );
    // An expired token frees its slot rather than accumulating forever.
    assert_eq!(
        authority.apply(
            NOW.after(LIFETIME),
            Event::Launch {
                origin: LaunchOrigin {
                    leader: OTHER_LEADER,
                    app_id: None,
                },
                token: TokenId::new(0xfeed),
                lifetime: LIFETIME,
            }
        ),
        vec![Effect::IssueToken {
            token: TokenId::new(0xfeed),
            leader: OTHER_LEADER,
            app_id: None,
            expires_at: NOW.after(LIFETIME * 2),
        }]
    );
}

#[test]
fn an_unanswered_prompt_expires_closed() {
    let mut authority = prompting();
    assert_eq!(
        authority.apply(
            NOW.after(LIFETIME),
            Event::ConsentAnswered {
                request: REQUEST,
                from: SHELL,
                answer: Answer::Allow,
            }
        ),
        vec![Effect::Answer {
            request: REQUEST,
            decision: Decision::Denied(Denial::ConsentExpired),
        }]
    );
    assert_eq!(
        authority.gate(NOW.after(LIFETIME), REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoGrant)
    );
}

#[test]
fn releasing_portal_ui_refuses_waiting_prompts() {
    let mut authority = prompting();
    assert_eq!(
        authority.apply(NOW, Event::PortalUiReleased { client: SHELL }),
        vec![
            Effect::PortalUi { provider: None },
            Effect::Answer {
                request: REQUEST,
                decision: Decision::Denied(Denial::NoPortalUi),
            },
        ]
    );
    assert_eq!(authority.provider(), None);
    assert_eq!(
        authority.gate(NOW, REQUESTER, Capability::Capture),
        Decision::Denied(Denial::NoGrant)
    );
    // A late answer to the withdrawn prompt is ignored, not granted.
    assert_eq!(
        authority.apply(
            NOW,
            Event::ConsentAnswered {
                request: REQUEST,
                from: SHELL,
                answer: Answer::Allow,
            }
        ),
        vec![Effect::Ignored {
            request: REQUEST,
            from: SHELL,
        }]
    );
}
