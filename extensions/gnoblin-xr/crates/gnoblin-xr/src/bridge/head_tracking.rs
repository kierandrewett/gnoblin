//! The bridge following the head: the head pose into the head camera, the eyes a layer declares, the layer budget
//! and the order Views are dropped in. Everything runs against a fake owner that applies the extension's `set` ops to
//! its tree and a stand-in compositor that draws a new picture of every View after each pose it saw written, so the
//! whole loop of write, show and draw is the bridge's own. Nothing here has met a real compositor or a headset.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gnoblin_proto::tree::change;
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, read_frame, write_frame};
use xr_capture::views::{Fault as CaptureFault, FrameSource};
use xr_capture::wayland::Frame as Picture;
use xr_gpu::FakePresenter;
use xr_layers::SubmittedLayer;
use xr_scene::{IDENTITY, Options, Pose};
use xr_session::frame_loop::FrameLoop;
use xr_session::testing::FakeRuntime;
use xr_tree::TreeClient;

use super::tests::{
    ATTACH_REQUEST_ID, Reports, double, node_ref, picture, room_of, run_frames, script, snapshot_node as node, text,
};
use super::{BridgeError, Outcome, run};
use crate::head::Status;

/// The head camera's node id in these rooms.
const CAMERA: u64 = 30;
/// How long one pass of the stand-in compositor takes: longer than the tracker's interval between writes.
const PASS: Duration = Duration::from_millis(20);

/// The fake owner: it keeps the ops the bridge sends, and answers a `set`-only transaction with the change that applies
/// it. `refuse` makes it refuse every `set` of the camera with that code instead.
struct Owner {
    ops: Receiver<Vec<op::Properties>>,
    push: Sender<Vec<change::Properties>>,
    pushed: Receiver<()>,
}

fn camera_set(op: &op::Properties) -> bool {
    op.kind == "set" && op.node == CAMERA
}

fn echo_owner(nodes: Vec<snapshot_node::Properties>, refuse: Option<&'static str>) -> (UnixStream, Owner) {
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    let writer = Arc::new(Mutex::new(server.try_clone().expect("a second handle")));
    let serial = Arc::new(AtomicU64::new(1));
    let (sent_ops, ops) = mpsc::channel();
    let (push, batches) = mpsc::channel::<Vec<change::Properties>>();
    let (done, pushed) = mpsc::channel();
    // The nodes the owner has destroyed: a write to one is refused `unknown-node`, as the compositor refuses it.
    let gone: Arc<Mutex<Vec<u64>>> = Arc::default();

    let pusher = Arc::clone(&writer);
    let pushed_serial = Arc::clone(&serial);
    let pushed_gone = Arc::clone(&gone);
    thread::spawn(move || {
        while let Ok(changes) = batches.recv() {
            let serial = pushed_serial.fetch_add(1, Ordering::SeqCst) + 1;
            pushed_gone.lock().expect("the list").extend(
                changes
                    .iter()
                    .filter(|change| change.kind == "destroyed")
                    .map(|change| change.node),
            );
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
        while let Ok(Some(request)) = read_frame(&mut server) {
            let args = request.payload_as::<TransactArgs>().expect("transact args");
            let sets_only = !args.ops.is_empty() && args.ops.iter().all(|op| op.kind == "set");
            let destroyed = args
                .ops
                .iter()
                .any(|op| gone.lock().expect("the list").contains(&op.node));
            let refused = if destroyed {
                Some("unknown-node")
            } else {
                refuse.filter(|_| args.ops.iter().any(camera_set))
            };
            let echo: Vec<change::Properties> = args
                .ops
                .iter()
                .filter(|_| sets_only && refused.is_none())
                .map(|op| change::Properties {
                    kind: "set".to_owned(),
                    node: op.node,
                    key: op.key.clone(),
                    value_kind: op.value_kind.clone(),
                    value_double: op.value_double,
                    value_list: op.value_list.clone(),
                    value_uint: op.value_uint,
                    ..Default::default()
                })
                .collect();
            let _ = sent_ops.send(args.ops);
            let mut out = writer.lock().expect("the writer");
            // The change comes first, as the owner publishes a batch before it answers: the bridge's mirror shows the
            // write by the time the transaction returns.
            if !echo.is_empty() {
                let serial = serial.fetch_add(1, Ordering::SeqCst) + 1;
                let event = Envelope::event_message("tree", "changes", &ChangesPayload { serial, changes: echo });
                if write_frame(&mut *out, &event).is_err() {
                    return;
                }
            }
            let reply = Envelope::reply_message(
                "tree",
                "transact",
                request.request_id,
                &ReplyPayload {
                    serial: serial.load(Ordering::SeqCst),
                    applied: refused.is_none(),
                    failed_op: if refused.is_some() { 0 } else { -1 },
                    code: refused.unwrap_or_default().to_owned(),
                    removed: Vec::new(),
                },
            );
            if write_frame(&mut *out, &reply).is_err() || out.flush().is_err() {
                return;
            }
        }
    });
    (client, Owner { ops, push, pushed })
}

/// A stand-in for the compositor and its capture: every pass takes [`PASS`], and a pose written to the camera in the
/// pass before makes a new picture of every View. It keeps every op the bridge sent.
struct Compositor {
    owner: Owner,
    views: Vec<u64>,
    serial: u64,
    current: BTreeMap<u64, Picture>,
    sent: Vec<op::Properties>,
    /// A batch of changes the owner pushes during the pass counted from 0.
    then: Option<(usize, Vec<change::Properties>)>,
    pass: usize,
}

impl Compositor {
    fn new(owner: Owner, views: &[u64]) -> Self {
        Self {
            owner,
            views: views.to_vec(),
            serial: 0,
            current: BTreeMap::new(),
            sent: Vec::new(),
            then: None,
            pass: 0,
        }
    }

    fn pushing(mut self, at: usize, changes: Vec<change::Properties>) -> Self {
        self.then = Some((at, changes));
        self
    }

    fn draw(&mut self) {
        self.serial += 1;
        let serial = self.serial;
        for view in &self.views {
            self.current
                .insert(*view, picture(8, 4, serial, u8::try_from(serial % 200).unwrap_or(0)));
        }
    }

    /// The values the bridge wrote to `key` of the camera, in order.
    fn camera_values(&self, key: &str) -> Vec<f64> {
        self.sent
            .iter()
            .filter(|op| camera_set(op) && op.key == key)
            .map(|op| op.value_double)
            .collect()
    }

    fn budget_writes(&self) -> Vec<u64> {
        self.sent
            .iter()
            .filter(|op| camera_set(op) && op.key == "layer_budget")
            .map(|op| op.value_uint)
            .collect()
    }
}

impl FrameSource for Compositor {
    fn pump(&mut self) -> Vec<CaptureFault> {
        thread::sleep(PASS);
        let before = self.sent.len();
        self.sent.extend(self.owner.ops.try_iter().flatten());
        let posed = self.sent[before..].iter().any(|op| op.kind == "set" && op.key == "x");
        if self.current.is_empty() || posed {
            self.draw();
        }
        let pass = self.pass;
        if let Some((_, changes)) = self.then.take_if(|(at, _)| *at == pass) {
            self.owner.push.send(changes).expect("the owner is listening");
            self.owner.pushed.recv().expect("the owner wrote the batch");
        }
        self.pass += 1;
        Vec::new()
    }

    fn sync(&mut self, _wanted: &std::collections::BTreeSet<u64>) -> Vec<CaptureFault> {
        Vec::new()
    }

    fn latest(&self, node: u64) -> Option<&Picture> {
        self.current.get(&node)
    }
}

/// Runs the bridge against `stream` with `runtime` until the script ends or the run fails.
fn run_head(
    stream: UnixStream,
    runtime: FakeRuntime,
    captures: &mut Compositor,
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
    // The bridge has ended: pick up whatever it sent last.
    captures.sent.extend(captures.owner.ops.try_iter().flatten());
    (outcome, frames, reports)
}

/// A room of Views, `ids` in a row 1 m ahead, all drawn through the stereo camera named `head`.
fn stereo_room(ids: &[u64]) -> Vec<snapshot_node::Properties> {
    let views: Vec<(u64, f64, f64)> = ids.iter().map(|&id| (id, 0.0, -1000.0)).collect();
    let mut nodes = room_of(&views);
    nodes.push(node(
        CAMERA,
        1,
        0,
        "camera",
        vec![
            text(CAMERA, "name", "head"),
            text(CAMERA, "projection", "stereo"),
            double(CAMERA, "metres", 0.001),
        ],
    ));
    for view in nodes.iter_mut().filter(|node| node.class == "view") {
        let id = view.id;
        view.props.push(node_ref(id, "camera", CAMERA));
    }
    nodes
}

fn head_at(x: f64) -> Pose {
    Pose {
        position: [x, 1.6, 0.0],
        orientation: IDENTITY,
    }
}

fn runtime_with(frames: u32, head: impl IntoIterator<Item = Option<Pose>>, layers: u32) -> FakeRuntime {
    let mut runtime = FakeRuntime::with_graphics(script(frames), layers);
    runtime.head_script = head.into_iter().collect();
    runtime
}

/// The mean of the two eyes' x of each submitted projection layer, in metres.
fn eye_middles(frames: &FrameLoop<FakeRuntime>) -> Vec<Vec<f64>> {
    frames
        .runtime()
        .submitted
        .iter()
        .map(|(_, layers)| {
            layers
                .iter()
                .filter_map(|layer| match layer {
                    SubmittedLayer::Projection(projection) => {
                        Some((projection.eyes[0].pose.position[0] + projection.eyes[1].pose.position[0]) / 2.0)
                    }
                    SubmittedLayer::Quad(_) => None,
                })
                .collect()
        })
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_head_locked_stereo_view_follows_the_head_and_each_layer_states_the_eyes_its_picture_was_drawn_from() {
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]);
    let poses = (1..=8).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, frames, reports) = run_head(stream, runtime_with(8, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let middles = eye_middles(&frames);
    assert_eq!(middles.len(), 8, "every frame submitted its layer");
    // Frame 1's picture was drawn before any pose was written: from the camera as the tree holds it, at the origin.
    assert_eq!(middles[0].len(), 1);
    assert!(near(middles[0][0], 0.0), "{middles:?}");
    // The write of each pass is drawn in the next pass's picture, and the layer states exactly that pose.
    for (frame, eyes) in middles.iter().enumerate().skip(1) {
        assert_eq!(eyes.len(), 1);
        let drawn_from = 0.1 * frame as f64;
        assert!(
            near(eyes[0], drawn_from),
            "frame {}: eyes at {} for a picture drawn from {drawn_from}",
            frame + 1,
            eyes[0]
        );
    }
    let xs = compositor.camera_values("x");
    assert_eq!(xs.len(), 8, "one write a pass: {xs:?}");
    for (index, x) in xs.iter().enumerate() {
        assert!((x - 100.0 * (index + 1) as f64).abs() < 1e-6, "{xs:?}");
    }
    for key in ["y", "z"] {
        assert_eq!(
            compositor.camera_values(key).len(),
            8,
            "{key} goes with x in one transaction"
        );
    }
    assert!(reports.faults.iter().all(Vec::is_empty));
}

#[test]
fn two_head_locked_views_share_one_write_a_pass_and_state_the_same_eyes() {
    let (stream, owner) = echo_owner(stereo_room(&[2, 4]), None);
    let mut compositor = Compositor::new(owner, &[2, 4]);
    let poses = (1..=5).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, frames, _) = run_head(stream, runtime_with(5, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(
        compositor.camera_values("x").len(),
        5,
        "one write a pass, not one a View"
    );
    for (frame, middles) in eye_middles(&frames).iter().enumerate() {
        assert_eq!(middles.len(), 2, "both Views are projection layers");
        assert!(near(middles[0], middles[1]), "frame {frame}: {middles:?}");
    }
}

#[test]
fn a_head_the_runtime_loses_for_a_few_frames_writes_nothing_then_follows_again() {
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]);
    let poses = [Some(head_at(0.1)), None, None, Some(head_at(0.4))];
    let (outcome, _, reports) = run_head(stream, runtime_with(4, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let xs = compositor.camera_values("x");
    assert_eq!(xs.len(), 2, "{xs:?}");
    assert!(near(xs[0], 100.0) && (xs[1] - 400.0).abs() < 1e-6, "{xs:?}");
    assert_eq!(reports.heads[1], Status::Untracked);
    assert_eq!(reports.heads[2], Status::Untracked);
}

#[test]
fn a_pose_that_is_not_finite_is_never_sent_and_the_frames_go_on() {
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]);
    let broken = [
        Pose {
            position: [f64::NAN, 1.6, 0.0],
            orientation: IDENTITY,
        },
        Pose {
            position: [0.0, f64::INFINITY, 0.0],
            orientation: IDENTITY,
        },
        Pose {
            position: [0.0; 3],
            orientation: [0.0; 4],
        },
    ];
    let (outcome, frames, reports) = run_head(stream, runtime_with(3, broken.map(Some), 16), &mut compositor);
    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert!(compositor.camera_values("x").is_empty(), "nothing reached the camera");
    assert!(
        reports.heads.iter().all(|status| *status == Status::Unusable),
        "{:?}",
        reports.heads
    );
    assert_eq!(frames.runtime().submitted.len(), 3, "every frame still ended");
}

#[test]
fn an_owner_that_refuses_the_pose_is_asked_once_and_the_frames_go_on() {
    let (stream, owner) = echo_owner(stereo_room(&[2]), Some("rate-limited"));
    let mut compositor = Compositor::new(owner, &[2]);
    let poses = (1..=8).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, frames, reports) = run_head(stream, runtime_with(8, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(
        compositor.camera_values("x").len(),
        1,
        "eight passes of 20 ms are inside the one second back-off"
    );
    assert_eq!(
        compositor.budget_writes().len(),
        1,
        "and so are they inside the budget's retry time"
    );
    assert_eq!(frames.runtime().submitted.len(), 8);
    assert!(
        reports.heads[1..].iter().all(|status| *status == Status::Refused),
        "{:?}",
        reports.heads
    );
}

#[test]
fn the_camera_destroyed_while_views_are_head_locked_stops_the_writes_and_the_frames_go_on() {
    let destroyed = change::Properties {
        kind: "destroyed".into(),
        node: CAMERA,
        ..Default::default()
    };
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]).pushing(2, vec![destroyed]);
    let poses = (1..=8).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, frames, reports) = run_head(stream, runtime_with(8, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let writes = compositor.camera_values("x").len();
    assert!((2..=4).contains(&writes), "writes stop when the camera goes: {writes}");
    assert_eq!(reports.heads.last(), Some(&Status::NoCamera));
    assert_eq!(frames.runtime().submitted.len(), 8, "every frame still ended");
    // The View's camera is gone, so it is a flat quad again, still shown.
    let last = &frames.runtime().submitted.last().expect("a frame").1;
    assert!(matches!(last.as_slice(), [SubmittedLayer::Quad(_)]), "{last:?}");
}

#[test]
fn a_camera_the_shell_makes_again_is_followed_from_the_first_pass() {
    let created = |node: u64, class: &str| change::Properties {
        kind: "created".into(),
        node,
        class: class.into(),
        ..Default::default()
    };
    let set_text = |key: &str, value: &str| change::Properties {
        kind: "set".into(),
        node: 31,
        key: key.into(),
        value_kind: "text".into(),
        value_text: value.into(),
        ..Default::default()
    };
    let rebuilt = vec![
        change::Properties {
            kind: "destroyed".into(),
            node: CAMERA,
            ..Default::default()
        },
        created(31, "camera"),
        change::Properties {
            kind: "parented".into(),
            node: 31,
            parent: 1,
            index: 0,
            ..Default::default()
        },
        set_text("name", "head"),
        set_text("projection", "stereo"),
        change::Properties {
            kind: "set".into(),
            node: 2,
            key: "camera".into(),
            value_kind: "node".into(),
            value_node: 31,
            ..Default::default()
        },
    ];
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]).pushing(2, rebuilt);
    let poses = (1..=8).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, frames, _) = run_head(stream, runtime_with(8, poses, 16), &mut compositor);

    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    let old: Vec<&op::Properties> = compositor
        .sent
        .iter()
        .filter(|op| op.kind == "set" && op.key == "x" && op.node == CAMERA)
        .collect();
    let new: Vec<&op::Properties> = compositor
        .sent
        .iter()
        .filter(|op| op.kind == "set" && op.key == "x" && op.node == 31)
        .collect();
    assert!(
        !old.is_empty() && !new.is_empty(),
        "old {} new {}",
        old.len(),
        new.len()
    );
    let order: Vec<u64> = compositor
        .sent
        .iter()
        .filter(|op| op.kind == "set" && op.key == "x")
        .map(|op| op.node)
        .collect();
    let switch = order
        .iter()
        .position(|node| *node == 31)
        .expect("the new camera is written");
    assert!(order[..switch].iter().all(|node| *node == CAMERA));
    assert!(
        order[switch..].iter().all(|node| *node == 31),
        "never the old one again: {order:?}"
    );
    // The layers are projection layers again once the new camera is the View's.
    let last = &frames.runtime().submitted.last().expect("a frame").1;
    assert!(matches!(last.as_slice(), [SubmittedLayer::Projection(_)]), "{last:?}");
}

#[test]
fn nothing_is_written_while_no_view_draws_through_the_head_camera() {
    // The camera is there, and the Views are flat: nobody needs the head pose.
    let mut nodes = room_of(&[(2, 0.0, -1000.0)]);
    nodes.push(node(
        CAMERA,
        1,
        0,
        "camera",
        vec![text(CAMERA, "name", "head"), text(CAMERA, "projection", "stereo")],
    ));
    let (stream, owner) = echo_owner(nodes, None);
    let mut compositor = Compositor::new(owner, &[2]);
    let poses = (1..=4).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, _, reports) = run_head(stream, runtime_with(4, poses, 16), &mut compositor);
    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert!(compositor.camera_values("x").is_empty());
    assert!(
        reports.heads.iter().all(|status| *status == Status::NoViews),
        "{:?}",
        reports.heads
    );
}

#[test]
fn the_runtimes_layer_limit_is_written_to_the_head_camera_once() {
    let (stream, owner) = echo_owner(stereo_room(&[2]), None);
    let mut compositor = Compositor::new(owner, &[2]);
    let poses = (1..=6).map(|k| Some(head_at(0.1 * f64::from(k))));
    let (outcome, _, reports) = run_head(stream, runtime_with(6, poses, 12), &mut compositor);
    assert!(matches!(outcome, Ok(Outcome::SessionExited)), "{outcome:?}");
    assert_eq!(
        compositor.budget_writes(),
        vec![12],
        "written when the camera does not hold it, and not again once it does"
    );
    assert!(reports.budgets.iter().all(|budget| *budget == 12));
}

#[test]
fn a_session_with_no_head_camera_writes_no_budget() {
    let (frames, _, _, reports) = run_frames(
        room_of(&[(2, 0.0, -1000.0)]),
        FakeRuntime::with_graphics(script(2), 16),
        vec![vec![(2, Some(picture(8, 4, 1, 1)))]],
    );
    assert_eq!(frames.runtime().submitted.len(), 2);
    assert_eq!(reports.heads, vec![Status::NoCamera, Status::NoCamera]);
}

#[test]
fn views_over_the_budget_are_dropped_focused_first_then_nearest() {
    // Views 2, 4 and 6 at 1, 3 and 2 m. The budget is two. Window 5 (View 4's) has the keyboard focus on a seat.
    let mut nodes = room_of(&[(2, 0.0, -1000.0), (4, 0.0, -3000.0), (6, 0.0, -2000.0)]);
    nodes.push(node(
        9,
        0,
        1,
        "seat",
        vec![text(9, "name", "main"), node_ref(9, "focus", 5)],
    ));
    let steps = vec![vec![
        (2, Some(picture(8, 4, 1, 1))),
        (4, Some(picture(8, 4, 1, 2))),
        (6, Some(picture(8, 4, 1, 3))),
    ]];
    let (frames, _, _, reports) = run_frames(nodes, FakeRuntime::with_graphics(script(1), 2), steps);

    let (_, layers) = &frames.runtime().submitted[0];
    let kept: Vec<u64> = layers
        .iter()
        .map(|layer| match layer {
            SubmittedLayer::Quad(quad) => quad.view,
            SubmittedLayer::Projection(projection) => projection.view,
        })
        .collect();
    assert_eq!(
        kept,
        vec![2, 4],
        "the focused far View and the nearest of the rest, in plan order"
    );
    assert_eq!(reports.dropped, vec![1]);
    assert_eq!(reports.budgets, vec![2]);
}

#[test]
fn the_view_that_has_the_focus_survives_a_budget_of_one() {
    let mut nodes = room_of(&[(2, 0.0, -1000.0), (4, 0.0, -3000.0)]);
    nodes.push(node(
        9,
        0,
        1,
        "seat",
        vec![text(9, "name", "main"), node_ref(9, "focus", 5)],
    ));
    let steps = vec![vec![(2, Some(picture(8, 4, 1, 1))), (4, Some(picture(8, 4, 1, 2)))]];
    let (frames, _, _, _) = run_frames(nodes, FakeRuntime::with_graphics(script(1), 1), steps);
    let (_, layers) = &frames.runtime().submitted[0];
    assert!(
        matches!(layers.as_slice(), [SubmittedLayer::Quad(quad)] if quad.view == 4),
        "{layers:?}"
    );
}
