/*
 * Recovery notices and transient notices drawn in Gnoblin's compositor-owned
 * ImGui overlay.
 *
 * This is presentation only. The session supervisor and native control decide
 * when a configuration fallback, shell failure, reload result or command
 * failure has occurred.
 */

#pragma once

#include <cstddef>
#include <cstdint>

#include "gnoblin-imgui-overlay.hpp"

enum class GnoblinRecoveryFallbackSource {
    None,
    LastKnownGood,
    BuiltIn,
};

struct GnoblinRecoveryPanel;

/* The panel never starts applications or reloads the runtime itself. The
 * compositor coordinator supplies only the actions it can perform safely. */
using GnoblinRecoveryPanelActionFunc = void (*)(void* user_data);

struct GnoblinRecoveryPanelActions {
    GnoblinRecoveryPanelActionFunc open_config_folder;
    GnoblinRecoveryPanelActionFunc open_terminal;
    GnoblinRecoveryPanelActionFunc reload_config;
    void* user_data;
};

GnoblinRecoveryPanel* gnoblin_recovery_panel_new();
void gnoblin_recovery_panel_free(GnoblinRecoveryPanel* panel);

/* Passing nullptr for @error_message clears the diagnostic. Calling this
 * repeatedly with unchanged state does not re-open a notice the user hid. */
bool gnoblin_recovery_panel_set_state(GnoblinRecoveryPanel* panel, const char* error_message,
                                      GnoblinRecoveryFallbackSource fallback_source,
                                      bool shell_missing);
void gnoblin_recovery_panel_set_actions(GnoblinRecoveryPanel* panel,
                                        const GnoblinRecoveryPanelActions* actions);

bool gnoblin_recovery_panel_should_draw(const GnoblinRecoveryPanel* panel);
void gnoblin_recovery_panel_dismiss(GnoblinRecoveryPanel* panel);

/* Suitable for a compositor UI coordinator's overlay callback. */
void gnoblin_recovery_panel_draw(GnoblinImGuiOverlay* overlay, GnoblinRecoveryPanel* panel);

/* Bounds of the last painted panel, in overlay logical coordinates. */
bool gnoblin_recovery_panel_contains(const GnoblinRecoveryPanel* panel, float x, float y);

/* Bottom edge of the painted panel when it covers any part of the horizontal
 * span [@left, @right), else 0. Notices use it to stay clear of the panel. */
float gnoblin_recovery_panel_bottom_over(const GnoblinRecoveryPanel* panel, float left,
                                         float right);

/* Transient notices ("toasts"): short, passive messages stacked at the top
 * centre of the output. They never take keys; the pointer only interacts inside
 * a notice, where a click dismisses it. Times are monotonic microseconds
 * supplied by the caller, so the lifecycle does not depend on a clock. */
enum class GnoblinNoticeKind {
    /* Config applied: "Config reloaded". Removed after a short time and
     * replaces any earlier reload failure. */
    ConfigReloaded,
    /* Config rejected: the first line of the error. Stays until dismissed or
     * until a reload succeeds. */
    ConfigFailed,
    /* A configured shortcut or command did not start. Removed after a while;
     * repeats of the same message are coalesced into one notice. */
    CommandFailed,
};

struct GnoblinNoticeStack;

GnoblinNoticeStack* gnoblin_notice_stack_new();
void gnoblin_notice_stack_free(GnoblinNoticeStack* stack);

/* @message is reduced to its first line and a bounded length. Returns true
 * when what is on screen changed. */
bool gnoblin_notice_stack_post(GnoblinNoticeStack* stack, GnoblinNoticeKind kind,
                               const char* message, std::int64_t now_us);

/* Removes notices whose time has run out. Returns true when any were removed. */
bool gnoblin_notice_stack_expire(GnoblinNoticeStack* stack, std::int64_t now_us);

/* Monotonic time of the next removal, or 0 when no notice is timed. */
std::int64_t gnoblin_notice_stack_next_expiry(const GnoblinNoticeStack* stack);

bool gnoblin_notice_stack_should_draw(const GnoblinNoticeStack* stack);
std::size_t gnoblin_notice_stack_count(const GnoblinNoticeStack* stack);
/* The message line of the notice at @index, oldest first, or "" past the end. */
const char* gnoblin_notice_stack_detail(const GnoblinNoticeStack* stack, std::size_t index);

/* Notices start below @recovery when it would otherwise sit under them. */
void gnoblin_notice_stack_draw(GnoblinImGuiOverlay* overlay, GnoblinNoticeStack* stack,
                               const GnoblinRecoveryPanel* recovery);

/* Bounds of the notices painted last, in overlay logical coordinates. The gaps
 * between notices are outside. */
bool gnoblin_notice_stack_contains(const GnoblinNoticeStack* stack, float x, float y);
