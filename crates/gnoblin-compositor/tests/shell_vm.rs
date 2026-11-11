//! The stock shell VM against a real compositor session (design/node-api-refactor.md, section 7.2).
//!
//! The shell connects through the runtime role's own path: [`connect_owner_socket_with_limits`] reads the
//! session's IPC socket from the environment and boots a [`ShellVm`] over the stock Lua
//! library. The tree is the compositor's: the nodes come from the session, not from a fake.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_lua::shell_role::{ShellRole, connect_owner_socket_with_limits, connect_session_shell_with_limits};
use gnoblin_lua::shell_vm::ShellLimits;
use support::{Client, Locker, Session};

/// How long a test waits for the session or the VM to reach a state before it fails. A passing run never waits this
/// long; it is the margin a busy machine gets, so a failure is a state that never came and not a slow turn.
const PATIENCE: Duration = Duration::from_secs(60);

/// The stock Lua library the shell VM loads (`node.lua` and the modules beside it).
fn stock_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

/// The budgets the tests run the VM under: the shipped ones, with the wall-clock handler deadline widened.
///
/// The instruction and memory budgets are what stop a runaway script, and they count the same on every machine.
/// The deadline is elapsed time, so a test machine that is busy (a full workspace build and test run share its
/// cores) can pass 100 ms inside one handler or 500 ms loading the shell without the script being slow. A test
/// that fails on that is measuring the machine, so the deadline here is a minute.
fn patient_limits() -> ShellLimits {
    const DEADLINE_TICKS: u64 = 60_000;
    let shipped = ShellLimits::default();
    ShellLimits {
        entry: gnoblin_lua::Limits {
            handler_ticks: DEADLINE_TICKS,
            ..shipped.entry
        },
        frame: gnoblin_lua::Limits {
            handler_ticks: DEADLINE_TICKS,
            ..shipped.frame
        },
        load: gnoblin_lua::Limits {
            handler_ticks: DEADLINE_TICKS,
            ..shipped.load
        },
    }
}

/// Connects the shell VM to `session` as the runtime role does, with the socket in the environment.
fn connect(session: &Session) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_owner_socket_with_limits(&env, &stock_library(), patient_limits())
        .expect("the shell VM attaches to the tree")
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
    let deadline = Instant::now() + PATIENCE;
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

/// Waits until the Desk under the pointer has a workspace with a size. The shell places a window in the space its
/// workspace has, so a window that maps before the screen's size has reached the workspaces lands in none.
fn wait_for_sized_workspace(role: &mut ShellRole) {
    tick_until(role, "the Desk's workspace never got a size", |role| {
        role.run(
            "workspace size",
            "local d = desk() return tostring(d ~= nil and d.active ~= nil and d.active.width > 0 and d.active.height > 0)",
        )
        .expect("the probe runs")
        .expect("the probe returns text")
            == "true"
    });
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

    wait_for_sized_workspace(&mut role);
    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    let _second = client.map_window("org.example.second", "Second");

    // `placed` is the shell's own flag, set when it writes the spot; the spot is in the mirror once the compositor
    // has echoed the write, so wait for the two spots themselves.
    tick_until(
        &mut role,
        "the shell never placed both windows on spots of their own",
        |role| {
            let placements = window_placements(role);
            placements.iter().filter(|(_, _, placed)| *placed).count() == 2
                && (placements[0].0, placements[0].1) != (placements[1].0, placements[1].1)
        },
    );

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
    wait_for_sized_workspace(&mut role);
    let mut client = Client::connect(session.wayland());
    let _first = client.map_window("org.example.first", "First");
    let _second = client.map_window("org.example.second", "Second");
    tick_until(&mut role, "the shell never placed both windows", |role| {
        window_placements(role).iter().filter(|(_, _, placed)| *placed).count() == 2
    });
    // What the compositor itself holds is what the shell chose, and stays so: each snapshot of an observer is
    // taken in a frame of the compositor's own, so the rounds after the first have seen its frames go by.
    let mut spots: Vec<(f64, f64)> = Vec::new();
    for round in 0..4 {
        let deadline = Instant::now() + PATIENCE;
        loop {
            role.tick(0.5 + f64::from(round) * 0.016).expect("a frame runs");
            let mut expected: Vec<(f64, f64)> = window_placements(&mut role).iter().map(|(x, y, _)| (*x, *y)).collect();
            expected.sort_by(f64_pairs);
            spots = published_spots(&session);
            spots.sort_by(f64_pairs);
            if spots == expected && expected.len() == 2 && expected[0] != expected[1] {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the compositor holds {spots:?}, not two different spots the shell chose ({expected:?}); VM diagnostics {:?}",
                role.diagnostics()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(spots.len(), 2, "both windows are in the compositor's tree: {spots:?}");
    assert_ne!(
        spots[0], spots[1],
        "the second window is elsewhere than the first: {spots:?}"
    );
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

fn f64_pairs(a: &(f64, f64), b: &(f64, f64)) -> std::cmp::Ordering {
    a.partial_cmp(b).expect("finite spots")
}

/// The `(x, y)` of every window in the compositor's own tree, read through a fresh observer. A value at its default
/// is not carried in a snapshot, so it reads as zero.
fn published_spots(session: &Session) -> Vec<(f64, f64)> {
    let (_wire, _, nodes) = observe(session);
    nodes
        .iter()
        .filter(|node| node.class == "window")
        .map(|node| {
            let value = |key: &str| {
                node.props
                    .iter()
                    .find(|prop| prop.key == key)
                    .map_or(0.0, |prop| prop.value_double)
            };
            (value("x"), value("y"))
        })
        .collect()
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
    tick_until(
        &mut role,
        "the window never reached the mirror and became ready",
        |role| {
            role.run(
                "link probe",
                "return tostring(#root:all('Window') >= 1 and ready_calls >= 1)",
            )
            .expect("the probe runs")
            .expect("the probe returns text")
                == "true"
        },
    );
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
    connect_session_shell_with_limits(&env, &stock_library(), Some(config), patient_limits())
        .expect("the runtime's shell entry starts the shell")
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
            && window_prop(role, "org.example.plain", "secure") == "false"
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

/// The workspace the window with `app_id` sits on, `-` when it is on none or there is no such window.
fn workspace_of(role: &mut ShellRole, app_id: &str) -> String {
    role.run(
        "workspace of window",
        &format!(
            "for _, w in ipairs(root:all('Window')) do if w.app_id == '{app_id}' then \
             local ws = w:closest(Workspace) return ws and ws.name or '-' end end return '-'"
        ),
    )
    .expect("the probe runs")
    .expect("the probe returns text")
}

/// A workspace rule from the user's file moves the mapped window to the workspace it names, on the real core.
#[test]
fn a_workspace_rule_in_the_users_shell_file_places_the_window_on_that_workspace() {
    let session = Session::start();
    let config = user_config(&[(
        "shell/10-rules.lua",
        "seat.data.window_rules = { { match = { app_id = '^org%.example%.term$' }, workspace = { id = '3' } } }\n",
    )]);
    let mut role = connect_session(&session, config.path());
    assert!(role.user_file_errors().is_empty());
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    let mut client = Client::connect(session.wayland());
    let _term = client.map_window("org.example.term", "Term");
    tick_until(&mut role, "the window never reached the tree", |role| {
        workspace_of(role, "org.example.term") != "-"
    });
    tick_until(&mut role, "the rule never placed the window on workspace 3", |role| {
        workspace_of(role, "org.example.term") == "3"
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

/// The screen's size in the mirror, read inside the VM.
fn screen_size(role: &mut ShellRole) -> (i32, i32) {
    let text = role
        .run(
            "screen size",
            "local s = root:find(Screen) return string.format('%d %d', s.width, s.height)",
        )
        .expect("the probe runs")
        .expect("the probe returns text");
    let mut sides = text.split(' ').map(|side| side.parse::<i32>().expect("a side"));
    (sides.next().unwrap(), sides.next().unwrap())
}

/// A stock Tiling workspace places three real clients, and what the shell wrote reaches each of them on the wire:
/// its configure size is its tile, it is told all four edges are tiled, and its bounds are the work area. This is the
/// check the old `a_master_stack_places_three_real_clients_at_their_configured_sizes` and
/// `a_tiled_master_receives_tiled_states_and_work_area_bounds_on_the_wire` made against the Rust layout engine.
#[test]
fn a_tiling_workspace_configures_three_real_clients_to_their_tiles_with_tiled_edges_and_the_work_area() {
    use wayland_protocols::xdg::shell::client::xdg_toplevel::State;

    let session = Session::start();
    let config = user_config(&[(
        "shell.lua",
        "require('gnoblin.shell.tiling')\nfunction Desk:workspace(name) return Tiling({ name = name }) end\n",
    )]);
    let mut role = connect_session(&session, config.path());
    assert!(
        role.user_file_errors().is_empty(),
        "the user's file loads: {:?}",
        role.user_file_errors()
    );
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
    let (screen_width, screen_height) = screen_size(&mut role);
    let (half, rows) = (screen_width / 2, screen_height / 2);

    let mut client = Client::connect(session.wayland());
    let master = client.map_window("org.example.master", "Master");
    let upper = client.map_window("org.example.stack", "Upper");
    let lower = client.map_window("org.example.stack", "Lower");

    let expected = [
        (&master, (half, screen_height)),
        (&upper, (half, rows)),
        (&lower, (half, rows)),
    ];
    let deadline = Instant::now() + PATIENCE;
    let mut now = 0.0;
    loop {
        role.tick(now).expect("a frame runs");
        now += 0.016;
        client.roundtrip();
        if expected
            .iter()
            .all(|(window, size)| client.configured_size(window) == Some(*size))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the clients were never configured to their tiles: {:?}; VM diagnostics {:?}",
            expected
                .iter()
                .map(|(window, size)| (client.configured_size(window), *size))
                .collect::<Vec<_>>(),
            role.diagnostics()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    for (window, _) in expected {
        let states = client.configured_states(window).expect("the window was configured");
        for edge in [State::TiledLeft, State::TiledRight, State::TiledTop, State::TiledBottom] {
            assert!(states.contains(&edge), "a tiled window is told {edge:?}: {states:?}");
        }
        assert_eq!(
            client.configured_bounds(window),
            Some((screen_width, screen_height)),
            "a tiled window is sent the work area as its bounds"
        );
    }
    assert!(
        role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        role.diagnostics()
    );
}

// ---- A restarted shell (design/node-core.md, section 4.6 step 5) ---------------------------------------------------

/// Connects a shell once the previous holder's connection has closed and the compositor has released the tree.
fn reconnect(session: &Session) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    let deadline = Instant::now() + PATIENCE;
    loop {
        match connect_owner_socket_with_limits(&env, &stock_library(), patient_limits()) {
            Ok(role) => return role,
            Err(error) => {
                assert!(Instant::now() < deadline, "the tree was never released: {error:?}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// How many nodes are under the root, read inside the VM.
fn node_count(role: &mut ShellRole) -> usize {
    role.run(
        "node count",
        "local function count(n) local c = 0 for _, k in ipairs(n.children) do c = c + 1 + count(k) end return c end \
         return tostring(count(root))",
    )
    .expect("the probe runs")
    .expect("the probe returns text")
    .parse()
    .expect("a count")
}

/// A shell that restarts finds the old shell's nodes gone with its connection and builds its own Desk, so the tree is
/// the size one shell makes it: not the old Desk, its workspaces and its bars left beside a second set. (gnoblin-pv63:
/// the nodes stayed, and the Lua state that made them did not, so nothing could adopt or remove them.)
#[test]
fn a_restarted_shell_builds_the_tree_one_shell_makes_not_a_second_copy() {
    let session = Session::start();
    let mut first = connect(&session);
    load_stock_shell(&mut first);
    tick_until(&mut first, "the first shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
    let one_shell = node_count(&mut first);
    assert!(one_shell > 1, "the stock shell builds a tree: {one_shell} nodes");
    drop(first);

    let mut second = reconnect(&session);
    load_stock_shell(&mut second);
    tick_until(
        &mut second,
        "the restarted shell never built the screen's Desk",
        |role| desk_probe(role) == "1 1",
    );
    tick_until(&mut second, "the old shell's nodes were never destroyed", |role| {
        node_count(role) == one_shell
    });
    for frame in 0..20 {
        second.tick(100.0 + f64::from(frame) * 0.016).expect("a frame runs");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(node_count(&mut second), one_shell, "nothing is built twice");
    assert!(
        second.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        second.diagnostics()
    );
}

// ---- XR mode: the stock shell's room (data/lua/gnoblin/shell/xr.lua) -----------------------------------------------

/// Attaches an observer connection and reads its snapshot, as the XR extension does.
fn observe(session: &Session) -> (support::Wire, u64, Vec<gnoblin_proto::tree::snapshot_node::Properties>) {
    use gnoblin_proto::Envelope;
    use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, SnapshotPayload};

    let mut wire = support::Wire::connect(session.socket());
    wire.send(&Envelope::request_message(
        "tree",
        "attach",
        1,
        &AttachArgs {
            version: 1,
            mode: "observer".to_owned(),
        },
    ));
    let attached: AttachedPayload = wire
        .receive()
        .expect("the attach reply")
        .payload_as()
        .expect("the attach payload decodes");
    let mut nodes = Vec::new();
    loop {
        let chunk: SnapshotPayload = wire
            .receive()
            .expect("a snapshot chunk")
            .payload_as()
            .expect("the chunk decodes");
        nodes.extend(chunk.nodes);
        if chunk.last {
            break;
        }
    }
    (wire, u64::from(attached.epoch as u32) << 32 | 1, nodes)
}

/// Makes the seat named `xr` the way the XR extension does: an observer connection that creates it unparented.
/// The seat lives as long as the returned wire; dropping the wire closes the connection, and the compositor
/// destroys the seat.
fn xr_seat(session: &Session) -> support::Wire {
    use gnoblin_proto::tree::op;
    use gnoblin_proto::tree::tree::{ReplyPayload, TransactArgs};
    use gnoblin_proto::{Envelope, Kind};

    let (mut wire, id, _) = observe(session);
    wire.send(&Envelope::request_message(
        "tree",
        "transact",
        2,
        &TransactArgs {
            base_serial: 0,
            serial: 1,
            ops: vec![
                op::Properties {
                    kind: "new".to_owned(),
                    class: "seat".to_owned(),
                    id,
                    ..Default::default()
                },
                op::Properties {
                    kind: "set".to_owned(),
                    node: id,
                    key: "name".to_owned(),
                    value_kind: "text".to_owned(),
                    value_text: "xr".to_owned(),
                    ..Default::default()
                },
            ],
        },
    ));
    loop {
        let envelope = wire.receive().expect("a frame");
        if matches!(envelope.kind, Some(Kind::Reply(_))) && envelope.member == "transact" {
            let reply: ReplyPayload = envelope.payload_as().expect("the reply decodes");
            assert!(reply.applied, "the xr seat was refused: {}", reply.code);
            return wire;
        }
    }
}

/// `room views windows`: whether the node named `room` exists, how many `View`s it holds (the compositor has
/// Views of its own elsewhere in the tree) and how many `Window`s the shell sees.
fn xr_probe(role: &mut ShellRole) -> String {
    role.run(
        "xr probe",
        "local room = root:find('#room') \
         return (room and 'room' or 'none') .. ' ' .. tostring(room and #room:all('View') or 0) .. ' ' \
         .. tostring(#root:all('Window'))",
    )
    .expect("the probe runs")
    .expect("the probe returns text")
}

/// Ticks the VM until the xr probe reads `want`, or fails after ten seconds with what it last read.
fn tick_until_xr(role: &mut ShellRole, what: &str, want: &str) {
    let deadline = Instant::now() + PATIENCE;
    let mut now = 0.0;
    loop {
        role.tick(now).expect("a frame runs");
        now += 0.016;
        let last = xr_probe(role);
        if last == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: the probe (room views windows) read `{last}`, not `{want}`; VM diagnostics {:?}",
            role.diagnostics()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Ticks the VM until `watcher` has heard the compositor publish the destruction of every node in `gone`, then ticks
/// it a few frames more so the shell has taken the same batch in. The shell destroys these nodes itself, so its own
/// view of them is gone at once; what the compositor did is the watcher's to say. A frame that fails fails the test.
fn tick_until_destroyed(role: &mut ShellRole, watcher: &mut support::Wire, gone: &[u64], what: &str) {
    use gnoblin_proto::Kind;
    use gnoblin_proto::tree::tree::ChangesPayload;

    let deadline = Instant::now() + PATIENCE;
    let mut now = 0.0;
    let mut heard: Vec<u64> = Vec::new();
    watcher.set_read_timeout(Duration::from_millis(5));
    while !gone.iter().all(|node| heard.contains(node)) {
        role.tick(now).expect("a frame runs");
        now += 0.016;
        while let Some(envelope) = watcher.receive_within() {
            if matches!(envelope.kind, Some(Kind::Event(_))) && envelope.member == "changes" {
                let batch: ChangesPayload = envelope.payload_as().expect("the batch decodes");
                for change in batch.changes.iter().filter(|change| change.kind == "destroyed") {
                    heard.push(change.node);
                    heard.extend(change.removed.iter().copied());
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "{what}: the compositor published the destruction of {heard:?}, not of all of {gone:?}; VM diagnostics {:?}",
            role.diagnostics()
        );
    }
    for _ in 0..5 {
        role.tick(now).expect("a frame runs after the destroy came back");
        now += 0.016;
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A shell with the stock library, one real client window and the xr room up: a View for the window. Also returns
/// a watcher that attached once the room was up, and the id of the room and of its View.
fn xr_room_with_one_window() -> (
    Session,
    ShellRole,
    support::Wire,
    support::Wire,
    Client,
    support::Window,
    (u64, u64),
) {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.first", "First");
    let seat = xr_seat(&session);
    tick_until_xr(&mut role, "the room never got a View for the window", "room 1 1");
    // The watcher attaches after the View exists in the shell's view, but the compositor may not have it yet: attach
    // again until its snapshot holds the View.
    let deadline = Instant::now() + PATIENCE;
    let (watcher, ids) = loop {
        role.tick(1.0).expect("a frame runs");
        let (watcher, _, nodes) = observe(&session);
        let room = nodes
            .iter()
            .find(|node| node.class == "camera")
            .map(|camera| camera.parent);
        let view = room.and_then(|room| nodes.iter().find(|node| node.class == "view" && node.parent == room));
        if let (Some(room), Some(view)) = (room, view) {
            break (watcher, (room, view.id));
        }
        assert!(
            Instant::now() < deadline,
            "the compositor never held the room: {nodes:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    (session, role, seat, watcher, client, window, ids)
}

/// A client that closes its window while the room is up: the compositor drops the View with the shell's
/// destroy, the shell's frames go on, and the room stays. (gnoblin-6zce: `core.destroy` answered nothing, so
/// `Node:destroy` failed and the View stayed in the room; and the compositor's echo of the shell's own destroy
/// failed the frame.)
#[test]
fn a_closed_clients_view_leaves_the_xr_room() {
    let (_session, mut role, _seat, mut watcher, mut client, window, (_, view)) = xr_room_with_one_window();

    client.destroy(window);
    tick_until_destroyed(
        &mut role,
        &mut watcher,
        &[view],
        "the closed window's View never left the room",
    );
    assert_eq!(xr_probe(&mut role), "room 0 0");
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

/// The xr seat goes (its extension disconnects): the room and its View are destroyed in the compositor, the shell
/// goes on running, and the window stays on its workspace.
#[test]
fn the_xr_room_is_torn_down_when_the_seat_goes_and_the_windows_stay_on_their_workspaces() {
    let (_session, mut role, seat, mut watcher, _client, _window, (room, view)) = xr_room_with_one_window();
    let on_workspace = |role: &mut ShellRole| {
        role.run(
            "workspace probe",
            "local w = root:all('Window')[1] return tostring(w:closest(Workspace) ~= nil)",
        )
        .expect("the probe runs")
        .expect("the probe returns text")
    };
    tick_until(&mut role, "the window never joined a workspace", |role| {
        on_workspace(role) == "true"
    });

    drop(seat);
    tick_until_destroyed(
        &mut role,
        &mut watcher,
        &[room, view],
        "the room was never torn down with the seat",
    );
    assert_eq!(xr_probe(&mut role), "none 0 1");
    assert_eq!(on_workspace(&mut role), "true", "the window stays on its workspace");
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

// ---- Canvas painting: the stock titlebar (data/lua/gnoblin/shell/decorations.lua) ----------------------------------

/// A canvas of the compositor's tree as the shell painted it: its size and the paint list it holds.
struct PaintedCanvas {
    width: u32,
    height: u32,
    list: gnoblin_paint::PaintList,
}

impl PaintedCanvas {
    /// The canvas rasterised with the embedded font, premultiplied, so the pixels do not depend on this machine's fonts.
    fn picture(&self) -> gnoblin_paint::Picture {
        use std::sync::Arc;
        gnoblin_paint::CanvasPainter::new(Arc::new(gnoblin_paint::FontSet::embedded()))
            .paint(&self.list, self.width, self.height)
            .expect("a canvas of a sane size")
            .picture
    }

    /// How many pixels of the picture are not transparent, and the most opaque alpha among them.
    fn ink(&self) -> (usize, u8) {
        let picture = self.picture();
        let alphas: Vec<u8> = picture.data.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).collect();
        (
            alphas.iter().filter(|alpha| **alpha != 0).count(),
            alphas.iter().copied().max().unwrap_or(0),
        )
    }
}

/// Every canvas in the compositor's own tree that holds a paint list and has a size, read through a fresh observer.
fn painted_canvases(session: &Session) -> Vec<PaintedCanvas> {
    let (_wire, _, nodes) = observe(session);
    nodes
        .iter()
        .filter(|node| node.class == "canvas")
        .filter_map(|node| {
            let prop = |key: &str| node.props.iter().find(|prop| prop.key == key);
            let content = prop("content")?;
            let width = prop("width").map_or(0.0, |prop| prop.value_double) as u32;
            let height = prop("height").map_or(0.0, |prop| prop.value_double) as u32;
            let list = gnoblin_paint::decode(&content.value_bytes).expect("the compositor holds only paint lists");
            (width > 0 && height > 0).then_some(PaintedCanvas { width, height, list })
        })
        .collect()
}

/// The fills the stock titlebar uses: activated and not.
const TITLEBAR_FILLS: [[u8; 3]; 2] = [[0x41, 0x45, 0x59], [0x23, 0x26, 0x34]];

fn is_titlebar_fill(canvas: &PaintedCanvas) -> bool {
    canvas.list.ops.iter().any(|op| {
        matches!(op, gnoblin_paint::PaintOp::Rect { fill, width, .. }
            if *width > 0.0 && fill.a == 255 && TITLEBAR_FILLS.contains(&[fill.r, fill.g, fill.b]))
    })
}

fn has_text(canvas: &PaintedCanvas, wanted: &str) -> bool {
    canvas
        .list
        .ops
        .iter()
        .any(|op| matches!(op, gnoblin_paint::PaintOp::Text { text, .. } if text == wanted))
}

fn has_icon(canvas: &PaintedCanvas, wanted: &str) -> bool {
    canvas
        .list
        .ops
        .iter()
        .any(|op| matches!(op, gnoblin_paint::PaintOp::Icon { name, .. } if name == wanted))
}

/// A mapped window gets the stock titlebar, and the compositor holds what paints it: a bar filled with the
/// titlebar colour, the window's title as text and a close icon, each a paint list the compositor accepted (a
/// list that did not decode would have been refused `bad-paint`, and the shell would have reported it).
/// Each list is drawn with the painter and its pixels read: the fill, the title's strokes and the icon's strokes.
#[test]
fn a_mapped_window_gets_a_titlebar_that_paints_its_fill_title_and_close_icon() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    wait_for_sized_workspace(&mut role);
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.titled", "A Title");

    tick_until(
        &mut role,
        "the titlebar's bar, title and close icon were never all painted",
        |_| {
            let canvases = painted_canvases(&session);
            canvases.iter().any(is_titlebar_fill)
                && canvases.iter().any(|canvas| has_text(canvas, "A Title"))
                && canvases.iter().any(|canvas| has_icon(canvas, "window-close-symbolic"))
        },
    );

    let canvases = painted_canvases(&session);
    let bar = canvases
        .iter()
        .find(|canvas| is_titlebar_fill(canvas))
        .expect("the bar");
    let title = canvases
        .iter()
        .find(|canvas| has_text(canvas, "A Title"))
        .expect("the title");
    let close = canvases
        .iter()
        .find(|canvas| has_icon(canvas, "window-close-symbolic"))
        .expect("the close icon");

    // The bar: the fill, opaque, at its corner and its far end.
    let picture = bar.picture();
    let fill = picture.pixel(1, 1).expect("inside the bar");
    assert_eq!(fill[3], 255, "the bar is opaque");
    assert!(
        TITLEBAR_FILLS.contains(&[fill[0], fill[1], fill[2]]),
        "the fill is a titlebar colour: {fill:?}"
    );
    assert_eq!(
        picture.pixel(bar.width - 2, bar.height - 2),
        Some(fill),
        "the fill runs the bar's length"
    );
    assert_eq!(bar.height, 28, "the stock titlebar is 28 pixels tall");

    // The title: strokes in the title colour, on a transparent canvas (the bar's fill is the bar's).
    let (ink, strongest) = title.ink();
    assert!(ink >= 20, "the title paints strokes, got {ink} pixels");
    assert!(
        strongest >= 128,
        "the title has pixels that are mostly covered, got {strongest}"
    );
    let colours: Vec<[u8; 4]> = title
        .picture()
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .filter(|pixel| pixel[3] == 255)
        .collect();
    assert!(
        colours.iter().all(|pixel| pixel[..3] == [0xc6, 0xd0, 0xf5]),
        "a fully covered pixel of the title is the title colour #c6d0f5"
    );

    // The close icon: a 16 pixel square of strokes.
    assert_eq!((close.width, close.height), (16, 16));
    let (ink, _) = close.ink();
    assert!(ink >= 10, "the close icon paints strokes, got {ink} pixels");
    assert_eq!(
        close.picture().pixel(0, 0),
        Some([0, 0, 0, 0]),
        "the icon's corner is clear"
    );

    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

/// A `Text` the shell makes asks the compositor for its width and takes the answer: the width stops being the
/// `size / 2` per character estimate and becomes what the painter lays the string out in.
#[test]
fn a_text_node_takes_the_width_the_compositor_measures() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    role.run(
        "measured text",
        "probe = root:add(Text({ text = 'Measured text', size = 20 })) return tostring(probe.width)",
    )
    .expect("the text is made");
    tick_until(&mut role, "the compositor never answered the measure", |role| {
        role.run("measured?", "return tostring(probe.measured.key ~= nil)")
            .expect("the probe runs")
            .expect("the probe returns text")
            == "true"
    });
    let width = role
        .run("width", "return tostring(probe.width)")
        .expect("the probe runs")
        .expect("the probe returns text")
        .parse::<f64>()
        .expect("a width") as u32;
    let guess = 13 * 20 / 2; // thirteen characters at half the size each
    assert!(width > 0, "the answered width is positive");
    assert_ne!(
        width, guess,
        "the answer is the painter's layout, not the estimate ({guess})"
    );
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}
