//! One test per documented failure mode, driven through the in-process loopback.
//!
//! Everything here goes through the real wire path: the client encodes an envelope,
//! the loopback hands it to a server that decodes it with the same codec a socket
//! will use, and the client decodes what comes back. Only the byte transport is
//! replaced, so these tests keep their meaning when the socket lands.

use std::path::Path;

use gnoblin_client::{Client, ClientError, ConnectError, Event, Loopback, Server, code};
use gnoblin_model::{
    Fields, Filter, Graph, ObjectId, Observation, OperationOutcome, RequestCompletion, RequestOutcome, Tick, TypeKey,
    Value,
};
use gnoblin_proto::Version;
use gnoblin_schema::Catalog;

const VERSION: Version = Version::new(0, 2, 0);
const TITLE: &str = "title";

fn graph() -> Graph {
    let catalog = Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads");
    Graph::new(catalog)
}

fn window_properties(title: &str) -> Fields {
    let mut properties = Fields::new();
    properties.insert("id".to_owned(), Value::Uint(1));
    properties.insert(TITLE.to_owned(), Value::Text(title.to_owned()));
    properties.insert("app_id".to_owned(), Value::Text("org.example.App".to_owned()));
    properties.insert("focused".to_owned(), Value::Bool(false));
    properties.insert("x".to_owned(), Value::Int(0));
    properties.insert("y".to_owned(), Value::Int(0));
    properties.insert("width".to_owned(), Value::Uint(200));
    properties.insert("height".to_owned(), Value::Uint(150));
    properties.insert("icon".to_owned(), Value::Text(String::new()));
    properties.insert("tag".to_owned(), Value::Text(String::new()));
    properties.insert("modal".to_owned(), Value::Bool(false));
    properties
}

fn open_window(server: &mut Server, title: &str) -> ObjectId {
    server
        .create(TypeKey::gnoblin("window"), window_properties(title))
        .expect("the window type and its properties are in the schema")
}

/// One server and one connected client.
fn pair() -> (Server, Client<Loopback>) {
    let mut server = Server::new(graph(), VERSION);
    let client = server.connect(VERSION).expect("the versions match");
    (server, client)
}

#[test]
fn a_late_subscriber_gets_the_snapshot_and_no_replay() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "First");
    server
        .set(window, TITLE, Value::Text("Second".to_owned()))
        .expect("title is a declared string property");

    let snapshot = client
        .subscribe(Filter::All)
        .expect("the compositor answers a subscribe");
    let view = snapshot.object(window).expect("the window is live");
    assert_eq!(view.property(TITLE), Some(&Value::Text("Second".to_owned())));

    server
        .set(window, TITLE, Value::Text("Third".to_owned()))
        .expect("title is a declared string property");

    match client.next_event().expect("the diff after the snapshot") {
        Event::Model(Observation::Changed {
            object,
            property,
            value,
            revision,
        }) => {
            assert_eq!(object, window);
            assert_eq!(property, TITLE);
            assert_eq!(value, Value::Text("Third".to_owned()));
            assert!(
                revision > snapshot.revision(),
                "{revision} is not after the snapshot's {}",
                snapshot.revision()
            );
        }
        other => panic!("expected the one later diff, got {other:?}"),
    }

    assert!(
        matches!(client.next_event(), Err(ClientError::Disconnected)),
        "nothing is replayed and nothing is repeated"
    );
}

#[test]
fn an_operation_is_delivered_exactly_once() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Focusable");
    let snapshot = client.subscribe(Filter::All).expect("subscribe");

    let operation = client
        .call(window, "focus", &Fields::new())
        .expect("`focus` is declared with no parameters");
    server
        .complete(operation.id(), OperationOutcome::Done)
        .expect("the first completion is the only one");
    assert_eq!(
        server.complete(operation.id(), OperationOutcome::Done),
        Err(gnoblin_model::ModelError::UnknownOperation {
            operation: operation.id()
        })
    );

    assert_eq!(
        client.await_operation(operation).expect("the completion arrives"),
        OperationOutcome::Done
    );

    let mut completions = 0;
    while let Ok(event) = client.next_event() {
        match event {
            Event::Model(Observation::Finished { revision, .. }) => {
                assert!(revision > snapshot.revision());
                completions += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(completions, 1, "the client sees one completion, never two");
}

#[test]
fn an_answer_after_the_finish_is_refused() {
    let (mut server, mut client) = pair();
    let request = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(1_000),
    );

    match client.next_event().expect("the offer arrives") {
        Event::RequestOffered {
            request: offered,
            deadline,
            slot,
            title,
            message,
        } => {
            assert_eq!(offered, request);
            assert_eq!(deadline, Tick::new(1_000));
            // The proposition travels with the offer, because the schema's `request`
            // object carries only the identity and the deadline and a provider that
            // had to ask what a request is about could not draw anything.
            assert_eq!(slot, "auth.polkit");
            assert_eq!(title, "Authentication required");
            assert_eq!(message, "Authenticate to continue");
        }
        other => panic!("expected an offer, got {other:?}"),
    }

    client.claim(request).expect("the first claim wins");
    client.answer(request, &[0x42]).expect("the owner answers once");

    match client.decline(request) {
        Err(ClientError::Refused { code: refused, .. }) => assert_eq!(refused, code::REQUEST_FINISHED),
        other => panic!("expected a finished refusal, got {other:?}"),
    }
    assert_eq!(
        server.finished(request),
        Some(RequestOutcome::Answered(vec![0x42])),
        "the host that offered it reads the one outcome"
    );

    match client.next_event().expect("the finish reaches the client") {
        Event::RequestFinished {
            request: finished,
            completion,
        } => {
            assert_eq!(finished, request);
            assert_eq!(completion, RequestCompletion::Answered);
        }
        other => panic!("expected a finish, got {other:?}"),
    }
}

#[test]
fn an_empty_answer_reaches_the_host_as_an_answer_and_decline_as_ended() {
    let (mut server, mut client) = pair();

    let answered = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(1_000),
    );
    assert!(
        matches!(client.next_event().expect("the offer arrives"), Event::RequestOffered { request, .. } if request == answered)
    );
    client.claim(answered).expect("the first claim wins");
    client.answer(answered, &[]).expect("an empty answer is sent");
    assert_eq!(
        server.finished(answered),
        Some(RequestOutcome::Answered(Vec::new())),
        "zero bytes arrives as an answer, not as an ended request"
    );
    assert!(matches!(
        client.next_event().expect("the finish reaches the client"),
        Event::RequestFinished {
            request,
            completion: RequestCompletion::Answered
        } if request == answered
    ));

    let ended = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(1_000),
    );
    assert!(
        matches!(client.next_event().expect("the second offer arrives"), Event::RequestOffered { request, .. } if request == ended)
    );
    client.claim(ended).expect("the first claim wins");
    client.decline(ended).expect("declining is its own member");
    assert_eq!(
        server.finished(ended),
        Some(RequestOutcome::Ended),
        "declining carries no value"
    );
    assert!(matches!(
        client.next_event().expect("the finish reaches the client"),
        Event::RequestFinished {
            request,
            completion: RequestCompletion::Ended
        } if request == ended
    ));
}

#[test]
fn two_clients_cannot_claim_one_request() {
    let mut server = Server::new(graph(), VERSION);
    let mut first = server.connect(VERSION).expect("the versions match");
    let mut second = server.connect(VERSION).expect("the versions match");
    let request = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(1_000),
    );

    for client in [&mut first, &mut second] {
        assert!(
            matches!(
                client.next_event().expect("every client is offered it"),
                Event::RequestOffered { request: offered, .. } if offered == request
            ),
            "the offer reaches both connections"
        );
    }

    first.claim(request).expect("the first claim wins");
    match second.claim(request) {
        Err(ClientError::Refused { code: refused, .. }) => assert_eq!(refused, code::REQUEST_TAKEN),
        other => panic!("expected a taken refusal, got {other:?}"),
    }
    match second.decline(request) {
        Err(ClientError::Refused { code: refused, .. }) => {
            assert_eq!(refused, code::REQUEST_TAKEN, "the loser cannot answer either")
        }
        other => panic!("expected a taken refusal, got {other:?}"),
    }
}

#[test]
fn diffs_arrive_in_revision_order() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Ordered");
    let snapshot = client.subscribe(Filter::All).expect("subscribe");
    let second = open_window(&mut server, "Later");

    for step in 0..4 {
        server
            .set(window, TITLE, Value::Text(format!("Step {step}")))
            .expect("title is a declared string property");
        server
            .signal(window, "closed", Fields::new())
            .expect("`closed` is a declared signal");
    }
    server.close(second).expect("the second window is live");

    let mut previous = snapshot.revision();
    let mut seen = 0;
    while let Ok(event) = client.next_event() {
        let Event::Model(observation) = event else {
            panic!("only model events were expected, got {event:?}");
        };
        assert!(
            observation.revision() > previous,
            "{} does not follow {previous}",
            observation.revision()
        );
        previous = observation.revision();
        seen += 1;
    }
    // One open, four sets, four signals, one close.
    assert_eq!(seen, 10, "every diff arrives exactly once");
}

#[test]
fn a_subscription_outliving_its_object_ends_at_closed() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Doomed");
    let snapshot = client.subscribe(Filter::Object(window)).expect("subscribe");
    assert_eq!(snapshot.objects().len(), 1);

    server.close(window).expect("the window is live");
    match client.next_event().expect("the end arrives") {
        Event::Model(Observation::Closed { object, revision }) => {
            assert_eq!(object, window);
            assert!(revision > snapshot.revision());
        }
        other => panic!("expected the object's end, got {other:?}"),
    }

    let fresh = open_window(&mut server, "Fresh");
    assert_ne!(fresh, window, "an identity is never reused");
    assert!(
        matches!(client.next_event(), Err(ClientError::Disconnected)),
        "the one-object filter lets nothing else through, and the closed object never closes twice"
    );
}

#[test]
fn a_version_mismatch_is_refused_at_connect() {
    let mut server = Server::new(graph(), VERSION);
    match server.connect(Version::new(0, 1, 0)) {
        Err(ConnectError::Refused(mismatch)) => {
            assert_eq!(mismatch.server, VERSION);
            assert_eq!(mismatch.client, Version::new(0, 1, 0));
        }
        other => panic!("expected a refusal naming both versions, got {other:?}"),
    }
}

#[test]
fn an_undeclared_method_is_refused_with_the_schema_code() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Declared");
    match client.call(window, "unfocus", &Fields::new()) {
        Err(ClientError::Refused {
            member, code: refused, ..
        }) => {
            assert_eq!(member, "object.call");
            assert_eq!(refused, code::UNKNOWN_MEMBER);
        }
        other => panic!("expected an unknown-member refusal, got {other:?}"),
    }
}

#[test]
fn a_dead_object_is_refused_with_its_own_code() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Doomed");
    server.close(window).expect("the window is live");

    match client.call(window, "focus", &Fields::new()) {
        Err(ClientError::Refused { code: refused, .. }) => {
            assert_eq!(refused, code::OBJECT_UNKNOWN)
        }
        other => panic!("expected an object-unknown refusal, got {other:?}"),
    }
}

#[test]
fn an_unanswered_request_times_out_and_the_client_learns_it() {
    let (mut server, mut client) = pair();
    let request = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(10),
    );
    assert!(matches!(
        client.next_event().expect("the offer arrives"),
        Event::RequestOffered { request: offered, .. } if offered == request
    ));

    assert_eq!(server.tick(Tick::new(9)), 0, "not yet due");
    assert_eq!(server.tick(Tick::new(10)), 1, "the deadline is inclusive");
    assert_eq!(server.tick(Tick::new(11)), 0, "it times out once");

    match client.next_event().expect("the finish arrives") {
        Event::RequestFinished {
            request: finished,
            completion,
        } => {
            assert_eq!(finished, request);
            assert_eq!(completion, RequestCompletion::TimedOut);
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    assert_eq!(server.finished(request), Some(RequestOutcome::TimedOut));
    match client.claim(request) {
        Err(ClientError::Refused { code: refused, .. }) => assert_eq!(refused, code::REQUEST_FINISHED),
        other => panic!("a timed-out request stays finished, got {other:?}"),
    }
}

#[test]
fn an_empty_answer_is_not_an_ended_request() {
    // The two must never share a wire value: an empty answer is a provider choosing
    // "yes" with no content, while an ended request is one nobody chose to answer.
    // The first is consent the caller acts on, the second a refusal, so this pins the
    // encoding and the outcome apart for the whole round trip.
    let (mut server, mut client) = pair();
    let answered = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(500),
    );
    let ended = server.offer(
        "auth.polkit",
        "Authentication required",
        "Authenticate to continue",
        Tick::new(500),
    );
    for _ in 0..2 {
        assert!(matches!(
            client.next_event().expect("every offer reaches the client"),
            Event::RequestOffered { .. }
        ));
    }

    client.claim(answered).expect("the first claim wins");
    client
        .answer(answered, &[])
        .expect("an empty answer is still an answer");
    client.claim(ended).expect("the first claim wins");
    client.decline(ended).expect("declining is not answering");

    assert_eq!(
        server.finished(answered),
        Some(RequestOutcome::Answered(Vec::new())),
        "an empty answer arrives as an answer, never as a refusal"
    );
    assert_eq!(
        server.finished(ended),
        Some(RequestOutcome::Ended),
        "and an ended request never arrives as an empty answer"
    );

    let mut completions = Vec::new();
    for _ in 0..2 {
        match client.next_event().expect("both finishes reach the client") {
            Event::RequestFinished { completion, .. } => completions.push(completion),
            other => panic!("expected a finish, got {other:?}"),
        }
    }
    assert_eq!(
        completions,
        vec![RequestCompletion::Answered, RequestCompletion::Ended],
        "the broadcast keeps the two apart as well"
    );
}

#[test]
fn calls_and_changes_interleave_without_losing_events() {
    let (mut server, mut client) = pair();
    let window = open_window(&mut server, "Interleaved");
    client.subscribe(Filter::All).expect("subscribe");

    let operation = client
        .call(window, "focus", &Fields::new())
        .expect("`focus` is declared with no parameters");
    server
        .set(window, TITLE, Value::Text("Second".to_owned()))
        .expect("title is a declared string property");
    server
        .complete(operation.id(), OperationOutcome::Done)
        .expect("the operation is pending");

    assert_eq!(
        client.await_operation(operation).expect("the completion arrives"),
        OperationOutcome::Done
    );
    assert!(
        matches!(
            client.next_event().expect("the change read while awaiting"),
            Event::Model(Observation::Changed { property, .. }) if property == TITLE
        ),
        "the diff read on the way is kept, in order"
    );
    assert!(
        matches!(
            client.next_event().expect("the completion event"),
            Event::Model(Observation::Finished { .. })
        ),
        "and so is the completion"
    );
}
