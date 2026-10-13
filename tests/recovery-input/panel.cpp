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
