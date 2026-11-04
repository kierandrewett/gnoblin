/*
 * Gnoblin native control dispatch: the compositor-side entry points that the
 * native control socket, the Lua runtime and the animation runtime call.
 * Implemented in gnoblin-native-dispatch.c.
 */

#pragma once

#include <glib.h>

#include "meta/display.h"
#include "meta/meta-base.h"
#include "meta/types.h"

META_EXPORT
int meta_gnoblin_layer_anchor(MetaWindow* window);

META_EXPORT
const char* meta_gnoblin_layer_namespace(MetaWindow* window);

META_EXPORT
gboolean meta_gnoblin_window_pattern_match(const char* pattern, const char* value, GError** error);

META_EXPORT
const char* meta_gnoblin_workspace_id(MetaWorkspace* workspace);

META_EXPORT
GVariant* meta_gnoblin_dispatch_native_api(MetaDisplay* display, const char* method,
                                           GVariant* arguments, GError** error);

META_EXPORT
gboolean meta_gnoblin_initialize_native_workspaces(MetaDisplay* display, GError** error);

gboolean meta_display_dispatch_gnoblin_shortcut_event(MetaDisplay* display, const char* event,
                                                      GVariant* payload, guint64 context_handle,
                                                      guint64 generation, gint64 expires_at_us);
