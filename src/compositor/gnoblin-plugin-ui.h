/* In-process diagnostics UI bridge for the C Mutter plugin. */

#pragma once

#include <glib.h>
#include <clutter/clutter.h>
#include "meta/display.h"

G_BEGIN_DECLS

typedef struct _GnoblinPluginUi GnoblinPluginUi;

GnoblinPluginUi *gnoblin_plugin_ui_new (ClutterActor *parent, MetaDisplay *display);
void gnoblin_plugin_ui_free (GnoblinPluginUi *ui);

void gnoblin_plugin_ui_toggle_console (GnoblinPluginUi *ui);
void gnoblin_plugin_ui_hide_console (GnoblinPluginUi *ui);
void gnoblin_plugin_ui_set_session_locked (GnoblinPluginUi *ui, gboolean locked);
gboolean gnoblin_plugin_ui_console_visible (GnoblinPluginUi *ui);

/* Console is modal; passive recovery captures pointer events inside its panel. */
gboolean gnoblin_plugin_ui_handle_event (GnoblinPluginUi   *ui,
                                         ClutterEvent      *event);

G_END_DECLS
