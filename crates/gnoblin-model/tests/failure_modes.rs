//! One test per documented failure mode, plus the behaviours the model exists to
//! provide.
//!
//! The schema is the real one (`crates/gnoblin-schema/schema`), so these tests fail
//! if the graph and the contract drift apart.

use std::path::Path;

use gnoblin_model::{
    Answer, ErrorCode, Fields, Filter, Graph, ModelError, ObjectId, Observation, OperationOutcome, OwnerId,
    RequestBroker, RequestError, RequestEvent, RequestOutcome, RequestState, Revision, Subscription, Tick, TypeKey,
    Value,
};
use gnoblin_schema::Catalog;

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
    properties
}

fn open_window(graph: &mut Graph, title: &str) -> ObjectId {
    graph
        .create(TypeKey::gnoblin("window"), window_properties(title))
        .expect("the window type and its properties are in the schema")
}

/// The diffs a subscription has accumulated, in the order it received them.
fn drain(graph: &mut Graph, subscription: &Subscription) -> Vec<Observation> {
    let mut drained = Vec::new();
    while let Some(observation) = graph.next(subscription) {
        drained.push(observation);
    }
    drained
}

fn changed_title(observation: &Observation) -> Option<&Value> {
    match observation {
        Observation::Changed { property, value, .. } if property == TITLE => Some(value),
        _ => None,
    }
}

#[test]
fn a_late_subscriber_misses_nothing_and_repeats_nothing() {
    let mut graph = graph();
    let window = open_window(&mut graph, "First");
    graph
        .set(window, TITLE, Value::Text("Second".to_owned()))
        .expect("title is a declared string property");

    let at = graph.revision();
    assert!(at > Revision::ZERO, "the two earlier changes are revisions");

    let subscription = graph.subscribe(Filter::All);
    assert_eq!(subscription.snapshot().revision(), at);
    let view = subscription.snapshot().object(window).expect("the window is live");
    assert_eq!(view.property(TITLE), Some(&Value::Text("Second".to_owned())));

    assert_eq!(
        graph.next(&subscription),
        None,
        "a subscriber must not be replayed the diffs its snapshot already contains"
    );

    graph
        .set(window, TITLE, Value::Text("Third".to_owned()))
        .expect("title is a declared string property");
    let observed = drain(&mut graph, &subscription);
    assert_eq!(observed.len(), 1, "exactly the one later diff");
    assert_eq!(changed_title(&observed[0]), Some(&Value::Text("Third".to_owned())));
    assert_eq!(observed[0].revision().get(), at.get() + 1);
    assert_eq!(graph.next(&subscription), None, "and exactly once");
}

#[test]
fn an_operation_completes_once() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Focusable");
    let subscription = graph.subscribe(Filter::All);

    let operation = graph
        .call(window, "focus", Fields::new())
        .expect("`focus` is declared with no parameters");
    assert_eq!(graph.pending().count(), 1);

    assert!(graph.complete(operation.id(), OperationOutcome::Done).is_ok());
    assert_eq!(graph.pending().count(), 0);
    assert_eq!(
        graph.complete(
            operation.id(),
            OperationOutcome::Failed(ErrorCode::parse("refused").expect("kebab-case"))
        ),
        Err(ModelError::UnknownOperation {
            operation: operation.id()
        })
    );

    let finished = drain(&mut graph, &subscription)
        .into_iter()
        .filter(|observation| matches!(observation, Observation::Finished { .. }))
        .count();
    assert_eq!(finished, 1, "a subscriber sees one completion, not two");
}

#[test]
fn an_answer_after_the_finish_is_refused() {
    let mut broker = RequestBroker::new();
    let deadline = Tick::new(1_000);
    let request = broker.offer(deadline);
    let owner = OwnerId::new(7);

    broker.claim(request, owner).expect("the first claim wins");
    assert_eq!(
        broker.events(),
        vec![RequestEvent::Offered { request, deadline }],
        "claiming is not an event of its own: only offers and finishes are"
    );

    broker
        .answer(request, owner, Answer::Answered(vec![0x42]))
        .expect("the owner may answer once");
    assert_eq!(
        broker.answer(request, owner, Answer::Ended),
        Err(RequestError::Finished { request })
    );
    assert_eq!(
        broker.tick(Tick::new(4_000)),
        0,
        "a finished request never times out again"
    );
    assert_eq!(
        broker.state(request),
        Some(RequestState::Finished(RequestOutcome::Answered(vec![0x42])))
    );
    assert_eq!(
        broker.events(),
        vec![RequestEvent::Finished {
            request,
            outcome: RequestOutcome::Answered(vec![0x42])
        }],
        "exactly one finished outcome, in order after the offer"
    );
}

#[test]
fn two_claimants_only_the_first_wins() {
    let mut broker = RequestBroker::new();
    let request = broker.offer(Tick::new(50));
    let first = OwnerId::new(1);
    let second = OwnerId::new(2);

    broker.claim(request, first).expect("the first claim wins");
    assert_eq!(
        broker.claim(request, second),
        Err(RequestError::Taken { request, owner: first }),
        "the loser learns who holds it instead of waiting"
    );
    assert_eq!(
        broker.claim(request, first),
        Ok(()),
        "a client retrying the claim it already won is not told it lost"
    );
    assert_eq!(
        broker.answer(request, second, Answer::Ended),
        Err(RequestError::NotOwner { request, owner: second }),
        "a non-owner cannot answer on the holder's behalf"
    );
    assert_eq!(
        broker.state(request),
        Some(RequestState::Claimed {
            owner: first,
            deadline: Tick::new(50)
        })
    );
}

#[test]
fn diffs_arrive_in_revision_order() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Ordered");
    let subscription = graph.subscribe(Filter::All);

    let mut published = 0_u64;
    for step in 0..8 {
        graph
            .set(window, TITLE, Value::Text(format!("Step {step}")))
            .expect("title is a declared string property");
        published += 1;
    }
    graph
        .signal(window, "closed", Fields::new())
        .expect("`closed` is a declared signal");
    published += 1;

    let observed = drain(&mut graph, &subscription);
    assert_eq!(observed.len() as u64, published);
    let mut previous = subscription.snapshot().revision();
    for observation in &observed {
        assert!(
            observation.revision() > previous,
            "{} is not after {previous}",
            observation.revision()
        );
        previous = observation.revision();
    }
    assert_eq!(previous, graph.revision());
}

#[test]
fn a_subscriber_outlives_its_object() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Doomed");
    let subscription = graph.subscribe(Filter::Object(window));

    graph.close(window).expect("the window is live");
    let end = graph.next(&subscription).expect("the object's end is delivered");
    assert!(matches!(end, Observation::Closed { object, .. } if object == window));
    assert_eq!(graph.next(&subscription), None, "and delivered exactly once");

    assert_eq!(
        graph.set(window, TITLE, Value::Text("ghost".to_owned())),
        Err(ModelError::UnknownObject { object: window }),
        "a closed object takes no writes"
    );
    assert_eq!(graph.close(window), Err(ModelError::UnknownObject { object: window }));

    let fresh = open_window(&mut graph, "Fresh");
    assert_ne!(fresh, window, "an identity is never reused");
    let late = graph.subscribe(Filter::All);
    assert!(
        late.snapshot().objects().iter().all(|view| view.id() != window),
        "a new snapshot never mentions a closed object"
    );
}

#[test]
fn closing_an_object_finishes_its_pending_operations() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Closing");
    let subscription = graph.subscribe(Filter::Object(window));

    let operation = graph
        .call(window, "focus", Fields::new())
        .expect("`focus` is declared with no parameters");
    graph.close(window).expect("the window is live");

    let observed = drain(&mut graph, &subscription);
    let outcomes: Vec<&OperationOutcome> = observed
        .iter()
        .filter_map(|observation| match observation {
            Observation::Finished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes.len(), 1, "the operation finished rather than dangling");
    assert_eq!(
        outcomes[0],
        &OperationOutcome::Failed(ErrorCode::parse("object-gone").expect("kebab-case")),
        "and the client learns why"
    );
    assert!(matches!(observed.last(), Some(Observation::Closed { .. })));
    assert_eq!(
        graph.complete(operation.id(), OperationOutcome::Done),
        Err(ModelError::UnknownOperation {
            operation: operation.id()
        }),
        "the end of the object was the operation's one completion"
    );
}

#[test]
fn a_wrongly_typed_value_never_enters_the_graph() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Typed");
    let subscription = graph.subscribe(Filter::All);
    let before = graph.revision();

    let refused = graph.set(window, TITLE, Value::Uint(5));
    assert!(
        matches!(refused, Err(ModelError::WrongType { .. })),
        "a uint is not a string: {refused:?}"
    );
    assert_eq!(graph.revision(), before, "a refused write publishes nothing");
    assert!(drain(&mut graph, &subscription).is_empty());

    let mut properties = window_properties("Partial");
    properties.remove(TITLE);
    assert!(matches!(
        graph.create(TypeKey::gnoblin("window"), properties),
        Err(ModelError::MissingProperty { .. })
    ));

    let mut properties = window_properties("Extra");
    properties.insert("focus".to_owned(), Value::Bool(true));
    assert!(matches!(
        graph.create(TypeKey::gnoblin("window"), properties),
        Err(ModelError::UnexpectedProperty { .. })
    ));
    assert_eq!(graph.revision(), before, "neither refused creation published anything");
}

#[test]
fn an_undeclared_member_is_refused() {
    let mut graph = graph();
    let window = open_window(&mut graph, "Declared");

    assert!(matches!(
        graph.create(TypeKey::gnoblin("nonexistent"), Fields::new()),
        Err(ModelError::UnknownType { .. })
    ));
    assert!(matches!(
        graph.set(window, "focusedness", Value::Bool(true)),
        Err(ModelError::UnknownProperty { .. })
    ));
    assert!(matches!(
        graph.call(window, "unfocus", Fields::new()),
        Err(ModelError::UnknownMethod { .. })
    ));
    assert!(matches!(
        graph.signal(window, "unclosed", Fields::new()),
        Err(ModelError::UnknownSignal { .. })
    ));
    assert!(matches!(
        graph.signal(window, "closed", {
            let mut fields = Fields::new();
            fields.insert("reason".to_owned(), Value::Text("no".to_owned()));
            fields
        }),
        Err(ModelError::UnexpectedField { .. })
    ));
    assert!(matches!(
        graph.call(window, "focus", {
            let mut arguments = Fields::new();
            arguments.insert("force".to_owned(), Value::Bool(true));
            arguments
        }),
        Err(ModelError::UnexpectedArgument { .. })
    ));
}

#[test]
fn a_request_times_out_fail_closed() {
    let mut broker = RequestBroker::new();
    let deadline = Tick::new(100);
    let request = broker.offer(deadline);
    let owner = OwnerId::new(1);
    broker.claim(request, owner).expect("claiming an offered request works");

    assert_eq!(broker.tick(Tick::new(99)), 0, "not yet due");
    assert_eq!(broker.state(request), Some(RequestState::Claimed { owner, deadline }));

    assert_eq!(broker.tick(deadline), 1, "the deadline is inclusive");
    assert_eq!(
        broker.state(request),
        Some(RequestState::Finished(RequestOutcome::TimedOut))
    );
    assert_eq!(
        broker.answer(request, owner, Answer::Ended),
        Err(RequestError::Finished { request }),
        "a timed-out request stays finished: fail closed"
    );
    assert_eq!(broker.tick(Tick::new(1_000)), 0, "it times out once");
    assert_eq!(broker.claim(request, owner), Err(RequestError::Finished { request }));

    let events = broker.events();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], RequestEvent::Offered { .. }));
    assert_eq!(
        events[1],
        RequestEvent::Finished {
            request,
            outcome: RequestOutcome::TimedOut
        }
    );
}

#[test]
fn a_departed_owner_finishes_its_requests() {
    let mut broker = RequestBroker::new();
    let first = broker.offer(Tick::new(10));
    let second = broker.offer(Tick::new(10));
    let untouched = broker.offer(Tick::new(500));
    let provider = OwnerId::new(3);
    let other = OwnerId::new(4);

    broker.claim(first, provider).expect("claim");
    broker.claim(second, provider).expect("claim");
    broker.claim(untouched, other).expect("claim");
    assert_eq!(broker.events().len(), 3, "three offers");

    assert_eq!(broker.release(provider), 2);
    let events = broker.events();
    assert_eq!(events.len(), 2, "each released request finishes exactly once");
    assert!(events.iter().all(|event| matches!(
        event,
        RequestEvent::Finished {
            outcome: RequestOutcome::Ended,
            ..
        }
    )));
    assert_eq!(broker.state(first), Some(RequestState::Finished(RequestOutcome::Ended)));
    assert_eq!(
        broker.state(untouched),
        Some(RequestState::Claimed {
            owner: other,
            deadline: Tick::new(500)
        }),
        "another owner's request is untouched"
    );
    assert_eq!(broker.release(provider), 0, "releasing twice leaves nothing hanging");
}

#[test]
fn a_session_signal_carries_its_declared_field() {
    let mut graph = graph();
    let mut properties = Fields::new();
    properties.insert("version".to_owned(), Value::Text("0.2.0".to_owned()));
    let session = graph
        .create(TypeKey::gnoblin("session"), properties)
        .expect("the session type takes a version string");
    let subscription = graph.subscribe(Filter::Object(session));

    graph
        .signal(session, "ready", Fields::new())
        .expect("`ready` is declared with no fields");
    let observed = drain(&mut graph, &subscription);
    assert!(matches!(
        observed.as_slice(),
        [Observation::Signalled { signal, .. }] if signal == "ready"
    ));
}

#[test]
fn unsubscribing_stops_the_queue_and_reports_whether_it_existed() {
    let mut graph = graph();
    let subscription = graph.subscribe(Filter::All);
    let id = subscription.id();
    let window = open_window(&mut graph, "After");

    assert_eq!(drain(&mut graph, &subscription).len(), 1);
    assert!(graph.unsubscribe(id));
    assert!(!graph.unsubscribe(id), "unsubscribing twice is reported");
    graph
        .set(window, TITLE, Value::Text("Later".to_owned()))
        .expect("the window is live");
    assert_eq!(graph.next(&subscription), None, "a dropped subscription gets nothing");
}
