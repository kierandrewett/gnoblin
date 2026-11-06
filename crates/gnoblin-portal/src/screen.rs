//! egui screens for the built-in portal dialogs (D5, D19).
//!
//! A [`PortalScreen`] wraps one dialog model and draws it with egui. The models stay
//! the source of truth: a screen forwards key, pointer and tick input to the
//! model and reports the model's decision as a [`Settled`] value. It does no I/O
//! and reads no clock: every input carries its own [`Tick`].
//!
//! Failure modes, decided here rather than left to the caller:
//! - Input after a settled result is ignored and returns `None`.
//! - A pointer press on a button that is not under the pointer resolves to
//!   "elsewhere", which the model ignores.
//! - A disarmed Allow button ignores pointer and Enter input until the arm
//!   delay has passed; the model decides this, the screen only shows it.
//! - Reduced motion (the caller's copy of the settings value, in [`Setup`])
//!   makes every animated value jump to its target immediately.
//! - egui text that cannot be laid out is not drawn; no fallback glyph is
//!   invented. The bundled default font covers the Latin range used by prompts.
//! - Choosers ignore pointer input. Keys reach a chooser only through
//!   [`PortalScreen::key`]; Escape cancels. A held key repeats like a first press.
//! - A FileChooser shows only the listing the caller gives it through
//!   [`PortalScreen::list`]; a listing for any other folder is refused.
//! - A refusal from the model is shown on the screen and leaves the dialog open.

use std::fmt::Write as _;

use egui::epaint::ClippedPrimitive;
use egui::{Context, Pos2, RawInput, Rect, Vec2};
use gnoblin_portal_core::model::{
    AppChooser, Confirmation, Consent, ConsentInput, ConsentTarget, Dialog, DialogError, DirectoryListing, FileChooser,
    FileChooserMode, Key, ScreenKey, Settled,
};
use gnoblin_types::Tick;

use gnoblin_portal_appchooser::draw_app;
use gnoblin_portal_consent::draw_consent;
use gnoblin_portal_core::canvas::{ChooserView, Drawn, Focus, Setup};
use gnoblin_portal_core::paint::{Frame, Textures, rasterise};
use gnoblin_portal_core::widgets::Label;
use gnoblin_portal_filechooser::{accept_label, draw_files, entry_label};

/// One egui point is one frame pixel. The headless renderer does not scale.
const PIXELS_PER_POINT: f32 = 1.0;

/// One dialog on screen. Holds the model, the egui context and the CPU textures.
pub struct PortalScreen {
    ctx: Context,
    textures: Textures,
    dialog: Dialog,
    setup: Setup,
    now: Tick,
    settled: Option<Settled>,
    hits: Vec<(ConsentTarget, Rect)>,
    labels: Vec<Label>,
    cursor: usize,
    focus: Focus,
    remember: bool,
    error: Option<DialogError>,
    listing_request: Option<String>,
}

impl PortalScreen {
    /// A screen for `dialog` at `now`. The first layout runs here, so pointer
    /// hit-testing works before the first [`PortalScreen::render`].
    #[must_use]
    pub fn new(dialog: Dialog, setup: Setup, now: Tick) -> Self {
        let (cursor, focus) = initial_view(&dialog);
        let mut screen = Self {
            ctx: Context::default(),
            textures: Textures::default(),
            dialog,
            setup,
            now,
            settled: None,
            hits: Vec::new(),
            labels: Vec::new(),
            cursor,
            focus,
            remember: false,
            error: None,
            listing_request: None,
        };
        screen.layout();
        screen
    }

    /// The settled result, once the model has decided or the screen was cancelled.
    #[must_use]
    pub const fn settled(&self) -> Option<&Settled> {
        self.settled.as_ref()
    }

    /// The text runs of the last layout, in draw order.
    #[must_use]
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Moves the screen's clock. Animations and the arm delay read this value.
    pub const fn tick(&mut self, now: Tick) {
        self.now = now;
    }

    /// Feeds one key press or repeat. Consent takes Escape, Enter and Tab. A
    /// chooser takes every key it uses and ignores `repeat`, so a held Up or
    /// Down keeps moving.
    pub fn key(&mut self, key: ScreenKey, repeat: bool) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        match self.dialog {
            Dialog::Consent(_) => {
                let key = match key {
                    ScreenKey::Escape => Key::Escape,
                    ScreenKey::Enter => Key::Enter,
                    ScreenKey::Tab => Key::Tab,
                    _ => return None,
                };
                self.consent_input(ConsentInput::Key { key, repeat })
            }
            Dialog::Files(_) | Dialog::App(_) => self.chooser_key(key),
        }
    }

    /// Feeds a pointer press at frame pixel `(x, y)`.
    pub fn pointer_press(&mut self, x: f32, y: f32) -> Option<Settled> {
        let target = self.target_at(x, y);
        self.consent_input(ConsentInput::PointerPress { target })
    }

    /// Feeds a pointer release at frame pixel `(x, y)`.
    pub fn pointer_release(&mut self, x: f32, y: f32) -> Option<Settled> {
        let target = self.target_at(x, y);
        self.consent_input(ConsentInput::PointerRelease { target })
    }

    /// Settles the screen as cancelled: the requester went away or the user
    /// dismissed the dialog outside the model. Returns `None` once settled.
    pub fn cancel(&mut self) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        self.settled = Some(Settled::Cancelled);
        self.settled.clone()
    }

    /// Gives a FileChooser the listing of the folder it asked for. A listing for
    /// any other folder is refused and returns `false`.
    pub fn list(&mut self, listing: DirectoryListing) -> bool {
        if self.settled.is_some() {
            return false;
        }
        let Dialog::Files(chooser) = &mut self.dialog else {
            return false;
        };
        if chooser.folder() != listing.folder.as_str() {
            return false;
        }
        chooser.set_entries(listing.entries);
        self.listing_request = None;
        self.cursor = 0;
        true
    }

    /// The folder a FileChooser entered and needs listed, until [`PortalScreen::list`]
    /// answers it.
    #[must_use]
    pub fn listing_request(&self) -> Option<&str> {
        self.listing_request.as_deref()
    }

    /// Lays out and rasterises the current state.
    #[must_use]
    pub fn render(&mut self) -> Frame {
        let output = self.layout();
        let primitives: Vec<ClippedPrimitive> = self.ctx.tessellate(output.shapes, PIXELS_PER_POINT);
        let mut frame = Frame::filled(
            self.setup.width,
            self.setup.height,
            self.setup.theme.background.to_array(),
        );
        rasterise(&mut frame, &self.textures, &primitives, PIXELS_PER_POINT);
        frame
    }

    /// A plain-text description of what the model holds and what is on screen.
    /// Stable for the same inputs; used as the golden structure check.
    #[must_use]
    pub fn structure(&self) -> String {
        let mut out = String::new();
        match &self.dialog {
            Dialog::Consent(consent) => {
                let _ = writeln!(out, "dialog consent");
                let _ = writeln!(out, "app {}", consent.app_id());
                let _ = writeln!(out, "prompt {}", consent.prompt());
                let _ = writeln!(out, "focus {:?}", consent.focus());
                let _ = writeln!(out, "armed {}", consent.armed(self.now));
                let _ = writeln!(out, "buttons Deny Allow");
                let _ = writeln!(out, "decision {:?}", consent.decision());
            }
            Dialog::Files(chooser) => {
                let _ = writeln!(out, "dialog files");
                let _ = writeln!(out, "mode {:?}", chooser.mode());
                let _ = writeln!(out, "folder {}", chooser.folder());
                let _ = writeln!(out, "multiple {}", chooser.multiple());
                let filter = chooser
                    .filters()
                    .get(chooser.filter_index())
                    .map_or("none", |filter| filter.name.as_str());
                let _ = writeln!(out, "filter {filter}");
                let entries = chooser.entries();
                let names: Vec<String> = chooser
                    .visible()
                    .into_iter()
                    .map(|index| entry_label(&entries[index]))
                    .collect();
                let _ = writeln!(out, "entries {names:?}");
                let selected: Vec<String> = chooser
                    .selection()
                    .into_iter()
                    .map(|index| entry_label(&entries[index]))
                    .collect();
                let _ = writeln!(out, "selected {selected:?}");
                let _ = writeln!(out, "name {}", chooser.name());
                let _ = writeln!(out, "overwrite {}", chooser.overwrite_pending());
                let _ = writeln!(out, "buttons Cancel {}", accept_label(chooser.mode()));
                let _ = writeln!(out, "cursor {}", self.cursor);
                let _ = writeln!(out, "focus {:?}", self.focus);
                let _ = writeln!(out, "error {:?}", self.error);
            }
            Dialog::App(chooser) => {
                let _ = writeln!(out, "dialog app");
                let _ = writeln!(out, "content_type {}", chooser.content_type());
                let _ = writeln!(out, "filter {}", chooser.filter_text());
                let names: Vec<&str> = chooser
                    .visible()
                    .into_iter()
                    .map(|choice| choice.name.as_str())
                    .collect();
                let _ = writeln!(out, "choices {names:?}");
                let _ = writeln!(out, "buttons Cancel Choose");
                let _ = writeln!(out, "cursor {}", self.cursor);
                let _ = writeln!(out, "focus {:?}", self.focus);
                let _ = writeln!(out, "remember {}", self.remember);
                let _ = writeln!(out, "error {:?}", self.error);
            }
        }
        if let Some(request) = &self.listing_request {
            let _ = writeln!(out, "listing_request {request}");
        }
        let _ = writeln!(out, "settled {:?}", self.settled);
        out
    }

    fn consent_input(&mut self, input: ConsentInput) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        let Dialog::Consent(consent) = &mut self.dialog else {
            return None;
        };
        let decision = consent.input(input, self.now)?;
        let settled = match Consent::grant(decision) {
            Some(grant) => Settled::Granted(grant),
            None => Settled::Denied,
        };
        self.settled = Some(settled.clone());
        Some(settled)
    }

    /// Routes a key to a chooser. Escape cancels; other keys go to the focused
    /// control, and the model decides any result.
    fn chooser_key(&mut self, key: ScreenKey) -> Option<Settled> {
        self.error = None;
        let settled = if key == ScreenKey::Escape {
            Some(Settled::Cancelled)
        } else {
            match self.dialog {
                Dialog::Files(_) => self.files_key(key),
                Dialog::App(_) => self.app_key(key),
                Dialog::Consent(_) => None,
            }
        };
        if let Some(settled) = &settled {
            self.settled = Some(settled.clone());
        }
        settled
    }

    fn files_key(&mut self, key: ScreenKey) -> Option<Settled> {
        let Dialog::Files(chooser) = &mut self.dialog else {
            return None;
        };
        let visible = chooser.visible();
        match key {
            ScreenKey::Tab => {
                self.focus = next_in(&files_focus_order(chooser), self.focus);
                None
            }
            ScreenKey::Up | ScreenKey::Down => {
                let down = key == ScreenKey::Down;
                if self.focus == Focus::Filter {
                    let next = step(chooser.filter_index(), chooser.filters().len(), down);
                    chooser.set_filter(next);
                    self.cursor = 0;
                } else {
                    self.cursor = step(self.cursor, visible.len(), down);
                }
                None
            }
            ScreenKey::Space if self.focus == Focus::List => {
                if let Some(&index) = visible.get(self.cursor) {
                    self.error = chooser.select(index, chooser.multiple()).err();
                }
                None
            }
            ScreenKey::Space if self.focus == Focus::Name => {
                edit_name(chooser, Some(' '));
                None
            }
            ScreenKey::Text(c) if self.focus == Focus::Name => {
                edit_name(chooser, Some(c));
                None
            }
            ScreenKey::Backspace if self.focus == Focus::Name => {
                edit_name(chooser, None);
                None
            }
            ScreenKey::Enter => {
                let target = match (self.focus, visible.get(self.cursor)) {
                    (Focus::List, Some(&index)) => Some(index),
                    _ => None,
                };
                let Some(index) = target else {
                    return confirm_files(chooser, &mut self.error);
                };
                if chooser.entries()[index].is_dir {
                    match chooser.enter(index) {
                        Ok(folder) => self.listing_request = Some(folder),
                        Err(refused) => self.error = Some(refused),
                    }
                    self.cursor = 0;
                    None
                } else if chooser.mode() == FileChooserMode::Save {
                    let name = chooser.entries()[index].name.clone();
                    chooser.set_name(name);
                    self.focus = Focus::Name;
                    None
                } else {
                    // Enter confirms a multi-selection already made with Space; with
                    // none made, the entry under the cursor is the choice.
                    let keep = chooser.multiple() && !chooser.selection().is_empty();
                    match if keep { Ok(()) } else { chooser.select(index, false) } {
                        Ok(()) => confirm_files(chooser, &mut self.error),
                        Err(refused) => {
                            self.error = Some(refused);
                            None
                        }
                    }
                }
            }
            _ => None,
        }
    }

    fn app_key(&mut self, key: ScreenKey) -> Option<Settled> {
        let Dialog::App(chooser) = &mut self.dialog else {
            return None;
        };
        match key {
            ScreenKey::Tab => {
                self.focus = next_in(&APP_FOCUS_ORDER, self.focus);
                None
            }
            ScreenKey::Up | ScreenKey::Down => {
                self.cursor = step(self.cursor, chooser.visible().len(), key == ScreenKey::Down);
                None
            }
            ScreenKey::Space if self.focus == Focus::Remember => {
                self.remember = !self.remember;
                None
            }
            ScreenKey::Space if self.focus == Focus::Filter => {
                edit_filter(chooser, Some(' '));
                self.cursor = 0;
                None
            }
            ScreenKey::Text(c) if self.focus == Focus::Filter => {
                edit_filter(chooser, Some(c));
                self.cursor = 0;
                None
            }
            ScreenKey::Backspace if self.focus == Focus::Filter => {
                edit_filter(chooser, None);
                self.cursor = 0;
                None
            }
            ScreenKey::Enter => {
                let id = chooser.visible().get(self.cursor)?.desktop_file_id.clone();
                match chooser.choose(&id, self.remember) {
                    Ok(grant) => Some(Settled::Granted(grant)),
                    Err(refused) => {
                        self.error = Some(refused);
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn target_at(&mut self, x: f32, y: f32) -> ConsentTarget {
        self.layout();
        let point = Pos2::new(x, y);
        match self.hits.iter().find(|(_, rect)| rect.contains(point)) {
            Some((target, _)) => *target,
            None => ConsentTarget::Elsewhere,
        }
    }

    /// Runs egui over the current state, records the hit rectangles and labels,
    /// and applies the texture delta. The delta is applied and its frees are
    /// processed here, before any mesh of this pass is painted: no mesh of this
    /// pass references a freed texture, and the CPU painter has no earlier
    /// frame still in flight.
    fn layout(&mut self) -> egui::FullOutput {
        let frame_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(self.setup.width as f32, self.setup.height as f32));
        let raw = RawInput {
            screen_rect: Some(frame_rect),
            time: Some(self.now.get() as f64 / 1000.0),
            ..RawInput::default()
        };
        let mut drawn = Drawn::default();
        let dialog = &self.dialog;
        let setup = &self.setup;
        let now = self.now;
        let view = ChooserView {
            cursor: self.cursor,
            focus: self.focus,
            remember: self.remember,
            error: self.error,
        };
        let mut output = self.ctx.run_ui(raw, |ui| match dialog {
            Dialog::Consent(consent) => draw_consent(ui, consent, setup, now, &mut drawn),
            Dialog::Files(chooser) => draw_files(ui, chooser, &view, setup, &mut drawn),
            Dialog::App(chooser) => draw_app(ui, chooser, &view, setup, &mut drawn),
        });
        self.textures.update(&output.textures_delta);
        self.textures.free(&output.textures_delta);
        // egui panics when a TexturesDelta is dropped with unapplied deltas.
        output.textures_delta.clear();
        self.hits = drawn.hits;
        self.labels = drawn.labels;
        output
    }
}

/// The cursor and focus a dialog opens with. An AppChooser starts in its filter
/// with the previous choice under the cursor when it is still offered.
fn initial_view(dialog: &Dialog) -> (usize, Focus) {
    match dialog {
        Dialog::App(app) => {
            let cursor = app
                .last_choice()
                .and_then(|last| app.visible().iter().position(|choice| choice.desktop_file_id == last))
                .unwrap_or(0);
            (cursor, Focus::Filter)
        }
        Dialog::Files(_) | Dialog::Consent(_) => (0, Focus::List),
    }
}

/// The next control in `order`, wrapping; the first when `current` is absent.
fn next_in(order: &[Focus], current: Focus) -> Focus {
    match order.iter().position(|focus| *focus == current) {
        Some(index) => order[(index + 1) % order.len()],
        None => order[0],
    }
}

/// Moves a cursor one step within `0..len`, clamped at both ends.
fn step(current: usize, len: usize, down: bool) -> usize {
    if down {
        (current + 1).min(len.saturating_sub(1))
    } else {
        current.saturating_sub(1)
    }
}

const APP_FOCUS_ORDER: [Focus; 3] = [Focus::Filter, Focus::List, Focus::Remember];

fn files_focus_order(chooser: &FileChooser) -> Vec<Focus> {
    let mut order = vec![Focus::List];
    if chooser.mode() == FileChooserMode::Save {
        order.push(Focus::Name);
    }
    if !chooser.filters().is_empty() {
        order.push(Focus::Filter);
    }
    order
}

/// Appends one character to the save name, removes the last one, or does
/// nothing when `edit` is `None` and the name is empty.
fn edit_name(chooser: &mut FileChooser, edit: Option<char>) {
    let mut name = chooser.name().to_owned();
    match edit {
        Some(c) => name.push(c),
        None => {
            name.pop();
        }
    }
    chooser.set_name(name);
}

fn edit_filter(chooser: &mut AppChooser, edit: Option<char>) {
    let mut text = chooser.filter_text().to_owned();
    match edit {
        Some(c) => text.push(c),
        None => {
            text.pop();
        }
    }
    chooser.set_filter_text(text);
}

/// Asks the model to confirm. An overwrite question is not a result: the
/// screen shows it and waits for the next confirm.
fn confirm_files(chooser: &mut FileChooser, error: &mut Option<DialogError>) -> Option<Settled> {
    match chooser.confirm() {
        Ok(Confirmation::Chosen(grant)) => Some(Settled::Granted(grant)),
        Ok(Confirmation::OverwriteAsk { .. }) => None,
        Err(refused) => {
            *error = Some(refused);
            None
        }
    }
}
