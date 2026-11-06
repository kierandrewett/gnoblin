//! The AppChooser dialog: the content type, the filter text, the application list, the
//! remember checkbox, a status line, and the Cancel and Choose buttons.

use egui::{CornerRadius, FontId, Pos2, Rect, Ui, Vec2};
use gnoblin_portal_core::canvas::{
    BUTTON_RADIUS, CARD_RADIUS, ChooserView, Drawn, Focus, ListView, MARGIN, ROW_PADDING, Setup, button_rects,
    card_rect, draw_centred, draw_list, draw_text, error_text, focus_outline,
};
use gnoblin_portal_core::model::AppChooser;

/// Draws an AppChooser: title, content type, filter text, the application list,
/// the remember checkbox, a status line, and the Cancel and Choose buttons.
pub fn draw_app(ui: &Ui, chooser: &AppChooser, view: &ChooserView, setup: &Setup, drawn: &mut Drawn) {
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
        "Open With",
        title_font,
        theme.text,
        wrap,
        Pos2::new(left, card.top() + MARGIN),
    );
    let kind = draw_text(
        painter,
        drawn,
        chooser.content_type(),
        body_font.clone(),
        theme.muted,
        wrap,
        Pos2::new(left, title.bottom() + 8.0),
    );

    let filter_top = kind.bottom() + 8.0;
    let filter_line = Rect::from_min_size(Pos2::new(left, filter_top), Vec2::new(wrap, row));
    let filter_text = format!("Filter: {}", chooser.filter_text());
    let at = Pos2::new(left + 6.0, filter_top + 5.0);
    draw_text(painter, drawn, &filter_text, body_font.clone(), theme.text, wrap, at);
    if view.focus == Focus::Filter {
        focus_outline(painter, theme, filter_line);
    }

    let (cancel_rect, choose_rect) = button_rects(card);
    let remember_top = choose_rect.top() - 12.0 - row;
    let remember_line = Rect::from_min_size(Pos2::new(left, remember_top), Vec2::new(wrap, row));
    let status_top = remember_top - row;
    let list_area = Rect::from_min_max(
        Pos2::new(left, filter_line.bottom() + 8.0),
        Pos2::new(left + wrap, status_top - 8.0),
    );
    let rows: Vec<String> = chooser
        .visible()
        .into_iter()
        .map(|choice| choice.name.clone())
        .collect();
    let list = ListView {
        rows: &rows,
        area: list_area,
        cursor: view.cursor,
        focused: view.focus == Focus::List,
        empty: "(no applications)",
    };
    draw_list(painter, drawn, theme, &body_font, &list);

    if let Some(refused) = view.error {
        let at = Pos2::new(left, status_top);
        draw_text(
            painter,
            drawn,
            error_text(refused),
            body_font.clone(),
            theme.accent,
            wrap,
            at,
        );
    }
    let mark = if view.remember { "[x]" } else { "[ ]" };
    let at = Pos2::new(left + 6.0, remember_top + 5.0);
    draw_text(
        painter,
        drawn,
        &format!("{mark} Remember this choice"),
        body_font.clone(),
        theme.text,
        wrap,
        at,
    );
    if view.focus == Focus::Remember {
        focus_outline(painter, theme, remember_line);
    }

    let radius = CornerRadius::same(BUTTON_RADIUS);
    painter.rect_filled(cancel_rect, radius, theme.button);
    painter.rect_filled(choose_rect, radius, theme.button);
    draw_centred(painter, drawn, "Cancel", cancel_rect, body_font.clone(), theme.text);
    draw_centred(painter, drawn, "Choose", choose_rect, body_font, theme.text);
}
