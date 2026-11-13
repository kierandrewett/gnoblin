//! The stock shell VM against a real compositor session (design/node-api-refactor.md, section 7.2).
//!
//! The shell connects through the runtime role's own path: [`connect_owner_socket_with_limits`] reads the
//! session's IPC socket from the environment and boots a [`ShellVm`] over the stock Lua
//! library, under the shipped budgets with only the handler and frame deadlines widened (`patient_handlers`); the
//! stock shell loads under the shipped load budget. The tree is the compositor's: the nodes come from the session, not from a fake.

mod support;

#[path = "../../xdg-desktop-portal-gnoblin/tests/support/bus.rs"]
mod bus;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_lua::Limits;
use gnoblin_lua::shell_role::{ShellRole, connect_owner_socket_with_limits, connect_session_shell_with_limits};
use gnoblin_lua::shell_vm::ShellLimits;
use support::{Client, Configure, Locker, Session, Window, wait_for_listed_id};
use wayland_protocols::xdg::shell::client::xdg_toplevel::State as ToplevelState;

/// How long a test waits for the session or the VM to reach a state before it fails. A passing run never waits this
/// long; it is the margin a busy machine gets, so a failure is a state that never came and not a slow turn.
const PATIENCE: Duration = Duration::from_secs(60);

/// The budgets the tests that run handlers (every test that ticks the shell) run the VM under: the shipped ones, with
/// the wall-clock handler and frame deadlines widened and nothing else changed. In particular the load budget is the
/// shipped one, so loading the stock shell and the user's files is tested as production runs it.
///
/// The instruction and memory budgets are what stop a runaway script, and they count the same on every machine. The
/// handler and frame deadlines are elapsed time (100 ms and 500 ms), so a test machine that is busy (a full workspace
/// build and test run share its cores) can pass them inside a handler without the script being slow: a run under
/// twelve busy loops on four cores failed `Window on_ready handler: the handler deadline (100 ms) passed`. A test
/// that fails on that measures the machine, so these two deadlines are a minute.
fn patient_handlers() -> ShellLimits {
    const DEADLINE_TICKS: u64 = 60_000;
    let shipped = ShellLimits::default();
    ShellLimits {
        entry: Limits {
            handler_ticks: DEADLINE_TICKS,
            ..shipped.entry
        },
        frame: Limits {
            handler_ticks: DEADLINE_TICKS,
            ..shipped.frame
        },
        load: shipped.load,
    }
}

/// The stock Lua library the shell VM loads (`node.lua` and the modules beside it).
fn stock_library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

/// Connects the shell VM to `session` as the runtime role does, with the socket in the environment.
fn connect(session: &Session) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_owner_socket_with_limits(&env, &stock_library(), patient_handlers())
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

/// Loads the stock shell inside the VM the way production does, under the load budget: `load_shell` runs
/// `require("gnoblin.shell")`, which loads `shell/init.lua` from the shipped Lua root and its siblings.
/// `sys` is the host's system table, which the shell reads when it loads; the test supplies it inert, and the load
/// leaves a table that is already there.
fn load_stock_shell(role: &mut ShellRole) {
    role.run(
        "sys",
        "sys = sys or { spawn = function() end, date = function() return '' end }",
    )
    .expect("the host table is set");
    let errors = role
        .load_shell(&stock_library(), None)
        .unwrap_or_else(|error| panic!("gnoblin.shell loads in the shell VM: {error:?}"));
    assert!(errors.is_empty(), "{errors:?}");
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

/// Starts the shell the way the runtime role does: [`connect_session_shell_with_limits`] over the session's owner socket
/// attaches the VM, loads the stock shell from the shipped library, then the user's files in `config`.
/// `crates/gnoblin-lua/src/role.rs` passes `connect_session_shell`, which is this under the default budgets, to
/// `ShellSlot::from_env`; the load budget is the same, and the handler deadlines are widened for a busy machine.
fn connect_session(session: &Session, config: &Path) -> ShellRole {
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    connect_session_shell_with_limits(&env, &stock_library(), Some(config), patient_handlers())
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
        match connect_owner_socket_with_limits(&env, &stock_library(), patient_handlers()) {
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
        "a fully covered pixel of the title is the title colour #c6d0f5, got {:?}",
        colours.iter().find(|pixel| pixel[..3] != [0xc6, 0xd0, 0xf5])
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

// ---- The stock titlebar on screen: pixels the compositor composed -------------------------------------------------

/// A PNG the compositor wrote: RGBA8, rows packed.
struct Shot {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Shot {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        self.rgba[at..at + 4].try_into().expect("a pixel inside the shot")
    }

    /// The pixels of the rectangle `x0..x1` by `y0..y1` that are not `fill`, clipped to the shot.
    fn ink(&self, x0: u32, x1: u32, y0: u32, y1: u32, fill: [u8; 4]) -> usize {
        (y0..y1.min(self.height))
            .flat_map(|y| (x0..x1.min(self.width)).map(move |x| (x, y)))
            .filter(|&(x, y)| self.pixel(x, y) != fill)
            .count()
    }

    /// Whether some pixel of the rectangle is within `tolerance` of `colour` on every channel.
    fn has_colour(&self, x0: u32, x1: u32, y0: u32, y1: u32, colour: [u8; 3], tolerance: u8) -> bool {
        (y0..y1.min(self.height))
            .flat_map(|y| (x0..x1.min(self.width)).map(move |x| (x, y)))
            .any(|(x, y)| {
                let pixel = self.pixel(x, y);
                pixel[..3].iter().zip(colour).all(|(a, b)| a.abs_diff(b) <= tolerance)
            })
    }
}

/// What `View:capture(path)` writes for `view`, read back. The View draws on a frame of its own, so an ask that finds
/// no picture (`unavailable`) is made again.
fn capture_view(session: &Session, role: &mut ShellRole, view: &str, name: &str) -> Shot {
    let path = std::fs::canonicalize(session.directory())
        .expect("the runtime directory")
        .join(name);
    let key = name.replace(['.', '-'], "_");
    let ask = format!(
        "shots = shots or {{}} shots.{key} = nil \
         go(function() local result, err = await({view}:capture('{path}')) \
         shots.{key} = {{ result = result, err = err }} end) return 'asked'",
        path = path.display(),
    );
    let probe = format!("local s = shots.{key} return s and (s.err .. '|' .. tostring(s.result)) or ''");
    let mut asked = false;
    let mut answer = String::new();
    tick_until(role, "the capture was never answered", |role| {
        if !asked {
            role.run("ask", &ask).expect("the ask runs");
            asked = true;
        }
        let outcome = role.run("answer", &probe).expect("the probe runs").unwrap_or_default();
        if outcome.starts_with("unavailable|") {
            asked = false;
        } else if !outcome.is_empty() {
            answer = outcome;
        }
        !answer.is_empty()
    });
    assert_eq!(
        answer,
        format!("|{}", path.display()),
        "the capture is answered with its path"
    );
    let decoder = png::Decoder::new(std::io::Cursor::new(std::fs::read(&path).expect("the PNG was written")));
    let mut reader = decoder.read_info().expect("a PNG");
    let mut rgba = vec![0; reader.output_buffer_size().expect("a sized PNG")];
    let info = reader.next_frame(&mut rgba).expect("a frame");
    assert_eq!(info.color_type, png::ColorType::Rgba, "the capture is RGBA");
    rgba.truncate(info.buffer_size());
    Shot {
        width: info.width,
        height: info.height,
        rgba,
    }
}

fn parse_numbers(text: &str) -> Vec<f64> {
    text.split(',')
        .map(|field| field.parse().unwrap_or_else(|_| panic!("a number in {text:?}")))
        .collect()
}

/// A mapped window's stock `Titlebar` is on screen: a View of the window, captured to a PNG, holds the bar's fill, the
/// title's glyphs inside the extent the compositor measured for them and nowhere else along the bar, and the close
/// button's strokes at the button's place. This is the end to end of `Canvas`: the shell's draw lists, the tree's
/// `bad-paint` check, the painter, the composer's picture and the software view renderer.
#[test]
fn the_stock_titlebar_paints_title_glyphs_and_a_close_button_where_the_shell_put_them() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    wait_for_sized_workspace(&mut role);
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.titled", "A Title");

    // The title's measure has come back: the painter's width, height and ascent, and not the estimate.
    tick_until(&mut role, "the title was never measured", |role| {
        role.run(
            "measured",
            "local w = root:all('Window')[1] local tb = w and w:all(Titlebar)[1] \
             local t = tb and tb:all(Text)[1] return tostring(t ~= nil and t.measured.key ~= nil)",
        )
        .expect("the probe runs")
        .expect("the probe returns text")
            == "true"
    });
    let geometry = role
        .run(
            "geometry",
            "local w = root:all('Window')[1] local tb = w:all(Titlebar)[1] local t = tb:all(Text)[1] \
             local i = tb:all(Icon)[1] \
             return table.concat({ w.x, w.y, w.width, w.height, tb.x, tb.y, tb.width, tb.height, \
             t.x, t.y, t.width, t.height, t.measured.width, t.measured.height, t.measured.ascent, \
             i.x, i.y, i.width, i.height }, ',') .. '|' .. tb.fill .. '|' .. t.text .. '|' .. i.color",
        )
        .expect("the probe runs")
        .expect("the probe returns text");
    let mut parts = geometry.split('|');
    let numbers = parse_numbers(parts.next().expect("the numbers"));
    let fill_text = parts.next().expect("the fill");
    assert_eq!(parts.next(), Some("A Title"), "the title is the window's");
    let icon_colour = parts.next().expect("the icon colour");
    let [
        window_x,
        window_y,
        window_width,
        window_height,
        bar_x,
        bar_y,
        bar_width,
        bar_height,
        title_x,
        title_y,
        title_width,
        title_height,
        measured_width,
        measured_height,
        measured_ascent,
        icon_x,
        icon_y,
        icon_width,
        icon_height,
    ] = numbers[..]
    else {
        panic!("nineteen numbers, got {numbers:?}");
    };
    // The measure round trip: the line box is the painter's (`floor(13 * 1.5)`), the baseline lies inside it, and the
    // width is a real text's width, not the `13 * 7 / 2` estimate of seven characters.
    assert_eq!(measured_height, 19.0, "a 13 pixel line box is 19 pixels tall");
    assert!(
        measured_ascent > 9.0 && measured_ascent < measured_height,
        "the baseline lies in the lower half of the line box: {measured_ascent}"
    );
    assert!(
        measured_width > 20.0 && measured_width < title_width,
        "'A Title' at 13 pixels is some tens of pixels: {measured_width}"
    );
    assert_ne!(measured_width, 45.0, "the width is not the estimate");
    assert_eq!(bar_height, 28.0, "the stock titlebar is 28 pixels tall");
    assert_eq!(bar_width, window_width, "the bar is as wide as its window");

    // A View of the window, large enough to hold it where the shell put it, shows what the compositor composed.
    let (view_width, view_height) = (
        (window_x + window_width).ceil() as u32,
        (window_y + window_height).ceil() as u32,
    );
    role.run(
        "a view of the window",
        &format!(
            "local w = root:all('Window')[1] \
             titled = View({{ width = {view_width}, height = {view_height}, source = w }}) root:add(titled) return 'made'"
        ),
    )
    .expect("the View is made");
    let hex = |text: &str| -> [u8; 3] {
        let digits = text.trim_start_matches('#');
        [0, 2, 4].map(|at| u8::from_str_radix(&digits[at..at + 2], 16).expect("a hex colour"))
    };
    let fill = {
        let [r, g, b] = hex(fill_text);
        [r, g, b, 255]
    };
    let (ox, oy) = ((window_x + bar_x) as u32, (window_y + bar_y) as u32);
    // The shot is taken again until the bar has been painted into the View, which draws on a frame of its own.
    let mut shot = capture_view(&session, &mut role, "titled", "titlebar-0.png");
    for attempt in 1..40 {
        if shot.pixel(ox + 2, oy + 2) == fill {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
        shot = capture_view(&session, &mut role, "titled", &format!("titlebar-{attempt}.png"));
    }
    assert_eq!((shot.width, shot.height), (view_width, view_height));

    // The bar: opaque fill at both corners, and the window's transparent buffer (black in a View) below it.
    let far = (ox + bar_width as u32 - 2, oy + bar_height as u32 - 2);
    assert_eq!(shot.pixel(ox + 2, oy + 2), fill, "the bar's near corner is its fill");
    assert_eq!(shot.pixel(far.0, far.1), fill, "the bar's far corner is its fill");
    assert_ne!(
        shot.pixel(ox + 100, oy + bar_height as u32 + 20),
        fill,
        "below the bar the window shows no fill"
    );

    // The title: glyph strokes inside the measured extent, in the title colour at full cover somewhere, and nothing
    // between the end of the measured text and the close button.
    let (tx, ty) = (ox + title_x as u32, oy + title_y as u32);
    let title_end = tx + measured_width as u32 + 2;
    let ink = shot.ink(tx, title_end, ty, ty + title_height as u32, fill);
    assert!(ink >= 20, "the title paints glyph strokes, got {ink} pixels");
    assert!(
        shot.has_colour(tx, title_end, ty, ty + title_height as u32, [0xc6, 0xd0, 0xf5], 24),
        "some stroke of the title is its colour #c6d0f5"
    );
    assert_eq!(
        shot.ink(title_end + 1, ox + icon_x as u32, oy, oy + bar_height as u32, fill),
        0,
        "the bar is plain fill between the end of the title and the close button"
    );

    // The close button: strokes in the icon's colour inside its square, its corners clear.
    let (ix, iy) = (ox + icon_x as u32, oy + icon_y as u32);
    let (iw, ih) = (icon_width as u32, icon_height as u32);
    assert_eq!((iw, ih), (16, 16), "the stock icon is 16 pixels");
    assert!(
        shot.ink(ix, ix + iw, iy, iy + ih, fill) >= 10,
        "the close button paints strokes"
    );
    assert!(
        shot.has_colour(ix, ix + iw, iy, iy + ih, hex(icon_colour), 24),
        "some stroke of the button is its colour {icon_colour}"
    );
    assert_eq!(shot.pixel(ix, iy), fill, "the button's corner is clear");
    assert_eq!(shot.pixel(ix + iw - 1, iy + ih - 1), fill, "and so is its far corner");
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

// ---- Loading under the load budget --------------------------------------------------------------------------------

/// The shipped budgets with the handler and frame limits cut to what could run nothing: one thousand instructions and
/// no time. The load budget is the shipped one.
fn starved_frames() -> ShellLimits {
    let starved = Limits {
        instructions: 1_000,
        memory_bytes: 64 * 1024 * 1024,
        handler_ticks: 0,
    };
    ShellLimits {
        entry: starved,
        frame: starved,
        ..ShellLimits::default()
    }
}

/// The regression: `require("gnoblin.shell")` ran under the frame budget, so a machine busy enough to pass 500 ms
/// failed the shell's start. Loading is not a frame: a VM whose frames could run nothing still boots `node.lua`
/// and loads the stock shell and the user's files.
#[test]
fn the_stock_shell_and_the_users_files_load_though_a_frame_could_run_nothing() {
    let session = Session::start();
    let config = user_config(&[("shell.lua", "user_ran = true")]);
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    let mut role = connect_owner_socket_with_limits(&env, &stock_library(), starved_frames())
        .expect("node.lua boots under the load budget");
    // The control: the frame budget refuses even a short loop, so the load below did not pass for lack of work.
    role.run("control", "local n = 0 for i = 1, 20000 do n = n + i end")
        .expect_err("a frame's budget cannot run this");
    let errors = role
        .load_shell(&stock_library(), Some(config.path()))
        .expect("the stock shell loads under the load budget");
    assert!(errors.is_empty(), "{errors:?}");
}

/// The shipped load budget is far wider than the stock shell needs: it loads inside a fiftieth of the instructions
/// (measured at about twenty thousand, against a budget of a hundred million), so a user's own files and a slower
/// stock shell fit with room to spare.
#[test]
fn the_stock_shell_needs_under_a_fiftieth_of_the_shipped_load_instruction_budget() {
    let session = Session::start();
    let shipped = ShellLimits::default();
    let limits = ShellLimits {
        load: Limits {
            instructions: shipped.load.instructions / 50,
            ..shipped.load
        },
        ..shipped
    };
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    let mut role = connect_owner_socket_with_limits(&env, &stock_library(), limits).expect("node.lua boots");
    role.load_shell(&stock_library(), None)
        .expect("the stock shell loads inside a fiftieth of the budget");
}

/// A user file that never ends, through the runtime's own entry: it is stopped by the load budget (here test-sized),
/// reported once naming the file, and the stock shell and the files after it still load. The shell then runs.
#[test]
fn a_runaway_user_file_is_stopped_by_the_load_budget_and_the_shell_still_runs() {
    let session = Session::start();
    let config = user_config(&[
        ("shell/10-before.lua", "ran_before = true"),
        ("shell/20-loop.lua", "while true do end"),
        ("shell/30-after.lua", "ran_after = true"),
    ]);
    let socket = session.socket().to_str().expect("a UTF-8 socket path").to_owned();
    let env = move |name: &str| (name == gnoblin_client::unix::SOCKET_VARIABLE).then(|| socket.clone());
    let patient = patient_handlers();
    let limits = ShellLimits {
        load: Limits {
            instructions: 3_000_000,
            handler_ticks: 600_000,
            ..patient.load
        },
        ..patient
    };
    let started = Instant::now();
    let mut role = connect_session_shell_with_limits(&env, &stock_library(), Some(config.path()), limits)
        .expect("a runaway user file does not fail the start");
    assert!(
        started.elapsed() < PATIENCE,
        "the loop was stopped by its instruction budget, not left to the clock: {:?}",
        started.elapsed()
    );
    let errors = role.user_file_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("20-loop.lua") && errors[0].contains("instruction budget"),
        "{errors:?}"
    );
    let flags = role
        .run(
            "flags",
            "return tostring(ran_before) .. ' ' .. tostring(ran_after) .. ' ' .. tostring(seat ~= nil)",
        )
        .expect("the probe runs");
    assert_eq!(flags.as_deref(), Some("true true true"));
    tick_until(&mut role, "the shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
}

/// A shell asks a View for a still with `view:capture(path)`: the compositor writes a PNG of what the View shows and
/// answers the path, and it answers a path outside the session's runtime directory with an error and writes nothing.
#[test]
fn a_view_capture_the_shell_asks_for_writes_a_png_and_answers_its_path() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    role.run(
        "a view of a canvas",
        "local canvas = Canvas({ width = 40, height = 30, draw = { { rect = { 0, 0, 40, 30 }, fill = '#2040c0' } } }) \
         root:add(canvas) \
         view = View({ width = 40, height = 30, source = canvas }) \
         root:add(view) \
         shots = {} \
         return 'made'",
    )
    .expect("the View is made");

    let directory = std::fs::canonicalize(session.directory()).expect("the runtime directory");
    let target = directory.join("shot.png");
    let outside = tempfile::tempdir().expect("another directory");
    let elsewhere = outside.path().join("shot.png");
    for (name, path) in [("inside", &target), ("outside", &elsewhere)] {
        let ask = format!(
            "shots.{name} = nil go(function() local result, err = await(view:capture('{}')) \
             shots.{name} = {{ result = result, err = err }} end) return 'asked'",
            path.display()
        );
        let probe = format!("local s = shots.{name} return s and (s.err .. '|' .. tostring(s.result)) or ''");
        let mut asked = false;
        let mut answer = String::new();
        // The View draws on a frame of its own, so a first ask may find no picture: ask again until the answer is
        // not `unavailable`.
        tick_until(&mut role, "the capture was never answered", |role| {
            if !asked {
                role.run("ask", &ask).expect("the ask runs");
                asked = true;
            }
            let outcome = role.run("answer", &probe).expect("the probe runs").unwrap_or_default();
            if outcome.starts_with("unavailable|") {
                asked = false;
            } else if !outcome.is_empty() {
                answer = outcome;
            }
            !answer.is_empty()
        });
        if name == "inside" {
            assert_eq!(
                answer,
                format!("|{}", target.display()),
                "the shell is answered with the path written"
            );
        } else {
            assert_eq!(answer, "bad-value|", "a path outside the runtime directory is refused");
        }
    }
    let png = std::fs::read(&target).expect("the PNG was written");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "the file is a PNG");
    assert!(!elsewhere.exists(), "nothing was written outside the runtime directory");
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

// ---- Maximise: the stock shell's answer to a control request (`gnoblin-2jv2`) --------------------------------------
//
// A toolkit window is maximised with `gnoblinctl window call ID set_maximized value=true`. The request is an event on
// the window's node, the stock shell answers it with `Window:maximise` (the flag and the work area's rectangle, in one
// transaction), and the compositor turns that into the client's configure. The client draws at every size it is told,
// as GTK does, and the compositor must not configure it back to the size it had.

/// The size a toolkit's window draws at before anyone asks it for another.
const OWN_SIZE: (i32, i32) = (800, 600);

/// The size of the stock shell's work area, which `Window:maximise` fills.
fn work_area(role: &mut ShellRole) -> (i32, i32) {
    let text = role
        .run(
            "work area",
            "local a = work_area() return tostring(math.floor(a.w)) .. ' ' .. tostring(math.floor(a.h))",
        )
        .expect("the probe runs")
        .expect("the probe returns text");
    let mut parts = text.split(' ').map(|n| n.parse::<i32>().expect("a size"));
    (parts.next().expect("a width"), parts.next().expect("a height"))
}

/// How long the shell and the client run together after the last thing the test waited for, so a configure the
/// compositor sends late has time to arrive.
const SETTLE: Duration = Duration::from_millis(500);

/// The shell VM and a toolkit client, run together: the VM reads what the compositor published, and the client reads
/// what the session sent and answers every new configure by drawing at its size, as a toolkit does.
struct Together<'a> {
    role: &'a mut ShellRole,
    client: &'a mut Client,
    window: &'a Window,
    clock: Instant,
    /// How many configures the client has answered with a draw; one that arrives while it draws is answered next.
    answered: usize,
}

impl Together<'_> {
    /// Runs both until `done` holds, or fails with `what` after the patience a busy machine gets.
    fn until(&mut self, what: &str, mut done: impl FnMut(&Client) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            self.role
                .tick(self.clock.elapsed().as_secs_f64())
                .expect("a frame runs");
            self.client.roundtrip();
            // What the client knows when it decides to draw: a configure the commit itself provokes is a new one.
            let known = self.client.configure_log().len();
            if known > self.answered {
                self.answered = known;
                self.client.draw_configured(self.window, OWN_SIZE);
            }
            if done(self.client) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: the client was configured {:#?}; VM diagnostics {:?}",
                self.client.configure_log(),
                self.role.diagnostics()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs both for [`SETTLE`] with nothing to wait for.
    fn settle(&mut self) {
        let until = Instant::now() + SETTLE;
        self.until("the settle never ended", |_| Instant::now() >= until);
    }
}

/// Whether a configure is the window maximised at `size`.
fn is_maximised_at(configure: &Configure, size: (i32, i32)) -> bool {
    configure.states.contains(&ToplevelState::Maximized) && (configure.width, configure.height) == size
}

/// Fails unless the client was configured to `size`, maximised or not as `maximised` says, by the configures in `log`,
/// and was never configured to anything else after that: a later configure with another size or state is the window
/// snapping back.
#[track_caller]
fn assert_configured_for_good(log: &[Configure], maximised: bool, size: (i32, i32)) {
    let is_maximised = |configure: &Configure| configure.states.contains(&ToplevelState::Maximized);
    let arrived = log
        .iter()
        .position(|configure| (configure.width, configure.height) == size && is_maximised(configure) == maximised)
        .unwrap_or_else(|| panic!("the client was never configured to {size:?} (maximised {maximised}): {log:#?}"));
    for later in &log[arrived..] {
        assert_eq!(
            ((later.width, later.height), is_maximised(later)),
            (size, maximised),
            "the window was configured again after it reached {size:?} (maximised {maximised}): {:#?}",
            &log[arrived..]
        );
    }
}

#[test]
fn a_window_the_stock_shell_maximises_is_not_configured_back_to_its_old_size() {
    let session = Session::start();
    let mut role = connect(&session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    wait_for_sized_workspace(&mut role);
    let mut client = Client::connect(session.wayland());
    let window = client.toolkit_window("org.example.toolkit", "Toolkit", OWN_SIZE);
    tick_until(&mut role, "the shell never placed the window", |role| {
        window_placements(role).iter().any(|(_, _, placed)| *placed)
    });
    let id = wait_for_listed_id(&session, "Toolkit").to_string();
    let area = work_area(&mut role);
    let mut together = Together {
        answered: client.configure_log().len(),
        role: &mut role,
        client: &mut client,
        window: &window,
        clock: Instant::now(),
    };

    let asked = together.client.configure_log().len();
    let maximise = |value: &str| session.ctl(&["window", "call", &id, "set_maximized", value]).ok();
    maximise("value=true");
    together.until("the window was never maximised to the work area", |client| {
        client
            .configure_log()
            .last()
            .is_some_and(|last| is_maximised_at(last, area))
    });
    together.settle();
    assert_configured_for_good(&together.client.configure_log()[asked..], true, area);

    let asked = together.client.configure_log().len();
    maximise("value=false");
    together.until("the window never left maximise", |client| {
        client
            .configure_log()
            .last()
            .is_some_and(|last| !last.states.contains(&ToplevelState::Maximized))
    });
    together.settle();
    assert_configured_for_good(&together.client.configure_log()[asked..], false, OWN_SIZE);
    assert!(
        together.role.diagnostics().is_empty(),
        "no refused transaction: {:?}",
        together.role.diagnostics()
    );
}

// ---- Gestures: a swipe on the real input path reaches the stock shell's Lua and changes shell state ---------------------
//
// The session is a real compositor, the shell is the stock shell in a real VM over the owner socket, and the swipe is
// fed to the session's own input path (`Session::input`), which takes it through the recogniser and the claim table
// exactly as a touchpad's sample goes. EIS has no gestures, so this is the nearest the input path gets to a device.

/// One touchpad sample of a swipe with `fingers` contacts that moved `dx` and `dy` since the last sample.
fn swipe_sample(
    phase: gnoblin_backend::RawTouchpadPhase,
    fingers: u8,
    dx: f64,
    dy: f64,
) -> gnoblin_backend::InputEvent {
    gnoblin_backend::InputEvent::Touchpad(gnoblin_backend::RawTouchpadSample {
        device: gnoblin_types::DeviceId::new(1),
        phase,
        kind: gnoblin_backend::RawTouchpadKind::Swipe,
        fingers,
        dx,
        dy,
        scale: 1.0,
        angle: 0.0,
    })
}

/// Begins a swipe of `fingers` fingers and moves it left by `steps` samples of 50 units each.
fn swipe_left(session: &Session, fingers: u8, steps: usize) {
    use gnoblin_backend::RawTouchpadPhase::{Begin, Update};
    assert!(session.input().inject(swipe_sample(Begin, fingers, 0.0, 0.0)));
    for _ in 0..steps {
        assert!(session.input().inject(swipe_sample(Update, fingers, -50.0, 0.0)));
    }
}

/// Lifts the fingers of the swipe in flight.
fn lift(session: &Session, fingers: u8) {
    assert!(
        session
            .input()
            .inject(swipe_sample(gnoblin_backend::RawTouchpadPhase::End, fingers, 0.0, 0.0))
    );
}

/// The name of the workspace the Desk under the pointer shows, read inside the VM.
fn active_workspace(role: &mut ShellRole) -> String {
    role.run("active workspace", "return tostring(desk().active.name)")
        .expect("the probe runs")
        .expect("the probe returns text")
}

/// How far the active workspace is displaced along x, read inside the VM: a swipe in flight pulls it.
fn active_offset(role: &mut ShellRole) -> f64 {
    role.run("active offset", "return tostring(desk().active.x)")
        .expect("the probe runs")
        .expect("the probe returns text")
        .parse()
        .expect("a number")
}

/// A stock shell on a started session with its Desk sized: the state every gesture test starts from.
fn shell_with_desk(session: &Session) -> ShellRole {
    let mut role = connect(session);
    load_stock_shell(&mut role);
    tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
        mirror_counts(role) == (1, 1)
    });
    tick_until(&mut role, "the stock shell never built the screen's Desk", |role| {
        desk_probe(role) == "1 1"
    });
    wait_for_sized_workspace(&mut role);
    role
}

/// Swipes three fingers left until the stock shell follows them, which says its claim of `Swipe3` is in force.
///
/// The shell's claim reaches the compositor a frame or two after it loads, and a swipe that begins before it is
/// nobody's: it does nothing and is not delivered. So the swipe is begun again until the workspace is pulled; the
/// swipe that is, is left in flight, with the workspace displaced.
fn swipe_until_followed(session: &Session, role: &mut ShellRole) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        swipe_left(session, 3, 3);
        let mut now = 0.0;
        for _ in 0..40 {
            role.tick(now).expect("a frame runs");
            now += 0.016;
            if active_offset(role) < 0.0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        lift(session, 3);
        assert!(
            Instant::now() < deadline,
            "the stock shell never followed a swipe: diagnostics {:?}",
            role.diagnostics()
        );
    }
}

#[test]
fn a_three_finger_swipe_on_the_input_path_slides_the_stock_shell_to_the_next_workspace() {
    let session = Session::start();
    let mut role = shell_with_desk(&session);
    assert_eq!(active_workspace(&mut role), "1");

    // The fingers move left and the shell follows them: the workspace is pulled before the fingers lift.
    swipe_until_followed(&session, &mut role);
    assert_eq!(
        active_workspace(&mut role),
        "1",
        "the slide has not finished while the fingers are down"
    );
    let pulled = role
        .run("next shown", "return tostring(desk().space.children[2].visible)")
        .expect("the probe runs")
        .expect("the probe returns text");
    assert_eq!(pulled, "true", "the workspace the fingers bring in is showing");

    // They lift after more than a quarter of the travel, so the slide finishes: the next workspace is active.
    lift(&session, 3);
    tick_until(&mut role, "the swipe never changed the active workspace", |role| {
        active_workspace(role) == "2"
    });
    assert!(
        role.diagnostics().is_empty(),
        "no handler failed and no transaction was refused: {:?}",
        role.diagnostics()
    );
}

#[test]
fn a_swipe_that_is_short_of_the_commit_slides_back_and_a_gesture_nobody_claimed_does_nothing() {
    let session = Session::start();
    let mut role = shell_with_desk(&session);
    swipe_until_followed(&session, &mut role);
    // The fingers go back to where they began before they lift: the workspace is not changed.
    use gnoblin_backend::RawTouchpadPhase::Update;
    for _ in 0..3 {
        assert!(session.input().inject(swipe_sample(Update, 3, 50.0, 0.0)));
    }
    lift(&session, 3);
    tick_until(&mut role, "the workspace never settled back", |role| {
        active_offset(role) == 0.0
    });
    assert_eq!(
        active_workspace(&mut role),
        "1",
        "a swipe that went nowhere changed nothing"
    );

    // Four fingers are nobody's claim: the stock shell binds three, so the swipe is dropped and moves nothing.
    swipe_left(&session, 4, 4);
    lift(&session, 4);
    // A claimed swipe after it proves the four-finger one was routed first.
    swipe_until_followed(&session, &mut role);
    assert_eq!(
        active_workspace(&mut role),
        "1",
        "the four-finger swipe changed nothing and the three-finger one has not finished"
    );
    lift(&session, 3);
    tick_until(&mut role, "the three-finger swipe never finished", |role| {
        active_workspace(role) == "2"
    });
}

/// The evdev codes of the keys the stock shell's `<Super>Left` chord is made of.
const KEY_LEFTMETA: u32 = 125;
const KEY_LEFT: u32 = 105;

/// Plays Super+Left on the EI keyboard: press and release of each key, innermost last.
fn play_super_left(remote: &mut support::ei::EiClient) {
    remote.typing(KEY_LEFTMETA, true);
    remote.typing(KEY_LEFT, true);
    remote.typing(KEY_LEFT, false);
    remote.typing(KEY_LEFTMETA, false);
}

#[test]
fn a_shell_that_goes_gives_back_the_chord_it_claimed_and_a_replacement_claims_again() {
    let session = Session::start();
    let mut role = shell_with_desk(&session);
    // The stock shell's `<Super>Left` moves a Strip's focus and does nothing on any other layout; this counts instead.
    role.run(
        "count",
        "hits = 0 seat.data.keys['<Super>Left'] = function() hits = hits + 1 end",
    )
    .expect("the probe runs");
    let mut client = support::ei::focused_client(&session);
    tick_until(&mut role, "the client's window never reached the tree", |role| {
        role.run("windows", "return tostring(#root:all('Window'))")
            .expect("the probe runs")
            .as_deref()
            == Some("1")
    });
    role.run("focus", "seat.focus = root:all('Window')[1]")
        .expect("the probe runs");
    let deadline = Instant::now() + PATIENCE;
    while client.keyboard_focus().is_empty() {
        role.tick(0.0).expect("a frame runs");
        client.roundtrip();
        assert!(Instant::now() < deadline, "the client never gained keyboard focus");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut remote = support::ei::keyboard(&session);

    // The claim is in force once the shell hears the chord, and no key of the chord but Super reached the client.
    let hits = |role: &mut ShellRole| -> u32 {
        role.run("hits", "return tostring(hits)")
            .expect("the probe runs")
            .expect("the probe returns text")
            .parse()
            .expect("a count")
    };
    let deadline = Instant::now() + PATIENCE;
    while hits(&mut role) == 0 {
        play_super_left(&mut remote);
        role.tick(0.0).expect("a frame runs");
        client.roundtrip();
        assert!(Instant::now() < deadline, "the shell never heard its claimed chord");
        std::thread::sleep(Duration::from_millis(20));
    }
    client.roundtrip();
    assert!(
        !client.keys().iter().any(|&(key, _)| key == KEY_LEFT),
        "the claimed chord reached the client: {:?}",
        client.keys()
    );

    // The shell goes. With nobody to hear it the chord is not withheld: its Left reaches the client.
    drop(role);
    let deadline = Instant::now() + PATIENCE;
    while !client.keys().iter().any(|&(key, state)| key == KEY_LEFT && state == 1) {
        play_super_left(&mut remote);
        client.roundtrip();
        assert!(
            Instant::now() < deadline,
            "the chord the gone shell claimed is still withheld: {:?}",
            client.keys()
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // A replacement shell claims afresh: its claim stands, and the chord is the new shell's again.
    let mut replacement = reconnect(&session);
    load_stock_shell(&mut replacement);
    tick_until(&mut replacement, "the replacement never built a Desk", |role| {
        desk_probe(role) == "1 1"
    });
    replacement
        .run(
            "count",
            "hits = 0 seat.data.keys['<Super>Left'] = function() hits = hits + 1 end",
        )
        .expect("the probe runs");
    let deadline = Instant::now() + PATIENCE;
    while hits(&mut replacement) == 0 {
        play_super_left(&mut remote);
        replacement.tick(0.0).expect("a frame runs");
        client.roundtrip();
        assert!(Instant::now() < deadline, "the replacement never heard the chord");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- Prompts: the stock shell replaces the built-in dialogs (data/lua/gnoblin/shell/prompts.lua) --------------------
//
// The compositor is a running session, the portal relay, the input role and the services role are real privileged
// connections, the shell is the stock shell in a real VM over the owner socket, and the keys come from an EI client
// on the session's own input path. The frame clock is the wall clock, as the runtime role runs it: the compositor's
// 500 ms arming is wall time, and the dialog arms on the same clock.

mod prompts {
    use std::collections::HashMap;
    use std::os::fd::IntoRawFd as _;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

    use gnoblin_client::unix::UnixTransport;
    use gnoblin_client::wire::{PortalRequest, VerifiedCaller, portal_key, portal_ui, read_portal_outcome};
    use gnoblin_client::{Client as IpcClient, Event};
    use gnoblin_model::{Fields, RequestCompletion};
    use gnoblin_services::keyring::KeyringAgent;
    use gnoblin_services::{Buses, Role};
    use xdg_desktop_portal_gnoblin::serve::{ServeOptions, serve};
    use xdg_desktop_portal_gnoblin::{BUS_NAME, IpcHost};
    use zbus::connection::Builder;
    use zbus::{Connection, ObjectPath, Proxy, Variant};

    use super::bus::Bus;
    use super::support::ei::{EiClient, keyboard};
    use super::support::{Locker, Session, Wire};
    use super::{
        Duration, Instant, PaintedCanvas, ShellRole, connect, connect_session, load_stock_shell, painted_canvases,
        user_config,
    };

    const KEY_ESC: u32 = 1;
    const KEY_TAB: u32 = 15;
    const KEY_ENTER: u32 = 28;
    const KEY_I: u32 = 23;
    const KEY_H: u32 = 35;

    /// The path the portal backend serves its interfaces at.
    const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

    const APP: &str = "org.example.Requester";
    const QUESTION: &str = "Share the screen?";
    /// The fills the stock consent dialog paints: the card, a focused button, an enabled one, a disabled one.
    const CARD: [u8; 3] = [0x1e, 0x1e, 0x2e];
    const FOCUSED: [u8; 3] = [0x89, 0xb4, 0xfa];
    const DISABLED: [u8; 3] = [0x2a, 0x2a, 0x3a];
    /// The colours a button's label is painted in: on the focused button, on a live one and on a disabled one.
    const LABEL_FOCUSED: [u8; 3] = [0x11, 0x11, 0x1b];
    const LABEL_LIVE: [u8; 3] = [0xcd, 0xd6, 0xf4];
    const LABEL_DISABLED: [u8; 3] = [0x6c, 0x70, 0x86];

    fn consent(timeout_ms: u64) -> PortalRequest {
        PortalRequest {
            interface: "Screenshot".to_owned(),
            caller: VerifiedCaller::new(0, APP),
            prompt: QUESTION.to_owned(),
            timeout_ms,
            options: Fields::default(),
            parent_window: String::new(),
        }
    }

    fn end_for(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Wire {
        let index = ends
            .iter()
            .position(|(name, _)| name == role)
            .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
        let (_, stream) = ends.remove(index);
        let wire = Wire::over(stream);
        wire.set_read_timeout(Duration::from_secs(5));
        wire
    }

    /// How a settled portal request ended, as the relay is told.
    fn outcome(relay: &mut Wire) -> Option<(String, String)> {
        relay.set_read_timeout(Duration::from_millis(1));
        let reply = relay.receive_within()?;
        Some(read_portal_outcome(reply.payload().unwrap_or(&[])).expect("a portal outcome reply"))
    }

    /// Every text a painted canvas of the compositor's tree holds.
    fn texts(canvases: &[PaintedCanvas]) -> Vec<String> {
        canvases
            .iter()
            .flat_map(|canvas| {
                canvas.list.ops.iter().filter_map(|op| match op {
                    gnoblin_paint::PaintOp::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect()
    }

    /// How many canvases paint a rectangle of `rgb`, opaque, with a width.
    fn filled(canvases: &[PaintedCanvas], rgb: [u8; 3]) -> usize {
        canvases
            .iter()
            .filter(|canvas| {
                canvas.list.ops.iter().any(|op| {
                    matches!(op, gnoblin_paint::PaintOp::Rect { fill, width, .. }
                        if *width > 0.0 && fill.a == 255 && [fill.r, fill.g, fill.b] == rgb)
                })
            })
            .count()
    }

    /// The colour the text `label` is painted in, when it is painted.
    fn label_colour(canvases: &[PaintedCanvas], label: &str) -> Option<[u8; 3]> {
        canvases.iter().find_map(|canvas| {
            canvas.list.ops.iter().find_map(|op| match op {
                gnoblin_paint::PaintOp::Text { text, colour, .. } if text == label => {
                    Some([colour.r, colour.g, colour.b])
                }
                _ => None,
            })
        })
    }

    /// A session with the relay, input and services roles, the stock shell loaded and its claims made, and an EI
    /// keyboard the session grants input to.
    struct Prompting {
        session: Session,
        role: Option<ShellRole>,
        relay: Wire,
        input: Wire,
        /// The services role's end as a client, for a test that offers requests as the role does. `None` when the
        /// test hands the end to the real services role instead (`services_end`).
        services: Option<IpcClient<UnixTransport>>,
        services_end: Option<UnixStream>,
        keyboard: EiClient,
        clock: Instant,
    }

    fn claimed(role: &mut ShellRole, slot: &str) -> bool {
        role.run("claimed", &format!("return tostring(slots.claimed('{slot}'))"))
            .expect("the probe runs")
            .as_deref()
            == Some("true")
    }

    impl Prompting {
        /// The stock shell loaded, with the user's files in `config` when there are some.
        fn start(config: Option<&std::path::Path>) -> Prompting {
            Prompting::start_holding(config, true)
        }

        /// [`Prompting::start`], with the services role's end either read as a client (`as_client`) or left in
        /// `services_end` for the real services role to adopt.
        fn start_holding(config: Option<&std::path::Path>, as_client: bool) -> Prompting {
            Prompting::start_configured(config, as_client, |_| {})
        }

        /// [`Prompting::start_holding`], with `configure` setting the rest of the session's configuration first.
        fn start_configured(
            config: Option<&std::path::Path>,
            as_client: bool,
            configure: impl FnOnce(&mut gnoblin_compositor::SessionConfig),
        ) -> Prompting {
            let (session, mut ends) = Session::start_privileged_configured(&["portal", "input", "services"], configure);
            let relay = end_for(&mut ends, "portal");
            let input = end_for(&mut ends, "input");
            let (_, stream) = {
                let index = ends
                    .iter()
                    .position(|(name, _)| name == "services")
                    .expect("a services end");
                ends.remove(index)
            };
            let (services, services_end) = if as_client {
                let mut services = IpcClient::connect(
                    UnixTransport::from_stream(stream).expect("a transport"),
                    gnoblin_compositor::version(),
                )
                .expect("the services handshake");
                services
                    .set_read_timeout(Some(Duration::from_millis(1)))
                    .expect("a read deadline");
                (Some(services), None)
            } else {
                (None, Some(stream))
            };
            let mut role = match config {
                Some(config) => connect_session(&session, config),
                None => {
                    let mut role = connect(&session);
                    load_stock_shell(&mut role);
                    role
                }
            };
            let keyboard = keyboard(&session);
            let clock = Instant::now();
            // Frames from here on run on the wall clock.
            let deadline = Instant::now() + super::PATIENCE;
            while !claimed(&mut role, "portal.ui") {
                role.tick(clock.elapsed().as_secs_f64()).expect("a frame runs");
                assert!(Instant::now() < deadline, "the stock shell never claimed portal.ui");
                std::thread::sleep(Duration::from_millis(5));
            }
            // A request offered before the shell has a Desk to draw on is declined, so the shell is ready for one
            // only once its Desk exists.
            while role
                .run("desk", "return tostring(desk() ~= nil)")
                .expect("the probe runs")
                .as_deref()
                != Some("true")
            {
                role.tick(clock.elapsed().as_secs_f64()).expect("a frame runs");
                assert!(Instant::now() < deadline, "the stock shell never built a Desk");
                std::thread::sleep(Duration::from_millis(5));
            }
            Prompting {
                session,
                role: Some(role),
                relay,
                input,
                services,
                services_end,
                keyboard,
                clock,
            }
        }

        fn role(&mut self) -> &mut ShellRole {
            self.role.as_mut().expect("the shell is running")
        }

        /// The services role's end as a client.
        fn services(&mut self) -> &mut IpcClient<UnixTransport> {
            self.services
                .as_mut()
                .expect("this rig holds the services end as a client")
        }

        /// Runs shell frames until `done` holds, or fails after the patience with `what`.
        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Prompting) -> bool) {
            let deadline = Instant::now() + super::PATIENCE;
            loop {
                let now = self.clock.elapsed().as_secs_f64();
                self.role().tick(now).expect("a frame runs");
                if done(self) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{what}: diagnostics {:?}",
                    self.role().diagnostics()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// Runs shell frames for `span`, so what the compositor sent has been handled.
        fn run_for(&mut self, span: Duration) {
            let end = Instant::now() + span;
            while Instant::now() < end {
                let now = self.clock.elapsed().as_secs_f64();
                self.role().tick(now).expect("a frame runs");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn key(&mut self, code: u32) {
            self.keyboard.typing(code, true);
            self.keyboard.typing(code, false);
        }

        fn painted(&self) -> Vec<PaintedCanvas> {
            painted_canvases(&self.session)
        }

        /// Waits for the consent dialog: the title, who asks and the question, all painted.
        fn dialog_shown(&mut self) {
            self.until("the consent dialog was never painted", |p| {
                let shown = texts(&p.painted());
                shown.iter().any(|t| t == "Screenshot")
                    && shown.iter().any(|t| t == &format!("Asked by {APP}"))
                    && shown.iter().any(|t| t == QUESTION)
                    && shown.iter().any(|t| t == "Deny")
                    && shown.iter().any(|t| t == "Allow")
            });
        }

        fn dialog_gone(&mut self) {
            self.until("the consent dialog never closed", |p| {
                !texts(&p.painted()).iter().any(|t| t == "Screenshot")
            });
        }

        /// Presses Escape on the built-in dialog a few times and checks the relay hears nothing: a built-in dialog
        /// open for the request would take the first and settle it.
        fn assert_no_built_in_dialog(&mut self) {
            for attempt in 0..8_u64 {
                self.input.send(&portal_key(5_000 + attempt, "escape", false));
                let _ack = self.input.receive();
                self.run_for(Duration::from_millis(30));
                assert!(
                    outcome(&mut self.relay).is_none(),
                    "the built-in dialog opened and took the key, while the shell held the claim"
                );
            }
        }

        /// Waits until the compositor's arming window is over (700 ms since `since`) and the dialog has armed itself,
        /// which its second button shows by leaving its dimmed look, and the focus has moved to where keys go.
        fn after_arming(&mut self, since: Instant) {
            while since.elapsed() < Duration::from_millis(700) {
                self.run_for(Duration::from_millis(20));
            }
            self.until("the dialog never armed", |p| {
                let canvases = p.painted();
                [label_colour(&canvases, "Allow"), label_colour(&canvases, "Unlock")].contains(&Some(LABEL_LIVE))
            });
            self.run_for(Duration::from_millis(150));
        }

        fn reply(&mut self, what: &str) -> (String, String) {
            let mut reply = None;
            self.until(what, |p| {
                reply = outcome(&mut p.relay);
                reply.is_some()
            });
            reply.expect("a reply")
        }

        /// Serves a request the built-in dialog must serve: a key on the input role settles it, and the shell keeps
        /// running meanwhile, as it does in the session.
        fn built_in_settles(&mut self) -> (String, String) {
            for attempt in 0..200_u64 {
                self.input.send(&portal_key(9_000 + attempt, "escape", false));
                let _ack = self.input.receive();
                self.run_for(Duration::from_millis(20));
                if let Some(done) = outcome(&mut self.relay) {
                    return done;
                }
            }
            panic!("the built-in dialog never served the request");
        }
    }

    #[test]
    fn the_stock_shell_claims_both_slots_and_draws_a_consent_dialog_with_deny_focused() {
        let mut p = Prompting::start(None);
        assert!(
            claimed(p.role(), "prompt.secret"),
            "the passphrase prompt is claimed too"
        );

        p.relay.set_read_timeout(Duration::from_secs(5));
        p.relay.send(&portal_ui(1, &consent(30_000)));
        p.dialog_shown();

        let canvases = p.painted();
        let card = canvases
            .iter()
            .find(|canvas| {
                canvas.list.ops.iter().any(|op| {
                    matches!(op, gnoblin_paint::PaintOp::Rect { fill, width, .. }
                        if *width >= 480.0 && fill.a == 255 && [fill.r, fill.g, fill.b] == CARD)
                })
            })
            .expect("the dialog's card is painted");
        assert_eq!(
            card.picture().pixel(card.width / 2, card.height / 2),
            Some([CARD[0], CARD[1], CARD[2], 255]),
            "the card's pixels are the card colour"
        );
        // Deny is the focused button, and Allow is not live yet: it is painted disabled until the arming window passes.
        assert_eq!(filled(&canvases, FOCUSED), 1, "one button is focused: Deny");
        assert_eq!(
            filled(&canvases, DISABLED),
            1,
            "Allow is disabled while the dialog is not armed"
        );
        assert_eq!(
            label_colour(&canvases, "Deny"),
            Some(LABEL_FOCUSED),
            "Deny's label is the focused one"
        );
        assert_eq!(
            label_colour(&canvases, "Allow"),
            Some(LABEL_DISABLED),
            "Allow's label is dimmed"
        );
        assert!(
            texts(&canvases).iter().all(|t| !t.contains('\n')),
            "no control character is painted"
        );

        // The built-in dialog stays closed while the shell holds the claim.
        p.assert_no_built_in_dialog();
        // And the dialog is secure, so a View capture would mask it: its card is marked in the compositor's tree.
        p.role()
            .run("probe", "return tostring(root:all(ConsentDialog)[1].secure)")
            .expect("the probe runs")
            .filter(|secure| secure == "true")
            .expect("the dialog is secure");
        assert!(
            p.role().diagnostics().is_empty(),
            "no handler failed and no transaction was refused: {:?}",
            p.role().diagnostics()
        );
    }

    #[test]
    fn tab_then_enter_through_the_real_input_path_allows_once_the_dialog_is_armed() {
        let mut p = Prompting::start(None);
        p.relay.set_read_timeout(Duration::from_secs(5));
        p.relay.send(&portal_ui(2, &consent(30_000)));
        let sent = Instant::now();
        p.dialog_shown();
        p.after_arming(sent);
        p.until("Allow never became live", |p| {
            let canvases = p.painted();
            label_colour(&canvases, "Allow") == Some(LABEL_LIVE) && filled(&canvases, DISABLED) == 0
        });

        p.key(KEY_TAB);
        p.until("Tab never moved the focus to Allow", |p| {
            // Both buttons are live and the focus colour is on one of them: Deny lost it, so it is enabled-coloured.
            let canvases = p.painted();
            label_colour(&canvases, "Allow") == Some(LABEL_FOCUSED)
                && label_colour(&canvases, "Deny") == Some(LABEL_LIVE)
        });
        assert!(outcome(&mut p.relay).is_none(), "Tab alone answers nothing");
        p.key(KEY_ENTER);

        let reply = p.reply("the allow never reached the requester");
        assert_eq!(reply, ("handled".to_owned(), "allowed".to_owned()));
        p.dialog_gone();
        p.assert_no_built_in_dialog();
        assert!(
            p.role().diagnostics().is_empty(),
            "no handler failed and no transaction was refused: {:?}",
            p.role().diagnostics()
        );
    }

    #[test]
    fn enter_on_the_default_button_and_escape_both_deny() {
        let mut p = Prompting::start(None);
        p.relay.set_read_timeout(Duration::from_secs(5));
        for (id, code) in [(3_u64, KEY_ENTER), (4, KEY_ESC)] {
            p.relay.send(&portal_ui(id, &consent(30_000)));
            let sent = Instant::now();
            p.dialog_shown();
            p.after_arming(sent);
            p.key(code);
            let reply = p.reply("the denial never reached the requester");
            assert_eq!(
                reply,
                ("cancelled".to_owned(), "user".to_owned()),
                "key {code} denies, and Deny is what Enter presses by default"
            );
            p.dialog_gone();
            p.assert_no_built_in_dialog();
        }
    }

    #[test]
    fn a_passphrase_typed_on_the_real_input_path_reaches_the_host_and_is_not_painted() {
        let mut p = Prompting::start(None);
        let request = p
            .services()
            .offer(
                "prompt.secret",
                "Unlock the keyring",
                "Enter your login password.",
                b"",
                Duration::from_secs(60),
            )
            .expect("the shell claimed the slot");
        let sent = Instant::now();
        p.until("the passphrase dialog was never painted", |p| {
            let shown = texts(&p.painted());
            shown.iter().any(|t| t == "Unlock the keyring") && shown.iter().any(|t| t == "Enter your login password.")
        });
        p.after_arming(sent);

        p.key(KEY_H);
        p.key(KEY_I);
        p.until("the entry never showed two characters", |p| {
            texts(&p.painted()).iter().any(|t| t == "**")
        });
        let shown = texts(&p.painted());
        assert!(
            !shown.iter().any(|t| t.contains("hi")),
            "the compositor must hold asterisks and never the passphrase: {shown:?}"
        );
        p.key(KEY_ENTER);

        let mut answer = None;
        p.until("the host never heard the passphrase", |p| {
            match p.services().next_event() {
                Ok(Event::RequestAnswered {
                    request: id,
                    answer: bytes,
                }) if id == request => {
                    answer = Some(bytes.to_vec());
                    true
                }
                _ => false,
            }
        });
        assert_eq!(
            answer.as_deref(),
            Some(&b"hi"[..]),
            "the typed text is the answer, and only the host reads it"
        );
        p.until("the dialog never closed", |p| {
            !texts(&p.painted()).iter().any(|t| t == "Unlock the keyring")
        });
        assert!(
            p.role().diagnostics().iter().all(|line| !line.contains("hi")),
            "no diagnostic carries the passphrase: {:?}",
            p.role().diagnostics()
        );
    }

    #[test]
    fn escape_in_the_passphrase_dialog_refuses_the_request() {
        let mut p = Prompting::start(None);
        let request = p
            .services()
            .offer(
                "prompt.secret",
                "Unlock the keyring",
                "Password?",
                b"",
                Duration::from_secs(60),
            )
            .expect("the shell claimed the slot");
        p.until("the passphrase dialog was never painted", |p| {
            texts(&p.painted()).iter().any(|t| t == "Unlock the keyring")
        });
        p.key(KEY_ESC);
        let mut finished = None;
        p.until("the host never heard the refusal", |p| {
            match p.services().next_event() {
                Ok(Event::RequestFinished {
                    request: id,
                    completion,
                }) if id == request => {
                    finished = Some(completion);
                    true
                }
                _ => false,
            }
        });
        assert!(
            matches!(finished, Some(RequestCompletion::Ended | RequestCompletion::Refused(_))),
            "a dismissed prompt ends the request and is never an answer: {finished:?}"
        );
    }

    #[test]
    fn a_killed_shell_releases_the_claim_and_the_next_request_uses_the_built_in_dialog() {
        let mut p = Prompting::start(None);
        // The shell goes: its connections close, and the compositor releases what it claimed.
        drop(p.role.take());
        p.relay.set_read_timeout(Duration::from_secs(5));
        p.relay.send(&portal_ui(6, &consent(30_000)));
        let mut input = p.input;
        let mut relay = p.relay;
        let mut served = None;
        for attempt in 0..200_u64 {
            input.send(&portal_key(9_000 + attempt, "escape", false));
            let _ack = input.receive();
            std::thread::sleep(Duration::from_millis(20));
            if let Some(done) = outcome(&mut relay) {
                served = Some(done);
                break;
            }
        }
        assert_eq!(
            served,
            Some(("cancelled".to_owned(), "user".to_owned())),
            "the built-in dialog served the request once the shell was gone"
        );

        // A new shell claims the slot again: the dead one's claim is not still held.
        let mut next = connect(&p.session);
        load_stock_shell(&mut next);
        let clock = Instant::now();
        let deadline = Instant::now() + super::PATIENCE;
        while !claimed(&mut next, "portal.ui") {
            next.tick(clock.elapsed().as_secs_f64()).expect("a frame runs");
            assert!(Instant::now() < deadline, "the restarted shell never claimed portal.ui");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            next.diagnostics().is_empty(),
            "a restart reports no refused claim: {:?}",
            next.diagnostics()
        );
    }

    #[test]
    fn a_handler_that_raises_hands_the_request_to_the_built_in_dialog() {
        let config = user_config(&[(
            "shell.lua",
            "seat.data.prompts = false\nslots.claim('portal.ui', function(offer) error('this dialog is broken') end)\n",
        )]);
        let mut p = Prompting::start(Some(config.path()));
        p.relay.set_read_timeout(Duration::from_secs(5));
        p.relay.send(&portal_ui(7, &consent(30_000)));
        assert_eq!(
            p.built_in_settles(),
            ("cancelled".to_owned(), "user".to_owned()),
            "the declined request is served by the built-in dialog, which Escape refuses"
        );
        assert!(
            p.role()
                .diagnostics()
                .iter()
                .any(|line| line.contains("this dialog is broken")),
            "the failure is reported: {:?}",
            p.role().diagnostics()
        );
    }

    #[test]
    fn a_handler_that_never_answers_lets_the_built_in_dialog_serve_after_its_window() {
        let config = user_config(&[(
            "shell.lua",
            "seat.data.prompts = false\nslots.claim('portal.ui', function(offer) end)\n",
        )]);
        let mut p = Prompting::start(Some(config.path()));
        p.relay.set_read_timeout(Duration::from_secs(5));
        // A 2 s request gives the claimant a 1 s window and the built-in dialog the rest.
        p.relay.send(&portal_ui(8, &consent(2_000)));
        assert_eq!(
            p.built_in_settles(),
            ("cancelled".to_owned(), "user".to_owned()),
            "past the claimant's window the built-in dialog serves the request"
        );
    }

    #[test]
    fn seat_data_prompts_false_leaves_both_dialogs_to_the_built_in() {
        let config = user_config(&[("shell.lua", "seat.data.prompts = false\n")]);
        let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
        let mut relay = end_for(&mut ends, "portal");
        let mut input = end_for(&mut ends, "input");
        let mut role = connect_session(&session, config.path());
        let clock = Instant::now();
        // Several frames run, so the first-frame claim would have happened.
        for _ in 0..40 {
            role.tick(clock.elapsed().as_secs_f64()).expect("a frame runs");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!claimed(&mut role, "portal.ui") && !claimed(&mut role, "prompt.secret"));
        relay.send(&portal_ui(9, &consent(30_000)));
        let mut served = None;
        for attempt in 0..200_u64 {
            input.send(&portal_key(9_000 + attempt, "escape", false));
            let _ack = input.receive();
            role.tick(clock.elapsed().as_secs_f64()).expect("a frame runs");
            std::thread::sleep(Duration::from_millis(20));
            if let Some(done) = outcome(&mut relay) {
                served = Some(done);
                break;
            }
        }
        assert_eq!(served, Some(("cancelled".to_owned(), "user".to_owned())));
        assert!(
            !texts(&painted_canvases(&session)).iter().any(|t| t == "Screenshot"),
            "the shell drew no dialog"
        );
    }

    /// The portal backend on a private session bus, served against the session's live compositor, and a caller on
    /// that bus: the frontend's stand-in. The portal is `serve`, the body of the shipped binary's `main`, on the
    /// owner's IPC connection, so a consent is offered to the shell exactly as the session offers it.
    struct DbusPortal {
        portal: Connection,
        caller: Connection,
        next: u32,
        _bus: Bus,
    }

    /// How one `AccessDialog` call ended: the response code and its results, or the D-Bus error.
    type AccessReply = zbus::Result<(u32, HashMap<String, zbus::OwnedVariant>)>;

    /// The app that asks and what the call tells the portal to show. The stock dialog paints the portal interface as
    /// its title ("Access") and the call's title and body together as its question.
    const ASKER: &str = "org.example.Maps";
    const ACCESS_TITLE: &str = "Share your location?";
    const ACCESS_BODY: &str = "Maps wants to know where you are.";

    /// The line the dialog paints under its title to say who asks.
    fn asked_by() -> String {
        format!("Asked by {ASKER}")
    }

    impl DbusPortal {
        fn start(session: &Session) -> DbusPortal {
            let bus = Bus::start();
            let transport = UnixTransport::connect(session.socket()).expect("the session is listening");
            let ipc = IpcClient::connect(transport, gnoblin_compositor::version()).expect("the session accepts us");
            let shell = IpcHost::pair(ipc);
            let portal = zbus::block_on(Builder::address(bus.address()).build()).expect("the portal's bus connection");
            let (ready, serving) = mpsc::channel();
            let connection = portal.clone();
            std::thread::Builder::new()
                .name("portal-dbus".to_owned())
                .spawn(move || {
                    let options = ServeOptions {
                        pictures: None,
                        secret_served: false,
                        consent_lifetime: 60_000,
                    };
                    let ended = zbus::block_on(serve(connection, Some(shell), options, &mut || {
                        let _ = ready.send(());
                    }));
                    if let Err(error) = ended {
                        eprintln!("the portal ended with an error: {error}");
                    }
                })
                .expect("the portal thread");
            serving
                .recv_timeout(Duration::from_secs(10))
                .expect("the portal serves");
            let caller = zbus::block_on(Builder::address(bus.address()).build()).expect("a caller on the private bus");
            DbusPortal {
                portal,
                caller,
                next: 0,
                _bus: bus,
            }
        }

        /// Calls `AccessDialog` as the frontend does, on a thread of its own so the test can run the shell while the
        /// call waits. Returns the request handle, which `Request.Close` cancels, and the call.
        fn ask(&mut self) -> (String, std::thread::JoinHandle<AccessReply>) {
            let sender = self
                .caller
                .unique_name()
                .expect("a connected caller has a name")
                .as_str()
                .trim_start_matches(':')
                .replace('.', "_");
            let handle = format!("{PORTAL_PATH}/request/{sender}/t{}", self.next);
            self.next += 1;
            let caller = self.caller.clone();
            let path = handle.clone();
            let call = std::thread::spawn(move || {
                zbus::block_on(async move {
                    let proxy = Proxy::new(&caller, BUS_NAME, PORTAL_PATH, "org.freedesktop.impl.portal.Access")
                        .await
                        .expect("a proxy for the backend");
                    let options: HashMap<&str, Variant<'_>> = HashMap::new();
                    let handle = ObjectPath::try_from(path.as_str()).expect("a request path");
                    let reply = proxy
                        .call_method(
                            "AccessDialog",
                            &(handle, ASKER, "", ACCESS_TITLE, "", ACCESS_BODY, options),
                        )
                        .await?;
                    Ok(reply.body().deserialize().expect("a response and results"))
                })
            });
            (handle, call)
        }

        /// `Request.Close`, which is how the frontend cancels a call that is waiting for the user.
        fn close_request(&self, handle: &str) {
            zbus::block_on(async {
                Proxy::new(
                    &self.caller,
                    BUS_NAME,
                    handle.to_owned(),
                    "org.freedesktop.impl.portal.Request",
                )
                .await
                .expect("a proxy for the request")
                .call_method("Close", &())
                .await
                .expect("Close answers")
            });
        }
    }

    impl Drop for DbusPortal {
        fn drop(&mut self) {
            let _ = zbus::block_on(self.portal.clone().close());
        }
    }

    impl Prompting {
        /// Waits for the access dialog the stock shell draws for the call: its title, who asks, the question and
        /// both buttons, all painted.
        fn access_dialog_shown(&mut self) {
            self.until("the access dialog was never painted", |p| {
                let shown = texts(&p.painted());
                shown.iter().any(|t| t == "Access")
                    && shown.iter().any(|t| t == &asked_by())
                    && shown.iter().any(|t| t == &format!("{ACCESS_TITLE} {ACCESS_BODY}"))
                    && shown.iter().any(|t| t == "Deny")
                    && shown.iter().any(|t| t == "Allow")
            });
        }

        fn access_dialog_gone(&mut self) {
            self.until("the access dialog never closed", |p| {
                !texts(&p.painted()).iter().any(|t| t == &asked_by())
            });
        }

        /// Runs shell frames until the D-Bus call has answered, then returns its answer.
        fn answered(&mut self, call: std::thread::JoinHandle<AccessReply>) -> AccessReply {
            self.until("the D-Bus call never answered", |_| call.is_finished());
            call.join().expect("the caller thread")
        }
    }

    #[test]
    fn a_dbus_access_dialog_is_drawn_by_the_stock_shell_and_escape_answers_the_caller_cancelled() {
        let mut p = Prompting::start(None);
        let mut dbus = DbusPortal::start(&p.session);
        let (_, call) = dbus.ask();
        p.access_dialog_shown();
        assert!(
            !call.is_finished(),
            "the call waits for the user while the dialog is on screen"
        );

        p.key(KEY_ESC);
        let (response, results) = p.answered(call).expect("AccessDialog answers");
        assert_eq!(response, 1, "Escape is the user's own refusal");
        assert!(results.is_empty(), "an access dialog returns no results");
        p.access_dialog_gone();
        assert!(
            p.role().diagnostics().is_empty(),
            "no handler failed and no transaction was refused: {:?}",
            p.role().diagnostics()
        );
    }

    #[test]
    fn tab_then_enter_on_a_dbus_access_dialog_answers_the_caller_allowed() {
        let mut p = Prompting::start(None);
        let mut dbus = DbusPortal::start(&p.session);
        let (_, call) = dbus.ask();
        let sent = Instant::now();
        p.access_dialog_shown();
        p.after_arming(sent);
        p.until("Allow never became live", |p| {
            label_colour(&p.painted(), "Allow") == Some(LABEL_LIVE)
        });
        p.key(KEY_TAB);
        p.until("Tab never moved the focus to Allow", |p| {
            label_colour(&p.painted(), "Allow") == Some(LABEL_FOCUSED)
        });
        assert!(!call.is_finished(), "Tab alone answers nothing");
        p.key(KEY_ENTER);

        let (response, results) = p.answered(call).expect("AccessDialog answers");
        assert_eq!(response, 0, "Allow is the caller's success");
        assert!(results.is_empty(), "an access dialog returns no results");
        p.access_dialog_gone();
    }

    #[test]
    fn a_cancelled_dbus_call_closes_the_dialog_and_the_next_call_gets_its_own() {
        let mut p = Prompting::start(None);
        let mut dbus = DbusPortal::start(&p.session);
        let (handle, call) = dbus.ask();
        p.access_dialog_shown();

        // The frontend cancels the request: the caller is answered, and the compositor is told to withdraw the dialog,
        // so the shell's offer ends and its handler closes the dialog nobody is waiting on.
        dbus.close_request(&handle);
        let (response, _) = p.answered(call).expect("a cancelled call is still answered");
        assert_ne!(response, 0, "a cancelled call never reports success");
        p.access_dialog_gone();

        // The shell is free again: a second call is drawn and answered like the first.
        let (_, next) = dbus.ask();
        p.access_dialog_shown();
        p.key(KEY_ESC);
        let (response, _) = p.answered(next).expect("AccessDialog answers");
        assert_eq!(response, 1);
        p.access_dialog_gone();
    }

    #[test]
    fn a_caller_that_leaves_the_bus_closes_its_dialog() {
        let mut p = Prompting::start(None);
        let mut dbus = DbusPortal::start(&p.session);
        let (_, call) = dbus.ask();
        p.access_dialog_shown();

        // A crashed app: its connection is gone and nobody is left to read an answer.
        let _ = zbus::block_on(dbus.caller.clone().close());
        p.access_dialog_gone();
        let _ = call.join();
    }

    /// The real services role, adopting the session's services end, with the Gnoblin keyring served on the private
    /// bus: a locked store whose unlock is a `prompt.secret` request to the stock shell. The role is the shipped one
    /// and the agent is the shipped `KeyringAgent`; only the bus it serves on is the test's.
    struct KeyringRole {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<Result<(), gnoblin_services::RoleError>>>,
        _directory: tempfile::TempDir,
    }

    impl KeyringRole {
        fn start(end: UnixStream, connection: Connection) -> KeyringRole {
            let directory = tempfile::tempdir().expect("a directory for the store");
            let agent = KeyringAgent::new(
                directory.path().join("keyring.db"),
                directory.path().join("handoff.sock"),
            )
            .on_connection(connection);
            let number = end.into_raw_fd();
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let role = Role {
                env: Box::new(move |name| (name == "GNOBLIN_IPC_FD").then(|| number.to_string())),
                deadline: Duration::from_secs(120),
                buses: Buses::None,
            };
            let thread = std::thread::Builder::new()
                .name("services-role".to_owned())
                .spawn(move || role.run(move || flag.load(Ordering::SeqCst), vec![Box::new(agent)]))
                .expect("the services role thread");
            KeyringRole {
                stop,
                thread: Some(thread),
                _directory: directory,
            }
        }
    }

    impl KeyringRole {
        /// Fails the test with the role's own error if the role has ended: a role that is gone serves nothing, and
        /// waiting out the patience for what it would have served hides why.
        #[track_caller]
        fn assert_running(&mut self) {
            if self.thread.as_ref().is_some_and(std::thread::JoinHandle::is_finished) {
                let ended = self
                    .thread
                    .take()
                    .expect("the thread")
                    .join()
                    .expect("the role did not panic");
                panic!("the services role ended: {ended:?}");
            }
        }
    }

    impl Drop for KeyringRole {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    const SECRETS: &str = "org.freedesktop.secrets";
    const SECRETS_PATH: &str = "/org/freedesktop/secrets";
    const DEFAULT_COLLECTION: &str = "/org/freedesktop/secrets/collection/default";

    /// What a Secret Service client saw while it unlocked the default collection: the first `Unlock`'s unlocked list
    /// and prompt, whether the prompt was dismissed, and the `Unlock` a moment after it.
    #[derive(Debug)]
    struct UnlockSeen {
        first_unlocked: Vec<String>,
        prompt: String,
        dismissed: bool,
        then_unlocked: Vec<String>,
        then_prompt: String,
    }

    /// A Secret Service client: `Unlock` the default collection, run the prompt it gets and wait for `Completed`.
    fn unlock_default(client: Connection) -> std::thread::JoinHandle<zbus::Result<UnlockSeen>> {
        std::thread::spawn(move || {
            zbus::block_on(async move {
                use futures_lite::StreamExt as _;
                use zbus::OwnedObjectPath;
                let service = Proxy::new(&client, SECRETS, SECRETS_PATH, "org.freedesktop.Secret.Service").await?;
                let collection = ObjectPath::try_from(DEFAULT_COLLECTION).expect("a collection path");
                let reply = service.call_method("Unlock", &(vec![collection.clone()],)).await?;
                let (first_unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = reply.body().deserialize()?;
                let prompter = Proxy::new(
                    &client,
                    SECRETS,
                    prompt.as_str().to_owned(),
                    "org.freedesktop.Secret.Prompt",
                )
                .await?;
                let mut completed = prompter.receive_signal("Completed").await?;
                prompter.call_method("Prompt", &("",)).await?;
                let signal = completed.next().await.expect("the prompt completes");
                let (dismissed, _): (bool, zbus::OwnedVariant) = signal.body().deserialize()?;
                let reply = service.call_method("Unlock", &(vec![collection],)).await?;
                let (then_unlocked, then_prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
                    reply.body().deserialize()?;
                let paths = |paths: Vec<OwnedObjectPath>| paths.iter().map(|path| path.to_string()).collect();
                Ok(UnlockSeen {
                    first_unlocked: paths(first_unlocked),
                    prompt: prompt.to_string(),
                    dismissed,
                    then_unlocked: paths(then_unlocked),
                    then_prompt: then_prompt.to_string(),
                })
            })
        })
    }

    #[test]
    fn a_secret_service_unlock_over_dbus_asks_the_stock_shell_and_the_typed_passphrase_unlocks_the_keyring() {
        let mut p = Prompting::start_holding(None, false);
        let bus = Bus::start();
        let service = zbus::block_on(Builder::address(bus.address()).build()).expect("the keyring's bus connection");
        let end = p.services_end.take().expect("the services role's end");
        let mut role = KeyringRole::start(end, service);
        let client = zbus::block_on(Builder::address(bus.address()).build()).expect("a client on the private bus");

        // The role decides `keyring.enabled` first, from the compositor's accepted configuration or, after its
        // window, the schema default, and only then takes the name.
        let watcher = client.clone();
        p.until("the keyring never took org.freedesktop.secrets", |_| {
            role.assert_running();
            zbus::block_on(async {
                let bus = zbus::fdo::DBusProxy::new(&watcher).await.expect("the bus proxy");
                let name = zbus::names::BusName::try_from(SECRETS).expect("a bus name");
                bus.name_has_owner(name).await.unwrap_or(false)
            })
        });

        let unlock = unlock_default(client);
        let sent = Instant::now();
        p.until("the keyring's passphrase dialog was never painted", |p| {
            let shown = texts(&p.painted());
            shown.iter().any(|t| t == "Unlock the keyring")
                && shown
                    .iter()
                    .any(|t| t == "Enter your login password to unlock the keyring.")
        });
        p.after_arming(sent);
        p.key(KEY_H);
        p.key(KEY_I);
        p.until("the entry never showed two characters", |p| {
            texts(&p.painted()).iter().any(|t| t == "**")
        });
        assert!(
            !unlock.is_finished(),
            "the client waits for the user while the dialog is on screen"
        );
        p.key(KEY_ENTER);

        p.until("the Secret Service client never saw the prompt complete", |_| {
            unlock.is_finished()
        });
        let seen = unlock
            .join()
            .expect("the client thread")
            .expect("the unlock calls answer");
        assert!(
            seen.first_unlocked.is_empty() && seen.prompt != "/",
            "a locked keyring answers Unlock with a prompt: {seen:?}"
        );
        assert!(!seen.dismissed, "the typed passphrase completed the prompt: {seen:?}");
        assert_eq!(
            (seen.then_unlocked.as_slice(), seen.then_prompt.as_str()),
            ([DEFAULT_COLLECTION.to_owned()].as_slice(), "/"),
            "the collection is unlocked once the passphrase was accepted: {seen:?}"
        );
        p.until("the dialog never closed", |p| {
            !texts(&p.painted()).iter().any(|t| t == "Unlock the keyring")
        });
    }

    const SAVER: &str = "org.freedesktop.ScreenSaver";
    const SAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
    const KEY_Y: u32 = 21;

    /// A probe of the shell VM's state as text.
    fn probe(p: &mut Prompting, code: &str) -> String {
        p.role()
            .run("lock probe", code)
            .expect("the probe runs")
            .expect("the probe returns text")
    }

    /// `org.freedesktop.ScreenSaver.GetActive` as a caller on the private bus sees it.
    fn saver_active(caller: &Connection) -> bool {
        zbus::block_on(async {
            let proxy = Proxy::new(caller, SAVER, SAVER_PATH, SAVER)
                .await
                .expect("the saver proxy");
            proxy
                .call_method("GetActive", &())
                .await
                .expect("GetActive answers")
                .body()
                .deserialize()
                .expect("a bool")
        })
    }

    /// The real ScreenSaver service over the real owner connection to the compositor, as the services role runs it:
    /// the lock node is attached, a thread pumps the connection's events into `ActiveChanged`, and the service is
    /// served on the private bus.
    struct Saver {
        stop: Arc<AtomicBool>,
        pump: Option<std::thread::JoinHandle<()>>,
    }

    impl Saver {
        fn start(socket: &std::path::Path, server: &Connection) -> Saver {
            let client = IpcClient::connect(
                UnixTransport::connect(socket).expect("the compositor's socket"),
                gnoblin_compositor::version(),
            )
            .expect("the owner's connection");
            let host = gnoblin_service_core::Host::new(client, Duration::from_secs(30));
            host.watch_lock().expect("the session has a lock node");
            host.set_read_timeout(Some(Duration::from_millis(20)));
            let service = gnoblin_service_screensaver::ScreenSaver::new(Box::new(host.clone()));
            service
                .register(server)
                .expect("the service registers on the private bus");
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let connection = server.clone();
            let pump = std::thread::Builder::new()
                .name("saver-pump".to_owned())
                .spawn(move || {
                    while !flag.load(Ordering::SeqCst) {
                        match host.next_event() {
                            Ok(Event::Lock(state)) => {
                                if let Some(active) = service.observe(state) {
                                    gnoblin_service_screensaver::ScreenSaver::publish(&connection, active);
                                }
                            }
                            Ok(_) | Err(gnoblin_client::ClientError::Idle) => {}
                            Err(_) => break,
                        }
                    }
                })
                .expect("the pump thread");
            Saver { stop, pump: Some(pump) }
        }
    }

    impl Drop for Saver {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(pump) = self.pump.take() {
                let _ = pump.join();
            }
        }
    }

    /// The whole lock path on the stock shell, in a real session. `org.freedesktop.ScreenSaver.Lock` takes the lock
    /// through the real service; the compositor covers the session and tells the stock shell, which engages its lock
    /// screen; a wrong password typed on the session's real input path (the EI keyboard) is denied and the session
    /// stays locked; the right one unlocks it, through the compositor's PAM-free scripted check.
    #[test]
    fn the_screensaver_locks_the_session_and_a_typed_password_unlocks_the_stock_lock_screen() {
        const USER: &str = "kieran";
        let mut p = Prompting::start_configured(None, true, |config| {
            config.wayland.lock_check = Some(gnoblin_wayland::LockCheck {
                authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, "hi")),
                user: Some(USER.to_owned()),
            });
        });
        let bus = Bus::start();
        let server = zbus::block_on(Builder::address(bus.address()).build()).expect("the service's bus connection");
        let _saver = Saver::start(p.session.socket(), &server);
        let caller = zbus::block_on(Builder::address(bus.address()).build()).expect("a caller on the private bus");
        assert!(!saver_active(&caller), "a new session is not locked");
        assert_eq!(probe(&mut p, "return lock.state"), "not-locked");

        // The call blocks until the cover is proven, so it runs on its own thread while the shell takes its frames.
        let asker = caller.clone();
        let call = std::thread::spawn(move || {
            zbus::block_on(async {
                let proxy = Proxy::new(&asker, SAVER, SAVER_PATH, SAVER).await?;
                proxy.call_method("Lock", &()).await.map(|_| ())
            })
        });
        p.until("the shell's lock screen never came up", |p| {
            probe(p, "return tostring(lock.engaged) .. ' ' .. tostring(#root:all(Entry))") == "true 1"
        });
        p.until("the Lock call never returned", |_| call.is_finished());
        call.join()
            .expect("the caller thread")
            .expect("Lock answers once the cover is confirmed");
        assert_eq!(probe(&mut p, "return lock.state"), "locked");
        assert!(saver_active(&caller), "GetActive is true while locked");

        // A wrong password through the real input path: denied, and the session stays locked.
        p.key(KEY_H);
        p.key(KEY_Y);
        p.until("the entry never showed the two characters", |p| {
            texts(&p.painted()).iter().any(|t| t == "**")
        });
        p.key(KEY_ENTER);
        p.until("the wrong password was never answered", |p| {
            texts(&p.painted()).iter().any(|t| t == "Wrong password.")
        });
        assert_eq!(
            probe(&mut p, "return lock.state"),
            "locked",
            "a denied password keeps the lock"
        );
        assert!(saver_active(&caller), "GetActive stays true after a denied password");

        // The right one unlocks.
        p.key(KEY_H);
        p.key(KEY_I);
        p.key(KEY_ENTER);
        p.until("the right password never unlocked the session", |p| {
            probe(p, "return lock.state") == "not-locked"
        });
        p.until("the service never announced the unlock", |_| !saver_active(&caller));
        assert!(
            p.role().diagnostics().is_empty(),
            "no handler failed and no transaction was refused: {:?}",
            p.role().diagnostics()
        );
    }

    /// The failure mode this guards: the exception that lets a typed password reach the shell's lock screen under the
    /// compositor's own lock also lets keys reach it under an `ext-session-lock-v1` locker's, where the locker owns every
    /// key. A key typed under an external lock never reaches the shell, even with its lock screen engaged and focused.
    #[test]
    fn keys_typed_under_an_external_locker_never_reach_the_shells_lock_screen() {
        let mut p = Prompting::start(None);
        let mut locker = Locker::connect(p.session.wayland());
        locker.lock();
        p.until("the shell's lock screen never came up", |p| {
            probe(p, "return tostring(lock.engaged) .. ' ' .. tostring(#root:all(Entry))") == "true 1"
        });
        p.key(KEY_H);
        p.key(KEY_I);
        p.run_for(Duration::from_millis(500));
        assert_eq!(
            probe(&mut p, "return root:all(Entry)[1].text"),
            "",
            "the shell's entry received no key under an external lock"
        );
        p.key(KEY_ENTER);
        p.run_for(Duration::from_millis(300));
        assert!(
            !texts(&p.painted()).iter().any(|t| t == "Wrong password."),
            "nothing was submitted"
        );
    }

    /// A session locked before any shell attaches: the request went nowhere, so the shell that comes up later must
    /// show the lock screen from its snapshot alone. Without that the session stays locked with no screen to unlock it.
    #[test]
    fn a_shell_that_attaches_to_a_locked_session_shows_the_lock_screen() {
        let session = Session::start();
        let bus = Bus::start();
        let server = zbus::block_on(Builder::address(bus.address()).build()).expect("the service's bus connection");
        let _saver = Saver::start(session.socket(), &server);
        let caller = zbus::block_on(Builder::address(bus.address()).build()).expect("a caller on the private bus");

        // No shell is attached, so the cover is the compositor's own and the call returns once the cover is proven.
        let asker = caller.clone();
        let call = std::thread::spawn(move || {
            zbus::block_on(async {
                let proxy = Proxy::new(&asker, SAVER, SAVER_PATH, SAVER).await?;
                proxy.call_method("Lock", &()).await.map(|_| ())
            })
        });
        let deadline = Instant::now() + super::PATIENCE;
        while !call.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the Lock call never returned with no shell attached"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        call.join()
            .expect("the caller thread")
            .expect("Lock answers once the cover is confirmed");
        assert!(saver_active(&caller), "GetActive is true while locked");

        // The shell attaches to the session that is already locked, and must draw its lock screen.
        let mut role = connect(&session);
        load_stock_shell(&mut role);
        super::tick_until(&mut role, "the attached shell never showed the lock screen", |role| {
            role.run(
                "lock probe",
                "return tostring(lock.engaged) .. ' ' .. tostring(#root:all(Entry))",
            )
            .expect("the probe runs")
            .as_deref()
                == Some("true 1")
        });
        assert!(
            saver_active(&caller),
            "the session stays locked while its shell attaches"
        );
        assert!(
            role.diagnostics().is_empty(),
            "no handler failed: {:?}",
            role.diagnostics()
        );
    }
}

// ---- The window menu and the not-responding alert (data/lua/gnoblin/shell/menus.lua, alerts.lua) ---------------------
//
// The compositor is a running session, the shell the stock shell in a real VM over the owner socket, the pointer and
// keys come from an EI client on the session's own input path, and a client is a real Wayland client. What the menu
// and the alert paint is read back from the compositor's own tree.

mod window_menu {
    use std::process::{Child, Command as Process, Stdio};

    use super::support::ei::{EiClient, frame, keyboard};
    use super::{
        Client, Duration, Instant, PATIENCE, PaintedCanvas, Session, ShellRole, Window, connect, connect_session,
        has_text, load_stock_shell, mirror_counts, observe, painted_canvases, user_config, wait_for_sized_workspace,
    };

    const BTN_LEFT: u32 = 272;
    const BTN_RIGHT: u32 = 273;
    const KEY_ESC: u32 = 1;
    const KEY_ENTER: u32 = 28;
    const KEY_TAB: u32 = 15;
    const KEY_DOWN: u32 = 108;

    struct Rig {
        session: Session,
        role: ShellRole,
        remote: EiClient,
        client: Client,
        clock: Instant,
    }

    impl Rig {
        /// The stock shell loaded and a client with one window titled `title`, which the pointer can reach.
        fn start(session: Session, role: Option<ShellRole>, title: &str) -> (Rig, Window) {
            let mut role = role.unwrap_or_else(|| {
                let mut role = connect(&session);
                load_stock_shell(&mut role);
                role
            });
            super::tick_until(&mut role, "the mirror never held one seat and one screen", |role| {
                mirror_counts(role) == (1, 1)
            });
            wait_for_sized_workspace(&mut role);
            let remote = keyboard(&session);
            let mut client = Client::connect(session.wayland());
            client.pointers_on_every_seat();
            let window = client.map_window("org.example.menu", title);
            let mut rig = Rig {
                session,
                role,
                remote,
                client,
                clock: Instant::now(),
            };
            rig.until("the titlebar was never built", |r| {
                r.probe("local w = root:all('Window')[1] return tostring(w ~= nil and #w:all(Titlebar) == 1)") == "true"
            });
            (rig, window)
        }

        fn probe(&mut self, code: &str) -> String {
            self.role
                .run("probe", code)
                .expect("the probe runs")
                .unwrap_or_default()
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Rig) -> bool) {
            let deadline = Instant::now() + PATIENCE;
            loop {
                self.role
                    .tick(self.clock.elapsed().as_secs_f64())
                    .expect("a frame runs");
                self.client.roundtrip();
                self.remote.pump();
                if done(self) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{what}: diagnostics {:?}",
                    self.role.diagnostics()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn run_for(&mut self, span: Duration) {
            let end = Instant::now() + span;
            self.until("never", |_| Instant::now() >= end);
        }

        fn menu_open(&mut self) -> bool {
            self.probe("return tostring(require('gnoblin.shell.menus').current() ~= nil)") == "true"
        }

        fn shows(&self, wanted: &str) -> bool {
            painted_canvases(&self.session)
                .iter()
                .any(|c: &PaintedCanvas| has_text(c, wanted))
        }

        fn maximised(&mut self) -> bool {
            self.probe("return tostring(root:all('Window')[1].maximised)") == "true"
        }

        fn key(&mut self, code: u32) {
            self.remote.typing(code, true);
            self.remote.typing(code, false);
            self.run_for(Duration::from_millis(60));
        }

        /// Moves the pointer to `at` and presses and releases `button`, on the session's input tap: the events take the
        /// seat path a device's do (the lock gate, the grabs, the shell's hit test). The keys come from EI.
        fn click(&mut self, button: u32, at: (f32, f32)) {
            use gnoblin_backend::InputEvent;
            use gnoblin_types::{DeviceId, OutputId, Point};
            let tap = self.session.input().clone();
            assert!(tap.inject(InputEvent::PointerMotion {
                device: DeviceId::VIRTUAL,
                output: OutputId::new(0),
                position: Point::new(at.0 as i32, at.1 as i32),
            }));
            self.run_for(Duration::from_millis(80));
            for pressed in [true, false] {
                assert!(tap.inject(InputEvent::PointerButton {
                    device: DeviceId::VIRTUAL,
                    button,
                    pressed,
                }));
            }
            self.run_for(Duration::from_millis(80));
        }

        /// A point on the window's titlebar, in the layout's coordinates.
        fn titlebar_point(&mut self, title: &str) -> (f32, f32) {
            let (x, y, _, _) = frame(&self.session, title);
            let origin = self.probe("local t = root:all('Window')[1]:all(Titlebar)[1] return t.x .. ',' .. t.y");
            let (bx, by) = origin.split_once(',').expect("two numbers");
            (
                x + bx.parse::<f32>().expect("a number") + 40.0,
                y + by.parse::<f32>().expect("a number") + 14.0,
            )
        }

        /// Right-clicks the titlebar and waits for the menu to be painted.
        fn open_menu(&mut self, title: &str) {
            let at = self.titlebar_point(title);
            self.click(BTN_RIGHT, at);
            self.until("the window menu was never painted", |r| {
                r.menu_open() && r.shows("Maximise") && r.shows("Close")
            });
        }
    }

    #[test]
    fn a_titlebar_right_click_opens_the_menu_at_the_pointer_and_down_then_enter_maximises_the_window() {
        let (mut rig, _window) = Rig::start(Session::start(), None, "Menu me");
        let at = rig.titlebar_point("Menu me");
        rig.open_menu("Menu me");
        let corner = rig.probe("local p = require('gnoblin.shell.menus').current().panel return p.x .. ',' .. p.y");
        let (x, y) = corner.split_once(',').expect("two numbers");
        let (x, y) = (x.parse::<f32>().unwrap(), y.parse::<f32>().unwrap());
        assert!(
            (x - at.0).abs() <= 2.0 && (y - at.1).abs() <= 2.0 + 28.0,
            "the menu's corner {x},{y} is at the pointer {at:?}"
        );
        assert!(!rig.maximised());
        rig.key(KEY_DOWN);
        rig.key(KEY_ENTER);
        rig.until("the window never maximised or the menu stayed", |r| {
            r.maximised() && !r.menu_open()
        });
        rig.until("the menu's pixels never went", |r| !r.shows("Maximise"));
        assert!(rig.role.diagnostics().is_empty(), "{:?}", rig.role.diagnostics());
    }

    #[test]
    fn escape_and_a_click_away_close_the_menu_and_run_nothing() {
        let (mut rig, _window) = Rig::start(Session::start(), None, "Dismiss me");
        rig.open_menu("Dismiss me");
        rig.key(KEY_ESC);
        rig.until("Escape never closed the menu", |r| {
            !r.menu_open() && !r.shows("Maximise")
        });
        rig.open_menu("Dismiss me");
        let (w, h) = super::screen_size(&mut rig.role);
        let far = (w as f32 - 20.0, h as f32 - 20.0);
        rig.click(BTN_LEFT, far);
        rig.until("a click away never closed the menu", |r| {
            !r.menu_open() && !r.shows("Maximise")
        });
        assert!(!rig.maximised(), "dismissing ran nothing");
    }

    #[test]
    fn a_clients_menu_request_opens_one_menu_even_over_the_titlebars_and_closing_the_window_closes_it() {
        let (mut rig, window) = Rig::start(Session::start(), None, "Client menu");
        let (x, y, _, _) = frame(&rig.session, "Client menu");
        rig.click(BTN_LEFT, (x + 60.0, y + 100.0));
        rig.until("the client never got a button serial", |r| r.client.clicked());
        rig.open_menu("Client menu");
        rig.client.show_window_menu(&window, 20, 20);
        rig.run_for(Duration::from_millis(300));
        assert_eq!(
            rig.probe("return tostring(#desk():all(WindowMenu))"),
            "1",
            "two menu requests never stack"
        );
        rig.client.destroy(window);
        rig.until("the menu stayed after its window closed", |r| {
            !r.menu_open() && !r.shows("Maximise")
        });
    }

    #[test]
    fn a_menu_for_a_secure_window_is_secure_in_the_tree() {
        let (mut rig, _window) = Rig::start(Session::start(), None, "Secret");
        rig.probe("root:all('Window')[1].secure = true return 'set'");
        rig.open_menu("Secret");
        let (_wire, _, nodes) = observe(&rig.session);
        let card = nodes
            .iter()
            .find(|node| {
                node.props
                    .iter()
                    .any(|prop| prop.key == "name" && prop.value_text == "window-menu")
            })
            .expect("the menu's card is in the tree");
        assert!(
            card.props.iter().any(|prop| prop.key == "secure" && prop.value_bool),
            "the card is secure: {:?}",
            card.props
        );
    }

    #[test]
    fn a_shell_that_loaded_no_menus_answers_a_menu_request_with_nothing() {
        let session = Session::start();
        let role = connect(&session); // node.lua only: no stock shell, so no menus.lua
        let mut role = role;
        super::tick_until(&mut role, "the mirror never held a screen", |role| {
            mirror_counts(role) == (1, 1)
        });
        let remote = keyboard(&session);
        let mut client = Client::connect(session.wayland());
        client.pointers_on_every_seat();
        let window = client.map_window("org.example.menu", "Bare");
        let mut rig = Rig {
            session,
            role,
            remote,
            client,
            clock: Instant::now(),
        };
        let (x, y, _, _) = frame(&rig.session, "Bare");
        rig.click(BTN_LEFT, (x + 60.0, y + 60.0));
        rig.until("the client never got a button serial", |r| r.client.clicked());
        rig.client.show_window_menu(&window, 10, 10);
        rig.run_for(Duration::from_millis(400));
        assert!(!rig.shows("Maximise"), "nothing draws a menu");
        assert!(rig.role.diagnostics().is_empty(), "{:?}", rig.role.diagnostics());
        assert!(
            rig.client.keyboard_focus().len() <= 2,
            "the client's keyboard focus did not move"
        );
    }

    #[test]
    fn a_users_shell_file_replaces_the_menus_entries() {
        let config = user_config(&[(
            "shell.lua",
            "function Window:menu_items() return { { label = 'Only one', run = function(w) w:close() end } } end",
        )]);
        let session = Session::start();
        let role = connect_session(&session, config.path());
        let (mut rig, _window) = Rig::start(session, Some(role), "Mine");
        let at = rig.titlebar_point("Mine");
        rig.click(BTN_RIGHT, at);
        rig.until("the user's menu was never painted", |r| r.shows("Only one"));
        assert!(!rig.shows("Maximise"), "the stock entries are replaced");
    }

    fn sleeper() -> Child {
        Process::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .spawn()
            .expect("sleep is on PATH")
    }

    #[test]
    fn a_window_that_stops_answering_gets_an_alert_and_wait_keeps_it_while_force_quit_kills_its_process() {
        use std::os::unix::process::ExitStatusExt as _;

        let mut victim = sleeper();
        let identity = gnoblin_wayland::ProcessIdentity::of(victim.id()).expect("the victim has a stat line");
        let (session, _ends) = Session::start_privileged_configured(&[], |config| {
            config.wayland.check_alive_timeout = Duration::from_millis(300);
            config.wayland.window_process_override = Some(gnoblin_wayland::WindowProcess::Connected(identity));
        });
        let (mut rig, _window) = Rig::start(session, None, "Stuck");
        rig.client.mute_pings(true);
        rig.until("the alert never opened", |r| r.shows("Stuck is not responding"));
        rig.key(KEY_ESC);
        rig.until("Wait never closed the alert", |r| !r.shows("Stuck is not responding"));
        assert!(
            victim.try_wait().expect("the victim is waitable").is_none(),
            "Wait kills nothing"
        );

        // It answers again, then stops again: the alert comes back, and this time Force quit is armed and pressed.
        rig.client.mute_pings(false);
        rig.run_for(Duration::from_millis(1500));
        rig.client.mute_pings(true);
        rig.until("the alert never came back", |r| r.shows("Stuck is not responding"));
        rig.run_for(Duration::from_millis(700));
        rig.key(KEY_TAB);
        rig.key(KEY_ENTER);
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            if let Some(status) = victim.try_wait().expect("the victim is waitable") {
                break status;
            }
            rig.run_for(Duration::from_millis(20));
            assert!(
                Instant::now() < deadline,
                "Force quit never killed the window's process"
            );
        };
        assert_eq!(status.signal(), Some(9), "the process was killed with SIGKILL");
    }
}
