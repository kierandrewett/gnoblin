#include "config.h"

#include "gnoblin-debug-panel.hpp"

#include <imgui.h>
#include <algorithm>

struct GnoblinDebugPanel {
    bool enabled;
    /* Rolling paint intervals in milliseconds; newest at the ring cursor. */
    float intervals[180];
    unsigned int interval_index;
    unsigned int interval_count;
    gint64 last_draw_us;
    guint tick_id;
    GnoblinImGuiOverlay* overlay;

    GnoblinDebugPanel() = default;
    ~GnoblinDebugPanel() = default;
};

static gboolean debug_scroll_tick(gpointer user_data) {
    auto* panel = static_cast<GnoblinDebugPanel*>(user_data);
    if (panel->overlay)
        gnoblin_imgui_overlay_queue_redraw(panel->overlay);
    return G_SOURCE_CONTINUE;
}

static void debug_notifications_stop(GnoblinDebugPanel* panel) {
    if (panel->tick_id) {
        g_source_remove(panel->tick_id);
        panel->tick_id = 0;
    }
    panel->overlay = nullptr;
    panel->last_draw_us = 0;
}

GnoblinDebugPanel* gnoblin_debug_panel_new() {
    return g_new0(GnoblinDebugPanel, 1);
}

void gnoblin_debug_panel_free(GnoblinDebugPanel* panel) {
    if (panel == nullptr)
        return;
    debug_notifications_stop(panel);
    g_free(panel);
}

void gnoblin_debug_panel_set_enabled(GnoblinDebugPanel* panel, GnoblinImGuiOverlay* overlay,
                                     gboolean enabled) {
    if (panel == nullptr)
        return;
    const bool wanted = enabled == TRUE;
    if (wanted && overlay != nullptr) {
        panel->overlay = overlay;
        if (!panel->tick_id)
            panel->tick_id = g_timeout_add(250, debug_scroll_tick, panel);
        return;
    }
    debug_notifications_stop(panel);
    panel->enabled = wanted;
}

gboolean gnoblin_debug_panel_enabled(const GnoblinDebugPanel* panel) {
    return panel != nullptr && panel->enabled;
}

void gnoblin_debug_panel_draw(GnoblinImGuiOverlay* overlay, GnoblinDebugPanel* panel) {
    if (panel == nullptr || !panel->enabled)
        return;
    const gint64 now = g_get_monotonic_time();
    if (panel->last_draw_us != 0) {
        const float interval_ms = static_cast<float>(now - panel->last_draw_us) / 1000.0f;
        if (interval_ms > 0.0f && interval_ms < 5000.0f) {
            panel->intervals[panel->interval_index] = interval_ms;
            panel->interval_index = (panel->interval_index + 1) % G_N_ELEMENTS(panel->intervals);
            if (panel->interval_count < G_N_ELEMENTS(panel->intervals))
                panel->interval_count++;
        }
    }
    panel->last_draw_us = now;

    ImGui::SetNextWindowPos(ImVec2(10.0f, 10.0f), ImGuiCond_FirstUseEver);
    ImGui::SetNextWindowSize(ImVec2(320.0f, 190.0f), ImGuiCond_FirstUseEver);
    const ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoBringToFrontOnFocus;
    if (!ImGui::Begin("debug", nullptr, flags)) {
        ImGui::End();
        return;
    }
    unsigned int index = panel->interval_index;
    float minimum = 0.0f;
    float maximum = 0.0f;
    float mean = 0.0f;
    for (unsigned int i = 0; i < panel->interval_count; i++) {
        const float value = panel->intervals[(index + i) % G_N_ELEMENTS(panel->intervals)];
        minimum = i == 0 || value < minimum ? value : minimum;
        maximum = value > maximum ? value : maximum;
        mean += value;
    }
    if (panel->interval_count > 0)
        mean /= static_cast<float>(panel->interval_count);
    ImGui::Text("fps %6.1f%s", mean > 0.0f ? 1000.0f / mean : 0.0f,
                panel->interval_count < 16 ? "  (warming up)" : "");
    if (panel->interval_count > 0)
        ImGui::PlotLines("##frame-ms", &panel->intervals[index], panel->interval_count, 0,
                         "paint interval ms", minimum, std::max(16.0f, maximum), ImVec2(-1, 72));
    ImGui::Text("paint %.1f ms  max %.1f", mean, maximum);
    ImGui::Separator();
    ImGui::TextDisabled("read-only; input falls through");
    ImGui::End();
}
