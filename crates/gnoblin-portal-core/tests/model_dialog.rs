//! Failure-mode tests for the built-in dialog models (D19 "UI step").

use gnoblin_portal_core::model::{AppChoice, AppChooser, Consent, ConsentDecision, ConsentInput, Key};
use gnoblin_types::Tick;

fn tick(raw: u64) -> Tick {
    Tick::new(raw)
}

#[test]
fn consent_escape_denies_and_input_after_decision_is_ignored() {
    let mut consent = Consent::open("app.x", "Share the screen?", tick(0));
    let decision = consent.input(
        ConsentInput::Key {
            key: Key::Escape,
            repeat: false,
        },
        tick(0),
    );
    assert_eq!(decision, Some(ConsentDecision::Denied));
    assert_eq!(consent.decision(), Some(ConsentDecision::Denied));
    assert_eq!(
        consent.input(
            ConsentInput::Key {
                key: Key::Enter,
                repeat: false,
            },
            tick(10_000),
        ),
        None
    );
}

#[test]
fn consent_enter_before_arm_delay_does_not_allow() {
    let mut consent = Consent::open("app.x", "Share the screen?", tick(1_000));
    assert!(!consent.armed(tick(1_000)));
    // Tab moves focus from the default Deny onto Allow.
    assert_eq!(
        consent.input(
            ConsentInput::Key {
                key: Key::Tab,
                repeat: false,
            },
            tick(1_000),
        ),
        None
    );
    let decision = consent.input(
        ConsentInput::Key {
            key: Key::Enter,
            repeat: true,
        },
        tick(1_000),
    );
    assert_eq!(decision, None);
    assert_eq!(consent.decision(), None);
}

#[test]
fn file_chooser_rejects_invalid_save_name_and_asks_before_overwrite() {
    use gnoblin_portal_core::model::{Confirmation, DialogError, FileChooser, FileChooserMode, FileEntry};
    let mut chooser = FileChooser::new(FileChooserMode::Save, false, "/home/u", Vec::new(), "notes.txt");
    chooser.set_name("bad/name");
    assert_eq!(chooser.confirm(), Err(DialogError::InvalidName));

    chooser.set_name("exists.txt");
    chooser.set_entries(vec![FileEntry {
        name: "exists.txt".to_owned(),
        is_dir: false,
        mime: None,
    }]);
    assert!(matches!(chooser.confirm(), Ok(Confirmation::OverwriteAsk { .. })));
    assert!(matches!(chooser.confirm(), Ok(Confirmation::Chosen(_))));
}

#[test]
fn app_chooser_does_not_offer_unlisted_apps() {
    let chooser = AppChooser::new(
        "text/plain",
        vec![AppChoice {
            desktop_file_id: "org.a.Editor.desktop".to_owned(),
            name: "Editor".to_owned(),
        }],
        None,
        "",
    );
    assert!(chooser.choose("org.evil.desktop", false).is_err());
    assert!(chooser.choose("org.a.Editor.desktop", true).is_ok());
}
