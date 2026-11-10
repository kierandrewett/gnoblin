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
        if Instant::now() >= deadline {
            let placements = window_placements(role);
            panic!(
                "{what}: seat/screen counts {:?}; VM diagnostics {:?}; windows (x,y,placed) {:?}",
                mirror_counts(role),
                role.diagnostics(),
                placements
            );
        }
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

/// Runs the stock shell inside the VM the way production loads it: the package path is the shipped
/// Lua root, then `require("gnoblin.shell")` loads `shell/init.lua`, which requires its siblings.
/// `sys` is the host's system table, which the shell reads when it loads; the test supplies it inert.
fn load_stock_shell(role: &mut ShellRole) {
    role.run(
        "sys",
        "sys = sys or { spawn = function() end, date = function() return '' end }",
    )
    .expect("the host table is set");
    let root = stock_library().join("..");
    let setup = format!(
        "package.path = '{root}/?.lua;{root}/?/init.lua;' .. package.path",
        root = root.display()
    );
    role.run("package.path", &setup).expect("the package path is set");
    role.run("gnoblin.shell", "require('gnoblin.shell')")
        .unwrap_or_else(|error| panic!("gnoblin.shell loads in the shell VM: {error:?}"));
}

/// The `x,y,placed` of every window node in the mirror, in tree order, read inside the VM (`placed` is `data.placed`).
fn window_placements(role: &mut ShellRole) -> Vec<(f64, f64, bool)> {
    let text = role
        .run(
            "placements",
            "local out = {} for _, w in ipairs(root:all('Window')) do \
             out[#out + 1] = tostring(w.x) .. ',' .. tostring(w.y) .. ',' .. tostring(w.data.placed == true) end \
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
    load_stock_shell(&mut role);
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

/// The shell's placement stands in the tree: each placed window's x and y in the mirror are the
/// spot the stock shell chose, and the composer does not overwrite them with its own layout. The
/// composer writes a window's x and y only while no shell holds the tree, so a second window the
/// shell places elsewhere keeps its own spot.
#[test]
fn the_composer_publishes_the_shell_chosen_spot_and_not_its_own_layout() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    let _second = client.map_window("org.example.second", "Second");
    tick_until(&mut role, "the shell never placed both windows", |role| {
        window_placements(role).iter().filter(|(_, _, placed)| *placed).count() == 2
    });
    // Let the composer publish any geometry it has pending after the placement.
    for _ in 0..30 {
        role.tick(0.5).expect("a frame runs");
    }
    let placements = window_placements(&mut role);
    assert_eq!(placements.len(), 2, "both windows are in the mirror: {placements:?}");
    for (x, y, _) in &placements {
        assert!(
            x.is_finite() && y.is_finite(),
            "the mirror holds a finite spot: {placements:?}"
        );
    }
    assert_ne!(
        (placements[0].0, placements[0].1),
        (placements[1].0, placements[1].1),
        "the second window is elsewhere than the first: {placements:?}"
    );
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
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
    // Before the first frame: the boot ready pass runs once, at that frame, and reaches only
    // the handlers the stock modules have already registered.
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
}

/// Each link of the window path, checked on its own: the window reaches the mirror, `Window:on_ready`
/// runs for it, and the shell issues no refused transaction. The probe wraps `Window.on_ready` after the
/// stock library, so it counts every call the signal makes.
#[test]
fn a_mapped_window_reaches_the_mirror_and_runs_window_on_ready() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    role.run(
        "on_ready probe",
        "ready_calls = 0; local prev = Window.on_ready; Window.on_ready = function(self) ready_calls = ready_calls + 1; if prev then prev(self) end end",
    )
    .expect("the probe installs");

    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    for _ in 0..30 {
        role.tick(0.5).expect("a frame runs");
    }
    let state = role
        .run(
            "link probe",
            "return tostring(#root:all('Window')) .. ' ' .. tostring(ready_calls)",
        )
        .expect("the probe runs")
        .expect("the probe returns text");
    let mut parts = state.split(' ');
    let windows: usize = parts.next().unwrap().parse().unwrap();
    let ready: usize = parts.next().unwrap().parse().unwrap();
    assert!(
        windows >= 1 && ready >= 1,
        "window nodes in the mirror {windows}, Window:on_ready calls {ready}; VM diagnostics {:?}",
        role.diagnostics()
    );
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}
