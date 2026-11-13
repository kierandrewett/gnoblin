//! Head tracking: the head pose into the tree, and the layer budget beside it.
//!
//! A stereo View whose `camera` is the camera named `head` shows its source from that camera's pose. The extension
//! measures the head, so the extension writes the pose onto the camera; the compositor redraws the View from it, and the
//! planner states the same eyes from the same camera. Three things make that safe and exact enough to use:
//!
//! - **What is written.** The head pose becomes the camera's `x`, `y`, `z` and `rotation` in one transaction
//!   ([`xr_scene::head::camera_pose`]). The compositor accepts those four keys and `layer_budget` from an observer
//!   connection, and only on the camera named `head`. It refuses a value that is not finite and a connection that is
//!   not the session owner's, and it caps how fast one observer may write (`rate-limited`). The extension never sends
//!   a value it would refuse, and stays under the cap on its own: see [`MAX_WRITES_PER_SECOND`].
//! - **When it is written.** Only while a View draws through the head camera. A pose measured longer ago than
//!   [`STALE_AFTER`] is dropped, not written: the next pass measures a fresh one. A runtime that reports no head pose
//!   writes nothing and the camera keeps its last pose. A write goes out at most [`MAX_WRITES_PER_SECOND`] times a
//!   second and only once the last one has shown in the tree and every driven View has drawn from it, or
//!   [`IN_FLIGHT_TIMEOUT`] has passed: one pose in flight, so redraws never queue behind each other.
//! - **Which eyes go with which picture.** A projection layer declares the eyes its picture was drawn from, and the
//!   runtime reprojects against the head's pose at display time. Eyes stated from a pose newer than the picture would
//!   make the world swim with the head. The extension pairs them itself: a picture that is first seen while a write
//!   is in flight is taken to be drawn from that write's pose, any other picture from the camera in the tree. A picture
//!   that was in fact drawn just before the write is paired wrongly for one write step at most (the write always
//!   causes a redraw, which pairs correctly). Pairing the other way round would be wrong for as long as nothing else
//!   redrew the View.
//!
//! Everything here is pure: it takes the clock as an argument and the tree as a value, so the rules run under `cargo
//! test` with no owner, no runtime and no sleeping. The bridge does the writes.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use xr_scene::head::{CameraPose, camera_pose};
use xr_scene::rig::eyes;
use xr_scene::{Class, Eye, Layer, NodeId, Pose, Projected, Rig, TreeView};

/// The name of the camera the extension follows the head with, and the one `xr-scene` treats as the viewer.
pub const CAMERA: &str = "head";

/// The most pose writes the extension makes in one second. The compositor's own cap is higher (it allows a burst and
/// 120 a second), so the extension never meets it in normal use.
pub const MAX_WRITES_PER_SECOND: u32 = 60;

/// The shortest time between two pose writes.
pub const MIN_INTERVAL: Duration = Duration::from_secs(1).checked_div(MAX_WRITES_PER_SECOND).unwrap();

/// A pose measured longer ago than this is not written. At 90 Hz it is nine frames.
pub const STALE_AFTER: Duration = Duration::from_millis(100);

/// How long a write may wait to show in the tree and to be drawn before the extension stops waiting and writes the
/// next pose. The compositor draws a View at its `fps` cap, so a View capped far below the write rate is what this
/// bounds.
pub const IN_FLIGHT_TIMEOUT: Duration = Duration::from_millis(250);

/// How long the extension leaves a write the owner refused alone before it tries again. A refusal is the owner's policy
/// (a connection that may not write, a rate cap met), and hammering it helps nobody.
pub const REFUSED_BACKOFF: Duration = Duration::from_secs(1);

/// The tolerance when comparing a pose written with the pose the tree reports back, in the camera's units and degrees.
/// The wire carries doubles, so only the radians-to-degrees round trip differs.
const SAME: f64 = 1.0e-9;

/// How far the head may move from the last pose written before another write is worth its redraw: 0.1 mm of position,
/// and an orientation angle of about 0.005 degrees (a quaternion dot product within `1e-9` of 1). A headset on a table
/// jitters by less than that, and would otherwise redraw every View through the head camera at the write rate.
const DEADBAND_METRES: f64 = 1.0e-4;
const DEADBAND_DOT: f64 = 1.0e-9;

/// The head the runtime measured, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// The pose, or `None` when the runtime does not track the head.
    pub pose: Option<Pose>,
    /// When it was measured, on the same clock as [`Input::now`].
    pub measured: Instant,
}

/// The head camera as the mirror of the tree holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraState {
    pub id: NodeId,
    /// Metres per camera unit.
    pub metres: f64,
    pub rig: Rig,
    /// `x`, `y` and `z`, in the camera's units.
    pub position: [f64; 3],
    /// The rotation in degrees about X, then Y, then Z.
    pub rotation_degrees: [f64; 3],
    /// The `layer_budget` last seen, if any.
    pub layer_budget: Option<u64>,
}

impl CameraState {
    /// The camera node `id` of `model`, or `None` when it is not a camera.
    #[must_use]
    pub fn of(model: &TreeView, id: NodeId) -> Option<Self> {
        let node = model.nodes.get(&id)?;
        let Class::Camera { rig, .. } = node.class else {
            return None;
        };
        Some(Self {
            id,
            // The schema's default, which the mirror already holds for a camera that never set one.
            metres: node.metres.unwrap_or(0.001),
            rig,
            position: node.position,
            rotation_degrees: node.rotation,
            layer_budget: node.layer_budget,
        })
    }
}

/// A View drawn through the head camera: the planner's eyes for it now, and the serial of the newest picture of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Driven {
    pub view: NodeId,
    /// The View's size in node units, which the eyes' frustum depends on.
    pub extent: [f64; 2],
    /// The serial of the newest picture, or `None` while the View has none.
    pub serial: Option<u64>,
    /// The eyes the planner states from the camera in the tree.
    pub eyes: [Eye; 2],
}

/// What one pass tells the tracker.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub now: Instant,
    pub sample: Sample,
    /// The camera named `head`, if the tree has one.
    pub camera: Option<CameraState>,
    /// The Views drawn through it.
    pub driven: &'a [Driven],
}

/// A pose write for the bridge to send.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Write {
    pub camera: NodeId,
    /// The head pose the camera pose was made from.
    pub head: Pose,
    pub pose: CameraPose,
}

/// Why a pass wrote nothing, or that it is following. The bridge logs it when it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The tree has no camera named `head`.
    NoCamera,
    /// No View is drawn through the head camera, so nothing needs its pose.
    NoViews,
    /// The runtime reports no head pose.
    Untracked,
    /// The pose was measured longer ago than [`STALE_AFTER`].
    Stale,
    /// The pose has a value that is not finite, or an orientation that is not a rotation, or is out of range.
    Unusable,
    /// The last write is still in flight.
    Waiting,
    /// The last write was not seen in the tree or drawn in time. The next pose goes out.
    Lost,
    /// The owner refused the last write.
    Refused,
    /// A write is due, but the last one was less than [`MIN_INTERVAL`] ago.
    Throttled,
    /// The camera already holds the head's pose.
    Following,
}

impl Status {
    /// The name the extension reports, in the form of the other reports (`layer-budget`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NoCamera => "no-head-camera",
            Self::NoViews => "no-views",
            Self::Untracked => "untracked",
            Self::Stale => "stale-pose",
            Self::Unusable => "unusable-pose",
            Self::Waiting => "waiting",
            Self::Lost => "lost-write",
            Self::Refused => "refused",
            Self::Throttled => "throttled",
            Self::Following => "following",
        }
    }

    /// Whether this is a state worth a log line when it begins: the head or a write went wrong. The rest come and go
    /// every few frames in normal use, and a refusal is logged by the bridge with the owner's code.
    #[must_use]
    pub const fn is_fault(self) -> bool {
        matches!(self, Self::Untracked | Self::Stale | Self::Unusable | Self::Lost)
    }
}

/// What a pass decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub write: Option<Write>,
    pub status: Status,
}

/// A write that was sent and has not been drawn yet.
#[derive(Debug, Clone, Copy, PartialEq)]
struct InFlight {
    pose: CameraPose,
    sent: Instant,
}

/// The eyes a View's newest picture is paired with.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Paired {
    serial: u64,
    eyes: [Eye; 2],
    /// Whether this View has drawn a picture since the write now in flight was sent.
    drawn_since_send: bool,
}

/// Follows the head with the camera, and pairs each driven View's picture with the eyes it was drawn from.
#[derive(Debug, Default)]
pub struct Tracker {
    camera: Option<NodeId>,
    in_flight: Option<InFlight>,
    /// The serial of each driven View when the write in flight was sent.
    at_send: BTreeMap<NodeId, Option<u64>>,
    last_write: Option<Instant>,
    refused_at: Option<Instant>,
    /// The head pose last written and the camera pose made from it. A head that has not moved more than the deadband
    /// since is not written again while the camera still holds that pose.
    held: Option<(Pose, CameraPose)>,
    paired: BTreeMap<NodeId, Paired>,
}

impl Tracker {
    /// Decides one pass. It adopts the pictures that arrived, settles the write in flight, and says whether to write
    /// the head's pose now.
    pub fn step(&mut self, input: &Input<'_>) -> Step {
        let Some(camera) = input.camera else {
            self.reset();
            return stop(Status::NoCamera);
        };
        if input.driven.is_empty() {
            self.reset();
            return stop(Status::NoViews);
        }
        if self.camera != Some(camera.id) {
            // The camera was destroyed and made again: what was in flight and what was paired belong to the old one.
            self.reset();
            self.camera = Some(camera.id);
        }
        self.adopt(&camera, input.driven);
        self.paired
            .retain(|view, _| input.driven.iter().any(|driven| driven.view == *view));

        let mut lost = false;
        if let Some(flight) = self.in_flight {
            let shown = same_pose(&camera, &flight.pose);
            let drawn = input
                .driven
                .iter()
                .filter(|driven| driven.serial.is_some())
                .all(|driven| {
                    self.paired
                        .get(&driven.view)
                        .is_some_and(|paired| paired.drawn_since_send)
                });
            if shown && drawn {
                self.settle();
            } else if input.now.saturating_duration_since(flight.sent) >= IN_FLIGHT_TIMEOUT {
                self.settle();
                lost = true;
            } else {
                return stop(Status::Waiting);
            }
        }

        if let Some(refused) = self.refused_at
            && input.now.saturating_duration_since(refused) < REFUSED_BACKOFF
        {
            return stop(Status::Refused);
        }
        let Some(head) = input.sample.pose else {
            return stop(Status::Untracked);
        };
        if input.now.saturating_duration_since(input.sample.measured) > STALE_AFTER {
            return stop(Status::Stale);
        }
        let Some(pose) = camera_pose(&head, camera.metres) else {
            return stop(Status::Unusable);
        };
        if let Some((then, written)) = &self.held
            && within_deadband(&head, then)
            && same_pose(&camera, written)
        {
            return stop(if lost { Status::Lost } else { Status::Following });
        }
        if same_pose(&camera, &pose) {
            return stop(if lost { Status::Lost } else { Status::Following });
        }
        if let Some(last) = self.last_write
            && input.now.saturating_duration_since(last) < MIN_INTERVAL
        {
            return stop(Status::Throttled);
        }
        Step {
            write: Some(Write {
                camera: camera.id,
                head,
                pose,
            }),
            status: if lost { Status::Lost } else { Status::Following },
        }
    }

    /// Records that `write` went out at `now`. The pose is now in flight: no other goes out until it has shown in the
    /// tree and been drawn, or [`IN_FLIGHT_TIMEOUT`] has passed. `driven` are the Views and serials of that pass.
    pub fn wrote(&mut self, now: Instant, write: &Write, driven: &[Driven]) {
        self.last_write = Some(now);
        self.held = Some((write.head, write.pose));
        self.refused_at = None;
        self.in_flight = Some(InFlight {
            pose: write.pose,
            sent: now,
        });
        self.at_send = driven.iter().map(|driven| (driven.view, driven.serial)).collect();
        for paired in self.paired.values_mut() {
            paired.drawn_since_send = false;
        }
    }

    /// Records that the owner refused the write sent at `now`. Nothing is in flight, and the next attempt waits
    /// [`REFUSED_BACKOFF`].
    pub fn refused(&mut self, now: Instant) {
        self.last_write = Some(now);
        self.held = None;
        self.refused_at = Some(now);
        self.in_flight = None;
        self.at_send.clear();
    }

    /// Records that the owner refused the write sent at `now` as `stale-base`: it was built on a tree the owner had
    /// already moved on from, because the mirror had not yet read a change to the camera. That is not the owner's policy,
    /// so there is no back-off: nothing is in flight, and the next pass writes from a newer tree.
    pub fn lagged(&mut self, now: Instant) {
        self.last_write = Some(now);
        self.held = None;
        self.in_flight = None;
        self.at_send.clear();
    }

    /// Gives every head-driven projection layer in `layers` the eyes its newest picture was drawn from.
    pub fn pair(&self, layers: &mut [Layer]) {
        for layer in layers {
            if let Layer::Projection(Projected {
                placement,
                camera,
                eyes,
            }) = layer
                && Some(*camera) == self.camera
                && let Some(paired) = self.paired.get(&placement.id)
            {
                *eyes = paired.eyes;
            }
        }
    }

    /// Forgets everything that belonged to the camera being followed, a refusal included: the owner refused a write
    /// to that node, and a new camera is a new target.
    fn reset(&mut self) {
        self.camera = None;
        self.in_flight = None;
        self.held = None;
        self.refused_at = None;
        self.at_send.clear();
        self.paired.clear();
    }

    fn settle(&mut self) {
        self.in_flight = None;
        self.at_send.clear();
    }

    /// Pairs each View's newest picture, when it is a new one. A picture first seen with a write in flight is taken to
    /// be drawn from the written pose, and any other from the camera in the tree.
    fn adopt(&mut self, camera: &CameraState, driven: &[Driven]) {
        for view in driven {
            let Some(serial) = view.serial else { continue };
            if self
                .paired
                .get(&view.view)
                .is_some_and(|paired| paired.serial == serial)
            {
                continue;
            }
            let (pair_eyes, drawn_since_send) = match self.in_flight {
                Some(flight) => {
                    let sent_with = self.at_send.get(&view.view).copied().flatten();
                    let new = sent_with != Some(serial);
                    let from_write = eyes(
                        flight.pose.position,
                        flight.pose.rotation_degrees,
                        camera.metres,
                        &camera.rig,
                        view.extent,
                    );
                    (from_write.unwrap_or(view.eyes), new)
                }
                None => (view.eyes, false),
            };
            self.paired.insert(
                view.view,
                Paired {
                    serial,
                    eyes: pair_eyes,
                    drawn_since_send,
                },
            );
        }
    }
}

fn stop(status: Status) -> Step {
    Step { write: None, status }
}

/// Whether the camera in the tree already holds `pose`.
fn same_pose(camera: &CameraState, pose: &CameraPose) -> bool {
    camera
        .position
        .iter()
        .zip(&pose.position)
        .chain(camera.rotation_degrees.iter().zip(&pose.rotation_degrees))
        .all(|(held, wanted)| (held - wanted).abs() <= SAME * wanted.abs().max(1.0))
}

/// Keeps the head camera's `layer_budget` equal to the runtime's limit.
#[derive(Debug, Default)]
pub struct Budget {
    tried: Option<(NodeId, u64, Instant)>,
}

/// How long a budget write the owner refused is left alone before it is tried again.
const BUDGET_RETRY: Duration = Duration::from_secs(5);

impl Budget {
    /// Forgets the last try, because the owner refused it `stale-base` (the mirror lagged): the next pass tries again.
    pub fn lagged(&mut self) {
        self.tried = None;
    }

    /// The `(camera, budget)` to write now, if the camera named `head` does not hold `budget` and a write of it has
    /// not been tried (and refused or lost) in the last [`BUDGET_RETRY`].
    pub fn due(&mut self, now: Instant, camera: Option<CameraState>, budget: u64) -> Option<(NodeId, u64)> {
        let camera = camera?;
        if camera.layer_budget == Some(budget) {
            self.tried = None;
            return None;
        }
        if let Some((id, value, at)) = self.tried
            && id == camera.id
            && value == budget
            && now.saturating_duration_since(at) < BUDGET_RETRY
        {
            return None;
        }
        self.tried = Some((camera.id, budget, now));
        Some((camera.id, budget))
    }
}

/// Whether the head `now` is within the deadband of the head `then`.
fn within_deadband(now: &Pose, then: &Pose) -> bool {
    let moved: f64 = now
        .position
        .iter()
        .zip(&then.position)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        .sqrt();
    let dot: f64 = now.orientation.iter().zip(&then.orientation).map(|(a, b)| a * b).sum();
    moved <= DEADBAND_METRES && (1.0 - dot.abs()) <= DEADBAND_DOT
}

#[cfg(test)]
mod tests;
