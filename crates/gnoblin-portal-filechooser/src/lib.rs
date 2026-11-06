//! The FileChooser dialog: the listing, the save name and filter lines, the status line,
//! and the Cancel and accept buttons.

use egui::{CornerRadius, FontId, Pos2, Rect, Ui, Vec2};
use gnoblin_portal_core::canvas::{
    BUTTON_RADIUS, CARD_RADIUS, ChooserView, Drawn, Focus, ListView, MARGIN, ROW_PADDING, Setup, button_rects,
    card_rect, draw_centred, draw_list, draw_text, error_text, focus_outline,
};
use gnoblin_portal_core::model::{DialogError, FileChooser, FileChooserMode, FileEntry};

pub fn entry_label(entry: &FileEntry) -> String {
    if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    }
}

fn files_title(mode: FileChooserMode, multiple: bool) -> &'static str {
    match (mode, multiple) {
        (FileChooserMode::Open, false) => "Open File",
        (FileChooserMode::Open, true) => "Open Files",
        (FileChooserMode::Save, _) => "Save File",
        (FileChooserMode::Folder, _) => "Choose Folder",
    }
}

pub fn accept_label(mode: FileChooserMode) -> &'static str {
    match mode {
        FileChooserMode::Open => "Open",
        FileChooserMode::Save => "Save",
        FileChooserMode::Folder => "Select",
    }
}

/// The status line of a FileChooser: the pending overwrite question, else the
/// last refusal.
fn files_status(chooser: &FileChooser, error: Option<DialogError>) -> Option<String> {
    if chooser.overwrite_pending() {
        return Some(format!(
            "{} exists. Press Enter again to replace it, or Escape.",
            chooser.name()
        ));
    }
    error.map(|refused| error_text(refused).to_owned())
}

/// Draws a FileChooser: title, folder, status line, listing, the save name and
/// filter lines, and the Cancel and accept buttons.
pub fn draw_files(ui: &Ui, chooser: &FileChooser, view: &ChooserView, setup: &Setup, drawn: &mut Drawn) {
    let theme = &setup.theme;
    let painter = ui.painter();
    let card = card_rect(setup);
    painter.rect_filled(card, CornerRadius::same(CARD_RADIUS), theme.card);

    let wrap = card.width() - 2.0 * MARGIN;
    let left = card.left() + MARGIN;
    let title_font = FontId::proportional(theme.font_size + 4.0);
    let body_font = FontId::proportional(theme.font_size);
    let row = theme.font_size + ROW_PADDING;

    let title = draw_text(
        painter,
        drawn,
        files_title(chooser.mode(), chooser.multiple()),
        title_font,
        theme.text,
        wrap,
        Pos2::new(left, card.top() + MARGIN),
    );
    let path = draw_text(
        painter,
        drawn,
        chooser.folder(),
        body_font.clone(),
        theme.muted,
        wrap,
        Pos2::new(left, title.bottom() + 8.0),
    );
    let status_top = path.bottom() + 8.0;
    if let Some(status) = files_status(chooser, view.error) {
        let at = Pos2::new(left, status_top);
        draw_text(painter, drawn, &status, body_font.clone(), theme.accent, wrap, at);
    }

    // Footer lines sit above the buttons: the save name, then the filter.
    let mut footer: Vec<(Focus, String)> = Vec::new();
    if chooser.mode() == FileChooserMode::Save {
        footer.push((Focus::Name, format!("Name: {}", chooser.name())));
    }
    if let Some(filter) = chooser.filters().get(chooser.filter_index()) {
        footer.push((Focus::Filter, format!("Filter: {}", filter.name)));
    }
    let (cancel_rect, accept_rect) = button_rects(card);
    let footer_top = accept_rect.top() - 12.0 - row * footer.len() as f32;
    let list_area = Rect::from_min_max(
        Pos2::new(left, status_top + row),
        Pos2::new(left + wrap, footer_top - 8.0),
    );

    let entries = chooser.entries();
    let selected = chooser.selection();
    let rows: Vec<String> = chooser
        .visible()
        .into_iter()
        .map(|index| {
            let marker = if selected.contains(&index) { "[x] " } else { "    " };
            format!("{marker}{}", entry_label(&entries[index]))
        })
        .collect();
    let list = ListView {
        rows: &rows,
        area: list_area,
        cursor: view.cursor,
        focused: view.focus == Focus::List,
        empty: "(nothing here)",
    };
    draw_list(painter, drawn, theme, &body_font, &list);

    for (index, (focus, text)) in footer.iter().enumerate() {
        let top = footer_top + index as f32 * row;
        let line = Rect::from_min_size(Pos2::new(left, top), Vec2::new(wrap, row));
        let at = Pos2::new(left + 6.0, top + 5.0);
        draw_text(painter, drawn, text, body_font.clone(), theme.text, wrap, at);
        if view.focus == *focus {
            focus_outline(painter, theme, line);
        }
    }

    let radius = CornerRadius::same(BUTTON_RADIUS);
    painter.rect_filled(cancel_rect, radius, theme.button);
    painter.rect_filled(accept_rect, radius, theme.button);
    draw_centred(painter, drawn, "Cancel", cancel_rect, body_font.clone(), theme.text);
    draw_centred(
        painter,
        drawn,
        accept_label(chooser.mode()),
        accept_rect,
        body_font,
        theme.text,
    );
}
