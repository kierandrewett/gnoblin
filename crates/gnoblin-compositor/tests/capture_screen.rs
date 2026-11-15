//! `capture.screen` through a real session and a real lock request.
//!
//! `gnoblin-capture`'s `still::choose_output` has its own unit tests, and `gnoblin-client` tests the refusal against a
//! scripted server. Neither proves the handler asks the live lock: the snapshot it reads and the compositor's own lock
//! state have to agree at the moment a locker asks, and a refusal has to be the named `locked` rather than a missing
//! picture. This is that end-to-end check, one of the capture-embargo failure modes in `gnoblin-capture`'s docs.

mod support;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, wire};
use support::{Locker, Session};

/// The first eight bytes of every PNG file.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

#[track_caller]
fn connect(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        gnoblin_compositor::version(),
    )
    .expect("the session accepts this build")
}

#[track_caller]
fn refusal_code<T: std::fmt::Debug>(outcome: Result<T, ClientError>) -> String {
    match outcome {
        Err(ClientError::Refused { code, .. }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A lock request that has not reached `locked` (the locker never maps a surface, so the cover is never proven)
/// is the `Locking` state, which the lock treats as covered: the picture of the desktop it would take is refused,
/// in both forms, by name.
#[test]
fn a_capture_while_the_lock_is_being_established_is_refused_by_name() {
    let session = Session::start();
    let mut client = connect(&session);
    let picture = client.capture_screen().expect("an unlocked session is captured");
    assert_eq!(
        picture.get(..8),
        Some(PNG_SIGNATURE.as_slice()),
        "the reply of an unlocked session is a PNG"
    );

    let mut locker = Locker::connect(session.wayland());
    locker.lock();
    assert_eq!(
        refusal_code(client.capture_screen()),
        wire::code::CAPTURE_LOCKED,
        "the frame the compositor composed is refused once the lock is requested"
    );
    assert_eq!(
        refusal_code(client.capture_screen_display()),
        wire::code::CAPTURE_LOCKED,
        "so is the frame as the display shows it"
    );
}
