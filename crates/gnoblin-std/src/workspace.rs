//! [`Workspace`]: a place windows are laid out in.
//!
//! The compositor does not know workspaces: the std library makes them, keeps their clients as a collection the shell may change, and
//! calls each one's layout when it changes. A workspace is shown by an output (one workspace per output, or one stretched over several:
//! see [`OutputPolicy`](crate::OutputPolicy)). Removing a workspace, or a window from one, never closes the window.
//!
//! ```ignore
//! let main = Workspace::new("main")?;            // made and added, with the default floating layout
//! g.on_client_created(move |window| { main.add(window).ok(); })?;
//! ```
#![allow(unsafe_code)]

use std::time::Duration;

use gnoblin_core::NodeId;
use gnoblin_core_abi::GnoblinStr;
use gnoblin_std_abi::{GnoblinStdShow, GnoblinStdWorkspace, GnoblinStdWorkspaceInfo, GnoblinStdWorkspaceSpec};

use crate::api::Std;
use crate::client::{AsClient, Client};
use crate::collection::{Collection, Member};
use crate::error::Result;
use crate::layout::{FloatingLayout, IntoLayout, RawLayout};
use crate::output::Output;
use gnoblin_std_abi as abi;

/// How a workspace is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShowOptions {
    /// How long the windows slide between the two workspaces; zero switches at once.
    pub duration: Duration,
    /// How much of the slide a touchpad gesture has made already, 0 to 1: the slide carries on from there.
    pub progress: f64,
}

impl ShowOptions {
    /// A slide of `duration`.
    pub fn slide(duration: Duration) -> ShowOptions {
        ShowOptions {
            duration,
            progress: 0.0,
        }
    }
}

/// What a workspace is now.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceInfo {
    /// The id, unique among the workspaces in the list.
    pub id: String,
    /// The name people see.
    pub name: String,
    /// The position in the list, or `None` while the workspace is not in it.
    pub index: Option<usize>,
    /// Whether the workspace is in the list.
    pub in_list: bool,
    /// Whether an output shows the workspace.
    pub active: bool,
    /// The output that shows it, if one does.
    pub output: Option<NodeId>,
    /// The gap between tiles.
    pub gap_inner: f64,
    /// The gap between the tiles and the edge of the area.
    pub gap_outer: f64,
}

/// A workspace. Cheap to clone and compare. A workspace is the shell's own and stays usable after it leaves the list.
#[derive(Clone, PartialEq)]
pub struct Workspace {
    std: Std,
    id: GnoblinStdWorkspace,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Workspace").field(&self.id).finish()
    }
}

/// The options of a workspace, before it is made.
pub struct WorkspaceBuilder {
    std: Std,
    id: String,
    name: Option<String>,
    layout: Option<RawLayout>,
    gaps: (f64, f64),
}

impl WorkspaceBuilder {
    /// The name people see; the id by default.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> WorkspaceBuilder {
        self.name = Some(name.into());
        self
    }

    /// The layout: [`FloatingLayout`] (the default) or any [`Layout`](crate::Layout).
    #[must_use]
    pub fn layout(mut self, layout: impl IntoLayout) -> WorkspaceBuilder {
        self.layout = Some(RawLayout::new(layout, &self.std));
        self
    }

    /// The gap between tiles and the gap between the tiles and the edge of the area, in logical pixels.
    #[must_use]
    pub fn gaps(mut self, inner: f64, outer: f64) -> WorkspaceBuilder {
        self.gaps = (inner, outer);
        self
    }

    /// Makes the workspace without adding it to the list. Add it with [`Workspace::add_to_list`].
    ///
    /// # Errors
    ///
    /// The id is empty or a gap is not a finite number of at least 0.
    pub fn create(self) -> Result<Workspace> {
        let layout = match self.layout {
            Some(layout) => layout,
            None => RawLayout::new(FloatingLayout::default(), &self.std),
        };
        let spec = GnoblinStdWorkspaceSpec {
            size: size_of::<GnoblinStdWorkspaceSpec>() as u32,
            reserved: 0,
            id: GnoblinStr::borrowed(&self.id),
            name: GnoblinStr::borrowed(self.name.as_deref().unwrap_or("")),
            gap_inner: self.gaps.0,
            gap_outer: self.gaps.1,
            layout: layout.spec(),
        };
        let mut id = 0;
        // SAFETY: `spec`, the strings it borrows and the layout's table are live for the call; the library copies what it keeps and,
        // on success, adopts the layout's `user`.
        let rc = unsafe { (self.std.api().workspace_new)(self.std.ctx(), &spec, &mut id) };
        self.std.check(rc)?;
        layout.adopted();
        Ok(Workspace::from_handle(&self.std, id))
    }

    /// Makes the workspace and adds it to the list.
    ///
    /// # Errors
    ///
    /// As for [`create`](WorkspaceBuilder::create), or a workspace with the same id is in the list already.
    pub fn add(self) -> Result<Workspace> {
        let workspace = self.create()?;
        workspace.add_to_list()?;
        Ok(workspace)
    }
}

impl Workspace {
    pub(crate) fn from_handle(std: &Std, id: GnoblinStdWorkspace) -> Workspace {
        Workspace { std: std.clone(), id }
    }

    pub(crate) fn raw(&self) -> GnoblinStdWorkspace {
        self.id
    }

    /// Makes a workspace with the default floating layout and adds it to the list, in the generation that is running (see
    /// [`Std::current`]).
    ///
    /// # Errors
    ///
    /// Called outside a shell's entry, the id is empty, or a workspace with this id is in the list already.
    pub fn new(id: impl Into<String>) -> Result<Workspace> {
        Workspace::builder(id)?.add()
    }

    /// The options of a workspace to make, in the generation that is running.
    ///
    /// # Errors
    ///
    /// Called outside a shell's entry.
    pub fn builder(id: impl Into<String>) -> Result<WorkspaceBuilder> {
        Ok(Workspace::builder_in(&Std::current()?, id))
    }

    /// The options of a workspace to make in `std`'s generation.
    pub fn builder_in(std: &Std, id: impl Into<String>) -> WorkspaceBuilder {
        WorkspaceBuilder {
            std: std.clone(),
            id: id.into(),
            name: None,
            layout: None,
            gaps: (0.0, 0.0),
        }
    }

    /// What the workspace is now.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn info(&self) -> Result<WorkspaceInfo> {
        let mut raw = GnoblinStdWorkspaceInfo {
            size: size_of::<GnoblinStdWorkspaceInfo>() as u32,
            attached: 0,
            id: GnoblinStr::EMPTY,
            name: GnoblinStr::EMPTY,
            index: -1,
            active: 0,
            reserved: 0,
            output: 0,
            gap_inner: 0.0,
            gap_outer: 0.0,
        };
        // SAFETY: `workspace_info` fills `raw` up to the `size` set; the strings it lends are copied before the next call.
        self.std
            .check(unsafe { (self.std.api().workspace_info)(self.std.ctx(), self.id, &mut raw) })?;
        let text = |raw: &GnoblinStr| {
            if raw.ptr.is_null() || raw.len == 0 {
                String::new()
            } else {
                // SAFETY: the library lends `len` bytes at `ptr` until the next call into it on this thread.
                String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) }).into_owned()
            }
        };
        Ok(WorkspaceInfo {
            id: text(&raw.id),
            name: text(&raw.name),
            index: usize::try_from(raw.index).ok(),
            in_list: raw.attached != 0,
            active: raw.active != 0,
            output: (raw.output != 0).then_some(NodeId(raw.output)),
            gap_inner: raw.gap_inner,
            gap_outer: raw.gap_outer,
        })
    }

    /// The workspace's id, which finds it across reloads.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn id(&self) -> Result<String> {
        Ok(self.info()?.id)
    }

    /// The name people see.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn name(&self) -> Result<String> {
        Ok(self.info()?.name)
    }

    /// Sets the name people see.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn set_name(&self, name: &str) -> Result<()> {
        // SAFETY: `workspace_set_name` reads `len` bytes at `name` for the call.
        self.std
            .check(unsafe { (self.std.api().workspace_set_name)(self.std.ctx(), self.id, name.as_ptr(), name.len()) })
            .map(|_| ())
    }

    /// The position in the list, or `None` while the workspace is not in it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn index(&self) -> Result<Option<usize>> {
        Ok(self.info()?.index)
    }

    /// Whether an output shows this workspace. With several outputs, several workspaces are active.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn is_active(&self) -> Result<bool> {
        Ok(self.info()?.active)
    }

    /// The output that shows this workspace, or `None` while none does.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn output(&self) -> Result<Option<Output>> {
        Ok(self.info()?.output.and_then(|node| Output::from_raw(&self.std, node.0)))
    }

    /// The gap between tiles and the gap between the tiles and the edge of the area: `(inner, outer)`.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn gaps(&self) -> Result<(f64, f64)> {
        let info = self.info()?;
        Ok((info.gap_inner, info.gap_outer))
    }

    /// Sets the gaps, in logical pixels; the workspace computes again.
    ///
    /// # Errors
    ///
    /// A gap is not a finite number of at least 0.
    pub fn set_gaps(&self, inner: f64, outer: f64) -> Result<()> {
        // SAFETY: `workspace_set_gaps` takes the context, a workspace handle and two numbers.
        self.std
            .check(unsafe { (self.std.api().workspace_set_gaps)(self.std.ctx(), self.id, inner, outer) })
            .map(|_| ())
    }

    /// Gives the workspace another layout. The windows stay where they are until it computes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn set_layout(&self, layout: impl IntoLayout) -> Result<()> {
        let raw = RawLayout::new(layout, &self.std);
        // SAFETY: the layout's table and `user` are live for the call; on success the library adopts `user`.
        let rc = unsafe { (self.std.api().workspace_set_layout)(self.std.ctx(), self.id, raw.spec()) };
        self.std.check(rc)?;
        raw.adopted();
        Ok(())
    }

    /// Tells the workspace that its layout changed in a way the layout's revision did not show: it computes again.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn invalidate(&self) -> Result<()> {
        // SAFETY: `workspace_invalidate` takes the context and a workspace handle.
        self.std
            .check(unsafe { (self.std.api().workspace_invalidate)(self.std.ctx(), self.id) })
            .map(|_| ())
    }

    /// The workspace's clients, which the shell may change: [`add`](Workspace::add), [`remove`](Workspace::remove) and
    /// [`swap`](Workspace::swap) are the same as on the collection.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn clients(&self) -> Result<Collection<Client>> {
        let mut collection = 0;
        // SAFETY: `workspace_clients` writes one handle through the pointer it is given, a live local.
        self.std
            .check(unsafe { (self.std.api().workspace_clients)(self.std.ctx(), self.id, &mut collection) })?;
        Ok(Collection::from_raw(self.std.clone(), collection))
    }

    /// Adds a client to the workspace: it leaves the one it was on. The layout is told it arrived.
    ///
    /// # Errors
    ///
    /// The workspace is not in the list, or the client is gone.
    pub fn add(&self, client: impl AsClient) -> Result<()> {
        let clients = self.clients()?;
        // SAFETY: `collection_add` takes the context, a collection and a member id.
        self.std
            .check(unsafe { (self.std.api().collection_add)(self.std.ctx(), clients.raw_id(), client.client_id().0) })
            .map(|_| ())
    }

    /// Takes a client off the workspace: `true` when it was there. The client is not closed and is on no workspace until it is given
    /// one.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn remove(&self, client: impl AsClient) -> Result<bool> {
        let clients = self.clients()?;
        // SAFETY: `collection_remove` takes the context, a collection and a member id.
        let removed = self.std.check(unsafe {
            (self.std.api().collection_remove)(self.std.ctx(), clients.raw_id(), client.client_id().0)
        })?;
        Ok(removed == 1)
    }

    /// Trades the places of two clients in the workspace's order; its layout hears of it.
    ///
    /// # Errors
    ///
    /// A client is not on the workspace.
    pub fn swap(&self, a: impl AsClient, b: impl AsClient) -> Result<()> {
        let clients = self.clients()?;
        // SAFETY: `collection_swap` takes the context, a collection and two member ids.
        self.std
            .check(unsafe {
                (self.std.api().collection_swap)(self.std.ctx(), clients.raw_id(), a.client_id().0, b.client_id().0)
            })
            .map(|_| ())
    }

    /// Adds the workspace to the list and gives it the windows that wait for one.
    ///
    /// # Errors
    ///
    /// A workspace with the same id is in the list already.
    pub fn add_to_list(&self) -> Result<()> {
        // SAFETY: `workspace_add` takes the context and a workspace handle.
        self.std
            .check(unsafe { (self.std.api().workspace_add)(self.std.ctx(), self.id) })
            .map(|_| ())
    }

    /// Takes the workspace out of the list: `true` when it left, `false` when it was refused (the last workspace stays). Its windows
    /// move to the workspace before it (after it, for the first) and none is closed.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn remove_from_list(&self) -> Result<bool> {
        // SAFETY: `workspace_remove` takes the context and a workspace handle.
        let removed = self
            .std
            .check(unsafe { (self.std.api().workspace_remove)(self.std.ctx(), self.id) })?;
        Ok(removed == 1)
    }

    /// Shows the workspace on the output under the pointer (the primary output when no pointer says), at once.
    ///
    /// # Errors
    ///
    /// The workspace is not in the list, or no output can show it.
    pub fn show(&self) -> Result<()> {
        self.show_with(ShowOptions::default())
    }

    /// Shows the workspace on the output under the pointer, sliding the windows as `options` say.
    ///
    /// # Errors
    ///
    /// As for [`show`](Workspace::show).
    pub fn show_with(&self, options: ShowOptions) -> Result<()> {
        self.show_raw(options, 0)
    }

    /// Shows the workspace on `output`, swapping it with the one that output showed.
    ///
    /// # Errors
    ///
    /// As for [`show`](Workspace::show).
    pub fn show_on(&self, output: &Output, options: ShowOptions) -> Result<()> {
        self.show_raw(options, output.id().0)
    }

    fn show_raw(&self, options: ShowOptions, output: u64) -> Result<()> {
        let show = GnoblinStdShow {
            size: size_of::<GnoblinStdShow>() as u32,
            reserved: 0,
            duration_ms: options.duration.as_secs_f64() * 1000.0,
            progress: options.progress,
            output,
        };
        // SAFETY: `workspace_show` reads the struct for the call.
        self.std
            .check(unsafe { (self.std.api().workspace_show)(self.std.ctx(), self.id, &show) })
            .map(|_| ())
    }

    /// Moves the keyboard `step` windows along the workspace's windows, in the order they are listed: 1 to the next, -1 to the
    /// previous. Nothing moves when no window of the workspace has the keyboard, and a step past either end leaves it where it is.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn walk(&self, step: i64) -> Result<()> {
        // SAFETY: `workspace_walk` takes the context, a workspace handle and a step.
        self.std
            .check(unsafe { (self.std.api().workspace_walk)(self.std.ctx(), self.id, step) })
            .map(|_| ())
    }
}

impl Member for Workspace {
    const KIND: u32 = abi::STD_COLLECTION_WORKSPACES;

    fn from_raw(std: &Std, raw: u64) -> Option<Workspace> {
        Some(Workspace::from_handle(std, raw))
    }

    fn raw(&self) -> u64 {
        self.id
    }

    fn key(&self) -> String {
        self.id().unwrap_or_default()
    }
}
