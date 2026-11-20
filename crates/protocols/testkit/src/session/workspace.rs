//! The workspace-list client the tests of `ext_workspace_v1` drive: what the manager, its groups and its workspaces are told is
//! recorded in [`App::workspace_view`].

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};

use super::{App, WorkspaceSeen};

impl Dispatch<ExtWorkspaceManagerV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Workspace { workspace } => state.workspace_view.workspaces.push((
                workspace.id().protocol_id(),
                WorkspaceSeen {
                    object: Some(workspace.id()),
                    ..WorkspaceSeen::default()
                },
            )),
            manager::Event::WorkspaceGroup { workspace_group } => {
                state.workspace_view.group = Some(workspace_group.id());
            }
            manager::Event::Done => state.workspace_view.dones += 1,
            manager::Event::Finished => state.workspace_view.finished = true,
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, ExtWorkspaceManagerV1, [
        manager::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        manager::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceGroupHandleV1,
        event: group::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let view = &mut state.workspace_view;
        match event {
            group::Event::Capabilities { capabilities } => {
                view.group_capabilities = Some(match capabilities {
                    WEnum::Value(bits) => bits.bits(),
                    WEnum::Unknown(bits) => bits,
                });
            }
            group::Event::OutputEnter { .. } => view.outputs_entered += 1,
            group::Event::OutputLeave { .. } => view.outputs_left += 1,
            group::Event::WorkspaceEnter { workspace } => view.in_group.push(workspace.id().protocol_id()),
            group::Event::WorkspaceLeave { workspace } => {
                let id = workspace.id().protocol_id();
                view.in_group.retain(|held| *held != id);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        workspace: &ExtWorkspaceHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = workspace.id().protocol_id();
        let Some((_, seen)) = state.workspace_view.workspaces.iter_mut().find(|(held, _)| *held == id) else {
            return;
        };
        match event {
            handle::Event::Id { id } => seen.id = id,
            handle::Event::Name { name } => seen.name = name,
            handle::Event::State { state } => {
                seen.state = match state {
                    WEnum::Value(bits) => bits.bits(),
                    WEnum::Unknown(bits) => bits,
                };
            }
            handle::Event::Capabilities { capabilities } => {
                seen.capabilities = Some(match capabilities {
                    WEnum::Value(bits) => bits.bits(),
                    WEnum::Unknown(bits) => bits,
                });
            }
            handle::Event::Removed => seen.removed = true,
            _ => {}
        }
    }
}
