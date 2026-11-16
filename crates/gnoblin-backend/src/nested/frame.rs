//! The window frame a nested session draws for itself, as values: whether one is wanted, how much room it takes, where
//! its four strips lie, what is under a point of them, and what they look like.
//!
//! The session is a client of a host compositor and one of its outputs is a toplevel of that host. The session asks the
//! host to draw the toplevel's frame (`xdg-decoration`, server-side) and follows the answer:
//!
//! | The host                                          | The session draws                          |
//! | ------------------------------------------------- | ------------------------------------------ |
//! | answers server-side                               | nothing: the host, or the shell it runs, owns the frame |
//! | answers client-side                               | a minimal frame, under `auto`               |
//! | has no `xdg-decoration`                           | a minimal frame, under `auto`               |
//! | holds the window full screen                      | nothing                                     |
//! | any of the above, under `off`                     | nothing                                     |
//!
//! # One surface for the session, four strips for the frame
//!
//! The session's own pixels stay the toplevel's surface, at the size the session's output has, so everything that is
//! said about that surface (the pointer's coordinates, the buffer plan, the dmabuf path) is unchanged by a frame. The
//! frame is four strips around it, each a synchronised subsurface that lies outside the surface (a title bar above, a
//! border on each of the other three sides), and `xdg_surface.set_window_geometry` states that the window is the
//! content and the strips together. The host's configure is the size of that window, so the session's output is the
//! configure's size less the [`Inset`] of the strips ([`content_size`]).
//!
//! The wire half is [`super::handler`]'s `host_frame`; nothing here touches a socket.
//!
//! # Failure modes
//!
//! 1. **A frame drawn for a host that draws its own.** A server-side answer and a full-screen window both want none:
//!    `tests::a_host_that_draws_the_frame_gets_none` and `tests::a_full_screen_window_has_no_frame`.
//! 2. **The session's output is the size of the whole window.** A frame takes room from the host's configure and the
//!    output is what is left, never less than a pixel: `tests::the_output_is_the_configure_less_the_frame`.
//! 3. **A point of the frame is taken for the wrong thing.** Corners, edges, buttons and title are tested on window
//!    coordinates, so a strip's local point is the same answer wherever the strip lies:
//!    `tests::the_corners_resize_diagonally`, `tests::a_title_bar_point_is_the_title_or_a_button`.

use gnoblin_config::NestedDecorations;
use gnoblin_paint::{CanvasPainter, PaintList, PaintOp, Rgba8};
use gnoblin_types::{Rect, Size};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, State};

/// The width of the border on the three sides that have one, in logical pixels. It is also the width of the resize band.
pub const BORDER: u32 = 4;

/// The height of the title bar, in logical pixels.
pub const TITLE_HEIGHT: u32 = 32;

/// The width of one title bar button, in logical pixels.
pub const BUTTON_WIDTH: u32 = 36;

/// How far along an edge from a corner a point still resizes diagonally.
const CORNER: u32 = 12;

/// The side of a button's icon.
const ICON: f32 = 16.0;

/// The size the title is drawn at.
const TITLE_SIZE: f32 = 13.0;

/// The room between the title and the left edge, and between the title and the buttons.
const TITLE_PAD: u32 = 12;

/// How long after a press a second one on the title is a double click, in milliseconds.
pub(super) const DOUBLE_CLICK_MS: u32 = 400;

/// What the host said when it was asked for a server-side frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecorationAnswer {
    /// The host draws the frame.
    Server,
    /// The window draws it.
    Client,
}

/// What a toplevel's configure says about the window that changes its frame.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct HostWindowState {
    /// The host has the window maximised.
    pub maximized: bool,
    /// The host has the window full screen.
    pub fullscreen: bool,
    /// The host has the window tiled against an edge.
    pub tiled: bool,
    /// The window has the keyboard.
    pub activated: bool,
}

impl HostWindowState {
    /// The state the `states` array of an `xdg_toplevel.configure` holds: native-endian 32-bit values.
    pub fn from_wire(states: &[u8]) -> Self {
        let mut state = Self::default();
        for value in states.as_chunks::<4>().0 {
            match State::try_from(u32::from_ne_bytes(*value)) {
                Ok(State::Maximized) => state.maximized = true,
                Ok(State::Fullscreen) => state.fullscreen = true,
                Ok(State::Activated) => state.activated = true,
                Ok(State::TiledLeft | State::TiledRight | State::TiledTop | State::TiledBottom) => state.tiled = true,
                _ => {}
            }
        }
        state
    }
}

/// Whether the session draws a frame for a window the host answered `answer` for.
///
/// `answer` is [`None`] when the host has no `xdg-decoration` or has not said, which the protocol states is the same as
/// client-side.
pub(super) fn wants_frame(
    setting: NestedDecorations,
    answer: Option<DecorationAnswer>,
    state: HostWindowState,
) -> bool {
    setting == NestedDecorations::Auto && !state.fullscreen && answer != Some(DecorationAnswer::Server)
}

/// The room a frame takes on each side of the session's output, in logical pixels.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Inset {
    /// The room to the left.
    pub left: u32,
    /// The room above.
    pub top: u32,
    /// The room to the right.
    pub right: u32,
    /// The room below.
    pub bottom: u32,
}

impl Inset {
    /// No frame.
    pub const NONE: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    /// The room the frame of a window in `state` takes: a title bar and a border all round, or a title bar alone while
    /// the host holds the window against the screen's edges, where there is nothing to resize.
    pub const fn of(state: HostWindowState) -> Self {
        if state.maximized || state.tiled {
            Self {
                left: 0,
                top: TITLE_HEIGHT,
                right: 0,
                bottom: 0,
            }
        } else {
            Self {
                left: BORDER,
                top: TITLE_HEIGHT,
                right: BORDER,
                bottom: BORDER,
            }
        }
    }

    /// The room the frame takes across.
    pub const fn horizontal(self) -> u32 {
        self.left + self.right
    }

    /// The room the frame takes down.
    pub const fn vertical(self) -> u32 {
        self.top + self.bottom
    }
}

/// The size the session's output takes for a configure of `width` by `height` (the window, frame included), or
/// `fallback` when the host leaves the size to the session. The output is never less than one pixel across.
pub(super) fn content_size(width: i32, height: i32, inset: Inset, fallback: Size) -> Size {
    if width > 0 && height > 0 {
        Size::new(
            width.unsigned_abs().saturating_sub(inset.horizontal()).max(1),
            height.unsigned_abs().saturating_sub(inset.vertical()).max(1),
        )
    } else {
        fallback
    }
}

/// One of the four strips of the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    /// The title bar, as wide as the window.
    Top,
    /// The border on the left, between the title bar and the bottom border.
    Left,
    /// The border on the right.
    Right,
    /// The border below, as wide as the window.
    Bottom,
}

impl Part {
    /// All four, in the order they are drawn and numbered.
    pub const ALL: [Self; 4] = [Self::Top, Self::Left, Self::Right, Self::Bottom];

    /// The strip's place in a table of four.
    pub const fn index(self) -> usize {
        match self {
            Self::Top => 0,
            Self::Left => 1,
            Self::Right => 2,
            Self::Bottom => 3,
        }
    }
}

/// The title bar's buttons, from left to right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Button {
    /// Asks the host to minimise the window.
    Minimise,
    /// Maximises the window, or restores it when it is maximised.
    Maximise,
    /// Closes the window.
    Close,
}

impl Button {
    /// All three, left to right.
    const ALL: [Self; 3] = [Self::Minimise, Self::Maximise, Self::Close];
}

/// What lies under a point of the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FrameHit {
    /// The title bar: dragging it moves the window, and a double click maximises it.
    Title,
    /// A title bar button.
    Button(Button),
    /// A resize band or corner: dragging it resizes the window from that edge.
    Resize(ResizeEdge),
}

/// How a frame lies around the session's output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FrameLayout {
    /// The size of the session's output, which is the size of its surface.
    pub content: Size,
    /// The room the strips take around it.
    pub inset: Inset,
}

impl FrameLayout {
    /// The window: the output and the strips together, from the surface's top left corner. This is the window geometry
    /// the host is told.
    pub fn window(&self) -> Rect {
        Rect {
            x: -signed(self.inset.left),
            y: -signed(self.inset.top),
            width: self.content.width + self.inset.horizontal(),
            height: self.content.height + self.inset.vertical(),
        }
    }

    /// Where a strip lies, or [`None`] when the frame has none of it.
    pub fn part(&self, part: Part) -> Option<Rect> {
        let inset = self.inset;
        let window = self.window();
        let rect = match part {
            Part::Top => Rect {
                height: inset.top,
                ..window
            },
            Part::Left => Rect {
                x: window.x,
                y: 0,
                width: inset.left,
                height: self.content.height,
            },
            Part::Right => Rect {
                x: signed(self.content.width),
                y: 0,
                width: inset.right,
                height: self.content.height,
            },
            Part::Bottom => Rect {
                y: signed(self.content.height),
                height: inset.bottom,
                ..window
            },
        };
        (rect.width > 0 && rect.height > 0).then_some(rect)
    }

    /// Whether the window can be resized from its frame: the borders are there to grab.
    pub const fn resizable(&self) -> bool {
        self.inset.left > 0
    }

    /// What lies under the point `(x, y)` of the strip `part`, in the strip's own logical pixels.
    pub fn hit(&self, part: Part, x: f64, y: f64) -> Option<FrameHit> {
        let rect = self.part(part)?;
        let window = self.window();
        // The same point on the window, whose top left corner is the origin.
        let wx = f64::from(rect.x - window.x) + x;
        let wy = f64::from(rect.y - window.y) + y;
        let (width, height) = (f64::from(window.width), f64::from(window.height));
        if wx < 0.0 || wy < 0.0 || wx >= width || wy >= height {
            return None;
        }
        if self.resizable() {
            let band = f64::from(BORDER);
            let corner = f64::from(CORNER);
            let (left, right) = (wx < band, wx >= width - band);
            let (top, bottom) = (wy < band, wy >= height - band);
            let near_left = wx < corner;
            let near_right = wx >= width - corner;
            let near_top = wy < corner;
            let near_bottom = wy >= height - corner;
            let edge = if (top && near_left) || (left && near_top) {
                Some(ResizeEdge::TopLeft)
            } else if (top && near_right) || (right && near_top) {
                Some(ResizeEdge::TopRight)
            } else if (bottom && near_left) || (left && near_bottom) {
                Some(ResizeEdge::BottomLeft)
            } else if (bottom && near_right) || (right && near_bottom) {
                Some(ResizeEdge::BottomRight)
            } else if top {
                Some(ResizeEdge::Top)
            } else if bottom {
                Some(ResizeEdge::Bottom)
            } else if left {
                Some(ResizeEdge::Left)
            } else if right {
                Some(ResizeEdge::Right)
            } else {
                None
            };
            if let Some(edge) = edge {
                return Some(FrameHit::Resize(edge));
            }
        }
        if wy >= f64::from(TITLE_HEIGHT) {
            return None;
        }
        Some(self.button_at(wx, width).map_or(FrameHit::Title, FrameHit::Button))
    }

    /// The title bar button at window x `wx`, when the window is `width` wide.
    fn button_at(&self, wx: f64, width: f64) -> Option<Button> {
        let first = width - f64::from(BUTTON_WIDTH) * 3.0;
        if wx < first {
            return None;
        }
        // Truncation is the point: the buttons are equal slices of the bar.
        let slot = ((wx - first) / f64::from(BUTTON_WIDTH)) as usize;
        Button::ALL.get(slot).copied()
    }
}

/// A logical extent as the signed coordinate the protocol positions with.
fn signed(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// What changes how a strip looks.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct Look {
    /// The window has the keyboard.
    pub active: bool,
    /// The window is maximised, so its middle button restores it.
    pub maximized: bool,
    /// The pointer is over this button.
    pub hover: Option<Button>,
    /// The primary button is held down on this button.
    pub pressed: Option<Button>,
}

impl Look {
    /// The part of a look a strip is drawn from: a border shows nothing of the buttons.
    pub fn of(part: Part, look: Self) -> Self {
        match part {
            Part::Top => look,
            _ => Self {
                active: look.active,
                ..Self::default()
            },
        }
    }
}

/// The frame's colours.
const BAR_ACTIVE: Rgba8 = Rgba8::new(0x2d, 0x2d, 0x30, 0xff);
/// The frame's colours while the window has not the keyboard.
const BAR_INACTIVE: Rgba8 = Rgba8::new(0x24, 0x24, 0x26, 0xff);
/// The title and the icons.
const INK_ACTIVE: Rgba8 = Rgba8::new(0xee, 0xee, 0xee, 0xff);
/// The title and the icons while the window has not the keyboard.
const INK_INACTIVE: Rgba8 = Rgba8::new(0x8a, 0x8a, 0x8e, 0xff);
/// A button under the pointer.
const HOVER: Rgba8 = Rgba8::new(0xff, 0xff, 0xff, 0x22);
/// A button held down.
const PRESSED: Rgba8 = Rgba8::new(0xff, 0xff, 0xff, 0x40);
/// The close button under the pointer or held down.
const CLOSE_HOVER: Rgba8 = Rgba8::new(0xc0, 0x1c, 0x28, 0xff);

/// What a strip is drawn as, in the strip's own logical pixels.
pub(super) fn paint_list(
    part: Part,
    layout: &FrameLayout,
    look: Look,
    title: &str,
    painter: &CanvasPainter,
) -> PaintList {
    let Some(rect) = layout.part(part) else {
        return PaintList::default();
    };
    let (width, height) = (rect.width as f32, rect.height as f32);
    let (bar, ink) = if look.active {
        (BAR_ACTIVE, INK_ACTIVE)
    } else {
        (BAR_INACTIVE, INK_INACTIVE)
    };
    let mut ops = vec![PaintOp::Rect {
        x: 0.0,
        y: 0.0,
        width,
        height,
        radius: 0.0,
        fill: bar,
        stroke: Rgba8::TRANSPARENT,
        stroke_width: 0.0,
    }];
    if part != Part::Top {
        return PaintList { ops };
    }
    let buttons = (BUTTON_WIDTH * 3) as f32;
    let pad = TITLE_PAD as f32;
    let room = (width - buttons - pad * 2.0).max(0.0);
    if !title.is_empty() && room > 0.0 {
        let extent = painter.measure(title, TITLE_SIZE, "sans");
        let text_width = extent.width as f32;
        // Centred on the window while it fits, and kept clear of the buttons when it does not: the clip cuts what is
        // left over.
        let centred = (width - text_width) / 2.0;
        let x = centred.clamp(pad, (pad + room - text_width).max(pad));
        ops.push(PaintOp::PushClip {
            x: pad,
            y: 0.0,
            width: room,
            height,
            radius: 0.0,
        });
        ops.push(PaintOp::Text {
            x,
            y: ((height - extent.height as f32) / 2.0).floor(),
            size: TITLE_SIZE,
            colour: ink,
            font: "sans".to_owned(),
            text: title.to_owned(),
        });
        ops.push(PaintOp::PopClip);
    }
    for (slot, button) in Button::ALL.into_iter().enumerate() {
        let x = width - (BUTTON_WIDTH as usize * (3 - slot)) as f32;
        let held = look.pressed == Some(button);
        let over = look.hover == Some(button);
        let (fill, colour) = match (button, over || held) {
            (Button::Close, true) => (CLOSE_HOVER, INK_ACTIVE),
            (_, true) => (if held { PRESSED } else { HOVER }, ink),
            _ => (Rgba8::TRANSPARENT, ink),
        };
        if !fill.is_transparent() {
            ops.push(PaintOp::Rect {
                x,
                y: 0.0,
                width: BUTTON_WIDTH as f32,
                height,
                radius: 0.0,
                fill,
                stroke: Rgba8::TRANSPARENT,
                stroke_width: 0.0,
            });
        }
        let name = match button {
            Button::Minimise => "window-minimize-symbolic",
            Button::Maximise if look.maximized => "window-restore-symbolic",
            Button::Maximise => "window-maximize-symbolic",
            Button::Close => "window-close-symbolic",
        };
        ops.push(PaintOp::Icon {
            x: x + (BUTTON_WIDTH as f32 - ICON) / 2.0,
            y: ((height - ICON) / 2.0).floor(),
            size: ICON,
            colour,
            name: name.to_owned(),
        });
    }
    PaintList { ops }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: Size = Size::new(800, 600);

    fn layout(state: HostWindowState) -> FrameLayout {
        FrameLayout {
            content: CONTENT,
            inset: Inset::of(state),
        }
    }

    fn states(values: &[State]) -> Vec<u8> {
        values.iter().flat_map(|state| (*state as u32).to_ne_bytes()).collect()
    }

    #[test]
    fn the_states_of_a_configure_are_read() {
        let state = HostWindowState::from_wire(&states(&[State::Maximized, State::Activated, State::TiledLeft]));
        assert_eq!(
            state,
            HostWindowState {
                maximized: true,
                fullscreen: false,
                tiled: true,
                activated: true
            }
        );
        assert_eq!(HostWindowState::from_wire(&[]), HostWindowState::default());
        // A value the protocol does not know yet, and a stray byte, change nothing.
        assert_eq!(
            HostWindowState::from_wire(&[0xee, 0, 0, 0, 1]),
            HostWindowState::default()
        );
    }

    #[test]
    fn a_host_that_draws_the_frame_gets_none() {
        let state = HostWindowState::default();
        assert!(!wants_frame(
            NestedDecorations::Auto,
            Some(DecorationAnswer::Server),
            state
        ));
        assert!(wants_frame(
            NestedDecorations::Auto,
            Some(DecorationAnswer::Client),
            state
        ));
        assert!(
            wants_frame(NestedDecorations::Auto, None, state),
            "a host with no xdg-decoration, or one that has not answered, is client-side"
        );
        for answer in [Some(DecorationAnswer::Server), Some(DecorationAnswer::Client), None] {
            assert!(
                !wants_frame(NestedDecorations::Off, answer, state),
                "`off` never draws one: {answer:?}"
            );
        }
    }

    #[test]
    fn a_full_screen_window_has_no_frame() {
        let state = HostWindowState {
            fullscreen: true,
            ..HostWindowState::default()
        };
        assert!(!wants_frame(
            NestedDecorations::Auto,
            Some(DecorationAnswer::Client),
            state
        ));
        assert!(!wants_frame(NestedDecorations::Auto, None, state));
    }

    #[test]
    fn the_output_is_the_configure_less_the_frame() {
        let inset = Inset::of(HostWindowState::default());
        assert_eq!(
            content_size(1024, 768, inset, CONTENT),
            Size::new(1024 - 2 * BORDER, 768 - TITLE_HEIGHT - BORDER)
        );
        assert_eq!(
            content_size(1024, 768, Inset::NONE, CONTENT),
            Size::new(1024, 768),
            "no frame, no room taken"
        );
        assert_eq!(
            content_size(0, 0, inset, CONTENT),
            CONTENT,
            "the host leaves the size to the session"
        );
        assert_eq!(
            content_size(2, 3, inset, CONTENT),
            Size::new(1, 1),
            "a window smaller than its own frame is still an output"
        );
        let maximised = Inset::of(HostWindowState {
            maximized: true,
            ..HostWindowState::default()
        });
        assert_eq!(
            content_size(1920, 1080, maximised, CONTENT),
            Size::new(1920, 1080 - TITLE_HEIGHT),
            "a maximised window has a title bar and no borders"
        );
    }

    #[test]
    fn the_strips_and_the_surface_tile_the_window() {
        let layout = layout(HostWindowState::default());
        let window = layout.window();
        assert_eq!(
            window,
            Rect {
                x: -(BORDER as i32),
                y: -(TITLE_HEIGHT as i32),
                width: 800 + 2 * BORDER,
                height: 600 + TITLE_HEIGHT + BORDER
            }
        );
        let mut area = u64::from(CONTENT.width) * u64::from(CONTENT.height);
        for part in Part::ALL {
            let rect = layout.part(part).expect("every strip has room");
            area += u64::from(rect.width) * u64::from(rect.height);
            // A strip is inside the window, and none is over the surface.
            assert!(rect.x >= window.x && rect.y >= window.y, "{part:?}");
            assert!(
                rect.x + rect.width as i32 <= window.x + window.width as i32
                    && rect.y + rect.height as i32 <= window.y + window.height as i32,
                "{part:?}"
            );
            let over_surface =
                rect.x < 800 && rect.x + rect.width as i32 > 0 && rect.y < 600 && rect.y + rect.height as i32 > 0;
            assert!(!over_surface, "{part:?} lies over the session's pixels");
        }
        assert_eq!(
            area,
            u64::from(window.width) * u64::from(window.height),
            "the surface and the four strips cover the window exactly once"
        );
    }

    #[test]
    fn a_maximised_window_has_only_its_title_bar() {
        let layout = layout(HostWindowState {
            maximized: true,
            ..HostWindowState::default()
        });
        assert!(layout.part(Part::Top).is_some());
        for part in [Part::Left, Part::Right, Part::Bottom] {
            assert_eq!(layout.part(part), None, "{part:?}");
        }
        assert!(!layout.resizable());
        assert_eq!(
            layout.hit(Part::Top, 2.0, 1.0),
            Some(FrameHit::Title),
            "nothing to resize from"
        );
    }

    #[test]
    fn the_corners_resize_diagonally() {
        let layout = layout(HostWindowState::default());
        let top = layout.part(Part::Top).expect("a title bar");
        let width = f64::from(top.width);
        assert_eq!(
            layout.hit(Part::Top, 1.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::TopLeft))
        );
        assert_eq!(
            layout.hit(Part::Top, width - 1.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::TopRight))
        );
        assert_eq!(
            layout.hit(Part::Top, 100.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::Top))
        );
        // The left border's own top, right under the title bar, is still the top left corner's reach.
        assert_eq!(
            layout.hit(Part::Left, 1.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::Left))
        );
        assert_eq!(
            layout.hit(Part::Left, 1.0, 300.0),
            Some(FrameHit::Resize(ResizeEdge::Left))
        );
        assert_eq!(
            layout.hit(Part::Right, 1.0, 300.0),
            Some(FrameHit::Resize(ResizeEdge::Right))
        );
        assert_eq!(
            layout.hit(Part::Left, 1.0, 595.0),
            Some(FrameHit::Resize(ResizeEdge::BottomLeft))
        );
        assert_eq!(
            layout.hit(Part::Right, 1.0, 595.0),
            Some(FrameHit::Resize(ResizeEdge::BottomRight))
        );
        assert_eq!(
            layout.hit(Part::Bottom, 100.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::Bottom))
        );
        assert_eq!(
            layout.hit(Part::Bottom, 1.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::BottomLeft))
        );
        assert_eq!(
            layout.hit(Part::Bottom, width - 1.0, 1.0),
            Some(FrameHit::Resize(ResizeEdge::BottomRight))
        );
        // The title bar's side bands are the left and right edges, and its upper corners reach down by the corner size.
        assert_eq!(
            layout.hit(Part::Top, 1.0, 20.0),
            Some(FrameHit::Resize(ResizeEdge::Left))
        );
        assert_eq!(
            layout.hit(Part::Top, width - 1.0, 20.0),
            Some(FrameHit::Resize(ResizeEdge::Right))
        );
        assert_eq!(
            layout.hit(Part::Top, 1.0, 5.0),
            Some(FrameHit::Resize(ResizeEdge::TopLeft))
        );
    }

    #[test]
    fn a_title_bar_point_is_the_title_or_a_button() {
        let layout = layout(HostWindowState::default());
        let top = layout.part(Part::Top).expect("a title bar");
        let width = f64::from(top.width);
        let button = f64::from(BUTTON_WIDTH);
        assert_eq!(layout.hit(Part::Top, 200.0, 16.0), Some(FrameHit::Title));
        assert_eq!(
            layout.hit(Part::Top, width - button / 2.0 - 5.0, 16.0),
            Some(FrameHit::Button(Button::Close))
        );
        assert_eq!(
            layout.hit(Part::Top, width - button * 1.5 - 5.0, 16.0),
            Some(FrameHit::Button(Button::Maximise))
        );
        assert_eq!(
            layout.hit(Part::Top, width - button * 2.5 - 5.0, 16.0),
            Some(FrameHit::Button(Button::Minimise))
        );
        assert_eq!(
            layout.hit(Part::Top, width - button * 3.0 - 5.0, 16.0),
            Some(FrameHit::Title),
            "left of the buttons is the title"
        );
        assert_eq!(layout.hit(Part::Top, -1.0, 5.0), None, "outside the strip");
    }

    #[test]
    fn a_title_that_does_not_fit_is_clipped_clear_of_the_buttons() {
        let painter = gnoblin_paint::SharedPainter::embedded();
        let layout = FrameLayout {
            content: Size::new(300, 80),
            inset: Inset::of(HostWindowState::default()),
        };
        let long = "A window title much longer than the bar it is drawn in".to_owned();
        let list = paint_list(Part::Top, &layout, Look::default(), &long, painter.get());
        let clip = list
            .ops
            .iter()
            .find_map(|op| match op {
                PaintOp::PushClip { x, width, .. } => Some((*x, *width)),
                _ => None,
            })
            .expect("the title is clipped");
        let bar_width = (300 + 2 * BORDER) as f32;
        assert!(
            clip.0 + clip.1 <= bar_width - (BUTTON_WIDTH * 3) as f32,
            "the clip ends before the buttons begin: {clip:?}"
        );
    }

    #[test]
    fn a_bar_too_narrow_for_a_title_shows_only_its_buttons() {
        let painter = gnoblin_paint::SharedPainter::embedded();
        let layout = FrameLayout {
            content: Size::new(120, 80),
            inset: Inset::of(HostWindowState::default()),
        };
        let list = paint_list(Part::Top, &layout, Look::default(), "A title", painter.get());
        assert!(
            !list.ops.iter().any(|op| matches!(op, PaintOp::Text { .. })),
            "no room between the edge and the buttons, so no text is drawn over them"
        );
        assert_eq!(
            list.ops.iter().filter(|op| matches!(op, PaintOp::Icon { .. })).count(),
            3,
            "the buttons are still there"
        );
    }

    #[test]
    fn the_buttons_are_drawn_with_the_icons_of_their_actions() {
        let painter = gnoblin_paint::SharedPainter::embedded();
        let icons = |look: Look| -> Vec<String> {
            paint_list(Part::Top, &layout(HostWindowState::default()), look, "t", painter.get())
                .ops
                .iter()
                .filter_map(|op| match op {
                    PaintOp::Icon { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            icons(Look::default()),
            [
                "window-minimize-symbolic",
                "window-maximize-symbolic",
                "window-close-symbolic"
            ]
        );
        assert_eq!(
            icons(Look {
                maximized: true,
                ..Look::default()
            })[1],
            "window-restore-symbolic"
        );
    }

    #[test]
    fn a_border_is_one_flat_colour() {
        let painter = gnoblin_paint::SharedPainter::embedded();
        for part in [Part::Left, Part::Right, Part::Bottom] {
            let list = paint_list(
                part,
                &layout(HostWindowState::default()),
                Look::default(),
                "t",
                painter.get(),
            );
            assert_eq!(list.ops.len(), 1, "{part:?}");
        }
    }
}
