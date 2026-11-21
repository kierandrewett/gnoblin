//! [`Workspace`]: a place windows are laid out in.
//!
//! The compositor does not know workspaces: the std library makes them, keeps their clients as a collection the shell may change, and
//! calls each one's layout when it changes. A workspace is shown by an output (one workspace per output, or one stretched over several:
//! see [`OutputPolicy`](crate::OutputPolicy)). Removing a workspace, or a window from one, never closes the window.
//!
//! A `Workspace` is a name for the class `workspace` of the object protocol (`crate::objects::Workspace`): every call goes through the
//! generated class, so a shell in Rust, Python or TypeScript sees one set of workspaces.
//!
//! ```ignore
//! let main = Workspace::new("main")?;            // made and added, with the default floating layout
//! g.on_client_created(move |window| { main.add(window).ok(); })?;
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_core::NodeId;

use crate::api::Std;
use crate::client::{AsClient, Client};
use crate::collection::{Collection, Member};
use crate::error::{Error, ErrorKind, Result};
use crate::layout::{FloatingLayout, IntoLayout, LayoutSpec};
use crate::objects as classes;
use crate::output::Output;

thread_local! {
    /// The workspaces this shell made, by id, for the ones that are not in the list.
    static MADE: RefCell<HashMap<String, Rc<classes::Workspace>>> = RefCell::new(HashMap::new());
}

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
#[derive(Clone)]
pub struct Workspace {
    std: Std,
    object: Rc<classes::Workspace>,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Workspace) -> bool {
        self.std == other.std && self.object.handle().handle() == other.object.handle().handle()
    }
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Workspace")
            .field(&self.object.handle().handle())
            .finish()
    }
}

/// The options of a workspace, before it is made.
pub struct WorkspaceBuilder {
    std: Std,
    id: String,
    name: Option<String>,
    layout: Option<LayoutSpec>,
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
        self.layout = Some(layout.into_layout());
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
        if self.id.is_empty() {
            return Err(Error::new(ErrorKind::Refused, "a workspace needs an id"));
        }
        let objects = self.std.objects()?;
        let object = classes::Workspace::new(&objects, &self.id)?;
        let object = Rc::new(object);
        MADE.with(|made| made.borrow_mut().insert(self.id.clone(), Rc::clone(&object)));
        let workspace = Workspace { std: self.std, object };
        if let Some(name) = &self.name {
            workspace.set_name(name)?;
        }
        workspace.set_gaps(self.gaps.0, self.gaps.1)?;
        let layout = self.layout.unwrap_or(LayoutSpec::Floating(FloatingLayout::default()));
        workspace.give_layout(layout)?;
        Ok(workspace)
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
    /// The workspace of the class object `object`.
    pub(crate) fn from_object(std: &Std, object: classes::Workspace) -> Workspace {
        Workspace {
            std: std.clone(),
            object: Rc::new(object),
        }
    }

    /// The workspace the library keeps under `id`: one in the list, or one this shell made and has not added yet.
    pub(crate) fn of_id(std: &Std, id: &str) -> Result<Workspace> {
        let objects = std.objects()?;
        if let Some(object) = classes::Workspaces::get(&objects)?.by_id(id)? {
            return Ok(Workspace::from_object(std, object));
        }
        MADE.with(|made| made.borrow().get(id).cloned())
            .map(|object| Workspace {
                std: std.clone(),
                object,
            })
            .ok_or_else(|| Error::new(ErrorKind::NoSuchNode, format!("no workspace `{id}`")))
    }

    /// The workspace that the output `output` (a node number) shows, or `None`.
    pub(crate) fn of_output(std: &Std, output: u64) -> Result<Option<Workspace>> {
        let objects = std.objects()?;
        let list = classes::Workspaces::get(&objects)?;
        Ok(list
            .of_output(output)?
            .map(|object| Workspace::from_object(std, object)))
    }

    /// The class object.
    pub(crate) fn object(&self) -> &classes::Workspace {
        &self.object
    }

    pub(crate) fn std(&self) -> &Std {
        &self.std
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
        let gaps = self.object.gaps()?.unwrap_or_default();
        let output = self.object.output()?;
        Ok(WorkspaceInfo {
            id: self.object.id()?,
            name: self.object.name()?,
            index: usize::try_from(self.object.index()?).ok(),
            in_list: self.object.attached()?,
            active: self.object.active()?,
            output: (output != 0).then_some(NodeId(output)),
            gap_inner: gaps.inner,
            gap_outer: gaps.outer,
        })
    }

    /// The workspace's id, which finds it across reloads.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn id(&self) -> Result<String> {
        Ok(self.object.id()?)
    }

    /// The name people see.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn name(&self) -> Result<String> {
        Ok(self.object.name()?)
    }

    /// Sets the name people see.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn set_name(&self, name: &str) -> Result<()> {
        Ok(self.object.set_name(name)?)
    }

    /// The position in the list, or `None` while the workspace is not in it.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn index(&self) -> Result<Option<usize>> {
        Ok(usize::try_from(self.object.index()?).ok())
    }

    /// Whether an output shows this workspace. With several outputs, several workspaces are active.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn is_active(&self) -> Result<bool> {
        Ok(self.object.active()?)
    }

    /// The output that shows this workspace, or `None` while none does.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn output(&self) -> Result<Option<Output>> {
        let output = self.object.output()?;
        Ok((output != 0).then(|| Output::from_raw(&self.std, output)).flatten())
    }

    /// The gap between tiles and the gap between the tiles and the edge of the area: `(inner, outer)`.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn gaps(&self) -> Result<(f64, f64)> {
        let gaps = self.object.gaps()?.unwrap_or_default();
        Ok((gaps.inner, gaps.outer))
    }

    /// Sets the gaps, in logical pixels; the workspace computes again.
    ///
    /// # Errors
    ///
    /// A gap is not a finite number of at least 0.
    pub fn set_gaps(&self, inner: f64, outer: f64) -> Result<()> {
        Ok(self.object.set_gaps(&classes::Gaps { inner, outer })?)
    }

    /// Gives the workspace another layout. The windows stay where they are until it computes.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn set_layout(&self, layout: impl IntoLayout) -> Result<()> {
        self.give_layout(layout.into_layout())
    }

    fn give_layout(&self, spec: LayoutSpec) -> Result<()> {
        // The workspace holds the layout it is given.
        let layout = crate::layout::make(&self.std, spec)?;
        Ok(self.object.set_layout(&layout)?)
    }

    /// Tells the workspace that its layout changed in a way the layout's revision did not show: it computes again.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn invalidate(&self) -> Result<()> {
        Ok(self.object.invalidate()?)
    }

    /// The workspace's clients, which the shell may change: [`add`](Workspace::add), [`remove`](Workspace::remove) and
    /// [`swap`](Workspace::swap) are the same as on the collection.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn clients(&self) -> Result<Collection<Client>> {
        let clients = self
            .object
            .clients()?
            .ok_or_else(|| Error::new(ErrorKind::Failed, "the workspace has no clients"))?;
        Ok(Collection::of_workspace(&self.std, &self.object.id()?, clients))
    }

    /// Adds a client to the workspace: it leaves the one it was on. The layout is told it arrived.
    ///
    /// # Errors
    ///
    /// The workspace is not in the list, or the client is gone.
    pub fn add(&self, client: impl AsClient) -> Result<()> {
        self.clients()?.add(client)
    }

    /// Takes a client off the workspace: `true` when it was there. The client is not closed and is on no workspace until it is given
    /// one.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn remove(&self, client: impl AsClient) -> Result<bool> {
        self.clients()?.remove(client)
    }

    /// Trades the places of two clients in the workspace's order; its layout hears of it.
    ///
    /// # Errors
    ///
    /// A client is not on the workspace.
    pub fn swap(&self, a: impl AsClient, b: impl AsClient) -> Result<()> {
        self.clients()?.swap(a, b)
    }

    /// Adds the workspace to the list and gives it the windows that wait for one.
    ///
    /// # Errors
    ///
    /// A workspace with the same id is in the list already.
    pub fn add_to_list(&self) -> Result<()> {
        Ok(classes::Workspaces::get(&self.std.objects()?)?.add(&self.object)?)
    }

    /// Takes the workspace out of the list: `true` when it left, `false` when it was refused (the last workspace stays). Its windows
    /// move to the workspace before it (after it, for the first) and none is closed.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn remove_from_list(&self) -> Result<bool> {
        Ok(classes::Workspaces::get(&self.std.objects()?)?.remove(&self.object)?)
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
        self.show_raw(options, None)
    }

    /// Shows the workspace on `output`, swapping it with the one that output showed.
    ///
    /// # Errors
    ///
    /// As for [`show`](Workspace::show).
    pub fn show_on(&self, output: &Output, options: ShowOptions) -> Result<()> {
        self.show_raw(options, Some(output.id().0))
    }

    fn show_raw(&self, options: ShowOptions, output: Option<u64>) -> Result<()> {
        let show = classes::ShowOptions {
            duration: options.duration.as_secs_f64() * 1000.0,
            progress: options.progress,
            output,
        };
        Ok(self.object.show(Some(&show))?)
    }

    /// Moves the keyboard `step` windows along the workspace's windows, in the order they are listed: 1 to the next, -1 to the
    /// previous. Nothing moves when no window of the workspace has the keyboard, and a step past either end leaves it where it is.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn walk(&self, step: i64) -> Result<()> {
        Ok(self.object.walk(step)?)
    }
}

impl Member for Workspace {
    fn from_key(std: &Std, key: &str) -> Option<Workspace> {
        Workspace::of_id(std, key).ok()
    }

    fn key(&self) -> String {
        self.id().unwrap_or_default()
    }
}
