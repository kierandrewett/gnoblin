//! Integration tests for the Lua layout host: the end-to-end path from a Lua config
//! chunk through `LuaEngine` and `Slot` to settled placements, then each failure mode
//! listed in `src/layout/mod.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_lua::layout::{Data, Gaps, Host, HostConfig, LuaEngine, WedgeHook, install, take_registrations};
use gnoblin_types::{OutputId, Scale, WindowId, WorkspaceId};
use gnoblin_wm::layout::{
    Collected, Generation, Job, Layout, LayoutEngine, LayoutExtent, LayoutFailure, LayoutOutcome, LayoutOutput,
    LayoutWindow, Params, Reply, Request, RequestKind, Slot, Transform3, settle,
};

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

fn host(dir: PathBuf) -> Rc<RefCell<Host>> {
    Rc::new(RefCell::new(Host::new(HostConfig::new(vec![dir]))))
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
    let output = LayoutOutput {
        id: OutputId::new(1),
        name: "test-output".into(),
        scale: Scale::integral(1),
    };
    Params::new(output, width, height, windows).expect("valid params")
}

fn engine(host: &Rc<RefCell<Host>>, layout: &str) -> LuaEngine {
    LuaEngine::new(Rc::clone(host), layout, WorkspaceId::new(1), Gaps::default())
}

/// Submits `job` as `generation` and collects it against a generous deadline.
fn run(engine: &mut LuaEngine, job: Job, generation: Generation) -> Result<Reply, LayoutFailure> {
    let _ = engine.submit(job, generation);
    match engine.collect(generation, Instant::now() + Duration::from_secs(5)) {
        LayoutOutcome::Ready {
            generation: answered,
            result,
        } => {
            assert_eq!(answered, generation, "the answer names its generation");
            result
        }
        LayoutOutcome::Pending => panic!("no answer within 5s"),
    }
}

fn arrange(engine: &mut LuaEngine, ids: &[u64], width: f64, height: f64) -> Result<Reply, LayoutFailure> {
    run(engine, Job::Arrange(params(ids, width, height)), Generation::FIRST)
}

/// Settles a reply the way the slot does, so every placement passes the wm validator.
fn settled(params: &Params, reply: Result<Reply, LayoutFailure>) -> Layout {
    let reply = reply.expect("the layout answered");
    settle(params, LayoutExtent::Output, reply).expect("the reply passes the wm validator")
}

fn no_options() -> Data {
    Data::Table(BTreeMap::new())
}

/// The master-stack layout: the first window takes the left half, the rest stack down
/// the right half. A request that includes window 99 is declined.
const MASTER_STACK: &str = r#"
local M = {}
M.kind = 'tiling'
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
function M.on_request(ctx)
  for _, window in ipairs(ctx.windows) do
    if window.id == 99 then return nil end
  end
  return M.arrange(ctx)
end
return M
"#;

const COUNTER: &str = r#"
local M = {}
M.kind = 'tiling'
function M.arrange(ctx)
  local n = (ctx.state and ctx.state.n or 0) + 1
  ctx.state = { n = n }
  return { placements = { { window = ctx.windows[1].id, x = n, y = 0, width = 10, height = 10 } } }
end
return M
"#;

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
fn e2e_config_registers_a_lua_layout_driven_through_the_slot() {
    // The configuration chunk registers the layout exactly as a user's config would.
    let lua = mlua::Lua::new();
    let gnoblin = lua.create_table().expect("a gnoblin table");
    install(&lua, &gnoblin).expect("the layout API installs");
    lua.globals().set("gnoblin", gnoblin).expect("the global");
    lua.load(r#"gnoblin.layout.register("master", "master")"#)
        .exec()
        .expect("the config chunk runs");
    let registrations = take_registrations(&lua).expect("registrations");
    assert_eq!(registrations.len(), 1);

    let dir = modules("e2e", &[("master", MASTER_STACK)]);
    let host = host(dir);
    for registration in &registrations {
        host.borrow_mut()
            .register(&registration.name, &registration.module, no_options())
            .expect("the registered layout loads");
    }

    // Three windows on a 1000x600 area: master left half, two stacked on the right.
    let mut slot = Slot::new(Box::new(engine(&host, "master")));
    let job_params = params(&[1, 2, 3], 1000.0, 600.0);
    slot.submit(Job::Arrange(job_params.clone()));
    let layout = match slot.collect(Instant::now() + Duration::from_secs(5)) {
        Collected::Settled(layout) => layout,
        other => panic!("expected a settled layout, got {other:?}"),
    };
    expect_geometry(
        &layout,
        &[
            (1, 0.0, 0.0, 500.0, 600.0),
            (2, 500.0, 0.0, 500.0, 300.0),
            (3, 500.0, 300.0, 500.0, 300.0),
        ],
    );

    // A declined request is observable from the engine as `Declined`, and the slot then
    // uses the built-in fallback.
    let mut engine = engine(&host, "master");
    let declined = run(
        &mut engine,
        Job::Request(Request {
            params: params(&[99], 1000.0, 600.0),
            kind: RequestKind::Place {
                window: WindowId::new(99),
            },
        }),
        Generation::FIRST,
    );
    assert!(matches!(declined, Ok(Reply::Declined)), "{declined:?}");

    // A handled request yields the layout's own placement.
    let handled_params = params(&[1], 1000.0, 600.0);
    let handled = run(
        &mut engine,
        Job::Request(Request {
            params: handled_params.clone(),
            kind: RequestKind::Place {
                window: WindowId::new(1),
            },
        }),
        Generation::FIRST.next(),
    );
    let layout = settled(&handled_params, handled);
    expect_geometry(&layout, &[(1, 0.0, 0.0, 1000.0, 600.0)]);
}

#[test]
fn state_persists_across_runs_and_is_fresh_per_host() {
    let counter_host = host(modules("state", &[("counter", COUNTER)]));
    counter_host
        .borrow_mut()
        .register("counter", "counter", no_options())
        .expect("registers");
    let mut counter = engine(&counter_host, "counter");
    let p = params(&[7], 100.0, 100.0);
    let first = settled(&p, run(&mut counter, Job::Arrange(p.clone()), Generation::FIRST));
    let second = settled(&p, run(&mut counter, Job::Arrange(p.clone()), Generation::FIRST.next()));
    assert_eq!(first.placements[0].transform, Transform3::at(1.0, 0.0));
    assert_eq!(second.placements[0].transform, Transform3::at(2.0, 0.0));

    let fresh = host(modules("state-fresh", &[("counter", COUNTER)]));
    fresh
        .borrow_mut()
        .register("counter", "counter", no_options())
        .expect("registers");
    let again = settled(
        &p,
        run(
            &mut engine(&fresh, "counter"),
            Job::Arrange(p.clone()),
            Generation::FIRST,
        ),
    );
    assert_eq!(again.placements[0].transform, Transform3::at(1.0, 0.0));
}

#[test]
fn identical_input_gives_identical_placements_on_fresh_hosts() {
    let p = params(&[1, 2, 3], 1000.0, 600.0);
    let mut results = Vec::new();
    for round in 0..2 {
        let h = host(modules(&format!("determinism-{round}"), &[("master", MASTER_STACK)]));
        h.borrow_mut()
            .register("master", "master", no_options())
            .expect("registers");
        results.push(run(
            &mut engine(&h, "master"),
            Job::Arrange(p.clone()),
            Generation::FIRST,
        ));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn stale_generation_is_never_answered_with_a_placement() {
    let h = host(modules("stale", &[("master", MASTER_STACK)]));
    h.borrow_mut()
        .register("master", "master", no_options())
        .expect("registers");
    let mut engine = engine(&h, "master");
    let p = params(&[1, 2], 1000.0, 600.0);
    let _ = engine.submit(Job::Arrange(p.clone()), Generation::FIRST);
    let _ = engine.submit(Job::Arrange(p.clone()), Generation::FIRST.next());

    let stale = engine.collect(Generation::FIRST, Instant::now() + Duration::from_secs(5));
    assert!(
        !matches!(stale, LayoutOutcome::Ready { result: Ok(_), .. }),
        "a superseded run must not answer: {stale:?}"
    );
    let current = engine.collect(Generation::FIRST.next(), Instant::now() + Duration::from_secs(5));
    assert!(
        matches!(current, LayoutOutcome::Ready { result: Ok(_), .. }),
        "{current:?}"
    );
}

#[test]
fn spinning_layout_is_aborted_by_the_instruction_budget() {
    let spin = "local M = {kind = 'tiling'} function M.arrange() while true do end end return M";
    let h = host(modules("spin", &[("spin", spin)]));
    h.borrow_mut()
        .register("spin", "spin", no_options())
        .expect("registers");
    let result = arrange(&mut engine(&h, "spin"), &[1], 100.0, 100.0);
    assert!(matches!(result, Err(LayoutFailure::Timeout)), "{result:?}");
}

#[test]
fn memory_bomb_is_refused() {
    let bomb = "local M = {kind = 'tiling'} function M.arrange() local t = {} for i = 1, 1e12 do t[i] = { i, i, i, i } end return nil end return M";
    let h = host(modules("bomb", &[("bomb", bomb)]));
    h.borrow_mut()
        .register("bomb", "bomb", no_options())
        .expect("registers");
    let result = arrange(&mut engine(&h, "bomb"), &[1], 100.0, 100.0);
    assert!(
        matches!(result, Err(LayoutFailure::Error | LayoutFailure::Timeout)),
        "{result:?}"
    );
}

#[test]
fn bad_returns_are_refused() {
    let cases = [
        (
            "nan",
            "local M={kind='tiling'} function M.arrange(ctx) return {placements={{window=ctx.windows[1].id,x=0,y=0,width=0/0,height=10}}} end return M",
        ),
        (
            "unknown",
            "local M={kind='tiling'} function M.arrange(ctx) return {placements={{window=12345,x=0,y=0,width=10,height=10}}} end return M",
        ),
        (
            "wrong_type",
            "local M={kind='tiling'} function M.arrange(ctx) return 42 end return M",
        ),
    ];
    for (name, source) in cases {
        let h = host(modules(name, &[(name, source)]));
        h.borrow_mut().register(name, name, no_options()).expect("registers");
        let p = params(&[1], 100.0, 100.0);
        let verdict = arrange(&mut engine(&h, name), &[1], 100.0, 100.0)
            .and_then(|reply| settle(&p, LayoutExtent::Output, reply));
        assert!(
            matches!(verdict, Err(LayoutFailure::InvalidReply(_) | LayoutFailure::Error)),
            "{name}: {verdict:?}"
        );
    }
}

#[test]
fn unbounded_extent_needs_a_tiling_kind() {
    for kind in ["floating", "kiosk"] {
        let source = format!("return {{kind = '{kind}', extent = 'unbounded', arrange = function() return {{}} end}}");
        let h = host(modules(&format!("extent-{kind}"), &[(kind, source.as_str())]));
        let failure = h
            .borrow_mut()
            .register(kind, kind, no_options())
            .expect_err("an unbounded extent is refused off tiling");
        assert!(
            failure.message.contains("needs kind `tiling`"),
            "{kind}: {}",
            failure.message
        );
    }
    let source = "return {kind = 'tiling', extent = 'unbounded', arrange = function() return {} end}";
    let h = host(modules("extent-tiling", &[("tiling", source)]));
    h.borrow_mut()
        .register("tiling", "tiling", no_options())
        .expect("a tiling layout may be unbounded");
}

#[test]
fn thrown_error_is_reported_as_an_error() {
    let boom = r#"local M={kind='tiling'} function M.arrange() error("boom") end return M"#;
    let h = host(modules("boom", &[("boom", boom)]));
    h.borrow_mut()
        .register("boom", "boom", no_options())
        .expect("registers");
    let result = arrange(&mut engine(&h, "boom"), &[1], 100.0, 100.0);
    assert!(matches!(result, Err(LayoutFailure::Error)), "{result:?}");
}

#[test]
fn require_is_sandboxed_to_the_configured_directory() {
    let escape = r#"local M = require("../escape") return M"#;
    let h = host(modules("sandbox", &[("escape", escape)]));
    assert!(h.borrow_mut().register("escape", "escape", no_options()).is_err());
    assert!(
        h.borrow_mut()
            .register("traversal", "../../etc/passwd", no_options())
            .is_err()
    );
}

#[test]
fn duplicate_register_is_refused_and_previous_registration_survives() {
    let h = host(modules("dup", &[("master", MASTER_STACK)]));
    h.borrow_mut()
        .register("master", "master", no_options())
        .expect("registers");
    assert!(h.borrow_mut().register("master", "missing", no_options()).is_err());
    let p = params(&[1, 2], 1000.0, 600.0);
    let layout = settled(
        &p,
        run(&mut engine(&h, "master"), Job::Arrange(p.clone()), Generation::FIRST),
    );
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
    let h = Rc::new(RefCell::new(Host::new(config)));
    h.borrow_mut()
        .register("wedged", "wedged", no_options())
        .expect("registers");
    h.borrow_mut()
        .register("fine", "fine", no_options())
        .expect("registers");
    // A wedge is abandoned by the call that starts a hard limit after the stall.
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(60));
        let result = arrange(&mut engine(&h, "wedged"), &[1], 100.0, 100.0);
        assert!(
            matches!(result, Err(LayoutFailure::VmWedged | LayoutFailure::Timeout)),
            "{result:?}"
        );
    }
    assert!(h.borrow().disabled(), "three abandoned workers disable layouts");
    assert!(h.borrow().leaked() >= 3);
    let result = arrange(&mut engine(&h, "fine"), &[1], 100.0, 100.0);
    assert!(matches!(result, Err(LayoutFailure::VmWedged)), "{result:?}");
}
