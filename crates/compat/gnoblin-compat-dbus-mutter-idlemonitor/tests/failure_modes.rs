//! One test per failure mode that needs no bus: the registry, and the graph that feeds it.
//!
//! The registry holds no clock, so a test drives the whole of it with two numbers. That is
//! deliberate: a watch that fires on the wrong advance is a policy bug that is invisible in
//! a session and obvious here.

mod support;

use gnoblin_client::Server;
use gnoblin_compat_dbus_mutter_idlemonitor::{IdleError, IdleLink, IdleState, WatchKind, Watches};
use gnoblin_model::{Fields, ObjectId, ObjectView, TypeKey, Value};
use support::{VERSION, graph};

#[test]
fn a_graph_without_an_idle_counter_is_refused() {
    // The failure mode: the adapter reports zero idle time because nothing told it
    // otherwise, so every policy believes the user is active and no watch ever fires.
    let mut server = Server::new(graph(), VERSION);
    server
        .create(TypeKey::gnoblin("lock"), support::unlocked())
        .expect("the lock type is in the schema");
    let client = server.connect(VERSION).expect("the versions match");

    match IdleLink::connect(client) {
        Err(IdleError::NoCounter) => {}
        other => panic!("a graph with no idle object is refused, got {other:?}"),
    }
}

#[test]
fn a_mistyped_counter_is_refused() {
    // The failure mode: a decoded frame carries a property the schema does not promise and
    // the adapter silently coerces it to zero. The graph validates what it creates, so the
    // only producer of a mistyped value is a peer's frame, which is exactly what a
    // `ObjectView` is on the client side.
    let mut properties = Fields::new();
    properties.insert("idle_ms".to_owned(), Value::Text("five minutes".to_owned()));
    let view = ObjectView::new(ObjectId::new(1), TypeKey::gnoblin("idle"), properties);

    match IdleState::read([&view]) {
        Err(IdleError::Unreadable { property, .. }) => assert_eq!(property, "idle_ms"),
        other => panic!("a mistyped counter is refused, got {other:?}"),
    }
}

#[test]
fn an_idle_watch_fires_on_every_crossing() {
    // The failure mode: a policy that dims at five minutes and suspends at thirty needs one
    // watch each, and each has to fire again after the user comes back.
    let mut watches = Watches::new();
    let dim = watches.add_idle(":1.1", 300_000);
    let suspend = watches.add_idle(":1.1", 1_800_000);

    assert_eq!(watches.advance(299_999), Vec::<u32>::new());
    assert_eq!(watches.advance(300_000), vec![dim]);
    assert_eq!(
        watches.advance(300_001),
        Vec::<u32>::new(),
        "no second fire at the same value"
    );
    assert_eq!(watches.advance(1_800_000), vec![suspend]);
    assert_eq!(watches.advance(0), Vec::<u32>::new(), "activity fires no idle watch");
    assert_eq!(
        watches.advance(300_000),
        vec![dim],
        "the watch is still live after a crossing"
    );
}

#[test]
fn a_watch_added_while_already_idle_fires_on_the_next_advance() {
    // The failure mode: a caller that adds a watch while the session is already past the
    // interval waits for an activity that never comes, so its policy is silently late.
    let mut watches = Watches::new();
    watches.advance(400_000);
    let late = watches.add_idle(":1.1", 300_000);

    assert_eq!(watches.advance(400_001), vec![late]);
}

#[test]
fn an_active_watch_fires_once_and_is_gone() {
    // The failure mode: a resume handler runs on every activity after an idle period.
    let mut watches = Watches::new();
    let resume = watches.add_active(":1.1");
    assert_eq!(watches.advance(60_000), Vec::<u32>::new());
    assert_eq!(watches.advance(0), vec![resume]);
    assert!(watches.is_empty(), "a fired active watch is removed");
    assert_eq!(watches.advance(60_000), Vec::<u32>::new());
    assert_eq!(watches.advance(0), Vec::<u32>::new(), "and it never fires again");
}

#[test]
fn removing_a_watch_twice_is_refused() {
    // The failure mode: a caller's cleanup reports success for a watch that was already
    // gone, so a bug in its bookkeeping stays invisible.
    let mut watches = Watches::new();
    let id = watches.add_idle(":1.1", 1_000);
    watches.remove(":1.1", id).expect("the owner removes it once");
    match watches.remove(":1.1", id) {
        Err(IdleError::NoWatch { id: refused }) => assert_eq!(refused, id),
        other => panic!("a second removal is refused, got {other:?}"),
    }
}

#[test]
fn a_watch_is_recorded_against_its_caller() {
    // The failure mode: two callers' watches are indistinguishable, so one process cancels
    // another's policy.
    let mut watches = Watches::new();
    let mine = watches.add_idle(":1.1", 1_000);
    let theirs = watches.add_active(":1.2");

    match watches.remove(":1.2", mine) {
        Err(IdleError::ForeignWatch { id }) => assert_eq!(id, mine),
        other => panic!("another caller's watch is refused, got {other:?}"),
    }
    assert_eq!(
        watches.iter().map(|watch| (watch.id, watch.kind)).collect::<Vec<_>>(),
        vec![(mine, WatchKind::Idle), (theirs, WatchKind::UserActive)]
    );
    assert_eq!(watches.forget_owner(":1.1"), 1, "one caller's watches went");
    assert_eq!(watches.len(), 1, "the other caller's watch stays");
}

#[test]
fn a_watch_id_is_never_reused() {
    // The failure mode: a caller keeps an id after removing the watch and a later caller's
    // watch is fired under it, so the first caller acts on somebody else's idle event.
    let mut watches = Watches::new();
    let first = watches.add_idle(":1.1", 1_000);
    watches.remove(":1.1", first).expect("the owner removes it");
    let second = watches.add_idle(":1.2", 1_000);
    assert!(second > first, "a new watch gets a new id");
    assert!(second > 0, "upstream guarantees ids greater than zero");
}
