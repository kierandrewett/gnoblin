//! The bridge: keeps the owner tree model current, plans every frame from it, and shows the Views in the headset.
//!
//! Each pass reads what the owner has sent, then runs one tick of the frame loop. A frame plans the `room` subtree
//! against the head pose the runtime located, keeps one capture and one swapchain per planned View, copies each picture
//! that changed into its swapchain, and ends the frame with the layers `xr-layers` builds from the plan and the
//! swapchains, in plan order. A frame the runtime says not to render ends with no layers.

use std::collections::BTreeSet;
use std::fmt;
use std::thread;
use std::time::Duration;

use xr_capture::views::FrameSource;
use xr_gpu::Presenter;
use xr_layers::{Limits, Notice, compose};
use xr_scene::{IDENTITY, Layer, Options, Plan, Pose, plan};
use xr_session::frame_loop::{FrameLoop, Tick};
use xr_session::runtime::{Runtime, RuntimeError};
use xr_tree::{TreeClient, TreeError};

use crate::dump::Dump;
use crate::pointer::Pointer;
use crate::surfaces::{Fault, Surfaces};

/// How long the bridge sleeps when the session is not running a frame.
const IDLE_INTERVAL: Duration = Duration::from_millis(10);

/// Why the bridge stopped with an error.
#[derive(Debug)]
pub enum BridgeError {
    /// The owner socket failed or sent something the model cannot apply.
    Tree(TreeError),
    /// The OpenXR runtime failed.
    Runtime(RuntimeError),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tree(error) => write!(f, "owner socket: {error}"),
            Self::Runtime(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<TreeError> for BridgeError {
    fn from(error: TreeError) -> Self {
        Self::Tree(error)
    }
}

impl From<RuntimeError> for BridgeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

/// Why [`run`] returned without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The runtime asked the application to exit.
    SessionExited,
    /// The owner closed the socket, so the tree can no longer change.
    OwnerClosed,
}

/// What one frame did, for the caller to log.
pub struct FrameReport<'a> {
    /// The plan of the frame.
    pub plan: &'a Plan,
    /// Views the layer step dropped or degraded: over the budget, with no swapchain yet, and so on.
    pub notices: &'a [Notice],
    /// Views that could not be shown because a swapchain or a copy failed.
    pub faults: &'a [Fault],
    /// How many layers the frame ended with.
    pub submitted: usize,
}

/// Runs until the session exits or the owner goes away, calling `on_frame` with the report of every frame.
///
/// With a `dump`, each picture copied into a swapchain is also written to its directory.
///
/// The swapchains are destroyed before it returns.
///
/// # Errors
///
/// The owner socket fails, or the runtime fails.
pub fn run<R: Runtime, P: Presenter, S: FrameSource>(
    tree: &mut TreeClient,
    frames: &mut FrameLoop<R>,
    opts: &Options,
    dump: Option<Dump>,
    captures: &mut S,
    presenter: &mut P,
    on_frame: impl FnMut(&FrameReport<'_>),
) -> Result<Outcome, BridgeError> {
    let mut surfaces = Surfaces::new(dump);
    let outcome = run_loop(tree, frames, opts, captures, presenter, &mut surfaces, on_frame);
    // Best effort: the session is ending, and a failure here has nothing left to protect.
    for fault in surfaces.clear(frames.runtime_mut()) {
        eprintln!("gnoblin-xr: {fault}");
    }
    outcome
}

fn run_loop<R: Runtime, P: Presenter, S: FrameSource>(
    tree: &mut TreeClient,
    frames: &mut FrameLoop<R>,
    opts: &Options,
    captures: &mut S,
    presenter: &mut P,
    surfaces: &mut Surfaces,
    mut on_frame: impl FnMut(&FrameReport<'_>),
) -> Result<Outcome, BridgeError> {
    let mut pointer = Pointer::default();
    let capabilities = frames.runtime().capabilities();
    let limits = Limits {
        max_layer_count: capabilities.max_layer_count,
        reserved: 0,
        color_scale_bias: capabilities.color_scale_bias,
    };
    loop {
        match tree.poll() {
            Ok(_) => {}
            Err(TreeError::Closed) => return Ok(Outcome::OwnerClosed),
            Err(error) => return Err(error.into()),
        }
        for (node, error) in captures.pump() {
            eprintln!("gnoblin-xr: view capture {node:?} failed: {error}");
        }
        match frames.start()? {
            Tick::Idle => thread::sleep(IDLE_INTERVAL),
            Tick::Frame(frame) => {
                let head = frame.head.unwrap_or(Pose {
                    position: [0.0; 3],
                    orientation: IDENTITY,
                });
                let planned = plan(tree.model(), &head, opts);
                let controller = frames.locate_controller(frame.display_time)?;
                pointer.update(tree, &planned.layers, &controller, frame.display_time)?;
                let wanted = view_nodes(&planned.layers);
                for (node, error) in captures.sync(&wanted) {
                    eprintln!("gnoblin-xr: capture of view {node:?} failed: {error}");
                }

                let mut layers = Vec::new();
                let mut notices = Vec::new();
                let mut faults = Vec::new();
                if frame.should_render {
                    faults = surfaces.sync(frames.runtime_mut(), presenter, &capabilities, &wanted, captures);
                    let eyes = if planned.layers.iter().any(|layer| matches!(layer, Layer::Projection(_))) {
                        frames.runtime_mut().locate_views(frame.display_time)?
                    } else {
                        None
                    };
                    let submission = compose(
                        &planned.layers,
                        &|view| surfaces.swapchain(view),
                        eyes.as_ref(),
                        &limits,
                    );
                    layers = submission.layers;
                    notices = submission.notices;
                }
                frames.finish(&frame, &layers)?;
                on_frame(&FrameReport {
                    plan: &planned,
                    notices: &notices,
                    faults: &faults,
                    submitted: layers.len(),
                });
            }
            Tick::Exit => return Ok(Outcome::SessionExited),
        }
    }
}

/// The View nodes the plan draws. Each is captured so its picture can be shown in the headset.
fn view_nodes(layers: &[Layer]) -> BTreeSet<u64> {
    layers
        .iter()
        .map(|layer| match layer {
            Layer::Quad(placement) | Layer::Projection(placement) => placement.id,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use gnoblin_proto::tree::change;
    use gnoblin_proto::tree::snapshot_node;
    use gnoblin_proto::tree::tree::{AttachedPayload, ReplyPayload, SnapshotPayload, TransactArgs};
    use gnoblin_proto::{Envelope, read_frame, write_frame};
    use std::collections::{BTreeMap, VecDeque};

    use xr_capture::ShmFormat;
    use xr_capture::views::Fault as CaptureFault;
    use xr_capture::wayland::Frame as Picture;
    use xr_gpu::{FakePresenter, SwapchainFormat};
    use xr_input::{BTN_LEFT, BTN_RIGHT, ControllerState, SCROLL_RATE};
    use xr_layers::{EyeVisibility, ImageRect, NoticeKind, SubmittedLayer, SwapchainId};
    use xr_scene::{Layer, Placement, Ray, Space};
    use xr_session::runtime::{FrameTiming, SessionEvent, SessionState};
    use xr_session::testing::{Call, FakeRuntime, Step, image_handle};

    use super::*;

    /// The request id the client uses for its attach request.
    const ATTACH_REQUEST_ID: u64 = 1;

    fn set(node: u64, key: &str) -> change::Properties {
        change::Properties {
            kind: "set".into(),
            node,
            key: key.into(),
            ..Default::default()
        }
    }

    fn text(node: u64, key: &str, value: &str) -> change::Properties {
        change::Properties {
            value_kind: "text".into(),
            value_text: value.into(),
            ..set(node, key)
        }
    }

    fn double(node: u64, key: &str, value: f64) -> change::Properties {
        change::Properties {
            value_kind: "double".into(),
            value_double: value,
            ..set(node, key)
        }
    }

    fn node_ref(node: u64, key: &str, value: u64) -> change::Properties {
        change::Properties {
            value_kind: "node".into(),
            value_node: value,
            ..set(node, key)
        }
    }

    fn snapshot_node(
        id: u64,
        parent: u64,
        index: i64,
        class: &str,
        props: Vec<change::Properties>,
    ) -> snapshot_node::Properties {
        snapshot_node::Properties {
            id,
            parent,
            index,
            class: class.into(),
            props,
            ..Default::default()
        }
    }

    /// A `room` holding one 1920x1080 View that shows the node `3`.
    fn room_with_one_view() -> Vec<snapshot_node::Properties> {
        vec![
            snapshot_node(1, 0, 0, "node", vec![text(1, "name", "room")]),
            snapshot_node(
                2,
                1,
                0,
                "view",
                vec![
                    double(2, "width", 1920.0),
                    double(2, "height", 1080.0),
                    node_ref(2, "source", 3),
                ],
            ),
            snapshot_node(3, 1, 1, "node", vec![]),
        ]
    }

    /// Starts a fake owner that answers the attach with the room snapshot. It holds
    /// the socket open until the client drops it, unless `hang_up` closes it at once.
    fn serve(nodes: Vec<snapshot_node::Properties>, hang_up: bool) -> UnixStream {
        serve_with_ops(nodes, hang_up, None)
    }

    /// As [`serve`], and sends each transaction's ops to `ops_sink` when one is given.
    fn serve_with_ops(
        nodes: Vec<snapshot_node::Properties>,
        hang_up: bool,
        ops_sink: Option<std::sync::mpsc::Sender<Vec<gnoblin_proto::tree::op::Properties>>>,
    ) -> UnixStream {
        serve_refusing(nodes, hang_up, ops_sink, &[])
    }

    /// As [`serve_with_ops`], and refuses (`applied: false`, code `refused-by-test`) every transaction that holds an
    /// `input` op of one of the `refused` kinds. The refused ops still reach `ops_sink`.
    fn serve_refusing(
        nodes: Vec<snapshot_node::Properties>,
        hang_up: bool,
        ops_sink: Option<std::sync::mpsc::Sender<Vec<gnoblin_proto::tree::op::Properties>>>,
        refused: &'static [&'static str],
    ) -> UnixStream {
        let events = vec![
            Envelope::reply_message(
                "tree",
                "attach",
                ATTACH_REQUEST_ID,
                &AttachedPayload {
                    epoch: 1,
                    root: 1,
                    serial: 1,
                },
            ),
            Envelope::event_message(
                "tree",
                "snapshot",
                &SnapshotPayload {
                    serial: 1,
                    chunk: 0,
                    last: true,
                    nodes,
                },
            ),
        ];
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        std::thread::spawn(move || {
            let attach = read_frame(&mut server).expect("read attach").expect("attach frame");
            assert_eq!(attach.request_id, ATTACH_REQUEST_ID);
            for event in &events {
                if write_frame(&mut server, event).is_err() {
                    return;
                }
            }
            if hang_up {
                return;
            }
            // Answer every transact as applied, and hand its ops to the test.
            let mut serial = 1;
            while let Ok(Some(request)) = read_frame(&mut server) {
                let args = request.payload_as::<TransactArgs>().expect("transact args");
                serial += 1;
                if let Some(sink) = &ops_sink {
                    let _ = sink.send(args.ops.clone());
                }
                // A refused entry is an input kind (`button`), or a kind and a state (`button:0`: releases only).
                let refuse = args.ops.iter().any(|op| {
                    refused.contains(&op.input_kind.as_str())
                        || refused.contains(&format!("{}:{}", op.input_kind, op.state).as_str())
                });
                let reply = Envelope::reply_message(
                    "tree",
                    "transact",
                    request.request_id,
                    &ReplyPayload {
                        serial,
                        applied: !refuse,
                        failed_op: if refuse { 0 } else { -1 },
                        code: if refuse {
                            "refused-by-test".to_owned()
                        } else {
                            String::new()
                        },
                        removed: Vec::new(),
                    },
                );
                if write_frame(&mut server, &reply).is_err() {
                    return;
                }
            }
        });
        client
    }

    /// As [`serve`], but holds the socket open until `close` fires or drops, then hangs up.
    fn serve_until(nodes: Vec<snapshot_node::Properties>, close: std::sync::mpsc::Receiver<()>) -> UnixStream {
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        std::thread::spawn(move || {
            let attach = read_frame(&mut server).expect("read attach").expect("attach frame");
            assert_eq!(attach.request_id, ATTACH_REQUEST_ID);
            let events = [
                Envelope::reply_message(
                    "tree",
                    "attach",
                    ATTACH_REQUEST_ID,
                    &AttachedPayload {
                        epoch: 1,
                        root: 1,
                        serial: 1,
                    },
                ),
                Envelope::event_message(
                    "tree",
                    "snapshot",
                    &SnapshotPayload {
                        serial: 1,
                        chunk: 0,
                        last: true,
                        nodes,
                    },
                ),
            ];
            for event in &events {
                if write_frame(&mut server, event).is_err() {
                    return;
                }
            }
            let _ = close.recv();
        });
        client
    }

    /// A quad at `z = -1` facing the viewer, drawing the window `3` at 1920x1080.
    fn front_quad() -> Vec<Layer> {
        vec![Layer::Quad(Placement {
            id: 2,
            source: 3,
            pose: Pose {
                position: [0.0, 0.0, -1.0],
                orientation: IDENTITY,
            },
            size: [1.0, 1.0],
            extent: [1920.0, 1080.0],
            opacity: 1.0,
            space: Space::World,
            input: true,
        })]
    }

    #[test]
    fn pointer_sends_motion_then_leave_over_the_window_under_the_ray() {
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree =
            TreeClient::from_stream(serve_with_ops(room_with_one_view(), false, Some(sink))).expect("attach");
        let mut pointer = Pointer::default();
        let layers = front_quad();
        let through_centre = Ray {
            origin: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        };

        pointer
            .update(&mut tree, &layers, &aiming(through_centre), 0)
            .expect("motion");
        let sent: Vec<_> = ops.try_iter().flatten().collect();
        let motion = sent
            .iter()
            .find(|op| op.input_kind == "motion")
            .expect("a motion op for the hit");
        assert_eq!(motion.node, 3, "input goes to the window the view shows");
        assert_eq!((motion.x, motion.y), (960.0, 540.0), "the ray hits the quad's centre");

        // The same pixel again sends nothing.
        pointer
            .update(&mut tree, &layers, &aiming(through_centre), FRAME)
            .expect("still hovering");
        assert!(ops.try_recv().is_err(), "an unchanged hit sends no new input");

        // A ray that misses leaves the window it was over.
        pointer
            .update(&mut tree, &layers, &ControllerState::default(), 2 * FRAME)
            .expect("leave");
        let left: Vec<_> = ops.try_iter().flatten().collect();
        let leave = left.iter().find(|op| op.input_kind == "leave").expect("a leave op");
        assert_eq!(leave.node, 3);
    }

    fn list(node: u64, key: &str, value: &[f64]) -> change::Properties {
        change::Properties {
            value_kind: "list_double".into(),
            value_list: value.to_vec(),
            ..set(node, key)
        }
    }

    /// The room the stock shell builds (`data/lua/gnoblin/shell/xr.lua`) for two 400 by 300 windows: millimetres, the
    /// room's floor 1.4 m under the head, the focused window's View (id 2, window 3) at the front 1.5 m ahead and
    /// 1.15 times larger, and the other's (id 5, window 6) 0.6 rad round the ring to the right, facing the viewer.
    fn stock_ring() -> Vec<snapshot_node::Properties> {
        let view = |id: u64, source: u64, index: i64, at: [f64; 3], yaw: f64, scale: f64| {
            snapshot_node(
                id,
                1,
                index,
                "view",
                vec![
                    double(id, "width", 400.0),
                    double(id, "height", 300.0),
                    node_ref(id, "source", source),
                    double(id, "x", at[0]),
                    double(id, "y", at[1]),
                    double(id, "z", at[2]),
                    list(id, "rotation", &[0.0, yaw, 0.0]),
                    list(id, "scale", &[scale, scale, scale]),
                ],
            )
        };
        let side = 0.6_f64;
        vec![
            snapshot_node(1, 0, 0, "node", vec![text(1, "name", "room"), double(1, "y", -1400.0)]),
            snapshot_node(
                4,
                1,
                0,
                "camera",
                vec![
                    text(4, "name", "head"),
                    text(4, "projection", "stereo"),
                    double(4, "metres", 0.001),
                ],
            ),
            // Back to front, as the stock shell lists them: the focused View last.
            view(
                5,
                6,
                1,
                [1600.0 * side.sin(), 1400.0, -1600.0 * side.cos()],
                -side,
                0.75,
            ),
            view(2, 3, 2, [0.0, 1400.0, -1500.0], 0.0, 0.75 * 1.15),
            snapshot_node(3, 1, 3, "node", vec![]),
            snapshot_node(6, 1, 4, "node", vec![]),
        ]
    }

    #[test]
    fn the_gaze_through_the_stock_ring_enters_the_front_window_on_the_xr_seat() {
        // The whole path of one frame, from the head's pose to the owner's op: the planner places the stock shell's
        // Views, the head's forward axis is the ray when no controller is tracked, the pick finds the front panel
        // and the pointer sends motion for its window on the seat it made.
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree = TreeClient::from_stream(serve_with_ops(stock_ring(), false, Some(sink))).expect("attach");
        let head = Pose {
            position: [0.0; 3],
            orientation: IDENTITY,
        };
        let planned = plan(tree.model(), &head, &Options::default());
        assert_eq!(planned.layers.len(), 2, "both Views are planned: {planned:?}");
        assert!(planned.skipped.is_empty(), "{:?}", planned.skipped);
        let gaze = xr_input::pointing_ray(None, Some(&head)).expect("the head's forward axis is a ray");
        let mut pointer = Pointer::default();

        pointer
            .update(&mut tree, &planned.layers, &aiming(gaze), 0)
            .expect("the first frame");

        let sent: Vec<_> = ops.try_iter().flatten().collect();
        let seat = sent
            .iter()
            .find(|op| op.kind == "new" && op.class == "seat")
            .expect("the xr seat is made");
        let motion = sent
            .iter()
            .find(|op| op.kind == "input" && op.input_kind == "motion")
            .expect("the gaze gives the front window motion");
        assert_eq!(motion.seat, seat.id, "on the xr seat");
        assert_eq!(
            motion.node, 3,
            "to the window of the focused front View, not the one at the side"
        );
        // 400 px at 0.75 mm per pixel scaled by 1.15, 1.5 m ahead on the gaze axis: the centre of the window.
        assert!(
            (motion.x - 200.0).abs() < 1e-6 && (motion.y - 150.0).abs() < 1e-6,
            "the gaze hits the window's centre: ({}, {})",
            motion.x,
            motion.y
        );
    }

    #[test]
    fn pointer_creates_the_xr_seat_when_the_snapshot_has_none() {
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree =
            TreeClient::from_stream(serve_with_ops(room_with_one_view(), false, Some(sink))).expect("attach");
        let mut pointer = Pointer::default();
        let through_centre = Ray {
            origin: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        };

        pointer
            .update(&mut tree, &front_quad(), &aiming(through_centre), 0)
            .expect("motion");
        let sent: Vec<_> = ops.try_iter().flatten().collect();
        let created = sent
            .iter()
            .find(|op| op.kind == "new" && op.class == "seat")
            .expect("the owner creates a seat");
        let name = sent
            .iter()
            .find(|op| op.kind == "set" && op.node == created.id && op.key == "name")
            .expect("the new seat is named");
        assert_eq!(name.value_text, "xr", "the pointer's seat is named xr");
        let motion = sent.iter().find(|op| op.input_kind == "motion").expect("a motion op");
        assert_eq!(motion.seat, created.id, "the pointer delivers on the seat it created");
    }

    #[test]
    fn the_xr_seat_is_created_on_the_first_frame_with_nothing_to_hit() {
        // The room, and so every View, exists only once the seat does, so the seat cannot wait for a ray to hit one.
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree = TreeClient::from_stream(serve_with_ops(Vec::new(), false, Some(sink))).expect("attach");
        let mut pointer = Pointer::default();

        pointer
            .update(&mut tree, &[], &ControllerState::default(), 0)
            .expect("a frame with no layers");
        let sent: Vec<_> = ops.try_iter().flatten().collect();
        let created = sent
            .iter()
            .find(|op| op.kind == "new" && op.class == "seat")
            .expect("the seat is made with no layer in the plan");
        assert!(
            sent.iter()
                .any(|op| op.kind == "set" && op.node == created.id && op.key == "name" && op.value_text == "xr"),
            "the seat is named xr: {sent:?}"
        );
        assert!(
            !sent.iter().any(|op| op.kind == "input"),
            "nothing was hit, so no input"
        );

        pointer
            .update(&mut tree, &[], &ControllerState::default(), FRAME)
            .expect("the next frame");
        assert!(ops.try_recv().is_err(), "the seat is made once");
    }

    #[test]
    fn pointer_reuses_an_existing_xr_seat() {
        let mut nodes = room_with_one_view();
        nodes.push(snapshot_node(9, 1, 2, "seat", vec![text(9, "name", "xr")]));
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree = TreeClient::from_stream(serve_with_ops(nodes, false, Some(sink))).expect("attach");
        let mut pointer = Pointer::default();
        let through_centre = Ray {
            origin: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        };

        pointer
            .update(&mut tree, &front_quad(), &aiming(through_centre), 0)
            .expect("motion");
        let sent: Vec<_> = ops.try_iter().flatten().collect();
        assert!(
            !sent.iter().any(|op| op.kind == "new"),
            "an existing xr seat is reused, not recreated"
        );
        let motion = sent.iter().find(|op| op.input_kind == "motion").expect("a motion op");
        assert_eq!(motion.seat, 9, "the pointer delivers on the existing xr seat");
    }

    /// The time between two frames in these tests: 16 ms, in nanoseconds.
    const FRAME: i64 = 16_000_000;

    /// A controller whose ray is `ray` and that holds nothing.
    fn aiming(ray: Ray) -> ControllerState {
        ControllerState {
            ray: Some(ray),
            ..ControllerState::default()
        }
    }

    /// The ray from `(x, 0, 0)` straight ahead.
    fn ahead(x: f64) -> Ray {
        Ray {
            origin: [x, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
        }
    }

    /// `ray` with the select input at `select`.
    fn selecting(ray: Ray, select: f32) -> ControllerState {
        ControllerState { select, ..aiming(ray) }
    }

    /// Two 1 m by 1 m windows of 1000 by 1000 px, side by side a metre ahead: the View `2` showing window `3` centred
    /// at x = -1, and the View `4` showing window `5` centred at x = +1.
    fn two_windows() -> Vec<Layer> {
        let quad = |id: u64, source: u64, x: f64| {
            Layer::Quad(Placement {
                id,
                source,
                pose: Pose {
                    position: [x, 0.0, -1.0],
                    orientation: IDENTITY,
                },
                size: [1.0, 1.0],
                extent: [1000.0, 1000.0],
                opacity: 1.0,
                space: Space::World,
                input: true,
            })
        };
        vec![quad(2, 3, -1.0), quad(4, 5, 1.0)]
    }

    /// A pointer, and the ops the fake owner receives from it.
    fn pointer_with_owner(
        refused: &'static [&'static str],
    ) -> (
        Pointer,
        TreeClient,
        std::sync::mpsc::Receiver<Vec<gnoblin_proto::tree::op::Properties>>,
    ) {
        let (sink, ops) = std::sync::mpsc::channel();
        let stream = serve_refusing(room_with_one_view(), false, Some(sink), refused);
        let tree = TreeClient::from_stream(stream).expect("attach");
        (Pointer::default(), tree, ops)
    }

    /// The input ops received since the last call, as `(kind, node, button, state)` for the kinds the tests compare.
    fn received(
        ops: &std::sync::mpsc::Receiver<Vec<gnoblin_proto::tree::op::Properties>>,
    ) -> Vec<gnoblin_proto::tree::op::Properties> {
        ops.try_iter().flatten().filter(|op| op.kind == "input").collect()
    }

    /// `(input kind, node)` for each op.
    fn kinds(sent: &[gnoblin_proto::tree::op::Properties]) -> Vec<(&str, u64)> {
        sent.iter().map(|op| (op.input_kind.as_str(), op.node)).collect()
    }

    #[test]
    fn a_press_then_a_release_sends_state_one_then_zero_for_the_left_button() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = front_quad();
        let centre = ahead(0.0);

        pointer
            .update(&mut tree, &layers, &selecting(centre, 0.0), 0)
            .expect("hover");
        assert_eq!(kinds(&received(&ops)), [("motion", 3)]);

        pointer
            .update(&mut tree, &layers, &selecting(centre, 1.0), FRAME)
            .expect("press");
        let sent = received(&ops);
        assert_eq!(
            kinds(&sent),
            [("button", 3)],
            "the press goes to the window under the ray"
        );
        assert_eq!((sent[0].button, sent[0].state), (u64::from(BTN_LEFT), 1));
        assert_eq!((sent[0].x, sent[0].y), (960.0, 540.0), "at the point the ray hit");

        pointer
            .update(&mut tree, &layers, &selecting(centre, 1.0), 2 * FRAME)
            .expect("held");
        assert!(received(&ops).is_empty(), "a held button sends nothing new");

        pointer
            .update(&mut tree, &layers, &selecting(centre, 0.0), 3 * FRAME)
            .expect("release");
        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("button", 3)]);
        assert_eq!((sent[0].button, sent[0].state), (u64::from(BTN_LEFT), 0));
    }

    #[test]
    fn the_secondary_input_is_the_right_button() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = front_quad();
        let mut state = aiming(ahead(0.0));
        state.secondary = 1.0;

        pointer.update(&mut tree, &layers, &state, 0).expect("press");
        let sent = received(&ops);
        let button = sent.iter().find(|op| op.input_kind == "button").expect("a button op");
        assert_eq!((button.button, button.state), (u64::from(BTN_RIGHT), 1));
    }

    #[test]
    fn a_drag_across_windows_keeps_the_grab_and_reports_pixels_outside_the_window() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = two_windows();

        // Press on window 3 (centre: x = -1 m).
        pointer
            .update(&mut tree, &layers, &selecting(ahead(-1.0), 1.0), 0)
            .expect("press");
        let sent = received(&ops);
        assert_eq!(
            kinds(&sent),
            [("motion", 3), ("button", 3)],
            "motion in, then the press"
        );

        // Drag over window 5 (x = +1 m): the owner still hears from window 3 only, and the point is 2 m right of
        // window 3's centre: (2 + 0.5) m of 1 m is 2500 px.
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 1.0), FRAME)
            .expect("drag");
        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("motion", 3)], "no leave, and nothing for window 5");
        assert_eq!((sent[0].x, sent[0].y), (2500.0, 500.0));

        // Drag into the void, far to the right of both windows: still window 3, plane coordinates.
        pointer
            .update(&mut tree, &layers, &selecting(ahead(10.0), 1.0), 2 * FRAME)
            .expect("drag out");
        let sent = received(&ops);
        assert_eq!(
            kinds(&sent),
            [("motion", 3)],
            "a miss does not leave while a button is held"
        );
        assert_eq!((sent[0].x, sent[0].y), (11500.0, 500.0));

        // A frame that locates no ray at all sends nothing and keeps the grab.
        pointer
            .update(
                &mut tree,
                &layers,
                &ControllerState {
                    select: 1.0,
                    ..ControllerState::default()
                },
                3 * FRAME,
            )
            .expect("tracking lost");
        assert!(received(&ops).is_empty());

        // Release over window 5: the last motion and the release go to window 3, then the pointer leaves 3 and enters 5.
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 0.0), 4 * FRAME)
            .expect("release");
        let sent = received(&ops);
        assert_eq!(
            kinds(&sent),
            [("motion", 3), ("button", 3), ("leave", 3), ("motion", 5)]
        );
        assert_eq!(sent[1].state, 0);
        assert_eq!((sent[3].x, sent[3].y), (500.0, 500.0));
    }

    #[test]
    fn a_release_in_the_void_ends_the_grab_and_then_leaves() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = two_windows();

        pointer
            .update(&mut tree, &layers, &selecting(ahead(-1.0), 1.0), 0)
            .expect("press");
        received(&ops);
        pointer
            .update(&mut tree, &layers, &selecting(ahead(10.0), 0.0), FRAME)
            .expect("release");
        let sent = received(&ops);
        let order: Vec<_> = kinds(&sent);
        assert_eq!(order, [("motion", 3), ("button", 3), ("leave", 3)]);
    }

    #[test]
    fn the_grab_ends_with_the_last_of_two_held_buttons() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = two_windows();
        let both = |ray, select, secondary| ControllerState {
            select,
            secondary,
            ..aiming(ray)
        };

        pointer
            .update(&mut tree, &layers, &both(ahead(-1.0), 1.0, 0.0), 0)
            .expect("left down");
        pointer
            .update(&mut tree, &layers, &both(ahead(1.0), 1.0, 1.0), FRAME)
            .expect("right down");
        received(&ops);
        // The left button comes up over window 5: the grab is still held by the right one.
        pointer
            .update(&mut tree, &layers, &both(ahead(1.0), 0.0, 1.0), 2 * FRAME)
            .expect("left up");
        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("button", 3)], "still grabbed: no leave");
        assert_eq!((sent[0].button, sent[0].state), (u64::from(BTN_LEFT), 0));
        pointer
            .update(&mut tree, &layers, &both(ahead(1.0), 0.0, 0.0), 3 * FRAME)
            .expect("right up");
        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("button", 3), ("leave", 3), ("motion", 5)]);
        assert_eq!((sent[0].button, sent[0].state), (u64::from(BTN_RIGHT), 0));
    }

    #[test]
    fn a_press_over_no_window_is_dropped_with_its_release() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = two_windows();

        pointer
            .update(&mut tree, &layers, &selecting(ahead(10.0), 1.0), 0)
            .expect("press in the void");
        // Dragged onto a window while still held: the window never saw the press, so it gets no release.
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 1.0), FRAME)
            .expect("onto a window");
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 0.0), 2 * FRAME)
            .expect("release");
        let sent = received(&ops);
        assert!(
            sent.iter().all(|op| op.input_kind != "button"),
            "no button op was delivered: {:?}",
            kinds(&sent)
        );
    }

    #[test]
    fn the_stick_scrolls_the_window_under_the_ray() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = front_quad();
        let stick = |x, y| ControllerState {
            scroll: (x, y),
            ..aiming(ahead(0.0))
        };

        pointer.update(&mut tree, &layers, &stick(0.0, 0.0), 0).expect("hover");
        received(&ops);
        // 50 ms of full deflection towards the user (down): 50 ms * SCROLL_RATE units a second, down the page.
        pointer
            .update(&mut tree, &layers, &stick(0.0, -1.0), 50_000_000)
            .expect("scroll down");
        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("scroll", 3)]);
        assert!(
            (sent[0].dy - f64::from(SCROLL_RATE) * 0.05).abs() < 1e-3,
            "{}",
            sent[0].dy
        );
        assert_eq!(sent[0].dx, 0.0);
        assert_eq!((sent[0].x, sent[0].y), (960.0, 540.0));

        pointer
            .update(&mut tree, &layers, &stick(1.0, 0.0), 100_000_000)
            .expect("scroll right");
        let sent = received(&ops);
        assert!(
            (sent[0].dx - f64::from(SCROLL_RATE) * 0.05).abs() < 1e-3,
            "{}",
            sent[0].dx
        );
        assert_eq!(sent[0].dy, 0.0);

        pointer
            .update(&mut tree, &layers, &stick(0.1, 0.1), 150_000_000)
            .expect("inside the deadzone");
        assert!(received(&ops).is_empty(), "a stick at rest scrolls nothing");
    }

    #[test]
    fn scroll_over_no_window_is_dropped_and_a_grab_takes_the_scroll() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&[]);
        let layers = two_windows();
        let stick = |ray, select| ControllerState {
            scroll: (0.0, -1.0),
            select,
            ..aiming(ray)
        };

        pointer
            .update(&mut tree, &layers, &stick(ahead(10.0), 0.0), 0)
            .expect("void");
        pointer
            .update(&mut tree, &layers, &stick(ahead(10.0), 0.0), 50_000_000)
            .expect("void scroll");
        assert!(received(&ops).is_empty(), "nothing to scroll over");

        pointer
            .update(&mut tree, &layers, &stick(ahead(-1.0), 1.0), 100_000_000)
            .expect("grab");
        received(&ops);
        pointer
            .update(&mut tree, &layers, &stick(ahead(1.0), 1.0), 150_000_000)
            .expect("scroll while dragging");
        let sent = received(&ops);
        assert_eq!(
            kinds(&sent),
            [("motion", 3), ("scroll", 3)],
            "the scroll follows the grab"
        );
        assert_eq!((sent[1].x, sent[1].y), (2500.0, 500.0));
    }

    #[test]
    fn a_refused_press_is_logged_and_starts_no_grab() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&["button"]);
        let layers = two_windows();

        pointer
            .update(&mut tree, &layers, &selecting(ahead(-1.0), 1.0), 0)
            .expect("refusal is not an error");
        assert_eq!(
            kinds(&received(&ops)),
            [("motion", 3), ("button", 3)],
            "the press was sent and refused"
        );

        // The loop carries on: the pointer is free, so it moves on to window 5, and the release is not sent.
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 1.0), FRAME)
            .expect("moves on");
        assert_eq!(
            kinds(&received(&ops)),
            [("leave", 3), ("motion", 5)],
            "no grab was taken"
        );
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 0.0), 2 * FRAME)
            .expect("release");
        assert!(
            received(&ops).is_empty(),
            "the window never took the press, so it gets no release"
        );
    }

    #[test]
    fn a_refused_motion_is_sent_again() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&["motion"]);
        let layers = front_quad();
        pointer
            .update(&mut tree, &layers, &aiming(ahead(0.0)), 0)
            .expect("refusal is not an error");
        pointer
            .update(&mut tree, &layers, &aiming(ahead(0.0)), FRAME)
            .expect("again");
        assert_eq!(
            kinds(&received(&ops)),
            [("motion", 3), ("motion", 3)],
            "a motion the owner refused is not remembered as sent"
        );
    }

    #[test]
    fn a_refused_release_still_ends_the_grab() {
        let (mut pointer, mut tree, ops) = pointer_with_owner(&["button:0"]);
        let layers = two_windows();

        pointer
            .update(&mut tree, &layers, &selecting(ahead(-1.0), 1.0), 0)
            .expect("press");
        received(&ops);
        pointer
            .update(&mut tree, &layers, &selecting(ahead(1.0), 0.0), FRAME)
            .expect("release");
        assert_eq!(
            kinds(&received(&ops)),
            [("motion", 3), ("button", 3), ("leave", 3), ("motion", 5)],
            "the refusal is logged, and the pointer moves on to window 5"
        );
    }

    #[test]
    fn the_bridge_delivers_controller_input_from_the_runtime_each_frame() {
        let (sink, ops) = std::sync::mpsc::channel();
        let mut tree =
            TreeClient::from_stream(serve_with_ops(room_of(&[(2, 0.0, -1000.0)]), false, Some(sink))).expect("attach");
        let mut runtime = FakeRuntime::new(script(4));
        let frame = |select: f32| ControllerState {
            select,
            ..aiming(ahead(0.0))
        };
        runtime.controller_script = [frame(0.0), frame(1.0), frame(1.0), frame(0.0)].into();
        let mut frames = FrameLoop::new(runtime);
        let outcome = run(
            &mut tree,
            &mut frames,
            &Options::default(),
            None,
            &mut ScriptedFrames::default(),
            &mut FakePresenter::default(),
            |_| {},
        )
        .expect("bridge runs");
        assert_eq!(outcome, Outcome::SessionExited);

        let sent = received(&ops);
        assert_eq!(kinds(&sent), [("motion", 3), ("button", 3), ("button", 3)]);
        assert_eq!((sent[1].button, sent[1].state), (u64::from(BTN_LEFT), 1));
        assert_eq!((sent[2].button, sent[2].state), (u64::from(BTN_LEFT), 0));
    }

    #[test]
    fn an_owner_that_refuses_all_input_does_not_stop_the_frames() {
        let (sink, ops) = std::sync::mpsc::channel();
        let stream = serve_refusing(
            room_of(&[(2, 0.0, -1000.0)]),
            false,
            Some(sink),
            &["motion", "button", "scroll", "leave"],
        );
        let mut tree = TreeClient::from_stream(stream).expect("attach");
        let mut runtime = FakeRuntime::new(script(3));
        runtime.controller = ControllerState {
            select: 1.0,
            scroll: (0.0, -1.0),
            ..aiming(ahead(0.0))
        };
        let mut frames = FrameLoop::new(runtime);
        let mut reports = Reports::default();
        let outcome = run(
            &mut tree,
            &mut frames,
            &Options::default(),
            None,
            &mut ScriptedFrames::default(),
            &mut FakePresenter::default(),
            |report| reports.push(report),
        )
        .expect("a refusal does not end the bridge");
        assert_eq!(outcome, Outcome::SessionExited);
        assert_eq!(reports.plans.len(), 3, "every frame ran");
        assert!(!received(&ops).is_empty(), "the bridge kept trying");
    }

    fn state(state: SessionState) -> Step {
        Step::Event(SessionEvent::StateChanged(state))
    }

    fn timing(display_time: i64) -> Step {
        Step::Frame(FrameTiming {
            predicted_display_time: display_time,
            should_render: true,
        })
    }

    /// Plays pictures for the Views. Each `pump` applies the next step: a View's new picture, or `None` to take it
    /// away. The bridge pumps once per pass, and a pass runs at most one frame, so step `n` is what frame `n` sees.
    #[derive(Default)]
    struct ScriptedFrames {
        steps: VecDeque<Vec<(u64, Option<Picture>)>>,
        current: BTreeMap<u64, Picture>,
        wanted: Vec<BTreeSet<u64>>,
    }

    impl ScriptedFrames {
        fn new(steps: Vec<Vec<(u64, Option<Picture>)>>) -> Self {
            Self {
                steps: steps.into(),
                ..Self::default()
            }
        }
    }

    impl FrameSource for ScriptedFrames {
        fn pump(&mut self) -> Vec<CaptureFault> {
            for (view, picture) in self.steps.pop_front().unwrap_or_default() {
                match picture {
                    Some(picture) => self.current.insert(view, picture),
                    None => self.current.remove(&view),
                };
            }
            Vec::new()
        }

        fn sync(&mut self, wanted: &BTreeSet<u64>) -> Vec<CaptureFault> {
            self.wanted.push(wanted.clone());
            Vec::new()
        }

        fn latest(&self, node: u64) -> Option<&Picture> {
            self.current.get(&node)
        }
    }

    fn picture(width: u32, height: u32, serial: u64, fill: u8) -> Picture {
        Picture {
            width,
            height,
            stride: width * 4,
            format: ShmFormat::Argb8888,
            bytes: vec![fill; (width * height * 4) as usize],
            serial,
        }
    }

    /// A `room` of one metre per thousand units, holding a View per `(id, x, z)` of 1920x1080 units. View `id`
    /// shows the window `id + 1`.
    fn room_of(views: &[(u64, f64, f64)]) -> Vec<snapshot_node::Properties> {
        let mut nodes = vec![snapshot_node(
            1,
            0,
            0,
            "node",
            vec![text(1, "name", "room"), double(1, "metres", 0.001)],
        )];
        for (index, &(id, x, z)) in views.iter().enumerate() {
            nodes.push(snapshot_node(
                id,
                1,
                index as i64,
                "view",
                vec![
                    double(id, "width", 1920.0),
                    double(id, "height", 1080.0),
                    node_ref(id, "source", id + 1),
                    double(id, "x", x),
                    double(id, "z", z),
                ],
            ));
        }
        nodes
    }

    /// The summary of what a run reported for each frame.
    #[derive(Default)]
    struct Reports {
        plans: Vec<Plan>,
        notices: Vec<Vec<Notice>>,
        faults: Vec<Vec<Fault>>,
    }

    impl Reports {
        fn push(&mut self, report: &FrameReport<'_>) {
            self.plans.push(report.plan.clone());
            self.notices.push(report.notices.to_vec());
            self.faults.push(report.faults.to_vec());
        }
    }

    /// Runs the bridge over `nodes` with a graphics runtime that ends after `frames` frames.
    fn run_frames(
        nodes: Vec<snapshot_node::Properties>,
        runtime: FakeRuntime,
        steps: Vec<Vec<(u64, Option<Picture>)>>,
    ) -> (FrameLoop<FakeRuntime>, FakePresenter, ScriptedFrames, Reports) {
        let mut tree = TreeClient::from_stream(serve(nodes, false)).expect("attach");
        let mut frames = FrameLoop::new(runtime);
        let mut presenter = FakePresenter::default();
        let mut captures = ScriptedFrames::new(steps);
        let mut reports = Reports::default();
        let outcome = run(
            &mut tree,
            &mut frames,
            &Options::default(),
            None,
            &mut captures,
            &mut presenter,
            |report| reports.push(report),
        )
        .expect("bridge runs");
        assert_eq!(outcome, Outcome::SessionExited);
        (frames, presenter, captures, reports)
    }

    fn script(frames: u32) -> Vec<Step> {
        let mut steps = vec![state(SessionState::Ready)];
        steps.extend((1..=frames).map(|n| timing(i64::from(n))));
        steps.push(state(SessionState::Exiting));
        steps
    }

    fn quads(layers: &[SubmittedLayer]) -> Vec<&xr_layers::QuadLayer> {
        layers
            .iter()
            .map(|layer| match layer {
                SubmittedLayer::Quad(quad) => quad,
                SubmittedLayer::Projection(_) => panic!("a quad layer, got {layer:?}"),
            })
            .collect()
    }

    #[test]
    fn a_planned_view_is_submitted_as_a_quad_at_the_planned_pose_with_its_picture() {
        let runtime = FakeRuntime::with_graphics(script(1), 16);
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 7)))]];
        let (frames, presenter, _, reports) = run_frames(room_of(&[(2, 500.0, -1000.0)]), runtime, steps);

        let [(time, layers)] = frames.runtime().submitted.as_slice() else {
            panic!("one frame was ended: {:?}", frames.runtime().submitted);
        };
        assert_eq!(*time, 1);
        let [quad] = quads(layers)[..] else {
            panic!("one quad: {layers:?}");
        };
        assert_eq!(quad.view, 2);
        assert_eq!(quad.swapchain, SwapchainId(1));
        assert_eq!(quad.space, Space::World);
        assert_eq!(quad.pose.position, [0.5, 0.0, -1.0], "the planned position, in metres");
        assert_eq!(quad.pose.orientation, IDENTITY);
        assert_eq!(quad.size, [1.92, 1.08], "the planned size, in metres");
        assert_eq!(quad.eye_visibility, EyeVisibility::Both);
        assert_eq!(
            quad.rect,
            ImageRect {
                x: 0,
                y: 0,
                width: 8,
                height: 4
            },
            "the whole picture"
        );
        assert_eq!(presenter.uploads.len(), 1);
        assert_eq!(presenter.uploads[0].target.image, image_handle(SwapchainId(1), 0));
        assert_eq!(presenter.uploads[0].target.format, SwapchainFormat::Rgba8Srgb);
        assert!(reports.notices[0].is_empty() && reports.faults[0].is_empty());
    }

    #[test]
    fn layers_are_submitted_in_plan_order_each_with_its_own_swapchain() {
        let runtime = FakeRuntime::with_graphics(script(1), 16);
        let views = [(2, -500.0, -1000.0), (4, 500.0, -1000.0)];
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 1))), (4, Some(picture(4, 4, 1, 2)))]];
        let (frames, presenter, _, _) = run_frames(room_of(&views), runtime, steps);

        let (_, layers) = &frames.runtime().submitted[0];
        let submitted = quads(layers);
        assert_eq!(submitted.iter().map(|quad| quad.view).collect::<Vec<_>>(), vec![2, 4]);
        assert_ne!(submitted[0].swapchain, submitted[1].swapchain);
        assert_eq!(submitted[0].pose.position[0], -0.5);
        assert_eq!(submitted[1].pose.position[0], 0.5);
        assert_eq!(presenter.uploads.len(), 2);
    }

    #[test]
    fn a_still_picture_is_uploaded_once_and_a_new_serial_again() {
        let runtime = FakeRuntime::with_graphics(script(3), 16);
        let steps = vec![
            vec![(2, Some(picture(8, 4, 1, 1)))],
            vec![(2, Some(picture(8, 4, 1, 1)))],
            vec![(2, Some(picture(8, 4, 2, 9)))],
        ];
        let (frames, presenter, _, _) = run_frames(room_of(&[(2, 0.0, -1000.0)]), runtime, steps);

        let creates = frames
            .runtime()
            .calls
            .iter()
            .filter(|call| matches!(call, Call::CreateSwapchain { .. }))
            .count();
        assert_eq!(creates, 1, "one swapchain for the View across frames");
        assert_eq!(
            presenter.uploads.len(),
            2,
            "uploaded for serial 1 and serial 2, not for the repeat"
        );
        assert_eq!(presenter.uploads[1].bytes[0], 9);
        assert_eq!(frames.runtime().submitted.len(), 3);
        assert!(
            frames.runtime().submitted.iter().all(|(_, layers)| layers.len() == 1),
            "the View is submitted every frame"
        );
    }

    #[test]
    fn a_new_picture_size_recreates_the_swapchain() {
        let runtime = FakeRuntime::with_graphics(script(2), 16);
        let steps = vec![
            vec![(2, Some(picture(8, 4, 1, 1)))],
            vec![(2, Some(picture(16, 8, 2, 1)))],
        ];
        let (frames, presenter, _, _) = run_frames(room_of(&[(2, 0.0, -1000.0)]), runtime, steps);

        let calls = &frames.runtime().calls;
        let creates: Vec<_> = calls
            .iter()
            .filter_map(|call| match call {
                Call::CreateSwapchain { width, height, .. } => Some((*width, *height)),
                _ => None,
            })
            .collect();
        assert_eq!(creates, vec![(8, 4), (16, 8)]);
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, Call::DestroySwapchain(_)))
                .count(),
            2
        );
        assert_eq!(presenter.uploads.len(), 2);
        let (_, layers) = &frames.runtime().submitted[1];
        let quad = quads(layers)[0];
        assert_eq!(
            quad.swapchain,
            SwapchainId(2),
            "the second frame shows the new swapchain"
        );
        assert_eq!((quad.rect.width, quad.rect.height), (16, 8));
        assert!(
            frames.runtime().swapchains.is_empty(),
            "the swapchains are destroyed when the bridge ends"
        );
    }

    #[test]
    fn a_view_with_no_picture_yet_is_not_submitted_and_is_reported() {
        let runtime = FakeRuntime::with_graphics(script(1), 16);
        let (frames, presenter, _, reports) = run_frames(room_of(&[(2, 0.0, -1000.0)]), runtime, vec![]);

        assert_eq!(frames.runtime().submitted, vec![(1, Vec::new())]);
        assert!(presenter.uploads.is_empty());
        assert_eq!(
            reports.notices[0],
            vec![Notice {
                view: 2,
                kind: NoticeKind::NoSwapchain
            }]
        );
    }

    #[test]
    fn a_session_without_graphics_submits_nothing_and_says_why() {
        let runtime = FakeRuntime::new(script(2));
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 1)))]];
        let (frames, presenter, _, reports) = run_frames(room_of(&[(2, 0.0, -1000.0)]), runtime, steps);

        assert!(
            frames.runtime().submitted.iter().all(|(_, layers)| layers.is_empty()),
            "no layer without a swapchain"
        );
        assert!(presenter.uploads.is_empty());
        assert!(
            !frames
                .runtime()
                .calls
                .iter()
                .any(|call| matches!(call, Call::CreateSwapchain { .. })),
            "no swapchain was asked for"
        );
        let [fault] = reports.faults[0].as_slice() else {
            panic!("one fault: {:?}", reports.faults[0]);
        };
        assert!(fault.to_string().contains("XR_KHR_vulkan_enable2"), "{fault}");
    }

    #[test]
    fn a_frame_the_runtime_says_not_to_render_has_no_layers_and_no_uploads() {
        let script = vec![
            state(SessionState::Ready),
            Step::Frame(FrameTiming {
                predicted_display_time: 1,
                should_render: false,
            }),
            state(SessionState::Exiting),
        ];
        let runtime = FakeRuntime::with_graphics(script, 16);
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 1)))]];
        let (frames, presenter, _, _) = run_frames(room_of(&[(2, 0.0, -1000.0)]), runtime, steps);

        assert_eq!(frames.runtime().submitted, vec![(1, Vec::new())]);
        assert!(presenter.uploads.is_empty());
    }

    #[test]
    fn views_beyond_the_layer_budget_are_dropped_and_reported() {
        let runtime = FakeRuntime::with_graphics(script(1), 1);
        let views = [(2, -500.0, -1000.0), (4, 500.0, -1000.0)];
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 1))), (4, Some(picture(8, 4, 1, 2)))]];
        let (frames, _, _, reports) = run_frames(room_of(&views), runtime, steps);

        let (_, layers) = &frames.runtime().submitted[0];
        assert_eq!(quads(layers).iter().map(|quad| quad.view).collect::<Vec<_>>(), vec![2]);
        assert_eq!(
            reports.notices[0],
            vec![Notice {
                view: 4,
                kind: NoticeKind::LayerBudget
            }]
        );
    }

    #[test]
    fn opacity_becomes_a_colour_scale_on_the_layer() {
        let runtime = FakeRuntime::with_graphics(script(1), 16);
        let mut nodes = room_of(&[(2, 0.0, -1000.0)]);
        nodes[1].props.push(double(2, "opacity", 0.5));
        let steps = vec![vec![(2, Some(picture(8, 4, 1, 1)))]];
        let (frames, _, _, _) = run_frames(nodes, runtime, steps);

        let (_, layers) = &frames.runtime().submitted[0];
        assert_eq!(quads(layers)[0].color_scale, Some([0.5; 4]));
    }

    #[test]
    fn the_captures_follow_the_planned_views() {
        let runtime = FakeRuntime::with_graphics(script(1), 16);
        let views = [(2, -500.0, -1000.0), (4, 500.0, -1000.0)];
        let (_, _, captures, _) = run_frames(room_of(&views), runtime, vec![]);
        assert_eq!(captures.wanted, vec![BTreeSet::from([2, 4])]);
    }

    #[test]
    fn each_frame_plans_the_room_from_the_owner_model() {
        let runtime = FakeRuntime::new(script(1));
        let (_, _, _, reports) = run_frames(room_with_one_view(), runtime, vec![]);

        assert_eq!(reports.plans.len(), 1, "one plan for the one frame");
        assert_eq!(reports.plans[0].layers.len(), 1, "the room's view becomes one layer");
        assert!(reports.plans[0].skipped.is_empty());
    }

    #[test]
    fn owner_closing_its_socket_ends_the_bridge() {
        let mut tree = TreeClient::from_stream(serve(room_with_one_view(), true)).expect("attach");
        let mut frames = FrameLoop::new(FakeRuntime::new(Vec::new()));
        let mut reports = Reports::default();
        let mut presenter = FakePresenter::default();
        let mut captures = ScriptedFrames::default();

        let outcome = run(
            &mut tree,
            &mut frames,
            &Options::default(),
            None,
            &mut captures,
            &mut presenter,
            |report| reports.push(report),
        )
        .expect("bridge runs");

        assert_eq!(outcome, Outcome::OwnerClosed);
        assert!(reports.plans.is_empty(), "no session frame ran");
    }

    /// Runs the bridge end to end against Monado with a Vulkan driver (lavapipe on Agent37): a fake owner with one View,
    /// a scripted picture, the real Vulkan session and presenter. The owner hangs up after a few frames. Every frame
    /// must end without a fault, and the View must be submitted.
    ///
    /// Run with `GNOBLIN_XR_MONADO=1 cargo test -- --ignored`, with `XR_RUNTIME_JSON` pointing at Monado's manifest and
    /// a running `monado-service`.
    #[test]
    #[ignore = "needs Monado and a Vulkan driver; set GNOBLIN_XR_MONADO=1 and run with --ignored"]
    fn monado_bridge_submits_a_view_end_to_end() {
        use xr_gpu::VulkanPresenter;
        use xr_session::openxr_runtime::OpenxrRuntime;

        if std::env::var_os("GNOBLIN_XR_MONADO").as_deref() != Some("1".as_ref()) {
            return;
        }
        let (hang_up, hung_up) = std::sync::mpsc::channel::<()>();
        let mut tree = TreeClient::from_stream(serve_until(room_of(&[(2, 500.0, -1000.0)]), hung_up)).expect("attach");
        let (runtime, device) = OpenxrRuntime::connect_vulkan("gnoblin-xr-test").expect("connect");
        let mut presenter = VulkanPresenter::new(device).expect("presenter");
        let mut frames = FrameLoop::new(runtime);
        let mut captures = ScriptedFrames::new(vec![vec![(2, Some(picture(64, 48, 1, 0x40)))]]);

        let mut reports = Reports::default();
        let outcome = run(
            &mut tree,
            &mut frames,
            &Options::default(),
            None,
            &mut captures,
            &mut presenter,
            |report| {
                reports.push(report);
                if reports.plans.len() == 5 {
                    let _ = hang_up.send(());
                }
            },
        )
        .expect("bridge runs");

        assert_eq!(outcome, Outcome::OwnerClosed);
        assert!(reports.faults.iter().all(Vec::is_empty), "faults: {:?}", reports.faults);
        assert!(
            reports.notices.iter().all(Vec::is_empty),
            "notices: {:?}",
            reports.notices
        );
        assert_eq!(reports.plans.len(), 5);
    }
}
