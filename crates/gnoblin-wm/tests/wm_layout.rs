//! End to end through the wm: a real, test-only master-stack engine answers the
//! layout contract for two workspaces while the wm handles windows, geometry,
//! state, workspace and output events.
//!
//! This covers the wm's own path only (events, slots, effects). It does not
//! cover the compositor or IPC: the wiring that installs the real engine comes in
//! a later lane.

use std::time::{Duration, Instant};

use gnoblin_types::{Direction, OutputId, Rect, WindowId, WorkspaceId};
use gnoblin_wm::layout::{
    Camera, Generation, Handled, Job, LayoutEngine, LayoutOutcome, Params, Placement, Reply, Ticket,
};
use gnoblin_wm::{ActivationDecision, Effect, Event, WindowSpec, Wm, WmOptions};

const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

/// Collects that answer `Pending` before each run is ready.
const PENDING_COLLECTS: u32 = 1;

fn ws(raw: u64) -> WorkspaceId {
    WorkspaceId::new(raw)
}

fn out(raw: u64) -> OutputId {
    OutputId::new(raw)
}

fn win(raw: u64) -> WindowId {
    WindowId::new(raw)
}

/// The first window takes the left half; the rest share the right half.
fn master_stack(params: &Params) -> Vec<Placement> {
    let (width, height) = (params.output_width(), params.output_height());
    let half = width / 2.0;
    let Some((master, stack)) = params.windows().split_first() else {
        return Vec::new();
    };
    let mut placements = vec![Placement::rect(master.id, 0.0, 0.0, half, height)];
    let count = stack.len();
    for (index, window) in stack.iter().enumerate() {
        let slot = height / count as f64;
        placements.push(Placement::rect(window.id, half, slot * index as f64, half, slot));
    }
    placements
}

/// A real engine for the contract. A submitted run is ready after
/// [`PENDING_COLLECTS`] pending collects, and only the latest run can be collected.
struct MasterStack {
    /// Answer every arrangement with `Declined`, so the built-in fallback places.
    decline: bool,
    pending: u32,
    ready: Option<(Generation, Reply)>,
}

impl LayoutEngine for MasterStack {
    fn submit(&mut self, job: Job, generation: Generation) -> Ticket {
        let reply = match job {
            Job::Arrange(_) | Job::Request(_) if self.decline => Reply::Declined,
            Job::Command(..) => Reply::Declined,
            Job::Arrange(params) => Reply::Handled(Handled {
                placements: master_stack(&params),
                camera: Camera::Orthographic,
                effects: Vec::new(),
            }),
            Job::Request(request) => Reply::Handled(Handled {
                placements: master_stack(&request.params),
                camera: Camera::Orthographic,
                effects: Vec::new(),
            }),
        };
        self.pending = PENDING_COLLECTS;
        self.ready = Some((generation, reply));
        Ticket::new(generation)
    }

    fn collect(&mut self, generation: Generation, _deadline: Instant) -> LayoutOutcome {
        if self.pending > 0 {
            self.pending -= 1;
            return LayoutOutcome::Pending;
        }
        match self.ready.take() {
            Some((ready, reply)) if ready == generation => LayoutOutcome::Ready {
                generation,
                result: Ok(reply),
            },
            other => {
                self.ready = other;
                LayoutOutcome::Pending
            }
        }
    }
}

fn wm_with_engines(decline: bool) -> Wm {
    let mut wm = Wm::new(WmOptions::default());
    for raw in [1, 2] {
        wm.apply(Event::WorkspaceAdded { workspace: ws(raw) });
    }
    wm.apply(Event::OutputConnected {
        output: out(0),
        rect: MONITOR,
    });
    let effects = wm.set_layout_engines(move |_workspace, _output| -> Box<dyn LayoutEngine> {
        Box::new(MasterStack {
            decline,
            pending: 0,
            ready: None,
        })
    });
    assert!(effects.is_empty(), "installing engines refuses no input");
    wm
}

fn open(wm: &mut Wm, raw: u64) {
    wm.apply(Event::WindowOpened(WindowSpec::normal(win(raw), "org.example.App")));
}

fn frame(wm: &mut Wm) -> Vec<Effect> {
    wm.apply(Event::Frame {
        deadline: Instant::now() + Duration::from_secs(1),
    })
}

/// Two frames: the first answers pending, the second collects the latest run.
fn settle(wm: &mut Wm) -> Vec<Effect> {
    let mut effects = frame(wm);
    effects.extend(frame(wm));
    effects
}

#[derive(Debug, PartialEq)]
struct Target {
    workspace: WorkspaceId,
    window: WindowId,
    width: f64,
    height: f64,
}

fn target(workspace: u64, window: u64, width: f64, height: f64) -> Target {
    Target {
        workspace: ws(workspace),
        window: win(window),
        width,
        height,
    }
}

/// The placement targets in the effects, sorted by workspace then window.
fn targets(effects: &[Effect]) -> Vec<Target> {
    let mut found: Vec<Target> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PlacementTarget { workspace, placement } => Some(Target {
                workspace: *workspace,
                window: placement.window,
                width: placement.width,
                height: placement.height,
            }),
            _ => None,
        })
        .collect();
    found.sort_by_key(|entry| (entry.workspace.get(), entry.window.get()));
    found
}

fn failures(effects: &[Effect]) -> Vec<&'static str> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::LayoutFailed { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

fn configured(effects: &[Effect], id: WindowId) -> Option<Rect> {
    effects.iter().find_map(|effect| match effect {
        Effect::Configure { window, geometry, .. } if *window == id => Some(*geometry),
        _ => None,
    })
}

#[test]
fn master_stack_follows_the_wm_through_a_session() {
    let mut wm = wm_with_engines(false);
    assert_eq!(wm.active_workspace(), Some(ws(1)));

    // Two windows on workspace 1. The first frame answers pending, so nothing is
    // placed yet; the second collects the latest run.
    open(&mut wm, 1);
    open(&mut wm, 2);
    assert!(targets(&frame(&mut wm)).is_empty(), "a pending run places nothing");
    let effects = frame(&mut wm);
    assert_eq!(
        targets(&effects),
        vec![target(1, 1, 960.0, 1080.0), target(1, 2, 960.0, 1080.0)]
    );
    assert!(failures(&effects).is_empty());

    // Set position is a geometry request on a tiled window, so it waits for the
    // next frame. The first collect is pending, so the request falls through to
    // the floating logic and window 2 is configured at its new rectangle. The
    // arrangement then re-runs against the new size.
    let moved = Rect::new(100, 100, 500, 400);
    let effects = wm.apply(Event::MoveResize {
        window: win(2),
        geometry: moved,
    });
    assert!(effects.is_empty(), "a tiled geometry request waits for the frame");
    let effects = frame(&mut wm);
    assert_eq!(configured(&effects, win(2)), Some(moved));
    assert_eq!(
        targets(&settle(&mut wm)),
        vec![target(1, 1, 960.0, 1080.0), target(1, 2, 960.0, 1080.0)]
    );

    // Maximised leaves the tiled set, so only window 1 is arranged.
    wm.apply(Event::ToggleMaximise { window: win(2) });
    assert_eq!(targets(&settle(&mut wm)), vec![target(1, 1, 960.0, 1080.0)]);

    // Fullscreen leaves the tiled set too. The set is empty, so nothing is
    // submitted and the last placements are dropped.
    wm.apply(Event::ToggleFullscreen { window: win(1) });
    let effects = settle(&mut wm);
    assert!(targets(&effects).is_empty(), "an empty tiled set places nothing");
    assert!(failures(&effects).is_empty());

    // Leaving fullscreen restores window 1 to tiling.
    wm.apply(Event::ToggleFullscreen { window: win(1) });
    assert_eq!(targets(&settle(&mut wm)), vec![target(1, 1, 960.0, 1080.0)]);

    // Floating takes window 1 out of tiling. Unmaximising window 2 with window 1
    // floating leaves window 2 alone in the set, and clearing floating on window 1
    // brings it back.
    wm.apply(Event::SetFloating {
        window: win(1),
        floating: true,
    });
    wm.apply(Event::ToggleMaximise { window: win(2) });
    assert_eq!(targets(&settle(&mut wm)), vec![target(1, 2, 960.0, 1080.0)]);
    wm.apply(Event::SetFloating {
        window: win(1),
        floating: false,
    });
    assert_eq!(
        targets(&settle(&mut wm)),
        vec![target(1, 1, 960.0, 1080.0), target(1, 2, 960.0, 1080.0)]
    );

    // Workspace 2 gets its own engine: its window is arranged alone.
    wm.apply(Event::WorkspaceActivated { workspace: ws(2) });
    open(&mut wm, 3);
    assert_eq!(targets(&settle(&mut wm)), vec![target(2, 3, 960.0, 1080.0)]);

    // Switching back keeps workspace 1's arrangement, so nothing is re-run.
    wm.apply(Event::WorkspaceActivated { workspace: ws(1) });
    assert!(targets(&settle(&mut wm)).is_empty(), "switching back re-runs nothing");

    // Output removal: a second output takes over, and every workspace re-arranges
    // against its work area.
    wm.apply(Event::OutputConnected {
        output: out(1),
        rect: Rect::new(1920, 0, 1280, 720),
    });
    wm.apply(Event::OutputDisconnected { output: out(0) });
    assert_eq!(
        targets(&settle(&mut wm)),
        vec![
            target(1, 1, 640.0, 720.0),
            target(1, 2, 640.0, 720.0),
            target(2, 3, 640.0, 720.0),
        ]
    );

    // With no output left, the layouts wait and submit nothing.
    let effects = wm.apply(Event::OutputDisconnected { output: out(1) });
    assert!(failures(&effects).is_empty());
    let effects = settle(&mut wm);
    assert!(targets(&effects).is_empty(), "no output, no placements");
    assert!(failures(&effects).is_empty());

    // A new output adopts the orphaned windows, and the arrangement returns to the
    // monitor's size.
    wm.apply(Event::OutputConnected {
        output: out(2),
        rect: MONITOR,
    });
    assert_eq!(
        targets(&settle(&mut wm)),
        vec![
            target(1, 1, 960.0, 1080.0),
            target(1, 2, 960.0, 1080.0),
            target(2, 3, 960.0, 1080.0),
        ]
    );
    assert!(wm.check_invariants().is_ok(), "the wm's invariants hold at the end");
}

#[test]
fn a_declined_arrangement_falls_back_and_still_places_every_window() {
    let mut wm = wm_with_engines(true);
    open(&mut wm, 1);
    open(&mut wm, 2);
    let effects = settle(&mut wm);
    let placed: Vec<WindowId> = targets(&effects).iter().map(|entry| entry.window).collect();
    assert_eq!(placed, vec![win(1), win(2)]);
    assert!(failures(&effects).is_empty(), "a decline is not a failure");
}

#[test]
fn without_an_engine_the_wm_places_nothing() {
    let mut wm = Wm::new(WmOptions::default());
    wm.apply(Event::WorkspaceAdded { workspace: ws(1) });
    wm.apply(Event::OutputConnected {
        output: out(0),
        rect: MONITOR,
    });
    open(&mut wm, 1);
    let effects = settle(&mut wm);
    assert!(targets(&effects).is_empty());
    assert!(failures(&effects).is_empty());
    assert!(wm.check_invariants().is_ok());
}

#[test]
fn directional_focus_follows_the_placed_rectangles() {
    let mut wm = wm_with_engines(false);
    open(&mut wm, 1);
    open(&mut wm, 2);
    open(&mut wm, 3);
    settle(&mut wm);

    // Opening a window does not focus it, so focus the stack window first.
    wm.apply(Event::ActivationRequested {
        window: win(2),
        decision: ActivationDecision::Allow,
    });
    assert_eq!(wm.focused(), Some(win(2)));
    // The master is the only window left of the stack, so Left lands on it.
    wm.apply(Event::FocusDirection {
        direction: Direction::Left,
    });
    assert_eq!(wm.focused(), Some(win(1)));
    // Nothing lies further left of the master, so focus stays put.
    wm.apply(Event::FocusDirection {
        direction: Direction::Left,
    });
    assert_eq!(wm.focused(), Some(win(1)));
    // Right from the master lands in the stack, not back on the master.
    wm.apply(Event::FocusDirection {
        direction: Direction::Right,
    });
    assert!(matches!(wm.focused(), Some(id) if id == win(2) || id == win(3)));
}
