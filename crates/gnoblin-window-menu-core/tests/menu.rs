//! Tests for the window menu model, choice checks, navigation, placement and
//! the provider fallback rules, as set out in D29.
//!
//! These use only the public API, so they hold the same contract a provider
//! or the compositor relies on.

use gnoblin_types::{Point, Rect, Size};
use gnoblin_window_menu_core::{
    Contribution, DismissReason, ItemId, MenuAction, MenuChoiceError, MenuKey, MenuRouted, MenuWindowState,
    MenuWorkspaces, NavOutcome, NavRefusal, NavState, Navigator, OpenError, ProviderSource, build_menu, place,
    reply_overdue, source_for, validate_choice,
};
use std::time::Duration;

/// A window that can do everything, on a multi-workspace setup.
fn full_state() -> MenuWindowState {
    MenuWindowState {
        maximised: false,
        minimised: false,
        resizable: true,
        movable: true,
        closable: true,
        always_on_top: false,
        on_all_workspaces: false,
    }
}

/// The ids of the top-level items, in order.
fn top_ids(menu: &gnoblin_window_menu_core::Menu) -> Vec<String> {
    menu.items
        .iter()
        .filter_map(|item| item.id.as_ref().map(|id| id.as_str().to_owned()))
        .collect()
}

#[test]
fn restore_shows_only_when_maximised() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let plain = build_menu(&full_state(), &workspaces, &[]);
    assert!(!top_ids(&plain).contains(&"window.restore".to_owned()));

    let maximised = MenuWindowState {
        maximised: true,
        ..full_state()
    };
    let menu = build_menu(&maximised, &workspaces, &[]);
    assert!(top_ids(&menu).contains(&"window.restore".to_owned()));
    // Maximize is turned off while the window is already maximised.
    let maximise = validate_choice(&menu, &MenuAction::Maximize.id());
    assert_eq!(maximise, Err(MenuChoiceError::Disabled("window.maximize".to_owned())));
}

#[test]
fn move_to_workspace_lists_every_workspace_and_disables_the_current_one() {
    let workspaces = MenuWorkspaces::new(3, 2).expect("three workspaces");
    let menu = build_menu(&full_state(), &workspaces, &[]);
    let move_to = ItemId::parse("window.move-to-workspace.2").expect("valid id");
    assert_eq!(
        validate_choice(&menu, &move_to),
        Err(MenuChoiceError::Disabled("window.move-to-workspace.2".to_owned()))
    );
    let other = ItemId::parse("window.move-to-workspace.3").expect("valid id");
    assert_eq!(
        validate_choice(&menu, &other),
        Ok(MenuRouted::Action(MenuAction::MoveToWorkspace(3)))
    );
}

#[test]
fn unknown_id_is_refused_not_in_menu() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let menu = build_menu(&full_state(), &workspaces, &[]);
    // Restore is absent because the window is not maximised, so the provider
    // was never offered it.
    let restore = MenuAction::Restore.id();
    assert_eq!(
        validate_choice(&menu, &restore),
        Err(MenuChoiceError::NotInMenu("window.restore".to_owned()))
    );
}

#[test]
fn enabled_action_routes_to_the_compositor() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let menu = build_menu(&full_state(), &workspaces, &[]);
    assert_eq!(
        validate_choice(&menu, &MenuAction::Close.id()),
        Ok(MenuRouted::Action(MenuAction::Close))
    );
}

#[test]
fn extension_item_routes_to_its_extension_never_to_an_action() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let item = gnoblin_window_menu_core::MenuItem::action(
        ItemId::extension("notes", "pin").expect("valid extension id"),
        "Pin note",
    );
    let contribution = Contribution::new("notes", vec![item]).expect("namespaced contribution");
    let menu = build_menu(&full_state(), &workspaces, &[contribution]);
    let id = ItemId::extension("notes", "pin").expect("valid extension id");
    assert_eq!(
        validate_choice(&menu, &id),
        Ok(MenuRouted::Extension {
            extension: "notes".to_owned(),
            local: "pin".to_owned(),
        })
    );
}

#[test]
fn submenu_group_id_is_not_a_choice() {
    let workspaces = MenuWorkspaces::new(2, 1).expect("two workspaces");
    let menu = build_menu(&full_state(), &workspaces, &[]);
    let group = ItemId::move_to_workspace_group();
    assert_eq!(
        validate_choice(&menu, &group),
        Err(MenuChoiceError::IsSubmenu(group.as_str().to_owned()))
    );
}

#[test]
fn a_disabled_item_is_refused_and_named() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let state = MenuWindowState {
        closable: false,
        ..full_state()
    };
    let menu = build_menu(&state, &workspaces, &[]);
    assert_eq!(
        validate_choice(&menu, &MenuAction::Close.id()),
        Err(MenuChoiceError::Disabled("window.close".to_owned()))
    );
}

#[test]
fn keyboard_open_selects_first_enabled_item_and_wraps() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let state = MenuWindowState {
        resizable: false,
        ..full_state()
    };
    let menu = build_menu(&state, &workspaces, &[]);
    let mut nav = Navigator::new(menu);
    nav.open(true).expect("opens");
    let NavState::Open { highlight, .. } = nav.state().clone() else {
        panic!("menu should be open");
    };
    assert!(highlight.is_some(), "first enabled item is highlighted");

    // Down must visit only enabled items and wrap back to the start. Separators
    // and disabled items are skipped, so the cycle is not `items.len()` long.
    let mut wrapped = false;
    for _ in 0..nav.menu().items.len() * 2 {
        nav.handle(MenuKey::Down).expect("moves");
        let NavState::Open {
            highlight: Some(index), ..
        } = nav.state().clone()
        else {
            panic!("menu should still be open");
        };
        assert!(nav.menu().items[index].enabled, "highlight landed on a disabled item");
        if Some(index) == highlight {
            wrapped = true;
            break;
        }
    }
    assert!(wrapped, "Down never wrapped back to the first item");
}

#[test]
fn escape_at_root_dismisses_and_keys_are_refused_when_closed() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let mut nav = Navigator::new(build_menu(&full_state(), &workspaces, &[]));
    assert_eq!(nav.handle(MenuKey::Down), Err(NavRefusal::Closed));
    nav.open(true).expect("opens");
    assert_eq!(
        nav.handle(MenuKey::Escape),
        Ok(NavOutcome::Dismissed(DismissReason::Escape))
    );
    assert_eq!(nav.state(), &NavState::Closed);
}

#[test]
fn left_at_root_is_refused() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let mut nav = Navigator::new(build_menu(&full_state(), &workspaces, &[]));
    nav.open(true).expect("opens");
    assert_eq!(nav.handle(MenuKey::Left), Err(NavRefusal::AtRoot));
}

#[test]
fn enter_chooses_the_highlighted_action_and_closes() {
    let workspaces = MenuWorkspaces::new(1, 1).expect("one workspace");
    let mut nav = Navigator::new(build_menu(&full_state(), &workspaces, &[]));
    nav.open(true).expect("opens");
    let NavState::Open {
        highlight: Some(index), ..
    } = nav.state().clone()
    else {
        panic!("a first item should be highlighted");
    };
    // Walk the highlight to the first action so the outcome is deterministic.
    let first = nav.menu().items[index].id.clone().expect("action has an id");
    let outcome = nav.handle(MenuKey::Enter).expect("chooses");
    assert_eq!(outcome, NavOutcome::Chose(first));
    assert_eq!(nav.state(), &NavState::Closed);
}

#[test]
fn placement_opens_below_the_anchor_when_it_fits() {
    let anchor = Rect::new(100, 50, 40, 20);
    let work = Rect::new(0, 0, 1000, 800);
    let placed = place(anchor, Size::new(200, 300), work);
    assert_eq!(placed.origin, Point::new(100, 70));
    assert!(!placed.flipped);
}

#[test]
fn placement_flips_above_when_it_does_not_fit_below() {
    let anchor = Rect::new(100, 700, 40, 20);
    let work = Rect::new(0, 0, 1000, 800);
    let placed = place(anchor, Size::new(200, 300), work);
    assert!(placed.flipped);
    assert_eq!(placed.origin.y, 700 - 300);
}

#[test]
fn placement_clamps_horizontally_into_the_work_area() {
    let anchor = Rect::new(950, 50, 40, 20);
    let work = Rect::new(0, 0, 1000, 800);
    let placed = place(anchor, Size::new(200, 100), work);
    assert_eq!(placed.origin.x, 1000 - 200);
}

#[test]
fn placement_pins_an_oversized_menu_to_the_top_left_corner() {
    let anchor = Rect::new(500, 500, 40, 20);
    let work = Rect::new(0, 0, 300, 300);
    let placed = place(anchor, Size::new(400, 400), work);
    assert_eq!(placed.origin, Point::new(0, 0));
}

#[test]
fn source_is_configured_only_when_the_provider_opened_the_menu() {
    assert_eq!(source_for(Ok(())), ProviderSource::Configured);
    assert_eq!(
        source_for(Err(OpenError::Missing)),
        ProviderSource::Builtin(OpenError::Missing)
    );
    assert_eq!(
        source_for(Err(OpenError::StartFailed)),
        ProviderSource::Builtin(OpenError::StartFailed)
    );
    assert_eq!(
        source_for(Err(OpenError::NoReply)),
        ProviderSource::Builtin(OpenError::NoReply)
    );
}

#[test]
fn reply_is_overdue_at_the_two_second_timeout() {
    assert!(!reply_overdue(Duration::from_millis(1999)));
    assert!(reply_overdue(Duration::from_secs(2)));
}
