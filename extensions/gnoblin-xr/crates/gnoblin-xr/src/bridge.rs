//! The bridge: keeps the owner tree model current and plans every frame from it.
//!
//! Each pass reads what the owner has sent, then runs one tick of the frame
//! loop. A frame plans the `room` subtree against the head pose the runtime
//! located. The plan is handed to the caller and not submitted: the frame loop
//! ends every frame with no layers.

use std::fmt;
use std::thread;
use std::time::Duration;

use xr_scene::{IDENTITY, Options, Plan, Pose, plan};
use xr_session::frame_loop::{FrameLoop, Tick};
use xr_session::runtime::{Runtime, RuntimeError};
use xr_tree::{TreeClient, TreeError};

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

/// Runs until the session exits or the owner goes away, calling `on_plan` with
/// the plan of every frame.
///
/// # Errors
///
/// The owner socket fails, or the runtime fails.
pub fn run<R: Runtime>(
    tree: &mut TreeClient,
    frames: &mut FrameLoop<R>,
    opts: &Options,
    mut on_plan: impl FnMut(&Plan),
) -> Result<Outcome, BridgeError> {
    loop {
        match tree.poll() {
            Ok(_) => {}
            Err(TreeError::Closed) => return Ok(Outcome::OwnerClosed),
            Err(error) => return Err(error.into()),
        }
        match frames.tick()? {
            Tick::Idle => thread::sleep(IDLE_INTERVAL),
            Tick::Frame(frame) => {
                let head = frame.head.unwrap_or(Pose {
                    position: [0.0; 3],
                    orientation: IDENTITY,
                });
                on_plan(&plan(tree.model(), &head, opts));
            }
            Tick::Exit => return Ok(Outcome::SessionExited),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use gnoblin_proto::tree::change;
    use gnoblin_proto::tree::snapshot_node;
    use gnoblin_proto::tree::tree::{AttachedPayload, SnapshotPayload};
    use gnoblin_proto::{Envelope, read_frame, write_frame};
    use xr_session::runtime::{FrameTiming, SessionEvent, SessionState};
    use xr_session::testing::{FakeRuntime, Step};

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
            let _attach = read_frame(&mut server).expect("read attach").expect("attach frame");
            for event in &events {
                if write_frame(&mut server, event).is_err() {
                    return;
                }
            }
            if !hang_up {
                let _ = read_frame(&mut server);
            }
        });
        client
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

    #[test]
    fn each_frame_plans_the_room_from_the_owner_model() {
        let mut tree = TreeClient::from_stream(serve(room_with_one_view(), false)).expect("attach");
        let script = [
            state(SessionState::Ready),
            state(SessionState::Focused),
            timing(1),
            state(SessionState::Exiting),
        ];
        let mut frames = FrameLoop::new(FakeRuntime::new(script));
        let mut plans = Vec::new();

        let outcome = run(&mut tree, &mut frames, &Options::default(), |plan| {
            plans.push(plan.clone())
        })
        .expect("bridge runs");

        assert_eq!(outcome, Outcome::SessionExited);
        assert_eq!(plans.len(), 1, "one plan for the one frame");
        assert_eq!(plans[0].layers.len(), 1, "the room's view becomes one layer");
        assert!(plans[0].skipped.is_empty());
    }

    #[test]
    fn owner_closing_its_socket_ends_the_bridge() {
        let mut tree = TreeClient::from_stream(serve(room_with_one_view(), true)).expect("attach");
        let mut frames = FrameLoop::new(FakeRuntime::new(Vec::new()));
        let mut plans = Vec::new();

        let outcome = run(&mut tree, &mut frames, &Options::default(), |plan| {
            plans.push(plan.clone())
        })
        .expect("bridge runs");

        assert_eq!(outcome, Outcome::OwnerClosed);
        assert!(plans.is_empty(), "no session frame ran");
    }
}
