/*
 * Recovery notices drawn in Gnoblin's compositor-owned ImGui overlay.
 *
 * This is presentation only. The session supervisor and native control decide
 * when a configuration fallback or shell failure has occurred.
 */

#pragma once

#include "gnoblin-imgui-overlay.hpp"

enum class GnoblinRecoveryFallbackSource {
    None,
    LastKnownGood,
    BuiltIn,
};

struct GnoblinRecoveryPanel;

/* The panel never starts applications or reloads the runtime itself. The
 * compositor coordinator supplies only the actions it can perform safely. */
using GnoblinRecoveryPanelActionFunc = void (*) (void *user_data);

struct GnoblinRecoveryPanelActions {
    GnoblinRecoveryPanelActionFunc open_config_folder;
    GnoblinRecoveryPanelActionFunc open_terminal;
    GnoblinRecoveryPanelActionFunc reload_config;
    void                          *user_data;
};

GnoblinRecoveryPanel *gnoblin_recovery_panel_new ();
void gnoblin_recovery_panel_free (GnoblinRecoveryPanel *panel);

/* Passing nullptr for @error_message clears the diagnostic. Calling this
 * repeatedly with unchanged state does not re-open a notice the user hid. */
bool gnoblin_recovery_panel_set_state (GnoblinRecoveryPanel          *panel,
                                       const char                    *error_message,
                                       GnoblinRecoveryFallbackSource  fallback_source,
                                       bool                           shell_missing);
void gnoblin_recovery_panel_set_actions (GnoblinRecoveryPanel             *panel,
                                         const GnoblinRecoveryPanelActions *actions);

bool gnoblin_recovery_panel_should_draw (const GnoblinRecoveryPanel *panel);
void gnoblin_recovery_panel_dismiss (GnoblinRecoveryPanel *panel);

/* Suitable for a compositor UI coordinator's overlay callback. */
void gnoblin_recovery_panel_draw (GnoblinImGuiOverlay *overlay,
                                  GnoblinRecoveryPanel *panel);

/* Bounds of the last painted panel, in overlay logical coordinates. */
bool gnoblin_recovery_panel_contains (const GnoblinRecoveryPanel *panel, float x, float y);
