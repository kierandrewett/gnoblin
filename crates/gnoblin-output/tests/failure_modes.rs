//! One test per documented failure mode, plus the behaviours the state machine
//! exists to provide.

use gnoblin_output::{
    Align, Edge, Edid, EdidError, Effect, Event, Layout, LayoutConfig, LayoutError, Mode, ModelOptions, OutputConfig,
    OutputDescriptor, OutputModel, Phase, PhysicalSize, Placement, Rejection, RollbackReason, TransactionToken,
    Verdict, VrrSupport, compute,
};
use gnoblin_types::{Orientation, OutputId, Rect, Scale};

const TIMEOUT: u64 = 10;

/// The connectors these tests use, in the order their graph tokens are allocated.
const CONNECTORS: [&str; 5] = ["DP-1", "DP-2", "DP-3", "DP-4", "HDMI-9"];

/// The graph token output discovery would have allocated for this connector.
fn output_token(connector: &str) -> OutputId {
    let index = CONNECTORS
        .iter()
        .position(|name| *name == connector)
        .expect("the test only names connectors it declared");
    OutputId::new(u64::try_from(index).expect("a small index") + 1)
}

fn name(connector: &str) -> String {
    connector.to_owned()
}

fn full_hd() -> Mode {
    Mode::new(1920, 1080, 60_000)
}

fn qhd() -> Mode {
    Mode::new(2560, 1440, 60_000)
}

/// A 1920x1080 panel that offers the scales the tests change between.
fn panel(connector: &str) -> OutputDescriptor {
    let mut descriptor = OutputDescriptor::new(output_token(connector), connector, vec![full_hd()]);
    descriptor.scales = vec![Scale::integral(1), Scale::integral(2)];
    descriptor
}

fn wide(connector: &str) -> OutputDescriptor {
    let mut descriptor = OutputDescriptor::new(output_token(connector), connector, vec![qhd()]);
    descriptor.scales = vec![Scale::integral(1)];
    descriptor
}

/// A session with `outputs` connected and `config` accepted.
fn configured(outputs: &[OutputDescriptor], config: LayoutConfig) -> OutputModel {
    let mut model = OutputModel::new(ModelOptions {
        confirm_timeout_ticks: TIMEOUT,
    });
    for descriptor in outputs {
        model.apply(Event::Connected(descriptor.clone()));
    }
    let effects = model.apply(Event::Configured(config));
    assert!(
        !effects.iter().any(|effect| matches!(effect, Effect::Rejected(_))),
        "the baseline configuration must resolve: {effects:?}"
    );
    model
}

fn entry(connector: &str, placement: Placement) -> OutputConfig {
    OutputConfig {
        placement: Some(placement),
        ..OutputConfig::enabled(connector)
    }
}

fn scaled(connector: &str, placement: Placement, scale: u32) -> OutputConfig {
    OutputConfig {
        scale: Some(Scale::integral(scale)),
        ..entry(connector, placement)
    }
}

fn mirror(connector: &str, of: &str) -> OutputConfig {
    OutputConfig {
        mirror_of: Some(name(of)),
        ..OutputConfig::enabled(connector)
    }
}

fn right_of(of: &str) -> Placement {
    Placement::Relative {
        of: name(of),
        edge: Edge::Right,
        align: Align::Start,
    }
}

fn row(connectors: &[&str]) -> LayoutConfig {
    let outputs = connectors
        .iter()
        .enumerate()
        .map(
            |(position, connector)| match position.checked_sub(1).and_then(|previous| connectors.get(previous)) {
                Some(previous) => entry(connector, right_of(previous)),
                None => entry(connector, Placement::Origin),
            },
        )
        .collect();
    LayoutConfig::new(outputs)
}

fn token_of(effects: &[Effect]) -> TransactionToken {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::TestRequested { token, .. } => Some(*token),
            _ => None,
        })
        .expect("a proposal asks the backend to test the layout")
}

fn rejection(effects: &[Effect]) -> Rejection {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Rejected(rejection) => Some(rejection.clone()),
            _ => None,
        })
        .expect("the request is refused")
}

fn rolled_back(effects: &[Effect]) -> (Layout, RollbackReason) {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RolledBack { layout, reason, .. } => Some((layout.clone(), *reason)),
            _ => None,
        })
        .expect("the transaction ends with a rollback")
}

/// Two monitors in these tests must never share a pixel.
fn assert_no_overlap(layout: &Layout) {
    for (index, monitor) in layout.monitors.iter().enumerate() {
        for other in &layout.monitors[index + 1..] {
            assert!(
                !monitor.rect.intersects(other.rect),
                "{} and {} overlap: {:?} and {:?}",
                monitor.monitor,
                other.monitor,
                monitor.rect,
                other.rect
            );
        }
    }
}

/// Failure mode 1: a rollback that restores something other than the previous layout.
#[test]
fn rollback_restores_the_previous_layout_exactly() {
    let mut model = configured(&[panel("DP-1"), panel("DP-2")], row(&["DP-1", "DP-2"]));
    let previous = model.layout().clone();

    let stacked = LayoutConfig::new(vec![
        scaled("DP-1", Placement::Origin, 2),
        entry(
            "DP-2",
            Placement::Relative {
                of: name("DP-1"),
                edge: Edge::Below,
                align: Align::End,
            },
        ),
    ]);
    let effects = model.apply(Event::Propose(stacked));
    let token = token_of(&effects);
    assert_eq!(token.get(), 0);
    assert_ne!(model.layout(), &previous, "the backend is showing the proposal");
    let effects = model.apply(Event::Verdict {
        token,
        verdict: Verdict::Passed,
    });
    assert!(matches!(effects.as_slice(), [Effect::Committed { .. }]));
    assert_ne!(model.layout(), &previous, "a commit keeps it");

    let effects = model.apply(Event::Tick { ticks: TIMEOUT });
    let (restored, reason) = rolled_back(&effects);
    assert_eq!(reason, RollbackReason::Timeout);
    assert_eq!(restored, previous, "the timeout restores the layout that was in force");
    assert_eq!(model.layout(), &previous);
    assert_eq!(model.phase(), Phase::Idle);
    assert!(model.check_invariants().is_ok());
}

/// Failure mode 1 through the other exit: an explicit rollback after a commit.
#[test]
fn an_explicit_rollback_after_commit_restores_the_previous_layout() {
    let mut model = configured(&[panel("DP-1"), panel("DP-2")], row(&["DP-1", "DP-2"]));
    let previous = model.layout().clone();

    let mirrored = LayoutConfig::new(vec![entry("DP-1", Placement::Origin), mirror("DP-2", "DP-1")]);
    let effects = model.apply(Event::Propose(mirrored));
    let token = token_of(&effects);
    model.apply(Event::Verdict {
        token,
        verdict: Verdict::Passed,
    });
    assert_eq!(model.layout().monitors.len(), 1, "the proposal is showing");

    let effects = model.apply(Event::Rollback { token });
    let (restored, reason) = rolled_back(&effects);
    assert_eq!(reason, RollbackReason::Requested);
    assert_eq!(restored, previous);
    assert_eq!(model.layout(), &previous);
    assert_eq!(
        model.apply(Event::Rollback { token }),
        vec![Effect::Rejected(Rejection::NoTransaction)]
    );
}

/// Failure mode 7: a test that fails must restore before anything is committed.
#[test]
fn a_failed_test_rolls_back_before_commit() {
    let mut model = configured(&[panel("DP-1")], row(&["DP-1"]));
    let previous = model.layout().clone();

    let effects = model.apply(Event::Propose(row(&["DP-1"])));
    let token = token_of(&effects);
    let effects = model.apply(Event::Verdict {
        token,
        verdict: Verdict::Failed,
    });
    let (restored, reason) = rolled_back(&effects);
    assert_eq!(reason, RollbackReason::TestFailed);
    assert_eq!(restored, previous);
    assert_eq!(model.phase(), Phase::Idle);
    assert!(model.check_invariants().is_ok());
}

/// Failure mode 2: a configuration that leaves the session with no output.
#[test]
fn a_configuration_that_enables_no_output_is_rejected() {
    let mut model = configured(&[panel("DP-1")], row(&["DP-1"]));
    let previous = model.layout().clone();

    let disabled = LayoutConfig::new(vec![OutputConfig::disabled("DP-1")]);
    let effects = model.apply(Event::Configured(disabled.clone()));
    assert_eq!(rejection(&effects), Rejection::Invalid(LayoutError::NoEnabledOutputs));
    assert_eq!(model.layout(), &previous, "a refused configuration changes nothing");

    let effects = model.apply(Event::Propose(disabled));
    assert_eq!(rejection(&effects), Rejection::Invalid(LayoutError::NoEnabledOutputs));
    assert_eq!(model.phase(), Phase::Idle);
    assert_eq!(
        rejection(&model.apply(Event::Configured(LayoutConfig::default()))),
        Rejection::Invalid(LayoutError::NoEnabledOutputs)
    );
    assert_eq!(
        compute(&[], &row(&["DP-1"])),
        Err(LayoutError::NoEnabledOutputs),
        "with nothing connected the engine refuses the same way"
    );
}

/// Failure mode 4: a confirmation that arrives after the transaction expired.
#[test]
fn confirming_after_the_timeout_rolls_back() {
    let mut model = configured(&[panel("DP-1"), panel("DP-2")], row(&["DP-1", "DP-2"]));
    let previous = model.layout().clone();

    let effects = model.apply(Event::Propose(row(&["DP-2", "DP-1"])));
    let token = token_of(&effects);
    model.apply(Event::Verdict {
        token,
        verdict: Verdict::Passed,
    });
    assert_eq!(model.phase(), Phase::Committed { elapsed_ticks: 0 });

    assert_eq!(
        model.apply(Event::Tick { ticks: TIMEOUT - 1 }),
        Vec::new(),
        "the transaction is still inside its window"
    );
    let effects = model.apply(Event::Tick { ticks: 1 });
    assert_eq!(rolled_back(&effects).1, RollbackReason::Timeout);

    assert_eq!(
        model.apply(Event::Confirm { token }),
        vec![Effect::Rejected(Rejection::NotCommitted)],
        "a confirmation after the rollback is refused"
    );
    assert_eq!(model.layout(), &previous);
}

/// Failure mode 3: an absolute placement that a scale change turns into an overlap.
#[test]
fn a_scale_change_that_would_overlap_is_rejected() {
    let outputs = [panel("DP-1"), panel("DP-2")];
    let fixed = Placement::Absolute { x: 960, y: 0 };
    let at_two = LayoutConfig::new(vec![scaled("DP-1", Placement::Origin, 2), entry("DP-2", fixed.clone())]);
    let layout = compute(&outputs, &at_two).expect("960 wide, and the neighbour starts at 960");
    assert_eq!(layout.monitors[0].rect.width, 960);
    assert_no_overlap(&layout);

    let at_one = LayoutConfig::new(vec![scaled("DP-1", Placement::Origin, 1), entry("DP-2", fixed)]);
    let overlap = LayoutError::Overlap {
        first: name("DP-1"),
        second: name("DP-2"),
    };
    assert_eq!(compute(&outputs, &at_one), Err(overlap.clone()));

    let mut model = configured(&outputs, at_two);
    assert_eq!(
        rejection(&model.apply(Event::Configured(at_one))),
        Rejection::Invalid(overlap)
    );
    assert!(model.check_invariants().is_ok());
}

/// Failure mode 3, resolved: relative placement follows the new logical size.
#[test]
fn a_scale_change_with_relative_placement_stays_flush() {
    let outputs = [panel("DP-1"), panel("DP-2")];
    let mut config = row(&["DP-1", "DP-2"]);
    config.outputs[0].scale = Some(Scale::integral(2));
    let first = compute(&outputs, &config).expect("a row resolves at any scale");
    assert_eq!(first.monitors[0].rect.width, 960);
    assert_eq!(first.monitors[1].rect.x, 960);

    config.outputs[0].scale = Some(Scale::integral(1));
    let second = compute(&outputs, &config).expect("the neighbour follows the width");
    assert_eq!(second.monitors[0].rect.width, 1920);
    assert_eq!(second.monitors[1].rect.x, 1920);
    assert_no_overlap(&second);
    assert!(
        second.monitors[0].rect.touches(second.monitors[1].rect),
        "the two monitors stay joined"
    );
}

/// Failure mode 5: an output going away while its layout is being tested.
#[test]
fn hotplug_mid_transaction_aborts_the_test() {
    let mut model = configured(&[panel("DP-1"), panel("DP-2")], row(&["DP-1", "DP-2"]));
    let effects = model.apply(Event::Propose(row(&["DP-2", "DP-1"])));
    let token = token_of(&effects);
    assert_eq!(model.phase(), Phase::Testing);

    let effects = model.apply(Event::Disconnected(output_token("DP-2")));
    let (restored, reason) = rolled_back(&effects);
    assert_eq!(reason, RollbackReason::Hotplug);
    assert_eq!(model.phase(), Phase::Idle);
    assert!(
        restored
            .monitors
            .iter()
            .all(|monitor| !monitor.contains(output_token("DP-2"))),
        "no monitor names the output that went away: {restored:?}"
    );
    assert_eq!(model.layout(), &restored);
    assert_eq!(
        model.apply(Event::Confirm { token }),
        vec![Effect::Rejected(Rejection::NotCommitted)],
        "the abandoned test cannot be confirmed afterwards"
    );
    assert!(model.check_invariants().is_ok());
}

/// Failure mode 5, resolved: the remaining outputs are rearranged, not orphaned.
#[test]
fn disconnecting_the_anchor_rearranges_the_remaining_outputs() {
    let mut model = configured(
        &[panel("DP-1"), panel("DP-2"), wide("DP-3")],
        row(&["DP-1", "DP-2", "DP-3"]),
    );
    assert_eq!(model.layout().monitors.len(), 3);

    let effects = model.apply(Event::Disconnected(output_token("DP-2")));
    assert!(matches!(effects.as_slice(), [Effect::Applied { .. }]));
    let layout = model.layout();
    assert_eq!(layout.monitors.len(), 2);
    assert_eq!(layout.monitors[0].monitor, "DP-1");
    assert_eq!(layout.monitors[1].monitor, "DP-3");
    assert_no_overlap(layout);
    let adjacent = i32::try_from(layout.monitors[0].rect.width).expect("a monitor is narrower than i32");
    assert_eq!(layout.monitors[1].rect.x, adjacent);
    assert!(model.check_invariants().is_ok());

    assert_eq!(
        model.apply(Event::Disconnected(output_token("DP-2"))),
        Vec::new(),
        "a repeated unplug changes nothing"
    );
}

/// Failure mode 10: mirroring, and the mirror shapes that cannot work.
#[test]
fn mirroring_two_outputs_produces_one_monitor() {
    let outputs = [panel("DP-1"), panel("DP-2")];
    let mirrored = LayoutConfig::new(vec![entry("DP-1", Placement::Origin), mirror("DP-2", "DP-1")]);
    let layout = compute(&outputs, &mirrored).expect("a clone shows the source's mode");
    assert_eq!(layout.monitors.len(), 1);
    let monitor = &layout.monitors[0];
    assert_eq!(monitor.monitor, "DP-1", "the lowest connector name names the monitor");
    assert_eq!(monitor.members, vec![output_token("DP-1"), output_token("DP-2")]);
    assert_eq!(monitor.rect, Rect::new(0, 0, 1920, 1080));

    let chain = LayoutConfig::new(vec![
        entry("DP-1", Placement::Origin),
        mirror("DP-2", "DP-1"),
        mirror("DP-4", "DP-2"),
    ]);
    assert_eq!(
        compute(&[panel("DP-1"), panel("DP-2"), panel("DP-4")], &chain),
        Err(LayoutError::MirrorOfMirror {
            output: name("DP-4"),
            source: name("DP-2"),
        })
    );

    // A third enabled output keeps the configuration from being refused for
    // presenting nothing, so the mirror itself is what is reported.
    let disabled_source = LayoutConfig::new(vec![
        OutputConfig::disabled("DP-1"),
        mirror("DP-2", "DP-1"),
        entry("DP-3", Placement::Origin),
    ]);
    assert_eq!(
        compute(&[panel("DP-1"), panel("DP-2"), panel("DP-3")], &disabled_source),
        Err(LayoutError::MirrorSourceUnavailable {
            output: name("DP-2"),
            source: name("DP-1"),
        })
    );

    let overriding = LayoutConfig::new(vec![
        entry("DP-1", Placement::Origin),
        OutputConfig {
            mode: Some(qhd()),
            ..mirror("DP-2", "DP-1")
        },
    ]);
    assert_eq!(
        compute(&outputs, &overriding),
        Err(LayoutError::MirrorOverridesSource(name("DP-2")))
    );
}

/// Failure mode 9: a gap in the layout, declared and undeclared.
#[test]
fn an_undeclared_gap_is_rejected_and_a_declared_gap_is_allowed() {
    let outputs = [panel("DP-1"), panel("DP-2")];
    let gapped = LayoutConfig::new(vec![
        entry("DP-1", Placement::Origin),
        entry("DP-2", Placement::Absolute { x: 2000, y: 0 }),
    ]);
    assert_eq!(compute(&outputs, &gapped), Err(LayoutError::Disconnected(name("DP-2"))));

    let declared = LayoutConfig {
        allow_gaps: true,
        ..gapped
    };
    let layout = compute(&outputs, &declared).expect("a declared gap is allowed");
    assert_eq!(layout.monitors.len(), 2);
    assert_eq!(layout.monitors[1].rect.x, 2000);

    let model = configured(&outputs, declared);
    assert_eq!(model.layout().monitors.len(), 2);
    assert!(
        model.check_invariants().is_ok(),
        "a declared gap is not a broken layout"
    );
}

/// Failure mode 8: configuration shapes that cannot be resolved.
#[test]
fn a_duplicate_or_unreachable_configuration_is_rejected() {
    let outputs = [panel("DP-1"), panel("DP-2")];
    let duplicated = LayoutConfig::new(vec![entry("DP-1", Placement::Origin), entry("DP-1", Placement::Origin)]);
    assert_eq!(
        compute(&outputs, &duplicated),
        Err(LayoutError::DuplicateOutput(name("DP-1")))
    );

    let dangling = LayoutConfig::new(vec![
        entry("DP-1", Placement::Origin),
        entry("DP-2", right_of("HDMI-9")),
    ]);
    assert_eq!(
        compute(&outputs, &dangling),
        Err(LayoutError::DanglingAnchor {
            output: name("DP-2"),
            anchor: name("HDMI-9"),
        })
    );

    let cycle = LayoutConfig::new(vec![entry("DP-1", right_of("DP-2")), entry("DP-2", right_of("DP-1"))]);
    assert_eq!(
        compute(&outputs, &cycle),
        Err(LayoutError::PlacementCycle(name("DP-1")))
    );

    let unplaced = LayoutConfig::new(vec![OutputConfig {
        placement: None,
        ..OutputConfig::enabled("DP-1")
    }]);
    assert_eq!(
        compute(&outputs, &unplaced),
        Err(LayoutError::MissingPlacement(name("DP-1")))
    );
}

/// Failure mode 8 through the mode, scale and primary checks.
#[test]
fn an_unsupported_mode_scale_or_primary_is_rejected() {
    let outputs = [panel("DP-1"), panel("DP-2")];

    let mut wrong_mode = row(&["DP-1"]);
    wrong_mode.outputs[0].mode = Some(qhd());
    assert_eq!(
        compute(&outputs, &wrong_mode),
        Err(LayoutError::ModeNotSupported {
            output: name("DP-1"),
            mode: qhd(),
        })
    );

    let mut wrong_scale = row(&["DP-1"]);
    wrong_scale.outputs[0].scale = Some(Scale::from_percent(150));
    assert_eq!(
        compute(&outputs, &wrong_scale),
        Err(LayoutError::ScaleNotSupported {
            output: name("DP-1"),
            scale: Scale::from_percent(150),
        })
    );

    let mut wrong_primary = row(&["DP-1"]);
    wrong_primary.primary = Some(name("DP-2"));
    assert_eq!(
        compute(&outputs, &wrong_primary),
        Err(LayoutError::PrimaryNotEnabled(name("DP-2")))
    );

    let mut mode_less = OutputDescriptor::new(output_token("DP-3"), "DP-3", Vec::new());
    mode_less.scales = vec![Scale::integral(1)];
    assert_eq!(
        compute(&[mode_less], &row(&["DP-3"])),
        Err(LayoutError::NoPreferredMode(name("DP-3")))
    );
}

/// VRR is a request: a connector without it keeps running without it.
#[test]
fn requesting_vrr_on_an_unsupported_output_downgrades_silently() {
    let mut plain = panel("DP-1");
    plain.vrr = VrrSupport::Unsupported;
    let mut capable = panel("DP-2");
    capable.vrr = VrrSupport::Supported;

    let mut config = row(&["DP-1", "DP-2"]);
    config.outputs[0].vrr = true;
    config.outputs[1].vrr = true;
    let layout = compute(&[plain, capable], &config).expect("a row resolves");
    assert!(!layout.monitors[0].vrr, "the panel without VRR runs without it");
    assert!(layout.monitors[1].vrr);
}

/// An output's orientation reaches the layout, so a quarter turn is visible there.
#[test]
fn a_quarter_turn_changes_the_logical_monitor_size() {
    let mut config = row(&["DP-1"]);
    config.outputs[0].orientation = Orientation::Rotate90;
    let layout = compute(&[panel("DP-1")], &config).expect("one monitor resolves");
    assert_eq!(layout.monitors[0].rect, Rect::new(0, 0, 1080, 1920));
    assert_eq!(layout.monitors[0].derived_size(), layout.monitors[0].rect.size());
}

/// Failure mode 6: an EDID block that is short, corrupt or hostile.
#[test]
fn a_malformed_edid_never_panics() {
    let block = edid_block();
    let parsed = Edid::parse(&block).expect("the constructed block is valid");
    assert_eq!(parsed.manufacturer.as_deref(), Some("DEL"));
    assert_eq!(parsed.product_code, 0x1234);
    assert_eq!(parsed.serial, Some(0xDEAD_BEEF));
    assert_eq!(parsed.physical, PhysicalSize::new(600, 340));
    assert_eq!(parsed.name.as_deref(), Some("GNB 27"));
    assert_eq!(parsed.preferred_mode, Some(full_hd()));

    let descriptor = OutputDescriptor::from_edid(output_token("DP-1"), "DP-1", &parsed);
    assert_eq!(descriptor.make.as_deref(), Some("DEL"));
    assert_eq!(descriptor.serial.as_deref(), Some("3735928559"));
    assert_eq!(descriptor.preferred_mode(), Some(full_hd()));

    assert_eq!(Edid::parse(&block[..120]), Err(EdidError::TooShort { length: 120 }));
    assert_eq!(Edid::parse(&[]), Err(EdidError::TooShort { length: 0 }));
    assert_eq!(
        Edid::parse(&[0x55; 200]),
        Err(EdidError::BadHeader),
        "a block from another device is refused"
    );
    let mut bad_checksum = block.clone();
    bad_checksum[127] = bad_checksum[127].wrapping_add(1);
    assert_eq!(Edid::parse(&bad_checksum), Err(EdidError::Checksum));

    // Every truncation and every single-byte mutation must be answered, not fatal.
    for length in 0..=block.len() {
        let _ = Edid::parse(&block[..length]);
    }
    for index in 0..block.len() {
        for mask in [0xFF_u8, 0x01, 0x80, 0x10] {
            let mut mutated = block.clone();
            mutated[index] ^= mask;
            let _ = Edid::parse(&mutated).map(|edid| edid.preferred_mode);
        }
    }

    // A deterministic sweep of unrelated bytes, which is where a bad slice index or
    // a division by a zero total would show up.
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    for _ in 0..512 {
        let mut bytes = vec![0_u8; 128];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        bytes[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        let sum = bytes.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        bytes[127] = 0_u8.wrapping_sub(sum);
        let _ = Edid::parse(&bytes);
    }
}

/// A valid 128-byte base block: DEL, product 0x1234, serial 0xDEADBEEF, 600x340 mm,
/// named "GNB 27", preferring 1920x1080 at 60 Hz.
fn edid_block() -> Vec<u8> {
    let mut bytes = vec![0_u8; 128];
    bytes[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    let manufacturer = (u16::from(b'D' - 64) << 10) | (u16::from(b'E' - 64) << 5) | u16::from(b'L' - 64);
    bytes[8..10].copy_from_slice(&manufacturer.to_be_bytes());
    bytes[10..12].copy_from_slice(&0x1234_u16.to_le_bytes());
    bytes[12..16].copy_from_slice(&0xDEAD_BEEF_u32.to_le_bytes());
    bytes[21] = 60;
    bytes[22] = 34;

    let timing = [
        0x02_u8, 0x3A, 0x80, 0x18, 0x71, 0x38, 0x2D, 0x40, 0x45, 0x40, 0xA0, 0x30, 0x30, 0x35, 0x00, 0x00, 0x00, 0x1E,
    ];
    bytes[54..72].copy_from_slice(&timing);

    let mut descriptor = [0_u8; 18];
    descriptor[3] = 0xFC;
    let name = b"GNB 27";
    descriptor[5..5 + name.len()].copy_from_slice(name);
    bytes[72..90].copy_from_slice(&descriptor);

    let sum = bytes.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    bytes[127] = 0_u8.wrapping_sub(sum);
    bytes
}
