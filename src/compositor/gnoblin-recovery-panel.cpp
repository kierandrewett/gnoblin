/* Compact recovery notice for the compositor's internal Dear ImGui overlay. */

#include "gnoblin-recovery-panel.hpp"

#include <imgui.h>

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

static bool state_requires_notice (const GnoblinRecoveryPanel *panel)
{
    return panel->fallback_source != GnoblinRecoveryFallbackSource::None ||
           panel->shell_missing || !panel->error_message.empty();
}

static void queue_panel_redraw (GnoblinImGuiOverlay *overlay)
{
    if (overlay)
        gnoblin_imgui_overlay_queue_redraw (overlay);
}

GnoblinRecoveryPanel *
gnoblin_recovery_panel_new ()
{
    return new GnoblinRecoveryPanel ();
}

void
gnoblin_recovery_panel_free (GnoblinRecoveryPanel *panel)
{
    delete panel;
}

bool
gnoblin_recovery_panel_set_state (GnoblinRecoveryPanel          *panel,
                                  const char                    *error_message,
                                  GnoblinRecoveryFallbackSource  fallback_source,
                                  bool                           shell_missing)
{
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

void
gnoblin_recovery_panel_set_actions (GnoblinRecoveryPanel             *panel,
                                    const GnoblinRecoveryPanelActions *actions)
{
    if (panel)
        panel->actions = actions ? *actions : GnoblinRecoveryPanelActions {};
}

bool
gnoblin_recovery_panel_should_draw (const GnoblinRecoveryPanel *panel)
{
    return panel && state_requires_notice (panel) && !panel->dismissed;
}

void
gnoblin_recovery_panel_dismiss (GnoblinRecoveryPanel *panel)
{
    if (panel)
        panel->dismissed = true;
}

bool
gnoblin_recovery_panel_contains (const GnoblinRecoveryPanel *panel, float x, float y)
{
    return gnoblin_recovery_panel_should_draw (panel) &&
           x >= panel->position.x && y >= panel->position.y &&
           x < panel->position.x + panel->size.x && y < panel->position.y + panel->size.y;
}

static void
draw_configuration_notice (const GnoblinRecoveryPanel *panel)
{
    if (panel->fallback_source == GnoblinRecoveryFallbackSource::None)
        return;

    /* The panel header already says "Configuration recovery active". */
    if (panel->fallback_source == GnoblinRecoveryFallbackSource::LastKnownGood)
        ImGui::TextWrapped ("Gnoblin started with your last working configuration.");
    else
        ImGui::TextWrapped ("Gnoblin started with its built-in default configuration.");
    ImGui::TextDisabled ("Fix your configuration, then run: gnoblinctl reload");
}

static void
draw_shell_notice (const GnoblinRecoveryPanel *panel)
{
    if (!panel->shell_missing)
        return;

    if (panel->fallback_source != GnoblinRecoveryFallbackSource::None)
        ImGui::Separator ();
    ImGui::TextUnformatted ("Shell unavailable");
    ImGui::Indent ();
    ImGui::TextWrapped ("Gnoblin is running, but no shell layer is active.");
    ImGui::TextDisabled ("Start your shell or correct its configuration.");
    ImGui::Unindent ();
}

static void
draw_error_notice (const GnoblinRecoveryPanel *panel)
{
    if (panel->error_message.empty ())
        return;

    if (panel->fallback_source != GnoblinRecoveryFallbackSource::None || panel->shell_missing)
        ImGui::Separator ();
    ImGui::TextUnformatted ("Details");
    ImGui::Indent ();
    ImGui::PushTextWrapPos (ImGui::GetCursorPosX () + 440.0f);
    ImGui::TextUnformatted (panel->error_message.c_str ());
    ImGui::PopTextWrapPos ();
    ImGui::Unindent ();
}

static bool
draw_action_button (const char                     *label,
                    GnoblinRecoveryPanelActionFunc  action,
                    void                            *user_data)
{
    if (!action)
        ImGui::BeginDisabled ();

    const bool pressed = ImGui::Button (label);

    if (!action) {
        ImGui::EndDisabled ();
        if (ImGui::IsItemHovered (ImGuiHoveredFlags_AllowWhenDisabled))
            ImGui::SetTooltip ("Unavailable in this session");
        return false;
    }

    if (pressed)
        action (user_data);
    return pressed;
}

void
gnoblin_recovery_panel_draw (GnoblinImGuiOverlay *overlay,
                             GnoblinRecoveryPanel *panel)
{
    if (!gnoblin_recovery_panel_should_draw (panel))
        return;

    constexpr float margin = 14.0f;
    constexpr float width = 500.0f;
    const ImGuiViewport *viewport = ImGui::GetMainViewport ();
    ImGui::SetNextWindowPos (ImVec2 (viewport->WorkPos.x + margin,
                                     viewport->WorkPos.y + margin),
                             ImGuiCond_Always);
    ImGui::SetNextWindowSizeConstraints (ImVec2 (320.0f, 0.0f), ImVec2 (width, FLT_MAX));
    ImGui::SetNextWindowBgAlpha (0.94f);

    constexpr ImGuiWindowFlags flags = ImGuiWindowFlags_NoCollapse |
                                       ImGuiWindowFlags_NoResize |
                                       ImGuiWindowFlags_NoMove |
                                       ImGuiWindowFlags_NoSavedSettings |
                                       ImGuiWindowFlags_AlwaysAutoResize;
    bool open = true;
    if (ImGui::Begin ("Gnoblin recovery", &open, flags)) {
        if (panel->fallback_source != GnoblinRecoveryFallbackSource::None)
            ImGui::TextUnformatted ("Configuration recovery active");
        else if (!panel->error_message.empty ())
            ImGui::TextUnformatted ("Gnoblin error");
        else
            ImGui::TextUnformatted ("Gnoblin needs attention");
        ImGui::Separator ();
        draw_configuration_notice (panel);
        draw_shell_notice (panel);
        draw_error_notice (panel);

        ImGui::Spacing ();
        const bool action_ran =
            draw_action_button ("Open config", panel->actions.open_config_folder,
                                panel->actions.user_data);
        ImGui::SameLine ();
        const bool terminal_opened =
            draw_action_button ("Terminal", panel->actions.open_terminal,
                                panel->actions.user_data);
        ImGui::SameLine ();
        const bool reload_requested =
            draw_action_button ("Reload", panel->actions.reload_config,
                                panel->actions.user_data);
        if (action_ran || terminal_opened || reload_requested)
            queue_panel_redraw (overlay);

        ImGui::SameLine ();
        if (ImGui::Button ("Dismiss")) {
            gnoblin_recovery_panel_dismiss (panel);
            queue_panel_redraw (overlay);
        }
        if (!panel->actions.open_config_folder && !panel->actions.open_terminal &&
            !panel->actions.reload_config)
            ImGui::TextDisabled ("Recovery actions are unavailable in this session.");
    }
    panel->position = ImGui::GetWindowPos ();
    panel->size = ImGui::GetWindowSize ();
    ImGui::End ();
    if (!open) {
        gnoblin_recovery_panel_dismiss (panel);
        queue_panel_redraw (overlay);
    }
}
