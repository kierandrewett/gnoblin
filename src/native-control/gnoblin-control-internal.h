/* Shared declarations for the native control plane. gnoblin-native-control.c owns the
 * state; the gnoblin-control-*.c files hold domains that were split out of it. This header is
 * internal to libmutter and is not installed. */
#pragma once

#include "core/gnoblin-native-control.h"
#include "core/gnoblin-auth-agent.h"
#include "core/gnoblin-location-agent.h"
#ifdef HAVE_REMOTE_DESKTOP
#include "core/gnoblin-pipewire-monitor.h"
#endif
#include "core/gnoblin-runtime-cache.h"
#include "core/gnoblin-runtime-protocol.h"
#include "core/gnoblin-touchpad-router.h"

#include <gio/gio.h>
#include <json-glib/json-glib.h>
#include <sys/types.h>

#include "backends/meta-keymap-description-private.h"
#include "clutter/clutter.h"
#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-cursor-tracker.h"
#include "meta/meta-monitor-manager.h"
#include "meta/meta-orientation-manager.h"
#include "meta/meta-remote-access-controller.h"
#include "meta/meta-wayland-compositor.h"
#include "meta/meta-workspace-manager.h"

#define NATIVE_CONTROL_OBJECT_DATA_KEY "gnoblin-native-control"
#define PORTAL_BACKEND_BUS_NAME "org.freedesktop.impl.portal.desktop.gnoblin"

#define MAX_NATIVE_COMMAND_ARGUMENTS 64
#define MAX_NATIVE_COMMAND_BYTES (64 * 1024)

typedef struct {
    char* type;
    char* id;
} NativeWindowInputSource;

typedef struct {
    char* type;
    char* id;
    char* layout;
    char* variant;
    char* short_name;
    char* name;
} NativeInputSource;

/* Cached JSON for one tracked record, kept by the snapshot domains between publishes. */
typedef struct {
    char* json;
    char* comparable_json;
    gboolean focused;
} NativeWindowState;

typedef struct {
    char* json;
    char* comparable_json;
} NativeLayerState;

typedef struct {
    char* json;
} NativeWorkspaceState;

typedef struct {
    char* json;
} NativeMonitorState;

#define NATIVE_DRAG_MOD_CONTROL (1u << 0)
#define NATIVE_DRAG_MAX_TARGETS 128

typedef struct {
    char* id;
    MtkRectangle hit;
    MtkRectangle frame;
    guint required_modifiers;
    guint forbidden_modifiers;
    gboolean maximize;
} NativeSnapTarget;

typedef struct {
    guint64 id;
    guint64 settings_revision;
    guint64 owner_generation;
    gint64 expires_at_us;
    char* window_id;
    char* monitor_id;
    char* committed_target_id;
    MtkRectangle monitor;
    MtkRectangle work_area;
    MtkRectangle frame;
    MtkRectangle original_frame;
    int pointer_x;
    int pointer_y;
    guint32 modifiers;
    gboolean maximized;
    GHashTable* client_tokens;
    guint64 socket_owner_client_id;
    gboolean runtime_offer_claimed;
    GPtrArray* targets;
} NativeWindowDrag;

typedef struct _NativeDynamicShortcut NativeDynamicShortcut;
typedef struct _NativeCornerToolkitCache NativeCornerToolkitCache;
typedef struct _GnoblinControlPromptBroker GnoblinControlPromptBroker;

typedef struct {
    GSocketConnection* connection;
    GnoblinNativeControl* control;
    guint64 client_id;
    pid_t peer_pid;
    GString* request;
    GQueue* outgoing;
    gsize pending_bytes;
    guint api_minor;
    guint windows_api_minor;
    guint monitors_api_minor;
    guint input_devices_api_minor;
    guint input_sources_api_minor;
    guint launch_api_minor;
    guint shortcut_capture_api_minor;
    guint event_api_minor;
    GHashTable* event_subscriptions;
    GHashTable* focus_grants;
    GHashTable* menu_grants;
    gboolean reading;
    gboolean writing;
    gboolean closing;
    gboolean track_windows;
    gboolean track_monitors;
    gboolean track_input_devices;
    gboolean track_input_sources;
    gboolean track_launches;
    gboolean track_shortcut_capture;
    gboolean close_after_response;
    guint pending_thumbnails;
    GHashTable* pending_grant_operations;
    guint pending_deferred_requests;
    gboolean watch_ui_sessions;
} Client;

typedef struct {
    Client* client;
    char* request_id;
    char* legacy_window_action;
    char* legacy_window_id;
    GnoblinNativeRuntimeResponseFunc callback;
    gpointer callback_data;
    GDestroyNotify callback_destroy;
    gboolean discard_response;
} PendingRuntimeRequest;

struct _NativeDynamicShortcut {
    GnoblinNativeControl* control;
    Client* client;
    guint64 client_id;
    char* id;
    char* accelerator;
    char* owner_id;
    guint action;
    guint64 session_id;
    guint32 hold_mask;
    guint32 held_mask;
    guint timeout_id;
    ClutterEvent* trigger_event;
    gboolean trigger_release;
    gboolean modal;
    gboolean capture_input;
    gboolean armed;
    gboolean active;
    gboolean releasing;
    gboolean activation_dispatched;
    gboolean activated;
};

typedef struct {
    const char* id;
    const char* description;
} NativeCapability;

typedef struct {
    GnoblinNativeControl* control;
    GnoblinLocationRequest* request;
    GHashTable* recipient_client_ids;
    guint64 request_id;
    guint requested_accuracy;
    gint64 expires_at_us;
    guint timeout_source_id;
    gboolean completed;
} PendingLocationAuthorization;

struct _GnoblinNativeControl {
    GSocketService* service;
    GDBusConnection* session_bus;
    GHashTable* clients;
    GHashTable* ui_sessions;
    GHashTable* windows;
    GHashTable* window_state;
    GHashTable* layer_state;
    GHashTable* workspace_state;
    GHashTable* workspace_signal_handler_ids;
    GHashTable* monitor_state;
    GHashTable* input_device_state;
    GHashTable* input_device_ids;
    GVariant* portal_grant_snapshot;
    GVariant* privacy_snapshot;
    GPtrArray* input_sources;
    GPtrArray* ibus_sources;
    GPtrArray* configured_input_source_ids;
    GPtrArray* configured_ibus_source_ids;
    GPtrArray* active_input_source_ids;
    GPtrArray* shortcuts;
    gboolean shortcut_handlers_connected;
    GHashTable* dynamic_shortcuts;
    NativeDynamicShortcut* bare_super_shortcut;
    NativeDynamicShortcut* active_shortcut_session;
    GPtrArray* launches;
    GHashTable* focus_contexts;
    GHashTable* window_input_sources;
    GHashTable* menu_contexts;
    GHashTable* text_targets;
    GHashTable* window_drags;
    GHashTable* snap_contexts;
    GHashTable* snap_restore_frames;
    GHashTable* window_shader_files;
    MetaDisplay* display;
    MetaWaylandCompositor* wayland_compositor;
    MetaWorkspaceManager* workspace_manager;
    MetaCursorTracker* cursor_tracker;
    MetaOrientationManager* orientation_manager;
    MetaMonitorManager* monitor_manager;
    ClutterSeat* input_seat;
    MetaBackend* backend;
    MetaRemoteAccessController* remote_access_controller;
#ifdef HAVE_REMOTE_DESKTOP
    GnoblinPipewireMonitor* pipewire_monitor;
#endif
    GnoblinTouchpadRouter* touchpad_router;
    GnoblinLocationAgent* location_agent;
    GnoblinRuntimeCache* runtime_cache;
    GnoblinRuntimeReader* runtime_reader;
    GnoblinRuntimeWriter* runtime_writer;
    int runtime_fd;
    GVariant* native_touchpad_gestures;
    MetaKeymapDescription* input_keymap_description;
    GSettings* appearance_settings;
    guint64 appearance_revision;
    GDBusConnection* ibus_bus;
    char* last_published_input_source;
    char* current_ibus_source_id;
    char* active_input_source_type;
    char* focused_input_window_id;
    guint64 next_input_device_id;
    guint64 input_source_selection_generation;
    guint publish_id;
    guint launch_tick_id;
    guint shortcut_capture_timeout_id;
    guint focus_context_timeout_id;
    guint policy_dbus_registration_id;
    guint portal_grant_added_subscription_id;
    guint portal_grant_removed_subscription_id;
    guint portal_owner_subscription_id;
    guint ibus_signal_subscription_id;
    guint ibus_owner_subscription_id;
    guint ibus_retry_source_id;
    guint64 ibus_owner_generation;
    guint64 ibus_engine_generation;
    guint portal_grant_retry_id;
    guint runtime_read_source_id;
    guint runtime_write_source_id;
    guint64 next_runtime_request_id;
    GHashTable* pending_runtime_requests;
    GHashTable* pending_location_authorizations;
    GnoblinAuthAgent* auth_agent;
    GHashTable* pending_auth;
    GnoblinControlPromptBroker* prompt_broker;
    GHashTable* pending_prompts;
    guint64 next_prompt_request_id;
    GHashTable* pending_captures;
    GHashTable* runtime_operation_ids;
    GHashTable* runtime_cancelled_operation_ids;
    GHashTable* runtime_event_subscriptions;
    GArray* display_signal_handler_ids;
    GArray* workspace_manager_signal_handler_ids;
    GArray* backend_signal_handler_ids;
    GArray* monitor_manager_signal_handler_ids;
    GArray* cursor_tracker_signal_handler_ids;
    GHashTable* window_signal_handler_ids;
    NativeCornerToolkitCache* corner_toolkit_cache;
    GQueue* pending_runtime_states;
    GQueue* pending_runtime_events;
    GQueue* pending_input_events;
    GHashTable* held_input_dispositions;
    guint input_decision_timeout_id;
    guint64 next_input_request_id;
    gboolean input_policy_bypassed;
    char* input_policy_error;
    GnoblinNativeConsoleToggleFunc console_toggle_handler;
    gpointer console_toggle_handler_data;
    GHashTable* accepted_input_requests;
    gint64 shortcut_capture_request_id;
    gulong session_lock_callback_id;
    guint64 state_revision;
    guint64 runtime_generation;
    /* Session-wide high-water mark; replacement workers must not reuse IDs
     * that may still have asynchronous compositor callbacks in flight. */
    guint64 last_runtime_operation_id;
    guint64 launch_revision;
    guint64 portal_grant_revision;
    guint64 privacy_revision;
    GVariant* monitor_privacy_screen_snapshot;
    guint64 monitor_privacy_screen_revision;
    gboolean monitor_privacy_screen_runtime_override;
    gboolean monitor_privacy_screen_runtime_value;
    guint portal_grant_retry_count;
    guint64 session_lock_revision;
    gboolean session_lifecycle_supported;
    GVariant* session_lifecycle_snapshot;
    guint64 session_lifecycle_revision;
    guint64 event_sequence;
    /* Gesture ordering belongs to the compositor and survives Lua worker recovery. */
    guint64 input_gesture_sequence;
    guint64 next_focus_context_handle;
    guint64 next_menu_context_handle;
    guint64 next_text_target_handle;
    guint64 next_shortcut_session_id;
    guint64 next_window_drag_id;
    guint64 next_snap_context_id;
    guint64 next_client_id;
    guint64 next_location_request_id;
    gboolean window_state_initialized;
    gboolean layer_state_initialized;
    gboolean workspace_state_initialized;
    gboolean monitor_state_initialized;
    gboolean input_device_state_initialized;
    gboolean input_source_state_initialized;
    gboolean per_window_input_sources;
    GHashTable* privacy_handles;
    gboolean privacy_screen_sharing;
    gboolean privacy_recording;
    gboolean privacy_microphone_available;
    gboolean privacy_microphone_in_use;
    gboolean privacy_camera_available;
    gboolean privacy_camera_in_use;
    gboolean privacy_location_available;
    gboolean privacy_location_in_use;
    guint privacy_camera_disable_source_id;
    gboolean supervised_runtime;
    gboolean runtime_hello_sent;
    gboolean runtime_worker_suspended;
    gboolean runtime_recovery_failed;
    gboolean stopping;
    gboolean teardown_complete;
    gboolean shortcut_capture_active;
    gboolean shortcut_capture_super_pressed;
    gboolean shortcut_session_capture_active;
    gboolean overlay_modifier_hook_available;
    gboolean super_left_pressed;
    gboolean super_right_pressed;
    gboolean control_left_pressed;
    gboolean control_right_pressed;
    gboolean alt_left_pressed;
    gboolean alt_right_pressed;
    gboolean policy_bus_name_owned;
    gboolean portal_backend_available;
    guint pending_input_source_ops;
    guint pending_ibus_queries;
    guint pending_portal_grant_ops;
    guint pending_thumbnail_count;
    GVariant* session_activity_snapshot;
    guint64 session_activity_revision;
    GVariant* orientation_lock_snapshot;
    guint64 orientation_lock_revision;
    gboolean orientation_lock_configured;
    gboolean orientation_lock_config_value;
    gboolean orientation_lock_runtime_override;
    gboolean orientation_lock_notifications_suppressed;
    guint session_activity_subscription_id;
    guint session_activity_owner_subscription_id;
    guint pending_activity_queries;
    guint64 session_activity_generation;
    char* path;
    dev_t device;
    ino_t inode;
};

/* Helpers that stay in gnoblin-native-control.c and are used by the split domains. */
G_GNUC_INTERNAL void gnoblin_control_publish_event(GnoblinNativeControl* control, const char* name,
                                                   GVariant* fields);
G_GNUC_INTERNAL const char* gnoblin_control_operation_error_code(const GError* error);
G_GNUC_INTERNAL void gnoblin_control_dispatch_operation_completion_full(
    GnoblinNativeControl* control, gint64 request_id, const char* method, gboolean ok,
    JsonNode* result, const char* error_code, const char* message, gboolean dispatch_lua);
G_GNUC_INTERNAL void gnoblin_control_publish_runtime_snapshot(GnoblinNativeControl* control,
                                                              const char* name, GVariant* snapshot,
                                                              guint64 revision);
G_GNUC_INTERNAL gboolean gnoblin_control_dispatch_event(GnoblinNativeControl* control,
                                                        const char* event, GVariant* payload);
G_GNUC_INTERNAL JsonNode* gnoblin_control_json_from_variant(GVariant* value);
G_GNUC_INTERNAL void gnoblin_control_publish_socket_event(GnoblinNativeControl* control,
                                                          JsonNode* payload);
G_GNUC_INTERNAL void gnoblin_control_maybe_free_stopped(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_clear_pending_grant_delivery(GnoblinNativeControl* control,
                                                                  gint64 request_id);
G_GNUC_INTERNAL guint64 gnoblin_control_config_generation(GnoblinNativeControl* control);
G_GNUC_INTERNAL guint64 gnoblin_control_config_revision(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_send_response(Client* client, char* response);
G_GNUC_INTERNAL gboolean gnoblin_control_focus_token_random(char token[65]);
G_GNUC_INTERNAL GVariant* gnoblin_control_capability_snapshot(GnoblinNativeControl* control);
G_GNUC_INTERNAL GVariant* gnoblin_control_capability_snapshot_record(
    GnoblinNativeControl* control, const NativeCapability* native_capability);
G_GNUC_INTERNAL const NativeCapability* gnoblin_control_capability_by_id(const char* id);
G_GNUC_INTERNAL GVariant* gnoblin_control_variant_from_json(JsonNode* node);
G_GNUC_INTERNAL GVariant* gnoblin_control_native_config_document(GnoblinNativeControl* control);
G_GNUC_INTERNAL char* gnoblin_control_native_window_id(MetaWindow* window);
G_GNUC_INTERNAL void gnoblin_control_schedule_windows(GnoblinNativeControl* control);

/* gnoblin-control-auth.c: polkit agent glue (auth.begin, auth.respond, auth.cancel). */
G_GNUC_INTERNAL void gnoblin_control_auth_init(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_auth_apply_config(GnoblinNativeControl* control,
                                                       GVariant* config);
G_GNUC_INTERNAL gboolean gnoblin_control_auth_available(GnoblinNativeControl* control);
G_GNUC_INTERNAL const char* gnoblin_control_auth_unavailable_reason(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_auth_stop(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_auth_note_recipient(GnoblinNativeControl* control,
                                                         gint64 request_id, guint64 client_id);
G_GNUC_INTERNAL GVariant* gnoblin_control_auth_operation(GnoblinNativeControl* control,
                                                         const char* method, GVariant* arguments,
                                                         guint64 client_id, GError** error);

/* gnoblin-control-prompts.c: keyring and GPG passphrase prompter (prompt.respond, prompt.cancel). */
G_GNUC_INTERNAL void gnoblin_control_prompts_init(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_prompts_apply_config(GnoblinNativeControl* control,
                                                          GVariant* config);
G_GNUC_INTERNAL void gnoblin_control_prompts_stop(GnoblinNativeControl* control);
G_GNUC_INTERNAL gboolean gnoblin_control_prompts_available(GnoblinNativeControl* control);
G_GNUC_INTERNAL const char* gnoblin_control_prompts_unavailable_reason(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_prompts_note_recipient(GnoblinNativeControl* control,
                                                            gint64 request_id, guint64 client_id);
G_GNUC_INTERNAL GVariant* gnoblin_control_prompts_operation(GnoblinNativeControl* control,
                                                            const char* method,
                                                            GVariant* arguments,
                                                            guint64 client_id, GError** error);

/* gnoblin-control-capture.c: command.capture. */
G_GNUC_INTERNAL void gnoblin_control_capture_init(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_capture_clear(GnoblinNativeControl* control);
G_GNUC_INTERNAL gboolean gnoblin_control_capture_begin(GnoblinNativeControl* control,
                                                       gint64 operation_id, GVariant* arguments,
                                                       GError** error);

/* gnoblin-control-privacy.c: privacy snapshot and location authorization glue. */
G_GNUC_INTERNAL void gnoblin_control_publish_privacy_snapshot(GnoblinNativeControl* control,
                                                              gboolean changed);
G_GNUC_INTERNAL void gnoblin_control_pending_location_authorization_free(gpointer user_data);
G_GNUC_INTERNAL void gnoblin_control_clear_pending_location_authorizations(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_location_agent_create(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_privacy_new_handle(MetaRemoteAccessController* controller,
                                                        MetaRemoteAccessHandle* handle,
                                                        gpointer user_data);
G_GNUC_INTERNAL GVariant* gnoblin_control_stop_privacy_sessions(GnoblinNativeControl* control,
                                                                gboolean recording,
                                                                GError** error);

/* gnoblin-control-grants.c: portal grant cache, grant operations and session activity. */
G_GNUC_INTERNAL void gnoblin_control_portal_grants_watch(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_activity_watch(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_activity_publish(GnoblinNativeControl* control,
                                                      gboolean available, gboolean idle,
                                                      guint64 threshold_ms, guint64 idle_for_ms);

/* gnoblin-control-input-sources.c: XKB and IBus input sources. */
G_GNUC_INTERNAL void gnoblin_control_window_input_source_free(gpointer data);
G_GNUC_INTERNAL GVariant* gnoblin_control_input_source_snapshot(GnoblinNativeControl* control);
G_GNUC_INTERNAL NativeInputSource* gnoblin_control_current_input_source(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL gboolean gnoblin_control_input_sources_refresh(GnoblinNativeControl* control,
                                                               GVariant* document);
G_GNUC_INTERNAL void gnoblin_control_input_sources_release(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_input_sources_start_ibus(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_input_source_focus_changed(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_input_source_restore_focused(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_publish_input_source_changes(GnoblinNativeControl* control,
                                                                  guint64 revision);
G_GNUC_INTERNAL gboolean gnoblin_control_select_xkb_source(GnoblinNativeControl* control,
                                                           const char* id, gint64 request_id,
                                                           const char* method,
                                                           gboolean internal_restore,
                                                           GError** error);

/* gnoblin-control-command.c: command spawning. */
G_GNUC_INTERNAL gboolean gnoblin_control_command_spawn(char** argv, GPid* pid, GError** error);
G_GNUC_INTERNAL void gnoblin_control_command_reap(GPid pid, gint status, gpointer user_data);
G_GNUC_INTERNAL gboolean gnoblin_control_queue_command(const char* category, const char* name,
                                                       char** argv, GError** error);
G_GNUC_INTERNAL void gnoblin_control_launch_command(const char* category, const char* name,
                                                    char** argv);
G_GNUC_INTERNAL char** gnoblin_control_command_run_argv(GVariant* arguments, GError** error);

/* Helpers in gnoblin-native-control.c that the workspace and monitor domain calls. */
G_GNUC_INTERNAL gboolean gnoblin_control_monitor_property_changed(JsonObject* previous,
                                                                  JsonObject* current,
                                                                  const char* property);
G_GNUC_INTERNAL gboolean gnoblin_control_workspace_signal_subscribed(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_clear_object_signal_watches(GObject* object,
                                                                 GArray* handler_ids);
G_GNUC_INTERNAL void gnoblin_control_refresh_object_signal_watches(GnoblinNativeControl* control,
                                                                   GObject* object,
                                                                   const char* source,
                                                                   GArray* handler_ids);

/* gnoblin-control-workspaces.c: workspace and monitor snapshots, events and signal watches. */
G_GNUC_INTERNAL JsonNode* gnoblin_control_workspace_snapshot_json(GnoblinNativeControl* control,
                                                                  GError** error);
G_GNUC_INTERNAL JsonNode* gnoblin_control_monitor_snapshot_json(GnoblinNativeControl* control,
                                                                gboolean update_lua_snapshot,
                                                                GError** error);
G_GNUC_INTERNAL GHashTable* gnoblin_control_workspace_state_from_snapshot(JsonNode* snapshot);
G_GNUC_INTERNAL GHashTable* gnoblin_control_monitor_state_from_snapshot(JsonNode* snapshot);
G_GNUC_INTERNAL char* gnoblin_control_monitor_id_for_index(JsonNode* snapshot, int monitor_index);
G_GNUC_INTERNAL void gnoblin_control_publish_workspace_changes(GnoblinNativeControl* control,
                                                               JsonNode* snapshot,
                                                               JsonNode* window_snapshot,
                                                               guint64 revision);
G_GNUC_INTERNAL gboolean gnoblin_control_publish_monitor_changes(GnoblinNativeControl* control,
                                                                 JsonNode* snapshot,
                                                                 guint64 revision);
G_GNUC_INTERNAL void gnoblin_control_clear_workspace_manager_signal_watches(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_refresh_workspace_manager_signal_watches(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_clear_all_workspace_signal_watches(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_refresh_all_workspace_signal_watches(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_clear_monitor_manager_signal_watches(
    GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_refresh_monitor_manager_signal_watches(
    GnoblinNativeControl* control);

/* gnoblin-control-layers.c: layer snapshots, layer events and input-device change events. */
G_GNUC_INTERNAL void gnoblin_control_layer_state_free(gpointer data);
G_GNUC_INTERNAL JsonNode* gnoblin_control_layer_snapshot_json(GnoblinNativeControl* control,
                                                              gboolean update_lua_snapshot,
                                                              GError** error);
G_GNUC_INTERNAL GHashTable* gnoblin_control_layer_state_from_snapshot(JsonNode* snapshot);
G_GNUC_INTERNAL GHashTable* gnoblin_control_input_device_state_from_snapshot(JsonNode* snapshot);
G_GNUC_INTERNAL JsonArray* gnoblin_control_changed_layer_properties(JsonNode* previous,
                                                                    JsonNode* current);
G_GNUC_INTERNAL void gnoblin_control_publish_input_device_changes(GnoblinNativeControl* control,
                                                                  JsonNode* snapshot,
                                                                  guint64 revision);

/* gnoblin-control-shortcuts.c: dynamic shortcut bindings and bare-Super arming. */
G_GNUC_INTERNAL char* gnoblin_control_encode_response(const char* id, JsonNode* result,
                                                      const char* message);
G_GNUC_INTERNAL void gnoblin_control_dynamic_shortcut_end_session(GnoblinNativeControl* control,
                                                                  NativeDynamicShortcut* shortcut,
                                                                  const char* reason);
G_GNUC_INTERNAL void gnoblin_control_dynamic_shortcut_free(gpointer data);
G_GNUC_INTERNAL guint gnoblin_control_dynamic_shortcut_count(GnoblinNativeControl* control,
                                                             Client* owner);
G_GNUC_INTERNAL NativeDynamicShortcut* gnoblin_control_find_dynamic_shortcut(Client* owner,
                                                                             const char* id);
G_GNUC_INTERNAL NativeDynamicShortcut* gnoblin_control_find_runtime_dynamic_shortcut(
    GnoblinNativeControl* control, const char* owner_id, const char* id);
G_GNUC_INTERNAL void gnoblin_control_remove_dynamic_shortcut(GnoblinNativeControl* control,
                                                             NativeDynamicShortcut* shortcut);
G_GNUC_INTERNAL void gnoblin_control_clear_client_dynamic_shortcuts(Client* client);
G_GNUC_INTERNAL void gnoblin_control_clear_runtime_dynamic_shortcuts(
    GnoblinNativeControl* control, const char* reason);
G_GNUC_INTERNAL void gnoblin_control_clear_configured_capture_shortcut(
    GnoblinNativeControl* control, const char* reason);
G_GNUC_INTERNAL gboolean gnoblin_control_dynamic_shortcut_id_valid(const char* id);
G_GNUC_INTERNAL gboolean gnoblin_control_dynamic_shortcut_accelerator_valid(
    const char* accelerator);
G_GNUC_INTERNAL NativeDynamicShortcut* gnoblin_control_arm_bare_super_shortcut(
    GnoblinNativeControl* control, const char* id, const char* owner_id, Client* client,
    guint64 session_id, GError** error);
G_GNUC_INTERNAL char* gnoblin_control_dynamic_shortcut_bind(Client* client, const char* request_id,
                                                            JsonObject* arguments);
G_GNUC_INTERNAL char* gnoblin_control_dynamic_shortcut_unbind(Client* client,
                                                              const char* request_id,
                                                              JsonObject* arguments);
G_GNUC_INTERNAL char* gnoblin_control_dynamic_shortcut_request_end_session(
    Client* client, const char* request_id, JsonObject* arguments);

/* gnoblin-control-window-rules.c: window rules, frame style, shadows, shaders and the corner
 * toolkit probe. */
G_GNUC_INTERNAL void gnoblin_control_apply_window_rules(GnoblinNativeControl* control,
                                                        MetaWindow* window);
G_GNUC_INTERNAL void gnoblin_control_refresh_window_rule_blur(MetaWindow* window);
G_GNUC_INTERNAL void gnoblin_control_apply_all_window_rules(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_place_window_by_rules(GnoblinNativeControl* control,
                                                           MetaWindow* window);
G_GNUC_INTERNAL void gnoblin_control_corner_toolkit_cache_shutdown(GnoblinNativeControl* control);
G_GNUC_INTERNAL void gnoblin_control_window_shader_file_free(gpointer data);

/* gnoblin-control-window-drag.c: window drag state, drag events and snap offers. */
G_GNUC_INTERNAL void gnoblin_control_window_drag_free(gpointer data);
G_GNUC_INTERNAL void gnoblin_control_cancel_window_drags(GnoblinNativeControl* control,
                                                         const char* reason);
G_GNUC_INTERNAL void gnoblin_control_window_drag_client_disconnected(
    GnoblinNativeControl* control, guint64 client_id);
G_GNUC_INTERNAL gboolean gnoblin_control_drag_monitor_snapshot(GnoblinNativeControl* control,
                                                               MetaWindow* window,
                                                               char** monitor_id,
                                                               MtkRectangle* monitor_rect,
                                                               MtkRectangle* work_area);
G_GNUC_INTERNAL gboolean gnoblin_control_window_drag_refresh(GnoblinNativeControl* control,
                                                             NativeWindowDrag* drag,
                                                             MetaWindow* window, int pointer_x,
                                                             int pointer_y, guint32 modifiers);
G_GNUC_INTERNAL gboolean gnoblin_control_rect_contains(const MtkRectangle* outer,
                                                       const MtkRectangle* inner);
G_GNUC_INTERNAL gboolean gnoblin_control_variant_rect(GVariant* value, MtkRectangle* rect);
G_GNUC_INTERNAL gboolean gnoblin_control_snap_offer_set_targets(NativeWindowDrag* drag,
                                                                GVariant* targets,
                                                                GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_window_snap_offer(GnoblinNativeControl* control,
                                                                   GVariant* arguments,
                                                                   guint64 owner_generation,
                                                                   GError** error);

/* gnoblin-control-runtime.c: runtime worker protocol helpers (resume, reload documents, config
 * results, queue flushing). The send path itself stays in gnoblin-native-control.c. */
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_send(GnoblinNativeControl* control,
                                                      GnoblinRuntimePacketType type,
                                                      guint64 request_id, GVariant* payload,
                                                      GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_config_values_equal(GVariant* left,
                                                                     GVariant* right);
G_GNUC_INTERNAL gboolean gnoblin_control_xwayland_server_flags_changed(GVariant* current,
                                                                       GVariant* candidate);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_reload_documents_valid(GVariant* current,
                                                                        GVariant* candidate);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_send_config_result(
    GnoblinNativeControl* control, guint64 transaction_id, gboolean accepted, guint64 revision,
    guint64 generation, const char* message, GError** error);
G_GNUC_INTERNAL void gnoblin_control_runtime_clear_queue(GQueue* queue);
G_GNUC_INTERNAL GHashTable* gnoblin_control_runtime_event_subscriptions_from_payload(
    GVariant* payload, GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_event_subscriptions_equal(GHashTable* left,
                                                                           GHashTable* right);
G_GNUC_INTERNAL void gnoblin_control_runtime_add_display_environment(GVariantBuilder* hello);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_resume_snapshot_matches(
    GnoblinNativeControl* control, GVariant* payload);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_reject_resume(GnoblinNativeControl* control,
                                                               const char* message,
                                                               GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_flush_state_snapshots(
    GnoblinNativeControl* control, GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_flush_pending_events(
    GnoblinNativeControl* control, GError** error);
G_GNUC_INTERNAL gboolean gnoblin_control_runtime_send_worker_suspended(
    GnoblinNativeControl* control, GError** error);
