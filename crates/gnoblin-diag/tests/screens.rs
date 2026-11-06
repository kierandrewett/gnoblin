//! Headless checks for the portal dialog screens: structure, input outcomes,
//! arm delay, cancellation, reduced motion, and rasterised pixels.

use gnoblin_diag::portal::{Label, PortalScreen, Setup, Theme};
use gnoblin_portal_model::{
    ARM_DELAY_MS, AppChoice, AppChooser, Consent, Dialog, DirectoryListing, FileChooser, FileChooserMode, FileEntry,
    FileFilter, Grant, ScreenKey, Settled,
};
use gnoblin_portal_paint::Frame;
use gnoblin_types::Tick;

const WIDTH: usize = 480;
const HEIGHT: usize = 240;

fn setup(reduced_motion: bool) -> Setup {
    Setup {
        width: WIDTH,
        height: HEIGHT,
        theme: Theme::default(),
        reduced_motion,
    }
}

fn consent_screen(prompt: &str, now: u64, reduced_motion: bool) -> PortalScreen {
    let consent = Consent::open("org.example.App", prompt, Tick::new(now));
    PortalScreen::new(Dialog::Consent(consent), setup(reduced_motion), Tick::new(now))
}

fn label<'a>(screen: &'a PortalScreen, text: &str) -> &'a Label {
    screen
        .labels()
        .iter()
        .find(|label| label.text == text)
        .unwrap_or_else(|| panic!("no label {text:?} in {:?}", screen.labels()))
}

fn centre(label: &Label) -> (f32, f32) {
    (label.rect.center().x, label.rect.center().y)
}

/// Most of a region is the fill colour; a drawn label adds pixels that are not.
fn non_fill_pixels(frame: &Frame, label: &Label, fill: [u8; 4]) -> usize {
    frame
        .region(label.rect)
        .into_iter()
        .filter(|pixel| *pixel != fill)
        .count()
}

fn theme_rgba(colour: egui::Color32) -> [u8; 4] {
    colour.to_array()
}

#[test]
fn consent_structure_is_golden() {
    let screen = consent_screen("Share your screen?", 0, false);
    let texts: Vec<&str> = screen.labels().iter().map(|label| label.text.as_str()).collect();
    assert_eq!(
        texts,
        ["Share your screen?", "Requested by org.example.App", "Deny", "Allow"]
    );
    assert_eq!(
        screen.structure(),
        "dialog consent\n\
         app org.example.App\n\
         prompt Share your screen?\n\
         focus Deny\n\
         armed false\n\
         buttons Deny Allow\n\
         decision None\n\
         settled None\n"
    );
}

#[test]
fn enter_on_default_focus_denies() {
    let mut screen = consent_screen("Share?", 0, false);
    assert_eq!(screen.key(ScreenKey::Enter, false), Some(Settled::Denied));
    assert_eq!(screen.settled(), Some(&Settled::Denied));
}

#[test]
fn escape_denies_even_when_allow_is_armed() {
    let mut screen = consent_screen("Share?", 0, false);
    screen.tick(Tick::new(ARM_DELAY_MS));
    assert_eq!(screen.key(ScreenKey::Escape, false), Some(Settled::Denied));
}

#[test]
fn tab_then_enter_allows_only_after_arm_delay() {
    let mut screen = consent_screen("Share?", 0, false);
    assert_eq!(screen.key(ScreenKey::Tab, false), None);
    assert_eq!(screen.key(ScreenKey::Enter, false), None, "Allow is disarmed at tick 0");
    assert!(screen.settled().is_none());

    screen.tick(Tick::new(ARM_DELAY_MS - 1));
    assert_eq!(screen.key(ScreenKey::Enter, false), None, "one tick before arming");

    screen.tick(Tick::new(ARM_DELAY_MS));
    assert_eq!(
        screen.key(ScreenKey::Enter, false),
        Some(Settled::Granted(Grant::Allowed))
    );
}

#[test]
fn key_repeat_on_allow_never_grants() {
    let mut screen = consent_screen("Share?", 0, false);
    screen.tick(Tick::new(ARM_DELAY_MS));
    screen.key(ScreenKey::Tab, false);
    assert_eq!(screen.key(ScreenKey::Enter, true), None);
    assert!(screen.settled().is_none());
}

#[test]
fn pointer_press_and_release_on_allow_grants_after_arm() {
    let mut screen = consent_screen("Share?", 0, false);
    screen.tick(Tick::new(ARM_DELAY_MS));
    let (x, y) = centre(label(&screen, "Allow"));
    assert_eq!(screen.pointer_press(x, y), None);
    assert_eq!(screen.pointer_release(x, y), Some(Settled::Granted(Grant::Allowed)));
}

#[test]
fn pointer_press_on_allow_before_arm_does_nothing() {
    let mut screen = consent_screen("Share?", 0, false);
    let (x, y) = centre(label(&screen, "Allow"));
    assert_eq!(screen.pointer_press(x, y), None);
    assert_eq!(screen.pointer_release(x, y), None);
    assert!(screen.settled().is_none());
}

#[test]
fn press_on_deny_release_elsewhere_does_not_deny() {
    let mut screen = consent_screen("Share?", 0, false);
    let (dx, dy) = centre(label(&screen, "Deny"));
    assert_eq!(screen.pointer_press(dx, dy), None);
    assert_eq!(screen.pointer_release(0.0, 0.0), None);
    assert!(screen.settled().is_none());
}

#[test]
fn press_and_release_on_deny_denies() {
    let mut screen = consent_screen("Share?", 0, false);
    let (dx, dy) = centre(label(&screen, "Deny"));
    screen.pointer_press(dx, dy);
    assert_eq!(screen.pointer_release(dx, dy), Some(Settled::Denied));
}

#[test]
fn requester_gone_cancels_and_ignores_later_input() {
    let mut screen = consent_screen("Share?", 0, false);
    assert_eq!(screen.cancel(), Some(Settled::Cancelled));
    assert_eq!(screen.key(ScreenKey::Escape, false), None);
    assert_eq!(screen.settled(), Some(&Settled::Cancelled));
    assert_eq!(screen.cancel(), None, "cancel is idempotent once settled");
}

#[test]
fn decision_is_final_after_settling() {
    let mut screen = consent_screen("Share?", 0, false);
    assert_eq!(screen.key(ScreenKey::Escape, false), Some(Settled::Denied));
    screen.tick(Tick::new(ARM_DELAY_MS));
    screen.key(ScreenKey::Tab, false);
    assert_eq!(screen.key(ScreenKey::Enter, false), None);
    assert_eq!(screen.settled(), Some(&Settled::Denied));
}

#[test]
fn rendered_frame_is_non_empty_and_background_filled() {
    let mut screen = consent_screen("Share?", 0, false);
    let frame = screen.render();
    assert_eq!((frame.width(), frame.height()), (WIDTH, HEIGHT));
    let background = theme_rgba(Theme::default().background);
    let painted = frame
        .bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| **pixel != background)
        .count();
    assert!(painted > 0, "the card and buttons must draw something");
    assert_eq!(frame.pixel(0, 0), background, "the margin keeps the background");
}

#[test]
fn prompt_and_button_labels_have_pixels_inside_their_rects() {
    let theme = Theme::default();
    let mut screen = consent_screen("Share your screen?", 0, false);
    screen.tick(Tick::new(ARM_DELAY_MS));
    let frame = screen.render();
    let card = theme_rgba(theme.card);
    let button = theme_rgba(theme.button);
    let armed = theme_rgba(theme.accent);

    let prompt = label(&screen, "Share your screen?");
    assert!(non_fill_pixels(&frame, prompt, card) > 0, "prompt text is drawn");

    let deny = label(&screen, "Deny");
    assert!(non_fill_pixels(&frame, deny, button) > 0, "Deny text is drawn");

    let allow = label(&screen, "Allow");
    assert!(non_fill_pixels(&frame, allow, armed) > 0, "Allow text is drawn");
}

#[test]
fn different_label_strings_draw_different_pixels_in_the_same_rect() {
    let mut first = consent_screen("Share your screen?", 0, false);
    let mut second = consent_screen("Share your camera?", 0, false);
    let frame_first = first.render();
    let frame_second = second.render();

    let rect_first = label(&first, "Share your screen?").rect;
    let rect_second = label(&second, "Share your camera?").rect;
    // Both prompts start at the same pixel, so the top-left of each rect is shared.
    assert_eq!(rect_first.min, rect_second.min);

    let region_first = frame_first.region(rect_first);
    let region_second = frame_second.region(rect_first);
    assert_ne!(
        region_first, region_second,
        "different text must paint different pixels"
    );
}

#[test]
fn tab_moves_focus_and_changes_the_frame() {
    let mut screen = consent_screen("Share?", 0, false);
    let before = screen.render();
    screen.key(ScreenKey::Tab, false);
    let after = screen.render();
    assert_ne!(before, after, "the focus ring moves to Allow");
    assert!(screen.structure().contains("focus Allow"));
}

#[test]
fn rendering_is_deterministic() {
    let mut first = consent_screen("Share your screen?", 0, false);
    let mut second = consent_screen("Share your screen?", 0, false);
    assert_eq!(first.render(), second.render());
    first.tick(Tick::new(ARM_DELAY_MS));
    second.tick(Tick::new(ARM_DELAY_MS));
    assert_eq!(first.render(), second.render());
}

#[test]
fn reduced_motion_draws_armed_allow_at_once() {
    let theme = Theme::default();
    let mut screen = consent_screen("Share?", 0, true);
    screen.tick(Tick::new(ARM_DELAY_MS));
    let frame = screen.render();
    let allow = label(&screen, "Allow");
    // The label's rect is inside the button; sample a pixel near the button's corner.
    let sample = (allow.rect.min.x as usize - 4, allow.rect.min.y as usize - 4);
    assert_eq!(frame.pixel(sample.0, sample.1), theme_rgba(theme.accent));
}

fn file(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        is_dir,
        mime: None,
    }
}

/// A FileChooser in `/home/u` with `draft.txt` as its save name and one filter,
/// whose listing is already delivered.
fn files_screen(mode: FileChooserMode, multiple: bool, entries: Vec<FileEntry>) -> PortalScreen {
    let filters = vec![FileFilter {
        name: "Images".to_owned(),
        patterns: vec!["*.png".to_owned()],
    }];
    let chooser = FileChooser::new(mode, multiple, "/home/u", filters, "draft.txt");
    let mut screen = PortalScreen::new(Dialog::Files(chooser), setup(false), Tick::new(0));
    assert!(screen.list(DirectoryListing {
        folder: "/home/u".to_owned(),
        entries,
    }));
    screen
}

/// An AppChooser offering a viewer and an editor for `image/png`.
fn app_screen(last: Option<&str>) -> PortalScreen {
    let choices = vec![
        AppChoice {
            desktop_file_id: "org.a.Viewer".to_owned(),
            name: "Viewer".to_owned(),
        },
        AppChoice {
            desktop_file_id: "org.b.Editor".to_owned(),
            name: "Editor".to_owned(),
        },
    ];
    let chooser = AppChooser::new("image/png", choices, last.map(str::to_owned), "");
    PortalScreen::new(Dialog::App(chooser), setup(false), Tick::new(0))
}

fn press(screen: &mut PortalScreen, keys: &[ScreenKey]) {
    for key in keys {
        screen.key(*key, false);
    }
}

fn granted_paths(settled: Option<&Settled>) -> Vec<String> {
    match settled {
        Some(Settled::Granted(Grant::Files { paths, .. })) => paths.clone(),
        other => panic!("expected a files grant, got {other:?}"),
    }
}

#[test]
fn open_enter_on_file_grants_its_path() {
    let mut screen = files_screen(
        FileChooserMode::Open,
        false,
        vec![file("a.png", false), file("b.png", false)],
    );
    press(&mut screen, &[ScreenKey::Down, ScreenKey::Enter]);
    assert_eq!(granted_paths(screen.settled()), vec!["/home/u/b.png".to_owned()]);
}

#[test]
fn multi_open_space_selection_survives_enter() {
    let mut screen = files_screen(
        FileChooserMode::Open,
        true,
        vec![file("a.png", false), file("b.png", false), file("c.png", false)],
    );
    // Space selects a.png, Down Down reaches c.png, Space selects it, Enter confirms both.
    press(
        &mut screen,
        &[
            ScreenKey::Space,
            ScreenKey::Down,
            ScreenKey::Down,
            ScreenKey::Space,
            ScreenKey::Enter,
        ],
    );
    assert_eq!(
        granted_paths(screen.settled()),
        vec!["/home/u/a.png".to_owned(), "/home/u/c.png".to_owned()],
    );
}

#[test]
fn enter_on_directory_requests_its_listing_without_settling() {
    let mut screen = files_screen(
        FileChooserMode::Open,
        false,
        vec![file("docs", true), file("a.png", false)],
    );
    press(&mut screen, &[ScreenKey::Enter]);
    assert_eq!(screen.listing_request(), Some("/home/u/docs"));
    assert!(screen.settled().is_none());
    // A listing for another folder is refused; the requested one is accepted.
    assert!(!screen.list(DirectoryListing {
        folder: "/etc".to_owned(),
        entries: vec![],
    }));
    assert!(screen.list(DirectoryListing {
        folder: "/home/u/docs".to_owned(),
        entries: vec![file("x.png", false)],
    }));
    assert!(screen.listing_request().is_none());
}

#[test]
fn save_asks_before_overwriting_then_grants_on_second_confirm() {
    let mut screen = files_screen(FileChooserMode::Save, false, vec![file("draft.txt", false)]);
    // Tab moves to the name field, which holds the existing "draft.txt": the first Enter asks.
    press(&mut screen, &[ScreenKey::Tab, ScreenKey::Enter]);
    assert!(screen.settled().is_none(), "the overwrite must be asked, not granted");
    press(&mut screen, &[ScreenKey::Enter]);
    assert_eq!(granted_paths(screen.settled()), vec!["/home/u/draft.txt".to_owned()]);
}

#[test]
fn save_to_a_new_name_grants_without_asking() {
    let mut screen = files_screen(FileChooserMode::Save, false, vec![file("other.txt", false)]);
    press(&mut screen, &[ScreenKey::Tab]);
    press(&mut screen, &[ScreenKey::Backspace; 9]);
    for c in "new.txt".chars() {
        press(&mut screen, &[ScreenKey::Text(c)]);
    }
    press(&mut screen, &[ScreenKey::Enter]);
    assert_eq!(granted_paths(screen.settled()), vec!["/home/u/new.txt".to_owned()]);
}

#[test]
fn folder_mode_enter_with_no_entries_grants_the_current_folder() {
    let mut screen = files_screen(FileChooserMode::Folder, false, vec![]);
    press(&mut screen, &[ScreenKey::Enter]);
    assert_eq!(granted_paths(screen.settled()), vec!["/home/u".to_owned()]);
}

#[test]
fn escape_cancels_a_chooser() {
    let mut screen = files_screen(FileChooserMode::Open, false, vec![file("a.png", false)]);
    press(&mut screen, &[ScreenKey::Escape]);
    assert_eq!(screen.settled(), Some(&Settled::Cancelled));
}

#[test]
fn app_chooser_remember_is_forwarded_with_the_cursor_row() {
    // The filter field starts focused. Down moves the cursor to Editor; Tab then
    // Tab reaches Remember, Space ticks it, and Tab Tab returns to the list.
    let mut screen = app_screen(None);
    press(
        &mut screen,
        &[ScreenKey::Down, ScreenKey::Tab, ScreenKey::Tab, ScreenKey::Space],
    );
    press(&mut screen, &[ScreenKey::Tab, ScreenKey::Tab, ScreenKey::Enter]);
    match screen.settled() {
        Some(Settled::Granted(Grant::App {
            desktop_file_id,
            remember,
        })) => {
            assert_eq!(desktop_file_id, "org.b.Editor");
            assert!(*remember);
        }
        other => panic!("expected an app grant, got {other:?}"),
    }
}

#[test]
fn app_filter_with_no_match_grants_nothing() {
    let mut screen = app_screen(None);
    for c in "zzz".chars() {
        press(&mut screen, &[ScreenKey::Text(c)]);
    }
    press(&mut screen, &[ScreenKey::Tab, ScreenKey::Enter]);
    assert!(screen.settled().is_none(), "no visible row to choose");
}

#[test]
fn app_filter_narrows_the_list_the_choice_is_made_from() {
    let mut screen = app_screen(None);
    for c in "edit".chars() {
        press(&mut screen, &[ScreenKey::Text(c)]);
    }
    press(&mut screen, &[ScreenKey::Tab, ScreenKey::Enter]);
    match screen.settled() {
        Some(Settled::Granted(Grant::App { desktop_file_id, .. })) => {
            assert_eq!(desktop_file_id, "org.b.Editor");
        }
        other => panic!("expected the filtered Editor, got {other:?}"),
    }
}

#[test]
fn file_cursor_move_changes_the_frame_and_is_deterministic() {
    let entries = vec![file("a.png", false), file("b.png", false)];
    let mut first = files_screen(FileChooserMode::Open, false, entries.clone());
    let mut second = files_screen(FileChooserMode::Open, false, entries);
    let before = first.render();
    assert_eq!(before, second.render());
    press(&mut first, &[ScreenKey::Down]);
    assert_ne!(before, first.render(), "moving the cursor repaints");
    press(&mut second, &[ScreenKey::Down]);
    assert_eq!(first.render(), second.render());
}

#[test]
fn chooser_structure_names_the_mode_and_its_rows() {
    let screen = files_screen(
        FileChooserMode::Open,
        false,
        vec![file("docs", true), file("a.png", false)],
    );
    let structure = screen.structure();
    assert!(structure.contains("mode Open\n"), "{structure}");
    assert!(structure.contains("docs/"), "{structure}");
    assert!(structure.contains("a.png"), "{structure}");
}
