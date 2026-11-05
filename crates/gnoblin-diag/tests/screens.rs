//! Headless checks for the portal dialog screens: structure, input outcomes,
//! arm delay, cancellation, reduced motion, and rasterised pixels.

use gnoblin_diag::paint::Frame;
use gnoblin_diag::portal::{ARM_DELAY_MS, Consent, Dialog, Grant, Label, Screen, ScreenKey, Settled, Setup, Theme};
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

fn consent_screen(prompt: &str, now: u64, reduced_motion: bool) -> Screen {
    let consent = Consent::open("org.example.App", prompt, Tick::new(now));
    Screen::new(Dialog::Consent(consent), setup(reduced_motion), Tick::new(now))
}

fn label<'a>(screen: &'a Screen, text: &str) -> &'a Label {
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
