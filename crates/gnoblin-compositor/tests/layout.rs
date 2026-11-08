//! The `layout` section through the real compositor path: a headless session, a real
//! Wayland client, and the configure sizes the client receives.
//!
//! The headless output is 1024x768 and the layouts run with zero gaps, so the expected
//! sizes are the shipped `master_stack` geometry for that output: one master column on
//! the left half and two stack windows sharing the right half.

mod support;

use std::path::PathBuf;
use std::time::Duration;

use gnoblin_compositor::LayoutSettings;
use gnoblin_config::LayoutSelector;
use support::{Client, Command, Session, Window, wait_for, wait_for_listed_id};

/// A layout named by its shipped or user name, as the `layout` section's string form.
fn named(name: &str) -> LayoutSelector {
    LayoutSelector::Named(name.to_owned())
}

/// The repository's shipped Lua layouts, the directory an installed prefix provides.
fn shipped_modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/lua")
}

/// The FNV-1a hash of a module's source, which a layout reference pins. The Lua host
/// computes the same function, but it is private to that crate, so the test writes it out.
fn source_hash(path: &std::path::Path) -> String {
    let source = std::fs::read(path).expect("the module source");
    let hash = source.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// The hash of the shipped `master_stack` module, the reference a test names it by.
fn master_stack_hash() -> String {
    source_hash(&shipped_modules().join("gnoblin/layouts/master_stack.lua"))
}

/// A session configured with `settings`, which is every layout test's only difference.
fn session_with(settings: LayoutSettings) -> Session {
    let (session, _ends) = Session::start_privileged_configured(&[], |config| config.layouts = settings);
    session
}

/// Maps three toplevels in order and returns them once each has its first configure.
fn map_three(client: &mut Client) -> [Window; 3] {
    let first = client.map_window("org.example.one", "one");
    let second = client.map_window("org.example.two", "two");
    let third = client.map_window("org.example.three", "three");
    let windows = [first, second, third];
    wait_for("every window to be configured", || {
        client.roundtrip();
        windows
            .iter()
            .all(|window| client.configured_size(window).is_some())
            .then_some(())
    });
    windows
}

/// The configure size of each window, once the sizes have settled over a few round trips.
fn settled_sizes(client: &mut Client, windows: &[Window; 3]) -> Vec<(i32, i32)> {
    let mut previous = Vec::new();
    wait_for("the configure sizes to settle", || {
        client.roundtrip();
        let sizes: Vec<(i32, i32)> = windows
            .iter()
            .map(|window| client.configured_size(window).expect("configured"))
            .collect();
        (sizes == previous).then_some(sizes.clone()).or_else(|| {
            previous = sizes;
            None
        })
    })
}

/// Waits until the configure sizes equal `expected`. `settled_sizes` alone would return
/// the stale sizes straight after a layout change, before the new arrangement is applied.
fn wait_for_sizes(client: &mut Client, windows: &[Window; 3], expected: &[(i32, i32)]) {
    wait_for("the configure sizes to reach the expected arrangement", || {
        client.roundtrip();
        let sizes: Vec<(i32, i32)> = windows
            .iter()
            .map(|window| client.configured_size(window).expect("configured"))
            .collect();
        (sizes == expected).then_some(())
    });
}

/// The frame rectangle `window get` publishes for the window with `id`: `x`, `y`, `width`
/// and `height`, in logical pixels.
fn published_rect(session: &Session, id: u64) -> [i64; 4] {
    ["x", "y", "width", "height"].map(|property| {
        let value = session.ctl(&["window", "get", &id.to_string(), property]).ok();
        value
            .out
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("`{property}` is a number, got {:?}", value.out))
    })
}

/// The published rectangle of each window, once two reads in a row agree.
fn settled_rects(session: &Session, ids: &[u64; 3]) -> Vec<[i64; 4]> {
    let mut previous = Vec::new();
    wait_for("the published rectangles to settle", || {
        let rects: Vec<[i64; 4]> = ids.iter().map(|&id| published_rect(session, id)).collect();
        (rects == previous).then_some(rects.clone()).or_else(|| {
            previous = rects;
            None
        })
    })
}

#[test]
fn master_stack_tiles_three_toplevels_on_the_headless_output() {
    let session = session_with(LayoutSettings {
        default: named("gnoblin.layouts.master_stack"),
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    // Master left half at full height; the two stack windows share the right half.
    let expected = vec![(512, 768), (512, 384), (512, 384)];
    assert_eq!(settled_sizes(&mut client, &windows), expected);
}

#[test]
fn an_unknown_layout_is_refused_and_its_workspaces_float() {
    let floating = {
        let session = session_with(LayoutSettings::default());
        let mut client = Client::connect(session.wayland());
        let windows = map_three(&mut client);
        settled_sizes(&mut client, &windows)
    };

    let session = session_with(LayoutSettings {
        default: named("gnoblin.layouts.no_such_layout"),
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    // A refused name runs exactly as the default floating session does.
    assert_eq!(settled_sizes(&mut client, &windows), floating);
}

#[test]
fn a_module_that_errors_falls_back_and_the_session_keeps_running() {
    let modules = tempfile::tempdir().expect("a module directory");
    let layouts = modules.path().join("layouts");
    std::fs::create_dir_all(&layouts).expect("the layouts directory");
    std::fs::write(
        layouts.join("broken.lua"),
        "local M = {}\nM.kind = 'tiling'\nM.options = {}\nfunction M.arrange(_ctx) error(\"broken on purpose\") end\n\
         function M.on_request(_ctx) return nil end\nreturn M\n",
    )
    .expect("the broken module");

    let floating = {
        let session = session_with(LayoutSettings::default());
        let mut client = Client::connect(session.wayland());
        let windows = map_three(&mut client);
        settled_sizes(&mut client, &windows)
    };

    let session = session_with(LayoutSettings {
        default: named("layouts.broken"),
        config_dir: Some(modules.path().to_path_buf()),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    // The arrange raises, so the wm applies its own fallback; every window is still
    // configured and the session still answers a new client.
    assert_eq!(settled_sizes(&mut client, &windows), floating);
    let mut late = Client::connect(session.wayland());
    let _ = late.map_window("org.example.late", "late");
}

#[test]
fn an_output_override_applies_to_the_workspaces_on_that_output() {
    // The headless output is HEADLESS-1 and the workspace has no override of its own,
    // so the output's layout arranges it.
    let session = session_with(LayoutSettings {
        outputs: vec![("HEADLESS-1".to_owned(), named("gnoblin.layouts.master_stack"))],
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    assert_eq!(
        settled_sizes(&mut client, &windows),
        vec![(512, 768), (512, 384), (512, 384)]
    );
}

#[test]
fn a_workspace_override_wins_over_the_output_override() {
    // Workspace 1 is floating, so the output's master_stack must not arrange it.
    let floating = {
        let session = session_with(LayoutSettings::default());
        let mut client = Client::connect(session.wayland());
        let windows = map_three(&mut client);
        settled_sizes(&mut client, &windows)
    };

    let session = session_with(LayoutSettings {
        outputs: vec![("HEADLESS-1".to_owned(), named("gnoblin.layouts.master_stack"))],
        workspaces: vec![("1".to_owned(), named("floating"))],
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    assert_eq!(settled_sizes(&mut client, &windows), floating);
}

/// The id of the first workspace `workspace list` prints. A session always has one.
fn workspace_id(session: &Session) -> u64 {
    wait_for("a workspace to be listed", || {
        let listed = session.ctl(&["workspace", "list"]).ok();
        listed
            .out
            .lines()
            .find_map(|line| line.split('\t').next()?.parse().ok())
    })
}

#[test]
fn set_layout_retiles_the_windows_on_the_workspace() {
    let session = session_with(LayoutSettings {
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    let floating = settled_sizes(&mut client, &windows);

    let workspace = workspace_id(&session);
    session
        .ctl(&[
            "workspace",
            "call",
            &workspace.to_string(),
            "set_layout",
            "root=shipped",
            "module=gnoblin.layouts.master_stack",
            &format!("hash={}", master_stack_hash()),
            "options=",
        ])
        .ok();
    wait_for_sizes(&mut client, &windows, &[(512, 768), (512, 384), (512, 384)]);
    assert_ne!(floating, vec![(512, 768), (512, 384), (512, 384)]);
}

#[test]
fn master_stack_publishes_the_rectangles_it_placed() {
    let session = session_with(LayoutSettings {
        default: named("gnoblin.layouts.master_stack"),
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    settled_sizes(&mut client, &windows);
    let ids = [
        wait_for_listed_id(&session, "one"),
        wait_for_listed_id(&session, "two"),
        wait_for_listed_id(&session, "three"),
    ];

    // The IPC rectangle is the placement, not the cascade a window was opened at.
    let expected = vec![[0, 0, 512, 768], [512, 0, 512, 384], [512, 384, 512, 384]];
    assert_eq!(settled_rects(&session, &ids), expected);
}

#[test]
fn set_layout_publishes_the_retiled_rectangles() {
    let session = session_with(LayoutSettings {
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    settled_sizes(&mut client, &windows);
    let ids = [
        wait_for_listed_id(&session, "one"),
        wait_for_listed_id(&session, "two"),
        wait_for_listed_id(&session, "three"),
    ];

    let workspace = workspace_id(&session);
    session
        .ctl(&[
            "workspace",
            "call",
            &workspace.to_string(),
            "set_layout",
            "root=shipped",
            "module=gnoblin.layouts.master_stack",
            &format!("hash={}", master_stack_hash()),
            "options=",
        ])
        .ok();
    wait_for_sizes(&mut client, &windows, &[(512, 768), (512, 384), (512, 384)]);

    let expected = vec![[0, 0, 512, 768], [512, 0, 512, 384], [512, 384, 512, 384]];
    assert_eq!(settled_rects(&session, &ids), expected);
}

#[test]
fn layout_command_is_delivered_to_the_layout_that_answers_it() {
    // The `fill` command places every window at the size its parameters name, so the
    // configure sizes show the command reached this module with those parameters.
    let modules = tempfile::tempdir().expect("a module directory");
    let layouts = modules.path().join("layouts");
    std::fs::create_dir_all(&layouts).expect("the layouts directory");
    std::fs::write(
        layouts.join("probe.lua"),
        r#"local M = {}
M.kind = "tiling"
M.options = {}
function M.arrange(_ctx) return { placements = {} } end
function M.on_request(_ctx) return nil end
M.command = {}
function M.command.fill(ctx, args)
  local placements = {}
  for i, window in ipairs(ctx.windows) do
    placements[i] = { window = window.id, x = 0, y = 0, width = args.width, height = args.height }
  end
  return { placements = placements }
end
return M
"#,
    )
    .expect("the probe module");

    let session = session_with(LayoutSettings {
        config_dir: Some(modules.path().to_path_buf()),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    settled_sizes(&mut client, &windows);

    let workspace = workspace_id(&session).to_string();
    session
        .ctl(&[
            "workspace",
            "call",
            &workspace,
            "set_layout",
            "root=user",
            "module=layouts.probe",
            &format!("hash={}", source_hash(&layouts.join("probe.lua"))),
            "options=",
        ])
        .ok();
    session
        .ctl(&[
            "workspace",
            "call",
            &workspace,
            "layout_command",
            "name=fill",
            "params={\"width\":300,\"height\":200}",
        ])
        .ok();

    wait_for_sizes(&mut client, &windows, &[(300, 200); 3]);
}

#[test]
fn an_unknown_layout_name_is_refused_over_ipc() {
    let session = session_with(LayoutSettings {
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let workspace = workspace_id(&session).to_string();

    let refusal = session.ctl(&[
        "workspace",
        "call",
        &workspace,
        "set_layout",
        "root=shipped",
        "module=gnoblin.layouts.no_such_layout",
        "hash=0000000000000000",
        "options=",
    ]);
    assert_ne!(refusal.code, 0, "an unknown layout is refused");
    assert!(
        refusal.out.contains("unknown-layout") || refusal.err.contains("unknown-layout"),
        "the refusal names unknown-layout: out {:?}, err {:?}",
        refusal.out,
        refusal.err
    );
}

#[test]
fn strip_commands_are_applied_over_ipc() {
    // Every command answers with a reply table, so the IPC call is applied (exit 0) and
    // the placements it returns reach the windows. A nil return would be `refused`.
    let session = session_with(LayoutSettings {
        shipped_dir: shipped_modules(),
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    settled_sizes(&mut client, &windows);

    let workspace = workspace_id(&session).to_string();
    let strip = source_hash(&shipped_modules().join("gnoblin/layouts/strip.lua"));
    session
        .ctl(&[
            "workspace",
            "call",
            &workspace,
            "set_layout",
            "root=shipped",
            "module=gnoblin.layouts.strip",
            &format!("hash={strip}"),
            "options=",
        ])
        .ok();
    let workspace_call = |name: &str, params: &str| {
        session
            .ctl(&[
                "workspace",
                "call",
                &workspace,
                "layout_command",
                &format!("name={name}"),
                params,
            ])
            .ok();
    };
    // `width` sets one column to 300 px, so one window is then 300 px wide.
    workspace_call("width", "params={\"px\":300}");
    wait_for("one column to take the 300 px width", || {
        client.roundtrip();
        windows
            .iter()
            .any(|window| client.configured_size(window).is_some_and(|(width, _)| width == 300))
            .then_some(())
    });
    for (name, params) in [
        ("move_column", "params={\"direction\":\"left\"}"),
        ("consume", "params={}"),
        ("expel", "params={}"),
        ("toggle_tabbed", "params={}"),
        ("fit_all", "params={}"),
        ("scroll_to", "params={\"x\":10}"),
    ] {
        workspace_call(name, params);
    }
}

#[test]
fn a_failing_layout_emits_layout_failed_and_its_windows_float() {
    let floating = {
        let session = session_with(LayoutSettings::default());
        let mut client = Client::connect(session.wayland());
        let windows = map_three(&mut client);
        settled_sizes(&mut client, &windows)
    };

    let modules = tempfile::tempdir().expect("a module directory");
    let layouts = modules.path().join("layouts");
    std::fs::create_dir_all(&layouts).expect("the layouts directory");
    std::fs::write(
        layouts.join("broken.lua"),
        "local M = {}\nM.kind = 'tiling'\nM.options = {}\nfunction M.arrange(_ctx) error(\"broken on purpose\") end\n\
         function M.on_request(_ctx) return nil end\nreturn M\n",
    )
    .expect("the broken module");

    let session = session_with(LayoutSettings {
        default: named("layouts.broken"),
        config_dir: Some(modules.path().to_path_buf()),
        ..LayoutSettings::default()
    });
    // Watch the workspace before any window maps: the failing run happens on the first
    // placement, and a watch subscribes only from its own start.
    let workspace = workspace_id(&session);
    let socket = session.socket().to_path_buf();
    let watcher = std::thread::spawn(move || {
        Command::run(&socket, &["workspace", "watch", &workspace.to_string(), "--count", "1"])
    });
    std::thread::sleep(Duration::from_millis(200));

    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    let watched = watcher.join().expect("the watch thread finishes");
    assert!(
        watched.out.contains("layout_failed"),
        "the workspace emits layout_failed: {:?}",
        watched.out
    );
    assert_eq!(settled_sizes(&mut client, &windows), floating);
}
