//! The dialog shell: one egui context per dialog, the trait every dialog kind implements,
//! and the translation from compositor keys and pointers into egui input.
//!
//! A dialog kind is one UI crate that implements [`DialogUi`] around its model. The
//! shell owns everything else: the context, the textures, the input events and the
//! rasterised frame. egui owns focus, Tab, arrows and button activation.

use egui::epaint::ClippedShape;
use egui::{
    CentralPanel, Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Theme, ThemePreference, Ui,
    Vec2, Visuals,
};
use gnoblin_types::Tick;

use crate::model::{DialogError, DirectoryListing, ScreenKey, Settled};
use crate::paint::{Frame, Textures, rasterise};

/// One egui point is one frame pixel. The headless renderer does not scale.
const PIXELS_PER_POINT: f32 = 1.0;

/// The frame a dialog is drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub width: usize,
    pub height: usize,
    /// Reduced motion: egui's animations finish at once.
    pub reduced_motion: bool,
}

/// One dialog kind. A UI crate implements this around its model; the shell calls it.
pub trait DialogUi {
    /// Draws the dialog for this frame and returns the result once it settles.
    fn show(&mut self, ui: &mut Ui, now: Tick) -> Option<Settled>;

    /// The folder a FileChooser entered and needs listed. Dialogs without a
    /// listing never ask.
    fn listing_request(&self) -> Option<&str> {
        None
    }

    /// Gives the dialog the listing it asked for. Returns `false` when the dialog
    /// is not waiting for that listing.
    fn list(&mut self, _listing: DirectoryListing) -> bool {
        false
    }
}

/// Draws one dialog at a time: runs egui frames for input and rasterises the output.
pub struct Shell {
    ctx: Context,
    textures: Textures,
    setup: Setup,
}

impl Shell {
    /// A shell for frames of `setup`. The look is egui's stock dark visuals.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        let ctx = Context::default();
        ctx.set_theme(ThemePreference::Dark);
        ctx.set_visuals_of(Theme::Dark, Visuals::dark());
        if setup.reduced_motion {
            ctx.style_mut_of(Theme::Dark, |style| style.animation_time = 0.0);
        }
        Self {
            ctx,
            textures: Textures::default(),
            setup,
        }
    }

    /// Runs one frame with no input. Pointer hit-testing needs a laid-out frame, so
    /// the owner runs this once before the first event.
    pub fn layout(&mut self, dialog: &mut dyn DialogUi, now: Tick) {
        self.run_frame(dialog, Vec::new(), now);
    }

    /// Feeds `events` to the dialog, one egui frame each, in order. Button presses
    /// need the frame that laid the button out, so each event gets its own frame.
    /// Returns the first result the dialog settles with.
    pub fn run(&mut self, dialog: &mut dyn DialogUi, events: Vec<Event>, now: Tick) -> Option<Settled> {
        for event in events {
            if let Some(settled) = self.run_frame(dialog, vec![event], now).0 {
                return Some(settled);
            }
        }
        None
    }

    /// Lays out and rasterises the dialog's current state.
    #[must_use]
    pub fn render(&mut self, dialog: &mut dyn DialogUi, now: Tick) -> Frame {
        let (_, shapes) = self.run_frame(dialog, Vec::new(), now);
        let primitives = self.ctx.tessellate(shapes, PIXELS_PER_POINT);
        let mut frame = Frame::filled(
            self.setup.width,
            self.setup.height,
            self.ctx.style_of(Theme::Dark).visuals.panel_fill.to_array(),
        );
        rasterise(&mut frame, &self.textures, &primitives, PIXELS_PER_POINT);
        frame
    }

    fn run_frame(
        &mut self,
        dialog: &mut dyn DialogUi,
        events: Vec<Event>,
        now: Tick,
    ) -> (Option<Settled>, Vec<ClippedShape>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(self.setup.width as f32, self.setup.height as f32),
            )),
            time: Some(now.get() as f64 / 1000.0),
            events,
            ..RawInput::default()
        };
        let mut settled = None;
        let mut output = self.ctx.run_ui(raw, |root| {
            CentralPanel::default().show(root, |panel| settled = dialog.show(panel, now));
        });
        self.textures.update(&output.textures_delta);
        self.textures.free(&output.textures_delta);
        output.textures_delta.clear();
        (settled, output.shapes)
    }
}

/// The egui events for one compositor key: a press then a release, or one text event.
#[must_use]
pub fn key_events(key: ScreenKey, repeat: bool) -> Vec<Event> {
    let key = match key {
        ScreenKey::Text(character) => return vec![Event::Text(character.to_string())],
        ScreenKey::Escape => Key::Escape,
        ScreenKey::Enter => Key::Enter,
        ScreenKey::Tab => Key::Tab,
        ScreenKey::Up => Key::ArrowUp,
        ScreenKey::Down => Key::ArrowDown,
        ScreenKey::Space => Key::Space,
        ScreenKey::Backspace => Key::Backspace,
    };
    [true, false]
        .into_iter()
        .map(|pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: repeat && pressed,
            modifiers: Modifiers::NONE,
        })
        .collect()
}

/// The egui events for a primary-button press at frame pixel `(x, y)`. The pointer
/// moves there first so egui hovers the widget before it is pressed.
#[must_use]
pub fn press_events(x: f32, y: f32) -> Vec<Event> {
    pointer_events(x, y, true)
}

/// The egui events for a primary-button release at frame pixel `(x, y)`.
#[must_use]
pub fn release_events(x: f32, y: f32) -> Vec<Event> {
    pointer_events(x, y, false)
}

fn pointer_events(x: f32, y: f32, pressed: bool) -> Vec<Event> {
    let pos = Pos2::new(x, y);
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

/// The words a refusal shows. The model names the refusal; these are its text.
#[must_use]
pub const fn error_text(refused: DialogError) -> &'static str {
    match refused {
        DialogError::EmptyName => "Enter a name first.",
        DialogError::InvalidName => "That name is not allowed.",
        DialogError::NoSelection => "Select something first.",
        DialogError::NotSelectable => "That cannot be chosen here.",
        DialogError::NotAFile => "That name is a folder.",
        DialogError::OutOfRange => "That entry is not shown.",
        DialogError::TooMany => "Only one item can be chosen.",
        DialogError::NotOffered => "That application was not offered.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dialog kind that settles as cancelled on Escape.
    struct Escapable;

    impl DialogUi for Escapable {
        fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
            ui.input(|input| input.key_pressed(Key::Escape))
                .then_some(Settled::Cancelled)
        }
    }

    /// A second dialog kind with a button. It settles as denied when clicked.
    struct Clickable;

    impl DialogUi for Clickable {
        fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
            ui.button("Go").clicked().then_some(Settled::Denied)
        }
    }

    fn setup() -> Setup {
        Setup {
            width: 320,
            height: 160,
            reduced_motion: false,
        }
    }

    #[test]
    fn shell_drives_any_dialog_kind_through_the_trait() {
        let mut shell = Shell::new(setup());
        let mut escapable = Escapable;
        let mut clickable = Clickable;
        {
            let dialogs: [&mut dyn DialogUi; 2] = [&mut escapable, &mut clickable];
            for dialog in dialogs {
                shell.layout(dialog, Tick::new(0));
            }
        }
        assert_eq!(
            shell.run(&mut escapable, key_events(ScreenKey::Escape, false), Tick::new(0)),
            Some(Settled::Cancelled)
        );
        let frame = shell.render(&mut clickable, Tick::new(0));
        assert_eq!(frame.bytes().len(), 320 * 160 * 4);
    }

    #[test]
    fn key_events_press_then_release_and_text_is_one_event() {
        let pressed = key_events(ScreenKey::Enter, false);
        assert_eq!(pressed.len(), 2);
        assert!(matches!(
            pressed[0],
            Event::Key {
                key: Key::Enter,
                pressed: true,
                ..
            }
        ));
        assert!(matches!(
            pressed[1],
            Event::Key {
                key: Key::Enter,
                pressed: false,
                ..
            }
        ));
        assert_eq!(
            key_events(ScreenKey::Text('a'), false),
            vec![Event::Text("a".to_owned())]
        );
    }
}
