//! The power verbs reach the session's [`PowerBackend`] once each, and a refusal or an absent
//! backend completes the operation with its named code.
//!
//! The backend is a fake the test owns: it records each verb it is asked for and answers with
//! the outcome the test chose. Nothing else is mocked; the session and the socket are real.

mod support;

use std::sync::{Arc, Mutex};

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_model::{
    Fields, Filter, ObjectId, OperationOutcome, PowerBackend, PowerError, PowerErrorKind, PowerVerb, TypeKey,
};
use gnoblin_types::ErrorCode;

use support::Session;

/// The verbs a backend was asked for, in order.
type Record = Arc<Mutex<Vec<PowerVerb>>>;

/// What the fake answers each verb with.
#[derive(Clone, Copy, Debug)]
enum Answer {
    Accept,
    Refuse(PowerErrorKind),
}

/// A backend that records every verb it is asked for, and answers with [`Answer`].
#[derive(Debug)]
struct Fake {
    asked: Record,
    answer: Answer,
}

impl PowerBackend for Fake {
    fn perform(&mut self, verb: PowerVerb) -> Result<(), PowerError> {
        self.asked.lock().expect("the record").push(verb);
        match self.answer {
            Answer::Accept => Ok(()),
            Answer::Refuse(kind) => Err(PowerError::new(verb, kind, "the fake refused")),
        }
    }
}

/// Starts a session with `power` as its backend.
fn started(power: Option<Fake>) -> Session {
    let (session, _) = Session::start_privileged_configured(&[], |config| {
        config.power = power.map(|fake| Box::new(fake) as Box<dyn PowerBackend>);
    });
    session
}

/// An IPC client at the served version.
fn ipc(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        gnoblin_compositor::version(),
    )
    .expect("the session accepts this build")
}

/// The `session` object's identity, read from a fresh snapshot.
fn session_object(client: &mut Client<UnixTransport>) -> ObjectId {
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("session"))
        .map(|view| view.id())
        .expect("the session object")
}

/// Calls `method` on the session object and returns how the operation finished.
fn call(client: &mut Client<UnixTransport>, object: ObjectId, method: &str) -> OperationOutcome {
    let operation = client.call(object, method, &Fields::new()).expect("the call is served");
    client.await_operation(operation).expect("the completion")
}

const ALL: [(&str, PowerVerb); 4] = [
    ("poweroff", PowerVerb::PowerOff),
    ("reboot", PowerVerb::Reboot),
    ("suspend", PowerVerb::Suspend),
    ("hibernate", PowerVerb::Hibernate),
];

/// Each verb reaches the backend exactly once, as the verb its method names, and the
/// operation completes `done`.
#[test]
fn each_verb_reaches_the_backend_once_with_its_verb() {
    let record: Record = Arc::new(Mutex::new(Vec::new()));
    let session = started(Some(Fake {
        asked: Arc::clone(&record),
        answer: Answer::Accept,
    }));
    let mut client = ipc(&session);
    let object = session_object(&mut client);

    for (method, verb) in ALL {
        let outcome = call(&mut client, object, method);
        assert!(outcome.is_done(), "{method} was accepted: {outcome:?}");
        assert_eq!(
            record.lock().expect("the record").last().copied(),
            Some(verb),
            "{method} reached the backend as {verb:?}"
        );
    }
    assert_eq!(
        record.lock().expect("the record").len(),
        ALL.len(),
        "one backend call per verb"
    );
    session.stop().expect("the session stops cleanly");
}

/// A backend refusal reaches the caller under the code its error carries.
#[test]
fn a_refused_verb_completes_with_the_code_its_error_carries() {
    let cases = [
        (PowerErrorKind::Denied, "denied"),
        (PowerErrorKind::Unsupported, "unsupported"),
        (PowerErrorKind::Busy, "busy"),
        (PowerErrorKind::Unavailable, "unavailable"),
        (PowerErrorKind::TimedOut, "timed-out"),
        (PowerErrorKind::Internal, "internal"),
    ];
    for (kind, code) in cases {
        let record: Record = Arc::new(Mutex::new(Vec::new()));
        let session = started(Some(Fake {
            asked: Arc::clone(&record),
            answer: Answer::Refuse(kind),
        }));
        let mut client = ipc(&session);
        let object = session_object(&mut client);

        let outcome = call(&mut client, object, "reboot");
        assert_eq!(
            outcome,
            OperationOutcome::Failed(ErrorCode::from_static(code)),
            "{kind:?} is refused {code}"
        );
        assert_eq!(record.lock().expect("the record").as_slice(), &[PowerVerb::Reboot]);
        session.stop().expect("the session stops cleanly");
    }
}

/// A session with no backend completes every verb `unavailable`, and asks nothing.
#[test]
fn a_session_without_a_backend_refuses_every_verb_unavailable() {
    let session = started(None);
    let mut client = ipc(&session);
    let object = session_object(&mut client);

    for (method, _) in ALL {
        let outcome = call(&mut client, object, method);
        assert_eq!(
            outcome,
            OperationOutcome::Failed(ErrorCode::from_static("unavailable")),
            "{method} with no backend"
        );
    }
    session.stop().expect("the session stops cleanly");
}
