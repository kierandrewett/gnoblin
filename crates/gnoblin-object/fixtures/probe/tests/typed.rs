//! The typed Rust API that `gnoblin-sdkgen` writes, run against a library built from the same catalog: an enum, a record, a list, a
//! closure property, an interface, a static function, a signal and a property listener, through the generated wrappers.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_fixture_object_probe::api::{Paint, PaintHandlers, Point, Probe, Shade, Span};
use gnoblin_fixture_object_probe::{live_objects, objects, problems, turn};
use gnoblin_object::Fail;

#[test]
fn the_catalog_is_implemented() {
    assert!(problems().is_empty(), "{:?}", problems());
}

#[test]
fn an_enum_a_record_and_a_list_cross_as_rust_types() {
    let objects = objects();
    let probe = Probe::new(&objects, 3).unwrap();
    assert_eq!(
        probe.count().unwrap(),
        3,
        "a constructor argument is a property that starts set"
    );
    assert_eq!(probe.shade().unwrap(), Shade::Light, "the schema default");
    probe.set_shade(Shade::HalfTone).unwrap();
    assert_eq!(probe.shade().unwrap(), Shade::HalfTone);
    assert_eq!(Shade::HalfTone.as_str(), "half-tone");
    assert_eq!(Shade::ALL.len(), 3);

    assert_eq!(
        probe.origin().unwrap(),
        None,
        "a record the library has not been given reads as absent"
    );
    let origin = Point {
        x: 3.0,
        y: 4.0,
        label: Some("here".to_owned()),
        ..Point::default()
    };
    assert_eq!(origin.shade, Shade::Dark, "a field with a default starts at it");
    probe.set_origin(&origin).unwrap();
    let read = probe.origin().unwrap().expect("set");
    assert_eq!(read, origin, "a record comes back as it went in");
    assert_eq!(probe.reach(&origin).unwrap(), 5.0);
    assert_eq!(
        probe.reach(&Point::default()).unwrap(),
        0.0,
        "a field left out is its default"
    );

    // A record with fields that are always there is made with them, and the optional one starts as none.
    let span = Span::new(1.0, 4.0);
    assert_eq!(span.closed, None);
    assert_eq!(probe.length_of(&span).unwrap(), 3.0);
    assert_eq!(
        probe
            .length_of(&Span {
                closed: Some(true),
                ..span
            })
            .unwrap(),
        4.0
    );

    let mark = probe.mark().unwrap();
    assert_eq!(
        (mark.x, mark.y, mark.label.as_deref(), mark.shade),
        (3.0, 1.0, Some("mark"), Shade::Dark)
    );
    assert_eq!(
        probe
            .total(&[
                Point {
                    x: 1.5,
                    ..Point::default()
                },
                Point {
                    x: 2.0,
                    ..Point::default()
                }
            ])
            .unwrap(),
        3.5
    );

    assert!(probe.names().unwrap().is_empty());
    probe.set_names(&["a".to_owned(), "b".to_owned()]).unwrap();
    assert_eq!(probe.names().unwrap(), ["a", "b"]);
}

#[test]
fn calls_take_optional_arguments_and_static_functions_need_no_object() {
    let objects = objects();
    let probe = Probe::new(&objects, 0).unwrap();
    assert_eq!(probe.add(2, 5).unwrap(), 7);
    assert_eq!(probe.greet("ada", None).unwrap(), "hello, ada");
    assert_eq!(probe.greet("ada", Some(true)).unwrap(), "HELLO, ADA!");
    assert_eq!(Probe::clamp(&objects, 15, 0, 10).unwrap(), 10);
    let refused: Fail = Probe::clamp(&objects, 1, 5, 2).unwrap_err();
    assert_eq!(refused.code, gnoblin_core_abi::E_TYPE);
    assert!(refused.message.contains("`low`"), "{}", refused.message);
}

#[test]
fn a_closure_property_a_signal_and_a_property_listener_run_from_the_turn() {
    let objects = objects();
    let probe = Probe::new(&objects, 0).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    probe
        .set_on_tick(move |x| {
            log.borrow_mut().push(x);
            Ok(x * 10)
        })
        .unwrap();
    let told = Rc::new(Cell::new(0));
    let counter = Rc::clone(&told);
    let ticked = probe.on_ticked(move |x| counter.set(counter.get() + x)).unwrap();
    let counts = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&counts);
    let watching = probe
        .on_count_changed(move |count| log.borrow_mut().push(count))
        .unwrap();

    probe.fire(4).unwrap();
    assert!(
        seen.borrow().is_empty(),
        "the closure never runs inside the call that asked for it"
    );
    for count in [1, 2, 3] {
        probe.set_count(count).unwrap();
    }
    turn();
    assert_eq!(*seen.borrow(), [4]);
    assert_eq!(
        probe.last().unwrap(),
        40,
        "the closure's answer came back to the library"
    );
    assert_eq!(told.get(), 4);
    assert_eq!(
        *counts.borrow(),
        [3],
        "the changes of one turn are told once, with the last value"
    );

    drop(ticked);
    drop(watching);
    probe.fire(1).unwrap();
    probe.set_count(9).unwrap();
    turn();
    assert_eq!(told.get(), 4, "a dropped guard ends its listener");
    assert_eq!(*counts.borrow(), [3]);
    assert_eq!(*seen.borrow(), [4, 1], "the closure property stays set");
}

#[test]
fn an_interface_the_shell_implements_runs_only_the_handlers_it_gave() {
    let objects = objects();
    let probe = Probe::new(&objects, 0).unwrap();
    let plain: Paint = PaintHandlers {
        colour: Box::new(|| Ok("teal".to_owned())),
        size: None,
        scaled: None,
    }
    .implement(&objects)
    .unwrap();
    probe.set_painter(&plain).unwrap();
    probe.paint().unwrap();
    turn();
    assert_eq!(
        probe.last_colour().unwrap(),
        "teal",
        "an absent optional handler is not called"
    );

    let sized: Paint = PaintHandlers {
        colour: Box::new(|| Ok("red".to_owned())),
        size: Some(Box::new(|| Ok(3.0))),
        scaled: Some(Box::new(|factor| Ok(factor * 2.0))),
    }
    .implement(&objects)
    .unwrap();
    probe.set_painter(&sized).unwrap();
    probe.paint().unwrap();
    turn();
    assert_eq!(
        probe.last_colour().unwrap(),
        "red 3",
        "a handler the shell gave is called"
    );
    assert!(
        probe.painter().unwrap().is_some(),
        "the painter reads back as an object"
    );
}

#[test]
fn a_handle_releases_itself_and_destroy_ends_it() {
    let objects = objects();
    let before = live_objects();
    let probe = Probe::new(&objects, 0).unwrap();
    assert_eq!(live_objects(), before + 1);
    drop(probe);
    assert_eq!(
        live_objects(),
        before,
        "an object nothing holds ends when its last reference goes"
    );

    let probe = Probe::new(&objects, 0).unwrap();
    probe.destroy().unwrap();
    assert_eq!(
        probe.count().unwrap_err().code,
        gnoblin_core_abi::E_STALE,
        "a destroyed object is stale, not a new one"
    );
}
