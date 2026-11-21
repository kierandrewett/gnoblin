//! Example 34 run for real from Rust and from Python: the shipped binary serving a shell that makes workspaces, tiles real windows with each
//! layout of the library and with one of its own, drags a divider with a pointer binding of the core, and shows another workspace.
//!
//! The windows are real Wayland toplevels, the chords come from a virtual keyboard, the drag from a virtual pointer, and what is asserted is
//! what the compositor's tree holds. A stock `ext_workspace_manager_v1` client reads which workspace is the active one: the library writes
//! the id of the workspace the primary output shows to the seat's `active_workspace`, and the compositor tells it to the client as that
//! workspace's `active` state.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as manager, ExtWorkspaceManagerV1},
};

use crate::support::native_shell::{BTN_LEFT, Gnoblin, Pointer, Start, Typist, screen_size, start_script, std_shells};
use crate::support::{Mapper, scene, snapshot};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

type Rects = BTreeMap<String, (f64, f64, f64, f64)>;

fn rects(session: &Gnoblin) -> Rects {
    scene(&session.ipc())
        .into_iter()
        .map(|window| (window.app_id, (window.x, window.y, window.width, window.height)))
        .collect()
}

/// The evdev code of the key `Super+n` names: `1` to `9` are 2 to 10 and `0` is 11.
fn key(n: u32) -> u32 {
    if n == 0 { 11 } else { n + 1 }
}

fn count(session: &Gnoblin, needle: &str) -> usize {
    session.log().iter().filter(|line| line.text.contains(needle)).count()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0
}

// ---- the stock client of ext_workspace_v1 ----------------------------------------------------------------------------------

/// What the manager told the client about one workspace.
#[derive(Default, Clone, Debug)]
struct Seen {
    name: String,
    id: String,
    state: u32,
}

/// What the client heard.
#[derive(Default)]
struct Heard {
    workspaces: BTreeMap<u32, Seen>,
    dones: u32,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Heard {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Workspace { workspace } => {
                heard.workspaces.insert(workspace.id().protocol_id(), Seen::default());
            }
            manager::Event::Done => heard.dones += 1,
            _ => {}
        }
    }

    event_created_child!(Heard, ExtWorkspaceManagerV1, [
        manager::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        manager::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Heard {
    fn event(
        _: &mut Self,
        _: &ExtWorkspaceGroupHandleV1,
        _: group::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Heard {
    fn event(
        heard: &mut Self,
        workspace: &ExtWorkspaceHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = workspace.id().protocol_id();
        let seen = heard.workspaces.entry(key).or_default();
        match event {
            handle::Event::Name { name } => seen.name = name,
            handle::Event::Id { id } => seen.id = id,
            handle::Event::State { state } => {
                seen.state = match state {
                    WEnum::Value(bits) => bits.bits(),
                    WEnum::Unknown(bits) => bits,
                };
            }
            handle::Event::Removed => {
                heard.workspaces.remove(&key);
            }
            _ => {}
        }
    }
}

/// A client that binds the workspace manager on the session's privileged socket and keeps what it is told.
struct Panel {
    _connection: Connection,
    queue: EventQueue<Heard>,
    heard: Heard,
    _manager: ExtWorkspaceManagerV1,
}

/// The `active` bit of a workspace's state.
const ACTIVE: u32 = 1;

impl Panel {
    fn connect(socket: &Path) -> Panel {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<Heard>(&connection).expect("the global list");
        let manager: ExtWorkspaceManagerV1 = globals
            .bind(&queue.handle(), 1..=1, ())
            .expect("a privileged client sees the workspace manager");
        Panel {
            _connection: connection,
            queue,
            heard: Heard::default(),
            _manager: manager,
        }
    }

    /// The names of the workspaces the manager says are active, after what it has said so far.
    fn active(&mut self) -> Vec<String> {
        self.queue.roundtrip(&mut self.heard).expect("the events are read");
        self.heard
            .workspaces
            .values()
            .filter(|seen| seen.state & ACTIVE != 0)
            .map(|seen| {
                if seen.id.is_empty() {
                    seen.name.clone()
                } else {
                    seen.id.clone()
                }
            })
            .collect()
    }
}

// ---- the layouts -----------------------------------------------------------------------------------------------------------

/// Waits until `check` holds of the windows' rectangles.
#[track_caller]
fn wait_rects(session: &mut Gnoblin, what: &str, check: impl Fn(&Rects) -> bool) {
    session.wait_for(what, |session| {
        let now = rects(session);
        now.len() == 4 && check(&now)
    });
}

/// Presses `Super+n` and waits until the shell says the layout `name` is on `main`.
#[track_caller]
fn choose(session: &mut Gnoblin, keys: &mut Typist, n: u32, name: &str) {
    let line = format!("workspaces: main has the {name} layout: true");
    let before = count(session, &line);
    keys.super_plus(key(n));
    session.wait_for(&line, |session| count(session, &line) > before);
}

fn assert_the_workspaces_tile_real_windows_and_a_divider_moves(session: &mut Gnoblin) {
    session.wait_log("the list holds both workspaces", |line| {
        line.contains("workspaces: the list holds 2 workspaces: main at 0, side at 1")
    });
    session.wait_log("the shell is running", |line| line.contains("workspaces: running"));
    // The first workspace of the list is the one the primary output shows: the seat's `active_workspace`.
    session.wait_for("the seat's active workspace is main", |session| {
        snapshot(&session.ipc()).iter().any(|node| {
            node.class == "seat"
                && node
                    .props
                    .iter()
                    .any(|prop| prop.key == "active_workspace" && prop.value_text == "main")
        })
    });
    let mut panel = Panel::connect(&session.wayland_privileged());
    session.wait_for("the stock client is told of one active workspace", |_| {
        panel.active().len() == 1
    });

    // The library adopts every window that opens onto the workspace that is shown.
    let mut mapper = Mapper::connect(&session.wayland());
    for name in ["a", "b", "c", "d"] {
        mapper.map_window(name, name);
    }
    session.wait_log("four windows joined main", |line| {
        line.contains("joined main, 4 windows")
    });

    let (width, height) = screen_size(&session.ipc());
    let mut keys = Typist::connect(&session.wayland_privileged());

    // The library's master and stack: the master on the left, the other three stacked on the right with one width.
    wait_rects(session, "master and stack", |now| {
        let (a, b, c, d) = (now["a"], now["b"], now["c"], now["d"]);
        a.0 < b.0 && near(b.0, c.0) && near(c.0, d.0) && near(b.2, c.2) && near(c.2, d.2) && b.1 < c.1 && c.1 < d.1
    });

    // A divider, dragged by a pointer binding of the core: the press is on the master, at its right edge, and the travel moves the divider.
    let master = rects(session)["a"];
    let stack = rects(session)["b"];
    let mut pointer = Pointer::connect(&session.wayland_privileged(), (width as u32, height as u32));
    let (x, y) = ((master.0 + master.2 - 3.0) as u32, (master.1 + master.3 / 2.0) as u32);
    pointer.move_to(x, y);
    keys.hold_super();
    pointer.button(BTN_LEFT, true);
    for step in 1..=8 {
        pointer.move_to(x + step * 10, y);
    }
    pointer.button(BTN_LEFT, false);
    keys.release_super();
    session.wait_log("the divider was dropped", |line| {
        line.contains("workspaces: divider") && line.contains("dropped")
    });
    assert_eq!(
        count(session, "the press is on no divider"),
        0,
        "{}",
        session.log_text()
    );
    wait_rects(session, "the master grew with the divider", |now| {
        now["a"].2 > master.2 + 40.0
    });
    let widened = rects(session);
    assert!(
        widened["b"].2 < stack.2 - 40.0,
        "the stack gave up what the master gained: {widened:?}"
    );

    // Each layout of the library, one chord for each.
    choose(session, &mut keys, 2, "grid");
    wait_rects(session, "a grid of two by two", |now| {
        let cells = ["a", "b", "c", "d"].map(|name| now[name]);
        cells
            .iter()
            .all(|cell| near(cell.2, cells[0].2) && near(cell.3, cells[0].3))
            && cells.iter().filter(|cell| near(cell.0, cells[0].0)).count() == 2
            && cells.iter().filter(|cell| near(cell.1, cells[0].1)).count() == 2
    });
    choose(session, &mut keys, 3, "monocle");
    wait_rects(session, "a monocle: every window has the whole area", |now| {
        ["b", "c", "d"].iter().all(|name| now[*name] == now["a"]) && now["a"].2 > f64::from(width as u32) * 0.9
    });
    choose(session, &mut keys, 4, "dwindle");
    wait_rects(session, "a dwindle: four tiles that overlap nothing", |now| {
        let tiles = ["a", "b", "c", "d"].map(|name| now[name]);
        tiles.iter().enumerate().all(|(at, one)| {
            tiles.iter().skip(at + 1).all(|other| {
                one.0 + one.2 <= other.0 + 1.0
                    || other.0 + other.2 <= one.0 + 1.0
                    || one.1 + one.3 <= other.1 + 1.0
                    || other.1 + other.3 <= one.1 + 1.0
            })
        }) && !near(tiles[0].2, tiles[3].2)
    });
    choose(session, &mut keys, 5, "strip");
    wait_rects(session, "a strip: columns of one height", |now| {
        let columns = ["a", "b", "c", "d"].map(|name| now[name]);
        columns
            .iter()
            .all(|column| near(column.1, columns[0].1) && near(column.3, columns[0].3))
    });
    let before_ring = rects(session);
    choose(session, &mut keys, 6, "arc");
    wait_rects(session, "an arc: the windows leave their strip places", |now| {
        *now != before_ring
    });
    let before_carousel = rects(session);
    choose(session, &mut keys, 7, "carousel");
    wait_rects(session, "a carousel: the windows leave their arc places", |now| {
        *now != before_carousel
    });

    // A layout the shell implements: four columns of one width and one height, side by side.
    choose(session, &mut keys, 8, "columns");
    wait_rects(session, "four columns", |now| {
        let columns = ["a", "b", "c", "d"].map(|name| now[name]);
        columns
            .iter()
            .all(|column| near(column.2, columns[0].2) && near(column.3, columns[0].3) && near(column.1, columns[0].1))
            && columns[0].0 < columns[1].0
            && columns[1].0 < columns[2].0
            && columns[2].0 < columns[3].0
    });

    // Another workspace is shown, and the stock client is told it is the active one.
    keys.super_plus(key(9));
    session.wait_log("side is shown", |line| line.contains("workspaces: show side: true"));
    session.wait_for("the seat's active workspace is side", |session| {
        snapshot(&session.ipc()).iter().any(|node| {
            node.class == "seat"
                && node
                    .props
                    .iter()
                    .any(|prop| prop.key == "active_workspace" && prop.value_text == "side")
        })
    });
    // The session declares no workspaces, so the manager lists the one implicit workspace and it stays the active one whichever workspace
    // the shell shows: what the client is told follows the declarations, and the seat's property above is what the shell wrote.
    keys.super_plus(key(0));
    session.wait_log("main is shown again", |line| {
        line.contains("workspaces: show main: true")
    });
    assert_eq!(panel.active().len(), 1, "one active workspace");
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_34_a_std_shell_tiles_real_windows_with_each_layout_and_drags_a_divider() {
    let mut session = start_std(&std_shells().workspaces);
    assert_the_workspaces_tile_real_windows_and_a_divider_moves(&mut session);
    assert_one_core_one_std(&session);
}

#[test]
fn example_34_a_python_shell_tiles_real_windows_with_each_layout_and_drags_a_divider() {
    let dir = copy_example("34-native-workspaces-py");
    let mut session = start_script(Start::Compositor, &dir.path().join("shell.py"), &[]);
    serving(&mut session);
    assert_the_workspaces_tile_real_windows_and_a_divider_moves(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
