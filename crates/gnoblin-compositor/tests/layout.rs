//! The `layout` section through the real compositor path: a headless session, a real
//! Wayland client, and the configure sizes the client receives.
//!
//! The headless output is 1024x768 and the layouts run with zero gaps, so the expected
//! sizes are the shipped `master_stack` geometry for that output: one master column on
//! the left half and two stack windows sharing the right half.

mod support;

use std::path::PathBuf;

use gnoblin_compositor::LayoutSettings;
use support::{Client, Session, Window, wait_for, wait_for_listed_id};

/// The repository's shipped Lua layouts, the directory an installed prefix provides.
fn shipped_modules() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/data/lua")
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

#[test]
fn master_stack_tiles_three_toplevels_on_the_headless_output() {
    let session = session_with(LayoutSettings {
        default: "master_stack".to_owned(),
        module_dirs: vec![shipped_modules()],
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
        default: "no_such_layout".to_owned(),
        module_dirs: vec![shipped_modules()],
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
    let layouts = modules.path().join("gnoblin/layouts");
    std::fs::create_dir_all(&layouts).expect("the layouts directory");
    std::fs::write(
        layouts.join("broken.lua"),
        "local M = {}\nM.options = {}\nfunction M.arrange(_ctx) error(\"broken on purpose\") end\n\
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
        default: "broken".to_owned(),
        module_dirs: vec![modules.path().to_path_buf()],
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
fn a_user_module_overrides_the_shipped_layout_of_the_same_name() {
    // Every window takes the whole area, which the shipped master_stack never does for
    // three windows. The user directory is listed first, so its module is the one that runs.
    let modules = tempfile::tempdir().expect("a module directory");
    let layouts = modules.path().join("gnoblin/layouts");
    std::fs::create_dir_all(&layouts).expect("the layouts directory");
    std::fs::write(
        layouts.join("master_stack.lua"),
        "local M = {}\n\
         function M.arrange(ctx)\n\
           local a = ctx.area\n\
           local placements = {}\n\
           for i, window in ipairs(ctx.windows) do\n\
             placements[i] = { window = window.id, x = a.x, y = a.y, width = a.width, height = a.height }\n\
           end\n\
           return { placements = placements }\n\
         end\n\
         function M.on_request(_ctx) return nil end\n\
         return M\n",
    )
    .expect("the user module");

    let session = session_with(LayoutSettings {
        default: "master_stack".to_owned(),
        module_dirs: vec![modules.path().to_path_buf(), shipped_modules()],
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    assert_eq!(settled_sizes(&mut client, &windows), vec![(1024, 768); 3]);
}

#[test]
fn an_output_override_applies_to_the_workspaces_on_that_output() {
    // The headless output is HEADLESS-1 and the workspace has no override of its own,
    // so the output's layout arranges it.
    let session = session_with(LayoutSettings {
        outputs: vec![("HEADLESS-1".to_owned(), "master_stack".to_owned())],
        module_dirs: vec![shipped_modules()],
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
        outputs: vec![("HEADLESS-1".to_owned(), "master_stack".to_owned())],
        workspaces: vec![("1".to_owned(), "floating".to_owned())],
        module_dirs: vec![shipped_modules()],
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);

    assert_eq!(settled_sizes(&mut client, &windows), floating);
}

#[test]
fn set_floating_overrides_the_workspace_layout() {
    let floating = {
        let session = session_with(LayoutSettings::default());
        let mut client = Client::connect(session.wayland());
        let windows = map_three(&mut client);
        settled_sizes(&mut client, &windows)
    };

    let session = session_with(LayoutSettings {
        default: "master_stack".to_owned(),
        module_dirs: vec![shipped_modules()],
        ..LayoutSettings::default()
    });
    let mut client = Client::connect(session.wayland());
    let windows = map_three(&mut client);
    assert_ne!(settled_sizes(&mut client, &windows), floating);

    for title in ["one", "two", "three"] {
        let id = wait_for_listed_id(&session, title);
        session
            .ctl(&["window", "call", &id.to_string(), "set_floating", "floating=true"])
            .ok();
    }
    assert_eq!(settled_sizes(&mut client, &windows), floating);
}
