//! The portal screens driven by key events, through the same shell the compositor uses.

use gnoblin_portal::PortalScreen;
use gnoblin_portal_core::model::{
    ARM_DELAY_MS, AppChoice, AppChooser, Consent, Dialog, FileChooser, FileChooserMode, Grant, ScreenKey, Settled,
};
use gnoblin_portal_ui::ui::{DialogScheme, Setup};
use gnoblin_types::Tick;

const WIDTH: usize = 320;
const HEIGHT: usize = 160;

fn setup() -> Setup {
    Setup {
        width: WIDTH,
        height: HEIGHT,
        reduced_motion: true,
        scheme: DialogScheme::Dark,
    }
}

fn consent_screen() -> PortalScreen {
    let consent = Consent::open("org.example.App", "Share your screen?", Tick::new(0));
    PortalScreen::new(Dialog::Consent(consent), setup(), Tick::new(0))
}

#[test]
fn allow_is_ignored_until_the_arm_delay_has_passed() {
    let mut screen = consent_screen();
    screen.render();
    assert_eq!(
        screen.key(ScreenKey::Tab, false),
        None,
        "Allow cannot take focus until armed"
    );
    assert!(screen.settled().is_none());

    screen.tick(Tick::new(ARM_DELAY_MS));
    screen.render();
    assert_eq!(screen.key(ScreenKey::Tab, false), None);
    assert_eq!(
        screen.key(ScreenKey::Enter, false),
        Some(Settled::Granted(Grant::Allowed))
    );
    assert_eq!(screen.settled(), Some(&Settled::Granted(Grant::Allowed)));
}

#[test]
fn escape_denies_consent_once_armed() {
    let mut screen = consent_screen();
    screen.tick(Tick::new(ARM_DELAY_MS));
    screen.render();
    assert_eq!(screen.key(ScreenKey::Escape, false), Some(Settled::Denied));
}

#[test]
fn a_settled_screen_ignores_later_input() {
    let mut screen = consent_screen();
    screen.tick(Tick::new(ARM_DELAY_MS));
    screen.render();
    assert!(screen.key(ScreenKey::Escape, false).is_some());
    assert_eq!(screen.key(ScreenKey::Enter, false), None);
    assert_eq!(screen.cancel(), None);
    assert!(screen.settled().is_some());
}

#[test]
fn the_compositor_can_cancel_a_screen_once() {
    let mut screen = consent_screen();
    assert_eq!(screen.cancel(), Some(Settled::Cancelled));
    assert_eq!(screen.cancel(), None);
}

#[test]
fn escape_cancels_a_file_chooser() {
    let chooser = FileChooser::new(FileChooserMode::Open, false, "/home/user", Vec::new(), "");
    let mut screen = PortalScreen::new(Dialog::Files(chooser), setup(), Tick::new(0));
    screen.render();
    assert_eq!(screen.key(ScreenKey::Escape, false), Some(Settled::Cancelled));
}

#[test]
fn escape_cancels_an_app_chooser() {
    let choices = vec![AppChoice {
        desktop_file_id: "org.example.Edit.desktop".to_string(),
        name: "Edit".to_string(),
    }];
    let chooser = AppChooser::new("text/plain", choices, None, "");
    let mut screen = PortalScreen::new(Dialog::App(chooser), setup(), Tick::new(0));
    screen.render();
    assert_eq!(screen.key(ScreenKey::Escape, false), Some(Settled::Cancelled));
}

#[test]
fn a_rendered_dialog_is_not_one_flat_colour() {
    let mut screen = consent_screen();
    let frame = screen.render();
    // The frame is the measured content: the 360 px minimum width clamped to the
    // 320 px output, and a height shorter than the output.
    let (width, height) = screen.size();
    assert_eq!(width, WIDTH, "the minimum width is clamped to the bounds");
    assert!(
        height > 0 && height < HEIGHT,
        "the frame fits the content, not the output"
    );
    assert_eq!((frame.width(), frame.height()), (width, height));
    let bytes = frame.bytes();
    assert_eq!(bytes.len(), width * height * 4);
    assert!(
        bytes.iter().any(|&byte| byte != bytes[0]),
        "the dialog paints text and buttons"
    );
}
