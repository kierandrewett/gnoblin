//! Which shell `gnoblin` starts, as processes (examples 13 to 16).
//!
//! Every test runs the shipped binary in a private home: the config, data and system-data directories discovery reads are
//! the test's own, so nothing of the machine's reaches it. The shells are the workspace's own `.so` examples (`shells()`).
//!
//! | example | what is proved |
//! | ------- | -------------- |
//! | 13 | `gnoblin` with no arguments finds `shell.so` in the config directory (tier 4), runs it, and the log names the tier |
//! | 14 | a launcher file in `$XDG_DATA_DIRS` is run by name (`--shell NAME`), its `module` key relative to the core's library directory; `--list-shells` prints its row |
//! | 15 | two equal candidates are an error with the candidate list and the tiers, a shell named `default` wins, a `config.toml` default overrides, and the table is in tier order |
//! | 16 | nothing installed: exit 1, the exact text of design 7.4, and no compositor was started |
//!
//! The packaged default (tier 7) and `--isolate` are here too: an installed layout (`Prefix`) is the only way to show what the
//! launcher finds beside itself.

use std::process::Output;

mod support;
use support::Mapper;
use support::native_shell::*;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn stderr(output: &Output) -> String {
    text(&output.stderr)
}

fn stdout(output: &Output) -> String {
    text(&output.stdout)
}

/// Whether a window of `app_id` sits where the minimal shell's cascade puts its `n`th (60 + 40n on both axes).
fn cascaded(window: &support::Window, n: u32) -> bool {
    let at = 60.0 + 40.0 * f64::from(n);
    (window.x - at).abs() < 0.5 && (window.y - at).abs() < 0.5
}

/// Waits for the session to serve and its shell to start, and returns the log line that says which shell was chosen.
#[track_caller]
fn started(session: &mut Gnoblin) -> String {
    let chosen = session.wait_log("the launcher says which shell it chose", |line| {
        line.contains("shell: ")
    });
    session.wait_serving();
    session.wait_log("the shell serves in process", |line| line.contains("serves in process"));
    chosen.text
}

/// Maps a window and waits for the minimal shell to cascade it first.
#[track_caller]
fn minimal_places_a_window(session: &mut Gnoblin) -> Mapper {
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    wait_for_window(session, "the minimal shell places the window", "first", |window| {
        cascaded(window, 0)
    });
    mapper
}

// ---- example 13 ----------------------------------------------------------------------------------------------------------

#[test]
fn example_13_gnoblin_with_no_arguments_runs_the_shell_in_the_config_directory_and_names_the_tier() {
    let home = Home::new();
    home.copy(&shells().minimal, "config/gnoblin/shell.so");
    let mut session = Gnoblin::launch(home, Launch::guardian(&[]));
    let chosen = started(&mut session);
    assert!(
        chosen.contains("chosen by tier 4 (user-config)") && chosen.contains("shell.so"),
        "the log names the tier that answered: {chosen}"
    );
    session.wait_log("the guardian heard the shell accepted", |line| {
        line.contains("[guardian] configuration revision 0 accepted")
    });
    let _mapper = minimal_places_a_window(&mut session);
    assert!(
        !session.log_text().contains("started shell ("),
        "an in-process shell has no child:\n{}",
        session.log_text()
    );
}

#[test]
fn a_shell_named_by_the_environment_beats_the_config_directory_and_a_path_beats_the_environment() {
    let home = Home::new();
    home.copy(&shells().minimal, "config/gnoblin/shell.so");
    let tiler = home.copy(&shells().tiler, "elsewhere/libtiler.so");
    let mut session = Gnoblin::launch(
        home,
        Launch::guardian(&[]).with_env("GNOBLIN_SHELL", &tiler.display().to_string()),
    );
    let chosen = started(&mut session);
    assert!(chosen.contains("chosen by tier 2 (environment)"), "{chosen}");
    drop(session);

    let home = Home::new();
    let minimal = home.copy(&shells().minimal, "elsewhere/libminimal.so");
    let tiler = home.copy(&shells().tiler, "elsewhere/libtiler.so");
    let mut session = Gnoblin::launch(
        home,
        Launch::guardian(&[])
            .with_arg(&minimal)
            .with_env("GNOBLIN_SHELL", &tiler.display().to_string()),
    );
    let chosen = started(&mut session);
    assert!(chosen.contains("chosen by tier 1 (command-line)"), "{chosen}");
    minimal_places_a_window(&mut session);
}

// ---- example 14 ----------------------------------------------------------------------------------------------------------

#[test]
fn example_14_a_system_installed_shell_is_run_by_name_and_listed_with_its_row() {
    let prefix = Prefix::new();
    // The launcher file names its module relative to the core's library directory, `<prefix>/lib/gnoblin/shells`, which the
    // launcher finds by asking the loader where the core is mapped from.
    prefix.install(&shells().tiler, &prefix.module_dir(), "libtiler.so");
    let home = Home::new();
    let launcher = home.write(
        "share/gnoblin/shells/tiler.toml",
        "module = \"libtiler.so\"\ndescription = \"A master and stack tiler\"\n",
    );

    let listing = home
        .command(&prefix.binary())
        .arg("--list-shells")
        .output()
        .expect("the binary runs");
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    let module = prefix.module_dir().join("libtiler.so");
    assert_eq!(
        stdout(&listing),
        format!(
            "NAME   TIER  LANGUAGE  DEFAULT  LOCATION\ntiler  6     native    yes      {} -> {}\n",
            launcher.display(),
            module.display()
        )
    );

    let mut session = Gnoblin::launch(
        home,
        Launch::guardian(&["--shell", "tiler"]).with_binary(prefix.binary()),
    );
    let chosen = started(&mut session);
    assert!(
        chosen.contains("chosen by tier 1 (command-line), found in tier 6"),
        "{chosen}"
    );
    // The tiler, not the compositor's own window management: one window fills the screen less the 10 pixel gaps.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    let (width, height) = screen_size(&session.ipc());
    wait_for_window(&mut session, "the named shell tiles the window", "a", |window| {
        (window.x - 10.0).abs() < 0.5
            && (window.y - 10.0).abs() < 0.5
            && (window.width - (width - 20.0)).abs() < 0.5
            && (window.height - (height - 20.0)).abs() < 0.5
    });
}

#[test]
fn a_name_that_is_not_installed_says_so_and_starts_nothing() {
    let prefix = Prefix::new();
    let home = Home::new();
    home.copy(&shells().minimal, "share/gnoblin/shells/lines.so");
    let output = home
        .command(&prefix.binary())
        .args(["--shell", "linez"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.starts_with("gnoblin: no shell named \"linez\""), "{message}");
    assert!(message.contains("lines"), "the installed shell is listed: {message}");
    assert!(!home.root().join("ipc.sock").exists(), "no compositor was started");
}

// ---- example 15 ----------------------------------------------------------------------------------------------------------

/// The remedies of every failure that has no explicit request, with the config file of `home`.
fn remedies(home: &Home) -> String {
    format!(
        "Run a shell directly:     gnoblin ./myshell.so\n\
         Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)\n\
         Set a default:            [shell] default = \"NAME\"  in {}",
        home.config().join("config.toml").display()
    )
}

/// The "Looked in this order" block, with the lines a test changes given as `(tier 5, tier 7)` text.
fn looked_in(home: &Home, prefix: &Prefix, config: &str, user: &str) -> String {
    format!(
        "Looked in this order:\n\
         \x20 1. command line            not given\n\
         \x20 2. GNOBLIN_SHELL           not set\n\
         \x20 3. {config_toml}   {config}\n\
         \x20 4. {config_dir}/shell.{{so,py,js,mjs,ts,mts}}   none present\n\
         \x20 5. {user_dir}/  {user}\n\
         \x20 6. {system}/  {system_state}\n\
         \x20 7. {packaged}/shell.*   not installed",
        config_toml = home.config().join("config.toml").display(),
        config_dir = home.config().display(),
        user_dir = home.user_shells().display(),
        system = home.system_shells().display(),
        system_state = if home.system_shells().is_dir() {
            "empty"
        } else {
            "missing"
        },
        packaged = prefix.default_shell_dir().display(),
    )
}

#[test]
fn example_15_two_equal_candidates_are_an_error_that_lists_them_and_where_it_looked() {
    let prefix = Prefix::new();
    let home = Home::new();
    let lines = home.copy(&shells().minimal, "data/gnoblin/shells/lines.so");
    let tiler = home.copy(&shells().tiler, "data/gnoblin/shells/tiler.so");
    let output = home.command(&prefix.binary()).output().expect("the binary runs");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(!home.root().join("ipc.sock").exists(), "no compositor was started");
    let expected = format!(
        "gnoblin: several shells are equally preferred:\n\
         \x20 lines  {}  (native)\n\
         \x20 tiler  {}  (native)\n\n{}\n\n{}",
        lines.display(),
        tiler.display(),
        looked_in(&home, &prefix, "missing", "2 shells")
            .replace(
                &format!("{}/  missing", home.system_shells().display()),
                &format!("{}/  not reached", home.system_shells().display())
            )
            .replace(
                &format!("{}/shell.*   not installed", prefix.default_shell_dir().display()),
                &format!("{}/shell.*   not reached", prefix.default_shell_dir().display())
            ),
        remedies(&home),
    );
    assert_eq!(stderr(&output).trim_end(), expected);
}

#[test]
fn example_15_a_shell_named_default_wins_and_a_config_default_overrides_it() {
    let prefix = Prefix::new();
    let home = Home::new();
    home.copy(&shells().minimal, "data/gnoblin/shells/lines.so");
    home.copy(&shells().tiler, "data/gnoblin/shells/default.so");
    let listing = home
        .command(&prefix.binary())
        .arg("--list-shells")
        .output()
        .expect("the binary runs");
    assert_eq!(listing.status.code(), Some(0));
    let table = stdout(&listing);
    let marked: Vec<&str> = table.lines().filter(|line| line.contains("yes")).collect();
    assert_eq!(marked.len(), 1, "one shell is marked: {table}");
    assert!(
        marked[0].starts_with("default"),
        "`default` is the one that would run: {table}"
    );

    // `default` wins: the tiler runs, so one window fills the screen.
    let mut session = Gnoblin::launch(home, Launch::guardian(&[]).with_binary(prefix.binary()));
    started(&mut session);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(&mut session, "the default shell (the tiler) tiles", "a", |window| {
        (window.x - 10.0).abs() < 0.5 && (window.y - 10.0).abs() < 0.5
    });
    drop(session);

    // A `config.toml` names `lines`, which beats the name `default` in the directory: tier 3 comes before tier 5.
    let home = Home::new();
    home.copy(&shells().minimal, "data/gnoblin/shells/lines.so");
    home.copy(&shells().tiler, "data/gnoblin/shells/default.so");
    home.write("config/gnoblin/config.toml", "[shell]\ndefault = \"lines\"\n");
    let mut session = Gnoblin::launch(home, Launch::guardian(&[]).with_binary(prefix.binary()));
    let chosen = started(&mut session);
    assert!(
        chosen.contains("chosen by tier 3 (config), found in tier 5"),
        "{chosen}"
    );
    minimal_places_a_window(&mut session);
}

#[test]
fn example_15_the_list_is_in_tier_order_and_marks_the_one_that_would_run() {
    let prefix = Prefix::new();
    let home = Home::new();
    let user = home.copy(&shells().minimal, "config/gnoblin/shell.so");
    let installed = home.copy(&shells().tiler, "data/gnoblin/shells/tiler.so");
    let system = home.copy(&shells().fault, "share/gnoblin/shells/fault.so");
    let packaged = prefix.install(&shells().minimal, &prefix.default_shell_dir(), "shell.so");
    let listing = home
        .command(&prefix.binary())
        .arg("--list-shells")
        .output()
        .expect("the binary runs");
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    assert_eq!(
        stdout(&listing),
        format!(
            "NAME     TIER  LANGUAGE  DEFAULT  LOCATION\n\
             shell    4     native    yes      {}\n\
             tiler    5     native             {}\n\
             fault    6     native             {}\n\
             default  7     native             {}  (packaged)\n",
            user.display(),
            installed.display(),
            system.display(),
            packaged.display()
        )
    );
}

// ---- example 16 ----------------------------------------------------------------------------------------------------------

#[test]
fn example_16_with_no_shell_anywhere_gnoblin_says_where_it_looked_and_starts_no_compositor() {
    let prefix = Prefix::new();
    let home = Home::new();
    let output = home.command(&prefix.binary()).output().expect("the binary runs");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "the failure is on standard error");
    let expected = format!(
        "gnoblin: no shell found.\n\n{}\n\n{}",
        looked_in(&home, &prefix, "missing", "missing"),
        remedies(&home)
    );
    assert_eq!(stderr(&output).trim_end(), expected);
    // Nothing was started: no socket, no Wayland display, no children.
    for name in ["ipc.sock", gnoblin_wayland::DEFAULT_SOCKET_NAME] {
        assert!(
            !home.root().join(name).exists(),
            "`{name}` exists, so a compositor started"
        );
    }
}

#[test]
fn a_bare_word_that_is_not_a_file_asks_whether_a_name_was_meant() {
    let prefix = Prefix::new();
    let home = Home::new();
    let output = home
        .command(&prefix.binary())
        .arg("tiler")
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output).trim_end(),
        "gnoblin: tiler: no such file (from the command line).\nDid you mean --shell tiler?    (a path needs a `/`, as in ./tiler)"
    );
}

// ---- the packaged default (tier 7) -----------------------------------------------------------------------------------------

#[test]
fn a_fresh_install_runs_the_packaged_default_found_beside_the_launcher() {
    let prefix = Prefix::new();
    let packaged = prefix.install(&shells().minimal, &prefix.default_shell_dir(), "shell.so");
    let home = Home::new();
    let listing = home
        .command(&prefix.binary())
        .arg("--list-shells")
        .output()
        .expect("the binary runs");
    assert_eq!(
        stdout(&listing),
        format!(
            "NAME     TIER  LANGUAGE  DEFAULT  LOCATION\ndefault  7     native    yes      {}  (packaged)\n",
            packaged.display()
        )
    );
    let mut session = Gnoblin::launch(home, Launch::guardian(&[]).with_binary(prefix.binary()));
    let chosen = started(&mut session);
    assert!(chosen.contains("chosen by tier 7 (packaged)"), "{chosen}");
    minimal_places_a_window(&mut session);

    // The same shell is a name: `--shell default` is what the packaged default is called.
    let output = Home::new()
        .command(&prefix.binary())
        .args(["--shell", "default", "--list-shells"])
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

// ---- the one code path ------------------------------------------------------------------------------------------------------

#[test]
fn an_explicit_path_and_a_discovered_shell_run_through_the_same_loader() {
    // `gnoblin ./x.so` and `gnoblin` differ only in which tier answered: the compositor child is started with the same words.
    let home = Home::new();
    let shell = home.copy(&shells().minimal, "config/gnoblin/shell.so");
    let mut discovered = Gnoblin::launch(
        Home::new(),
        Launch::guardian(&[]).with_env("GNOBLIN_SHELL", &shell.display().to_string()),
    );
    started(&mut discovered);
    let mut explicit = Gnoblin::launch(home, Launch::guardian(&[]).with_arg(&shell));
    started(&mut explicit);
    for session in [&discovered, &explicit] {
        let log = session.log_text();
        assert!(log.contains("serves in process (generation 1)"), "{log}");
    }
}

#[test]
fn an_unreadable_config_toml_ends_the_start_when_it_decides_and_only_warns_when_a_higher_tier_does() {
    let prefix = Prefix::new();
    let home = Home::new();
    home.write("config/gnoblin/config.toml", "[shell\n");
    let output = home.command(&prefix.binary()).output().expect("the binary runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("config.toml"), "{}", stderr(&output));

    let shell = home.copy(&shells().minimal, "elsewhere/libminimal.so");
    let mut session = Gnoblin::launch(
        home,
        Launch::guardian(&[]).with_binary(prefix.binary()).with_arg(&shell),
    );
    started(&mut session);
    session.wait_log("the unreadable file is a warning", |line| {
        line.contains("config.toml") && line.contains("ignored")
    });
}

// ---- isolation -------------------------------------------------------------------------------------------------------------

/// The pid of the latest `started <name> (generation N, pid P)` the guardian logged.
fn pid_of(session: &Gnoblin, name: &str) -> i32 {
    session
        .log()
        .iter()
        .rev()
        .find(|line| line.text.contains(&format!("started {name} (")))
        .and_then(|line| {
            let rest = line.text.split("pid ").nth(1)?;
            rest.split(')').next()?.trim().parse().ok()
        })
        .unwrap_or_else(|| panic!("no `started {name}` in:\n{}", session.log_text()))
}

fn maps_of(pid: i32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/maps")).unwrap_or_default()
}

#[test]
fn isolate_runs_the_shell_in_a_process_of_its_own_and_a_crash_in_it_does_not_take_the_compositor() {
    let home = Home::new();
    let shell = home.copy(&shells().minimal, "elsewhere/libminimal.so");
    let mut session = Gnoblin::launch(home, Launch::guardian(&["--isolate"]).with_arg(&shell));
    session.wait_log("the shell child is ready", |line| {
        line.contains("serves in its own process")
    });
    session.wait_serving();
    session.wait_log("the guardian heard the shell accepted", |line| {
        line.contains("[guardian] configuration revision 0 accepted")
    });
    let compositor = pid_of(&session, "compositor");
    let host = pid_of(&session, "shell");
    assert_ne!(compositor, host);
    assert!(
        maps_of(host).contains("/gen-"),
        "the shell's copy of the module is mapped in the shell's process"
    );
    assert!(
        !maps_of(compositor).contains("/gen-"),
        "and not in the compositor's: {}",
        maps_of(compositor)
    );

    // The shell holds the tree: it places the window.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    wait_for_window(
        &mut session,
        "the isolated shell places the window",
        "first",
        |window| cascaded(window, 0),
    );

    // A crash in shell code (an abort here, as a native shell's panic across the boundary ends) is the shell's: the compositor and the client carry on, and the guardian starts the shell again.
    let id = rustix::process::Pid::from_raw(host).expect("a pid");
    rustix::process::kill_process(id, rustix::process::Signal::ABORT).expect("the shell is aborted");
    session.wait_log("the guardian started the shell again", |line| {
        line.contains("started shell (generation 2")
    });
    session.wait_for("the new shell serves", |session| {
        session
            .log()
            .iter()
            .filter(|line| line.text.contains("serves in its own process (generation 1)"))
            .count()
            >= 2
    });
    assert!(
        rustix::process::test_kill_process(rustix::process::Pid::from_raw(compositor).expect("a pid")).is_ok(),
        "the compositor is still there:\n{}",
        session.log_text()
    );
    mapper.roundtrip();
    mapper.map_window("second", "Second");
    wait_for_window(
        &mut session,
        "the restarted shell places the next window",
        "second",
        |window| window.x >= 60.0,
    );
}
