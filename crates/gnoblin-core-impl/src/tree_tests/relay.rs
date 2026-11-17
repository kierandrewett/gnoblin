//! The view of the tree a runtime with an engine of its own (the JavaScript engine) reads while the library of `gnoblin-std` works on the
//! core's mirror: one holder of the compositor's link, two readers (design section 25.3, B2).
//!
//! The engine's side is played by a `TreeSession` over a link that calls the five `tree_link_*` functions, which is what the runtime's
//! engine does over the table.

use super::*;
use std::time::Duration;

use gnoblin_tree_client::relay;
use gnoblin_tree_client::session::TreeSession;
use gnoblin_tree_client::tree_mirror::{TreeIncoming, TreeTx, TxAnimation};

use crate::{link, session};

/// The engine's link: the relayed functions of the core, as the runtime calls them.
struct EngineLink(GnoblinCtx);

impl TreeLink for EngineLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        match link::attach(self.0, version) {
            abi::E_OK => Ok(()),
            _ => Err(LinkError(last_error())),
        }
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        let bytes = relay::encode_tx(tx);
        // SAFETY: `bytes` is a live local of `len` bytes.
        match unsafe { link::send(self.0, bytes.as_ptr(), bytes.len()) } {
            abi::E_OK => Ok(()),
            _ => Err(LinkError(last_error())),
        }
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        let mut buf = GnoblinBuf {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        // SAFETY: `buf` is a live local.
        if unsafe { link::recv(self.0, &raw mut buf) } != abi::E_OK {
            return Err(LinkError(last_error()));
        }
        let bytes = if buf.len == 0 {
            Vec::new()
        } else {
            // SAFETY: the core lent `len` bytes at `ptr`.
            unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
        };
        // SAFETY: `buf_free` is a function of the core's table, given a buffer it lent.
        assert_eq!(unsafe { (api().buf_free)(&raw mut buf) }, abi::E_OK);
        relay::decode_incoming(&bytes).map_err(|error| LinkError(error.to_string()))
    }
}

/// The rig, and an engine that has opened and attached its view of the tree.
fn with_engine() -> (Rig, TreeSession) {
    let rig = rig();
    assert_eq!(link::open(rig.ctx(), abi::ATTACH_HOLDER), abi::E_OK, "{}", last_error());
    let engine =
        TreeSession::attach(Box::new(EngineLink(rig.ctx())), Duration::from_secs(1)).expect("the engine attaches");
    (rig, engine)
}

fn core_nodes() -> Vec<u64> {
    session::with(|session| session.tree_ref().expect("attached").mirror().ids().collect())
}

fn core_number(node: u64, key: &str) -> f64 {
    session::with(|session| match session.tree_ref().expect("attached").get(node, key) {
        Some(MirrorValue::Double(value)) => *value,
        other => panic!("{key} of {node} is {other:?}"),
    })
}

#[test]
fn the_engine_boots_from_the_core_s_mirror_and_opening_the_view_is_not_refused_after_tree_attach() {
    let _serial = serial();
    let (rig, engine) = with_engine();
    let seen: Vec<u64> = engine.mirror().ids().collect();
    assert_eq!(seen, core_nodes(), "the engine sees the nodes the core holds");
    assert_eq!(
        engine.mirror().serial(),
        session::with(|s| s.tree_ref().unwrap().serial())
    );
    assert!(seen.contains(&WINDOW) && seen.contains(&SEAT));
    // A second `tree_attach` of the same shell finds the tree it holds.
    // SAFETY: `tree_attach` is a function of the core's table.
    assert_eq!(unsafe { (api().tree_attach)(rig.ctx(), abi::ATTACH_HOLDER) }, abi::E_OK);
}

#[test]
fn what_the_compositor_changes_reaches_both_readers() {
    let _serial = serial();
    let (rig, mut engine) = with_engine();
    rig.comp.external_set(WINDOW, "title", text("Renamed"));
    rig.dispatch();
    engine.pump().expect("the engine reads the change");
    let title = |value: Option<&MirrorValue>| match value {
        Some(MirrorValue::Text(text)) => text.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(title(engine.get(WINDOW, "title")), "Renamed");
    let core = session::with(|s| title(s.tree_ref().unwrap().get(WINDOW, "title")));
    assert_eq!(core, "Renamed");
}

#[test]
fn the_engine_writes_through_the_one_link_and_its_own_reply_settles_its_gate() {
    let _serial = serial();
    let (rig, mut engine) = with_engine();
    engine
        .set(WINDOW, "x", MirrorValue::Double(30.0), TxAnimation::default())
        .expect("a write");
    assert!(engine.flush().expect("a flush"));
    assert!(!engine.is_settled(), "the transaction is in flight");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !engine.is_settled() {
        assert!(
            std::time::Instant::now() < deadline,
            "the reply never reached the engine"
        );
        rig.dispatch();
        engine.pump().expect("a pump");
    }
    assert_eq!(rig.comp.number(WINDOW, "x"), 30.0);
    rig.dispatch();
    assert_eq!(
        core_number(WINDOW, "x"),
        30.0,
        "the core's mirror shows the engine's write"
    );
    assert_eq!(rig.comp.sent_count(), 1, "one transaction crossed the link");
}

#[test]
fn both_sides_write_in_one_turn_and_neither_transaction_is_refused() {
    let _serial = serial();
    let (rig, mut engine) = with_engine();
    let ctx = rig.ctx();
    assert_eq!(set(ctx, WINDOW, "window", "y", &double(7.0), 0.0), abi::E_OK);
    engine
        .set(WINDOW, "x", MirrorValue::Double(11.0), TxAnimation::default())
        .expect("a write");
    // The engine's transaction goes first and the core's waits behind it: two transactions are never on the wire at once.
    assert!(engine.flush().expect("a flush"));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        rig.dispatch();
        engine.pump().expect("a pump");
        if rig.comp.number(WINDOW, "x") == 11.0 && rig.comp.number(WINDOW, "y") == 7.0 && engine.is_settled() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "both writes never landed");
    }
    assert_eq!(rig.comp.refused(), 0, "no transaction was refused");
    assert_eq!(rig.comp.sent_count(), 2);
}

#[test]
fn the_core_and_the_engine_never_give_the_compositor_the_same_node_id() {
    let _serial = serial();
    let (_rig, mut engine) = with_engine();
    let theirs = engine.create("canvas").expect("a node");
    let ours = session::with(|s| s.tree().expect("attached").create("canvas")).expect("a node");
    assert_ne!(theirs, ours);
    assert!(
        theirs & 0xffff_ffff < ours & 0xffff_ffff,
        "the engine counts from one, the core from its own range"
    );
    assert_eq!(
        theirs >> 32,
        ours >> 32,
        "both are of the one epoch the compositor gave the holder"
    );
}
