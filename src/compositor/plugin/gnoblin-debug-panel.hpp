/*
 * Developer diagnostics drawn in Gnoblin's compositor-owned ImGui overlay.
 *
 * Presentation only. The compositor coordinator enables the panel from the
 * session configuration; the panel itself records paint cadence and never
 * claims input.
 */

#pragma once

#include <glib.h>

#include "gnoblin-imgui-overlay.hpp"

struct GnoblinDebugPanel;

GnoblinDebugPanel* gnoblin_debug_panel_new();
void gnoblin_debug_panel_free(GnoblinDebugPanel* panel);

/* Enabling starts a coarse repaint tick so FPS and the frame-time graph stay
 * live on an otherwise quiet stage. Pass a null overlay to stop the source
 * without changing the enabled state, for example during teardown. */
void gnoblin_debug_panel_set_enabled(GnoblinDebugPanel* panel, GnoblinImGuiOverlay* overlay,
                                     gboolean enabled);
gboolean gnoblin_debug_panel_enabled(const GnoblinDebugPanel* panel);

/* Suitable for a compositor UI coordinator's overlay callback; draws nothing
 * while the panel is disabled. */
void gnoblin_debug_panel_draw(GnoblinImGuiOverlay* overlay, GnoblinDebugPanel* panel);
