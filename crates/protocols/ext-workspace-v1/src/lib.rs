//! XML protocol `ext-workspace-v1`: the interfaces `ext_workspace_manager_v1`, `ext_workspace_group_handle_v1` and
//! `ext_workspace_handle_v1`.
//!
//! A panel or an overview lists the session's workspaces and shows which one is active. This module serves that read
//! side. The compositor keeps no workspaces of its own: the shell arranges them (the workspaces the configuration
//! declares, `Seat.workspaces`, and the one the shell shows, `Seat.active_workspace`), so what a client hears is what
//! those two properties say, and a session that declares none has the one implicit workspace, id `1`.
//!
//! ## What a client sees
//!
//! - One workspace group holding every output the session has (`output_enter` for each `wl_output` the client bound,
//!   `output_leave` when an output goes), and every workspace: workspaces are not per output here, because the shell's
//!   are not.
//! - Each workspace has its `id` (the declared id), its `name`, and the `active` state when it is the one the shell
//!   named in `Seat.active_workspace`. With no shell, or one that has not named a workspace, a session with a single
//!   workspace reports it active and a session with several reports none, because the compositor does not know.
//! - A client that binds late is told everything it missed in one batch ending in `done`; later changes arrive the
//!   same way, one `done` per batch, from [`follow`], which the session runs once per turn.
//!
//! ## What a client cannot do
//!
//! The group and every workspace advertise **no capabilities**: `activate`, `deactivate`, `remove`, `assign` and
//! `create_workspace` are not offered, and a client that sends them anyway is ignored, as the protocol allows for a
//! request whose capability is absent. Switching a workspace is the shell's act and the session has no request that
//! carries a client's wish to it; the capability bits are for that request to switch on when it exists, and a client
//! that honours them (it must) shows a read-only list until then.
//!
//! ## Who may bind it
//!
//! The list names every workspace the user has, which is the shape of their work, so the manager is a privileged
//! global ([`crate::access::Global::Workspace`]): the session's panels and tools see it on the privileged socket, an
//! application on the ordinary socket does not, and a sandbox never does.
//!
//! ## Failure modes
//!
//! Each is a test in `tests/ext_workspace.rs`:
//!
//! 1. **An application learns the workspace layout**: the manager is in `tests/access.rs`'s privileged list.
//! 2. **A client binds and is told nothing**: `a_client_is_told_the_session_workspaces_when_it_binds`.
//! 3. **The declared workspaces are not what is listed**: `declared_workspaces_are_listed_with_their_names`.
//! 4. **The active workspace is stale or invented**: `the_shells_active_workspace_is_the_active_one` and
//!    `no_shell_and_several_workspaces_means_none_active`.
//! 5. **A change is sent without its `done`, or a late binder misses the state**: every test reads only up to a
//!    `done`, and `a_late_client_is_told_the_same_state`.
//! 6. **A request the compositor does not offer moves something**: `requests_without_capabilities_are_ignored`.
//! 7. **A stopped manager keeps being sent events**: `a_stopped_manager_is_finished_and_forgotten`.

use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1, GroupCapabilities},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1, State as WorkspaceState, WorkspaceCapabilities},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};
use smithay::reexports::wayland_server::backend::{ClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// The version of `ext_workspace_manager_v1` this session serves.
pub const MANAGER_VERSION: u32 = 1;

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-workspace-v1",
    interfaces: &[ServedInterface::new("ext_workspace_manager_v1", MANAGER_VERSION)],
    privilege: Privilege::Roster(Global::Workspace),
};

/// The id of the workspace a session with none declared has.
pub(crate) const IMPLICIT_ID: &str = "1";

/// One workspace as a client is told it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceInfo {
    /// The declared id, which is stable for the workspace's life.
    pub id: String,
    /// The name a panel shows.
    pub name: String,
    /// Whether the shell shows it now.
    pub active: bool,
}

/// What the session has now: its workspaces and its outputs.
pub struct Snapshot {
    /// The workspaces, in declaration order.
    pub workspaces: Vec<WorkspaceInfo>,
    /// The outputs, which are all in the one group.
    pub outputs: Vec<Output>,
}

/// What this module asks of the compositor. The whole of its reach into the compositor is these three methods, so the
/// protocol can be served from outside this crate's state.
pub trait Host: Sized + 'static {
    /// The workspaces and outputs the session has now.
    fn workspace_snapshot(&self) -> Snapshot;
    /// The display, which creates the objects a client is sent.
    fn workspace_display(&self) -> DisplayHandle;
    /// The protocol's bookkeeping.
    fn workspaces(&mut self) -> &mut Workspaces;
}

/// The workspaces a session has, from its declared `id:name` items and the id the shell named active.
///
/// An item with no colon is an id that is also its name. No declared items is the one implicit workspace.
pub fn workspaces_of(declared: &[String], active: Option<&str>) -> Vec<WorkspaceInfo> {
    let mut workspaces: Vec<WorkspaceInfo> = declared
        .iter()
        .map(|item| {
            let (id, name) = item.split_once(':').unwrap_or((item.as_str(), item.as_str()));
            WorkspaceInfo {
                id: id.to_owned(),
                name: name.to_owned(),
                active: false,
            }
        })
        .collect();
    if workspaces.is_empty() {
        workspaces.push(WorkspaceInfo {
            id: IMPLICIT_ID.to_owned(),
            name: IMPLICIT_ID.to_owned(),
            active: false,
        });
    }
    let named = active.and_then(|id| workspaces.iter().position(|workspace| workspace.id == id));
    match named {
        Some(index) => workspaces[index].active = true,
        // The shell named none, or one that is not listed: with a single workspace there is no doubt which it is.
        None if workspaces.len() == 1 => workspaces[0].active = true,
        None => {}
    }
    workspaces
}

/// What one bound manager has been told.
struct Bound {
    manager: ExtWorkspaceManagerV1,
    /// The group, once it has been sent.
    group: Option<ExtWorkspaceGroupHandleV1>,
    /// The `wl_output` objects of this client the group has been told about.
    outputs: Vec<WlOutput>,
    /// The workspaces this client has handles for, with what it was last told of each.
    workspaces: Vec<(WorkspaceInfo, ExtWorkspaceHandleV1)>,
}

/// The global and every bound manager.
pub struct Workspaces {
    /// Kept alive so `ext_workspace_manager_v1` stays registered; the bind is gated by the access roster.
    #[allow(dead_code)]
    global: GlobalId,
    bound: Vec<Bound>,
}

/// The global data of the manager: the roster's answer for who may bind it.
pub struct ManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

/// The user data of a bound manager.
pub struct ManagerUserData;

/// The user data of a group handle.
pub struct GroupUserData;

/// The user data of a workspace handle.
pub struct WorkspaceUserData;

/// Creates the `ext_workspace_manager_v1` global behind the access roster.
pub fn register<D>(registrar: &Registrar<'_>) -> Workspaces
where
    D: GlobalDispatch<ExtWorkspaceManagerV1, ManagerGlobalData> + 'static,
{
    Workspaces {
        global: registrar.display().create_global::<D, ExtWorkspaceManagerV1, _>(
            MANAGER_VERSION,
            ManagerGlobalData {
                filter: Box::new(registrar.filter(Global::Workspace)),
            },
        ),
        bound: Vec::new(),
    }
}

/// Brings every bound manager up to date with the session, one `done` per manager that heard a change. Called once per
/// turn and when a client binds.
pub fn follow<H>(host: &mut H)
where
    H: Host + Dispatch<ExtWorkspaceGroupHandleV1, GroupUserData> + Dispatch<ExtWorkspaceHandleV1, WorkspaceUserData>,
{
    let snapshot = host.workspace_snapshot();
    let display = host.workspace_display();
    let workspaces = host.workspaces();
    workspaces.bound.retain(|bound| bound.manager.is_alive());
    for bound in &mut workspaces.bound {
        if sync::<H>(&display, bound, &snapshot) {
            bound.manager.done();
        }
    }
}

/// Sends one manager what changed since it was last told. `true` when something was sent.
fn sync<D>(display: &DisplayHandle, bound: &mut Bound, snapshot: &Snapshot) -> bool
where
    D: Dispatch<ExtWorkspaceGroupHandleV1, GroupUserData> + Dispatch<ExtWorkspaceHandleV1, WorkspaceUserData> + 'static,
{
    let Some(client) = bound.manager.client() else {
        return false;
    };
    let mut changed = false;

    // The group exists from the first sync on.
    if bound.group.is_none() {
        let Ok(created) =
            client.create_resource::<ExtWorkspaceGroupHandleV1, _, D>(display, bound.manager.version(), GroupUserData)
        else {
            return false;
        };
        bound.manager.workspace_group(&created);
        created.capabilities(GroupCapabilities::empty());
        bound.group = Some(created);
        changed = true;
    }
    let Some(group) = bound.group.clone() else {
        return changed;
    };

    // Outputs: the `wl_output` objects this client bound, entered once and left when they go.
    let wanted: Vec<WlOutput> = snapshot
        .outputs
        .iter()
        .flat_map(|output| output.client_outputs(&client))
        .collect();
    bound.outputs.retain(|held| {
        let keep = held.is_alive() && wanted.contains(held);
        if !keep && held.is_alive() {
            group.output_leave(held);
            changed = true;
        }
        keep
    });
    for output in wanted {
        if !bound.outputs.contains(&output) {
            group.output_enter(&output);
            bound.outputs.push(output);
            changed = true;
        }
    }

    // Workspaces that are gone.
    bound.workspaces.retain(|(info, handle)| {
        let keep = snapshot.workspaces.iter().any(|current| current.id == info.id);
        if !keep && handle.is_alive() {
            group.workspace_leave(handle);
            handle.removed();
            changed = true;
        }
        keep
    });
    // Workspaces that are new or changed, in declaration order.
    for current in &snapshot.workspaces {
        match bound.workspaces.iter_mut().find(|(info, _)| info.id == current.id) {
            Some((info, handle)) => {
                if info.name != current.name {
                    handle.name(current.name.clone());
                    changed = true;
                }
                if info.active != current.active {
                    handle.state(state_of(current));
                    changed = true;
                }
                *info = current.clone();
            }
            None => {
                let Ok(created) = client.create_resource::<ExtWorkspaceHandleV1, _, D>(
                    display,
                    bound.manager.version(),
                    WorkspaceUserData,
                ) else {
                    continue;
                };
                bound.manager.workspace(&created);
                created.id(current.id.clone());
                created.name(current.name.clone());
                created.state(state_of(current));
                created.capabilities(WorkspaceCapabilities::empty());
                group.workspace_enter(&created);
                bound.workspaces.push((current.clone(), created));
                changed = true;
            }
        }
    }
    changed
}

/// The state bits a workspace has.
fn state_of(workspace: &WorkspaceInfo) -> WorkspaceState {
    if workspace.active {
        WorkspaceState::Active
    } else {
        WorkspaceState::empty()
    }
}

impl<D> GlobalDispatch2<ExtWorkspaceManagerV1, D> for ManagerGlobalData
where
    D: Host
        + Dispatch<ExtWorkspaceManagerV1, ManagerUserData>
        + Dispatch<ExtWorkspaceGroupHandleV1, GroupUserData>
        + Dispatch<ExtWorkspaceHandleV1, WorkspaceUserData>
        + 'static,
{
    fn bind(
        &self,
        state: &mut D,
        _dh: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        let bound = data_init.init(resource, ManagerUserData);
        state.workspaces().bound.push(Bound {
            manager: bound,
            group: None,
            outputs: Vec::new(),
            workspaces: Vec::new(),
        });
        log::debug!(area = "workspace", client = ?client.id(), "a client bound the workspace manager");
        // The client hears the session's state now, which is what its `done` is for.
        follow(state);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D: Host> Dispatch2<ExtWorkspaceManagerV1, D> for ManagerUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        manager_object: &ExtWorkspaceManagerV1,
        request: manager::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            // Nothing a client can ask for is pending, so there is nothing to apply.
            manager::Request::Commit => {}
            manager::Request::Stop => {
                state
                    .workspaces()
                    .bound
                    .retain(|bound| bound.manager != *manager_object);
                manager_object.finished();
                log::debug!(area = "workspace", "a client stopped its workspace manager");
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, manager_object: &ExtWorkspaceManagerV1) {
        state
            .workspaces()
            .bound
            .retain(|bound| bound.manager != *manager_object);
    }
}

impl<D> Dispatch2<ExtWorkspaceGroupHandleV1, D> for GroupUserData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _group: &ExtWorkspaceGroupHandleV1,
        request: group::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // `create_workspace` needs a capability this group does not advertise, so it is ignored; `destroy` is the
        // destructor, which Smithay carries out.
        if let group::Request::CreateWorkspace { workspace } = request {
            log::debug!(area = "workspace", name = %workspace, "a create_workspace request without its capability was ignored");
        }
    }
}

impl<D: Host> Dispatch2<ExtWorkspaceHandleV1, D> for WorkspaceUserData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _workspace: &ExtWorkspaceHandleV1,
        request: handle::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // Every request but `destroy` needs a capability this workspace does not advertise, so it is ignored.
        if !matches!(request, handle::Request::Destroy) {
            log::debug!(
                area = "workspace",
                "a workspace request without its capability was ignored"
            );
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, workspace: &ExtWorkspaceHandleV1) {
        for bound in &mut state.workspaces().bound {
            bound.workspaces.retain(|(_, handle)| handle != workspace);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceInfo, workspaces_of};

    fn declared(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn active_ids(list: &[WorkspaceInfo]) -> Vec<&str> {
        list.iter()
            .filter(|workspace| workspace.active)
            .map(|workspace| workspace.id.as_str())
            .collect()
    }

    #[test]
    fn none_declared_is_one_implicit_workspace_that_is_active() {
        let list = workspaces_of(&[], None);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.as_str(), list[0].name.as_str()), ("1", "1"));
        assert!(list[0].active);
    }

    #[test]
    fn an_item_is_an_id_and_a_name() {
        let list = workspaces_of(&declared(&["code:Code", "web:web", "plain"]), None);
        let pairs: Vec<(&str, &str)> = list.iter().map(|w| (w.id.as_str(), w.name.as_str())).collect();
        assert_eq!(pairs, [("code", "Code"), ("web", "web"), ("plain", "plain")]);
    }

    #[test]
    fn the_named_workspace_is_the_only_active_one() {
        let list = workspaces_of(&declared(&["a:A", "b:B", "c:C"]), Some("b"));
        assert_eq!(active_ids(&list), ["b"]);
    }

    #[test]
    fn several_workspaces_and_no_name_is_none_active() {
        let list = workspaces_of(&declared(&["a:A", "b:B"]), None);
        assert!(active_ids(&list).is_empty());
        let list = workspaces_of(&declared(&["a:A", "b:B"]), Some("gone"));
        assert!(active_ids(&list).is_empty(), "an id that is not listed names nothing");
    }

    #[test]
    fn a_single_declared_workspace_is_active_whatever_the_shell_says() {
        let list = workspaces_of(&declared(&["only:Only"]), Some("gone"));
        assert_eq!(active_ids(&list), ["only"]);
    }
}
