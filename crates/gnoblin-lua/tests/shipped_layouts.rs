//! Integration tests for the shipped layout modules in `src/data/lua/gnoblin/layouts/`.
//!
//! Every test loads a shipped file through the real `Host` and `LuaEngine`, then settles
//! the reply with the wm validator, the same path `tests/layout.rs` uses.
//!
//! Failure modes, each checked by the tests named in brackets:
//! - zero windows gives no placements and no error [every_layout_settles_for_every_count_and_area]
//! - one window fills the area for kiosk and monocle [every_layout_settles_for_every_count_and_area]
//! - a tiny area (3x2) and large counts (20) keep every placement finite [every_layout_settles_for_every_count_and_area]
//! - tiling layouts (master_stack, dwindle, grid) place every window inside the area with no overlap [every_layout_settles_for_every_count_and_area]
//! - floating places nothing and floats every window; no window is both placed and floated [every_layout_settles_for_every_count_and_area]
//! - kiosk and monocle show one window and hide the rest [every_layout_settles_for_every_count_and_area]
//! - the same input on a fresh host gives the same settled layout [every_layout_settles_for_every_count_and_area]
//! - removing the focused or only window, adding windows and re-adding windows keep state stable [add_and_remove_sequences_keep_every_window_placed]
//! - scrolling columns may lie outside the area, and carousel3d uses depth, so bounds are not checked for them [every_layout_settles_for_every_count_and_area]
//! - commands mutate state and change geometry as documented [command_tests]
//! - the loader resolves `gnoblin.layouts.<name>` from the installed prefix alone [installed_layouts_load_from_their_prefix]
//! - the CMake install rule (cmake/install-lua-data.cmake) copies the same tree that the loader then resolves [cmake_install_rule_installs_shipped_layouts]
//!
//! Commands are plain Lua functions the host does not dispatch. Command tests therefore
//! generate a driver module that runs a command and then arranges through the real module.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gnoblin_lua::layout::{
    Data, Gaps, Host, HostConfig, LayoutSnapshot, LuaEngine, OutputInfo, default_prefix, installed_module_dir,
};
use gnoblin_types::{OutputId, RectF, Scale, Vec3, WindowId, WorkspaceId};
use gnoblin_wm::layout::{
    Camera, Generation, Job, Layout, LayoutEngine, LayoutFailure, LayoutOutcome, LayoutWindow, Params, Placement,
    Reply, Transform3, settle,
};

const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/data/lua");
const LAYOUTS: [&str; 8] = [
    "floating",
    "kiosk",
    "monocle",
    "master_stack",
    "dwindle",
    "grid",
    "scrolling",
    "carousel3d",
];
const COUNTS: [usize; 6] = [0, 1, 2, 3, 7, 20];
const AREAS: [(f64, f64); 3] = [(1920.0, 1080.0), (640.0, 480.0), (3.0, 2.0)];
const EPS: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn rect(placement: &Placement) -> Rect {
    match &placement.transform {
        Transform3::Trs { position, .. } => Rect {
            x: position.x,
            y: position.y,
            w: placement.width,
            h: placement.height,
        },
        Transform3::Matrix(_) => panic!("shipped layouts place with Trs transforms: {placement:?}"),
    }
}

fn by_id(layout: &Layout) -> BTreeMap<u64, Placement> {
    layout
        .placements
        .iter()
        .map(|placement| (window_key(placement.window), *placement))
        .collect()
}

fn window_key(id: WindowId) -> u64 {
    id.get()
}

fn ids(count: usize) -> Vec<u64> {
    (1..=count as u64).collect()
}

fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gnoblin-shipped-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn write_module(dir: &Path, name: &str, source: &str) {
    fs::write(dir.join(format!("{name}.lua")), source).expect("a module file");
}

/// A host with `dirs` searched in order and `layouts` registered as `(name, module)`.
fn hosted(dirs: Vec<PathBuf>, layouts: &[(&str, &str)]) -> Rc<RefCell<Host>> {
    let host = Rc::new(RefCell::new(Host::new(HostConfig::new(dirs))));
    for (name, module) in layouts {
        host.borrow_mut()
            .register(name, module, Data::Table(BTreeMap::new()))
            .expect("the shipped layout loads");
    }
    host
}

fn shipped(layout: &str) -> Rc<RefCell<Host>> {
    let module = format!("gnoblin.layouts.{layout}");
    hosted(vec![PathBuf::from(SHIPPED)], &[(layout, module.as_str())])
}

fn snapshot(focus: Option<u64>) -> LayoutSnapshot {
    LayoutSnapshot {
        workspace: WorkspaceId::new(1),
        output: OutputInfo {
            id: OutputId::new(1),
            name: "test-output".into(),
            scale: Scale::integral(1),
        },
        area: RectF {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        windows: Vec::new(),
        focus: focus.map(WindowId::new),
        gaps: Gaps::default(),
    }
}

fn params(ids: &[u64], width: f64, height: f64) -> Params {
    let windows = ids
        .iter()
        .map(|id| LayoutWindow {
            id: WindowId::new(*id),
            width: 100.0,
            height: 100.0,
        })
        .collect();
    Params::new(width, height, windows).expect("valid params")
}

/// Arranges `ids` in a `width` by `height` output through `layout` on `host`.
fn arrange(host: &Rc<RefCell<Host>>, layout: &str, focus: Option<u64>, ids: &[u64], width: f64, height: f64) -> Layout {
    let mut engine = LuaEngine::new(Rc::clone(host), layout, snapshot(focus));
    let _ = engine.submit(Job::Arrange(params(ids, width, height)), Generation::FIRST);
    let reply = match engine.collect(Generation::FIRST, Instant::now() + Duration::from_secs(5)) {
        LayoutOutcome::Ready { generation, result } => {
            assert_eq!(generation, Generation::FIRST, "the answer names its generation");
            result
        }
        LayoutOutcome::Pending => panic!("no answer within 5s"),
    };
    let reply: Result<Reply, LayoutFailure> = reply;
    settle(&params(ids, width, height), reply.expect("the layout answered")).expect("the reply passes the wm validator")
}

fn finite(value: f64) -> bool {
    value.is_finite()
}

fn vec_finite(v: &Vec3) -> bool {
    finite(v.x) && finite(v.y) && finite(v.z)
}

fn assert_finite(layout: &Layout) {
    for placement in &layout.placements {
        let r = rect(placement);
        assert!(
            finite(r.x) && finite(r.y) && finite(r.w) && finite(r.h) && finite(placement.opacity),
            "{placement:?}"
        );
        assert!(placement.width >= 0.0 && placement.height >= 0.0, "{placement:?}");
        if let Transform3::Trs {
            rotation_degrees,
            scale,
            ..
        } = &placement.transform
        {
            assert!(vec_finite(rotation_degrees) && vec_finite(scale), "{placement:?}");
        }
        if let Camera::Perspective { fov_degrees, .. } = layout.camera {
            assert!(finite(fov_degrees), "{:?}", layout.camera);
        }
    }
}

fn inside(r: Rect, width: f64, height: f64) -> bool {
    r.x >= -EPS && r.y >= -EPS && r.x + r.w <= width + EPS && r.y + r.h <= height + EPS
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w - EPS && b.x < a.x + a.w - EPS && a.y < b.y + b.h - EPS && b.y < a.y + a.h - EPS
}

fn placed_ids(layout: &Layout) -> Vec<u64> {
    let mut out: Vec<u64> = layout.placements.iter().map(|p| window_key(p.window)).collect();
    out.sort_unstable();
    out
}

/// The per-layout invariants, given the window ids and the output size.
fn check_layout(layout: &str, ids: &[u64], width: f64, height: f64, settled: &Layout) {
    assert_finite(settled);
    let n = ids.len();
    match layout {
        "floating" => {
            assert!(settled.placements.is_empty(), "floating places nothing");
            let floated: Vec<u64> = settled.effects.iter().map(|effect| effect.window().get()).collect();
            let mut sorted = floated.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, ids, "every window is floated");
        }
        "kiosk" | "monocle" => {
            assert_eq!(placed_ids(settled), ids, "{layout} places every window");
            let visible: Vec<&Placement> = settled.placements.iter().filter(|p| p.visible).collect();
            assert_eq!(visible.len(), n.min(1), "{layout} shows one window");
            for placement in &settled.placements {
                if placement.visible {
                    assert_eq!(
                        rect(placement),
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            w: width,
                            h: height
                        },
                        "{layout} fills the area"
                    );
                }
            }
        }
        "master_stack" | "dwindle" | "grid" => {
            assert_eq!(placed_ids(settled), ids, "{layout} places every window");
            let rects: Vec<Rect> = settled.placements.iter().map(rect).collect();
            for (i, r) in rects.iter().enumerate() {
                assert!(settled.placements[i].visible, "{layout} shows every window");
                assert!(
                    inside(*r, width, height),
                    "{layout} keeps {r:?} inside {width}x{height}"
                );
                for other in &rects[i + 1..] {
                    assert!(!overlaps(*r, *other), "{layout} windows overlap: {r:?} {other:?}");
                }
            }
        }
        "scrolling" | "carousel3d" => {
            assert_eq!(placed_ids(settled), ids, "{layout} places every window");
        }
        other => panic!("no invariant for {other}"),
    }
}

#[test]
fn every_layout_settles_for_every_count_and_area() {
    for layout in LAYOUTS {
        for count in COUNTS {
            let ids = ids(count);
            for (width, height) in AREAS {
                let first = arrange(&shipped(layout), layout, None, &ids, width, height);
                check_layout(layout, &ids, width, height, &first);
                let again = arrange(&shipped(layout), layout, None, &ids, width, height);
                assert_eq!(
                    first, again,
                    "{layout}: {count} windows in {width}x{height} is deterministic"
                );
            }
        }
    }
}

#[test]
fn add_and_remove_sequences_keep_every_window_placed() {
    // Each step is a window set and the focus, which may name a window already removed.
    let steps: [(&[u64], Option<u64>); 7] = [
        (&[1, 2, 3, 4], Some(1)),
        (&[1, 3, 4], Some(2)),
        (&[1, 3], Some(3)),
        (&[1], Some(1)),
        (&[], None),
        (&[1, 2, 3, 4, 5], Some(5)),
        (&[2], None),
    ];
    for layout in LAYOUTS {
        // One host per layout: its state persists across steps. The engine fixes the focus
        // at construction, so each step gets its own engine on that host.
        let host = shipped(layout);
        for (ids, focus) in steps {
            let settled = arrange(&host, layout, focus, ids, 1000.0, 600.0);
            check_layout(layout, ids, 1000.0, 600.0, &settled);
        }
    }
}

/// A module that runs `body` against the real `layout`, then arranges through it.
fn driver(test: &str, layout: &str, body: &str) -> PathBuf {
    let dir = scratch_dir(test);
    let source = format!(
        "local real = require(\"gnoblin.layouts.{layout}\")\n\
         local M = {{}}\n\
         M.options = real.options\n\
         function M.arrange(ctx)\n\
         {body}\n\
         \x20 return real.arrange(ctx)\n\
         end\n\
         return M\n"
    );
    write_module(&dir, test, &source);
    dir
}

/// Arranges `ids` through the driver module for `test` on the 1000 by 600 output.
fn driven(test: &str, layout: &str, body: &str, focus: Option<u64>, ids: &[u64]) -> Layout {
    let dir = driver(test, layout, body);
    let host = hosted(vec![dir, PathBuf::from(SHIPPED)], &[(test, test)]);
    arrange(&host, test, focus, ids, 1000.0, 600.0)
}

fn baseline(layout: &str, focus: Option<u64>, ids: &[u64]) -> Layout {
    arrange(&shipped(layout), layout, focus, ids, 1000.0, 600.0)
}

#[test]
fn command_tests() {
    // master_stack: inc gives two master windows; they share the left half.
    let default = baseline("master_stack", None, &[1, 2, 3]);
    assert!((rect(&by_id(&default)[&2]).x - 500.0).abs() < EPS);
    let inc = driven(
        "drv_ms_inc",
        "master_stack",
        "  real.command.inc(ctx, {})",
        None,
        &[1, 2, 3],
    );
    assert!(rect(&by_id(&inc)[&2]).x.abs() < EPS, "two masters on the left");

    // master_stack: ratio grows the master region by the delta.
    let ratio = driven(
        "drv_ms_ratio",
        "master_stack",
        "  real.command.ratio(ctx, {delta = 0.2})",
        None,
        &[1, 2, 3],
    );
    assert!(
        (rect(&by_id(&ratio)[&1]).w - 700.0).abs() < EPS,
        "master takes 0.7 of the width"
    );

    // master_stack: swap_master moves the named window to the master slot.
    let swap = driven(
        "drv_ms_swap",
        "master_stack",
        "  real.command.swap_master(ctx, {window = 3})",
        None,
        &[1, 2, 3],
    );
    assert_eq!(
        rect(&by_id(&swap)[&3]),
        rect(&by_id(&default)[&1]),
        "window 3 takes the master slot"
    );

    // dwindle: swap exchanges two windows' rectangles.
    let before = baseline("dwindle", Some(1), &[1, 2]);
    let after = driven(
        "drv_dw_swap",
        "dwindle",
        "  real.command.swap(ctx, {window = 2})",
        Some(1),
        &[1, 2],
    );
    assert_eq!(
        rect(&by_id(&after)[&2]),
        rect(&by_id(&before)[&1]),
        "window 2 takes window 1's rect"
    );
    assert_eq!(
        rect(&by_id(&after)[&1]),
        rect(&by_id(&before)[&2]),
        "window 1 takes window 2's rect"
    );

    // monocle: next shows the following window in stacking order.
    let mono_before = baseline("monocle", None, &[1, 2, 3]);
    let mono_after = driven(
        "drv_mono_next",
        "monocle",
        "  real.command.next(ctx, {})",
        None,
        &[1, 2, 3],
    );
    let shown = |layout: &Layout| placed_ids_visible(layout);
    assert_eq!(shown(&mono_before), vec![1], "monocle starts on the first window");
    assert_eq!(shown(&mono_after), vec![2], "next shows the second window");

    // scrolling: with focus 1 the columns read 1, 3, 2 (reconcile adds each new window
    // right of the focused column). consume pulls window 3 into window 1's column;
    // expel moves it back out, restoring the baseline columns.
    let scroll = baseline("scrolling", Some(1), &[1, 2, 3]);
    let consumed = driven(
        "drv_sc_consume",
        "scrolling",
        "  real.command.consume(ctx, {window = 3})",
        Some(1),
        &[1, 2, 3],
    );
    assert_eq!(
        rect(&by_id(&consumed)[&3]).x,
        rect(&by_id(&consumed)[&1]).x,
        "3 shares 1's column"
    );
    assert_ne!(
        rect(&by_id(&consumed)[&2]).x,
        rect(&by_id(&consumed)[&1]).x,
        "2 keeps its own column"
    );
    let expelled = driven(
        "drv_sc_expel",
        "scrolling",
        "  real.command.consume(ctx, {window = 3})\n  real.command.expel(ctx, {window = 3})",
        Some(1),
        &[1, 2, 3],
    );
    for id in [1, 2, 3] {
        assert_eq!(
            rect(&by_id(&expelled)[&id]),
            rect(&by_id(&scroll)[&id]),
            "expel restores window {id}"
        );
    }

    // scrolling: move_column right swaps focused column 1 with its right neighbour 3.
    let moved = driven(
        "drv_sc_move",
        "scrolling",
        "  real.command.move_column(ctx, {direction = \"right\"})",
        Some(1),
        &[1, 2, 3],
    );
    let (x1, x2, x3) = (
        rect(&by_id(&moved)[&1]).x,
        rect(&by_id(&moved)[&2]).x,
        rect(&by_id(&moved)[&3]).x,
    );
    assert!(x3 < x1 && x1 < x2, "columns read 3, 1, 2: {x3} {x1} {x2}");

    // scrolling: fit_all gives every column an equal share of the area.
    let fitted = driven(
        "drv_sc_fit",
        "scrolling",
        "  real.command.fit_all(ctx, {})",
        Some(1),
        &[1, 2, 3],
    );
    for id in [1, 2, 3] {
        assert!(
            (rect(&by_id(&fitted)[&id]).w - 1000.0 / 3.0).abs() < 1e-3,
            "window {id} takes a third"
        );
    }
}

fn placed_ids_visible(layout: &Layout) -> Vec<u64> {
    let mut out: Vec<u64> = layout
        .placements
        .iter()
        .filter(|p| p.visible)
        .map(|p| window_key(p.window))
        .collect();
    out.sort_unstable();
    out
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a directory in the prefix");
    for entry in fs::read_dir(from).expect("the source tree") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("a copied module");
        }
    }
}

#[test]
fn installed_layouts_load_from_their_prefix() {
    // Install the shipped tree the way the CMake install step does: src/data/lua/ to
    // share/gnoblin/lua under the prefix. The source tree is not on the module path.
    let prefix = scratch_dir("install-prefix");
    copy_tree(Path::new(SHIPPED), &installed_module_dir(&prefix));

    // Default resolution: the prefix comes from the given environment value.
    let resolved = default_prefix(Some(prefix.as_os_str()));
    assert_eq!(resolved, prefix);
    assert_eq!(default_prefix(None), PathBuf::from("/usr"));
    let dir = installed_module_dir(&resolved);
    assert_eq!(dir, prefix.join("share/gnoblin/lua"));

    let host = Rc::new(RefCell::new(Host::new(HostConfig::installed(&resolved))));
    for layout in LAYOUTS {
        let module = format!("gnoblin.layouts.{layout}");
        assert!(
            dir.join("gnoblin/layouts").join(format!("{layout}.lua")).is_file(),
            "{layout} is installed"
        );
        host.borrow_mut()
            .register(layout, &module, Data::Table(BTreeMap::new()))
            .unwrap_or_else(|failure| panic!("{layout} loads from the installed prefix: {failure:?}"));
        let settled = arrange(&host, layout, None, &ids(3), 1000.0, 600.0);
        check_layout(layout, &ids(3), 1000.0, 600.0, &settled);
    }
}

#[test]
fn installed_host_refuses_path_traversal() {
    let prefix = scratch_dir("install-traversal");
    copy_tree(Path::new(SHIPPED), &installed_module_dir(&prefix));
    let host = Rc::new(RefCell::new(Host::new(HostConfig::installed(&prefix))));
    for module in ["../escape", "../../etc/passwd"] {
        assert!(
            host.borrow_mut()
                .register("escape", module, Data::Table(BTreeMap::new()))
                .is_err(),
            "{module} is outside the installed directory"
        );
    }
}

/// Runs the real install rule, cmake/install-lua-data.cmake, in CMake script mode against a
/// temp prefix, then loads the result through `HostConfig::installed`. install-session.cmake
/// includes this same file, so the rule under test is the one the install step runs.
/// The test is skipped with a notice when no `cmake` binary is on PATH.
#[test]
fn cmake_install_rule_installs_shipped_layouts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rule = root.join("cmake/install-lua-data.cmake");
    let scratch = scratch_dir("cmake-install");
    let prefix = scratch.join("prefix");
    let wrapper = scratch.join("install.cmake");
    fs::write(
        &wrapper,
        format!(
            "set(ROOT \"{}\")\nset(INSTALL_PREFIX \"{}\")\ninclude(\"{}\")\n",
            root.display(),
            prefix.display(),
            rule.display()
        ),
    )
    .expect("a wrapper script");
    let output = match Command::new("cmake").arg("-P").arg(&wrapper).output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("cmake_install_rule_installs_shipped_layouts skipped: no cmake on PATH");
            return;
        }
        Err(error) => panic!("cmake runs: {error}"),
    };
    assert!(
        output.status.success(),
        "cmake install rule failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let installed = installed_module_dir(&prefix);
    assert_eq!(
        tree_files(&installed),
        tree_files(Path::new(SHIPPED)),
        "the install copies every shipped module"
    );
    let host = Rc::new(RefCell::new(Host::new(HostConfig::installed(&prefix))));
    for layout in LAYOUTS {
        host.borrow_mut()
            .register(
                layout,
                &format!("gnoblin.layouts.{layout}"),
                Data::Table(BTreeMap::new()),
            )
            .unwrap_or_else(|failure| panic!("{layout} loads from the cmake install: {failure:?}"));
        let settled = arrange(&host, layout, None, &ids(3), 1000.0, 600.0);
        check_layout(layout, &ids(3), 1000.0, 600.0, &settled);
    }
}

fn tree_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("a readable tree") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(path.strip_prefix(base).expect("a path under the tree").to_path_buf());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}
