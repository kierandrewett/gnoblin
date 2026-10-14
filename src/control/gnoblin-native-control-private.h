#ifndef GNOBLIN_NATIVE_CONTROL_PRIVATE_H
#define GNOBLIN_NATIVE_CONTROL_PRIVATE_H

/* The first native owner of compositor-v1: Lua-validated compositor methods. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"
#include "core/gnoblin-native-control.h"
#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-keybinding-catalog.h"
#include "core/gnoblin-auth-agent.h"
#include "core/gnoblin-location-agent.h"
#include "core/gnoblin-input-disposition.h"
#ifdef HAVE_REMOTE_DESKTOP
#include "core/gnoblin-pipewire-monitor.h"
#endif
#include "core/gnoblin-runtime-cache.h"
#include "core/gnoblin-runtime-protocol.h"
#include "core/gnoblin-touchpad-router.h"
#include "core/gnoblin-priority.h"
#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib/gqsort.h>
#include <glib/gstdio.h>
#include <glib-unix.h>
#include <json-glib/json-glib.h>
#include <errno.h>
#include <dirent.h>
#include <fcntl.h>
#include <math.h>
#include <string.h>
#include <sys/random.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <poll.h>
#include <spawn.h>
#include <signal.h>
#include <unistd.h>
#include <xkbcommon/xkbcommon.h>
#include "backends/meta-backend-private.h"
#include "backends/meta-keymap-description-private.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-output.h"
#include "backends/meta-settings-private.h"
#include "clutter/clutter.h"
#include "compositor/meta-window-actor-private.h"
#include "compositor/meta-window-actor-x11.h"
#include "compositor/meta-window-actor-wayland.h"
#include "compositor/meta-gnoblin-window-effects.h"
#include "core/display-private.h"
#include "core/events.h"
#include "core/keybindings-private.h"
#include "core/util-private.h"
#include "core/window-private.h"
#include "wayland/gnoblin-portal-policy.h"
#include "wayland/gnoblin-lua-pattern.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-layer-shell.h"
#include "wayland/meta-gnoblin-live-protocols.h"
#include "wayland/meta-wayland-activation.h"
#include "wayland/meta-wayland-seat.h"
#include "wayland/meta-wayland-text-input.h"
#include "wayland/meta-window-wayland.h"
#include "wayland/meta-gnoblin-window-frame.h"
#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-cursor-tracker.h"
#include "meta/meta-context.h"
#include "meta/meta-logical-monitor.h"
#include "meta/meta-monitor.h"
#include "meta/meta-wayland-compositor.h"
#ifdef HAVE_XWAYLAND
#include "meta/meta-x11-display-private.h"
#include "wayland/meta-xwayland-private.h"
#endif
#include "meta/meta-monitor-manager.h"
#include "meta/meta-orientation-manager.h"
#include "meta/meta-remote-access-controller.h"
#include "meta/meta-workspace-manager.h"
#include "meta/workspace.h"
#include "meta/meta-wayland-surface.h"
#include "meta/prefs.h"
#include "meta/util.h"
#include "meta/window.h"
#include "wayland/meta-wayland-session-lock.h"
#include <xkbcommon/xkbregistry.h>
#define MAX_REQUEST_BYTES (64 * 1024)
#define MAX_PENDING_BYTES (1024 * 1024)
#define MAX_UI_SESSIONS 64
#define MAX_UI_SESSION_NAME_LENGTH 128
#define MAX_UI_SESSION_PAYLOAD_BYTES (8 * 1024)
#define NATIVE_CONSOLE_TOGGLE_BRIDGE_DATA_KEY "gnoblin-console-toggle-bridge"
#define NATIVE_DEBUG_BRIDGE_DATA_KEY "gnoblin-debug-bridge"
#define OVERLAY_MODIFIER_HOOK_AVAILABLE_DATA_KEY "gnoblin-overlay-modifier-hook-available"
#define MAX_LAUNCHES 64
#define MAX_FOCUS_CONTEXTS 128
#define MAX_MENU_CONTEXTS 256
#define MAX_TEXT_TARGETS 128
#define MAX_SNAP_CONTEXTS 128
#define MAX_PENDING_THUMBNAILS 4
#define MAX_PENDING_THUMBNAILS_PER_CLIENT 1
#define FOCUS_CONTEXT_LIFETIME_US (5 * G_USEC_PER_SEC)
#define NATIVE_POLICY_BUS_NAME "org.gnoblin.Compositor"
#define NATIVE_POLICY_OBJECT_PATH "/org/gnoblin/Compositor"
#define NATIVE_POLICY_INTERFACE "org.gnoblin.Compositor"
typedef struct _NativeMutterSignalWatch NativeMutterSignalWatch;
struct _NativeMutterSignalWatch {
    GnoblinNativeControl* control;
    char* event;
    char* source;
    char* signal;
    guint signal_id;
};
typedef struct {
    GnoblinNativeConsoleToggleFunc handler;
    gpointer user_data;
} NativeConsoleToggleBridge;
typedef struct {
    GnoblinNativeDebugFunc handler;
    gpointer user_data;
} NativeDebugBridge;
typedef struct {
    guint64 owner_client_id;
    JsonNode* state;
    char* surface;
    char** companions;
    gboolean companions_above;
} NativeUiSession;
typedef struct {
    guint64 request_id;
    guint64 generation;
    ClutterEvent* event;
    ClutterActor* actor;
    GVariant* handler_ids;
    char* held_disposition_key;
    guint64 focus_context_handle;
    gint64 focus_context_expires_at_us;
    GVariant* target_window;
    gboolean reserved_press;
    gboolean dispatched;
} NativePendingInputEvent;
typedef struct {
    GnoblinNativeControl* control;
    guint source_id;
    guint64 client_id;
    gint64 operation_id;
    char* window_id;
    guint width;
    guint height;
} PendingThumbnail;
typedef struct {
    Client* client;
    char* response;
} PendingWrite;
typedef struct {
    char* name;
    char* binding;
    char** bindings;
    guint binding_count;
    char** argv;
    char* action_id;
    guint action;
    gboolean enabled;
    gboolean overlay;
    gboolean release;
} NativeShortcut;
typedef struct {
    char* token;
    char* application;
    char* normalized_application;
    char* state;
    char* initial_focus_id;
    GHashTable* initial_window_ids;
    gint64 started_at;
    gint64 deadline_us;
    guint timeout_ms;
    guint64 revision;
} NativeLaunch;
typedef struct {
    guint64 generation;
    gint64 expires_at_us;
    guint32 timestamp;
    guint64 surface_id;
    guint64 client_id;
    guint64 focus_epoch;
    guint32 modifiers;
    xkb_mod_mask_t allowed_modifier_mask;
    gboolean pointer_binding;
    guint pointer_button;
    guint64 pointer_target_sequence;
    double pointer_x;
    double pointer_y;
} NativeFocusContext;
typedef struct {
    guint64 generation;
    gint64 expires_at_us;
    guint64 surface_id;
    guint64 client_id;
    guint64 focus_epoch;
    xkb_mod_mask_t allowed_modifier_mask;
    guint64 socket_owner_client_id;
} NativeTextTarget;
typedef struct {
    guint64 settings_revision;
    guint64 native_generation;
    gint64 expires_at_us;
    char* window_id;
    char* monitor_id;
    MtkRectangle work_area;
    MtkRectangle original_frame;
    guint64 socket_owner_client_id;
} NativeSnapContext;
typedef struct {
    guint64 handle;
    guint64 generation;
    gint64 expires_at_us;
} NativeFocusGrant;
typedef struct {
    char* window_id;
    guint64 generation;
    gint64 expires_at_us;
    guint64 socket_owner_client_id;
} NativeMenuContext;
typedef struct {
    guint64 handle;
    guint64 generation;
    gint64 expires_at_us;
} NativeMenuGrant;
typedef enum {
    INPUT_BOOLEAN,
    INPUT_DOUBLE,
    INPUT_INTEGER,
    INPUT_MILLISECONDS,
    INPUT_STRING,
    INPUT_STRINGS,
    INPUT_CHOICE,
    INPUT_ACCEL_CURVE,
    INPUT_ACCESSIBILITY,
    INPUT_PAD_BUTTONS,
    INPUT_TABLET_AREA,
    INPUT_TABLET_OUTPUT,
} InputKind;
typedef struct {
    const char* group;
    const char* name;
    InputKind kind;
    const char* choices;
    double minimum;
    double maximum;
} InputField;
typedef struct {
    guint64 generation;
    gint64 expires_at_us;
} NativeAcceptedInputRequest;

/* Declarations shared between the split files. */
extern GnoblinRuntimeCache* bootstrap_runtime_cache;
extern guint64 bootstrap_runtime_generation;
void native_runtime_abort(GnoblinNativeControl* control);
GVariant* native_config_document(GnoblinNativeControl* control);
void native_apply_location_policy(GnoblinNativeControl* control, GVariant* config);
guint64 native_config_revision(GnoblinNativeControl* control);
guint64 native_config_generation(GnoblinNativeControl* control);
void native_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                     GVariant* snapshot, guint64 revision);
MetaWindow* native_window_by_stable_id(GnoblinNativeControl* control, const char* stable_id);
gboolean native_runtime_begin_thumbnail(GnoblinNativeControl* control, gint64 operation_id,
                                        GVariant* arguments, guint64 client_id, GError** error);
extern const NativeCapability native_capabilities[];
extern const gsize native_capabilities_count;
extern const char* native_socket_events[];
gboolean start_native_policy_dbus(GnoblinNativeControl* control);
void stop_native_policy_dbus(GnoblinNativeControl* control);
void native_shortcut_free(gpointer data);
GVariant* native_shortcut_snapshot(GnoblinNativeControl* control);
void native_window_state_free(gpointer data);
void native_snap_context_free(gpointer data);
char** native_command_argv(GVariant* command);
void native_shortcut_activated(MetaDisplay* display, guint action, gpointer device, guint timestamp,
                               gpointer user_data);
void capture_focus_identity(GnoblinNativeControl* control, NativeFocusContext* context,
                            const ClutterEvent* event);
gboolean native_pointer_button_is_down(GnoblinNativeControl* control, guint button);
void native_trusted_shortcut_activated(MetaDisplay* display, guint action,
                                       const ClutterEvent* event, gpointer user_data);
gboolean focus_token_random(char token[65]);
void revoke_focus_grants_for_handle(GnoblinNativeControl* control, guint64 handle);
void revoke_focus_contexts(GnoblinNativeControl* control);
void native_menu_context_free(gpointer data);
void revoke_menu_contexts(GnoblinNativeControl* control);
guint64 native_menu_context_create(GnoblinNativeControl* control, const char* window_id,
                                   guint64 socket_owner_client_id, gint64* expires_at_us);
void native_menu_prune_client_grants(Client* client, gint64 now);
void revoke_text_targets(GnoblinNativeControl* control);
void native_socket_revoke_client_tokens(Client* client);
void prune_focus_contexts(GnoblinNativeControl* control, gint64 now);
gboolean focus_context_expiry_tick(gpointer user_data);
void dynamic_shortcut_publish_event(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                    const char* event, GVariant* payload);
gboolean issue_focus_context_for_session_key(GnoblinNativeControl* control,
                                             NativeDynamicShortcut* shortcut,
                                             const ClutterEvent* event, guint64* handle_out,
                                             guint64* generation_out, gint64* expires_at_us_out,
                                             char token_out[65]);
void dynamic_shortcut_end_session(GnoblinNativeControl* control, NativeDynamicShortcut* shortcut,
                                  const char* reason);
gboolean native_shortcut_session_timeout(gpointer user_data);
gboolean apply_configured_capture_shortcut(GnoblinNativeControl* control, GVariant* document,
                                           GError** error);
void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                         const ClutterEvent* event);
void native_shortcut_deactivated(MetaDisplay* display, guint action, gpointer device,
                                 guint timestamp, gpointer user_data);
gboolean apply_native_keybindings(GnoblinNativeControl* control, GVariant* document,
                                  GError** error);
gboolean start_native_shortcuts(GnoblinNativeControl* control, GVariant* document, GError** error);
gboolean replace_native_shortcuts(GnoblinNativeControl* control, GVariant* document,
                                  GError** error);
GVariant* normalize_input_fields(const char* group, GVariant* fields, GError** error);
gboolean valid_input_device(const char* group, const char* device);
GVariant* native_orientation_lock_snapshot(GnoblinNativeControl* control, guint64 revision);
void native_orientation_lock_publish(GnoblinNativeControl* control);
GVariant* native_monitor_privacy_screen_snapshot(GnoblinNativeControl* control, guint64 revision);
void native_monitor_privacy_screen_publish(GnoblinNativeControl* control);
void native_orientation_manager_notified(GObject* manager, GParamSpec* property,
                                         gpointer user_data);
void native_orientation_manager_accelerometer_notified(GObject* manager, GParamSpec* property,
                                                       gpointer user_data);
void native_orientation_manager_orientation_changed(MetaOrientationManager* manager,
                                                    gpointer user_data);
gboolean apply_native_input(GnoblinNativeControl* control, MetaContext* context, GVariant* document,
                            GError** error);
void client_maybe_free(Client* client);
void pending_runtime_request_free(gpointer data);
void client_close(Client* client);
JsonNode* json_from_variant(GVariant* value);
GVariant* variant_from_json(JsonNode* node);
char* encode_response(const char* id, JsonNode* result, const char* message);
char* encode_event_subscription(JsonArray* events, guint api_minor);
GVariant* capability_snapshot_record(GnoblinNativeControl* control,
                                     const NativeCapability* native_capability);
GVariant* capability_snapshot(GnoblinNativeControl* control);
const NativeCapability* native_capability_by_id(const char* id);
GVariant* input_device_record(GnoblinNativeControl* control, ClutterInputDevice* device);
gboolean native_input_operation_allowed(GnoblinNativeControl* control, guint64 id);
void native_input_cancel_pending(GnoblinNativeControl* control, gboolean forward);
gboolean native_input_handle_decision(GnoblinNativeControl* control,
                                      const GnoblinRuntimePacket* packet, GError** error);
GVariant* input_device_snapshot(GnoblinNativeControl* control);
void native_control_maybe_free_stopped(GnoblinNativeControl* control);
void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete, gboolean ok,
                                  const char* accelerator, const char* error_code,
                                  const char* message);
gboolean native_shortcut_hold_family_pressed(GnoblinNativeControl* control, guint32 hold_mask);
void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data);
GVariant* native_session_lock_snapshot(MetaWaylandCompositor* compositor, guint64 revision);
void native_session_lock_changed(MetaWaylandCompositor* compositor,
                                 MetaWaylandSessionLockState state, gpointer user_data);
JsonNode* window_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                               GError** error);
char* window_snapshot(GnoblinNativeControl* control);
typedef struct {
    const char* native_name;
    const char* lua_name;
} WindowPropertyName;

extern const WindowPropertyName window_property_names[];
extern const gsize window_property_names_count;
gboolean window_record_is_modal(JsonObject* record);
JsonNode* lua_window_record(JsonNode* native_record, guint64 revision);
gboolean native_event_is_subscribable(const char* name);
gboolean native_client_has_mutter_event_subscriptions(Client* client);
void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload);
void native_publish_mutter_socket_event(GnoblinNativeControl* control, const char* event,
                                        GVariant* payload);
void dispatch_lua_layer_event(GnoblinNativeControl* control, guint64 revision, const char* event,
                              const char* id, JsonNode* layer, JsonNode* last, JsonArray* changed);
void native_publish_request_event(GnoblinNativeControl* control, const char* name,
                                  GVariant* fields);
const char* native_runtime_status_state(GnoblinNativeControl* control);
void native_runtime_publish_status(GnoblinNativeControl* control);
GVariant* appearance_snapshot_new(GnoblinNativeControl* control);
void appearance_color_scheme_changed(GSettings* settings, const char* key, gpointer user_data);
void privacy_pipewire_state_changed(GnoblinPipewireMonitor* monitor, gboolean available,
                                    gboolean microphone_in_use, gboolean camera_in_use,
                                    gpointer user_data);
GVariant* native_location_authorize_operation(GnoblinNativeControl* control, GVariant* arguments,
                                              guint64 client_id, GError** error);
void native_settings_changed(guint64 revision, gpointer user_data);
GVariant* filter_native_touchpad_gestures(GVariant* gestures);
gboolean native_config_event(MetaDisplay* display, const char* event, GVariant* document,
                             GVariant* payload, gpointer user_data);
void native_native_event(MetaDisplay* display, const char* event, GVariant* payload,
                         gpointer user_data);
gboolean focus_token_has_valid_shape(const char* token);
gboolean native_socket_has_exact_fields(JsonObject* object, const char* const* fields,
                                        gsize n_fields);
gboolean native_socket_has_nul_escape(const char* data, gsize length);
gboolean native_socket_take_focus_grant(Client* client, JsonObject* arguments,
                                        NativeFocusGrant* out_grant, GError** error);
GVariant* native_socket_focus_window(Client* client, JsonObject* json_arguments, GError** error);
GVariant* native_socket_begin_window_grab(Client* client, const char* method,
                                          JsonObject* json_arguments, GError** error);
GVariant* native_socket_begin_menu_window_grab(Client* client, const char* method,
                                               JsonObject* json_arguments, GError** error);
void native_launch_free(gpointer data);
char* normalize_launch_application(const char* value);
char* native_window_id(MetaWindow* window);
gboolean native_text_modifiers_allowed(GnoblinNativeControl* control, xkb_mod_mask_t allowed);
gboolean native_text_valid(const char* text);
GVariant* native_socket_create_text_target(Client* client, JsonObject* arguments, GError** error);
GVariant* native_socket_insert_text(Client* client, JsonObject* json_arguments,
                                    gboolean has_nul_escape, GError** error);
gboolean focus_identity_matches(GnoblinNativeControl* control, guint64 surface_id,
                                guint64 client_id, guint64 focus_epoch);
gboolean native_monitor_lookup(GnoblinNativeControl* control, const char* wanted_id,
                               int* monitor_number, MtkRectangle* monitor_rect);
GVariant* native_socket_create_snap_context(Client* client, JsonObject* arguments, GError** error);
GVariant* native_socket_commit_snap_context(Client* client, JsonObject* json_arguments,
                                            GError** error);
GVariant* native_insert_text_owned(MetaDisplay* display, GVariant* arguments,
                                   guint64 socket_owner_client_id, GError** error);
void update_launch_snapshot(GnoblinNativeControl* control);
char* monitor_snapshot(GnoblinNativeControl* control);
char* native_monitor_id_for_index(JsonNode* snapshot, int monitor_index);
void native_show_osd_requested(MetaDisplay* display, gint monitor_index, const gchar* icon_name,
                               const gchar* message, gpointer user_data);
ClutterActor* native_show_pad_osd_requested(MetaDisplay* display, ClutterInputDevice* pad,
                                            GSettings* settings, const gchar* layout_path,
                                            gboolean edition_mode, gint monitor_index,
                                            gpointer user_data);
void publish_layer_changes(GnoblinNativeControl* control, JsonNode* snapshot, guint64 revision);
void publish_window_changes(GnoblinNativeControl* control, JsonNode* snapshot, guint64 revision);
gboolean native_api_animation_read_method(const char* method);
gboolean native_api_read_method(const char* method);
gboolean runtime_config_values_equal(GVariant* left, GVariant* right);
char* queue_runtime_api_request_internal(Client* client, const char* request_id, const char* method,
                                         GVariant* arguments, const char* kind,
                                         const char* legacy_window_action,
                                         const char* legacy_window_id);
char* queue_runtime_api_request(Client* client, const char* request_id, const char* method,
                                GVariant* arguments, const char* kind);
GVariant* native_socket_snap_rect(JsonNode* node);
char* native_socket_window_snap_offer(Client* client, const char* id, JsonObject* arguments);
void native_socket_consume_rejected_menu_context(Client* client, JsonObject* request);
void native_ui_session_free(gpointer data);
void native_ui_sessions_client_disconnected(GnoblinNativeControl* control, guint64 client_id);
char* handle_ui_session_request(Client* client, JsonObject* request);
char* handle_request(Client* client, const char* data, gsize length);
void send_response(Client* client, char* response);
void schedule_windows(GnoblinNativeControl* control);
gboolean native_mutter_event_is_subscribable(GnoblinNativeControl* control, const char* name);
void clear_display_signal_watches(GnoblinNativeControl* control);
void refresh_display_signal_watches(GnoblinNativeControl* control);
void clear_backend_signal_watches(GnoblinNativeControl* control);
void refresh_backend_signal_watches(GnoblinNativeControl* control);
void clear_cursor_tracker_signal_watches(GnoblinNativeControl* control);
void refresh_cursor_tracker_signal_watches(GnoblinNativeControl* control);
void refresh_all_window_signal_watches(GnoblinNativeControl* control);
void native_refresh_all_mutter_signal_watches(GnoblinNativeControl* control);
void track_window(GnoblinNativeControl* control, MetaWindow* window);
void gnoblin_control_watch_vr_status(GnoblinNativeControl* control);
void window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data);
void display_notified(GObject* display, GParamSpec* property, gpointer user_data);
void display_restacked(MetaDisplay* display, gpointer user_data);
void workspace_manager_changed(MetaWorkspaceManager* manager, gpointer user_data);
void monitor_manager_changed(MetaMonitorManager* manager, gpointer user_data);
void display_work_areas_changed(MetaDisplay* display, gpointer user_data);
void monitor_privacy_screen_changed(MetaMonitorManager* manager,
                                    MetaLogicalMonitor* logical_monitor, gboolean enabled,
                                    gpointer user_data);
void backend_keymap_changed(MetaBackend* backend, gpointer user_data);
void backend_keymap_layout_group_changed(MetaBackend* backend, guint group, gpointer user_data);
void input_device_added(ClutterSeat* seat, ClutterInputDevice* device, gpointer user_data);
void input_device_removed(ClutterSeat* seat, ClutterInputDevice* device, gpointer user_data);
void workspace_manager_index_changed(MetaWorkspaceManager* manager, int index, gpointer user_data);
void process_buffer(Client* client);
gboolean client_connected(GSocketService* service, GSocketConnection* connection,
                          GObject* source_object, gpointer user_data);
gboolean native_runtime_send(GnoblinNativeControl* control, GnoblinRuntimePacketType type,
                             guint64 request_id, GVariant* payload, GError** error);
gboolean native_runtime_send_event_packet(GnoblinNativeControl* control, const char* event,
                                          GVariant* packet);
gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                       GVariant* payload);
gboolean native_runtime_flush_state_snapshots(GnoblinNativeControl* control, GError** error);
gboolean native_runtime_flush_pending_events(GnoblinNativeControl* control, GError** error);
void native_runtime_fail_pending_requests(GnoblinNativeControl* control, const char* reason);
gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data);

#endif /* GNOBLIN_NATIVE_CONTROL_PRIVATE_H */
