//! One tracked hand: a `controller` node the XR program writes, what the library reads of it once a frame, and the inputs the shell
//! claims on it.

use std::cell::{Cell, RefCell};

use gnoblin_core::{Gnoblin, NodeId, Value};
use gnoblin_object::{ErrorCode, Fail, Obj, Res};

use super::trigger::INPUTS;
use crate::math::transform::PoseMatrix;
use crate::math::vector::{Point3, RayLine};

/// A trigger or a grip pulled this far is down, and stays down until it falls to [`RELEASE`].
const PRESS: f64 = 0.75;
const RELEASE: f64 = 0.25;

/// Who holds a claim: a script, by the name it gave, or a drag of a binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimOwner {
    Script(String),
    Binding(Obj),
}

/// What the library keeps of one controller of a session.
pub(crate) struct ControllerData {
    /// The object that stands for it, set once it is made.
    pub(crate) obj: Cell<Obj>,
    pub(crate) node: u64,
    /// Read once: a hand does not change, and a controller that has gone still says which it was.
    pub(crate) hand: String,
    /// The buttons that are down as of the last frame, as indices into `trigger::BUTTONS`, in order.
    pub(crate) down: RefCell<Vec<usize>>,
    /// Each owner's claim; the node holds the union.
    pub(crate) claims: RefCell<Vec<(ClaimOwner, Vec<usize>)>>,
}

/// A number of the node, 0 when it cannot be read.
fn number(g: &Gnoblin, node: u64, key: &str) -> f64 {
    crate::nodes::number(g, node, key)
}

fn flag(g: &Gnoblin, node: u64, key: &str) -> bool {
    crate::nodes::flag(g, node, key)
}

impl ControllerData {
    pub(crate) fn new(g: &Gnoblin, node: u64) -> ControllerData {
        let hand = g.node(NodeId(node)).get_text("hand").unwrap_or_default();
        ControllerData {
            obj: Cell::new(0),
            node,
            hand,
            down: RefCell::new(Vec::new()),
            claims: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn alive(&self, g: &Gnoblin) -> bool {
        g.node(NodeId(self.node))
            .class_name()
            .is_ok_and(|class| class == "controller")
    }

    pub(crate) fn tracked(&self, g: &Gnoblin) -> bool {
        flag(g, self.node, "tracked")
    }

    pub(crate) fn number(&self, g: &Gnoblin, key: &str) -> f64 {
        number(g, self.node, key)
    }

    /// The pose of the aim ray, in the room's placement frame.
    pub(crate) fn pose(&self, g: &Gnoblin) -> PoseMatrix {
        let rotation = match g.node(NodeId(self.node)).get("rotation") {
            Ok(Value::List(angles)) => [
                angles.first().copied().unwrap_or(0.0),
                angles.get(1).copied().unwrap_or(0.0),
                angles.get(2).copied().unwrap_or(0.0),
            ],
            _ => [0.0; 3],
        };
        PoseMatrix::new(
            Point3::new(self.number(g, "x"), self.number(g, "y"), self.number(g, "z")),
            rotation,
            1.0,
        )
    }

    /// The aim ray: from the pose's position along its negative z.
    pub(crate) fn ray(&self, g: &Gnoblin) -> RayLine {
        let pose = self.pose(g);
        RayLine {
            origin: pose.position,
            direction: pose.transform_direction(Point3::new(0.0, 0.0, -1.0)),
        }
    }

    /// What the node says is claimed.
    pub(crate) fn claimed(&self, g: &Gnoblin) -> Vec<String> {
        match g.node(NodeId(self.node)).get("claimed") {
            Ok(Value::Texts(names)) => names,
            _ => Vec::new(),
        }
    }

    /// Reads the values once and settles which buttons are down: a pull is down at 0.75 and up at 0.25, a click as the node says.
    pub(crate) fn update(&self, g: &Gnoblin) {
        let mut down = self.down.borrow_mut();
        for (button, key) in [(0, "trigger"), (1, "grip")] {
            let value = number(g, self.node, key);
            if down.contains(&button) {
                if value <= RELEASE {
                    down.retain(|held| *held != button);
                }
            } else if value >= PRESS {
                down.push(button);
            }
        }
        for (button, key) in [(2, "thumbstick_click"), (3, "primary"), (4, "secondary")] {
            let pressed = flag(g, self.node, key);
            if pressed && !down.contains(&button) {
                down.push(button);
            } else if !pressed {
                down.retain(|held| *held != button);
            }
        }
        down.sort_unstable();
    }

    /// The label of a refusal: `Controller#<id>.claimed`.
    pub(crate) fn target(&self) -> String {
        format!("Controller#{}.claimed", self.node)
    }

    /// Claims `inputs` for `owner`, in place of that owner's earlier claim, and writes the union when it changed. Nothing changes when
    /// the write is refused.
    ///
    /// # Errors
    ///
    /// The controller is gone (`not_found`), or the compositor refused the write.
    pub(crate) fn claim(&self, g: &Gnoblin, owner: ClaimOwner, inputs: Vec<usize>) -> Res<()> {
        let target = self.target();
        if !self.alive(g) {
            return Err(Fail::refused(
                ErrorCode::NotFound,
                format!("{target}: the controller is gone"),
            ));
        }
        let before = self.claims.borrow().clone();
        {
            let mut claims = self.claims.borrow_mut();
            claims.retain(|(held, _)| *held != owner);
            if !inputs.is_empty() {
                claims.push((owner, inputs));
            }
        }
        if let Err(fail) = self.flush(g) {
            *self.claims.borrow_mut() = before;
            return Err(fail);
        }
        Ok(())
    }

    /// Drops `owner`'s claim; other claims stand. A controller that is gone holds nothing to write.
    ///
    /// # Errors
    ///
    /// The compositor refused the write.
    pub(crate) fn release(&self, g: &Gnoblin, owner: &ClaimOwner) -> Res<()> {
        let had = {
            let mut claims = self.claims.borrow_mut();
            let before = claims.len();
            claims.retain(|(held, _)| held != owner);
            claims.len() != before
        };
        if !had || !self.alive(g) {
            return Ok(());
        }
        self.flush(g)
    }

    /// Drops every owner's claim, as the generation that made them ends.
    ///
    /// # Errors
    ///
    /// The compositor refused the write.
    pub(crate) fn release_all(&self, g: &Gnoblin) -> Res<()> {
        let had = !std::mem::take(&mut *self.claims.borrow_mut()).is_empty();
        if !had || !self.alive(g) {
            return Ok(());
        }
        self.flush(g)
    }

    /// Writes the union of the claims, once, when it differs from what the node says.
    fn flush(&self, g: &Gnoblin) -> Res<()> {
        let union: Vec<String> = {
            let claims = self.claims.borrow();
            INPUTS
                .iter()
                .enumerate()
                .filter(|(at, _)| claims.iter().any(|(_, held)| held.contains(at)))
                .map(|(_, name)| (*name).to_owned())
                .collect()
        };
        if self.claimed(g) == union {
            return Ok(());
        }
        g.node(NodeId(self.node))
            .set("claimed", Value::Texts(union))
            .map_err(|error| {
                Fail::new(
                    error.kind().code(),
                    format!("{}: the shell refused the write ({error})", self.target()),
                )
            })
    }
}

/// Reads a list of input names a shell claims.
///
/// # Errors
///
/// A name that is not one of the five inputs in lower case, with `target` in front.
pub(crate) fn inputs_of(target: &str, names: &[String]) -> Res<Vec<usize>> {
    let mut inputs = Vec::new();
    for name in names {
        if *name != name.to_lowercase() {
            return Err(Fail::type_error(format!(
                "{target}: input names are lowercase; write \"{}\", not \"{name}\"",
                name.to_lowercase()
            )));
        }
        let Some(at) = INPUTS.iter().position(|known| known == name) else {
            return Err(Fail::type_error(format!(
                "{target}: \"{name}\" is not an input; use {}",
                INPUTS.join(", ")
            )));
        };
        if !inputs.contains(&at) {
            inputs.push(at);
        }
    }
    Ok(inputs)
}
