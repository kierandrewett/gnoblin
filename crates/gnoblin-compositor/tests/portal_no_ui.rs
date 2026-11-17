//! A session with no portal UI provider refuses every portal dialog `no-ui`, and grants nothing.
//!
//! The dialogs are std's `UiProvider` (`portal_ui.rs` covers them end to end). A session that registered none has nothing to ask
//! the user with, so a consent request must fail closed: the requester is told `no-ui` and no dialog is ever shown or answered.

mod support;

use std::time::Duration;

use gnoblin_client::wire::{PortalRequest, VerifiedCaller, portal_ui};
use gnoblin_model::Fields;
use support::Session;

/// How long a test waits for a reply that must arrive.
const REPLY_WAIT: Duration = Duration::from_secs(5);

fn consent(interface: &str) -> PortalRequest {
    PortalRequest {
        interface: interface.to_owned(),
        caller: VerifiedCaller::new(0, "org.example.Requester"),
        prompt: "Share the screen?".to_owned(),
        timeout_ms: 30_000,
        options: Fields::default(),
        parent_window: String::new(),
    }
}

#[test]
fn a_consent_request_is_refused_no_ui_by_a_session_with_no_provider() {
    let (session, _roles) = Session::start_privileged_configured(&[], support::core_only);
    let mut owner = session.wire();
    owner.set_read_timeout(REPLY_WAIT);

    for (request_id, interface) in [(1, "Screenshot"), (2, "ScreenCast"), (3, "RemoteDesktop")] {
        owner.send(&portal_ui(request_id, &consent(interface)));
        let reply = owner.receive().expect("the refusal arrives before the deadline");
        assert_eq!(reply.request_id, request_id, "the reply answers the request sent");
        assert_eq!(
            reply.error_code(),
            Some("no-ui"),
            "{interface} needs a dialog nobody can draw"
        );
    }
    drop(session);
}
