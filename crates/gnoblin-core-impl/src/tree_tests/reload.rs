//! What a reload hands from one generation to the next, and what an abandoned stage gives back (`design/modular-framework.md`
//! section 8.1): nodes found again by name, ephemeral nodes that die with their generation, the versioned state, and the writes,
//! names and state a failed stage must not leave behind.
//!
//! A "reload" here is what `NativeShell::reload` does, done by hand on the registry so that each step can be looked at: a staged
//! generation runs its `init` against the live instance, the old one ends, the new one takes over, the old one retires.

use super::*;
use gnoblin_tree_client::tree_mirror::TxOp;

fn adopt(ctx: GnoblinCtx, class_name: &str, name: &str, flags: u32) -> (i32, u64) {
    let mut node = 0;
    // SAFETY: `node_adopt` is a function of the core's table; `name` is `len` readable bytes and `node` a live local.
    let rc = unsafe { (api().node_adopt)(ctx, class(class_name), name.as_ptr(), name.len(), flags, &raw mut node) };
    (rc, node)
}

fn keep(ctx: GnoblinCtx, key: &str, bytes: &[u8], version: u32) -> i32 {
    // SAFETY: `state_put` is a function of the core's table; both ranges are live for the call.
    unsafe { (api().state_put)(ctx, key.as_ptr(), key.len(), bytes.as_ptr(), bytes.len(), version) }
}

fn kept(ctx: GnoblinCtx, key: &str) -> Option<(Vec<u8>, u32)> {
    let mut buf = GnoblinBuf {
        ptr: core::ptr::null_mut(),
        len: 0,
        cap: 0,
    };
    let mut version = 0;
    // SAFETY: `state_get` is a function of the core's table; `buf` and `version` are live locals.
    let rc = unsafe { (api().state_get)(ctx, key.as_ptr(), key.len(), &raw mut buf, &raw mut version) };
    if rc == abi::E_NO_SUCH_MEMBER {
        return None;
    }
    assert_eq!(rc, abi::E_OK, "{}", last_error());
    // SAFETY: the core lent `len` bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
    // SAFETY: `buf_free` is a function of the core's table, given a buffer it lent.
    assert_eq!(unsafe { (api().buf_free)(&raw mut buf) }, abi::E_OK);
    Some((bytes, version))
}

fn new_nodes(comp: &FakeCompositor) -> usize {
    comp.ops().iter().filter(|op| matches!(op, TxOp::New { .. })).count()
}

fn dispatch(ctx: GnoblinCtx) {
    // SAFETY: `dispatch` is a function of the core's table.
    unsafe { (api().dispatch)(ctx, 0) };
}

/// The commit of a reload: the old generation's code ends, the new generation takes over, the old one retires.
fn commit(old: GnoblinCtx, new: GnoblinCtx) {
    registry::run_shutdown_hooks(old);
    registry::activate(new).expect("the staged generation takes over");
    registry::finish_retire(old);
    dispatch(new);
}

#[test]
fn a_node_adopted_by_name_is_the_same_node_in_the_next_generation_and_nothing_is_made_twice() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    let (rc, badge) = adopt(first, "canvas", "badge", 0);
    assert_eq!(rc, abi::E_OK, "{}", last_error());
    let (rc, again) = adopt(first, "canvas", "badge", 0);
    assert_eq!(
        (rc, again),
        (abi::E_OK, badge),
        "asking twice in one generation is the same node"
    );
    rig.dispatch();
    assert_eq!(
        rig.comp.named("badge"),
        Some(badge),
        "the node carries its name in the compositor"
    );
    assert_eq!(new_nodes(&rig.comp), 1);

    let next = Gen::staged();
    let (rc, adopted) = adopt(next.ctx, "canvas", "badge", 0);
    assert_eq!(rc, abi::ADOPTED, "an earlier generation made it");
    assert_eq!(adopted, badge, "the same node, so the same id");
    commit(first, next.ctx);
    assert_eq!(new_nodes(&rig.comp), 1, "the reload made no second node");
    assert!(rig.comp.exists(badge), "and did not destroy the first");
    assert_eq!(rig.comp.destroys_of(badge), 0);

    // A third generation adopts it again, and a name of another class is another node.
    let third = Gen::staged();
    assert_eq!(adopt(third.ctx, "canvas", "badge", 0), (abi::ADOPTED, badge));
    let (rc, other) = adopt(third.ctx, "view", "badge", 0);
    assert_eq!(rc, abi::E_OK, "a name is per class: {}", last_error());
    assert_ne!(other, badge);
    commit(next.ctx, third.ctx);
    registry::finish_retire(third.ctx);
}

#[test]
fn a_node_the_shell_destroyed_is_made_again_under_its_name() {
    let _serial = serial();
    let rig = rig();
    let (rc, badge) = adopt(rig.ctx(), "canvas", "badge", 0);
    assert_eq!(rc, abi::E_OK);
    rig.dispatch();
    // SAFETY: `write_destroy` is a function of the core's table.
    assert_eq!(unsafe { (api().write_destroy)(rig.ctx(), badge) }, abi::E_OK);
    rig.dispatch();
    assert!(!rig.comp.exists(badge));
    // The compositor's batch reaches the shell's mirror on a later turn.
    // SAFETY: `node_exists` is a function of the core's table.
    rig.dispatch_until(|| unsafe { (api().node_exists)(rig.ctx(), badge) } == 0);

    let next = Gen::staged();
    let (rc, fresh) = adopt(next.ctx, "canvas", "badge", 0);
    assert_eq!(rc, abi::E_OK, "nothing is left to adopt: {}", last_error());
    assert_ne!(fresh, badge, "node ids are never reused");
    commit(rig.ctx(), next.ctx);
    registry::finish_retire(next.ctx);
}

#[test]
fn an_ephemeral_node_dies_with_its_generation_unless_the_next_one_adopts_it() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    let (_, toast) = adopt(first, "canvas", "toast", abi::NODE_FLAG_EPHEMERAL);
    let (_, overlay) = adopt(first, "canvas", "overlay", abi::NODE_FLAG_EPHEMERAL);
    let (_, badge) = adopt(first, "canvas", "badge", 0);
    let (_, plain) = {
        let mut node = 0;
        // SAFETY: `write_create` is a function of the core's table.
        let rc = unsafe { (api().write_create)(first, class("canvas"), &raw mut node) };
        (rc, node)
    };
    rig.dispatch();
    for node in [toast, overlay, badge, plain] {
        assert!(rig.comp.exists(node));
    }

    let next = Gen::staged();
    assert_eq!(
        adopt(next.ctx, "canvas", "overlay", abi::NODE_FLAG_EPHEMERAL),
        (abi::ADOPTED, overlay),
        "the new generation still wants the overlay"
    );
    // Nothing dies while the generation is only staged: a failed reload must leave the toast on the screen.
    dispatch(first);
    assert!(rig.comp.exists(toast), "staged is not committed");

    commit(first, next.ctx);
    assert!(
        !rig.comp.exists(toast),
        "an ephemeral node nobody adopted is destroyed at the takeover"
    );
    assert_eq!(rig.comp.destroys_of(toast), 1);
    assert!(rig.comp.exists(overlay), "an adopted one stays");
    assert!(
        rig.comp.exists(badge),
        "a node that is not ephemeral outlives its generation"
    );
    assert!(rig.comp.exists(plain), "so does one `write_create` made");
    assert!(
        rig.comp.exists(WINDOW),
        "the compositor's own nodes are not the reload's to touch"
    );

    // The overlay is ephemeral for the generation that adopted it: a third that does not ask for it ends it.
    let third = Gen::staged();
    commit(next.ctx, third.ctx);
    assert!(!rig.comp.exists(overlay));
    assert!(rig.comp.exists(badge));
    assert_eq!(rig.comp.destroys_of(WINDOW), 0);
    registry::finish_retire(third.ctx);
}

#[test]
fn a_failed_stage_gives_back_what_its_init_wrote_named_and_kept() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    let (_, badge) = adopt(first, "canvas", "badge", abi::NODE_FLAG_EPHEMERAL);
    assert_eq!(keep(first, "count", &[1], 1), abi::E_OK);
    rig.dispatch();
    let made = new_nodes(&rig.comp);

    let failing = Gen::staged();
    // Its `init` adopts the badge (claiming it), makes a node of its own, writes a property, and keeps state: then it fails.
    assert_eq!(adopt(failing.ctx, "canvas", "badge", 0), (abi::ADOPTED, badge));
    let (rc, ghost) = adopt(failing.ctx, "canvas", "ghost", 0);
    assert_eq!(rc, abi::E_OK);
    assert_eq!(set(failing.ctx, ghost, "canvas", "x", &double(7.0), 0.0), abi::E_OK);
    assert_eq!(keep(failing.ctx, "count", &[99], 2), abi::E_OK);
    assert_eq!(keep(failing.ctx, "extra", &[7, 7], 1), abi::E_OK);
    assert_eq!(
        kept(failing.ctx, "count"),
        Some((vec![99], 2)),
        "the staged generation reads what it wrote"
    );
    registry::finish_retire(failing.ctx);

    // The running generation serves on; the compositor was never told of the ghost; the state is as it was.
    rig.dispatch();
    assert_eq!(new_nodes(&rig.comp), made, "the abandoned stage made no node");
    assert_eq!(rig.comp.named("ghost"), None);
    assert_eq!(kept(first, "count"), Some((vec![1], 1)), "value and version come back");
    assert_eq!(kept(first, "extra"), None, "a key the stage added is gone");

    // The names are as they were: the ghost is new again, and the badge was not claimed by the failed generation, so a later
    // generation that does not adopt it ends it.
    let next = Gen::staged();
    assert_eq!(
        adopt(next.ctx, "canvas", "ghost", 0).0,
        abi::E_OK,
        "the ghost was forgotten"
    );
    commit(first, next.ctx);
    assert!(
        !rig.comp.exists(badge),
        "the failed generation's claim on the badge was undone, so it is unadopted and ephemeral"
    );
    assert_eq!(kept(next.ctx, "count"), Some((vec![1], 1)));
    registry::finish_retire(next.ctx);
}

#[test]
fn a_stage_that_flushed_before_it_failed_keeps_the_nodes_the_compositor_has_and_says_so() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    rig.dispatch();

    let failing = Gen::staged();
    let (rc, late) = adopt(failing.ctx, "canvas", "late", abi::NODE_FLAG_EPHEMERAL);
    assert_eq!(rc, abi::E_OK);
    // SAFETY: `flush` is a function of the core's table; no callback is given, so nothing is adopted.
    assert_eq!(
        unsafe { (api().flush)(failing.ctx, None, core::ptr::null_mut(), None, core::ptr::null_mut()) },
        abi::E_OK
    );
    assert!(rig.comp.exists(late), "the transaction went out");
    registry::finish_retire(failing.ctx);
    assert!(
        rig.comp.exists(late),
        "what the compositor holds cannot be taken back by a rollback"
    );

    // Its name stays, so the next generation finds the node instead of leaving it orphaned.
    let next = Gen::staged();
    assert_eq!(adopt(next.ctx, "canvas", "late", 0), (abi::ADOPTED, late));
    commit(first, next.ctx);
    assert!(rig.comp.exists(late));
    registry::finish_retire(next.ctx);
}

#[test]
fn state_is_kept_by_name_with_its_version_and_a_committed_stage_keeps_what_it_wrote() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    assert_eq!(keep(first, "counters", &[3, 0, 0, 0], 1), abi::E_OK);
    assert_eq!(keep(first, "layout", b"tall", 7), abi::E_OK);

    let next = Gen::staged();
    assert_eq!(
        kept(next.ctx, "counters"),
        Some((vec![3, 0, 0, 0], 1)),
        "the next generation reads it by name"
    );
    assert_eq!(
        kept(next.ctx, "layout"),
        Some((b"tall".to_vec(), 7)),
        "and the version its writer gave it, so it can tell a layout it understands from one it does not"
    );
    assert_eq!(kept(next.ctx, "never-kept"), None);
    // Version 2 of the counters: the new code migrated them.
    assert_eq!(keep(next.ctx, "counters", &[4, 0, 0, 0, 0, 0, 0, 0], 2), abi::E_OK);
    commit(first, next.ctx);
    assert_eq!(
        kept(next.ctx, "counters"),
        Some((vec![4, 0, 0, 0, 0, 0, 0, 0], 2)),
        "a committed stage's writes stay"
    );
    registry::finish_retire(next.ctx);
}

#[test]
fn state_the_old_generation_writes_as_it_ends_is_read_by_a_post_the_new_one_made_in_init() {
    let _serial = serial();
    let rig = rig();
    let first = rig.ctx();
    assert_eq!(keep(first, "count", &[1], 1), abi::E_OK);

    // The new generation's `init` ran before the old one ended, so it read the old value; the old generation's shutdown hook
    // then writes the final one. A post queued in `init` runs at the first dispatch after the takeover, which is the place to
    // read what the old generation wrote as it ended.
    let next = Gen::staged();
    assert_eq!(kept(next.ctx, "count"), Some((vec![1], 1)));
    registry::run_shutdown_hooks(first);
    assert_eq!(
        keep(first, "count", &[42], 1),
        abi::E_OK,
        "the old generation still writes while it ends"
    );
    registry::activate(next.ctx).unwrap();
    registry::finish_retire(first);
    assert_eq!(kept(next.ctx, "count"), Some((vec![42], 1)));
    registry::finish_retire(next.ctx);
}

#[test]
fn node_adopt_refuses_what_cannot_be_adopted_and_says_why() {
    let _serial = serial();
    let rig = rig();
    let ctx = rig.ctx();
    let (rc, _) = adopt(ctx, "canvas", "", 0);
    assert_eq!(rc, abi::E_ERR);
    assert_eq!(last_error(), "E_ERR: node_adopt: a node needs a name to be found by");
    let (rc, _) = adopt(ctx, "canvas", "x", 0x80);
    assert_eq!(rc, abi::E_ERR);
    assert_eq!(
        last_error(),
        "E_ERR: node_adopt: 0x80 holds flags this core does not know"
    );
    // A class of a namespace other than the tree has no nodes to adopt.
    let (rc, _) = adopt(ctx, "gnoblin.session", "x", 0);
    assert_eq!(rc, abi::E_NO_SUCH_MEMBER, "{}", last_error());
    assert_eq!(
        last_error(),
        "E_NO_SUCH_MEMBER: node_adopt: `gnoblin.session` is not a class of the tree"
    );

    let retired = Gen::staged();
    registry::finish_retire(retired.ctx);
    let (rc, _) = adopt(retired.ctx, "canvas", "x", 0);
    assert_eq!(rc, abi::E_RETIRED, "a retired generation adopts nothing");
    assert!(last_error().starts_with("E_RETIRED: generation "), "{}", last_error());
}
