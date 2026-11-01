/* Recovery notice and transient notices for the compositor's internal ImGui overlay. */

#include "gnoblin-recovery-panel.hpp"

#include <imgui.h>

#include <algorithm>
#include <cfloat>
#include <cstdint>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

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

float gnoblin_recovery_panel_bottom_over(const GnoblinRecoveryPanel* panel, float left,
                                         float right) {
    if (!gnoblin_recovery_panel_should_draw(panel) || panel->position.x >= right ||
        panel->position.x + panel->size.x <= left)
        return 0.0f;
    return panel->position.y + panel->size.y;
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

/* Transient notices. They share the palette above, which is why they live in
 * this file. */

namespace {
constexpr std::int64_t kReloadedLifetimeUs = 2 * 1000 * 1000;
constexpr std::int64_t kCommandFailedLifetimeUs = 8 * 1000 * 1000;
constexpr std::size_t kMaxNotices = 4;
constexpr std::size_t kMaxDetailBytes = 160;
constexpr float kNoticeWidth = 380.0f;
constexpr float kNoticeMargin = 14.0f;
constexpr float kNoticeGap = 6.0f;
const ImVec2 kNoticePadding(14.0f, 10.0f);
const ImVec4 kSuccess = rgb(0x8ce99a);

struct Notice {
    std::uint64_t id = 0;
    GnoblinNoticeKind kind = GnoblinNoticeKind::CommandFailed;
    std::string title;
    std::string detail;
    /* Monotonic microseconds; 0 stays until dismissed. */
    std::int64_t expires_us = 0;
    bool dismissed = false;
    ImVec2 position = {};
    ImVec2 size = {};
};

/* One line of bounded length. Lua errors and spawn failures can carry a
 * traceback or a full path; the first line is the message. */
std::string notice_detail(const char* message) {
    std::string text = message ? message : "";
    text.erase(std::min(text.find_first_of("\r\n"), text.size()));
    text.erase(0, text.find_first_not_of(" \t"));
    text.erase(text.find_last_not_of(" \t") + 1);
    if (text.size() > kMaxDetailBytes) {
        std::size_t cut = kMaxDetailBytes - 3;
        while (cut > 0 && (static_cast<unsigned char>(text[cut]) & 0xc0) == 0x80)
            cut--;
        text.erase(cut);
        text += "...";
    }
    return text;
}

const ImVec4& notice_tone(GnoblinNoticeKind kind) {
    switch (kind) {
    case GnoblinNoticeKind::ConfigReloaded:
        return kSuccess;
    case GnoblinNoticeKind::ConfigFailed:
        return kDanger;
    case GnoblinNoticeKind::CommandFailed:
        break;
    }
    return kWarning;
}
} // namespace

struct GnoblinNoticeStack {
    std::vector<Notice> notices;
    std::uint64_t next_id = 1;
};

GnoblinNoticeStack* gnoblin_notice_stack_new() {
    return new GnoblinNoticeStack();
}

void gnoblin_notice_stack_free(GnoblinNoticeStack* stack) {
    delete stack;
}

bool gnoblin_notice_stack_post(GnoblinNoticeStack* stack, GnoblinNoticeKind kind,
                               const char* message, std::int64_t now_us) {
    if (!stack)
        return false;

    std::string title;
    std::string detail = notice_detail(message);
    std::int64_t expires_us = 0;
    switch (kind) {
    case GnoblinNoticeKind::ConfigReloaded:
        detail = "Config reloaded";
        expires_us = now_us + kReloadedLifetimeUs;
        break;
    case GnoblinNoticeKind::ConfigFailed:
        title = "Config reload failed";
        if (detail.empty())
            detail = "The configuration was not applied.";
        break;
    case GnoblinNoticeKind::CommandFailed:
        if (detail.empty())
            detail = "A command could not start.";
        expires_us = now_us + kCommandFailedLifetimeUs;
        break;
    }

    auto& list = stack->notices;
    auto same = [&](const Notice& notice) {
        return notice.kind == kind && notice.title == title && notice.detail == detail;
    };
    bool changed = false;

    /* The latest reload outcome replaces the earlier one, so a success clears
     * a stale failure and the other way round. */
    if (kind != GnoblinNoticeKind::CommandFailed) {
        const auto stale = std::remove_if(list.begin(), list.end(), [&](const Notice& notice) {
            return notice.kind != GnoblinNoticeKind::CommandFailed && !same(notice);
        });
        changed = stale != list.end();
        list.erase(stale, list.end());
    }

    /* A repeat of what is already showing restarts its time and nothing more. */
    const auto existing = std::find_if(list.begin(), list.end(), same);
    if (existing != list.end()) {
        existing->expires_us = expires_us;
        return changed;
    }

    Notice notice;
    notice.id = stack->next_id++;
    notice.kind = kind;
    notice.title = std::move(title);
    notice.detail = std::move(detail);
    notice.expires_us = expires_us;
    list.push_back(std::move(notice));
    if (list.size() > kMaxNotices)
        list.erase(list.begin());
    return true;
}

bool gnoblin_notice_stack_expire(GnoblinNoticeStack* stack, std::int64_t now_us) {
    if (!stack)
        return false;
    auto& list = stack->notices;
    const auto expired = std::remove_if(list.begin(), list.end(), [&](const Notice& notice) {
        return notice.expires_us != 0 && notice.expires_us <= now_us;
    });
    const bool changed = expired != list.end();
    list.erase(expired, list.end());
    return changed;
}

std::int64_t gnoblin_notice_stack_next_expiry(const GnoblinNoticeStack* stack) {
    std::int64_t next = 0;
    if (!stack)
        return next;
    for (const Notice& notice : stack->notices)
        if (notice.expires_us != 0 && (next == 0 || notice.expires_us < next))
            next = notice.expires_us;
    return next;
}

bool gnoblin_notice_stack_should_draw(const GnoblinNoticeStack* stack) {
    return stack && !stack->notices.empty();
}

std::size_t gnoblin_notice_stack_count(const GnoblinNoticeStack* stack) {
    return stack ? stack->notices.size() : 0;
}

const char* gnoblin_notice_stack_detail(const GnoblinNoticeStack* stack, std::size_t index) {
    return stack && index < stack->notices.size() ? stack->notices[index].detail.c_str() : "";
}

bool gnoblin_notice_stack_contains(const GnoblinNoticeStack* stack, float x, float y) {
    if (!stack)
        return false;
    for (const Notice& notice : stack->notices)
        if (x >= notice.position.x && y >= notice.position.y &&
            x < notice.position.x + notice.size.x && y < notice.position.y + notice.size.y)
            return true;
    return false;
}

void gnoblin_notice_stack_draw(GnoblinImGuiOverlay* overlay, GnoblinNoticeStack* stack,
                               const GnoblinRecoveryPanel* recovery) {
    if (!gnoblin_notice_stack_should_draw(stack))
        return;

    const ImGuiViewport* viewport = ImGui::GetMainViewport();
    const float width =
        std::max(1.0f, std::min(kNoticeWidth, viewport->WorkSize.x - kNoticeMargin * 2));
    const float wrap = std::max(1.0f, width - kNoticePadding.x * 2);
    const float x = viewport->WorkPos.x + (viewport->WorkSize.x - width) / 2;
    float y = std::max(viewport->WorkPos.y + kNoticeMargin,
                       gnoblin_recovery_panel_bottom_over(recovery, x, x + width) + kNoticeGap);
    const float line_gap = ImGui::GetStyle().ItemSpacing.y;

    constexpr ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoResize |
        ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoScrollbar |
        ImGuiWindowFlags_NoScrollWithMouse | ImGuiWindowFlags_NoFocusOnAppearing |
        ImGuiWindowFlags_NoNav;

    bool dismissed = false;
    for (Notice& notice : stack->notices) {
        const ImVec4& tone = notice_tone(notice.kind);
        float height = kNoticePadding.y * 2 +
                       ImGui::CalcTextSize(notice.detail.c_str(), nullptr, false, wrap).y;
        if (!notice.title.empty())
            height += ImGui::CalcTextSize(notice.title.c_str(), nullptr, false, wrap).y + line_gap;

        ImGui::SetNextWindowPos(ImVec2(x, y), ImGuiCond_Always);
        ImGui::SetNextWindowSize(ImVec2(width, height), ImGuiCond_Always);
        ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, kNoticePadding);
        ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 1.0f);
        ImGui::PushStyleColor(ImGuiCol_WindowBg, rgb(0x1c1c1c, 0.97f));
        ImGui::PushStyleColor(ImGuiCol_Border, ImVec4(tone.x, tone.y, tone.z, 0.6f));
        char name[48];
        std::snprintf(name, sizeof name, "Gnoblin notice %llu",
                      static_cast<unsigned long long>(notice.id));
        if (ImGui::Begin(name, nullptr, flags)) {
            /* The whole notice is one click target that dismisses it. */
            ImGui::SetCursorPos(ImVec2(0.0f, 0.0f));
            if (ImGui::InvisibleButton("##dismiss", ImVec2(width, height)))
                notice.dismissed = true;
            ImGui::SetCursorPos(kNoticePadding);
            ImGui::PushTextWrapPos(kNoticePadding.x + wrap);
            if (!notice.title.empty())
                ImGui::TextColored(tone, "%s", notice.title.c_str());
            ImGui::TextUnformatted(notice.detail.c_str());
            ImGui::PopTextWrapPos();
        }
        notice.position = ImGui::GetWindowPos();
        notice.size = ImGui::GetWindowSize();
        ImGui::End();
        ImGui::PopStyleColor(2);
        ImGui::PopStyleVar(2);

        dismissed = dismissed || notice.dismissed;
        y += notice.size.y + kNoticeGap;
    }

    if (dismissed) {
        auto& list = stack->notices;
        list.erase(std::remove_if(list.begin(), list.end(),
                                  [](const Notice& notice) { return notice.dismissed; }),
                   list.end());
        queue_panel_redraw(overlay);
    }
}
