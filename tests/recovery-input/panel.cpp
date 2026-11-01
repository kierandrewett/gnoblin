/* Real ImGui input queue, production UI event handler and recovery panel.
 * Rendering and console transport are absent; neither decides recovery clicks. */
#include "events.h"
#include "gnoblin-recovery-panel.hpp"
#include <imgui.h>
#include <imgui_internal.h>
#include <memory>

struct _GnoblinImGuiOverlay {
    ImGuiContext* context;
};
struct Console {
    bool visible() const {
        return false;
    }
    bool handle_key(guint, ClutterModifierType, gboolean, bool = false) {
        return false;
    }
    void add_text(const char*) {}
};
struct GnoblinPluginUi {
    GnoblinImGuiOverlay* overlay;
    bool session_locked = false;
    guint recovery_buttons = 0;
    std::unique_ptr<GnoblinRecoveryPanel, decltype(&gnoblin_recovery_panel_free)> recovery;
    std::unique_ptr<GnoblinNoticeStack, decltype(&gnoblin_notice_stack_free)> notices;
    std::unique_ptr<Console> console;
};
static GnoblinPluginUi* ui;

void gnoblin_imgui_overlay_queue_redraw(GnoblinImGuiOverlay*) {}
void gnoblin_imgui_overlay_add_scroll(GnoblinImGuiOverlay*, float, float) {}
#include "ui-input.inc"

static void no_action(void*) {}
void panel_new(void) {
    auto* overlay = new GnoblinImGuiOverlay{ImGui::CreateContext()};
    ImGuiIO& io = ImGui::GetIO();
    io.IniFilename = nullptr;
    io.DisplaySize = ImVec2(1024, 768);
    io.DeltaTime = 1.0f / 60;
    unsigned char* pixels;
    int width, height;
    io.Fonts->GetTexDataAsRGBA32(&pixels, &width, &height);
    ui = new GnoblinPluginUi{overlay,
                             false,
                             0,
                             {gnoblin_recovery_panel_new(), gnoblin_recovery_panel_free},
                             {gnoblin_notice_stack_new(), gnoblin_notice_stack_free},
                             std::make_unique<Console>()};
    gnoblin_recovery_panel_set_state(ui->recovery.get(), "bingux exited 255",
                                     GnoblinRecoveryFallbackSource::None, false);
    const GnoblinRecoveryPanelActions actions = {no_action, no_action, no_action, nullptr};
    gnoblin_recovery_panel_set_actions(ui->recovery.get(), &actions);
    panel_frame();
    panel_frame();
}
void panel_free(void) {
    ImGui::DestroyContext(ui->overlay->context);
    delete ui->overlay;
    delete ui;
    ui = nullptr;
}
void panel_frame(void) {
    ImGui::NewFrame();
    gnoblin_recovery_panel_draw(ui->overlay, ui->recovery.get());
    gnoblin_notice_stack_draw(ui->overlay, ui->notices.get(), ui->recovery.get());
    ImGui::Render();
}
void panel_dismiss_coords(float* x, float* y) {
    const ImGuiWindow* window = ImGui::FindWindowByName("Gnoblin recovery");
    const ImGuiStyle& style = ImGui::GetStyle();
    // Dismiss is the final item on its line. ImGui retains its end position.
    const float width = ImGui::CalcTextSize("Dismiss").x + style.FramePadding.x * 2;
    const float height = ImGui::GetFontSize() + style.FramePadding.y * 2;
    *x = window->DC.CursorPosPrevLine.x - style.ItemSpacing.x - width / 2;
    *y = window->DC.CursorPosPrevLine.y + height / 2;
}
gboolean panel_visible(void) {
    return gnoblin_recovery_panel_should_draw(ui->recovery.get());
}
gboolean panel_handle(const ClutterEvent* event) {
    // Baseline signature is mutable; the production handler only reads events.
    return gnoblin_plugin_ui_handle_event(ui, const_cast<ClutterEvent*>(event));
}

/* Transient notices share the fixture: same ImGui context, same production
 * event handler. The recovery notice is hidden so it cannot overlap them. */
void notice_reset(void) {
    gnoblin_recovery_panel_dismiss(ui->recovery.get());
}
void notice_post(int kind, const char* message, gint64 now_us) {
    gnoblin_notice_stack_post(ui->notices.get(), static_cast<GnoblinNoticeKind>(kind), message,
                              now_us);
}
gboolean notice_expire(gint64 now_us) {
    return gnoblin_notice_stack_expire(ui->notices.get(), now_us);
}
gsize notice_count(void) {
    return gnoblin_notice_stack_count(ui->notices.get());
}
gint64 notice_next_expiry(void) {
    return gnoblin_notice_stack_next_expiry(ui->notices.get());
}
const char* notice_detail(gsize index) {
    return gnoblin_notice_stack_detail(ui->notices.get(), index);
}
/* A point inside the first painted notice, found by probing the painted bounds. */
gboolean notice_point(float* x, float* y) {
    const ImVec2 display = ImGui::GetIO().DisplaySize;
    for (float probe = 0.0f; probe < display.y; probe += 1.0f) {
        if (gnoblin_notice_stack_contains(ui->notices.get(), display.x / 2, probe)) {
            *x = display.x / 2;
            *y = probe + 8.0f; /* clear of the border */
            return TRUE;
        }
    }
    return FALSE;
}
/* True when notices are painted and no pixel belongs to both them and the recovery notice. */
gboolean notice_clear_of_recovery(void) {
    const ImVec2 display = ImGui::GetIO().DisplaySize;
    const float x = 400.0f;
    bool seen = false;
    for (float y = 0.0f; y < display.y; y += 1.0f) {
        const bool notice = gnoblin_notice_stack_contains(ui->notices.get(), x, y);
        if (notice && gnoblin_recovery_panel_contains(ui->recovery.get(), x, y))
            return FALSE;
        seen = seen || notice;
    }
    return seen;
}
