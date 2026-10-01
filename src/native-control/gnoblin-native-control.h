/* Native control endpoint for the standalone Gnoblin compositor preview. */
#pragma once

#include <glib.h>
#include <clutter/clutter.h>
#include "mtk/mtk.h"
#include "meta/window.h"

#include "meta/meta-context.h"
#include "meta/meta-enums.h"
#include "core/gnoblin-runtime-cache.h"

#define GNOBLIN_NATIVE_CONTROL_API_MAJOR 1
#define GNOBLIN_NATIVE_CONTROL_API_MINOR 51

typedef struct _GnoblinNativeControl GnoblinNativeControl;

META_EXPORT
void gnoblin_native_control_window_menu_requested(MetaDisplay* display, MetaWindow* window,
                                                  MetaWindowMenuType menu, int x, int y);

META_EXPORT
GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   int runtime_fd, guint64 settings_revision,
                                                   GError** error);
META_EXPORT
GVariant* gnoblin_native_control_receive_runtime_config(int runtime_fd, guint64* settings_revision,
                                                        GError** error);
gboolean gnoblin_native_control_dispatch_runtime_event(MetaDisplay* display, const char* event,
                                                       GVariant* payload, gboolean* claimed,
                                                       GError** error);
META_EXPORT
gboolean gnoblin_native_control_overlay_modifier_pressed(MetaDisplay* display,
                                                         const ClutterEvent* event);
META_EXPORT
void gnoblin_native_control_observe_key_event(MetaDisplay* display, const ClutterEvent* event);
META_EXPORT
void gnoblin_native_control_set_overlay_modifier_hook_available(MetaDisplay* display,
                                                                gboolean available);
META_EXPORT
void gnoblin_native_control_cancel_shortcut_session(MetaDisplay* display, guint64 session_id,
                                                    const char* reason);
META_EXPORT
gboolean gnoblin_native_control_is_session(MetaDisplay* display);
gboolean gnoblin_native_control_is_supervised(MetaDisplay* display);
META_EXPORT
GVariant* gnoblin_native_control_get_config_document(MetaDisplay* display);
META_EXPORT
guint64 gnoblin_native_control_get_config_revision(MetaDisplay* display);
META_EXPORT
gboolean gnoblin_native_control_get_config_bool(MetaDisplay* display, const char* section,
                                                const char* key, gboolean fallback);
META_EXPORT
gboolean gnoblin_native_control_protocol_enabled(const char* protocol);
META_EXPORT
void gnoblin_native_control_stop(GnoblinNativeControl* control);

/* Dispatch a queued Lua input-source selection through native input control. */
gboolean gnoblin_native_control_select_input_source(MetaDisplay* display, GVariant* arguments,
                                                    gint64 request_id, const char* method,
                                                    GError** error);

/* Dispatch launch-feedback operations for the Lua runtime operation drain. */
GVariant* gnoblin_native_control_dispatch_launch(MetaDisplay* display, const char* method,
                                                 GVariant* arguments, GError** error);

/* Deliver a lock request to subscribed shell clients; completion is not lock proof. */
GVariant* gnoblin_native_control_request_session_lock(MetaDisplay* display, GVariant* arguments,
                                                      GError** error);

/* Start a single native shortcut capture and complete its Lua operation later. */
gboolean gnoblin_native_control_begin_shortcut_capture(MetaDisplay* display, GVariant* arguments,
                                                       gint64 request_id, GError** error);
GVariant* gnoblin_native_control_focus_window(MetaDisplay* display, GVariant* arguments,
                                              guint64 context_handle, guint64 generation,
                                              GError** error);
GVariant* gnoblin_native_control_begin_window_grab(MetaDisplay* display, const char* method,
                                                   GVariant* arguments, guint64 context_handle,
                                                   guint64 generation, GError** error);
GVariant* gnoblin_native_control_begin_menu_window_grab(MetaDisplay* display, const char* method,
                                                        GVariant* arguments, guint64 context_handle,
                                                        guint64 generation,
                                                        guint64 socket_owner_client_id,
                                                        GError** error);
GVariant* gnoblin_native_control_create_text_target(MetaDisplay* display, guint64 context_handle,
                                                    guint64 generation, gint64 expires_at_us,
                                                    GError** error);
GVariant* gnoblin_native_control_insert_text(MetaDisplay* display, GVariant* arguments,
                                             GError** error);
void gnoblin_native_control_revoke_focus_contexts(MetaDisplay* display);
guint64 gnoblin_native_control_window_drag_begin(MetaDisplay* display, MetaWindow* window,
                                                 int pointer_x, int pointer_y, guint32 modifiers);
void gnoblin_native_control_window_drag_update(MetaDisplay* display, guint64 drag_id,
                                               MetaWindow* window, int pointer_x, int pointer_y,
                                               guint32 modifiers);
gboolean gnoblin_native_control_take_window_drag_snap(MetaDisplay* display, guint64 drag_id,
                                                      MetaWindow* window, int release_x,
                                                      int release_y, guint32 modifiers,
                                                      MtkRectangle* out_frame, char** out_target_id,
                                                      gboolean* out_maximize);
void gnoblin_native_control_window_drag_end(MetaDisplay* display, guint64 drag_id,
                                            gboolean committed, const char* reason);
GVariant* gnoblin_native_control_create_snap_context(MetaDisplay* display, guint64 context_handle,
                                                     guint64 generation, gint64 expires_at_us,
                                                     GError** error);
GVariant* gnoblin_native_control_commit_snap_context(MetaDisplay* display, GVariant* arguments,
                                                     GError** error);
GVariant* gnoblin_native_control_restore_or_minimize_window(MetaDisplay* display,
                                                            GVariant* arguments, GError** error);

/* Dispatch portal grant reads and revocations through the session portal. */
gboolean gnoblin_native_control_portal_grant_operation(MetaDisplay* display, const char* method,
                                                       GVariant* arguments, gint64 request_id,
                                                       GError** error);
