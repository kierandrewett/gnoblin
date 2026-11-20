//! A view of a workspace: the windows on it, drawn without moving them. `data/js/gnoblin-std/scene_mirror.ts` is the reference.
//!
//! A holder node under the view holds one child view for each window of the workspace, put where the window is in the workspace's
//! area; the holder's scale and offset fit the area to the view. The mirror follows the workspace: it runs from the scene's turn, after
//! the layout pass has written where the windows are, and when the view's size or fit changes. It writes a property only when the node
//! does not hold the value already, so a steady picture costs no writes. It removes the nodes it made when it stops.

use gnoblin_core::{NodeId, Rect};
use gnoblin_object::{Fail, Res};

use super::registry::World;
use super::{Kind, read_through, write};
use crate::animation::staging::Prop;
use crate::nodes;

/// How a picture is fitted to the view's rectangle: the places of `content_fit`, counted from one.
pub(crate) const CONTAIN: u32 = 1;
pub(crate) const COVER: u32 = 2;

/// The nodes a view's mirror made.
#[derive(Debug)]
pub(crate) struct Mirror {
    /// The workspace's id.
    pub(crate) workspace: String,
    holder: Option<u64>,
    /// The child view of each window, by the window's node.
    children: Vec<(u64, u64)>,
}

/// One window of the mirrored workspace: its node and its rectangle in the compositor's logical pixels.
struct Shown {
    node: u64,
    rect: Rect,
    visible: bool,
}

/// What the mirror reads of a workspace.
struct Source {
    /// The windows on the workspace now, in stacking order.
    clients: Vec<Shown>,
    /// The rectangle the workspace fills (its output), or none while no output shows it.
    area: Option<Rect>,
}

/// The scale and the offset that fit `area` into `size` under `fit`. A size with no area or no extent is left as it is.
pub(crate) fn fit_scale(fit: u32, area: (f64, f64), size: (f64, f64)) -> ([f64; 3], f64, f64) {
    if !(area.0 > 0.0 && area.1 > 0.0 && size.0 > 0.0 && size.1 > 0.0) {
        return ([1.0, 1.0, 1.0], 0.0, 0.0);
    }
    let across = size.0 / area.0;
    let down = size.1 / area.1;
    let (sx, sy) = match fit {
        CONTAIN => (across.min(down), across.min(down)),
        COVER => (across.max(down), across.max(down)),
        _ => (across, down),
    };
    (
        [sx, sy, 1.0],
        (size.0 - area.0 * sx) / 2.0,
        (size.1 - area.1 * sy) / 2.0,
    )
}

/// The smallest rectangle that holds every window: the area of a workspace no output shows.
fn bounding_box(items: &[Shown]) -> Rect {
    if items.is_empty() {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    let left = items.iter().map(|item| item.rect.x).fold(f64::INFINITY, f64::min);
    let top = items.iter().map(|item| item.rect.y).fold(f64::INFINITY, f64::min);
    let right = items
        .iter()
        .map(|item| item.rect.x + item.rect.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = items
        .iter()
        .map(|item| item.rect.y + item.rect.height)
        .fold(f64::NEG_INFINITY, f64::max);
    Rect::new(left, top, right - left, bottom - top)
}

impl World {
    /// What the library knows of the workspace `id`: `None` when there is no such workspace.
    fn workspace_source(&self, id: &str) -> Option<Source> {
        let std = crate::state::find(self.ctx)?;
        let st = std.st.try_borrow().ok()?;
        let workspace = st
            .workspaces
            .iter()
            .copied()
            .find(|workspace| st.ws.get(workspace).is_some_and(|ws| ws.id == id))?;
        let clients = st
            .ws
            .get(&workspace)?
            .clients
            .iter()
            .map(|client| Shown {
                node: *client,
                rect: nodes::rect_of(&self.g, *client),
                visible: !nodes::flag(&self.g, *client, "minimised")
                    && st.rec.get(client).and_then(|rec| rec.shown_by_layout).unwrap_or(true),
            })
            .collect();
        let area = st
            .desk_showing(workspace)
            .and_then(|desk| st.desk(desk))
            .and_then(|desk| desk.output)
            .map(|output| nodes::rect_of(&self.g, output));
        Some(Source { clients, area })
    }

    /// Whether the library has a workspace by this id.
    pub(crate) fn has_workspace(&self, id: &str) -> bool {
        self.workspace_source(id).is_some()
    }

    /// Starts the mirror of the workspace `id` in the view `view`, ending the one it had.
    pub(crate) fn mirror_start(&self, view: u64, workspace: &str) -> Res<()> {
        self.mirror_stop(view);
        self.mirrors.borrow_mut().insert(
            view,
            Mirror {
                workspace: workspace.to_owned(),
                holder: None,
                children: Vec::new(),
            },
        );
        self.mirror_sync(view);
        Ok(())
    }

    /// Ends the mirror of a view and removes the nodes it made.
    pub(crate) fn mirror_stop(&self, view: u64) {
        let Some(mirror) = self.mirrors.borrow_mut().remove(&view) else {
            return;
        };
        if let Some(holder) = mirror.holder {
            let _ = self.state.borrow_mut().destroy(&self.g, holder);
        }
    }

    /// The view ended: its mirror is gone with its nodes.
    pub(crate) fn mirror_forget(&self, view: u64) {
        let Some(mirror) = self.mirrors.borrow_mut().remove(&view) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        for node in mirror
            .holder
            .into_iter()
            .chain(mirror.children.iter().map(|(_, child)| *child))
        {
            state.forget(node);
        }
    }

    /// Whether the view shows a workspace.
    pub(crate) fn mirrors_workspace(&self, view: u64) -> Option<String> {
        self.mirrors.borrow().get(&view).map(|mirror| mirror.workspace.clone())
    }

    /// Every mirror follows its workspace.
    pub(crate) fn mirror_turn(&self) {
        // In the order the views were made, which is the order of their numbers.
        let mut views: Vec<u64> = self.mirrors.borrow().keys().copied().collect();
        views.sort_unstable();
        for view in views {
            self.mirror_sync(view);
        }
    }

    /// Writes `key` of `node` unless the node holds `value` already.
    fn put(&self, node: u64, key: &str, value: Prop) -> Res<()> {
        if read_through(&self.g, node, key).as_ref() == Some(&value) {
            return Ok(());
        }
        write(&self.g, node, "the workspace picture", key, value).map_err(Fail::from)
    }

    /// Brings the nodes under `view` in step with the workspace it mirrors.
    pub(crate) fn mirror_sync(&self, view: u64) {
        let Some(workspace) = self.mirrors_workspace(view) else {
            return;
        };
        if let Err(fail) = self.sync(view, &workspace) {
            self.warn(&format!("{}.content: {}", self.label(view), fail.message));
        }
    }

    fn sync(&self, view: u64, workspace: &str) -> Res<()> {
        let source = self.workspace_source(workspace).unwrap_or(Source {
            clients: Vec::new(),
            area: None,
        });
        let area = source.area.unwrap_or_else(|| bounding_box(&source.clients));
        let number = |key: &str| match read_through(&self.g, view, key) {
            Some(Prop::Number(value)) => value,
            _ => 0.0,
        };
        let size = (number("width"), number("height"));
        let fit = self.entry(view).map_or(3, |entry| entry.fit);
        let (scale, x, y) = fit_scale(fit, (area.width, area.height), size);
        let held = self.mirrors.borrow().get(&view).and_then(|mirror| mirror.holder);
        let holder = match held {
            Some(holder) => holder,
            None => {
                let made = self.make(Kind::Node)?;
                self.insert(made, view, Some(0))?;
                if let Some(mirror) = self.mirrors.borrow_mut().get_mut(&view) {
                    mirror.holder = Some(made);
                }
                made
            }
        };
        self.put(holder, "x", Prop::Number(x))?;
        self.put(holder, "y", Prop::Number(y))?;
        self.put(holder, "scale", Prop::Triple(scale))?;
        let mut seen = Vec::with_capacity(source.clients.len());
        for item in &source.clients {
            seen.push(item.node);
            let known = self.mirrors.borrow().get(&view).and_then(|mirror| {
                mirror
                    .children
                    .iter()
                    .find(|(client, _)| *client == item.node)
                    .map(|pair| pair.1)
            });
            let child = match known {
                Some(child) => child,
                None => {
                    let made = self.make(Kind::View)?;
                    if let Some(mirror) = self.mirrors.borrow_mut().get_mut(&view) {
                        mirror.children.push((item.node, made));
                    }
                    self.insert(made, holder, None)?;
                    self.put(made, "source", Prop::Node(item.node))?;
                    made
                }
            };
            self.put(child, "x", Prop::Number(item.rect.x - area.x))?;
            self.put(child, "y", Prop::Number(item.rect.y - area.y))?;
            self.put(child, "width", Prop::Number(item.rect.width))?;
            self.put(child, "height", Prop::Number(item.rect.height))?;
            self.put(child, "visible", Prop::Bool(item.visible))?;
        }
        let gone: Vec<(u64, u64)> = {
            let mut mirrors = self.mirrors.borrow_mut();
            let Some(mirror) = mirrors.get_mut(&view) else {
                return Ok(());
            };
            let (keep, gone): (Vec<_>, Vec<_>) =
                mirror.children.drain(..).partition(|(client, _)| seen.contains(client));
            mirror.children = keep;
            gone
        };
        for (_, child) in gone {
            let _ = self.state.borrow_mut().destroy(&self.g, child);
        }
        Ok(())
    }

    /// Puts the node under `parent` at `index` (the end when none).
    pub(crate) fn insert(&self, node: u64, parent: u64, index: Option<usize>) -> Res<()> {
        self.g
            .node(NodeId(node))
            .insert_into(&self.g.node(NodeId(parent)), index)
            .map_err(|error| Fail::new(error.kind().code(), error.to_string()))
    }
}
