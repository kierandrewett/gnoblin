#include "config.h"

#include "gnoblin-debug-panel.hpp"

#include <clutter/clutter.h>
#include <imgui.h>
#include <unistd.h>
#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <functional>

namespace {

constexpr unsigned int kHistory = 240;
constexpr float kBackgroundAlpha = 0.20f;
constexpr float kDroppedFrameMs = 25.0f;

} // namespace

struct GnoblinDebugPanel {
    bool enabled;
    /* Rolling intervals between presented compositor frames, in milliseconds. */
    float intervals[kHistory];
    unsigned int interval_index;
    unsigned int interval_count;
    gint64 last_present_us;
    guint64 frames;
    guint64 dropped;
    gpointer tracked_view;
    guint tick_id;
    gulong presented_id;
    ClutterStage* stage;
    GnoblinImGuiOverlay* overlay;
    /* Process cost, sampled about once a second. */
    gint64 sample_us;
    guint64 sample_cpu_ticks;
    float cpu_percent;
    float rss_mb;

    GnoblinDebugPanel() = default;
    ~GnoblinDebugPanel() = default;
};

static gboolean debug_scroll_tick(gpointer user_data) {
    auto* panel = static_cast<GnoblinDebugPanel*>(user_data);
    if (panel->overlay)
        gnoblin_imgui_overlay_queue_redraw(panel->overlay);
    return G_SOURCE_CONTINUE;
}

/* Count real compositor presentations rather than the panel's own redraws. With several
 * outputs only the first one seen is measured, so intervals do not interleave. */
static void debug_frame_presented(ClutterStage* stage, gpointer view, gpointer info,
                                  gpointer user_data) {
    (void)stage;
    (void)info;
    auto* panel = static_cast<GnoblinDebugPanel*>(user_data);
    if (panel->tracked_view == nullptr)
        panel->tracked_view = view;
    if (view != panel->tracked_view)
        return;
    const gint64 now = g_get_monotonic_time();
    if (panel->last_present_us != 0) {
        const float interval_ms = static_cast<float>(now - panel->last_present_us) / 1000.0f;
        /* A long idle gap is a quiet stage, not a dropped frame. */
        if (interval_ms > 0.0f && interval_ms < 250.0f) {
            panel->intervals[panel->interval_index] = interval_ms;
            panel->interval_index = (panel->interval_index + 1) % kHistory;
            if (panel->interval_count < kHistory)
                panel->interval_count++;
            if (interval_ms > kDroppedFrameMs)
                panel->dropped++;
        }
    }
    panel->last_present_us = now;
    panel->frames++;
}

static void debug_notifications_stop(GnoblinDebugPanel* panel) {
    if (panel->tick_id) {
        g_source_remove(panel->tick_id);
        panel->tick_id = 0;
    }
    if (panel->stage && panel->presented_id) {
        g_signal_handler_disconnect(panel->stage, panel->presented_id);
        panel->presented_id = 0;
    }
    panel->stage = nullptr;
    panel->overlay = nullptr;
    panel->last_present_us = 0;
    panel->tracked_view = nullptr;
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
        panel->enabled = true;
        panel->overlay = overlay;
        if (!panel->tick_id)
            panel->tick_id = g_timeout_add(250, debug_scroll_tick, panel);
        ClutterActor* actor = gnoblin_imgui_overlay_get_actor(overlay);
        ClutterActor* stage = actor ? clutter_actor_get_stage(actor) : nullptr;
        if (stage && !panel->presented_id) {
            panel->stage = CLUTTER_STAGE(stage);
            panel->presented_id = g_signal_connect(panel->stage, "presented",
                                                   G_CALLBACK(debug_frame_presented), panel);
        }
        return;
    }
    debug_notifications_stop(panel);
    panel->enabled = wanted;
}

gboolean gnoblin_debug_panel_enabled(const GnoblinDebugPanel* panel) {
    return panel != nullptr && panel->enabled;
}

/* Resident memory and CPU use of the compositor process. */
static void debug_sample_process(GnoblinDebugPanel* panel) {
    const gint64 now = g_get_monotonic_time();
    if (panel->sample_us != 0 && now - panel->sample_us < G_USEC_PER_SEC)
        return;

    unsigned long resident_pages = 0;
    if (FILE* statm = std::fopen("/proc/self/statm", "r")) {
        unsigned long size = 0;
        if (std::fscanf(statm, "%lu %lu", &size, &resident_pages) == 2)
            panel->rss_mb = static_cast<float>(resident_pages) *
                            static_cast<float>(sysconf(_SC_PAGESIZE)) / (1024.0f * 1024.0f);
        std::fclose(statm);
    }
    if (FILE* stat = std::fopen("/proc/self/stat", "r")) {
        char line[1024] = {};
        if (std::fgets(line, sizeof line, stat)) {
            /* Fields 14 and 15 (utime, stime) follow the parenthesised command name. */
            const char* after = std::strrchr(line, ')');
            unsigned long utime = 0, stime = 0;
            if (after &&
                std::sscanf(after + 2, "%*c %*d %*d %*d %*d %*d %*u %*u %*u %*u %*u %lu %lu",
                            &utime, &stime) == 2) {
                const guint64 ticks = utime + stime;
                if (panel->sample_us != 0) {
                    const double seconds = static_cast<double>(now - panel->sample_us) / 1e6;
                    const double used =
                        static_cast<double>(ticks - panel->sample_cpu_ticks) / sysconf(_SC_CLK_TCK);
                    panel->cpu_percent = static_cast<float>(used / seconds * 100.0);
                }
                panel->sample_cpu_ticks = ticks;
            }
        }
        std::fclose(stat);
    }
    panel->sample_us = now;
}

/* The background is mostly transparent, so each line carries a one pixel shadow to stay
 * readable over any wallpaper. */
static void debug_line(const ImVec4& colour, const char* format, ...) {
    char buffer[160];
    va_list arguments;
    va_start(arguments, format);
    std::vsnprintf(buffer, sizeof buffer, format, arguments);
    va_end(arguments);
    const ImVec2 position = ImGui::GetCursorScreenPos();
    ImGui::GetWindowDrawList()->AddText(ImVec2(position.x + 1.0f, position.y + 1.0f),
                                        IM_COL32(0, 0, 0, 200), buffer);
    ImGui::TextColored(colour, "%s", buffer);
}

static ImVec4 debug_fps_colour(float fps) {
    if (fps >= 55.0f)
        return ImVec4(0.45f, 0.95f, 0.55f, 1.0f);
    if (fps >= 30.0f)
        return ImVec4(1.0f, 0.82f, 0.35f, 1.0f);
    return ImVec4(1.0f, 0.45f, 0.45f, 1.0f);
}

void gnoblin_debug_panel_draw(GnoblinImGuiOverlay* overlay, GnoblinDebugPanel* panel) {
    (void)overlay;
    if (panel == nullptr || !panel->enabled)
        return;
    debug_sample_process(panel);
    const ImVec4 dim = ImVec4(0.88f, 0.9f, 0.95f, 0.92f);

    /* Ordered newest last, so the graph scrolls left like a profiler. */
    float ordered[kHistory];
    const unsigned int count = panel->interval_count;
    for (unsigned int i = 0; i < count; i++)
        ordered[i] = panel->intervals[(panel->interval_index + kHistory - count + i) % kHistory];

    float mean = 0.0f, worst = 0.0f, last = 0.0f;
    for (unsigned int i = 0; i < count; i++) {
        mean += ordered[i];
        worst = std::max(worst, ordered[i]);
    }
    if (count > 0) {
        mean /= static_cast<float>(count);
        last = ordered[count - 1];
    }
    /* 1% low: the frame rate of the slowest one percent of recent frames. */
    float slowest_percent = 0.0f;
    if (count >= 20) {
        float sorted[kHistory];
        std::copy(ordered, ordered + count, sorted);
        std::sort(sorted, sorted + count, std::greater<float>());
        const unsigned int tail = std::max(1u, count / 100u);
        for (unsigned int i = 0; i < tail; i++)
            slowest_percent += sorted[i];
        slowest_percent /= static_cast<float>(tail);
    }
    const float fps = mean > 0.0f ? 1000.0f / mean : 0.0f;

    ImGui::SetNextWindowPos(ImVec2(12.0f, 44.0f), ImGuiCond_Always);
    ImGui::SetNextWindowBgAlpha(kBackgroundAlpha);
    const ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoInputs | ImGuiWindowFlags_NoNav |
        ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoFocusOnAppearing |
        ImGuiWindowFlags_NoBringToFrontOnFocus | ImGuiWindowFlags_AlwaysAutoResize;
    ImGui::PushStyleColor(ImGuiCol_Border, ImVec4(1.0f, 1.0f, 1.0f, 0.10f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(10.0f, 8.0f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 1.0f);
    if (!ImGui::Begin("gnoblin debug", nullptr, flags)) {
        ImGui::End();
        ImGui::PopStyleVar(2);
        ImGui::PopStyleColor();
        return;
    }

    if (count == 0) {
        debug_line(dim, "fps  --");
        debug_line(dim, "waiting for frames");
    } else {
        debug_line(debug_fps_colour(fps), "%5.1f fps", fps);
        debug_line(dim, "%5.1f ms", last);
    }

    const ImVec4 line = ImVec4(0.45f, 0.95f, 0.55f, 0.9f);
    ImGui::PushStyleColor(ImGuiCol_PlotLines, line);
    ImGui::PushStyleColor(ImGuiCol_FrameBg, ImVec4(0.0f, 0.0f, 0.0f, 0.25f));
    if (count > 0)
        ImGui::PlotLines("##frame-ms", ordered, static_cast<int>(count), 0, nullptr, 0.0f,
                         std::max(33.3f, worst), ImVec2(220.0f, 44.0f));
    ImGui::PopStyleColor(2);

    if (count > 0) {
        debug_line(dim, "avg %.1f   max %.1f", mean, worst);
        if (slowest_percent > 0.0f)
            debug_line(dim, "1%% low %.0f fps", 1000.0f / slowest_percent);
        debug_line(dim, "frames %llu   slow %llu", static_cast<unsigned long long>(panel->frames),
                   static_cast<unsigned long long>(panel->dropped));
    }
    debug_line(dim, "cpu %.0f%%   mem %.0f MB", panel->cpu_percent, panel->rss_mb);
    ImGui::End();
    ImGui::PopStyleVar(2);
    ImGui::PopStyleColor();
}
