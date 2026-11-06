//! The shell runs any [`DialogUi`], not only the portal's own dialog kinds.

use egui::{Key, Ui};
use gnoblin_portal_core::model::{ScreenKey, Settled};
use gnoblin_portal_core::ui::{DialogUi, Setup, Shell, key_events};
use gnoblin_types::Tick;

/// A dialog kind the shell has never heard of: it settles as denied on Escape.
struct Probe;

impl DialogUi for Probe {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        ui.label("probe");
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            Some(Settled::Denied)
        } else {
            None
        }
    }
}

fn shell() -> Shell {
    Shell::new(Setup {
        width: 320,
        height: 160,
        reduced_motion: true,
    })
}

#[test]
fn the_shell_settles_any_dialog_kind() {
    let mut shell = shell();
    let mut probe = Probe;
    shell.layout(&mut probe, Tick::new(0));
    assert_eq!(shell.run(&mut probe, Vec::new(), Tick::new(0)), None);
    assert_eq!(
        shell.run(&mut probe, key_events(ScreenKey::Escape, false), Tick::new(0)),
        Some(Settled::Denied)
    );
}
