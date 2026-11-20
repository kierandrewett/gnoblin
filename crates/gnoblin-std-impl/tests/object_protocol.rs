#![allow(unsafe_code)]
//! The object protocol of `libgnoblin-std` through its C table, in this process: `entry_model` and `focus_policy` made, edited and
//! listened to the way a shell does, over a real core generation. The `#[ignore]`d test measures what a call costs.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use gnoblin_core_abi::{self as abi, GnoblinStr};
use gnoblin_core_impl::host;
use gnoblin_object::client::{Api, closure};
use gnoblin_object::{ClassId, MemberId, Val};
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, GnoblinStdApi};

/// The core's registry is one per process, so the tests of this file take turns.
fn process_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn last_error() -> String {
    // SAFETY: with a null buffer and zero capacity the call only returns the length.
    let len = unsafe { gnoblin_std_impl::gnoblin_std_last_error(std::ptr::null_mut(), 0) };
    let mut buf = vec![0_u8; len + 1];
    // SAFETY: `buf` is writable for `len + 1` bytes.
    let written = unsafe { gnoblin_std_impl::gnoblin_std_last_error(buf.as_mut_ptr(), buf.len()) };
    buf.truncate(written.min(len));
    String::from_utf8_lossy(&buf).into_owned()
}

/// A generation of the core with the std object protocol over it.
struct Rig {
    api: Api,
    ctx: u64,
    _lock: MutexGuard<'static, ()>,
}

impl Rig {
    fn new() -> Rig {
        let lock = process_lock();
        host::reset_session();
        let ctx = host::open_generation("object-protocol", abi::LANG_NATIVE).expect("a generation opens");
        let mut table: *const GnoblinStdApi = std::ptr::null();
        // SAFETY: `table` is a live local the library writes.
        let code = unsafe { gnoblin_std_impl::gnoblin_std_get_api(ABI_MAJOR, ABI_MINOR, &raw mut table) };
        assert_eq!(code, abi::E_OK);
        // SAFETY: the library's table is static.
        let table = unsafe { &*table };
        let mut raw: *const abi::GnoblinObjectApi = std::ptr::null();
        // SAFETY: `raw` is a live local the library writes.
        let code = unsafe { (table.object_api)(ctx, GnoblinStr::borrowed("gnoblin.std"), &raw mut raw) };
        assert_eq!(code, abi::E_OK, "{}", last_error());
        // SAFETY: the table `object_api` returns is static.
        let api = Api::new(unsafe { &*raw }, ctx, last_error);
        Rig { api, ctx, _lock: lock }
    }

    /// Runs the turn: what the library queued is told to the listeners.
    fn turn(&self) {
        // SAFETY: the core's table is static and `dispatch` takes plain values.
        unsafe { (host::api().dispatch)(self.ctx, 0) };
    }

    fn member(&self, class: ClassId, kind: u32, name: &str) -> MemberId {
        self.api
            .member(class, kind, name)
            .expect("the member is in the catalog")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        host::close_generation(self.ctx);
        host::reset_session();
    }
}

fn texts(value: &Val) -> String {
    value.text().expect("text").to_owned()
}

#[test]
fn an_entry_model_is_made_edited_and_heard() {
    let rig = Rig::new();
    let api = rig.api;
    let class = api.class("entry_model").expect("the class");
    let text = rig.member(class, abi::MEMBER_PROPERTY, "text");
    let length = rig.member(class, abi::MEMBER_PROPERTY, "length");
    let caret = rig.member(class, abi::MEMBER_PROPERTY, "caret");
    let selection = rig.member(class, abi::MEMBER_PROPERTY, "selection");
    let insert = rig.member(class, abi::MEMBER_METHOD, "insert");
    let select_all = rig.member(class, abi::MEMBER_METHOD, "select_all");
    let backspace = rig.member(class, abi::MEMBER_METHOD, "backspace");
    let left = rig.member(class, abi::MEMBER_METHOD, "left");
    let before = rig.member(class, abi::MEMBER_METHOD, "before");
    let changed = rig.member(class, abi::MEMBER_SIGNAL, "changed");

    let obj = api
        .create(class, &[(text, Val::from("héllo"))])
        .expect("an entry model");
    assert_eq!(
        api.get(obj, length).unwrap().uint().unwrap(),
        5,
        "code points, not bytes"
    );
    assert_eq!(
        api.get(obj, caret).unwrap().uint().unwrap(),
        5,
        "the caret starts at the end"
    );
    assert!(api.get(obj, selection).unwrap().is_none(), "nothing is selected");

    // A listener hears `changed` once for each call that changed something, from the next turn, never inside the call.
    let heard = Rc::new(RefCell::new(0_u32));
    let counter = Rc::clone(&heard);
    api.listen(obj, changed, move |_, _| {
        *counter.borrow_mut() += 1;
        Ok(Val::None)
    })
    .expect("a listener");
    let texted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&texted);
    api.listen(obj, text, move |_, args| {
        seen.borrow_mut().push(args[0].text().expect("the new text").to_owned());
        Ok(Val::None)
    })
    .expect("a property listener");

    api.call(obj, insert, &[Val::from(" wörld\u{7}")])
        .expect("insert types over the caret, leaving the control character");
    assert_eq!(
        *heard.borrow(),
        0,
        "a listener never runs inside the call that caused the change"
    );
    assert_eq!(texts(&api.get(obj, text).unwrap()), "héllo wörld");
    rig.turn();
    assert_eq!(*heard.borrow(), 1);
    assert_eq!(*texted.borrow(), vec!["héllo wörld".to_owned()]);

    api.call(obj, select_all, &[]).unwrap();
    let range = api.get(obj, selection).unwrap();
    assert_eq!(
        range
            .record()
            .unwrap()
            .iter()
            .map(|v| v.uint().unwrap())
            .collect::<Vec<_>>(),
        [0, 11]
    );
    api.call(obj, backspace, &[]).unwrap();
    assert_eq!(
        texts(&api.get(obj, text).unwrap()),
        "",
        "deleting removes the selection"
    );
    api.call(obj, left, &[]).unwrap();
    api.call(obj, left, &[]).unwrap();
    rig.turn();
    assert_eq!(
        *heard.borrow(),
        3,
        "select-all and the delete changed the model and each is told; the two moves left at the start changed nothing and tell nothing"
    );
    assert_eq!(*texted.borrow(), vec!["héllo wörld".to_owned(), String::new()]);
    assert_eq!(texts(&api.call(obj, before, &[Val::Int(0)]).unwrap()), "");

    // The checks the catalog implies.
    let failure = api.set(obj, length, &Val::Uint(3)).expect_err("`length` is read-only");
    assert_eq!(failure.code, abi::E_TYPE, "{failure}");
    let failure = api
        .call(obj, insert, &[Val::Bool(true)])
        .expect_err("insert takes text");
    assert_eq!(failure.code, abi::E_TYPE, "{failure}");
    let failure = api.call(obj, insert, &[]).expect_err("insert needs its argument");
    assert_eq!(failure.code, abi::E_TYPE, "{failure}");

    // A destroyed object's handle is stale for good, even when its slot is used again.
    api.destroy(obj).unwrap();
    assert_eq!(api.get(obj, text).expect_err("stale").code, abi::E_STALE);
    let again = api.create(class, &[]).unwrap();
    assert_ne!(again, obj);
    assert_eq!(api.get(obj, text).expect_err("still stale").code, abi::E_STALE);
    api.release(again).unwrap();
}

#[test]
fn the_focus_decisions_cross_as_records_and_enums() {
    let rig = Rig::new();
    let api = rig.api;
    let class = api.class("focus_policy").expect("the class");
    let decision = rig.member(class, abi::MEMBER_METHOD, "focus_decision");
    let mode = api.class("focus_mode").expect("the enum");
    let kind = api.class("focus_event_kind").expect("the enum");
    let value = |class: ClassId, name: &str| Val::Enum {
        class,
        value: api.member(class, abi::MEMBER_VALUE, name).expect("an enum value"),
    };
    // Sloppy focus with rest-to-focus on: an entry asks for the delayed focus, and a held button asks for nothing.
    let settings = Val::Record {
        class: 0,
        fields: vec![
            value(mode, "sloppy"),
            Val::None,
            Val::None,
            Val::Bool(true),
            Val::Double(250.0),
            Val::Bool(true),
        ],
    };
    let event = |name: &str, held: bool| Val::Record {
        class: 0,
        fields: vec![value(kind, name), Val::None, Val::Bool(held)],
    };
    let answer = api
        .call_static(class, decision, &[settings.clone(), event("enter", false)])
        .unwrap();
    let fields = answer.record().unwrap();
    assert!(!fields[0].bool().unwrap(), "rest-to-focus does not focus at once");
    assert_eq!(fields[3].double().unwrap(), 200.0, "it asks for the rest delay");
    assert_eq!(fields[4].double().unwrap(), 250.0, "and auto raise starts on entry");
    let answer = api
        .call_static(class, decision, &[settings, event("enter", true)])
        .unwrap();
    assert!(
        answer
            .record()
            .unwrap()
            .iter()
            .all(|field| matches!(field, Val::Bool(false) | Val::None))
    );
    // A value of the wrong type is refused with the member's name.
    let failure = api
        .call_static(class, decision, &[Val::Bool(true), Val::None])
        .expect_err("not a record");
    assert_eq!(failure.code, abi::E_TYPE, "{failure}");
}

#[test]
fn a_generation_ending_drops_what_a_closure_holds() {
    let rig = Rig::new();
    let api = rig.api;
    let class = api.class("entry_model").unwrap();
    let changed = rig.member(class, abi::MEMBER_SIGNAL, "changed");
    let obj = api.create(class, &[]).unwrap();
    let dropped = Rc::new(RefCell::new(false));
    struct Flag(Rc<RefCell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            *self.0.borrow_mut() = true;
        }
    }
    let flag = Flag(Rc::clone(&dropped));
    api.listen(obj, changed, move |_, _| {
        let _ = &flag;
        Ok(Val::None)
    })
    .unwrap();
    assert!(!*dropped.borrow());
    api.destroy(obj).unwrap();
    assert!(
        *dropped.borrow(),
        "the listener's `drop_user` ran when its object ended"
    );
    // A closure given in a call that fails is not adopted: the caller still owns it.
    let dropped_here = Rc::new(RefCell::new(false));
    let guard = Flag(Rc::clone(&dropped_here));
    let callback = closure(move |_, _| {
        let _ = &guard;
        Ok(Val::None)
    });
    let text = rig.member(class, abi::MEMBER_PROPERTY, "text");
    let object = api.create(class, &[]).unwrap();
    assert!(api.set(object, text, &Val::Closure(callback)).is_err());
    assert!(
        *dropped_here.borrow(),
        "the failed call's closure was freed by its owner"
    );
}

/// What a call costs across the C table, in nanoseconds. Run it with
/// `cargo test -p gnoblin-std-impl --release --test object_protocol -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn what_a_call_costs() {
    let rig = Rig::new();
    let api = rig.api;
    let class = api.class("entry_model").unwrap();
    let caret = rig.member(class, abi::MEMBER_PROPERTY, "caret");
    let text = rig.member(class, abi::MEMBER_PROPERTY, "text");
    let home = rig.member(class, abi::MEMBER_METHOD, "home");
    let insert = rig.member(class, abi::MEMBER_METHOD, "insert");
    let changed = rig.member(class, abi::MEMBER_SIGNAL, "changed");
    let obj = api
        .create(class, &[(text, Val::from("a text of some length"))])
        .unwrap();
    let n: u32 = 200_000;
    let each = |label: &str, run: &mut dyn FnMut()| {
        run();
        let start = Instant::now();
        for _ in 0..n {
            run();
        }
        println!(
            "object-protocol {label}: {:.0} ns",
            start.elapsed().as_nanos() as f64 / f64::from(n)
        );
    };
    each("get of a number (caret)", &mut || {
        let _ = api.get(obj, caret).unwrap();
    });
    each("get of text", &mut || {
        let _ = api.get(obj, text).unwrap();
    });
    each("set of text", &mut || {
        api.set(obj, text, &Val::from("a text of some length")).unwrap()
    });
    each("call of a method, no arguments (home)", &mut || {
        let _ = api.call(obj, home, &[]).unwrap();
    });
    each("call of a method with text (insert, then undo)", &mut || {
        let _ = api.call(obj, insert, &[Val::from("x")]).unwrap();
    });
    each("subscribe and unsubscribe", &mut || {
        let reg = api.listen(obj, changed, |_, _| Ok(Val::None)).unwrap();
        api.unlisten(reg).unwrap();
    });
    let heard = Rc::new(RefCell::new(0_u32));
    let counter = Rc::clone(&heard);
    api.listen(obj, changed, move |_, _| {
        *counter.borrow_mut() += 1;
        Ok(Val::None)
    })
    .unwrap();
    let start = Instant::now();
    for _ in 0..n {
        api.call(obj, insert, &[Val::from("y")]).unwrap();
        rig.turn();
    }
    let per = start.elapsed().as_nanos() as f64 / f64::from(n);
    println!("object-protocol call that changes + signal delivered into a closure, with the turn: {per:.0} ns");
    assert_eq!(*heard.borrow(), n);
}
