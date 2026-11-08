//! One test per documented failure mode, plus the behaviours the state machine
//! exists to provide.

use gnoblin_types::{Insets, OutputId, Rect, Scale, Tick, WindowId, WorkspaceId};
use gnoblin_wm::{
    ActivationDecision, DragConstraint, Effect, Event, FocusMode, MAX_RULES, Match, Pattern, PlacementMode, Rejection,
    Rule, RuleError, RuleFields, RuleRejection, SnapZone, WindowSpec, Wm, WmOptions, WorkspaceDirection,
    WorkspaceSelector, WorkspaceSet, WorkspaceSetEntry, WorkspaceSetError,
};

const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

fn wm_with(options: WmOptions) -> Wm {
    let mut wm = Wm::new(options);
    for raw in 1..=3 {
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(raw),
        });
    }
    wm.apply(Event::OutputConnected {
        output: OutputId::new(0),
        rect: MONITOR,
    });
    wm
}

fn default_wm() -> Wm {
    wm_with(WmOptions::default())
}

fn window(raw: u64) -> WindowId {
    WindowId::new(raw)
}

fn open(wm: &mut Wm, raw: u64) {
    let effects = wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(raw), "org.example.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert!(
        effects.iter().any(|effect| matches!(effect, Effect::Configure { .. })),
        "opening a window configures it"
    );
}

fn focus(wm: &mut Wm, raw: u64, decision: ActivationDecision) -> Vec<Effect> {
    wm.apply(Event::ActivationRequested {
        window: window(raw),
        decision,
    })
}

fn rules(wm: &mut Wm, rules: Vec<Rule>) {
    wm.apply(Event::RulesChanged { rules });
}

fn rule(matcher: Match, fields: RuleFields) -> Rule {
    Rule { matcher, fields }
}

fn place(selector: WorkspaceSelector) -> RuleFields {
    RuleFields {
        workspace: Some(selector),
        ..RuleFields::default()
    }
}

fn app(app_id: &str) -> Match {
    Match {
        app_id: Some(Pattern::new(app_id).expect("test pattern is valid")),
        ..Match::default()
    }
}

/// Fails the test on the first broken invariant, and returns how many checks ran.
fn check(wm: &Wm) -> usize {
    match wm.check_invariants() {
        Ok(checks) => checks,
        Err(broken) => panic!("broken invariant: {broken:?}"),
    }
}

fn configured(effects: &[Effect], id: WindowId) -> Option<Rect> {
    effects.iter().find_map(|effect| match effect {
        Effect::Configure { window, geometry, .. } if *window == id => Some(*geometry),
        _ => None,
    })
}

fn visibility(effects: &[Effect], id: WindowId) -> Option<bool> {
    effects.iter().find_map(|effect| match effect {
        Effect::SetVisible { window, visible } if *window == id => Some(*visible),
        _ => None,
    })
}

// Failure mode 1: focus lands on a closed window.
#[test]
fn focus_never_lands_on_a_closed_window() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    focus(&mut wm, 2, ActivationDecision::Allow);
    assert_eq!(wm.focused(), Some(window(2)));

    let effects = wm.apply(Event::WindowClosed { window: window(2) });
    assert_eq!(wm.focused(), Some(window(1)));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Focus {
                previous: Some(previous),
                focused: Some(focused)
            } if *previous == window(2) && *focused == window(1)
        )),
        "closing the focused window reports the focus change"
    );
    assert!(!wm.focus_history().contains(&window(2)));
    assert!(!wm.stack().contains(&window(2)));

    let effects = focus(&mut wm, 2, ActivationDecision::Allow);
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::UnknownWindow(window(2))
        }]
    );
    assert_eq!(wm.focused(), Some(window(1)));

    wm.apply(Event::WindowClosed { window: window(1) });
    assert_eq!(wm.focused(), None, "no window can hold focus");
    check(&wm);
}

// Failure mode 2: minimised windows keep focus.
#[test]
fn minimising_the_focused_window_moves_focus_away() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    focus(&mut wm, 2, ActivationDecision::Allow);

    let effects = wm.apply(Event::Minimise { window: window(2) });
    assert_eq!(wm.focused(), Some(window(1)));
    assert_eq!(visibility(&effects, window(2)), Some(false));
    assert!(!wm.is_visible(window(2)));

    // Re-minimising changes nothing at all.
    assert!(wm.apply(Event::Minimise { window: window(2) }).is_empty());
    assert_eq!(wm.focused(), Some(window(1)));

    // Minimising an unfocused window leaves focus alone.
    wm.apply(Event::Minimise { window: window(1) });
    assert_eq!(wm.focused(), None);
    check(&wm);

    // Unminimising with no focus takes focus, because a visible desktop with no
    // focused window cannot receive a key press.
    wm.apply(Event::Unminimise { window: window(2) });
    assert_eq!(wm.focused(), Some(window(2)));
    check(&wm);
}

// Failure mode 3: a window is placed off every output after hotplug.
#[test]
fn unplugging_an_output_keeps_windows_on_a_valid_output() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1920, 1080),
    });
    open(&mut wm, 1);
    open(&mut wm, 2);
    wm.apply(Event::ToggleMaximise { window: window(1) });
    wm.apply(Event::ToggleFullscreen { window: window(2) });
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(Some(OutputId::new(0))));

    let effects = wm.apply(Event::OutputDisconnected {
        output: OutputId::new(0),
    });
    let survivor_area = Rect::new(1920, 0, 1920, 1080);
    for raw in [1, 2] {
        let managed = wm.window(window(raw));
        assert_eq!(
            managed.map(|w| w.output),
            Some(Some(OutputId::new(1))),
            "window {raw} moved to the surviving output"
        );
    }
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(survivor_area),
        "a maximised window takes the new work area"
    );
    assert_eq!(
        wm.window(window(2)).map(|w| w.geometry),
        Some(survivor_area),
        "a fullscreen window takes the new monitor rectangle"
    );
    assert!(configured(&effects, window(1)).is_some());
    assert!(configured(&effects, window(2)).is_some());
    check(&wm);
}

#[test]
fn unplugging_the_last_output_hides_every_window() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    focus(&mut wm, 1, ActivationDecision::Allow);

    let effects = wm.apply(Event::OutputDisconnected {
        output: OutputId::new(0),
    });
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(None));
    assert_eq!(visibility(&effects, window(1)), Some(false));
    assert!(!wm.is_visible(window(1)));
    assert_eq!(wm.focused(), None, "a window with no output cannot hold focus");
    check(&wm);

    // Reconnecting adopts the window again instead of stranding it.
    let effects = wm.apply(Event::OutputConnected {
        output: OutputId::new(0),
        rect: MONITOR,
    });
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(Some(OutputId::new(0))));
    assert_eq!(visibility(&effects, window(1)), Some(true));
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

// Failure mode 4: stacking order inconsistent with focus.
#[test]
fn raising_keeps_the_fullscreen_layer_on_top() {
    let mut wm = default_wm();
    for raw in 1..=3 {
        open(&mut wm, raw);
    }
    assert_eq!(wm.stack(), [window(3), window(2), window(1)]);

    wm.apply(Event::ToggleFullscreen { window: window(2) });
    assert_eq!(wm.stack(), [window(2), window(3), window(1)]);

    let effects = focus(&mut wm, 1, ActivationDecision::Allow);
    assert!(
        effects.iter().any(|effect| matches!(effect, Effect::Stack { .. })),
        "activation with raise reports the new order"
    );
    assert_eq!(
        wm.stack(),
        [window(2), window(1), window(3)],
        "a fullscreen window stays above the window that raised"
    );
    check(&wm);

    // Lowering the fullscreen window cannot put it under normal windows.
    wm.apply(Event::Lower { window: window(2) });
    assert_eq!(wm.stack(), [window(2), window(1), window(3)]);
    check(&wm);

    // Leaving fullscreen drops it into the normal layer, where raise and lower apply.
    wm.apply(Event::ToggleFullscreen { window: window(2) });
    assert_eq!(wm.stack(), [window(2), window(1), window(3)]);
    wm.apply(Event::Lower { window: window(2) });
    assert_eq!(wm.stack(), [window(1), window(3), window(2)]);
    check(&wm);
}

#[test]
fn focus_without_raise_leaves_the_stack_alone() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    let before = wm.stack().to_vec();
    let effects = focus(&mut wm, 1, ActivationDecision::AllowWithoutRaise);
    assert_eq!(wm.stack(), before);
    assert!(
        effects.iter().any(|effect| matches!(effect, Effect::Focus { .. })),
        "focus changed even though the order did not"
    );
    check(&wm);
}

// Failure mode 5: a rule applied again when its match does not change.
#[test]
fn rules_do_not_reapply_while_the_same_rules_match() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            app("^org.example.Editor$"),
            RuleFields {
                workspace: Some(WorkspaceSelector::Id(WorkspaceId::new(2))),
                maximise: Some(true),
                secure: None,
                opacity: None,
                corner_radius: None,
            },
        )],
    );
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.example.Editor").with_title("Untitled 1"),
    ));
    let managed = |wm: &Wm| {
        wm.window(window(1))
            .map(|w| (w.workspace, w.state.maximised, w.applied_rules, w.geometry))
    };
    let after_open = managed(&wm);
    assert_eq!(
        after_open,
        Some((WorkspaceId::new(2), true, 1, MONITOR)),
        "the rule placed and maximised the window once"
    );

    assert!(
        wm.apply(Event::WindowTitleChanged {
            window: window(1),
            title: "notes.txt".to_owned(),
        })
        .is_empty(),
        "a title change that keeps the same rules matching changes nothing"
    );
    assert!(
        wm.apply(Event::WindowAppIdChanged {
            window: window(1),
            app_id: "org.example.Other".to_owned(),
        })
        .is_empty(),
        "losing a match does not undo what the rule already did"
    );
    rules(
        &mut wm,
        vec![rule(Match::default(), place(WorkspaceSelector::Number(1)))],
    );
    // The rules reload above does not run, so the window keeps the placement it had.
    assert_eq!(
        wm.window(window(1))
            .map(|w| (w.workspace, w.state.maximised, w.geometry)),
        Some((WorkspaceId::new(2), true, MONITOR)),
        "losing a match leaves the window where the rule put it"
    );
    check(&wm);
}

#[test]
fn a_title_that_newly_matches_a_rule_applies_it() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            Match {
                title: Some(Pattern::new("^Notes$").expect("test pattern is valid")),
                ..Match::default()
            },
            place(WorkspaceSelector::Id(WorkspaceId::new(2))),
        )],
    );
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.example.App").with_title("Untitled"),
    ));
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(1)));
    let effects = wm.apply(Event::WindowTitleChanged {
        window: window(1),
        title: "Notes".to_owned(),
    });
    let managed = wm.window(window(1));
    assert_eq!(managed.map(|w| w.workspace), Some(WorkspaceId::new(2)));
    assert_eq!(managed.map(|w| w.applied_rules), Some(1));
    assert_eq!(
        visibility(&effects, window(1)),
        Some(false),
        "the window left the active workspace, so it is hidden"
    );
    check(&wm);
}

#[test]
fn an_app_id_that_newly_matches_a_rule_applies_it() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            app("^org.example.Editor$"),
            place(WorkspaceSelector::Id(WorkspaceId::new(2))),
        )],
    );
    open(&mut wm, 1);
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(1)));
    wm.apply(Event::WindowAppIdChanged {
        window: window(1),
        app_id: "org.example.Editor".to_owned(),
    });
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(2)));
    assert_eq!(wm.window(window(1)).map(|w| w.applied_rules), Some(1));
    check(&wm);
}

// A re-match merges every rule that now matches, so the last rule in the list still
// wins even when an earlier rule is the one that newly matched.
#[test]
fn a_re_match_merges_all_matching_rules_in_list_order() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![
            rule(
                Match {
                    title: Some(Pattern::new("^Notes$").expect("test pattern is valid")),
                    ..Match::default()
                },
                place(WorkspaceSelector::Id(WorkspaceId::new(2))),
            ),
            rule(
                app("^org.example.Editor$"),
                place(WorkspaceSelector::Id(WorkspaceId::new(3))),
            ),
        ],
    );
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.example.Editor").with_title("Untitled"),
    ));
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(3)));
    wm.apply(Event::WindowTitleChanged {
        window: window(1),
        title: "Notes".to_owned(),
    });
    assert_eq!(
        wm.window(window(1)).map(|w| (w.workspace, w.applied_rules)),
        Some((WorkspaceId::new(3), 2)),
        "rule 1 is still last among the two that match, so it still wins"
    );
    check(&wm);
}

// Failure mode 6: a workspace switch loses a fullscreen window.
#[test]
fn switching_workspace_keeps_the_fullscreen_window_intact() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    focus(&mut wm, 1, ActivationDecision::Allow);
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), Some(MONITOR));

    let effects = wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(2),
    });
    assert_eq!(visibility(&effects, window(1)), Some(false));
    assert!(!wm.is_visible(window(1)));
    assert_eq!(wm.focused(), None);
    check(&wm);

    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(1),
    });
    let managed = wm.window(window(1));
    assert_eq!(managed.map(|w| w.state.fullscreen), Some(true));
    assert_eq!(managed.map(|w| w.geometry), Some(MONITOR));
    assert_eq!(managed.map(|w| w.workspace), Some(WorkspaceId::new(1)));
    assert_eq!(wm.stack(), [window(1)]);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

// Failure mode 7: a rule loops.
#[test]
fn contradicting_rules_settle_on_the_last_match() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![
            rule(Match::default(), place(WorkspaceSelector::Id(WorkspaceId::new(2)))),
            rule(Match::default(), place(WorkspaceSelector::Id(WorkspaceId::new(3)))),
        ],
    );
    let effects = wm.apply(Event::WindowOpened(WindowSpec::normal(window(1), "org.example.App")));
    assert_eq!(
        wm.window(window(1)).map(|w| w.workspace),
        Some(WorkspaceId::new(3)),
        "the last matching rule wins"
    );
    assert_eq!(wm.window(window(1)).map(|w| w.applied_rules), Some(2));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Configure { .. }))
            .count(),
        1,
        "one configure per open, however many rules matched"
    );
    check(&wm);
}

#[test]
fn a_rule_naming_a_missing_workspace_reports_a_rejection() {
    let mut wm = default_wm();
    let selector = WorkspaceSelector::Id(WorkspaceId::new(99));
    rules(&mut wm, vec![rule(Match::default(), place(selector))]);
    let effects = wm.apply(Event::WindowOpened(WindowSpec::normal(window(1), "org.example.App")));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::RuleRejected {
                rule: 0,
                reason: RuleRejection::MissingWorkspace(found),
                ..
            } if *found == selector
        )),
        "the caller is told the placement target does not exist"
    );
    assert_eq!(
        wm.window(window(1)).map(|w| w.workspace),
        Some(WorkspaceId::new(1)),
        "the window stays on the active workspace"
    );
    assert_eq!(
        wm.window(window(1)).map(|w| w.applied_rules),
        Some(0),
        "a rejected rule is not counted as applied"
    );
    check(&wm);
}

// A reload that fails validation must leave the rules already in force untouched.
#[test]
fn a_refused_rules_list_leaves_the_rules_in_force() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            Match::default(),
            place(WorkspaceSelector::Id(WorkspaceId::new(2))),
        )],
    );
    let refused = vec![rule(Match::default(), RuleFields::default()); MAX_RULES + 1];
    let effects = wm.apply(Event::RulesChanged { rules: refused });
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Rejected {
                reason: Rejection::Rules(RuleError::TooManyRules { count, max }),
            } if *count == MAX_RULES + 1 && *max == MAX_RULES
        )),
        "a list over the cap is refused and the refusal names the count"
    );
    open(&mut wm, 1);
    assert_eq!(
        wm.window(window(1)).map(|w| w.workspace),
        Some(WorkspaceId::new(2)),
        "the rules already in force still apply to a new window"
    );
    check(&wm);
}

// Failure mode 9: a snapped or maximised window drifting from its zone.
#[test]
fn reserved_space_reapplies_to_maximised_windows() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::ToggleMaximise { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), Some(MONITOR));

    let reserved = Insets {
        top: 32,
        ..Insets::ZERO
    };
    let effects = wm.apply(Event::OutputReserved {
        output: OutputId::new(0),
        reserved,
    });
    let area = Rect::new(0, 32, 1920, 1048);
    assert_eq!(wm.output(OutputId::new(0)).map(|o| o.work_area()), Some(area));
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), Some(area));
    assert_eq!(configured(&effects, window(1)), Some(area));
    check(&wm);
}

#[test]
fn reserved_space_reapplies_to_snapped_windows() {
    let mut wm = wm_with(WmOptions {
        edge_tiling: true,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    wm.apply(Event::SnapRequested {
        window: window(1),
        zone: SnapZone::Right,
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(960, 0, 960, 1080))
    );
    wm.apply(Event::OutputReserved {
        output: OutputId::new(0),
        reserved: Insets {
            left: 64,
            ..Insets::ZERO
        },
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(992, 0, 928, 1080)),
        "the snapped window follows the new work area"
    );
    check(&wm);
}

#[test]
fn an_output_move_reclamps_floating_windows() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(1600, 900, 600, 400),
    });
    wm.apply(Event::OutputMoved {
        output: OutputId::new(0),
        rect: Rect::new(0, 0, 1280, 720),
    });
    let geometry = wm.window(window(1)).map(|w| w.geometry);
    assert_eq!(geometry, Some(Rect::new(680, 320, 600, 400)));
    check(&wm);
}

// Snapping.
#[test]
fn snapping_and_unsnapping_restore_the_floating_rectangle() {
    let mut wm = wm_with(WmOptions {
        edge_tiling: true,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    let floating = wm.window(window(1)).map(|w| w.geometry);
    assert_eq!(floating, Some(Rect::new(0, 0, 600, 400)));

    wm.apply(Event::SnapRequested {
        window: window(1),
        zone: SnapZone::Left,
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(0, 0, 960, 1080))
    );
    assert_eq!(
        wm.window(window(1)).map(|w| w.state.snapped),
        Some(Some(SnapZone::Left))
    );

    wm.apply(Event::Unsnap { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), floating);
    assert_eq!(wm.window(window(1)).map(|w| w.state.snapped), Some(None));
    assert!(wm.apply(Event::Unsnap { window: window(1) }).is_empty());
    check(&wm);
}

#[test]
fn snapping_a_maximised_window_floats_it_and_unsnapping_restores_it() {
    let mut wm = wm_with(WmOptions {
        edge_tiling: true,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    let floating = wm.window(window(1)).map(|w| w.geometry);
    wm.apply(Event::ToggleMaximise { window: window(1) });
    wm.apply(Event::SnapRequested {
        window: window(1),
        zone: SnapZone::BottomRight,
    });
    let managed = wm.window(window(1));
    assert_eq!(managed.map(|w| w.state.maximised), Some(false));
    assert_eq!(managed.map(|w| w.state.snapped), Some(Some(SnapZone::BottomRight)));

    wm.apply(Event::Unsnap { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), floating);
    check(&wm);
}

#[test]
fn snapping_is_refused_when_edge_tiling_is_off() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let effects = wm.apply(Event::SnapRequested {
        window: window(1),
        zone: SnapZone::Left,
    });
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::EdgeTilingDisabled
        }]
    );
    assert_eq!(wm.window(window(1)).map(|w| w.state.snapped), Some(None));
    check(&wm);
}

// Move and resize constraints.
#[test]
fn a_drag_is_constrained_to_the_work_area() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let effects = wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(-400, -400, 600, 400),
    });
    assert_eq!(configured(&effects, window(1)), Some(Rect::new(0, 0, 600, 400)));
    check(&wm);
}

#[test]
fn a_drag_may_leave_a_visible_strip_when_configured() {
    let mut wm = wm_with(WmOptions {
        drag_constraint: DragConstraint::MinimumStrip,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(-5000, -5000, 600, 400),
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(-525, -325, 600, 400)),
        "75 pixels of the window stay on screen"
    );
    check(&wm);
}

#[test]
fn dragging_a_maximised_window_floats_it_and_fullscreen_refuses_the_drag() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::ToggleMaximise { window: window(1) });
    wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(300, 200, 500, 400),
    });
    let managed = wm.window(window(1));
    assert_eq!(managed.map(|w| w.state.maximised), Some(false));
    assert_eq!(managed.map(|w| w.geometry), Some(Rect::new(300, 200, 500, 400)));

    wm.apply(Event::ToggleFullscreen { window: window(1) });
    let effects = wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(0, 0, 100, 100),
    });
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::NotFloating(window(1))
        }]
    );
    check(&wm);
}

// Toggle chains.
#[test]
fn fullscreen_returns_to_the_maximised_or_floating_rectangle() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let floating = wm.window(window(1)).map(|w| w.geometry);
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), floating);

    wm.apply(Event::ToggleMaximise { window: window(1) });
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    assert_eq!(
        wm.window(window(1)).map(|w| w.state.maximised),
        Some(true),
        "fullscreen does not clear maximise"
    );
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    assert_eq!(wm.window(window(1)).map(|w| w.geometry), Some(MONITOR));
    check(&wm);
}

#[test]
fn maximising_a_fullscreen_window_is_refused() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    let effects = wm.apply(Event::ToggleMaximise { window: window(1) });
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::NotFloating(window(1))
        }]
    );
    check(&wm);
}

// Transients.
#[test]
fn a_dialog_opens_above_its_parent_on_the_parents_workspace() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::MoveResize {
        window: window(1),
        geometry: Rect::new(200, 150, 800, 600),
    });
    wm.apply(Event::WindowOpened(
        WindowSpec::transient(window(2), "org.example.App", window(1)).with_requested(Rect::new(0, 0, 300, 200)),
    ));
    let dialog = wm.window(window(2));
    assert_eq!(
        dialog.map(|w| w.geometry),
        Some(Rect::new(450, 350, 300, 200)),
        "a dialog is centred over its parent"
    );
    assert_eq!(dialog.map(|w| w.workspace), Some(WorkspaceId::new(1)));
    assert_eq!(dialog.map(|w| w.transient_for), Some(Some(window(1))));
    let order = wm.stack();
    let dialog_at = order.iter().position(|id| *id == window(2));
    let parent_at = order.iter().position(|id| *id == window(1));
    assert!(dialog_at < parent_at, "the dialog is above its parent");
    check(&wm);
}

#[test]
fn workspace_rules_do_not_move_a_dialog_away_from_its_parent() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            Match::default(),
            place(WorkspaceSelector::Id(WorkspaceId::new(2))),
        )],
    );
    open(&mut wm, 1);
    wm.apply(Event::WindowOpened(WindowSpec::transient(
        window(2),
        "org.example.App",
        window(1),
    )));
    assert_eq!(
        wm.window(window(2)).map(|w| w.workspace),
        Some(WorkspaceId::new(2)),
        "the dialog follows the parent's workspace even when a rule targets another"
    );
    check(&wm);
}

// Workspaces.
#[test]
fn removing_a_workspace_moves_its_windows_and_keeps_focus_valid() {
    let mut wm = default_wm();
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(2),
    });
    open(&mut wm, 1);
    focus(&mut wm, 1, ActivationDecision::Allow);
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(3),
    });

    let effects = wm.apply(Event::WorkspaceRemoved {
        workspace: WorkspaceId::new(2),
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.workspace),
        Some(WorkspaceId::new(1)),
        "the window moves to the neighbouring workspace"
    );
    assert_eq!(wm.active_workspace(), Some(WorkspaceId::new(3)));
    assert!(!wm.is_visible(window(1)));
    assert_eq!(visibility(&effects, window(1)), Some(false));
    check(&wm);

    wm.apply(Event::WorkspaceRemoved {
        workspace: WorkspaceId::new(3),
    });
    let effects = wm.apply(Event::WorkspaceRemoved {
        workspace: WorkspaceId::new(1),
    });
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::LastWorkspace(WorkspaceId::new(1))
        }],
        "the last workspace cannot be removed"
    );
    check(&wm);
}

#[test]
fn removing_the_active_workspace_shows_its_neighbour_and_refocuses() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    focus(&mut wm, 1, ActivationDecision::Allow);
    wm.apply(Event::WorkspaceRemoved {
        workspace: WorkspaceId::new(1),
    });
    assert_eq!(wm.active_workspace(), Some(WorkspaceId::new(2)));
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(2)));
    assert!(wm.is_visible(window(1)));
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

#[test]
fn a_window_needs_a_workspace_and_an_output() {
    let mut empty = Wm::new(WmOptions::default());
    assert_eq!(
        empty.apply(Event::WindowOpened(WindowSpec::normal(window(1), "app"))),
        vec![Effect::Rejected {
            reason: Rejection::NoWorkspace
        }]
    );
    empty.apply(Event::WorkspaceAdded {
        workspace: WorkspaceId::new(1),
    });
    assert_eq!(
        empty.apply(Event::WindowOpened(WindowSpec::normal(window(1), "app"))),
        vec![Effect::Rejected {
            reason: Rejection::NoOutput
        }]
    );
    assert_eq!(empty.windows().count(), 0);
    check(&empty);
}

// Placement modes.
#[test]
fn free_space_placement_avoids_existing_windows() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    let first = wm.window(window(1)).map(|w| w.geometry);
    let second = wm.window(window(2)).map(|w| w.geometry);
    assert_eq!(first, Some(Rect::new(0, 0, 600, 400)));
    match (first, second) {
        (Some(first), Some(second)) => assert!(!first.intersects(second)),
        _ => panic!("both windows are managed"),
    }
    check(&wm);
}

#[test]
fn centered_placement_opens_new_windows_in_the_middle() {
    let mut wm = wm_with(WmOptions {
        placement: PlacementMode::Center,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(660, 340, 600, 400))
    );
    open(&mut wm, 2);
    assert_ne!(
        wm.window(window(2)).map(|w| w.geometry),
        wm.window(window(1)).map(|w| w.geometry),
        "the second centred window cascades instead of covering the first"
    );
    check(&wm);
}

#[test]
fn new_windows_open_on_the_focused_windows_output() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1920, 1080),
    });
    open(&mut wm, 1);
    // Unplugging the first output moves the window to the second, and focusing it
    // there is what makes the next window open on the same monitor.
    wm.apply(Event::OutputDisconnected {
        output: OutputId::new(0),
    });
    focus(&mut wm, 1, ActivationDecision::Allow);
    open(&mut wm, 2);
    assert_eq!(
        wm.window(window(2)).map(|w| w.output),
        Some(Some(OutputId::new(1))),
        "a new window follows the focused window's output"
    );
    check(&wm);
}

// No-op events.
#[test]
fn no_op_events_produce_no_effects() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    focus(&mut wm, 1, ActivationDecision::Allow);
    assert!(
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(1),
        })
        .is_empty()
    );
    assert_eq!(
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        }),
        vec![Effect::Rejected {
            reason: Rejection::DuplicateWorkspace(WorkspaceId::new(1))
        }],
        "a duplicate workspace is reported, not applied"
    );
    assert!(wm.apply(Event::Unsnap { window: window(1) }).is_empty());
    assert!(
        wm.apply(Event::RulesChanged { rules: Vec::new() }).is_empty(),
        "a rules reload changes no window"
    );
    check(&wm);
}

#[test]
fn a_denied_activation_reports_the_decision_without_changing_focus() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    let effects = focus(&mut wm, 2, ActivationDecision::Deny);
    assert_eq!(effects, vec![Effect::ActivationDenied { window: window(2) }]);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

#[test]
fn activating_a_hidden_window_is_refused() {
    let mut wm = default_wm();
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(2),
    });
    open(&mut wm, 1);
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(1),
    });
    let effects = focus(&mut wm, 1, ActivationDecision::Allow);
    assert_eq!(
        effects,
        vec![Effect::Rejected {
            reason: Rejection::NotVisible(window(1))
        }]
    );
    assert_eq!(wm.focused(), None);
    check(&wm);
}

#[test]
fn duplicate_and_unknown_identities_are_rejected() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    assert_eq!(
        wm.apply(Event::WindowOpened(WindowSpec::normal(window(1), "app"))),
        vec![Effect::Rejected {
            reason: Rejection::DuplicateWindow(window(1))
        }]
    );
    assert_eq!(
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: MONITOR
        }),
        vec![Effect::Rejected {
            reason: Rejection::DuplicateOutput(OutputId::new(0))
        }]
    );
    assert_eq!(
        wm.apply(Event::WindowClosed { window: window(9) }),
        vec![Effect::Rejected {
            reason: Rejection::UnknownWindow(window(9))
        }]
    );
    assert_eq!(
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(9)
        }),
        vec![Effect::Rejected {
            reason: Rejection::UnknownWorkspace(WorkspaceId::new(9))
        }]
    );
    check(&wm);
}

// Failure mode: pointer focus. Focus follows the pointer only as `focus_mode` asks.
fn pointer_wm(mode: FocusMode, rest: bool) -> Wm {
    let mut wm = wm_with(WmOptions {
        focus_mode: mode,
        focus_change_on_pointer_rest: rest,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    assert_eq!(wm.focused(), Some(window(1)), "the first window is focused");
    wm
}

fn pointer_moved(wm: &mut Wm, over: Option<u64>, ms: u64) {
    wm.apply(Event::PointerMoved {
        window: over.map(window),
        at: Tick::new(ms),
    });
}

fn pointer_tick(wm: &mut Wm, ms: u64) {
    wm.apply(Event::PointerTick { at: Tick::new(ms) });
}

#[test]
fn click_mode_ignores_pointer_motion() {
    let mut wm = pointer_wm(FocusMode::Click, false);
    pointer_moved(&mut wm, Some(2), 0);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

#[test]
fn hover_keeps_focus_when_the_pointer_crosses_a_gap() {
    let mut wm = pointer_wm(FocusMode::Hover, false);
    pointer_moved(&mut wm, Some(2), 0);
    assert_eq!(wm.focused(), Some(window(2)));
    pointer_moved(&mut wm, None, 10);
    assert_eq!(wm.focused(), Some(window(2)));
    check(&wm);
}

#[test]
fn hover_strict_clears_focus_when_the_pointer_crosses_a_gap() {
    let mut wm = pointer_wm(FocusMode::HoverStrict, false);
    pointer_moved(&mut wm, Some(2), 0);
    assert_eq!(wm.focused(), Some(window(2)));
    pointer_moved(&mut wm, None, 10);
    assert_eq!(wm.focused(), None);
    check(&wm);
}

#[test]
fn motion_cancels_a_pending_rest_focus() {
    let mut wm = pointer_wm(FocusMode::Hover, true);
    pointer_moved(&mut wm, Some(2), 0);
    pointer_moved(&mut wm, None, 50);
    pointer_tick(&mut wm, 400);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

// Failure mode: raise on click and auto raise. Raising never moves pointer focus,
// and an armed auto raise is cancelled by leaving, focus moving or closing.
fn raise_wm(auto: bool, delay: u64) -> Wm {
    let mut wm = wm_with(WmOptions {
        focus_mode: FocusMode::Hover,
        auto_raise: auto,
        auto_raise_delay: delay,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    wm
}

#[test]
fn a_click_on_a_buried_window_raises_it() {
    let mut wm = raise_wm(false, 0);
    assert_eq!(wm.stack().first(), Some(&window(1)));
    wm.apply(Event::PointerPressed { window: window(2) });
    assert_eq!(wm.stack().first(), Some(&window(2)));
    check(&wm);
}

#[test]
fn a_click_with_raise_on_click_off_keeps_the_stacking() {
    let mut wm = wm_with(WmOptions {
        raise_on_click: false,
        ..WmOptions::default()
    });
    open(&mut wm, 1);
    open(&mut wm, 2);
    let before = wm.stack().to_vec();
    wm.apply(Event::PointerPressed { window: window(1) });
    assert_eq!(wm.stack(), before.as_slice());
    check(&wm);
}

#[test]
fn auto_raise_is_cancelled_by_leaving_the_window() {
    let mut wm = raise_wm(true, 500);
    pointer_moved(&mut wm, Some(2), 100);
    pointer_moved(&mut wm, None, 200);
    pointer_tick(&mut wm, 1000);
    assert_eq!(wm.stack().first(), Some(&window(1)));
    check(&wm);
}

#[test]
fn closing_the_armed_window_cancels_its_auto_raise() {
    let mut wm = raise_wm(true, 500);
    pointer_moved(&mut wm, Some(2), 0);
    wm.apply(Event::WindowClosed { window: window(2) });
    pointer_tick(&mut wm, 1000);
    check(&wm);
}

#[test]
fn a_pointer_resting_over_a_window_focuses_it_once_settled() {
    let mut wm = pointer_wm(FocusMode::Hover, true);
    pointer_moved(&mut wm, Some(2), 0);
    pointer_moved(&mut wm, Some(2), 100);
    pointer_tick(&mut wm, 250);
    assert_eq!(wm.focused(), Some(window(1)), "the timer restarted at the last motion");
    pointer_tick(&mut wm, 300);
    assert_eq!(wm.focused(), Some(window(2)));
    check(&wm);
}

#[test]
fn a_closed_window_cancels_its_pending_rest_focus() {
    let mut wm = pointer_wm(FocusMode::Hover, true);
    pointer_moved(&mut wm, Some(2), 0);
    wm.apply(Event::WindowClosed { window: window(2) });
    pointer_tick(&mut wm, 400);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

#[test]
fn a_focus_change_while_a_button_is_held_is_ignored() {
    let mut wm = pointer_wm(FocusMode::HoverStrict, false);
    pointer_moved(&mut wm, Some(2), 0);
    assert_eq!(wm.focused(), Some(window(2)));
    wm.apply(Event::PointerGrab { active: true });
    pointer_moved(&mut wm, None, 10);
    assert_eq!(wm.focused(), Some(window(2)), "a drag across a gap keeps its focus");
    wm.apply(Event::PointerGrab { active: false });
    check(&wm);
}

#[test]
fn a_rest_focus_does_not_fire_while_a_button_is_held() {
    let mut wm = pointer_wm(FocusMode::Hover, true);
    pointer_moved(&mut wm, Some(2), 0);
    wm.apply(Event::PointerGrab { active: true });
    pointer_tick(&mut wm, 400);
    assert_eq!(wm.focused(), Some(window(1)));
    check(&wm);
}

// The surface record of a window, read by its window scope and the window manager's
// provider. `None` once the window has closed and its record has left the registry.
fn secure_of(wm: &Wm, raw: u64) -> Option<bool> {
    wm.layers()
        .records()
        .find(|record| {
            record.spec.scope == gnoblin_layers::LayerScope::Window(Some(window(raw)))
                && record.provider == gnoblin_layers::LayerProvider::Gnoblin("gnoblin-wm")
        })
        .map(|record| wm.layers().effective_secure(record))
}

fn secure_effect(effects: &[Effect], raw: u64) -> Option<bool> {
    effects.iter().find_map(|effect| match effect {
        Effect::SecureChanged { window: id, secure } if *id == window(raw) => Some(*secure),
        _ => None,
    })
}

fn secret_rule() -> Rule {
    rule(
        app("org.secret.App"),
        RuleFields {
            secure: Some(true),
            ..RuleFields::default()
        },
    )
}

// A window matching a `secure` rule has its surface marked secure, so capture masks it.
#[test]
fn a_secure_rule_marks_the_matching_window_secure() {
    let mut wm = default_wm();
    rules(&mut wm, vec![secret_rule()]);
    let effects = wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.secret.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert_eq!(
        secure_effect(&effects, 1),
        Some(true),
        "the matched window turns secure"
    );
    assert_eq!(secure_of(&wm, 1), Some(true));
    check(&wm);
}

// A window the rule does not match keeps its surface unmasked, and gets no effect.
#[test]
fn a_non_matching_window_is_not_secure() {
    let mut wm = default_wm();
    rules(&mut wm, vec![secret_rule()]);
    let effects = wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.example.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert_eq!(
        secure_effect(&effects, 1),
        None,
        "an unmatched window raises no secure change"
    );
    assert_eq!(secure_of(&wm, 1), Some(false));
    check(&wm);
}

// Opacity outside 0 to 1 is refused by name, and the rules already in force stay.
#[test]
fn an_out_of_range_opacity_is_refused_and_the_old_rules_stay() {
    let mut wm = default_wm();
    rules(&mut wm, vec![secret_rule()]);
    let effects = wm.apply(Event::RulesChanged {
        rules: vec![rule(
            app("org.faded.App"),
            RuleFields {
                opacity: Some(1.5),
                ..RuleFields::default()
            },
        )],
    });
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Rejected {
                reason: Rejection::Rules(RuleError::InvalidField {
                    field: "fields.opacity",
                    ..
                }),
            }
        )),
        "an opacity of 1.5 is refused and names the field: {effects:?}"
    );
    check(&wm);
}

// A negative corner radius is refused by name, and never reaches the scene.
#[test]
fn a_negative_corner_radius_is_refused() {
    let mut wm = default_wm();
    let effects = wm.apply(Event::RulesChanged {
        rules: vec![rule(
            app("org.round.App"),
            RuleFields {
                corner_radius: Some(-4.0),
                ..RuleFields::default()
            },
        )],
    });
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Rejected {
                reason: Rejection::Rules(RuleError::InvalidField {
                    field: "fields.corner_radius",
                    ..
                }),
            }
        )),
        "a negative radius is refused and names the field: {effects:?}"
    );
    check(&wm);
}

// A window matching an appearance rule gets its opacity and radius, and the change is
// announced once so the scene can apply it.
#[test]
fn a_matching_appearance_rule_announces_opacity_and_radius() {
    let mut wm = default_wm();
    rules(
        &mut wm,
        vec![rule(
            app("org.faded.App"),
            RuleFields {
                opacity: Some(0.5),
                corner_radius: Some(12.0),
                ..RuleFields::default()
            },
        )],
    );
    let effects = wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.faded.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Appearance { window: id, opacity, corner_radius }
                if *id == window(1) && *opacity == 0.5 && *corner_radius == 12.0
        )),
        "the matched window gets opacity 0.5 and radius 12: {effects:?}"
    );
    check(&wm);
}

// Reloading the rules without the secure rule clears the flag, and says so once.
#[test]
fn reloading_without_the_secure_rule_clears_secure() {
    let mut wm = default_wm();
    rules(&mut wm, vec![secret_rule()]);
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.secret.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert_eq!(secure_of(&wm, 1), Some(true));
    let effects = wm.apply(Event::RulesChanged { rules: Vec::new() });
    assert_eq!(secure_effect(&effects, 1), Some(false), "the reload clears secure");
    assert_eq!(secure_of(&wm, 1), Some(false));
    check(&wm);
}

// A title change that stops a rule matching clears secure; one that starts a match sets it.
#[test]
fn a_title_change_toggles_secure_with_the_match() {
    let mut wm = default_wm();
    let titled = rule(
        Match {
            title: Some(Pattern::new("secret").expect("test pattern is valid")),
            ..Match::default()
        },
        RuleFields {
            secure: Some(true),
            ..RuleFields::default()
        },
    );
    rules(&mut wm, vec![titled]);
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.example.App")
            .with_title("public")
            .with_requested(Rect::new(0, 0, 600, 400)),
    ));
    assert_eq!(secure_of(&wm, 1), Some(false), "the title does not match yet");

    let effects = wm.apply(Event::WindowTitleChanged {
        window: window(1),
        title: "secret notes".to_owned(),
    });
    assert_eq!(secure_effect(&effects, 1), Some(true), "the new title matches");

    let effects = wm.apply(Event::WindowTitleChanged {
        window: window(1),
        title: "public again".to_owned(),
    });
    assert_eq!(secure_effect(&effects, 1), Some(false), "the title no longer matches");
    check(&wm);
}

// A closed window's record leaves the registry, so nothing stays secure after it.
#[test]
fn a_closed_secure_window_leaves_no_secure_record() {
    let mut wm = default_wm();
    rules(&mut wm, vec![secret_rule()]);
    wm.apply(Event::WindowOpened(
        WindowSpec::normal(window(1), "org.secret.App").with_requested(Rect::new(0, 0, 600, 400)),
    ));
    wm.apply(Event::WindowClosed { window: window(1) });
    assert_eq!(secure_of(&wm, 1), None, "the closed window has no surface record");
    check(&wm);
}

// A workspace switch tells the renderer which way the new workspace lies in the order, so the
// windows can slide the right way. Without it a switch cannot be told from a jump.
#[test]
fn a_workspace_switch_reports_which_way_the_new_workspace_lies() {
    let mut wm = default_wm();
    let effects = wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(2),
    });
    assert!(
        effects.contains(&Effect::WorkspaceShown {
            workspace: WorkspaceId::new(2),
            direction: Some(WorkspaceDirection::Later),
        }),
        "moving to a later workspace slides towards the later side: {effects:?}"
    );
    let effects = wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(1),
    });
    assert!(
        effects.contains(&Effect::WorkspaceShown {
            workspace: WorkspaceId::new(1),
            direction: Some(WorkspaceDirection::Earlier),
        }),
        "moving back to an earlier workspace slides the other way: {effects:?}"
    );
    check(&wm);
}

// Moving a window to another workspace takes its transient children with it: a transient is
// always on its parent's workspace, so moving the parent must not leave a dialog stranded.
#[test]
fn moving_a_parent_to_another_workspace_takes_its_transient_with_it() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::WindowOpened(
        WindowSpec::transient(window(2), "org.example.App", window(1)).with_requested(Rect::new(0, 0, 300, 200)),
    ));
    let effects = wm.apply(Event::MoveToWorkspace {
        window: window(1),
        workspace: WorkspaceId::new(2),
    });
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(2)));
    assert_eq!(wm.window(window(2)).map(|w| w.workspace), Some(WorkspaceId::new(2)));
    assert_eq!(visibility(&effects, window(1)), Some(false));
    assert_eq!(visibility(&effects, window(2)), Some(false));
    assert_eq!(
        wm.focused(),
        None,
        "nothing visible on the active workspace takes focus"
    );
    check(&wm);
}

// A transient cannot leave its parent's workspace on its own, because that would break the
// rule that a transient shares its parent's workspace. The move is refused and nothing changes.
#[test]
fn a_transient_cannot_leave_its_parents_workspace_on_its_own() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    wm.apply(Event::WindowOpened(
        WindowSpec::transient(window(2), "org.example.App", window(1)).with_requested(Rect::new(0, 0, 300, 200)),
    ));
    let effects = wm.apply(Event::MoveToWorkspace {
        window: window(2),
        workspace: WorkspaceId::new(2),
    });
    assert!(
        effects.contains(&Effect::Rejected {
            reason: Rejection::TransientFollowsParent(window(2)),
        }),
        "the caller is told the transient must follow its parent: {effects:?}"
    );
    assert_eq!(wm.window(window(2)).map(|w| w.workspace), Some(WorkspaceId::new(1)));
    check(&wm);
}

#[test]
fn moving_an_unknown_window_or_to_an_unknown_workspace_is_rejected() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let effects = wm.apply(Event::MoveToWorkspace {
        window: window(9),
        workspace: WorkspaceId::new(2),
    });
    assert!(effects.contains(&Effect::Rejected {
        reason: Rejection::UnknownWindow(window(9)),
    }));
    let effects = wm.apply(Event::MoveToWorkspace {
        window: window(1),
        workspace: WorkspaceId::new(99),
    });
    assert!(effects.contains(&Effect::Rejected {
        reason: Rejection::UnknownWorkspace(WorkspaceId::new(99)),
    }));
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(1)));
    check(&wm);
}

// Moving a window to the workspace it is already on is not an error, and changes nothing.
#[test]
fn moving_to_the_current_workspace_changes_nothing() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let effects = wm.apply(Event::MoveToWorkspace {
        window: window(1),
        workspace: WorkspaceId::new(1),
    });
    assert!(
        !effects.iter().any(|effect| matches!(effect, Effect::Rejected { .. })),
        "a no-op move is not reported as a rejection: {effects:?}"
    );
    assert_eq!(
        visibility(&effects, window(1)),
        None,
        "the window is not hidden or shown again"
    );
    assert_eq!(wm.window(window(1)).map(|w| w.workspace), Some(WorkspaceId::new(1)));
    check(&wm);
}

// A focused window that leaves the active workspace must not keep focus, or input would go to a
// window the user cannot see.
#[test]
fn moving_the_focused_window_away_hands_focus_to_a_visible_window() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    open(&mut wm, 2);
    focus(&mut wm, 1, ActivationDecision::Allow);
    assert_eq!(wm.focused(), Some(window(1)));
    wm.apply(Event::MoveToWorkspace {
        window: window(1),
        workspace: WorkspaceId::new(2),
    });
    assert_eq!(wm.focused(), Some(window(2)));
    check(&wm);
}

#[test]
fn moving_a_window_to_an_unknown_output_is_rejected() {
    let mut wm = default_wm();
    open(&mut wm, 1);
    let effects = wm.apply(Event::MoveToOutput {
        window: window(1),
        output: OutputId::new(7),
    });
    assert!(effects.contains(&Effect::Rejected {
        reason: Rejection::UnknownOutput(OutputId::new(7)),
    }));
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(Some(OutputId::new(0))));
    check(&wm);
}

// The window keeps its size and is pulled inside the new output's work area, the same way a window
// is reshaped when its output's layout changes.
#[test]
fn moving_a_window_to_another_output_reshapes_it_into_that_work_area() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1280, 720),
    });
    open(&mut wm, 1);
    let effects = wm.apply(Event::MoveToOutput {
        window: window(1),
        output: OutputId::new(1),
    });
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(Some(OutputId::new(1))));
    assert_eq!(configured(&effects, window(1)), Some(Rect::new(1920, 0, 600, 400)));
    check(&wm);
}

#[test]
fn a_maximised_window_fills_the_work_area_of_its_new_output() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1280, 720),
    });
    open(&mut wm, 1);
    wm.apply(Event::ToggleMaximise { window: window(1) });
    wm.apply(Event::MoveToOutput {
        window: window(1),
        output: OutputId::new(1),
    });
    assert_eq!(
        wm.window(window(1)).map(|w| w.geometry),
        Some(Rect::new(1920, 0, 1280, 720))
    );
    check(&wm);
}

#[test]
fn a_fullscreen_window_is_not_moved_between_outputs() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1280, 720),
    });
    open(&mut wm, 1);
    wm.apply(Event::ToggleFullscreen { window: window(1) });
    let effects = wm.apply(Event::MoveToOutput {
        window: window(1),
        output: OutputId::new(1),
    });
    assert!(effects.contains(&Effect::Rejected {
        reason: Rejection::NotFloating(window(1)),
    }));
    assert_eq!(wm.window(window(1)).map(|w| w.output), Some(Some(OutputId::new(0))));
    check(&wm);
}

// A transient follows its parent to another output, so the pair stays together.
#[test]
fn a_transient_follows_its_parent_to_another_output() {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1280, 720),
    });
    open(&mut wm, 1);
    wm.apply(Event::WindowOpened(
        WindowSpec::transient(window(2), "org.example.App", window(1)).with_requested(Rect::new(0, 0, 300, 200)),
    ));
    wm.apply(Event::MoveToOutput {
        window: window(1),
        output: OutputId::new(1),
    });
    assert_eq!(wm.window(window(2)).map(|w| w.output), Some(Some(OutputId::new(1))));
    check(&wm);
}

// --- Workspace sets ----------------------------------------------------------

fn set(name: &str, entries: &[(u64, Option<&str>)]) -> WorkspaceSet {
    WorkspaceSet {
        name: name.to_owned(),
        entries: entries
            .iter()
            .map(|(raw, output)| WorkspaceSetEntry {
                workspace: WorkspaceId::new(*raw),
                output: output.map(str::to_owned),
            })
            .collect(),
    }
}

fn define_sets(wm: &mut Wm, sets: Vec<WorkspaceSet>) -> Vec<Effect> {
    wm.apply(Event::WorkspaceSetsChanged { sets })
}

fn activate_set(wm: &mut Wm, name: &str) -> Vec<Effect> {
    wm.apply(Event::WorkspaceSetActivated { name: name.to_owned() })
}

/// A wm with two outputs: `DP-1` (primary, workspace 1 shown) and `HDMI-1` (workspace 1 shown).
fn two_output_wm() -> Wm {
    let mut wm = default_wm();
    wm.apply(Event::OutputConnected {
        output: OutputId::new(1),
        rect: Rect::new(1920, 0, 1920, 1080),
    });
    wm.apply(Event::OutputIdentified {
        output: OutputId::new(0),
        name: "DP-1".to_owned(),
        scale: Scale::IDENTITY,
    });
    wm.apply(Event::OutputIdentified {
        output: OutputId::new(1),
        name: "HDMI-1".to_owned(),
        scale: Scale::IDENTITY,
    });
    wm
}

fn rejection(effects: &[Effect]) -> Option<Rejection> {
    effects.iter().find_map(|effect| match effect {
        Effect::Rejected { reason } => Some(reason.clone()),
        _ => None,
    })
}

#[test]
fn activating_an_unknown_set_is_refused_and_changes_nothing() {
    let mut wm = default_wm();
    define_sets(&mut wm, vec![set("work", &[(2, None)])]);
    let before = wm.active_workspace();
    let effects = activate_set(&mut wm, "nope");
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::UnknownSet(
            "nope".to_owned()
        )))
    );
    assert_eq!(wm.active_workspace(), before);
    check(&wm);
}

#[test]
fn a_set_with_no_entries_is_refused_and_the_previous_sets_stay() {
    let mut wm = default_wm();
    define_sets(&mut wm, vec![set("work", &[(2, None)])]);
    let effects = define_sets(&mut wm, vec![set("empty", &[])]);
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::EmptySet("empty".to_owned())))
    );
    assert_eq!(wm.workspace_sets().len(), 1);
    assert_eq!(wm.workspace_sets()[0].name, "work");
    check(&wm);
}

#[test]
fn a_set_naming_an_undeclared_workspace_is_refused_as_a_whole() {
    let mut wm = default_wm();
    define_sets(&mut wm, vec![set("work", &[(2, None)])]);
    let effects = define_sets(&mut wm, vec![set("good", &[(1, None)]), set("bad", &[(99, None)])]);
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::UndeclaredWorkspace {
            set: "bad".to_owned(),
            workspace: WorkspaceId::new(99),
        }))
    );
    assert_eq!(
        wm.workspace_sets()[0].name,
        "work",
        "a refused list is never partly applied"
    );
    check(&wm);
}

#[test]
fn a_set_whose_workspace_is_removed_refuses_to_activate() {
    let mut wm = default_wm();
    define_sets(&mut wm, vec![set("work", &[(3, None)])]);
    wm.apply(Event::WorkspaceRemoved {
        workspace: WorkspaceId::new(3),
    });
    let effects = activate_set(&mut wm, "work");
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::UndeclaredWorkspace {
            set: "work".to_owned(),
            workspace: WorkspaceId::new(3),
        }))
    );
    check(&wm);
}

#[test]
fn duplicate_set_names_are_refused() {
    let mut wm = default_wm();
    let effects = define_sets(&mut wm, vec![set("work", &[(1, None)]), set("work", &[(2, None)])]);
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::DuplicateName(
            "work".to_owned()
        )))
    );
    assert!(wm.workspace_sets().is_empty());
}

#[test]
fn an_empty_set_name_is_refused() {
    let mut wm = default_wm();
    let effects = define_sets(&mut wm, vec![set("", &[(1, None)])]);
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::EmptyName))
    );
}

#[test]
fn two_entries_for_one_output_in_a_set_are_refused() {
    let mut wm = two_output_wm();
    let effects = define_sets(&mut wm, vec![set("clash", &[(1, Some("HDMI-1")), (2, Some("HDMI-1"))])]);
    assert_eq!(
        rejection(&effects),
        Some(Rejection::WorkspaceSet(WorkspaceSetError::DuplicateOutput {
            set: "clash".to_owned(),
            output: "HDMI-1".to_owned(),
        }))
    );
}

#[test]
fn activating_a_set_twice_is_idempotent() {
    let mut wm = two_output_wm();
    define_sets(&mut wm, vec![set("work", &[(2, Some("HDMI-1"))])]);
    let first = activate_set(&mut wm, "work");
    assert!(!first.is_empty(), "the first activation changes the live set");
    let second = activate_set(&mut wm, "work");
    assert!(second.is_empty(), "the second activation changes nothing");
    assert_eq!(wm.shown_workspace(OutputId::new(1)), Some(WorkspaceId::new(2)));
    check(&wm);
}

#[test]
fn an_unknown_output_falls_back_to_the_primary_with_a_warning() {
    let mut wm = two_output_wm();
    define_sets(&mut wm, vec![set("work", &[(3, Some("GONE-1"))])]);
    let effects = activate_set(&mut wm, "work");
    assert!(effects.contains(&Effect::WorkspaceSetOutputFallback {
        set: "work".to_owned(),
        output: "GONE-1".to_owned(),
    }));
    assert_eq!(
        wm.active_workspace(),
        Some(WorkspaceId::new(3)),
        "placed on the primary output"
    );
    check(&wm);
}

#[test]
fn a_set_shows_each_named_workspace_on_its_own_output() {
    let mut wm = two_output_wm();
    open(&mut wm, 1);
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(2),
    });
    open(&mut wm, 2);
    wm.apply(Event::MoveToOutput {
        window: window(2),
        output: OutputId::new(1),
    });
    wm.apply(Event::WorkspaceActivated {
        workspace: WorkspaceId::new(1),
    });
    assert!(!wm.is_visible(window(2)), "workspace 2 is not shown anywhere yet");
    define_sets(&mut wm, vec![set("split", &[(1, Some("DP-1")), (2, Some("HDMI-1"))])]);
    activate_set(&mut wm, "split");
    assert_eq!(wm.active_workspace(), Some(WorkspaceId::new(1)));
    assert_eq!(wm.shown_workspace(OutputId::new(1)), Some(WorkspaceId::new(2)));
    assert!(wm.is_visible(window(1)));
    assert!(wm.is_visible(window(2)));
    check(&wm);
}
