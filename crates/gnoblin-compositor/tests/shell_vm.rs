//! The stock shell VM against a real compositor session (design/node-api-refactor.md, section 7.2).
//!
//! The shell connects through the runtime role's own path: [`connect_owner_socket`] reads the
//! session's IPC socket from the environment and boots a [`ShellVm`] over the stock Lua
//! library. The tree is the compositor's: the nodes come from the session, not from a fake.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_lua::shell_role::{ShellRole, connect_owner_socket, connect_session_shell};
use support::{Client, Locker, Session};

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

/// Runs the stock shell inside the VM the way production loads it: `require("gnoblin.shell")`
/// loads `shell/init.lua` from the shipped Lua root, which requires its siblings.
/// `sys` is the host's system table, which the shell reads when it loads; the test supplies it inert.
fn load_stock_shell(role: &mut ShellRole) {
    role.run(
        "sys",
        "sys = sys or { spawn = function() end, date = function() return '' end }",
    )
    .expect("the host table is set");
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

/// A client that closes its window: the composer drops the node, the mirror reports `destroyed`,
/// the shell's `Window:on_destroyed` runs, and the node is gone from the tree the shell reads.
#[test]
fn a_closed_window_runs_on_destroyed_and_leaves_the_tree() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    role.run(
        "on_destroyed probe",
        "destroyed_calls = 0; local prev = Window.on_destroyed; Window.on_destroyed = function(self) destroyed_calls = destroyed_calls + 1; if prev then prev(self) end end",
    )
    .expect("the probe installs");

    let mut client = Client::connect(session.wayland());
    let first = client.map_window("org.example.first", "First");
    tick_until(&mut role, "the shell never placed the window", |role| {
        window_placements(role).iter().any(|(_, _, placed)| *placed)
    });
    assert_eq!(
        window_placements(&mut role).len(),
        1,
        "one window is in the mirror before it closes"
    );

    client.destroy(first);
    tick_until(&mut role, "the closed window never left the tree", |role| {
        window_placements(role).is_empty()
    });

    let calls = role
        .run("destroyed probe", "return tostring(destroyed_calls)")
        .expect("the probe runs")
        .expect("the probe returns text");
    assert_eq!(calls, "1", "Window:on_destroyed ran once for the closed window");
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

/// A lock screen asks the session to lock: the compositor's lock node reports `requested` to the
/// shell, and the stock `Lock:on_requested` engages the lock before anything is unlocked.
#[test]
fn a_session_lock_request_engages_the_stock_lock_screen() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    role.run(
        "requested probe",
        "requested_calls = 0; requested_engaged = false; local prev = Lock.on_requested; Lock.on_requested = function(self) requested_calls = requested_calls + 1; if prev then prev(self) end; requested_engaged = self.engaged == true end",
    )
    .expect("the probe installs");

    let mut locker = Locker::connect(session.wayland());
    locker.lock();
    tick_until(&mut role, "the shell never received the lock request", |role| {
        let state = role
            .run(
                "requested state",
                "return tostring(requested_calls) .. ' ' .. tostring(requested_engaged)",
            )
            .expect("the probe runs")
            .expect("the probe returns text");
        state == "1 true"
    });
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

/// A config directory holding the given user shell files, as the user's `~/.config/gnoblin` does.
fn user_config(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary config directory");
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a user shell file");
    }
    dir
}

/// Starts the shell the way the runtime role does: [`connect_session_shell`] over the session's owner socket
/// attaches the VM, loads the stock shell from the shipped library, then the user's files in `config`.
/// `crates/gnoblin-lua/src/role.rs` passes this same function to `ShellSlot::from_env`.
fn connect_session(session: &Session, config: &Path) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_session_shell(&env, &stock_library(), Some(config)).expect("the runtime's shell entry starts the shell")
}

/// `prop` of the window whose app id is `app_id`, as text, read inside the VM; `none` when there is no such window.
fn window_prop(role: &mut ShellRole, app_id: &str, prop: &str) -> String {
    role.run(
        "window prop",
        &format!(
            "for _, w in ipairs(root:all('Window')) do if w.app_id == '{app_id}' then return tostring(w.{prop}) end end return 'none'"
        ),
    )
    .expect("the probe runs")
    .expect("the probe returns text")
}

/// A rule in the user's `shell.lua` reaches a window the compositor maps: the stock shell reads
/// `seat.data.window_rules` when the window is ready and writes the rule's fields to the node.
#[test]
fn a_window_rule_in_the_users_shell_file_applies_to_a_mapped_window() {
    let session = Session::start();
    let config = user_config(&[(
        "shell.lua",
        "seat.data.window_rules = {\n\
         \x20 { match = { app_id = '^org%.example%.secret$' }, secure = true },\n\
         \x20 { match = { app_id = '^org%.example%.faded$' }, opacity = 0.5 },\n\
         }\n",
    )]);
    let mut role = connect_session(&session, config.path());
    assert!(
        role.user_file_errors().is_empty(),
        "the user's file loads: {:?}",
        role.user_file_errors()
    );
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });

    let mut client = Client::connect(session.wayland());
    let _secret = client.map_window("org.example.secret", "Secret");
    let _faded = client.map_window("org.example.faded", "Faded");
    let _plain = client.map_window("org.example.plain", "Plain");
    tick_until(&mut role, "the rules never reached the mapped windows", |role| {
        window_prop(role, "org.example.secret", "secure") == "true"
            && window_prop(role, "org.example.faded", "opacity") == "0.5"
    });
    assert_eq!(window_prop(&mut role, "org.example.plain", "secure"), "false");
    assert_eq!(window_prop(&mut role, "org.example.secret", "opacity"), "1.0");
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

/// A user file with a syntax error costs the session nothing: the stock shell is running and places windows, and
/// the error comes back once for the file.
#[test]
fn a_syntax_error_in_the_users_shell_file_leaves_the_stock_shell_running() {
    let session = Session::start();
    let config = user_config(&[("shell.lua", "seat.data.window_rules = {{{ this is not lua")]);
    let mut role = connect_session(&session, config.path());
    let errors = role.user_file_errors().to_vec();
    assert_eq!(errors.len(), 1, "one error for the one broken file: {errors:?}");
    assert!(errors[0].contains("shell.lua"), "the error names the file: {errors:?}");
    // The stock shell is running: its Desk is built and it places a mapped window.
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    tick_until(&mut role, "the stock shell never placed the window", |role| {
        window_placements(role).iter().any(|(_, _, placed)| *placed)
    });
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

/// A workspace set from the user's file activates through `activate_set`: the set's workspace shows on the output.
#[test]
fn a_workspace_set_in_the_users_shell_file_activates() {
    let session = Session::start();
    let config = user_config(&[(
        "shell.lua",
        "seat.data.workspace_sets = { { name = 'work', entries = { { workspace = '2' } } } }\n",
    )]);
    let mut role = connect_session(&session, config.path());
    assert!(role.user_file_errors().is_empty());
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
    let activated = role
        .run("activate", "return tostring(activate_set('work'))")
        .expect("the probe runs")
        .expect("the probe returns text");
    assert_eq!(activated, "true", "diagnostics {:?}", role.diagnostics());
    let shown = role
        .run("shown", "return desk().active.name")
        .expect("the probe runs")
        .expect("the probe returns text");
    role.tick(1.0).expect("a frame runs");
    let dump = role
        .run(
            "dump",
            "local out = {} for _, ws in ipairs(root:all(Workspace)) do out[#out+1] = tostring(ws.name) .. (ws.active and '*' or '') end \
             local s = {} for _, sc in ipairs(root:all(Screen)) do s[#s+1] = tostring(sc.name) .. '/' .. tostring(sc.primary) end \
             return table.concat(out, ',') .. ' | ' .. table.concat(s, ',')",
        )
        .expect("the probe runs");
    assert_eq!(
        shown,
        "2",
        "the set's workspace shows on the output: {dump:?} {:?}",
        role.diagnostics()
    );
}
