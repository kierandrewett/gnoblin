/* In-process diagnostics UI bridge for the C Mutter plugin. */

#pragma once

#include <glib.h>
#include <clutter/clutter.h>
#include "meta/display.h"
#include "core/gnoblin-native-control.h"

G_BEGIN_DECLS

typedef struct _GnoblinPluginUi GnoblinPluginUi;

GnoblinPluginUi* gnoblin_plugin_ui_new(ClutterActor* parent, MetaDisplay* display);
void gnoblin_plugin_ui_free(GnoblinPluginUi* ui);

void gnoblin_plugin_ui_toggle_console(GnoblinPluginUi* ui);

/* Implemented in gnoblin-plugin.c, because Mutter's public headers have no C++
 * linkage. Output scale of the view being painted, or 1 when there is none. The
 * overlay bakes its text at this density. */
float gnoblin_plugin_paint_scale(ClutterPaintContext* paint_context);
/* Clipboard text for the panels, backed by Mutter's selection in gnoblin-plugin.c. */
const char* gnoblin_plugin_clipboard_text(void);
void gnoblin_plugin_clipboard_set_text(const char* text);

void gnoblin_plugin_ui_hide_console(GnoblinPluginUi* ui);

void gnoblin_plugin_ui_set_session_locked(GnoblinPluginUi* ui, gboolean locked);
/* Developer debug overlay driven by the compositor.debug config key. The panel
 * never takes input, so callers only toggle visibility here. */
void gnoblin_plugin_ui_set_debug(GnoblinPluginUi* ui, gboolean enabled);
gboolean gnoblin_plugin_ui_console_visible(GnoblinPluginUi* ui);
/* Show a short transient notice drawn above windows. Passive: it takes no keys, and a pointer
 * click only counts inside the notice, where it dismisses it. */
void gnoblin_plugin_ui_post_notice(GnoblinPluginUi* ui, GnoblinNativeNoticeKind kind,
                                   const char* message);

/* Console is modal; passive recovery panels and notices capture pointer events inside their
 * bounds. */
gboolean gnoblin_plugin_ui_handle_event(GnoblinPluginUi* ui, const ClutterEvent* event);

/* Key events, offered before keybindings and shortcuts. While the console is open it takes every
 * key except the Ctrl+Alt+F1..F12 console switches. */
gboolean gnoblin_plugin_ui_handle_key_event(GnoblinPluginUi* ui, const ClutterEvent* event);

G_END_DECLS
