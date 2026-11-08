//! Model-based test: random event sequences, checked against every invariant.
//!
//! The generator is a small hand-written xorshift so the suite needs no dependency
//! and every failure is reproducible from its seed. Each sequence replays twice: the
//! second run must produce an identical effect stream, which is the determinism
//! promise the parity harness relies on.
//!
//! Invariants (from [`Wm::check_invariants`]):
//!
//! - every managed window is on exactly one declared workspace;
//! - every managed window is on exactly one connected output, or on none while no
//!   output is connected;
//! - exactly zero or one window is focused, and a focused window is visible;
//! - the stack contains every managed window exactly once and never interleaves
//!   layers;
//! - maximised, fullscreen and snapped windows match the rectangle their state
//!   implies;
//! - a transient shares its parent's workspace;
//! - the focus history names no window that has gone away.

use gnoblin_types::{Insets, OutputId, Rect, WindowId, WorkspaceId};
use gnoblin_wm::{
    ActivationDecision, DragConstraint, Effect, Event, Match, Pattern, PlacementMode, Rule, RuleFields, SnapZone,
    WindowSpec, Wm, WmOptions, WorkspaceSelector,
};

/// Windows the generator draws ids from; the pool is smaller than the event count so
/// opens, closes, duplicate opens and activations of gone windows all occur.
const WINDOW_POOL: u32 = 6;

/// Outputs the generator draws ids from.
const OUTPUT_POOL: u32 = 3;

/// Workspaces the generator declares and switches between.
const WORKSPACE_POOL: u32 = 4;

/// Sequences per test run.
const SEQUENCES: u64 = 128;

/// Events per sequence.
const EVENTS_PER_SEQUENCE: u32 = 240;

/// A failing run must report at least this many invariant checks, so a suite that
/// silently stops exercising the state machine is visible.
const MIN_CHECKS: usize = 200_000;

/// A deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    /// A number below `bound`, which must be greater than zero.
    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }

    fn rect(&mut self) -> Rect {
        Rect::new(
            self.below(2600) as i32 - 400,
            self.below(1600) as i32 - 300,
            self.below(900) + 100,
            self.below(700) + 100,
        )
    }

    fn window(&mut self) -> WindowId {
        WindowId::new(u64::from(self.below(WINDOW_POOL)) + 1)
    }

    fn output(&mut self) -> OutputId {
        OutputId::new(u64::from(self.below(OUTPUT_POOL)) + 1)
    }

    fn workspace(&mut self) -> WorkspaceId {
        WorkspaceId::new(u64::from(self.below(WORKSPACE_POOL)) + 1)
    }

    fn decision(&mut self) -> ActivationDecision {
        match self.below(3) {
            0 => ActivationDecision::Allow,
            1 => ActivationDecision::AllowWithoutRaise,
            _ => ActivationDecision::Deny,
        }
    }
}

/// A rule list that exercises both the workspace and maximise fields and both selector kinds.
fn rules(rng: &mut Rng) -> Vec<Rule> {
    let app_id =
        Pattern::new(if rng.below(2) == 0 { "org.example" } else { "Other" }).expect("fixed patterns are valid");
    let selector = if rng.below(2) == 0 {
        WorkspaceSelector::Id(rng.workspace())
    } else {
        WorkspaceSelector::Number(rng.below(WORKSPACE_POOL) + 1)
    };
    let maximise = if rng.below(2) == 0 { Some(true) } else { None };
    vec![
        Rule {
            matcher: Match {
                app_id: Some(app_id),
                ..Match::default()
            },
            fields: RuleFields {
                workspace: Some(selector),
                maximise,
                secure: None,
                opacity: None,
                corner_radius: None,
            },
        },
        Rule {
            matcher: Match {
                title: Some(Pattern::new("^Report").expect("fixed patterns are valid")),
                ..Match::default()
            },
            fields: RuleFields::default(),
        },
    ]
}

/// One event from the generator, covering every variant of [`Event`].
fn next_event(rng: &mut Rng) -> Event {
    match rng.below(101) {
        0..=20 => {
            let id = rng.window();
            if rng.below(6) == 0 {
                let parent = rng.window();
                Event::WindowOpened(WindowSpec::transient(id, "org.example.Dialog", parent).with_requested(rng.rect()))
            } else {
                let app_id = if rng.below(2) == 0 {
                    "org.example.Editor"
                } else {
                    "Other.App"
                };
                let title = if rng.below(3) == 0 { "Report 1" } else { "Untitled" };
                Event::WindowOpened(
                    WindowSpec::normal(id, app_id)
                        .with_title(title)
                        .with_requested(rng.rect()),
                )
            }
        }
        21..=28 => Event::WindowClosed { window: rng.window() },
        29..=36 => Event::ActivationRequested {
            window: rng.window(),
            decision: rng.decision(),
        },
        37..=41 => Event::Minimise { window: rng.window() },
        42..=46 => Event::Unminimise { window: rng.window() },
        47..=53 => Event::ToggleMaximise { window: rng.window() },
        54..=61 => Event::ToggleFullscreen { window: rng.window() },
        62..=67 => Event::SnapRequested {
            window: rng.window(),
            zone: SnapZone::ALL[rng.below(8) as usize],
        },
        68..=70 => Event::Unsnap { window: rng.window() },
        71..=78 => Event::MoveResize {
            window: rng.window(),
            geometry: rng.rect(),
        },
        79..=82 => Event::Raise { window: rng.window() },
        83..=85 => Event::Lower { window: rng.window() },
        86..=89 => Event::WorkspaceActivated {
            workspace: rng.workspace(),
        },
        90..=91 => Event::WorkspaceAdded {
            workspace: rng.workspace(),
        },
        92..=93 => Event::WorkspaceRemoved {
            workspace: rng.workspace(),
        },
        94 => Event::OutputDisconnected { output: rng.output() },
        95 => Event::OutputConnected {
            output: rng.output(),
            rect: Rect::new(
                coord(rng.below(3)) * 1920,
                coord(rng.below(2)) * 1080,
                1280 + rng.below(1280),
                720 + rng.below(720),
            ),
        },
        96 => Event::OutputMoved {
            output: rng.output(),
            rect: Rect::new(0, 0, 1280, 720),
        },
        97 => Event::OutputReserved {
            output: rng.output(),
            reserved: Insets {
                top: coord(rng.below(64)),
                left: coord(rng.below(64)),
                ..Insets::ZERO
            },
        },
        98 => Event::RulesChanged { rules: rules(rng) },
        _ => Event::WindowTitleChanged {
            window: rng.window(),
            title: "Report 2".to_owned(),
        },
    }
}

/// Converts a generator value to a coordinate without a lossy cast on the signed type.
fn coord(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(0)
}

/// Runs one sequence, checking every invariant after every event.
fn run(seed: u64) -> (Vec<Vec<Effect>>, usize) {
    let mut rng = Rng::new(seed);
    let options = WmOptions {
        // Both new-window policies are exercised: the seed picks one.
        new_window_focus: if seed.is_multiple_of(3) {
            gnoblin_wm::NewWindowFocus::Prevent
        } else {
            gnoblin_wm::NewWindowFocus::Allow
        },
        // Every mode is exercised: the seed picks one of each pair.
        placement: match seed % 3 {
            0 => PlacementMode::FreeSpace,
            1 => PlacementMode::Center,
            _ => PlacementMode::Cascade,
        },
        edge_tiling: seed.is_multiple_of(2),
        drag_constraint: if seed.is_multiple_of(4) {
            DragConstraint::MinimumStrip
        } else {
            DragConstraint::WorkArea
        },
        focus_mode: gnoblin_wm::FocusMode::Click,
        focus_change_on_pointer_rest: false,
        raise_on_click: !seed.is_multiple_of(5),
        auto_raise: seed.is_multiple_of(7),
        auto_raise_delay: 0,
    };
    let mut wm = Wm::new(options);
    for raw in 1..=WORKSPACE_POOL {
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(u64::from(raw)),
        });
    }
    for raw in 1..=OUTPUT_POOL {
        wm.apply(Event::OutputConnected {
            output: OutputId::new(u64::from(raw)),
            rect: Rect::new(coord(raw - 1) * 1920, 0, 1920, 1080),
        });
    }
    let mut stream = Vec::with_capacity(EVENTS_PER_SEQUENCE as usize);
    let mut checks = 0_usize;
    for _ in 0..EVENTS_PER_SEQUENCE {
        let event = next_event(&mut rng);
        let effects = wm.apply(event.clone());
        match wm.check_invariants() {
            Ok(ran) => checks += ran,
            Err(broken) => panic!("seed {seed}: {broken:?} after {event:?}"),
        }
        stream.push(effects);
    }
    (stream, checks)
}

#[test]
fn random_event_sequences_keep_every_invariant() {
    let mut checks = 0_usize;
    let mut effects = 0_usize;
    let mut busiest = 0_usize;
    for seed in 0..SEQUENCES {
        let (first, first_checks) = run(seed);
        let (second, second_checks) = run(seed);
        assert_eq!(first, second, "seed {seed} produced two different effect streams");
        assert_eq!(first_checks, second_checks);
        checks += first_checks;
        effects += first.iter().map(Vec::len).sum::<usize>();
        busiest = busiest.max(
            first
                .iter()
                .flatten()
                .filter(|effect| matches!(effect, Effect::Configure { .. }))
                .count(),
        );
    }
    let events = SEQUENCES * u64::from(EVENTS_PER_SEQUENCE);
    println!(
        "model: {SEQUENCES} sequences x {EVENTS_PER_SEQUENCE} events = {events} events, {checks} invariant checks, \
         {effects} effects, busiest sequence {busiest} configures"
    );
    assert!(
        checks >= MIN_CHECKS,
        "expected at least {MIN_CHECKS} invariant checks, ran {checks}"
    );
}

#[test]
fn hotplug_storms_never_strand_a_window() {
    // A focused version of the invariant that keeps breaking in a hotplugged tree:
    // after any sequence of connects and disconnects, either an output exists and
    // every window is on one, or no output exists and no window claims one.
    for seed in 0..32 {
        let mut rng = Rng::new(seed);
        let mut wm = Wm::new(WmOptions {
            edge_tiling: true,
            ..WmOptions::default()
        });
        for raw in 1..=WORKSPACE_POOL {
            wm.apply(Event::WorkspaceAdded {
                workspace: WorkspaceId::new(u64::from(raw)),
            });
        }
        wm.apply(Event::OutputConnected {
            output: OutputId::new(1),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        for _ in 0..80 {
            let event = match rng.below(5) {
                0 => Event::OutputConnected {
                    output: rng.output(),
                    rect: Rect::new(0, 0, 1600, 900),
                },
                1 => Event::OutputDisconnected { output: rng.output() },
                2 => {
                    Event::WindowOpened(WindowSpec::normal(rng.window(), "org.example.App").with_requested(rng.rect()))
                }
                3 => Event::ToggleFullscreen { window: rng.window() },
                _ => Event::ToggleMaximise { window: rng.window() },
            };
            wm.apply(event);
            assert!(
                wm.check_invariants().is_ok(),
                "seed {seed} broke an invariant during a hotplug storm"
            );
        }
        let outputs = wm.outputs().len();
        for window in wm.windows() {
            assert_eq!(
                window.output.is_some(),
                outputs > 0,
                "every window agrees with the set of connected outputs"
            );
        }
    }
}
