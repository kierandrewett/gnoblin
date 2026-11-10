//! The stock shell VM against a real compositor session (design/node-api-refactor.md, section 7.2).
//!
//! The shell connects through the runtime role's own path: [`connect_owner_socket`] reads the
//! session's IPC socket from the environment and boots a [`ShellVm`] over the stock Lua
//! library. The tree is the compositor's: the nodes come from the session, not from a fake.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_lua::shell_role::{ShellRole, connect_owner_socket};
use support::{Client, Session};

/// The stock Lua library the shell VM loads (`node.lua` and the modules beside it).
fn stock_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

/// Connects the shell VM to `session` as the runtime role does, with the socket in the environment.
fn connect(session: &Session) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_owner_socket(&env, &stock_library()).expect("the shell VM attaches to the tree")
}

/// Counts of the mirror's `seat` and `screen` nodes, read from inside the VM.
fn mirror_counts(role: &mut ShellRole) -> (usize, usize) {
    let text = role
        .run(
            "probe",
            "return tostring(#root:all('Seat')) .. ' ' .. tostring(#root:all('Screen'))",
        )
        .expect("the probe runs")
        .expect("the probe returns counts");
    let mut parts = text.split(' ').map(|n| n.parse::<usize>().expect("a count"));
    (parts.next().unwrap(), parts.next().unwrap())
}

/// Ticks the VM until `done` holds, or fails after ten seconds with `what`.
fn tick_until(role: &mut ShellRole, what: &str, mut done: impl FnMut(&mut ShellRole) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = 0.0;
    loop {
        role.tick(now).expect("a frame runs");
        now += 0.016;
        if done(role) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: seat/screen counts {:?}",
            mirror_counts(role)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_shell_vm_attaches_and_its_mirror_holds_the_seat_and_a_screen() {
    let session = Session::start();
    let mut role = connect(&session);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
}

/// The stock shell's modules, in the order the shell loads them (`shell/init.lua` needs the
/// others first: floating and strip before init turns every Desk workspace into a Floating).
const STOCK_SHELL: &[&str] = &[
    "shell/ui.lua",
    "shell/floating.lua",
    "shell/strip.lua",
    "shell/focus.lua",
    "shell/snap.lua",
    "shell/rules.lua",
    "shell/init.lua",
    "shell/panel.lua",
    "shell/lock.lua",
    "shell/overview.lua",
    "shell/decorations.lua",
    "shell/launcher.lua",
    "shell/workspaces.lua",
];

/// Runs the stock shell's modules inside the VM, after `node.lua`. `sys` is the host's system
/// table, which the shell reads when it loads; the test supplies it inert, as the Lua harness does.
fn load_stock_shell(role: &mut ShellRole) {
    role.run(
        "sys",
        "sys = sys or { spawn = function() end, date = function() return '' end }",
    )
    .expect("the host table is set");
    for module in STOCK_SHELL {
        let src = std::fs::read_to_string(stock_library().join(module)).expect("a stock module");
        role.run(module, &src)
            .unwrap_or_else(|error| panic!("{module} loads in the shell VM: {error:?}"));
    }
}

/// The `x,y,placed` of every window node in the mirror, in tree order, read inside the VM.
fn window_placements(role: &mut ShellRole) -> Vec<(f64, f64, bool)> {
    let text = role
        .run(
            "placements",
            "local out = {} for _, w in ipairs(root:all('Window')) do \
             out[#out + 1] = tostring(w.x) .. ',' .. tostring(w.y) .. ',' .. tostring(w.placed == true) end \
             return table.concat(out, ';')",
        )
        .expect("the probe runs")
        .unwrap_or_default();
    text.split(';')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let mut parts = entry.split(',');
            let x = parts.next().unwrap().parse().expect("x");
            let y = parts.next().unwrap().parse().expect("y");
            let placed = parts.next().unwrap() == "true";
            (x, y, placed)
        })
        .collect()
}

#[test]
fn the_stock_shell_places_each_mapped_window_on_a_spot_of_its_own() {
    let session = Session::start();
    let mut role = connect(&session);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    let published = role
        .run(
            "published",
            "local s = root:find(Seat).screen return tostring(s ~= nil) .. ' ' .. tostring(root:all('Screen')[1] ~= nil)",
        )
        .expect("the probe runs")
        .expect("the probe returns text");
    assert_eq!(published, "true true", "the seat publishes its screen to the mirror");
    load_stock_shell(&mut role);

    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    let _second = client.map_window("org.example.second", "Second");

    tick_until(&mut role, "the shell never placed both windows", |role| {
        window_placements(role).iter().filter(|(_, _, placed)| *placed).count() == 2
    });

    let placements = window_placements(&mut role);
    let (a, b) = (placements[0], placements[1]);
    assert_ne!(
        (a.0, a.1),
        (b.0, b.1),
        "the shell placed the windows on different spots"
    );
}

/// The Desk for the screen under the pointer: one per screen, built by `Screen:on_ready`.
fn desk_probe(role: &mut ShellRole) -> String {
    role.run(
        "desks",
        "local ds = root:all('Desk') local s = root:find(Seat).screen \
         local mine = 0 for _, d in ipairs(ds) do if d.screen == s then mine = mine + 1 end end \
         return tostring(#ds) .. ' ' .. tostring(mine)",
    )
    .expect("the probe runs")
    .expect("the probe returns text")
}

#[test]
fn the_stock_shell_builds_one_desk_for_the_screen_under_the_pointer() {
    let session = Session::start();
    let mut role = connect(&session);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    load_stock_shell(&mut role);
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
}
