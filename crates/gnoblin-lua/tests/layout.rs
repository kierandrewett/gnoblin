//! Integration tests for the Lua layout host: the end-to-end path from a Lua layout
//! to settled placements, then each failure mode listed in `src/layout/mod.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_lua::layout::{
    HostConfig, Data, Gaps, Host, LayoutOutcome, OutputInfo, LayoutReason, LayoutRequest, LayoutSnapshot, WedgeHook, SnapshotWindow,
};
use gnoblin_types::{OutputId, RectF, Scale, WindowId, WorkspaceId};
use gnoblin_wm::layout::{Layout, Origin, Transform3};

/// A fresh module directory holding the given `name -> source` modules.
fn modules(test: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gnoblin-layout-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a module directory");
    for (name, source) in files {
        fs::write(dir.join(format!("{name}.lua")), source).expect("a module file");
    }
    dir
}

fn snapshot(ids: &[u64], width: f64, height: f64) -> LayoutSnapshot {
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
            width,
            height,
        },
        windows: ids
            .iter()
            .map(|id| SnapshotWindow {
                id: WindowId::new(*id),
                app_id: "app".into(),
                title: format!("window {id}"),
                width: 100.0,
                height: 100.0,
                size_hints: Default::default(),
            })
            .collect(),
        focus: ids.first().map(|id| WindowId::new(*id)),
        gaps: Gaps::default(),
    }
}

fn host(dir: PathBuf) -> Host {
    Host::new(HostConfig::new(vec![dir]))
}

fn no_options() -> Data {
    Data::Table(BTreeMap::new())
}

/// The master-stack layout: the first window takes the left half, the rest stack down
/// the right half. A place request for window 99 is declined, so the fallback applies.
const MASTER_STACK: &str = r#"
local M = {}
function M.arrange(ctx)
  local a = ctx.area
  local n = #ctx.windows
  local placements = {}
  if n == 0 then return { placements = placements } end
  local master_w = n == 1 and a.width or a.width / 2
  placements[1] = { window = ctx.windows[1].id, x = a.x, y = a.y, width = master_w, height = a.height }
  local rest = n - 1
  for i = 2, n do
    local slot = i - 2
    local h = a.height / rest
    placements[i] = {
      window = ctx.windows[i].id,
      x = a.x + a.width / 2,
      y = a.y + slot * h,
      width = a.width / 2,
      height = h,
    }
  end
  return { placements = placements }
end
function M.on_added(ctx, event)
  if event.window == 99 then return nil end
  return M.arrange(ctx)
end
return M
"#;

const COUNTER: &str = r#"
local M = {}
function M.arrange(ctx)
  local n = (ctx.state and ctx.state.n or 0) + 1
  ctx.state = { n = n }
  return { placements = { { window = ctx.windows[1].id, x = n, y = 0, width = 10, height = 10 } } }
end
return M
"#;

fn placed(outcome: LayoutOutcome) -> Layout {
    match outcome {
        LayoutOutcome::Placed(layout) => layout,
        LayoutOutcome::Failed { reason, message, .. } => panic!("expected a placement, got {reason:?}: {message}"),
    }
}

fn failed(outcome: LayoutOutcome) -> (LayoutReason, String) {
    match outcome {
        LayoutOutcome::Failed { reason, message, .. } => (reason, message),
        LayoutOutcome::Placed(layout) => panic!("expected a failure, got {layout:?}"),
    }
}

/// Checks placements against `(window, x, y, width, height)` rows, in window order.
fn expect_geometry(layout: &Layout, expected: &[(u64, f64, f64, f64, f64)]) {
    assert_eq!(layout.placements.len(), expected.len(), "placement count");
    for (placement, (id, x, y, width, height)) in layout.placements.iter().zip(expected) {
        assert_eq!(placement.window, WindowId::new(*id), "window order");
        assert_eq!(placement.transform, Transform3::at(*x, *y), "window {id} position");
        assert_eq!(placement.width, *width, "window {id} width");
        assert_eq!(placement.height, *height, "window {id} height");
        assert!(placement.visible);
    }
}

#[test]
fn e2e_master_stack_places_windows_and_falls_through_when_declined() {
    let mut host = host(modules("master", &[("master", MASTER_STACK)]));
    host.register("master", "master", no_options())
        .expect("the layout registers");

    // Three windows on a 1000x600 area: master left half, two stacked on the right.
    let layout = placed(host.arrange("master", &snapshot(&[1, 2, 3], 1000.0, 600.0)));
    expect_geometry(
        &layout,
        &[
            (1, 0.0, 0.0, 500.0, 600.0),
            (2, 500.0, 0.0, 500.0, 300.0),
            (3, 500.0, 300.0, 500.0, 300.0),
        ],
    );
    assert_eq!(layout.origin, Origin::Reply);

    // A declined place request yields the built-in fallback, not a failure.
    let declined = placed(host.message(
        "master",
        LayoutRequest::Place {
            window: WindowId::new(99),
        },
        &snapshot(&[99], 1000.0, 600.0),
    ));
    assert_eq!(declined.origin, Origin::Fallback);

    // A handled request yields the layout's placement.
    let handled = placed(host.message(
        "master",
        LayoutRequest::Place {
            window: WindowId::new(1),
        },
        &snapshot(&[1], 1000.0, 600.0),
    ));
    assert_eq!(handled.origin, Origin::Reply);
    expect_geometry(&handled, &[(1, 0.0, 0.0, 1000.0, 600.0)]);
}

#[test]
fn state_persists_across_calls_and_is_fresh_per_host() {
    let snap = snapshot(&[7], 100.0, 100.0);
    let mut counter_host = host(modules("state", &[("counter", COUNTER)]));
    counter_host
        .register("counter", "counter", no_options())
        .expect("registers");
    let first = placed(counter_host.arrange("counter", &snap));
    let second = placed(counter_host.arrange("counter", &snap));
    assert_eq!(first.placements[0].transform, Transform3::at(1.0, 0.0));
    assert_eq!(second.placements[0].transform, Transform3::at(2.0, 0.0));

    let mut fresh = host(modules("state-fresh", &[("counter", COUNTER)]));
    fresh.register("counter", "counter", no_options()).expect("registers");
    let again = placed(fresh.arrange("counter", &snap));
    assert_eq!(again.placements[0].transform, Transform3::at(1.0, 0.0));
}

#[test]
fn spinning_layout_is_aborted_by_the_instruction_budget() {
    let spin = "local M = {} function M.arrange() while true do end end return M";
    let mut host = host(modules("spin", &[("spin", spin)]));
    host.register("spin", "spin", no_options()).expect("registers");
    let (reason, _) = failed(host.arrange("spin", &snapshot(&[1], 100.0, 100.0)));
    assert!(matches!(reason, LayoutReason::Stalled | LayoutReason::Deadline), "{reason:?}");
}

#[test]
fn memory_bomb_is_refused() {
    let bomb = "local M = {} function M.arrange() local t = {} for i = 1, 1e12 do t[i] = { i, i, i, i } end return nil end return M";
    let mut host = host(modules("bomb", &[("bomb", bomb)]));
    host.register("bomb", "bomb", no_options()).expect("registers");
    let (reason, message) = failed(host.arrange("bomb", &snapshot(&[1], 100.0, 100.0)));
    assert!(
        matches!(
            reason,
            LayoutReason::Memory | LayoutReason::Error | LayoutReason::Stalled | LayoutReason::Deadline
        ),
        "{reason:?}: {message}"
    );
}

#[test]
fn bad_returns_are_refused() {
    let cases = [
        (
            "nan",
            "local M={} function M.arrange(ctx) return {placements={{window=ctx.windows[1].id,x=0,y=0,width=0/0,height=10}}} end return M",
        ),
        (
            "unknown",
            "local M={} function M.arrange(ctx) return {placements={{window=12345,x=0,y=0,width=10,height=10}}} end return M",
        ),
        (
            "wrong_type",
            "local M={} function M.arrange(ctx) return 42 end return M",
        ),
    ];
    for (name, source) in cases {
        let mut host = host(modules(name, &[(name, source)]));
        host.register(name, name, no_options()).expect("registers");
        let (reason, _) = failed(host.arrange(name, &snapshot(&[1], 100.0, 100.0)));
        assert!(
            matches!(reason, LayoutReason::RefusedReply | LayoutReason::InvalidLayout | LayoutReason::Error),
            "{name}: {reason:?}"
        );
    }
}

#[test]
fn thrown_error_is_reported_with_its_message() {
    let boom = r#"local M={} function M.arrange() error("boom") end return M"#;
    let mut host = host(modules("boom", &[("boom", boom)]));
    host.register("boom", "boom", no_options()).expect("registers");
    let (reason, message) = failed(host.arrange("boom", &snapshot(&[1], 100.0, 100.0)));
    assert_eq!(reason, LayoutReason::Error);
    assert!(message.contains("boom"), "{message}");
}

#[test]
fn require_is_sandboxed_to_the_configured_directory() {
    let escape = r#"local M = require("../escape") return M"#;
    let mut host = host(modules("sandbox", &[("escape", escape)]));
    assert!(host.register("escape", "escape", no_options()).is_err());
    assert!(host.register("traversal", "../../etc/passwd", no_options()).is_err());
}

#[test]
fn duplicate_register_is_refused_and_previous_registration_survives() {
    let mut host = host(modules("dup", &[("master", MASTER_STACK)]));
    host.register("master", "master", no_options()).expect("registers");
    let err = host.register("master", "missing", no_options());
    assert!(err.is_err(), "a failed re-register must fail");
    let layout = placed(host.arrange("master", &snapshot(&[1, 2], 1000.0, 600.0)));
    expect_geometry(&layout, &[(1, 0.0, 0.0, 500.0, 600.0), (2, 500.0, 0.0, 500.0, 600.0)]);
}

#[test]
fn wedged_worker_is_abandoned_and_the_leak_cap_disables_layouts() {
    let wedge: WedgeHook = Arc::new(|name: &str| name == "wedged");
    let mut config = HostConfig::new(vec![modules(
        "wedge",
        &[("wedged", MASTER_STACK), ("fine", MASTER_STACK)],
    )]);
    config.wedge = Some(wedge);
    config.hard_limit = Duration::from_millis(50);
    let mut host = Host::new(config);
    host.register("wedged", "wedged", no_options()).expect("registers");
    host.register("fine", "fine", no_options()).expect("registers");
    let snap = snapshot(&[1], 100.0, 100.0);
    // A wedge is abandoned by the first call that starts a hard limit after the stall.
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(60));
        let (reason, _) = failed(host.arrange("wedged", &snap));
        assert!(matches!(reason, LayoutReason::VmWedged | LayoutReason::Deadline), "{reason:?}");
    }
    assert!(host.disabled(), "three abandoned workers disable layouts");
    assert!(host.leaked() >= 3);
    let (reason, _) = failed(host.arrange("fine", &snap));
    assert_eq!(reason, LayoutReason::VmWedged);
}
