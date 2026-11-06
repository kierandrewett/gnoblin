//! The consent card of the portal dialogs: the prompt, the requester, and the Deny and
//! Allow buttons.

use egui::{CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};
use gnoblin_portal_core::canvas::{
    ANIMATION_SECONDS, BUTTON_GAP, BUTTON_RADIUS, BUTTON_SIZE, CARD_RADIUS, Drawn, FOCUS_WIDTH, MARGIN, Setup,
    draw_centred, draw_text,
};
use gnoblin_portal_core::model::{Consent, ConsentFocus, ConsentTarget};
use gnoblin_types::Tick;

/// Draws a consent card: prompt, requester, and the Deny and Allow buttons.
pub fn draw_consent(ui: &Ui, consent: &Consent, setup: &Setup, now: Tick, drawn: &mut Drawn) {
    let theme = &setup.theme;
    let painter = ui.painter();
    let frame_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(setup.width as f32, setup.height as f32));
    let card = frame_rect.shrink(MARGIN);
    painter.rect_filled(card, CornerRadius::same(CARD_RADIUS), theme.card);

    let wrap = card.width() - 2.0 * MARGIN;
    let left = card.left() + MARGIN;
    let title_font = FontId::proportional(theme.font_size + 4.0);
    let body_font = FontId::proportional(theme.font_size);
    let title = draw_text(
        painter,
        drawn,
        consent.prompt(),
        title_font,
        theme.text,
        wrap,
        Pos2::new(left, card.top() + MARGIN),
    );
    draw_text(
        painter,
        drawn,
        &format!("Requested by {}", consent.app_id()),
        body_font.clone(),
        theme.muted,
        wrap,
        Pos2::new(left, title.bottom() + 8.0),
    );

    let row_top = card.bottom() - MARGIN - BUTTON_SIZE.y;
    let allow_left = card.right() - MARGIN - BUTTON_SIZE.x;
    let deny_rect = Rect::from_min_size(Pos2::new(allow_left - BUTTON_GAP - BUTTON_SIZE.x, row_top), BUTTON_SIZE);
    let allow_rect = Rect::from_min_size(Pos2::new(allow_left, row_top), BUTTON_SIZE);

    // The arm animation is keyed on the app id, so two dialogs never share it.
    let arm_time = if setup.reduced_motion { 0.0 } else { ANIMATION_SECONDS };
    let arm_progress = ui.ctx().animate_bool_with_time(
        ui.scope_id().with(("consent-arm", consent.app_id())),
        consent.armed(now),
        arm_time,
    );
    let allow_fill = theme.button_disarmed.lerp_to_gamma(theme.accent, arm_progress);

    let radius = CornerRadius::same(BUTTON_RADIUS);
    painter.rect_filled(deny_rect, radius, theme.button);
    painter.rect_filled(allow_rect, radius, allow_fill);
    draw_centred(painter, drawn, "Deny", deny_rect, body_font.clone(), theme.text);
    draw_centred(painter, drawn, "Allow", allow_rect, body_font, theme.text);

    let focused = match consent.focus() {
        ConsentFocus::Deny => deny_rect,
        ConsentFocus::Allow => allow_rect,
    };
    painter.rect_stroke(
        focused.expand(FOCUS_WIDTH),
        radius,
        Stroke::new(FOCUS_WIDTH, theme.accent),
        StrokeKind::Outside,
    );

    drawn.hits.push((ConsentTarget::Deny, deny_rect));
    drawn.hits.push((ConsentTarget::Allow, allow_rect));
}
