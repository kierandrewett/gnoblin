//! The controllers in the tree: one `controller` node for each hand the runtime reports.
//!
//! The program is an observer on the owner socket, and a binding in the shell reads these nodes (`design/js-xr.md`):
//!
//! - A node is made when the runtime reports a hand's controller, with every value in one transaction, so a reader never
//!   sees a hand without its pose. It is destroyed when the runtime stops reporting the hand, and when the session ends.
//! - Its pose is the aim ray's, in the room's node units and the placement frame the Views use
//!   ([`xr_scene::controller`]); `tracked` is false while the runtime cannot locate the aim pose, and the node then keeps
//!   its last pose.
//! - A frame writes both hands in one transaction, and only the values that changed. A write is at most once for each
//!   hand in a frame, and the frames of a fast display are thinned to [`MIN_INTERVAL`] apart, under the rate the
//!   compositor allows an observer. The last values always arrive: a skipped frame is only a later write.
//! - The shell writes `claimed` on the node. The program reads it back from its mirror ([`Controllers::claims`]) and
//!   withholds the inputs it names from the window under the ray (`xr_input::Arbiter`).
//!
//! The owner refusing a write is logged once and is not fatal: the values are written again on the next frame.

use xr_input::{Claims, Hand, HandState, Hands};
use xr_scene::NodeId;
use xr_scene::controller::{PlacedPose, placed_pose};
use xr_tree::{TreeClient, TreeError, Value};

use crate::bridge::STALE_BASE;

#[cfg(test)]
pub(crate) mod owner;
#[cfg(test)]
mod tests;

/// The shortest time, in nanoseconds of display time, between two frames that write a controller: about 90 a second at
/// most, under the 120 the compositor allows an observer and under any display's rate that would spend it.
pub const MIN_INTERVAL: i64 = 11_000_000;

/// Where a controller sits before the runtime has located it.
const ORIGIN: PlacedPose = PlacedPose {
    position: [0.0; 3],
    rotation_degrees: [0.0; 3],
};

/// What a controller node holds: the values the program wrote last, or is about to.
#[derive(Debug, Clone, PartialEq)]
struct Values {
    tracked: bool,
    pose: PlacedPose,
    trigger: f64,
    grip: f64,
    thumbstick: (f64, f64),
    thumbstick_click: bool,
    primary: bool,
    secondary: bool,
}

/// A runtime value as the node holds it: a number a runtime reports out of range, or that is not a number at all, is
/// clamped or taken for rest, since the owner refuses a value that is not finite.
fn level(value: f32, low: f64, high: f64) -> f64 {
    if value.is_finite() {
        f64::from(value).clamp(low, high)
    } else {
        0.0
    }
}

impl Values {
    /// The values of `state`. `held` is the last pose the runtime located for the hand: a hand with no aim pose keeps
    /// it, and a pose that is located replaces it.
    fn of(state: &HandState, metres: f64, held: &mut Option<PlacedPose>) -> Self {
        let located = state.aim.as_ref().and_then(|aim| placed_pose(aim, metres));
        if located.is_some() {
            *held = located;
        }
        Self {
            tracked: located.is_some(),
            pose: located.or(*held).unwrap_or(ORIGIN),
            trigger: level(state.trigger, 0.0, 1.0),
            grip: level(state.grip, 0.0, 1.0),
            thumbstick: (
                level(state.thumbstick.0, -1.0, 1.0),
                level(state.thumbstick.1, -1.0, 1.0),
            ),
            thumbstick_click: state.thumbstick_click,
            primary: state.primary,
            secondary: state.secondary,
        }
    }

    /// Every property, in the order they are written.
    fn all(&self) -> [(&'static str, Value); 12] {
        let [x, y, z] = self.pose.position;
        [
            ("tracked", Value::Bool(self.tracked)),
            ("x", Value::Double(x)),
            ("y", Value::Double(y)),
            ("z", Value::Double(z)),
            (
                "rotation",
                Value::Doubles(self.pose.rotation_degrees.map(f64::to_radians).to_vec()),
            ),
            ("trigger", Value::Double(self.trigger)),
            ("grip", Value::Double(self.grip)),
            ("thumbstick_x", Value::Double(self.thumbstick.0)),
            ("thumbstick_y", Value::Double(self.thumbstick.1)),
            ("thumbstick_click", Value::Bool(self.thumbstick_click)),
            ("primary", Value::Bool(self.primary)),
            ("secondary", Value::Bool(self.secondary)),
        ]
    }

    /// The properties that differ from `old`, in the order they are written.
    fn changed(&self, old: &Self) -> Vec<(&'static str, Value)> {
        self.all()
            .into_iter()
            .zip(old.all())
            .filter(|(new, old)| new.1 != old.1)
            .map(|(new, _)| new)
            .collect()
    }
}

/// One hand's node and what is known of it.
#[derive(Debug, Default)]
struct Slot {
    /// The node the program made for the hand, while there is one.
    node: Option<NodeId>,
    /// What the node holds, as far as the owner has confirmed.
    written: Option<Values>,
    /// The last pose the runtime located for the hand.
    held: Option<PlacedPose>,
}

const fn index(hand: Hand) -> usize {
    match hand {
        Hand::Left => 0,
        Hand::Right => 1,
    }
}

/// The program's controller nodes.
#[derive(Debug, Default)]
pub struct Controllers {
    slots: [Slot; 2],
    /// The display time of the frame that last wrote, for [`MIN_INTERVAL`].
    last: Option<i64>,
    /// The code of the owner's last refusal of a write, while it lasts, so a repeated one is logged once.
    refusal: Option<String>,
}

/// A write a frame has decided on: the hand it is for, its node, what the node will hold, and the properties to write.
struct Write {
    hand: Hand,
    node: NodeId,
    values: Values,
    set: Vec<(&'static str, Value)>,
}

impl Controllers {
    /// The inputs the shell has claimed on `hand`'s controller, as its node says. Nothing is claimed while there is no
    /// node, or the owner has not yet told the program about the shell's write.
    #[must_use]
    pub fn claims(&self, tree: &TreeClient, hand: Hand) -> Claims {
        self.slots[index(hand)]
            .node
            .and_then(|node| tree.model().nodes.get(&node))
            .map_or_else(Claims::default, |node| Claims::from_names(&node.claimed))
    }

    /// Brings the nodes up to date with `hands`, at display time `time`, for a room of `metres` metres a node unit.
    ///
    /// A hand that is gone loses its node at once. A hand that is there gets a node, or its changed values, unless the
    /// frame is within [`MIN_INTERVAL`] of the last that wrote. Everything a frame writes goes in one transaction.
    ///
    /// # Errors
    ///
    /// The socket fails or closes. A refusal is not an error.
    pub fn update(&mut self, tree: &mut TreeClient, hands: &Hands, metres: f64, time: i64) -> Result<(), TreeError> {
        let due = self
            .last
            .is_none_or(|last| time < last || time.saturating_sub(last) >= MIN_INTERVAL);
        let mut sent = false;
        let mut writes: Vec<Write> = Vec::new();
        for hand in Hand::BOTH {
            let slot = &mut self.slots[index(hand)];
            let Some(state) = hands.get(hand) else {
                // The pose of a hand that is lost is not the pose of the hand that comes back.
                slot.held = None;
                if let Some(node) = slot.node.take() {
                    slot.written = None;
                    self.destroy(tree, node)?;
                }
                continue;
            };
            if !due {
                continue;
            }
            let values = Values::of(state, metres, &mut slot.held);
            match (slot.node, &slot.written) {
                (None, _) => {
                    let mut set = vec![("hand", Value::Text(hand.name().to_owned()))];
                    set.extend(values.all());
                    sent = true;
                    match tree.create_controller(&set) {
                        Ok(node) => {
                            slot.node = Some(node);
                            slot.written = Some(values);
                            self.refusal = None;
                        }
                        Err(error) => self.refused(error)?,
                    }
                }
                (Some(node), Some(old)) => {
                    let set = values.changed(old);
                    if !set.is_empty() {
                        writes.push(Write {
                            hand,
                            node,
                            values,
                            set,
                        });
                    }
                }
                (Some(_), None) => {}
            }
        }
        if writes.is_empty() {
            if sent {
                self.last = Some(time);
            }
            return Ok(());
        }
        let batch: Vec<(NodeId, &[(&str, Value)])> =
            writes.iter().map(|write| (write.node, write.set.as_slice())).collect();
        match tree.set_nodes(&batch) {
            Ok(_) => {
                self.refusal = None;
                for write in writes {
                    self.slots[index(write.hand)].written = Some(write.values);
                }
            }
            Err(error) => self.refused(error)?,
        }
        self.last = Some(time);
        Ok(())
    }

    /// Ends the controllers' part in the session: destroys the nodes this program made, so the shell sees its hands go.
    /// The controllers are ready to start again afterwards.
    ///
    /// # Errors
    ///
    /// The socket fails or closes. A refusal is not an error.
    pub fn end(&mut self, tree: &mut TreeClient) -> Result<(), TreeError> {
        for hand in Hand::BOTH {
            let slot = &mut self.slots[index(hand)];
            slot.written = None;
            slot.held = None;
            if let Some(node) = slot.node.take() {
                self.destroy(tree, node)?;
            }
        }
        self.last = None;
        self.refusal = None;
        Ok(())
    }

    /// Destroys `node`. The owner refusing, as when the node is already gone, leaves nothing to keep.
    fn destroy(&mut self, tree: &mut TreeClient, node: NodeId) -> Result<(), TreeError> {
        match tree.destroy_node(node) {
            Ok(()) => Ok(()),
            Err(error) => self.refused(error),
        }
    }

    /// Takes the owner's refusal of a write: a lagging mirror is tried again without a word, any other refusal is logged
    /// once and tried again at the next frame. Anything else that went wrong is the caller's to stop for.
    fn refused(&mut self, error: TreeError) -> Result<(), TreeError> {
        match error {
            TreeError::Refused { code, .. } if code == STALE_BASE => Ok(()),
            TreeError::Refused { code, .. } => {
                if self.refusal.as_deref() != Some(code.as_str()) {
                    eprintln!("gnoblin-xr: the owner refused a controller write: {code}");
                    self.refusal = Some(code);
                }
                Ok(())
            }
            other => Err(other),
        }
    }
}
