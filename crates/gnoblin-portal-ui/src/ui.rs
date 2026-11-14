//! The dialog shell: one egui context per dialog, the trait every dialog kind implements,
//! and the translation from compositor keys and pointers into egui input.
//!
//! A dialog kind is one UI crate that implements [`DialogUi`] around its model. The
//! shell owns everything else: the context, the textures, the input events and the
//! rasterised frame. egui owns focus, Tab, arrows and button activation.

use egui::epaint::{ClippedShape, Shape};
use egui::{
    Context, Event, Frame as EguiFrame, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Theme, ThemePreference,
    Ui, Vec2, Visuals,
};
use gnoblin_portal_core::model::{DialogError, DirectoryListing, ScreenKey, Settled};
use gnoblin_types::Tick;

use crate::paint::{Frame, Textures, rasterise};
use crate::style::{DialogStyle, ResolvedStyle, publish};

/// The narrowest a dialog frame is drawn, in pixels. A dialog with one short line
/// would otherwise be a sliver. This is a floor on width only: height follows the
/// content, and the bounds still win when they are narrower.
const MIN_WIDTH: f32 = 360.0;

/// The light or dark egui visuals a dialog is drawn with. The compositor maps its
/// `appearance.color-scheme` onto this: `prefer-light` is [`DialogScheme::Light`],
/// and the default and `prefer-dark` are [`DialogScheme::Dark`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogScheme {
    /// egui's stock dark visuals.
    #[default]
    Dark,
    /// egui's stock light visuals.
    Light,
}

impl DialogScheme {
    /// The egui theme and visuals this scheme draws with: the stock light or dark set.
    fn theme(self) -> (Theme, Visuals) {
        match self {
            Self::Dark => (Theme::Dark, Visuals::dark()),
            Self::Light => (Theme::Light, Visuals::light()),
        }
    }
}

/// The largest frame a dialog may be drawn into, in pixels. A dialog is drawn at
/// its content's size, up to these bounds; the owner sets them to the output's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    /// The largest width in pixels.
    pub width: usize,
    /// The largest height in pixels.
    pub height: usize,
    /// Reduced motion: egui's animations finish at once.
    pub reduced_motion: bool,
    /// The visuals the dialog is drawn with.
    pub scheme: DialogScheme,
    /// The clamped look: radius, accent, font, scale and button layout.
    pub style: DialogStyle,
}

/// One dialog kind. A UI crate implements this around its model; the shell calls it.
/// `Send` because the compositor moves a live screen onto its IPC dispatch thread.
pub trait DialogUi: Send {
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
pub struct DialogShell {
    ctx: Context,
    /// The egui theme of `scheme`, which every style lookup uses.
    theme: Theme,
    textures: Textures,
    setup: Setup,
    /// The style as the dialogs read it: the request after clamping, and the accent after correction against this
    /// theme's dialog colour.
    resolved: ResolvedStyle,
    /// The scale the dialog is drawn at now: the style's, lowered when the dialog would not fit its bounds.
    scale: f32,
    /// The surface size: the content's size, within the bounds.
    size: (usize, usize),
}

impl DialogShell {
    /// A shell for frames of `setup`. The look is egui's stock visuals for the
    /// setup's scheme. The surface starts at the bounds; the first render fits it to
    /// the content.
    #[must_use]
    pub fn new(setup: Setup) -> Self {
        let ctx = Context::default();
        let (theme, visuals) = setup.scheme.theme();
        ctx.set_theme(match theme {
            Theme::Light => ThemePreference::Light,
            Theme::Dark => ThemePreference::Dark,
        });
        ctx.set_visuals_of(theme, visuals);
        let mut resolved = ResolvedStyle::default();
        ctx.style_mut_of(theme, |style| {
            if setup.reduced_motion {
                style.animation_time = 0.0;
            }
            resolved = setup.style.apply(style);
        });
        // One egui point is `scale` frame pixels' worth of layout: the dialog is laid out in points and drawn at
        // `scale` pixels to the point.
        let scale = setup.style.scale();
        ctx.set_zoom_factor(scale);
        let size = (setup.width, setup.height);
        Self {
            ctx,
            theme,
            textures: Textures::default(),
            setup,
            resolved,
            scale,
            size,
        }
    }

    /// Sets the largest frame the dialog may take; the owner passes the output's
    /// size. The surface shrinks at once if it is now too large, and the next render
    /// fits it to the content within the new bounds.
    pub fn set_bounds(&mut self, width: usize, height: usize) {
        self.setup.width = width;
        self.setup.height = height;
        self.size = (self.size.0.min(width), self.size.1.min(height));
    }

    /// The size the dialog is drawn at: its content, within the bounds.
    #[must_use]
    pub const fn size(&self) -> (usize, usize) {
        self.size
    }

    /// Fits the surface to the dialog and runs one frame with no input. Pointer
    /// hit-testing needs a laid-out frame, and the owner needs the true size before
    /// it places the dialog, so the owner runs this once before the first event.
    pub fn layout(&mut self, dialog: &mut dyn DialogUi, now: Tick) {
        self.fit(dialog, now);
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

    /// Lays out and rasterises the dialog's current state. The surface is first fitted
    /// to the dialog's content, so the frame is only as large as the dialog needs.
    #[must_use]
    pub fn render(&mut self, dialog: &mut dyn DialogUi, now: Tick) -> Frame {
        self.fit(dialog, now);
        let (_, shapes, _) = self.run_frame(dialog, Vec::new(), now);
        let scale = self.pixels_per_point();
        let primitives = self.ctx.tessellate(shapes, scale);
        let mut frame = Frame::filled(
            self.size.0,
            self.size.1,
            self.ctx.style_of(self.theme).visuals.panel_fill.to_array(),
        );
        rasterise(&mut frame, &self.textures, &primitives, scale);
        frame
    }

    /// What the dialog draws right now: every line of text and every filled rectangle, in points. For checks that a
    /// style cannot hide a line the dialog must show (who asks, the lock) or shrink a control below its floor.
    #[must_use]
    pub fn shown(&mut self, dialog: &mut dyn DialogUi, now: Tick) -> Shown {
        self.fit(dialog, now);
        let (_, shapes, _) = self.run_frame(dialog, Vec::new(), now);
        let mut shown = Shown::default();
        for clipped in &shapes {
            collect(&clipped.shape, &mut shown);
        }
        shown
    }

    /// How many frame pixels one egui point is: the style's scale, or less when the dialog would not fit its bounds
    /// at that scale (see [`Self::fit`]).
    const fn pixels_per_point(&self) -> f32 {
        self.scale
    }

    /// Sizes the surface to the dialog's content, within the bounds.
    ///
    /// The dialog is measured once with the surface at the bounds, then the surface
    /// takes the content's size. Measuring at the bounds rather than at the current
    /// size is deliberate: a label wraps and a list fills the space it is given, so a
    /// measurement made in a surface already shrunk to an earlier guess could describe
    /// a different dialog.
    ///
    /// A style's scale is a request, not a right: a dialog that is taller than its bounds at that scale would have its
    /// buttons or its requester clipped off the surface, so the scale is lowered, down to stock, until it fits.
    fn fit(&mut self, dialog: &mut dyn DialogUi, now: Tick) {
        let bounds = (self.setup.width, self.setup.height);
        self.scale = self.setup.style.scale();
        let mut content = self.measure(dialog, now, bounds);
        for _ in 0..3 {
            let too_big = content.x > bounds.0 as f32 || content.y > bounds.1 as f32;
            if self.scale <= 1.0 || !too_big {
                break;
            }
            let shrink = (bounds.0 as f32 / content.x).min(bounds.1 as f32 / content.y);
            self.scale = (self.scale * shrink * 0.98).max(1.0);
            content = self.measure(dialog, now, bounds);
        }
        let min_width = MIN_WIDTH * self.scale;
        self.size = fitted_size(content, min_width, bounds);
    }

    /// The dialog's content size in frame pixels with the surface at `bounds`.
    fn measure(&mut self, dialog: &mut dyn DialogUi, now: Tick, bounds: (usize, usize)) -> Vec2 {
        self.size = bounds;
        self.run_frame(dialog, Vec::new(), now).2
    }

    /// Runs one egui frame and returns what settled, the shapes, and the content's size in frame pixels.
    fn run_frame(
        &mut self,
        dialog: &mut dyn DialogUi,
        events: Vec<Event>,
        now: Tick,
    ) -> (Option<Settled>, Vec<ClippedShape>, Vec2) {
        let scale = self.pixels_per_point();
        self.ctx.set_zoom_factor(scale);
        // egui lays out in points; the surface and the pointer are in frame pixels.
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(self.size.0 as f32 / scale, self.size.1 as f32 / scale),
            )),
            time: Some(now.get() as f64 / 1000.0),
            events: events.into_iter().map(|event| in_points(event, scale)).collect(),
            ..RawInput::default()
        };
        publish(&self.ctx, self.resolved);
        // The panel frame is egui's stock window frame. A plain `Frame` sizes itself to
        // its content, unlike `CentralPanel`, which always fills the surface: measuring
        // the frame's own rect therefore gives the content's size, margin included.
        let panel_frame = EguiFrame::window(&self.ctx.style_of(self.theme));
        let mut settled = None;
        let mut content = Vec2::ZERO;
        let mut output = self.ctx.run_ui(raw, |root| {
            let framed = panel_frame.show(root, |panel| {
                settled = dialog.show(panel, now);
            });
            content = framed.response.rect.size();
        });
        self.textures.update(&output.textures_delta);
        self.textures.free(&output.textures_delta);
        output.textures_delta.clear();
        (settled, output.shapes, content * scale)
    }
}

/// What a dialog draws: its lines of text and its filled rectangles, in egui points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// Every line of text, in paint order.
    pub texts: Vec<String>,
    /// Every filled or outlined rectangle, in paint order.
    pub rects: Vec<Rect>,
}

fn collect(shape: &Shape, shown: &mut Shown) {
    match shape {
        Shape::Vec(inner) => {
            for shape in inner {
                collect(shape, shown);
            }
        }
        Shape::Text(text) => shown.texts.push(text.galley.text().to_owned()),
        Shape::Rect(rect) => shown.rects.push(rect.rect),
        _ => {}
    }
}

/// A pointer event moved from frame pixels into egui points. Other events carry no position.
fn in_points(event: Event, scale: f32) -> Event {
    let point = |pos: Pos2| Pos2::new(pos.x / scale, pos.y / scale);
    match event {
        Event::PointerMoved(pos) => Event::PointerMoved(point(pos)),
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } => Event::PointerButton {
            pos: point(pos),
            button,
            pressed,
            modifiers,
        },
        other => other,
    }
}

/// The surface size for a dialog whose content, margin included, is `content` pixels: at
/// least `min_width` wide, and never larger than `bounds` on either axis.
#[must_use]
fn fitted_size(content: Vec2, min_width: f32, bounds: (usize, usize)) -> (usize, usize) {
    let width = content.x.ceil().max(min_width).min(bounds.0 as f32);
    let height = content.y.ceil().min(bounds.1 as f32);
    (width as usize, height as usize)
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
            scheme: DialogScheme::Dark,
            style: DialogStyle::default(),
        }
    }

    #[test]
    fn shell_drives_any_dialog_kind_through_the_trait() {
        let mut shell = DialogShell::new(setup());
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
        // The surface is the measured content, clamped to the bounds: the 360 px
        // minimum width is clamped to the 320 px output, and the height is the
        // button's content, shorter than the output.
        let (width, height) = shell.size();
        assert_eq!(width, 320, "the minimum width is clamped to the bounds");
        assert!(
            height > 0 && height < 160,
            "the surface fits the content, not the output"
        );
        let frame = shell.render(&mut clickable, Tick::new(0));
        assert_eq!((frame.width(), frame.height()), (width, height));
        assert_eq!(frame.bytes().len(), width * height * 4);
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
