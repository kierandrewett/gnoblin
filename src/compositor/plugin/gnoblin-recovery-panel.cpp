/* Compact recovery notice for the compositor's internal Dear ImGui overlay. */

#include "gnoblin-recovery-panel.hpp"

#include <imgui.h>

#include <algorithm>
#include <cfloat>
#include <string>

struct GnoblinRecoveryPanel {
    std::string error_message;
    GnoblinRecoveryFallbackSource fallback_source = GnoblinRecoveryFallbackSource::None;
    GnoblinRecoveryPanelActions actions = {};
    bool shell_missing = false;
    bool dismissed = false;
    ImVec2 position = {};
    ImVec2 size = {};
};

static bool state_requires_notice(const GnoblinRecoveryPanel* panel) {
    return panel->fallback_source != GnoblinRecoveryFallbackSource::None || panel->shell_missing ||
           !panel->error_message.empty();
}

static void queue_panel_redraw(GnoblinImGuiOverlay* overlay) {
    if (overlay)
        gnoblin_imgui_overlay_queue_redraw(overlay);
}

GnoblinRecoveryPanel* gnoblin_recovery_panel_new() {
    return new GnoblinRecoveryPanel();
}

void gnoblin_recovery_panel_free(GnoblinRecoveryPanel* panel) {
    delete panel;
}

bool gnoblin_recovery_panel_set_state(GnoblinRecoveryPanel* panel, const char* error_message,
                                      GnoblinRecoveryFallbackSource fallback_source,
                                      bool shell_missing) {
    if (!panel)
        return false;

    const std::string next_error = error_message ? error_message : "";
    const bool changed = panel->error_message != next_error ||
                         panel->fallback_source != fallback_source ||
                         panel->shell_missing != shell_missing;

    panel->error_message = next_error;
    panel->fallback_source = fallback_source;
    panel->shell_missing = shell_missing;

    /* Steady-state polling must not re-open a notice the user dismissed. */
    if (changed)
        panel->dismissed = false;
    return changed;
}

void gnoblin_recovery_panel_set_actions(GnoblinRecoveryPanel* panel,
                                        const GnoblinRecoveryPanelActions* actions) {
    if (panel)
        panel->actions = actions ? *actions : GnoblinRecoveryPanelActions{};
}

bool gnoblin_recovery_panel_should_draw(const GnoblinRecoveryPanel* panel) {
    return panel && state_requires_notice(panel) && !panel->dismissed;
}

void gnoblin_recovery_panel_dismiss(GnoblinRecoveryPanel* panel) {
    if (panel)
        panel->dismissed = true;
}

bool gnoblin_recovery_panel_contains(const GnoblinRecoveryPanel* panel, float x, float y) {
    return gnoblin_recovery_panel_should_draw(panel) && x >= panel->position.x &&
           y >= panel->position.y && x < panel->position.x + panel->size.x &&
           y < panel->position.y + panel->size.y;
}

namespace {
constexpr float kContentWidth = 440.0f;
constexpr float kDetailsMaxHeight = 150.0f;

ImVec4 rgb(unsigned hex, float alpha = 1.0f) {
    return ImVec4(((hex >> 16) & 0xff) / 255.0f, ((hex >> 8) & 0xff) / 255.0f,
                  (hex & 0xff) / 255.0f, alpha);
}

/* Neutral grey with one accent and two severity colours, so the notice reads as
 * a warning without borrowing any shell's palette. */
const ImVec4 kWarning = rgb(0xe5a50a);
const ImVec4 kDanger = rgb(0xff8080);
const ImVec4 kQuiet = rgb(0x9a9a9a);

void paragraph(const char* text, const ImVec4* colour = nullptr) {
    if (colour != nullptr)
        ImGui::PushStyleColor(ImGuiCol_Text, *colour);
    ImGui::PushTextWrapPos(ImGui::GetCursorPosX() + kContentWidth);
    ImGui::TextUnformatted(text);
    ImGui::PopTextWrapPos();
    if (colour != nullptr)
        ImGui::PopStyleColor();
}
} // namespace

static void draw_configuration_notice(const GnoblinRecoveryPanel* panel) {
    if (panel->fallback_source == GnoblinRecoveryFallbackSource::None)
        return;

    paragraph(panel->fallback_source == GnoblinRecoveryFallbackSource::LastKnownGood
                  ? "Running your last working configuration."
                  : "Running the built-in default configuration.");
}

static void draw_shell_notice(const GnoblinRecoveryPanel* panel) {
    if (!panel->shell_missing)
        return;

    if (panel->fallback_source != GnoblinRecoveryFallbackSource::None)
        ImGui::Spacing();
    ImGui::TextColored(kWarning, "No shell layer is active.");
    paragraph("Start your shell or correct its configuration.", &kQuiet);
}

/* The supervisor's message is often a Lua traceback. The first line is the
 * error itself, so it is coloured; the rest is context. */
static void draw_error_notice(const GnoblinRecoveryPanel* panel) {
    if (panel->error_message.empty())
        return;

    if (panel->fallback_source != GnoblinRecoveryFallbackSource::None || panel->shell_missing)
        ImGui::Spacing();

    const std::string& message = panel->error_message;
    const std::string::size_type split = message.find('\n');
    const std::string first = message.substr(0, split);
    const std::string rest = split == std::string::npos ? "" : message.substr(split + 1);

    const ImVec2 padding(10.0f, 8.0f);
    const float wrap = kContentWidth - padding.x * 2;
    float text_height = ImGui::CalcTextSize(first.c_str(), nullptr, false, wrap).y;
    if (!rest.empty())
        text_height += ImGui::GetStyle().ItemSpacing.y +
                       ImGui::CalcTextSize(rest.c_str(), nullptr, false, wrap).y;
    const float height = std::min(text_height + padding.y * 2, kDetailsMaxHeight);

    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, padding);
    ImGui::PushStyleColor(ImGuiCol_ChildBg, rgb(0x161616));
    ImGui::PushStyleColor(ImGuiCol_Border, ImVec4(kDanger.x, kDanger.y, kDanger.z, 0.35f));
    if (ImGui::BeginChild("##details", ImVec2(kContentWidth, height), ImGuiChildFlags_Borders)) {
        ImGui::PushStyleColor(ImGuiCol_Text, kDanger);
        ImGui::TextWrapped("%s", first.c_str());
        ImGui::PopStyleColor();
        if (!rest.empty()) {
            ImGui::PushStyleColor(ImGuiCol_Text, kQuiet);
            ImGui::TextWrapped("%s", rest.c_str());
            ImGui::PopStyleColor();
        }
    }
    ImGui::EndChild();
    ImGui::PopStyleColor(2);
    ImGui::PopStyleVar();
}

static bool draw_action_button(const char* label, GnoblinRecoveryPanelActionFunc action,
                               void* user_data) {
    if (!action)
        ImGui::BeginDisabled();

    const bool pressed = ImGui::Button(label);

    if (!action) {
        ImGui::EndDisabled();
        if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled))
            ImGui::SetTooltip("Unavailable in this session");
        return false;
    }

    if (pressed)
        action(user_data);
    return pressed;
}

void gnoblin_recovery_panel_draw(GnoblinImGuiOverlay* overlay, GnoblinRecoveryPanel* panel) {
    if (!gnoblin_recovery_panel_should_draw(panel))
        return;

    constexpr float margin = 14.0f;
    const ImGuiStyle& style = ImGui::GetStyle();
    const ImGuiViewport* viewport = ImGui::GetMainViewport();
    ImGui::SetNextWindowPos(ImVec2(viewport->WorkPos.x + margin, viewport->WorkPos.y + margin),
                            ImGuiCond_Always);
    const float window_width = kContentWidth + 32.0f;
    ImGui::SetNextWindowSizeConstraints(ImVec2(window_width, 0.0f), ImVec2(window_width, FLT_MAX));

    const bool failed = panel->fallback_source == GnoblinRecoveryFallbackSource::None &&
                        !panel->error_message.empty();
    const ImVec4 tone = failed ? kDanger : kWarning;

    constexpr ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoResize |
        ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings |
        ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoScrollbar;
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(16.0f, 12.0f));
    const bool open = ImGui::Begin("Gnoblin recovery", nullptr, flags);
    ImGui::PopStyleVar();
    if (open) {
        const float x0 = ImGui::GetCursorPosX();

        if (panel->fallback_source != GnoblinRecoveryFallbackSource::None)
            ImGui::TextColored(tone, "Configuration failed to load");
        else if (!panel->error_message.empty())
            ImGui::TextColored(tone, "Gnoblin error");
        else
            ImGui::TextColored(tone, "Gnoblin needs attention");
        draw_configuration_notice(panel);
        draw_shell_notice(panel);
        draw_error_notice(panel);

        ImGui::Dummy(ImVec2(0.0f, 2.0f));
        const bool action_ran = draw_action_button("Open config", panel->actions.open_config_folder,
                                                   panel->actions.user_data);
        ImGui::SameLine();
        const bool terminal_opened =
            draw_action_button("Terminal", panel->actions.open_terminal, panel->actions.user_data);
        ImGui::SameLine();
        const bool reload_requested =
            draw_action_button("Reload", panel->actions.reload_config, panel->actions.user_data);
        if (action_ran || terminal_opened || reload_requested)
            queue_panel_redraw(overlay);

        const float dismiss_width = ImGui::CalcTextSize("Dismiss").x + style.FramePadding.x * 2;
        ImGui::SameLine(x0 + kContentWidth - dismiss_width);
        if (ImGui::Button("Dismiss")) {
            gnoblin_recovery_panel_dismiss(panel);
            queue_panel_redraw(overlay);
        }
        if (!panel->actions.open_config_folder && !panel->actions.open_terminal &&
            !panel->actions.reload_config)
            ImGui::TextColored(kQuiet, "Recovery actions are unavailable in this session.");
    }
    panel->position = ImGui::GetWindowPos();
    panel->size = ImGui::GetWindowSize();
    ImGui::End();
}
