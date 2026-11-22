//! Reloading the shell of a real session (examples 10 and 18).
//!
//! The shell is example 10, built the way a developer builds it and swapped in under a running `gnoblin --watch`. Windows are
//! real Wayland windows of a client that stays connected for the whole test, and what each assertion reads is what the compositor's
//! tree holds (through observer connections), so the claims are the session's: a client's connection and surface survive, the
//! new behaviour is in force, nothing is made twice, and a replacement that cannot take over leaves the running shell serving.
//!
//! Three doors reach the reload, and each is used: the file watcher (`--watch`, a rebuilt library), the socket
//! (`gnoblinctl shell reload`, and `gnoblinctl shell watch`), and the shell itself (`request_reload`, the chord of example 10).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod support;
use support::native_shell::*;
use support::{Mapper, snapshot};

/// The compositor's socket as `gnoblinctl` reaches it.
fn ctl(session: &Gnoblin, arguments: &[&str]) -> (u8, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = gnoblinctl::run(
        arguments.iter().map(OsString::from),
        gnoblinctl::SocketConnection::at(session.ipc()),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

/// Replaces `live` with `from` the way a linker's last step does: a whole file appears under the name at once.
fn swap_in(from: &Path, live: &Path) {
    let part = live.with_extension("part");
    fs::copy(from, &part).expect("the replacement is written");
    fs::rename(&part, live).expect("the replacement takes the name");
}

/// Writes `bytes` as the library, whole.
fn swap_in_bytes(bytes: &[u8], live: &Path) {
    let part = live.with_extension("part");
    fs::write(&part, bytes).expect("the replacement is written");
    fs::rename(&part, live).expect("the replacement takes the name");
}

/// The nodes of `class` named `name` in the tree: (id, x, y, z).
fn named(session: &Gnoblin, class: &str, name: &str) -> Vec<(u64, f64, f64, f64)> {
    let value = |node: &gnoblin_proto::tree::snapshot_node::Properties, key: &str| {
        node.props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(0.0, |prop| prop.value_double)
    };
    snapshot(&session.ipc())
        .iter()
        .filter(|node| {
            node.class == class
                && node
                    .props
                    .iter()
                    .any(|prop| prop.key == "name" && prop.value_text == name)
        })
        .map(|node| (node.id, value(node, "x"), value(node, "y"), value(node, "z")))
        .collect()
}

/// How many log lines satisfy `matches`.
fn count(session: &Gnoblin, matches: impl Fn(&str) -> bool) -> usize {
    session.log().iter().filter(|line| matches(&line.text)).count()
}

/// Waits for `n` log lines that satisfy `matches`.
#[track_caller]
fn wait_count(session: &mut Gnoblin, what: &str, n: usize, matches: impl Fn(&str) -> bool) {
    session.wait_for(what, |session| count(session, &matches) >= n);
}

/// Where the nth window example 10 places is, for a cascade of `step` pixels.
fn spot(n: u32, step: f64) -> f64 {
    60.0 + step * f64::from(n)
}

fn at(window: &support::Window, n: u32, step: f64) -> bool {
    (window.x - spot(n, step)).abs() < 0.5 && (window.y - spot(n, step)).abs() < 0.5
}

/// A session of example 10 as it is, watched, with a client connected and two windows placed by it.
struct Running {
    session: Gnoblin,
    live: PathBuf,
    client: Mapper,
    ids: Vec<u64>,
}

fn running(arguments: &[&str]) -> Running {
    let hot = hot();
    let home = Home::new();
    let live = home.copy(&hot.v1, "live/libhot_reload.so");
    let mut launch = Launch::compositor(arguments);
    launch.args.push(live.clone().into_os_string());
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    session.wait_log("generation 1 serves", |line| {
        line.contains("serves in process (generation 1)")
    });
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("first", "First");
    let first = wait_for_window(
        &mut session,
        "generation 1 places the first window",
        "first",
        |window| at(window, 0, 40.0),
    );
    client.map_window("second", "Second");
    let second = wait_for_window(&mut session, "and the second a step down", "second", |window| {
        at(window, 1, 40.0)
    });
    Running {
        session,
        live,
        client,
        ids: vec![first.id, second.id],
    }
}

#[test]
fn example_10_a_rebuilt_library_reloads_with_the_clients_connected_and_nothing_made_twice() {
    let hot = hot();
    let mut run = running(&["--watch"]);
    let before = named(&run.session, "canvas", "reload-badge");
    assert_eq!(before.len(), 1, "generation 1 made the badge once: {before:?}");
    assert!(
        (before[0].1 - 10.0).abs() < 0.5,
        "the badge shows version 1 (x = 10): {before:?}"
    );
    assert_eq!(
        named(&run.session, "canvas", "reload-toast").len(),
        1,
        "generation 1 made its ephemeral toast"
    );

    // The rebuild: the file the shell was loaded from is replaced by one that behaves differently.
    swap_in(&hot.v2, &run.live);
    wait_count(
        &mut run.session,
        "the watcher reloaded the shell to generation 2",
        1,
        |line| line.contains("the shell reloaded"),
    );
    run.session.wait_for("generation 2 is serving", |session| {
        named(session, "canvas", "reload-badge")
            .first()
            .is_some_and(|badge| (badge.1 - 20.0).abs() < 0.5)
    });

    // The client's connection and its surfaces survived: the same connection still answers, the windows are the same nodes in the
    // same places, and a window opened now is placed by the new code (an 80 pixel cascade), counting on from the state the first
    // generation kept (two placed already).
    run.client.roundtrip();
    let windows = support::scene(&run.session.ipc());
    for (app, id, n) in [("first", run.ids[0], 0), ("second", run.ids[1], 1)] {
        let window = windows
            .iter()
            .find(|window| window.app_id == app)
            .unwrap_or_else(|| panic!("`{app}` is gone: {windows:?}"));
        assert_eq!(window.id, id, "`{app}` is the same node");
        assert!(at(window, n, 40.0), "`{app}` has not moved: {window:?}");
    }
    run.client.map_window("third", "Third");
    wait_for_window(
        &mut run.session,
        "generation 2 places the third window by its own step",
        "third",
        |window| at(window, 2, 80.0),
    );

    // Nothing was made twice: one badge, the same node, and the ephemeral toast the new code did not adopt is gone.
    let after = named(&run.session, "canvas", "reload-badge");
    assert_eq!(after.len(), 1, "one badge: {after:?}");
    assert_eq!(after[0].0, before[0].0, "the badge is the node generation 1 made");
    assert!(
        named(&run.session, "canvas", "reload-toast").is_empty(),
        "the toast generation 2 did not adopt was removed"
    );

    // The second door: `gnoblinctl shell reload`, the same file again.
    let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "reloaded: generation 3\n"),
        "{err}\n{}",
        run.session.log_text()
    );
    run.client.roundtrip();
    run.client.map_window("fourth", "Fourth");
    wait_for_window(
        &mut run.session,
        "generation 3 counts on from the kept state, once",
        "fourth",
        |window| at(window, 3, 80.0),
    );
    let (code, out, _) = ctl(&run.session, &["shell", "status"]);
    assert_eq!(code, 0);
    assert!(out.contains("generation 3 is running"), "{out}");
    assert!(out.contains("2 reload(s) committed, 0 failed"), "{out}");
    assert_eq!(
        named(&run.session, "canvas", "reload-badge").len(),
        1,
        "still one badge after two reloads"
    );
    assert!(
        run.session.exited().is_none(),
        "the session runs on:\n{}",
        run.session.log_text()
    );
}

/// The `z` of the reload badge: what `<Super>n` counts.
fn badge_z(session: &Gnoblin) -> Option<f64> {
    named(session, "canvas", "reload-badge").first().map(|badge| badge.3)
}

/// Types `<Super>n` until the badge's `z` reaches `wanted`. The new generation claims the chord when its `init` has run and the
/// compositor has read the claim, so a press typed too early reaches a client and is lost; the typist tries again after it has
/// looked, and never types once `z` is where it should be.
#[track_caller]
fn count_a_press(session: &mut Gnoblin, typist: &mut Typist, wanted: f64) {
    session.wait_for(&format!("a press of `<Super>n` is counted (z = {wanted})"), |session| {
        if badge_z(session).is_some_and(|z| (z - wanted).abs() < 0.5) {
            return true;
        }
        typist.super_plus(KEY_N);
        std::thread::sleep(Duration::from_millis(250));
        badge_z(session).is_some_and(|z| (z - wanted).abs() < 0.5)
    });
    assert!(
        badge_z(session).is_some_and(|z| (z - wanted).abs() < 0.5),
        "exactly one listener counted it: z = {:?}\n{}",
        badge_z(session),
        session.log_text()
    );
}

/// In a real session: a shell reloaded twice by `gnoblinctl shell reload` keeps what it asked the core to keep, finds the nodes
/// it adopted with the identity they had, is not called by the generations it replaced (a thread of a retired generation is refused
/// with `Retired` and what it posted never runs), and the Wayland client is connected throughout.
#[test]
fn example_10_two_reloads_by_gnoblinctl_keep_state_and_node_identity_and_never_call_a_retired_generation() {
    let mut run = running(&[]);
    let before = named(&run.session, "canvas", "reload-badge");
    assert_eq!(before.len(), 1, "generation 1 made the badge once: {before:?}");
    let badge = before[0].0;
    let mut typist = Typist::connect(&run.session.wayland());

    // Generation 1 counts one press and keeps the count (`keep_state("presses")`); two windows are placed and counted (`opened`).
    count_a_press(&mut run.session, &mut typist, 1.0);

    // The first reload, of a rebuilt library (80 pixel cascade).
    swap_in(&hot().v2, &run.live);
    let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "reloaded: generation 2\n"),
        "{err}\n{}",
        run.session.log_text()
    );
    run.client.roundtrip();
    run.client.map_window("third", "Third");
    wait_for_window(
        &mut run.session,
        "generation 2 counts the third window on from the kept state, by its own step",
        "third",
        |window| at(window, 2, 80.0),
    );

    // The second reload, of the same file.
    let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "reloaded: generation 3\n"),
        "{err}\n{}",
        run.session.log_text()
    );
    run.client.roundtrip();
    run.client.map_window("fourth", "Fourth");
    wait_for_window(
        &mut run.session,
        "generation 3 counts on from state two reloads old",
        "fourth",
        |window| at(window, 3, 80.0),
    );

    // The press count went through two reloads: generation 3 reads 1 and counts 2, and only one listener (its own) counted it.
    count_a_press(&mut run.session, &mut typist, 2.0);

    // The badge is the node generation 1 adopted, once, and the client's windows are the nodes they were.
    let after = named(&run.session, "canvas", "reload-badge");
    assert_eq!(after.len(), 1, "one badge after two reloads: {after:?}");
    assert_eq!(after[0].0, badge, "the badge kept its node id through both reloads");
    run.client.roundtrip();
    let windows = support::scene(&run.session.ipc());
    for (app, id) in [("first", run.ids[0]), ("second", run.ids[1])] {
        let window = windows
            .iter()
            .find(|window| window.app_id == app)
            .unwrap_or_else(|| panic!("`{app}` is gone: {windows:?}"));
        assert_eq!(window.id, id, "`{app}` is the node it was");
    }

    // The retired generations: the thread each one started was refused with `Retired` once its generation was replaced, and the
    // closure it posted then never ran. The generation that serves is not refused.
    for generation in [1, 2] {
        let wanted = format!("poster of generation {generation} refused (Retired): the refused closure ran: false");
        run.session
            .wait_log(&format!("the poster of generation {generation} is refused"), |line| {
                line.contains(&wanted)
            });
    }
    assert!(
        count(&run.session, |line| line.contains("poster of generation 3 refused")) == 0,
        "the serving generation's poster works: {}",
        run.session.log_text()
    );
    let (code, status, _) = ctl(&run.session, &["shell", "status"]);
    assert_eq!(code, 0);
    assert!(status.contains("generation 3 is running"), "{status}");
    assert!(status.contains("2 reload(s) committed, 0 failed"), "{status}");
    assert!(run.session.exited().is_none(), "{}", run.session.log_text());
}

#[test]
fn example_10_the_shell_asks_for_its_own_reload_with_a_chord() {
    let mut run = running(&[]);
    // Rebuilt and not watched: nothing reloads until somebody asks, and here the shell does, as a developer binds a key to.
    swap_in(&hot().v2, &run.live);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        count(&run.session, |line| line.contains("the shell reloaded")),
        0,
        "no --watch, no reload"
    );
    let mut typist = Typist::connect(&run.session.wayland());
    typist.super_plus(KEY_R);
    wait_count(&mut run.session, "the chord reloaded the shell", 1, |line| {
        line.contains("the shell reloaded")
    });
    run.client.roundtrip();
}

#[test]
fn gnoblinctl_shell_watch_reloads_the_session_each_time_the_library_is_rebuilt() {
    let hot = hot();
    let mut run = running(&[]);
    // `shell watch --once` is the developer's loop with the exit after one reload: it waits for the file to change, reloads, and
    // exits with the reload's own code.
    let ipc = run.session.ipc();
    let live = run.live.clone();
    let watcher = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = gnoblinctl::run(
            ["shell", "watch", "--once"]
                .iter()
                .map(OsString::from)
                .chain([live.into_os_string()]),
            gnoblinctl::SocketConnection::at(ipc),
            &mut out,
            &mut err,
        );
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    });
    std::thread::sleep(Duration::from_millis(700));
    swap_in(&hot.v2, &run.live);
    let (code, out, err) = watcher.join().expect("the watcher ends");
    assert_eq!(code, 0, "{out}\n{err}\n{}", run.session.log_text());
    assert!(out.contains("watching"), "{out}");
    assert!(out.contains("reloaded: generation 2"), "{out}");
    run.client.roundtrip();
    run.client.map_window("third", "Third");
    wait_for_window(
        &mut run.session,
        "the reloaded shell places by its own step",
        "third",
        |window| at(window, 2, 80.0),
    );
}

#[test]
fn example_18_replacements_that_cannot_take_over_leave_the_running_shell_serving_and_a_good_one_works_after() {
    let hot = hot();
    let mut run = running(&["--watch"]);
    let failed = |session: &Gnoblin| {
        count(session, |line| {
            line.contains("the shell did not reload and keeps running")
        })
    };
    let reasons = |session: &Gnoblin, needle: &str| {
        count(session, |line| {
            line.contains("did not reload") && line.contains("still running generation 1") && line.contains(needle)
        })
    };

    // 1. A replacement whose `init` registers a listener and then returns an error.
    swap_in(&hot.bad, &run.live);
    wait_count(
        &mut run.session,
        "the failed init is reported with its reason",
        1,
        |line| line.contains("did not reload") && line.contains("init failed"),
    );
    assert!(
        reasons(&run.session, &run.live.display().to_string()) >= 1,
        "the report names the module and the generation that still runs:\n{}",
        run.session.log_text()
    );
    // 2. One that panics in `init`.
    swap_in(&hot.panics, &run.live);
    wait_count(&mut run.session, "the panic in init is reported", 1, |line| {
        line.contains("did not reload") && line.contains("E_PANIC")
    });
    // 3. A file a build is still writing: cut inside the ELF header.
    let whole = fs::read(&hot.v2).expect("the library");
    swap_in_bytes(&whole[..40], &run.live);
    wait_count(&mut run.session, "the truncated file is reported", 1, |line| {
        line.contains("did not reload") && line.contains("file too short")
    });
    // 4. A file that is not a library at all.
    swap_in_bytes(&b"this is not a library\n".repeat(50), &run.live);
    wait_count(&mut run.session, "the garbage is reported", 1, |line| {
        line.contains("did not reload") && line.contains("invalid ELF header")
    });
    assert!(failed(&run.session) >= 4, "{}", run.session.log_text());

    // Through all of it the first generation served, and the client never noticed: the connection answers, the windows are where
    // they were, the badge is the first generation's, and a window opened now is placed by the first generation (a 40 pixel step).
    run.client.roundtrip();
    let windows = support::scene(&run.session.ipc());
    for (app, id, n) in [("first", run.ids[0], 0), ("second", run.ids[1], 1)] {
        let window = windows
            .iter()
            .find(|window| window.app_id == app)
            .expect("the window is there");
        assert_eq!(window.id, id);
        assert!(at(window, n, 40.0), "{window:?}");
    }
    run.client.map_window("third", "Third");
    wait_for_window(
        &mut run.session,
        "the running generation places the third window",
        "third",
        |window| at(window, 2, 40.0),
    );
    let badge = named(&run.session, "canvas", "reload-badge");
    assert_eq!(badge.len(), 1, "no node of a failed generation is left: {badge:?}");
    assert!((badge[0].1 - 10.0).abs() < 0.5, "{badge:?}");
    assert!(
        !run.session.log_text().contains("generation 2 serves") && run.session.exited().is_none(),
        "{}",
        run.session.log_text()
    );

    // The status says so, and `shell reload` of a bad file is exit 1 with the same text.
    let (code, out, _) = ctl(&run.session, &["shell", "status"]);
    assert_eq!(code, 0);
    assert!(out.contains("generation 1 is running"), "{out}");
    assert!(out.contains("0 reload(s) committed"), "{out}");
    let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert!(
        err.contains("reload of") && err.contains("failed, still running generation 1"),
        "the refusal is the host's text: {err}"
    );

    // 5. A good build takes over after all of that, and the new behaviour is in force.
    swap_in(&hot.v2, &run.live);
    wait_count(&mut run.session, "a good replacement reloads", 1, |line| {
        line.contains("the shell reloaded")
    });
    run.session.wait_for("generation 2 is serving", |session| {
        named(session, "canvas", "reload-badge")
            .first()
            .is_some_and(|badge| (badge.1 - 20.0).abs() < 0.5)
    });
    run.client.roundtrip();
    run.client.map_window("fourth", "Fourth");
    wait_for_window(
        &mut run.session,
        "the new generation places by its own step",
        "fourth",
        |window| at(window, 3, 80.0),
    );
}

#[test]
fn an_isolated_shell_reloads_itself_when_its_library_is_rebuilt_and_the_compositor_never_notices() {
    let hot = hot();
    let home = Home::new();
    let live = home.copy(&hot.v1, "live/libhot_reload.so");
    let mut session = Gnoblin::launch(
        home,
        Launch::guardian(&["--isolate", "--watch"]).with_arg(live.clone().into_os_string()),
    );
    session.wait_serving();
    session.wait_log("generation 1 serves in its own process", |line| {
        line.contains("serves in its own process (generation 1)")
    });
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("first", "First");
    let first = wait_for_window(
        &mut session,
        "generation 1 places the first window",
        "first",
        |window| at(window, 0, 40.0),
    );
    swap_in(&hot.v2, &live);
    wait_count(&mut session, "the isolated shell reloaded", 1, |line| {
        line.contains("the shell reloaded")
    });
    client.roundtrip();
    client.map_window("second", "Second");
    wait_for_window(
        &mut session,
        "generation 2 places by its own step",
        "second",
        |window| at(window, 1, 80.0),
    );
    let windows = support::scene(&session.ipc());
    assert_eq!(
        windows
            .iter()
            .find(|window| window.app_id == "first")
            .map(|window| window.id),
        Some(first.id),
        "the first window is the node it was"
    );
    assert_eq!(
        count(&session, |line| line.contains("started shell (")),
        1,
        "the shell process was not restarted:\n{}",
        session.log_text()
    );
    // The socket's shell members reach only a shell in the compositor's process, and say so.
    let (code, _, err) = ctl(&session, &["shell", "reload"]);
    assert_eq!(code, 1);
    assert!(err.contains("runs no shell in its own process"), "{err}");
}

#[test]
fn a_session_with_no_in_process_shell_says_there_is_nothing_to_reload() {
    let home = Home::new();
    let mut session = Gnoblin::launch(home, Launch::compositor(&[]));
    session.wait_serving();
    let (code, _, err) = ctl(&session, &["shell", "reload"]);
    assert_eq!(code, 1);
    assert!(err.contains("runs no shell in its own process"), "{err}");
}

/// Whether a mapped path is one of the unique copies the host makes of a shell library (`.../gen-N/...`).
fn is_shell_copy(path: &str) -> bool {
    path.contains("/gen-")
}

#[test]
fn the_retired_libraries_are_bounded_at_32_and_the_reload_past_the_limit_is_refused_while_the_shell_serves_on() {
    // A retired native generation is never unloaded, so every reload keeps one library mapped for the life of the process. The
    // bound is 32 libraries, the ones of failed replacements included (a library that was mapped and refused may have left a thread
    // or a destructor behind, as a retired one may). Past it a reload is refused with a text that says what to do, and nothing else
    // changes: the running generation serves, the client stays connected.
    let hot = hot();
    let mut run = running(&[]);
    for generation in 2..=31_u32 {
        let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
        assert_eq!(
            (code, out.as_str()),
            (0, format!("reloaded: generation {generation}\n").as_str()),
            "{err}\n{}",
            run.session.log_text()
        );
    }
    // 30 reloads retired 30 generations. Two replacements whose `init` fails and panics are refused, and their libraries stay mapped: 32.
    for (replacement, why) in [(&hot.bad, "init failed"), (&hot.panics, "E_PANIC")] {
        swap_in(replacement, &run.live);
        let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
        assert_eq!(code, 1, "{out}\n{err}");
        assert!(
            err.contains(why) && err.contains("still running generation 31"),
            "{err}"
        );
    }
    let (code, status, _) = ctl(&run.session, &["shell", "status"]);
    assert_eq!(code, 0);
    assert!(
        status.contains("32 of 32 retired shell libraries retained"),
        "the status counts the libraries of the failed replacements too: {status}"
    );

    // The 33rd library would be one past the bound: a good replacement is refused, before anything is copied or mapped.
    swap_in(&hot.v2, &run.live);
    let (code, out, err) = ctl(&run.session, &["shell", "reload"]);
    assert_eq!(code, 1, "{out}\n{err}");
    assert!(
        err.contains("32 retired shell libraries are still mapped (the limit is 32)")
            && err.contains("restart gnoblin"),
        "the refusal says what is mapped, the limit and what to do: {err}"
    );
    assert!(
        err.contains("still running generation 31"),
        "and which generation serves: {err}"
    );
    let copies = run
        .session
        .mapped_files()
        .iter()
        .filter(|path| is_shell_copy(path))
        .count();
    assert_eq!(
        copies,
        33,
        "30 retired generations, 2 refused replacements and the running one stay mapped, and the refused reload mapped nothing: {:#?}",
        run.session.mapped_files()
    );

    // The shell that serves is generation 31, by its own step: a window opened now is placed by it, the client never noticed, and the
    // session runs on. A watcher or a chord asking for a reload meets the same refusal and the same running generation.
    run.client.roundtrip();
    run.client.map_window("third", "Third");
    wait_for_window(
        &mut run.session,
        "generation 31 places the third window by its 40 pixel step",
        "third",
        |window| at(window, 2, 40.0),
    );
    let (code, status, _) = ctl(&run.session, &["shell", "status"]);
    assert_eq!(code, 0);
    assert!(status.contains("generation 31 is running"), "{status}");
    assert!(run.session.exited().is_none(), "{}", run.session.log_text());
}
