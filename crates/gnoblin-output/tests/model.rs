//! Model-based test: random hotplug and configuration sequences, checked against
//! every invariant.
//!
//! The generator is a small hand-written xorshift so the suite needs no dependency
//! and every failure is reproducible from its seed. Each sequence replays twice and
//! the effect streams must match, which is the determinism promise the parity
//! harness relies on.
//!
//! Invariants (from [`OutputModel::check_invariants`]):
//!
//! - a monitor's rectangle is always the size its mode, scale and orientation imply,
//!   so a logical size is never a rounded product;
//! - enabled outputs never overlap;
//! - the layout is connected unless the configuration declares gaps;
//! - every member of a monitor is connected, and every enabled, connected output is
//!   shown exactly once;
//! - a layout with monitors has a primary output, and the primary is shown;
//! - monitors are emitted in a fixed order;
//! - a rollback restores the layout that was in force when the proposal arrived.

use gnoblin_output::{
    Align, Edge, Effect, Event, Layout, LayoutConfig, Mode, ModelOptions, OutputConfig, OutputDescriptor, OutputModel,
    Phase, Placement, TransactionToken, Verdict, VrrSupport,
};
use gnoblin_types::{Orientation, OutputId, Scale};

/// Connectors the generator connects, disconnects and configures.
const CONNECTORS: [&str; 5] = ["DP-1", "DP-2", "DP-3", "HDMI-1", "eDP-1"];

/// Sequences per test run.
const SEQUENCES: u64 = 64;

/// Events per sequence.
const EVENTS_PER_SEQUENCE: u32 = 240;

/// A passing run must report at least this many invariant checks, so a suite that
/// silently stops exercising the state machine is visible.
const MIN_CHECKS: usize = 20_000;

/// The graph token output discovery would have allocated for this connector.
fn output_token(connector: &str) -> OutputId {
    let index = CONNECTORS
        .iter()
        .position(|name| *name == connector)
        .expect("the generator only names declared connectors");
    OutputId::new(u64::try_from(index).expect("a small index") + 1)
}

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

    fn pick<T: Copy>(&mut self, options: &[T]) -> T {
        options[self.below(options.len() as u32) as usize]
    }

    fn connector(&mut self) -> &'static str {
        self.pick(&CONNECTORS)
    }
}

fn full_hd() -> Mode {
    Mode::new(1920, 1080, 60_000)
}

fn qhd() -> Mode {
    Mode::new(2560, 1440, 60_000)
}

fn small() -> Mode {
    Mode::new(1280, 720, 59_940)
}

/// A connector with a random mode list and scale list, so configurations hit both
/// supported and unsupported combinations.
fn descriptor(rng: &mut Rng) -> OutputDescriptor {
    let connector = rng.connector();
    let modes = match rng.below(6) {
        0 => Vec::new(),
        1 => vec![full_hd()],
        2 => vec![qhd(), full_hd()],
        3 => vec![small(), full_hd(), qhd()],
        _ => vec![full_hd(), qhd()],
    };
    let scales = match rng.below(4) {
        0 => vec![Scale::integral(1)],
        1 => vec![Scale::integral(1), Scale::integral(2)],
        2 => vec![Scale::integral(1), Scale::from_percent(150)],
        _ => vec![Scale::integral(2)],
    };
    let mut descriptor = OutputDescriptor::new(output_token(connector), connector, modes);
    descriptor.scales = scales;
    descriptor.vrr = if rng.below(4) == 0 {
        VrrSupport::Supported
    } else {
        VrrSupport::Unsupported
    };
    descriptor
}

/// A configuration that is sometimes valid and sometimes not: unreachable anchors,
/// cycles, duplicates, unsupported modes and disabled-everything all occur.
fn config(rng: &mut Rng) -> LayoutConfig {
    let count = 1 + rng.below(3);
    let mut outputs = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let connector = rng.connector();
        let enabled = rng.below(10) != 0;
        let placement = match rng.below(7) {
            0 => None,
            1 => Some(Placement::Absolute {
                x: rng.below(2000) as i32 - 400,
                y: rng.below(1000) as i32 - 200,
            }),
            2..=3 => Some(Placement::Relative {
                of: rng.connector().to_owned(),
                edge: rng.pick(&[Edge::Left, Edge::Right, Edge::Above, Edge::Below]),
                align: rng.pick(&[Align::Start, Align::Center, Align::End]),
            }),
            _ => Some(Placement::Origin),
        };
        let mode = match rng.below(4) {
            0..=1 => None,
            2 => Some(full_hd()),
            _ => Some(qhd()),
        };
        let scale = match rng.below(4) {
            0..=1 => None,
            2 => Some(Scale::integral(2)),
            _ => Some(Scale::from_percent(150)),
        };
        let mirror_of = if rng.below(7) == 0 {
            Some(rng.connector().to_owned())
        } else {
            None
        };
        outputs.push(OutputConfig {
            connector: connector.to_owned(),
            enabled,
            mode,
            scale,
            orientation: rng.pick(&[
                Orientation::Normal,
                Orientation::Normal,
                Orientation::Rotate90,
                Orientation::Rotate180,
                Orientation::Flipped90,
            ]),
            vrr: rng.below(4) == 0,
            placement,
            mirror_of,
        });
    }
    LayoutConfig {
        outputs,
        primary: if rng.below(4) == 0 {
            Some(rng.connector().to_owned())
        } else {
            None
        },
        allow_gaps: rng.below(5) == 0,
    }
}

/// What one sequence produced, for the determinism comparison and the coverage floor.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Summary {
    effects: Vec<Effect>,
    final_layout: Layout,
    checks: usize,
    rollback_checks: usize,
    rollbacks: usize,
    test_requests: usize,
    applications: usize,
    rejections: usize,
}

fn run(seed: u64) -> Summary {
    let mut rng = Rng::new(seed);
    let mut model = OutputModel::new(ModelOptions {
        confirm_timeout_ticks: 5,
    });
    let mut summary = Summary::default();
    let mut connected: Vec<OutputId> = Vec::new();
    let mut tokens: Vec<TransactionToken> = Vec::new();
    let mut before: Option<(Layout, Vec<OutputId>)> = None;

    for _ in 0..EVENTS_PER_SEQUENCE {
        let roll = rng.below(100);
        let event = if roll < 14 {
            let descriptor = descriptor(&mut rng);
            if !connected.contains(&descriptor.id) {
                connected.push(descriptor.id);
            }
            before = None;
            Event::Connected(descriptor)
        } else if roll < 24 {
            let id = output_token(rng.connector());
            connected.retain(|other| *other != id);
            before = None;
            Event::Disconnected(id)
        } else if roll < 44 {
            Event::Configured(config(&mut rng))
        } else if roll < 64 {
            if model.phase() == Phase::Idle {
                before = Some((model.layout().clone(), connected.clone()));
            }
            Event::Propose(config(&mut rng))
        } else if roll < 76 {
            match rng.below(4) {
                _ if tokens.is_empty() => Event::Tick {
                    ticks: u64::from(rng.below(8)),
                },
                // Sometimes answer with a token that is no longer current.
                0 if tokens.len() > 1 => Event::Verdict {
                    token: tokens[rng.below(tokens.len() as u32 - 1) as usize],
                    verdict: Verdict::Passed,
                },
                _ => Event::Verdict {
                    token: tokens[rng.below(tokens.len() as u32) as usize],
                    verdict: if rng.below(2) == 0 {
                        Verdict::Passed
                    } else {
                        Verdict::Failed
                    },
                },
            }
        } else if roll < 84 {
            Event::Tick {
                ticks: u64::from(rng.below(8)),
            }
        } else if roll < 92 {
            match tokens.last().copied() {
                Some(token) => Event::Confirm { token },
                None => Event::Tick { ticks: 1 },
            }
        } else {
            match tokens.last().copied() {
                Some(token) => Event::Rollback { token },
                None => Event::Tick { ticks: 1 },
            }
        };

        let effects = model.apply(event);

        for effect in &effects {
            match effect {
                Effect::TestRequested { token, .. } => {
                    tokens.push(*token);
                    summary.test_requests += 1;
                }
                Effect::Applied { .. } => summary.applications += 1,
                Effect::Rejected(_) => summary.rejections += 1,
                Effect::RolledBack { layout, .. } => {
                    summary.rollbacks += 1;
                    if let Some((previous, set)) = before.take()
                        && set == connected
                    {
                        assert_eq!(
                            layout, &previous,
                            "seed {seed}: a rollback must restore the layout that was in force"
                        );
                        summary.rollback_checks += 1;
                    }
                }
                Effect::Confirmed { .. } => before = None,
                Effect::Committed { .. } => {}
            }
        }
        summary.effects.extend(effects);

        summary.checks += model
            .check_invariants()
            .unwrap_or_else(|broken| panic!("seed {seed}: {broken:?}"));
    }

    summary.final_layout = model.layout().clone();
    summary
}

#[test]
fn random_hotplug_and_configuration_sequences_hold_every_invariant() {
    let mut checks = 0_usize;
    let mut rollback_checks = 0_usize;
    let mut coverage = Summary::default();

    for seed in 0..SEQUENCES {
        let first = run(seed);
        let second = run(seed);
        assert_eq!(
            first.effects, second.effects,
            "seed {seed}: the same inputs must produce the same effect stream"
        );
        assert_eq!(
            first.final_layout, second.final_layout,
            "seed {seed}: the same inputs must produce the same layout"
        );

        checks += first.checks;
        rollback_checks += first.rollback_checks;
        coverage.rollbacks += first.rollbacks;
        coverage.test_requests += first.test_requests;
        coverage.applications += first.applications;
        coverage.rejections += first.rejections;
        assert_eq!(
            first.rollback_checks, second.rollback_checks,
            "seed {seed}: the harvest of rollback comparisons is deterministic"
        );
    }

    println!(
        "{checks} invariant checks, {rollback_checks} rollback comparisons, \
         {} proposals, {} rollbacks, {} hotplug applications, {} refusals",
        coverage.test_requests, coverage.rollbacks, coverage.applications, coverage.rejections
    );
    assert!(checks >= MIN_CHECKS, "only {checks} invariant checks ran");
    assert!(
        rollback_checks > 0,
        "no rollback was compared with the layout it restored"
    );
    assert!(coverage.test_requests > 0, "no proposal reached the backend");
    assert!(coverage.rollbacks > 0, "no rollback happened");
    assert!(coverage.applications > 0, "no hotplug changed the layout");
    assert!(coverage.rejections > 0, "no invalid configuration was refused");
}
