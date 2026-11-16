//! The Rust side of workspaces: what a reload carries from one shell to the next.
//!
//! A workspace is a script object (`Workspace` in `data/js/gnoblin-core/host.ts`), so the compositor does not know which
//! workspace a window is on. A new generation starts without that knowledge. [`JsWorkspaceSnapshot`] is
//! what the old shell hands over ([`JsShell::workspace_snapshot`](crate::JsShell::workspace_snapshot)) and
//! the new shell reconciles by workspace id ([`JsShell::reconcile`](crate::JsShell::reconcile)).

use gnoblin_tree_client::tree_mirror::NodeId;
use rquickjs::{Array, Ctx, Object};

/// One workspace of a snapshot: its id and the windows it held, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsWorkspaceEntry {
    pub id: String,
    pub clients: Vec<NodeId>,
}

/// What one output showed: the output's name and the id of its workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsOutputShown {
    pub output: String,
    pub workspace: String,
}

/// A rectangle as the shell keeps it: `[x, y, width, height]`.
pub type JsKeptRect = [f64; 4];

/// The room a window's frame takes on each side of its content, as the shell keeps it: `[top, right, bottom, left]`.
pub type JsKeptInset = [f64; 4];

/// What a shell remembers of one window beyond where it is: the rectangles it goes back to when it stops being
/// maximised, fullscreen or snapped. They live in the shell's script, so a reload would lose them and leave a window that
/// was maximised when the config changed with nothing to return to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsKeptWindow {
    pub node: NodeId,
    /// The rectangle a maximised window had before it was maximised.
    pub floating: Option<JsKeptRect>,
    /// The rectangle a fullscreen window had before it was.
    pub windowed: Option<JsKeptRect>,
    /// Where the user left a floating window (`floatingBounds`).
    pub left_at: Option<JsKeptRect>,
    /// The zone a snapped window sits in and the rectangle unsnapping returns it to.
    pub snapped: Option<(String, JsKeptRect)>,
    /// The room the window's frame kept when the window was last fitted. A reload that turns decorations off or on, or
    /// changes the frame, makes the next generation compare it with the inset the window has then, so the frame keeps
    /// its place across the reload.
    pub inset: Option<JsKeptInset>,
}

/// The workspaces of a shell, in order, the id of the one the primary output showed, the one each output showed, and the
/// windows with rectangles to go back to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsWorkspaceSnapshot {
    pub active: Option<String>,
    pub workspaces: Vec<JsWorkspaceEntry>,
    pub outputs: Vec<JsOutputShown>,
    pub kept: Vec<JsKeptWindow>,
}

/// Four numbers under `key`: a kept rectangle or a kept inset. `None` when absent or not four numbers.
fn four_from<'js>(object: &Object<'js>, key: &str) -> rquickjs::Result<Option<[f64; 4]>> {
    let Some(numbers) = object.get::<_, Option<Vec<f64>>>(key)? else {
        return Ok(None);
    };
    Ok(<[f64; 4]>::try_from(numbers.as_slice()).ok())
}

impl JsWorkspaceSnapshot {
    /// Reads the object `entry.snapshot()` returns.
    pub(crate) fn from_js<'js>(value: &Object<'js>) -> rquickjs::Result<Self> {
        let active: Option<String> = value.get("active")?;
        let list: Array<'js> = value.get("workspaces")?;
        let mut workspaces = Vec::with_capacity(list.len());
        for item in list.iter::<Object<'js>>() {
            let item = item?;
            let clients: Vec<f64> = item.get("clients")?;
            workspaces.push(JsWorkspaceEntry {
                id: item.get("id")?,
                // The ids are node ids the shell turned into numbers, so each is exact.
                clients: clients.into_iter().map(|id| id as NodeId).collect(),
            });
        }
        let shown: Array<'js> = value.get("outputs")?;
        let mut outputs = Vec::with_capacity(shown.len());
        for item in shown.iter::<Object<'js>>() {
            let item = item?;
            outputs.push(JsOutputShown {
                output: item.get("output")?,
                workspace: item.get("workspace")?,
            });
        }
        let mut kept = Vec::new();
        // An older shell hands over no `kept`: nothing to restore.
        if let Some(list) = value.get::<_, Option<Array<'js>>>("kept")? {
            for item in list.iter::<Object<'js>>() {
                let item = item?;
                let node: f64 = item.get("node")?;
                let snapped = match item.get::<_, Option<String>>("zone")? {
                    Some(zone) => four_from(&item, "snapFloating")?.map(|rect| (zone, rect)),
                    None => None,
                };
                kept.push(JsKeptWindow {
                    node: node as NodeId,
                    floating: four_from(&item, "floating")?,
                    windowed: four_from(&item, "windowed")?,
                    left_at: four_from(&item, "leftAt")?,
                    snapped,
                    inset: four_from(&item, "inset")?,
                });
            }
        }
        Ok(Self {
            active,
            workspaces,
            outputs,
            kept,
        })
    }

    /// The object `entry.reconcile(previous)` takes.
    pub(crate) fn to_js<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let list = Array::new(ctx.clone())?;
        for (index, entry) in self.workspaces.iter().enumerate() {
            let item = Object::new(ctx.clone())?;
            item.set("id", entry.id.as_str())?;
            item.set(
                "clients",
                entry.clients.iter().map(|id| *id as f64).collect::<Vec<f64>>(),
            )?;
            list.set(index, item)?;
        }
        let shown = Array::new(ctx.clone())?;
        for (index, entry) in self.outputs.iter().enumerate() {
            let item = Object::new(ctx.clone())?;
            item.set("output", entry.output.as_str())?;
            item.set("workspace", entry.workspace.as_str())?;
            shown.set(index, item)?;
        }
        let kept = Array::new(ctx.clone())?;
        for (index, window) in self.kept.iter().enumerate() {
            let item = Object::new(ctx.clone())?;
            item.set("node", window.node as f64)?;
            item.set("floating", window.floating.map(Vec::from))?;
            item.set("windowed", window.windowed.map(Vec::from))?;
            item.set("leftAt", window.left_at.map(Vec::from))?;
            if let Some((zone, rect)) = &window.snapped {
                item.set("zone", zone.as_str())?;
                item.set("snapFloating", Vec::from(*rect))?;
            }
            item.set("inset", window.inset.map(Vec::from))?;
            kept.set(index, item)?;
        }
        let object = Object::new(ctx.clone())?;
        object.set("active", self.active.as_deref())?;
        object.set("workspaces", list)?;
        object.set("outputs", shown)?;
        object.set("kept", kept)?;
        Ok(object)
    }
}
