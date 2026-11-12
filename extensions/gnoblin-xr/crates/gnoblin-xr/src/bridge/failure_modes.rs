//! The bridge when something goes wrong: the runtime fails or loses a swapchain, a pose is missing, and Views change
//! under a streaming session. Every case here must end with the frames still ending, or with the run ending cleanly:
//! swapchains destroyed, captures closed, and the `xr` seat the bridge made destroyed so the shell takes the room down.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use gnoblin_proto::tree::change;
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, read_frame, write_frame};
use xr_capture::views::{Fault as CaptureFault, FrameSource};
use xr_capture::wayland::Frame as Picture;
use xr_gpu::FakePresenter;
use xr_input::ControllerState;
use xr_layers::SubmittedLayer;
use xr_scene::{IDENTITY, Options, Pose, SkipReason};
use xr_session::frame_loop::FrameLoop;
use xr_session::testing::{Call, FakeRuntime};
use xr_tree::TreeClient;

use super::tests::{
    ATTACH_REQUEST_ID, Reports, ScriptedFrames, ahead, aiming, double, kinds, node_ref, picture, quads, received,
    room_of, script, snapshot_node, text,
};
use super::{BridgeError, Outcome, run};

/// A change that destroys `node`.
fn destroyed(node: u64) -> change::Properties {
    change::Properties {
        kind: "destroyed".into(),
        node,
        ..Default::default()
    }
}

/// A fake owner that can change the tree while the session streams.
struct LiveOwner {
    /// The transactions the bridge sent, in order.
    ops: Receiver<Vec<op::Properties>>,
    /// Pushes a batch of changes to the bridge. It returns once the batch is in the socket.
    push: Sender<Vec<change::Properties>>,
    pushed: Receiver<()>,
    /// The owner's end of the socket, to hang up with.
    socket: UnixStream,
}

/// Starts a fake owner that answers the attach with `nodes`, applies every transaction, and pushes the batches it is
/// given as `changes` events.
fn live_owner(nodes: Vec<snapshot_node::Properties>) -> (UnixStream, LiveOwner) {
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    let writer = Arc::new(Mutex::new(server.try_clone().expect("a second handle")));
    let socket = server.try_clone().expect("a third handle");
    let (sent_ops, ops) = mpsc::channel();
    let (push, batches) = mpsc::channel::<Vec<change::Properties>>();
    let (done, pushed) = mpsc::channel();

    let pusher = Arc::clone(&writer);
    thread::spawn(move || {
        let mut serial = 1;
        while let Ok(changes) = batches.recv() {
            serial += 1;
            let event = Envelope::event_message("tree", "changes", &ChangesPayload { serial, changes });
            let wrote = write_frame(&mut *pusher.lock().expect("the writer"), &event).is_ok();
            if !wrote || done.send(()).is_err() {
                return;
            }
        }
    });
    thread::spawn(move || {
        let attach = read_frame(&mut server).expect("read attach").expect("attach frame");
        assert_eq!(attach.request_id, ATTACH_REQUEST_ID);
        let attached = Envelope::reply_message(
            "tree",
            "attach",
            ATTACH_REQUEST_ID,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 1,
            },
        );
        let snapshot = Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 1,
                chunk: 0,
                last: true,
                nodes,
            },
        );
        {
            let mut out = writer.lock().expect("the writer");
            for event in [&attached, &snapshot] {
                if write_frame(&mut *out, event).is_err() {
                    return;
                }
            }
        }
        let mut serial = 1;
        while let Ok(Some(request)) = read_frame(&mut server) {
            let args = request.payload_as::<TransactArgs>().expect("transact args");
            serial += 1;
            let _ = sent_ops.send(args.ops);
            let reply = Envelope::reply_message(
                "tree",
                "transact",
                request.request_id,
                &ReplyPayload {
                    serial,
                    applied: true,
                    failed_op: -1,
                    code: String::new(),
                    removed: Vec::new(),
                },
            );
            let mut out = writer.lock().expect("the writer");
            if write_frame(&mut *out, &reply).is_err() || out.flush().is_err() {
                return;
            }
        }
    });
    (
        client,
        LiveOwner {
            ops,
            push,
            pushed,
            socket,
        },
    )
}

/// What the fake owner does on the pass it is set for.
enum Then {
    /// Push these changes to the bridge.
    Change(Vec<change::Properties>),
    /// Close the socket.
    HangUp,
}

/// Plays the pictures of a `ScriptedFrames` and, on one pass (counted from 0), changes the tree or hangs up.
struct ChangingTree {
    frames: ScriptedFrames,
    owner: LiveOwner,
    at: usize,
    then: Option<Then>,
    pass: usize,
}

impl ChangingTree {
    /// `then` happens during the pump of pass `at`: a change is read by the next pass, so the first frame that plans it
    /// is the one after.
    fn new(frames: ScriptedFrames, owner: LiveOwner, at: usize, then: Then) -> Self {
        Self {
            frames,
            owner,
            at,
            then: Some(then),
            pass: 0,
        }
    }

    /// A source that changes nothing.
    fn quiet(frames: ScriptedFrames, owner: LiveOwner) -> Self {
        Self {
            frames,
            owner,
            at: usize::MAX,
            then: None,
            pass: 0,
        }
    }
}

impl FrameSource for ChangingTree {
    fn pump(&mut self) -> Vec<CaptureFault> {
        if self.pass == self.at
            && let Some(then) = self.then.take()
        {
            match then {
                Then::Change(changes) => {
                    self.owner.push.send(changes).expect("the owner is listening");
                    self.owner.pushed.recv().expect("the owner wrote the batch");
                }
                Then::HangUp => self.owner.socket.shutdown(std::net::Shutdown::Both).expect("hang up"),
            }
        }
        self.pass += 1;
        self.frames.pump()
    }

    fn sync(&mut self, wanted: &BTreeSet<u64>) -> Vec<CaptureFault> {
        self.frames.sync(wanted)
    }

    fn latest(&self, node: u64) -> Option<&Picture> {
        self.frames.latest(node)
    }
}

/// Runs the bridge against `owner`'s tree, with `runtime`, until the script ends or the run fails.
fn run_with<S: FrameSource>(
    stream: UnixStream,
    runtime: FakeRuntime,
    captures: &mut S,
) -> (Result<Outcome, BridgeError>, FrameLoop<FakeRuntime>, Reports) {
    let mut tree = TreeClient::from_stream(stream).expect("attach");
    let mut frames = FrameLoop::new(runtime);
    let mut reports = Reports::default();
    let outcome = run(
        &mut tree,
        &mut frames,
        &Options::default(),
        None,
        captures,
        &mut FakePresenter::default(),
        |report| reports.push(report),
    );
    (outcome, frames, reports)
}

fn submitted_views(frames: &FrameLoop<FakeRuntime>) -> Vec<Vec<u64>> {
    frames
        .runtime()
        .submitted
        .iter()
        .map(|(_, layers)| {
            layers
                .iter()
                .map(|layer| match layer {
                    SubmittedLayer::Quad(quad) => quad.view,
                    SubmittedLayer::Projection(projection) => projection.view,
                })
                .collect()
        })
        .collect()
}

/// The ops that made or destroyed a seat: `(kind, node)`.
fn seat_ops(ops: &Receiver<Vec<op::Properties>>) -> Vec<(String, u64)> {
    ops.try_iter()
        .flatten()
        .filter_map(|op| match op.kind.as_str() {
            "new" if op.class == "seat" => Some(("new".to_owned(), op.id)),
            "destroy" => Some(("destroy".to_owned(), op.node)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_view_moved_while_streaming_moves_its_layer_and_keeps_its_swapchain_and_picture() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let still = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 7)))]]);
    // The change is pushed during the second pass, so the third frame is the first to plan the new place.
    let mut captures = ChangingTree::new(still, owner, 1, Then::Change(vec![double(2, "x", 500.0)]));
    let (outcome, frames, reports) = run_with(stream, FakeRuntime::with_graphics(script(3), 16), &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let xs: Vec<f64> = frames
        .runtime()
        .submitted
        .iter()
        .map(|(_, layers)| quads(layers)[0].pose.position[0])
        .collect();
    assert_eq!(xs, vec![0.0, 0.0, 0.5], "the layer follows the View");
    let chains: BTreeSet<_> = frames
        .runtime()
        .submitted
        .iter()
        .map(|(_, layers)| quads(layers)[0].swapchain)
        .collect();
    assert_eq!(chains.len(), 1, "one swapchain throughout");
    let creates = frames
        .runtime()
        .calls
        .iter()
        .filter(|call| matches!(call, Call::CreateSwapchain { .. }))
        .count();
    assert_eq!(creates, 1);
    assert!(reports.faults.iter().all(Vec::is_empty) && reports.notices.iter().all(Vec::is_empty));
}

#[test]
fn a_view_destroyed_while_streaming_loses_its_layer_and_its_swapchain() {
    let (stream, owner) = live_owner(room_of(&[(2, -500.0, -1000.0), (4, 500.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![
        (2, Some(picture(8, 4, 1, 1))),
        (4, Some(picture(8, 4, 1, 2))),
    ]]);
    let mut captures = ChangingTree::new(pictures, owner, 1, Then::Change(vec![destroyed(2)]));
    let (outcome, frames, reports) = run_with(stream, FakeRuntime::with_graphics(script(3), 16), &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(submitted_views(&frames), vec![vec![2, 4], vec![2, 4], vec![4]]);
    assert!(
        frames.runtime().swapchains.is_empty(),
        "every swapchain is gone at the end"
    );
    let destroys = frames
        .runtime()
        .calls
        .iter()
        .filter(|call| matches!(call, Call::DestroySwapchain(_)))
        .count();
    assert_eq!(destroys, 2, "View 2's swapchain went when it did, View 4's at the end");
    assert_eq!(reports.plans[2].layers.len(), 1);
    assert!(
        reports.plans[2].skipped.is_empty(),
        "a destroyed View is gone, not skipped"
    );
    assert_eq!(
        captures.frames.wanted.last(),
        Some(&BTreeSet::new()),
        "the end closes every capture"
    );
}

#[test]
fn two_views_at_the_same_pose_are_both_submitted_in_tree_order_and_the_last_takes_the_ray() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0), (4, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![
        (2, Some(picture(8, 4, 1, 1))),
        (4, Some(picture(8, 4, 1, 2))),
    ]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let mut runtime = FakeRuntime::with_graphics(script(2), 16);
    runtime.controller = aiming(ahead(0.0));
    let (outcome, frames, _) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(submitted_views(&frames), vec![vec![2, 4], vec![2, 4]]);
    // View 2 shows window 3 and View 4 shows window 5: the View drawn last is on top, so the ray reaches window 5.
    let sent = received(&captures.owner.ops);
    assert_eq!(kinds(&sent)[0], ("motion", 5), "{:?}", kinds(&sent));
    assert!(sent.iter().all(|op| op.node == 5), "nothing reached the hidden window");
}

#[test]
fn a_zero_size_view_is_skipped_reported_and_never_captured_and_its_neighbour_is_shown() {
    let mut nodes = room_of(&[(2, -500.0, -1000.0), (4, 500.0, -1000.0)]);
    // View 2 is zero wide.
    for node in &mut nodes {
        if node.id == 2 {
            node.props.retain(|prop| prop.key != "width");
            node.props.push(double(2, "width", 0.0));
        }
    }
    let (stream, owner) = live_owner(nodes);
    let pictures = ScriptedFrames::new(vec![vec![
        (2, Some(picture(8, 4, 1, 1))),
        (4, Some(picture(8, 4, 1, 2))),
    ]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let (outcome, frames, reports) = run_with(stream, FakeRuntime::with_graphics(script(2), 16), &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(submitted_views(&frames), vec![vec![4], vec![4]]);
    for plan in &reports.plans {
        assert_eq!(
            plan.skipped.iter().map(|s| (s.id, s.reason)).collect::<Vec<_>>(),
            vec![(2, SkipReason::InvalidSize)]
        );
    }
    assert_eq!(
        captures.frames.wanted[0],
        BTreeSet::from([4]),
        "a skipped View is not captured"
    );
    assert!(reports.faults.iter().all(Vec::is_empty));
}

#[test]
fn a_missing_head_pose_and_an_untracked_controller_do_not_change_the_layers_or_stop_the_frames() {
    let layers_with = |head: Option<Pose>| {
        let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
        let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
        let mut captures = ChangingTree::quiet(pictures, owner);
        let mut runtime = FakeRuntime::with_graphics(script(3), 16);
        runtime.head = head;
        runtime.controller = ControllerState::default();
        let (outcome, frames, reports) = run_with(stream, runtime, &mut captures);
        assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
        assert_eq!(reports.plans.len(), 3, "every frame ran");
        frames.runtime().submitted.clone()
    };
    let lost = layers_with(None);
    let found = layers_with(Some(Pose {
        position: [4.0, 1.7, -3.0],
        orientation: IDENTITY,
    }));
    assert_eq!(lost, found, "the head pose is a measurement the layers do not read");
    assert_eq!(lost.len(), 3);
}

#[test]
fn a_lost_swapchain_is_remade_and_the_view_comes_back_the_next_frame() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let mut runtime = FakeRuntime::with_graphics(script(3), 16);
    // The first swapchain the bridge makes is the runtime's id 1, and the runtime has lost it.
    runtime.lost.insert(1);
    let (outcome, frames, reports) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(submitted_views(&frames), vec![vec![], vec![2], vec![2]]);
    assert_eq!(
        reports.faults[0].len(),
        1,
        "the first frame reports the refusal: {:?}",
        reports.faults[0]
    );
    assert!(reports.faults[1].is_empty() && reports.faults[2].is_empty());
    let [first, ..] = frames.runtime().submitted[1].1.as_slice() else {
        panic!("a layer in the second frame");
    };
    assert!(matches!(first, SubmittedLayer::Quad(quad) if quad.swapchain == xr_layers::SwapchainId(2)));
    assert!(frames.runtime().swapchains.is_empty());
}

/// The index (1-based) of the `n`th `EndFrame` call of a clean run, to fail on.
fn nth_end_frame(frames: u32, n: usize) -> usize {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let (_, clean, _) = run_with(stream, FakeRuntime::with_graphics(script(frames), 16), &mut captures);
    clean
        .runtime()
        .calls
        .iter()
        .enumerate()
        .filter(|(_, call)| matches!(call, Call::EndFrame(_)))
        .map(|(index, _)| index + 1)
        .nth(n - 1)
        .expect("the clean run ended that many frames")
}

#[test]
fn a_runtime_that_fails_mid_session_ends_the_run_with_a_runtime_error_and_cleans_up() {
    let fail_at = nth_end_frame(4, 2);
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let mut runtime = FakeRuntime::with_graphics(script(4), 16);
    runtime.fail_on_call = Some(fail_at);
    runtime.controller = aiming(ahead(0.0));
    let (outcome, frames, reports) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Err(BridgeError::Runtime(_))), "{outcome:?}");
    assert_eq!(
        reports.plans.len(),
        1,
        "the frame whose end failed was not reported: only the first one was"
    );
    assert!(frames.runtime().swapchains.is_empty(), "the swapchains are destroyed");
    assert_eq!(
        captures.frames.wanted.last(),
        Some(&BTreeSet::new()),
        "the captures are closed"
    );
    let seats = seat_ops(&captures.owner.ops);
    let [(made, seat), (gone, destroyed_seat)] = seats.as_slice() else {
        panic!("the seat is made and destroyed: {seats:?}");
    };
    assert_eq!((made.as_str(), gone.as_str()), ("new", "destroy"));
    assert_eq!(
        seat, destroyed_seat,
        "the seat the bridge made is the seat it destroyed"
    );
}

#[test]
fn a_session_that_ends_destroys_the_seat_it_made_and_leaves_the_window_the_ray_was_over() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let mut runtime = FakeRuntime::with_graphics(script(2), 16);
    runtime.controller = aiming(ahead(0.0));
    let (outcome, _, _) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let all: Vec<op::Properties> = captures.owner.ops.try_iter().flatten().collect();
    let order: Vec<&str> = all
        .iter()
        .map(|op| {
            if op.kind == "input" {
                op.input_kind.as_str()
            } else {
                op.kind.as_str()
            }
        })
        .collect();
    assert_eq!(order, ["new", "set", "motion", "leave", "destroy"], "{order:?}");
}

#[test]
fn a_seat_that_was_already_there_is_not_destroyed_when_the_session_ends() {
    let mut nodes = room_of(&[(2, 0.0, -1000.0)]);
    nodes.push(snapshot_node(9, 0, 1, "seat", vec![text(9, "name", "xr")]));
    let (stream, owner) = live_owner(nodes);
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::quiet(pictures, owner);
    let mut runtime = FakeRuntime::with_graphics(script(2), 16);
    runtime.controller = aiming(ahead(0.0));
    let (outcome, _, _) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(
        seat_ops(&captures.owner.ops),
        Vec::new(),
        "it made no seat and destroyed none"
    );
}

#[test]
fn an_owner_that_goes_during_the_session_is_the_end_of_the_run_not_a_failure() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::new(pictures, owner, 2, Then::HangUp);
    let mut runtime = FakeRuntime::with_graphics(script(50), 16);
    runtime.controller = aiming(ahead(0.0));
    let (outcome, frames, reports) = run_with(stream, runtime, &mut captures);

    assert!(matches!(outcome, Ok(Outcome::OwnerClosed)), "{outcome:?}");
    assert!(
        !reports.plans.is_empty() && reports.plans.len() < 50,
        "the run stopped when the owner went"
    );
    assert!(
        frames.runtime().swapchains.is_empty(),
        "the swapchains are still destroyed"
    );
}

/// Nodes that name a View's source and camera must survive a model that lacks the source: the View has nothing to show.
#[test]
fn a_view_whose_source_is_cleared_while_streaming_leaves_the_plan_and_the_swapchain() {
    let (stream, owner) = live_owner(room_of(&[(2, 0.0, -1000.0)]));
    let pictures = ScriptedFrames::new(vec![vec![(2, Some(picture(8, 4, 1, 1)))]]);
    let mut captures = ChangingTree::new(pictures, owner, 1, Then::Change(vec![node_ref(2, "source", 0)]));
    let (outcome, frames, _) = run_with(stream, FakeRuntime::with_graphics(script(3), 16), &mut captures);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(submitted_views(&frames), vec![vec![2], vec![2], vec![]]);
    assert!(frames.runtime().swapchains.is_empty());
}
