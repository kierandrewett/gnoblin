//! `ext_workspace_manager_v1`: the workspace list a panel reads, from a real client.
//!
//! The scenarios are the failure modes listed at the top of `src/protocols/ext_workspace_v1.rs`. The compositor keeps
//! no workspaces: the list is the seat's declared workspaces and the one the shell wrote to `active_workspace`, so
//! the tests that need a shell use the stand-in of `tests/session/shell.rs` to write it, and the ones that declare
//! workspaces start the session with a configuration that does. Who may bind the manager is `tests/access.rs`: the
//! manager is in its `PRIVILEGED` list.

mod session;
mod stock;

use session::shell::Shell;
use session::{Client, Session, WorkspaceSeen};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};

/// The `active` bit of a workspace's state.
const ACTIVE: u32 = 1;

wayland_client::delegate_noop!(session::App: ignore WlOutput);

impl Dispatch<ExtWorkspaceManagerV1, ()> for session::App {
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

    wayland_client::event_created_child!(session::App, ExtWorkspaceManagerV1, [
        manager::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        manager::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for session::App {
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

impl Dispatch<ExtWorkspaceHandleV1, ()> for session::App {
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

/// Binds the manager on a privileged connection and waits for the batch the bind is answered with.
#[track_caller]
fn bind(client: &mut Client) -> ExtWorkspaceManagerV1 {
    let before = client.state.workspace_view.dones;
    let manager: ExtWorkspaceManagerV1 = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("a privileged client sees the workspace manager");
    client.wait_until("the first done", |app| app.workspace_view.dones > before);
    manager
}

/// The group object, found from the workspace the group entered.
fn web_group(client: &Client) -> ExtWorkspaceGroupHandleV1 {
    let object = client.state.workspace_view.group.clone().expect("the group was sent");
    ExtWorkspaceGroupHandleV1::from_id(&client.connection, object).expect("the group is alive")
}

fn present(client: &Client) -> Vec<(String, String, u32)> {
    client.state.workspace_view.present()
}

#[test]
fn an_application_does_not_see_the_workspace_manager() {
    Session::start().run(|session| {
        let application = Client::connect(session.socket());
        assert!(!application.has_global("ext_workspace_manager_v1"));
        let privileged = Client::connect(session.privileged_socket());
        assert!(privileged.has_global("ext_workspace_manager_v1"));
    });
}

#[test]
fn a_client_is_told_the_session_workspaces_when_it_binds() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let _manager = bind(&mut client);
        let view = &client.state.workspace_view;
        assert_eq!(
            present(&client),
            [("1".to_owned(), "1".to_owned(), ACTIVE)],
            "a session that declares none has one workspace, and it is the one shown"
        );
        assert_eq!(view.workspaces[0].1.capabilities, Some(0), "and no request is on offer");
        assert_eq!(view.group_capabilities, Some(0));
        assert_eq!(view.in_group.len(), 1, "the group holds the workspace");
        assert_eq!(view.outputs_entered, 1, "and the one output this client bound");
        assert_eq!(view.dones, 1, "everything arrived in one batch");
    });
}

#[test]
fn declared_workspaces_are_listed_with_their_names() {
    let (shell, hooks) = Shell::new();
    Session::start_declaring(&["code:Code", "web:Web", "plain"], hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let _manager = bind(&mut client);
        assert_eq!(
            present(&client),
            [
                ("code".to_owned(), "Code".to_owned(), 0),
                ("web".to_owned(), "Web".to_owned(), 0),
                ("plain".to_owned(), "plain".to_owned(), 0),
            ],
            "the order is the declared one, and with several workspaces and none named active, none is"
        );
        assert_eq!(client.state.workspace_view.in_group.len(), 3);
    });
}

#[test]
fn the_shells_active_workspace_is_the_active_one() {
    let (shell, hooks) = Shell::new();
    Session::start_declaring(&["code:Code", "web:Web"], hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let _manager = bind(&mut client);

        shell.declare_active_workspace("web");
        client.wait_until("web to be active", |app| {
            app.workspace_view
                .present()
                .iter()
                .any(|(id, _, state)| id == "web" && *state == ACTIVE)
        });
        assert_eq!(
            present(&client),
            [
                ("code".to_owned(), "Code".to_owned(), 0),
                ("web".to_owned(), "Web".to_owned(), ACTIVE),
            ]
        );

        // The change is one batch: the client heard a `done` for it.
        let dones = client.state.workspace_view.dones;
        shell.declare_active_workspace("code");
        client.wait_until("code to be active", |app| {
            app.workspace_view
                .present()
                .iter()
                .any(|(id, _, state)| id == "code" && *state == ACTIVE)
        });
        assert!(client.state.workspace_view.dones > dones, "the change ends in a done");
        assert_eq!(
            client
                .state
                .workspace_view
                .present()
                .iter()
                .filter(|(_, _, state)| *state == ACTIVE)
                .count(),
            1,
            "exactly one workspace is active"
        );
    });
}

#[test]
fn no_shell_and_several_workspaces_means_none_active() {
    // Declared workspaces and nobody to say which one is shown: the compositor does not know, so it says none.
    let (_shell, hooks) = Shell::new();
    Session::start_declaring(&["a:A", "b:B"], hooks).run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let _manager = bind(&mut client);
        assert!(
            present(&client).iter().all(|(_, _, state)| *state == 0),
            "{:?}",
            present(&client)
        );
    });
}

#[test]
fn a_late_client_is_told_the_same_state() {
    let (shell, hooks) = Shell::new();
    Session::start_declaring(&["code:Code", "web:Web"], hooks).run(move |session| {
        shell.attach();
        shell.declare_active_workspace("web");
        let mut early = Client::connect(session.privileged_socket());
        let _early = bind(&mut early);
        early.wait_until("web to be active", |app| {
            app.workspace_view
                .present()
                .iter()
                .any(|(id, _, state)| id == "web" && *state == ACTIVE)
        });

        let mut late = Client::connect(session.privileged_socket());
        let _late = bind(&mut late);
        assert_eq!(present(&late), present(&early));
    });
}

#[test]
fn requests_without_capabilities_are_ignored() {
    let (shell, hooks) = Shell::new();
    Session::start_declaring(&["code:Code", "web:Web"], hooks).run(move |session| {
        shell.attach();
        shell.declare_active_workspace("code");
        let mut client = Client::connect(session.privileged_socket());
        let manager = bind(&mut client);
        client.wait_until("code to be active", |app| {
            app.workspace_view
                .present()
                .iter()
                .any(|(id, _, state)| id == "code" && *state == ACTIVE)
        });
        let before = present(&client);

        // Asking for what was never offered does nothing, and the connection survives it.
        let object = client
            .state
            .workspace_view
            .workspaces
            .iter()
            .find(|(_, seen)| seen.id == "web")
            .and_then(|(_, seen)| seen.object.clone())
            .expect("the web workspace");
        let web = ExtWorkspaceHandleV1::from_id(&client.connection, object).expect("the handle is alive");
        web.activate();
        web.deactivate();
        web.assign(&web_group(&client));
        web.remove();
        web_group(&client).create_workspace("scratch".to_owned());
        manager.commit();
        client.roundtrip();
        assert_eq!(
            present(&client),
            before,
            "none of the requests changed anything, and the connection survived them"
        );
    });
}

#[test]
fn a_stopped_manager_is_finished_and_forgotten() {
    let (shell, hooks) = Shell::new();
    Session::start_declaring(&["code:Code", "web:Web"], hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let manager = bind(&mut client);
        manager.stop();
        client.wait_until("finished", |app| app.workspace_view.finished);

        // Nothing is sent to it after that: a change on the seat leaves the connection as it is.
        let dones = client.state.workspace_view.dones;
        shell.declare_active_workspace("web");
        let mut other = Client::connect(session.privileged_socket());
        let _other = bind(&mut other);
        other.wait_until("web to be active", |app| {
            app.workspace_view
                .present()
                .iter()
                .any(|(id, _, state)| id == "web" && *state == ACTIVE)
        });
        client.roundtrip();
        assert_eq!(client.state.workspace_view.dones, dones);
    });
}
