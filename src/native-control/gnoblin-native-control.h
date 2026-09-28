/* Native control endpoint for the standalone Gnoblin compositor preview. */
#pragma once

#include <glib.h>

#include "meta/meta-context.h"

#define GNOBLIN_NATIVE_CONTROL_API_MAJOR 1
#define GNOBLIN_NATIVE_CONTROL_API_MINOR 16

typedef struct _GnoblinNativeControl GnoblinNativeControl;

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   GError** error);
void gnoblin_native_control_stop(GnoblinNativeControl* control);

/* Dispatch a queued Lua input-source selection through native input control. */
gboolean gnoblin_native_control_select_input_source(MetaDisplay* display, GVariant* arguments,
                                                    gint64 request_id, const char* method,
                                                    GError** error);

/* Dispatch launch-feedback operations for the Lua runtime operation drain. */
GVariant* gnoblin_native_control_dispatch_launch(MetaDisplay* display, const char* method,
                                                 GVariant* arguments, GError** error);

/* Start a single native shortcut capture and complete its Lua operation later. */
gboolean gnoblin_native_control_begin_shortcut_capture(MetaDisplay* display, GVariant* arguments,
                                                       gint64 request_id, GError** error);
GVariant* gnoblin_native_control_focus_window(MetaDisplay* display, GVariant* arguments,
                                              guint64 context_handle, guint64 generation,
                                              GError** error);
GVariant* gnoblin_native_control_begin_window_grab(MetaDisplay* display, const char* method,
                                                   GVariant* arguments, guint64 context_handle,
                                                   guint64 generation, GError** error);
void gnoblin_native_control_revoke_focus_contexts(MetaDisplay* display);

/* Dispatch portal grant reads and revocations through the session portal. */
gboolean gnoblin_native_control_portal_grant_operation(MetaDisplay* display, const char* method,
                                                       GVariant* arguments, gint64 request_id,
                                                       GError** error);
