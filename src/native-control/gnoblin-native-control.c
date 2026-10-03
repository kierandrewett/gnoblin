/* The first native owner of compositor-v1: Lua-validated compositor methods. */
#include "config.h"

#include "core/gnoblin-native-control.h"
#include "core/gnoblin-location-agent.h"
#ifdef HAVE_REMOTE_DESKTOP
#include "core/gnoblin-pipewire-monitor.h"
#endif
#include "core/gnoblin-runtime-cache.h"
#include "core/gnoblin-runtime-protocol.h"
#include "core/gnoblin-touchpad-router.h"

#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib/gqsort.h>
#include <glib/gstdio.h>
#include <glib-unix.h>
#include <json-glib/json-glib.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <string.h>
#include <sys/random.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <poll.h>
#include <unistd.h>
#include <xkbcommon/xkbcommon.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-keymap-description-private.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-output.h"
#include "clutter/clutter.h"
#include "compositor/meta-window-actor-private.h"
#include "compositor/meta-window-actor-x11.h"
#include "compositor/meta-window-actor-wayland.h"
#include "compositor/meta-gnoblin-window-effects.h"
#include "core/display-private.h"
#include "core/events.h"
#include "core/util-private.h"
#include "core/window-private.h"
#include "wayland/gnoblin-portal-policy.h"
#include "wayland/gnoblin-lua-pattern.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-layer-shell.h"
#include "wayland/meta-wayland-activation.h"
#include "wayland/meta-wayland-seat.h"
#include "wayland/meta-wayland-text-input.h"
#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-cursor-tracker.h"
#include "meta/meta-context.h"
#include "meta/meta-logical-monitor.h"
#include "meta/meta-monitor.h"
#include "meta/meta-wayland-compositor.h"
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
#define NATIVE_CONTROL_OBJECT_DATA_KEY "gnoblin-native-control"
#define MAX_LAUNCHES 64
#define MAX_FOCUS_CONTEXTS 128
#define MAX_MENU_CONTEXTS 256
#define MAX_TEXT_TARGETS 128
#define MAX_SNAP_CONTEXTS 128
#define MAX_WINDOW_DRAG_CLIENTS 256
#define MAX_DYNAMIC_SHORTCUTS_PER_CLIENT 32
#define MAX_DYNAMIC_SHORTCUTS 128
#define MAX_PENDING_THUMBNAILS 4
#define MAX_PENDING_THUMBNAILS_PER_CLIENT 1
#define MAX_PENDING_LOCATION_AUTHORIZATIONS 32
#define LOCATION_AUTHORIZATION_TIMEOUT_SECONDS 25
#define MAX_CORNER_TOOLKIT_CACHE_ENTRIES 256
#define MAX_CORNER_TOOLKIT_MAPS_BYTES (4 * 1024 * 1024)
#define FOCUS_CONTEXT_LIFETIME_US (5 * G_USEC_PER_SEC)
#define PORTAL_GRANT_TIMEOUT_MS 5000
#define PORTAL_BACKEND_BUS_NAME "org.freedesktop.impl.portal.desktop.gnoblin"
#define NATIVE_POLICY_BUS_NAME "org.gnoblin.Compositor"
#define NATIVE_POLICY_OBJECT_PATH "/org/gnoblin/Compositor"
#define NATIVE_POLICY_INTERFACE "org.gnoblin.Compositor"
#define IBUS_BUS_NAME "org.freedesktop.IBus"
#define IBUS_OBJECT_PATH "/org/freedesktop/IBus"
#define IBUS_INTERFACE "org.freedesktop.IBus"

typedef struct _NativeDynamicShortcut NativeDynamicShortcut;
typedef struct _NativeMutterSignalWatch NativeMutterSignalWatch;
typedef struct _NativeCornerToolkitCache NativeCornerToolkitCache;
typedef struct _NativeWindowShaderFile NativeWindowShaderFile;

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

struct _NativeCornerToolkitCache {
    gint ref_count;
    GnoblinNativeControl* control;
    GHashTable* entries;
    guint pending_count;
};

struct _NativeMutterSignalWatch {
    GnoblinNativeControl* control;
    char* event;
    char* source;
    char* signal;
    guint signal_id;
};

struct _NativeWindowShaderFile {
    GnoblinNativeControl* control;
    char* path;
    char* source;
    GFileMonitor* monitor;
    guint reload_timeout_id;
};

struct _GnoblinNativeControl {
    GSocketService* service;
    GDBusConnection* session_bus;
    GHashTable* clients;
    GHashTable* windows;
    GHashTable* window_state;
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
    GHashTable* dynamic_shortcuts;
    NativeDynamicShortcut* bare_super_shortcut;
    NativeDynamicShortcut* active_shortcut_session;
    GPtrArray* launches;
    GHashTable* focus_contexts;
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
    guint64 next_input_device_id;
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
    guint portal_grant_retry_count;
    guint64 session_lock_revision;
    guint64 event_sequence;
    guint64 next_focus_context_handle;
    guint64 next_menu_context_handle;
    guint64 next_text_target_handle;
    guint64 next_shortcut_session_id;
    guint64 next_window_drag_id;
    guint64 next_snap_context_id;
    guint64 next_client_id;
    guint64 next_location_request_id;
    gboolean window_state_initialized;
    gboolean workspace_state_initialized;
    gboolean monitor_state_initialized;
    gboolean input_device_state_initialized;
    gboolean input_source_state_initialized;
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

/* Initial native snapshot is available before MetaContext creates the display,
 * so Mutter preferences/workspace setup can read it without starting Lua. */
static GnoblinRuntimeCache* bootstrap_runtime_cache;
static guint64 bootstrap_runtime_generation;
static void native_cancel_window_drags(GnoblinNativeControl* control, const char* reason);
static void revoke_focus_contexts(GnoblinNativeControl* control);
static void revoke_menu_contexts(GnoblinNativeControl* control);
static void revoke_text_targets(GnoblinNativeControl* control);
static void native_runtime_fail_pending_requests(GnoblinNativeControl* control, const char* reason);
static void clear_runtime_dynamic_shortcuts(GnoblinNativeControl* control, const char* reason);
static void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete,
                                         gboolean ok, const char* accelerator,
                                         const char* error_code, const char* message);
static void dynamic_shortcut_end_session(GnoblinNativeControl* control,
                                         NativeDynamicShortcut* shortcut, const char* reason);
static void native_corner_toolkit_cache_shutdown(GnoblinNativeControl* control);
static void clear_pending_location_authorizations(GnoblinNativeControl* control);
static void start_ibus_input_source_tracking(GnoblinNativeControl* control);

static void native_runtime_abort(GnoblinNativeControl* control) {
    if (!control)
        return;
    control->stopping = TRUE;
    clear_pending_location_authorizations(control);
    native_corner_toolkit_cache_shutdown(control);
    if (control->runtime_read_source_id) {
        g_source_remove(control->runtime_read_source_id);
        control->runtime_read_source_id = 0;
    }
    if (control->runtime_write_source_id) {
        g_source_remove(control->runtime_write_source_id);
        control->runtime_write_source_id = 0;
    }
    native_runtime_fail_pending_requests(control, "Lua runtime stopped before replying");
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session, "runtime_stopped");
    clear_runtime_dynamic_shortcuts(control, "runtime_stopped");
    g_clear_pointer(&control->window_shader_files, g_hash_table_unref);
    stop_native_shortcut_capture(control, FALSE, FALSE, NULL, NULL, NULL);
    native_cancel_window_drags(control, "runtime_stopped");
    revoke_text_targets(control);
    revoke_focus_contexts(control);
    revoke_menu_contexts(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
}

static gboolean native_runtime_send(GnoblinNativeControl* control, GnoblinRuntimePacketType type,
                                    guint64 request_id, GVariant* payload, GError** error);
static gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                              GVariant* payload);
static gboolean native_runtime_flush_state_snapshots(GnoblinNativeControl* control, GError** error);
static gboolean native_runtime_flush_pending_events(GnoblinNativeControl* control, GError** error);
static void native_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                            GVariant* snapshot, guint64 revision);
static void native_activity_publish(GnoblinNativeControl* control, gboolean available,
                                    gboolean idle, guint64 threshold_ms, guint64 idle_for_ms);
static void native_activity_query(GnoblinNativeControl* control);
static void native_apply_window_rules(GnoblinNativeControl* control, MetaWindow* window);
static void native_apply_all_window_rules(GnoblinNativeControl* control);

/* The supervised compositor may consume configuration only from the immutable
 * snapshot transferred by its Lua-owning parent. Keep legacy reads confined to
 * the unsupervised compatibility path. */
static GVariant* native_config_document(GnoblinNativeControl* control) {
    if (control && control->supervised_runtime)
        return gnoblin_runtime_cache_get_document(control->runtime_cache);
    if (!control && bootstrap_runtime_cache)
        return gnoblin_runtime_cache_get_document(bootstrap_runtime_cache);
    return NULL;
}

static guint64 native_config_revision(GnoblinNativeControl* control) {
    if (control && control->supervised_runtime)
        return gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
    if (!control && bootstrap_runtime_cache)
        return gnoblin_runtime_cache_get_settings_revision(bootstrap_runtime_cache);
    return 0;
}

GVariant* gnoblin_native_control_get_config_document(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    return native_config_document(control);
}

guint64 gnoblin_native_control_get_config_revision(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    return native_config_revision(control);
}

gboolean gnoblin_native_control_get_config_bool(MetaDisplay* display, const char* section,
                                                const char* key, gboolean fallback) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    GnoblinRuntimeCache* cache =
        control && control->supervised_runtime ? control->runtime_cache : bootstrap_runtime_cache;
    return gnoblin_runtime_cache_get_bool(cache, section, key, fallback);
}

gboolean gnoblin_native_control_protocol_enabled(const char* protocol) {
    g_return_val_if_fail(protocol != NULL && *protocol != '\0', FALSE);
    if (!gnoblin_native_control_is_session(NULL))
        return FALSE;
    return gnoblin_native_control_get_config_bool(NULL, "protocols", protocol, TRUE);
}

static guint64 native_config_generation(GnoblinNativeControl* control) {
    return native_config_revision(control);
}

static void native_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                            GVariant* snapshot, guint64 revision) {
    if (!control || control->stopping || control->runtime_worker_suspended)
        return;
    if (!control->supervised_runtime) {
        g_warning("gnoblin-native-control: refusing state snapshot without the Lua supervisor");
        return;
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state_version", g_variant_new_uint32(1));
    g_variant_builder_add(&builder, "{sv}", "name", g_variant_new_string(name));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "available", g_variant_new_boolean(snapshot != NULL));
    if (snapshot)
        g_variant_builder_add(&builder, "{sv}", "snapshot", snapshot);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    if (!control->runtime_hello_sent) {
        g_queue_push_tail(control->pending_runtime_states, g_variant_ref(payload));
        return;
    }
    g_autoptr(GError) error = NULL;
    if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_STATE, 0, payload, &error)) {
        native_runtime_abort(control);
        g_warning("gnoblin-native-control: cannot send state snapshot to Lua supervisor: %s",
                  error ? error->message : "unknown error");
    }
}

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
} Client;

typedef struct {
    Client* client;
    char* request_id;
    char* legacy_window_action;
    char* legacy_window_id;
} PendingRuntimeRequest;

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
    GnoblinNativeControl* control;
    MetaKeymapDescription* description;
    GPtrArray* source_ids;
    char* selected_type;
    char* selected_id;
    gint64 request_id;
    char* method;
    guint group;
    guint64 ibus_owner_generation;
} PendingInputSource;

typedef struct {
    GnoblinNativeControl* control;
    guint64 ibus_owner_generation;
    guint64 ibus_engine_generation;
} PendingIBusQuery;

typedef struct {
    GnoblinNativeControl* control;
    gint64 operation_id;
    guint64 runtime_generation;
    char* method;
    char* kind;
    char* id;
    guint64 expected_created_at_ms;
    gboolean has_expected_created_at;
} PendingPortalGrantOperation;

typedef struct {
    GnoblinNativeControl* control;
    guint64 cache_revision;
} PendingPortalGrantSnapshot;

typedef struct {
    const char* id;
    const char* description;
} NativeCapability;

static void schedule_windows(GnoblinNativeControl* control);
static JsonNode* json_from_variant(GVariant* value);
static void send_response(Client* client, char* response);
static void process_buffer(Client* client);
static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload);
static void native_socket_revoke_client_tokens(Client* client);
static GVariant* native_socket_snap_rect(JsonNode* node);
static void prune_focus_contexts(GnoblinNativeControl* control, gint64 now);
static gboolean focus_context_expiry_tick(gpointer user_data);
static gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data);
static void publish_shortcut_focus_event(GnoblinNativeControl* control, const char* shortcut,
                                         guint64 handle, guint64 generation, gint64 expires_at_us);
static void publish_input_source_changes(GnoblinNativeControl* control, guint64 revision);
static GVariant* native_orientation_lock_snapshot(GnoblinNativeControl* control, guint64 revision);
static void native_orientation_lock_publish(GnoblinNativeControl* control);
static void update_launch_snapshot(GnoblinNativeControl* control);
static void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                                const ClutterEvent* event);
static gboolean native_shortcut_session_timeout(gpointer user_data);
static void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data);
static gboolean native_shortcut_hold_family_pressed(GnoblinNativeControl* control,
                                                    guint32 hold_mask);
static void dispatch_dynamic_shortcut_repeat(GnoblinNativeControl* control, guint action,
                                             const ClutterEvent* event);
static void native_control_maybe_free_stopped(GnoblinNativeControl* control);
static const char* native_operation_error_code(const GError* error);
static void dispatch_operation_completion_full(GnoblinNativeControl* control, gint64 request_id,
                                               const char* method, gboolean ok, JsonNode* result,
                                               const char* error_code, const char* message,
                                               gboolean dispatch_lua);

static Client* native_client_by_id(GnoblinNativeControl* control, guint64 client_id) {
    if (!control || !control->clients || !client_id)
        return NULL;
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->clients);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        Client* client = key;
        if (client->client_id == client_id && !client->closing)
            return client;
    }
    return NULL;
}

static MetaWindow* native_window_by_stable_id(GnoblinNativeControl* control,
                                              const char* stable_id) {
    if (!control || !stable_id || !*stable_id)
        return NULL;
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    MetaWindow* match = NULL;
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        g_autofree char* id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        if (g_str_equal(id, stable_id)) {
            match = window;
            break;
        }
    }
    g_slist_free(windows);
    return match;
}

GVariant* gnoblin_native_control_restore_or_minimize_window(MetaDisplay* display,
                                                            GVariant* arguments, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* id = NULL;
    if (!control || control->stopping || !control->snap_restore_frames ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1 || !g_variant_lookup(arguments, "id", "&s", &id) ||
        !id || !*id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.restore_or_minimize requires only a stable window id");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "window.restore_or_minimize is unavailable while the session is locked");
        return NULL;
    }
    MetaWindow* window = native_window_by_stable_id(control, id);
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window.restore_or_minimize target is no longer available");
        return NULL;
    }
    const char* action = NULL;
    if (meta_window_get_maximize_flags(window) != 0) {
        meta_window_set_unmaximize_flags(window, META_MAXIMIZE_BOTH);
        g_hash_table_remove(control->snap_restore_frames, window);
        action = "unmaximize";
    } else {
        MtkRectangle* frame = g_hash_table_lookup(control->snap_restore_frames, window);
        if (frame) {
            MtkRectangle restore_frame = *frame;
            g_hash_table_remove(control->snap_restore_frames, window);
            meta_window_move_resize_frame(window, TRUE, restore_frame.x, restore_frame.y,
                                          restore_frame.width, restore_frame.height);
            action = "restore";
        } else if (meta_window_can_minimize(window)) {
            meta_window_minimize(window);
            action = "minimize";
        } else {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "window cannot be minimized and has no saved snap frame");
            return NULL;
        }
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "action", g_variant_new_string(action));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void pending_thumbnail_free(PendingThumbnail* pending) {
    if (!pending)
        return;
    g_free(pending->window_id);
    g_free(pending);
}

static gboolean native_thumbnail_idle(gpointer user_data) {
    PendingThumbnail* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    pending->source_id = 0;
    Client* owner = native_client_by_id(control, pending->client_id);
    if (owner && owner->pending_thumbnails)
        owner->pending_thumbnails--;
    if (control->pending_thumbnail_count)
        control->pending_thumbnail_count--;

    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) png = NULL;
    g_autoptr(JsonNode) result = NULL;
    int actual_width = 0;
    int actual_height = 0;
    if (control->stopping || (pending->client_id && !owner)) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                            "thumbnail requester disconnected");
    } else if (control->wayland_compositor &&
               meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
    } else {
        MetaWindow* window = native_window_by_stable_id(control, pending->window_id);
        if (!window) {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "thumbnail window no longer exists");
        } else {
            MetaWindowActor* actor = meta_window_actor_from_window(window);
            if (!actor) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "thumbnail window has no compositor actor");
            } else {
                png = meta_window_actor_paint_to_png(actor, pending->width, pending->height,
                                                     &actual_width, &actual_height, &error);
                /* The window must still resolve to the same stable sequence after capture. */
                if (png && !native_window_by_stable_id(control, pending->window_id)) {
                    g_clear_pointer(&png, g_bytes_unref);
                    g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                        "thumbnail window closed during capture");
                }
            }
        }
    }

    if (png && !(control->wayland_compositor &&
                 meta_wayland_session_lock_is_active(control->wayland_compositor))) {
        gsize length = 0;
        const guchar* bytes = g_bytes_get_data(png, &length);
        if (length > 512 * 1024 || actual_width < 1 || actual_width > 480 || actual_height < 1 ||
            actual_height > 320) {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "thumbnail output exceeded the requested bounds");
        } else {
            g_autofree char* encoded = g_base64_encode(bytes, length);
            JsonObject* object = json_object_new();
            json_object_set_string_member(object, "window_id", pending->window_id);
            json_object_set_int_member(object, "width", actual_width);
            json_object_set_int_member(object, "height", actual_height);
            json_object_set_string_member(object, "data", encoded);
            result = json_node_new(JSON_NODE_OBJECT);
            json_node_take_object(result, object);
        }
    } else if (png) {
        g_clear_pointer(&png, g_bytes_unref);
        g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
    }

    if (!control->stopping)
        dispatch_operation_completion_full(
            control, pending->operation_id, "window.thumbnail", result != NULL, result,
            native_operation_error_code(error), error ? error->message : NULL, FALSE);
    pending_thumbnail_free(pending);
    native_control_maybe_free_stopped(control);
    return G_SOURCE_REMOVE;
}

static gboolean native_runtime_begin_thumbnail(GnoblinNativeControl* control, gint64 operation_id,
                                               GVariant* arguments, guint64 client_id,
                                               GError** error) {
    const char* window_id = NULL;
    gint64 width = 0;
    gint64 height = 0;
    if (!g_variant_lookup(arguments, "id", "&s", &window_id) || !window_id || !*window_id ||
        !g_variant_lookup(arguments, "width", "x", &width) ||
        !g_variant_lookup(arguments, "height", "x", &height) || width < 1 || width > 480 ||
        height < 1 || height > 320) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.thumbnail requires an id and dimensions up to 480x320");
        return FALSE;
    }
    Client* owner = native_client_by_id(control, client_id);
    if (client_id && !owner) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CANCELLED,
                            "thumbnail requester disconnected");
        return FALSE;
    }
    if ((owner && owner->pending_thumbnails >= MAX_PENDING_THUMBNAILS_PER_CLIENT) ||
        control->pending_thumbnail_count >= MAX_PENDING_THUMBNAILS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY, "thumbnail capture limit reached");
        return FALSE;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window thumbnails are unavailable while the session is locked");
        return FALSE;
    }
    if (!native_window_by_stable_id(control, window_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "thumbnail window no longer exists");
        return FALSE;
    }

    if (owner)
        owner->pending_thumbnails++;
    control->pending_thumbnail_count++;
    guint64* operation_key = g_new(guint64, 1);
    *operation_key = (guint64)operation_id;
    g_hash_table_add(control->runtime_operation_ids, operation_key);

    PendingThumbnail* pending = g_new0(PendingThumbnail, 1);
    pending->control = control;
    pending->client_id = client_id;
    pending->operation_id = operation_id;
    pending->window_id = g_strdup(window_id);
    pending->width = (guint)width;
    pending->height = (guint)height;
    pending->source_id = g_idle_add_full(G_PRIORITY_LOW, native_thumbnail_idle, pending, NULL);
    return TRUE;
}

static const NativeCapability native_capabilities[] = {
    {"layer-list", "List layer-shell surfaces and their placement."},
    {"monitor-list", "List active logical monitors and their geometry."},
    {"window-list", "Read managed windows and their compositor state."},
    {"window-actions", "Request supported window actions."},
    {"typed-window-operations", "Set window state, geometry, workspace, or monitor."},
    {"window-interactive-grabs",
     "Start a keyboard move or resize with a trusted shortcut context."},
    {"window-thumbnails", "Capture bounded compositor-rendered window previews."},
    {"window-drag-snap-offers",
     "Receive pointer-drag state and submit capability-bound snap regions."},
    {"session-activity", "Read session idle/active state and receive activity changes."},
    {"workspace-management", "List and manage native workspaces."},
    {"window-change-events", "Receive changes to managed window properties."},
    {"window-lifecycle-events", "Receive window creation, focus, attention, and close events."},
    {"shell-request-events", "Receive window-menu and compositor OSD requests for shell UI."},
    {"workspace-lifecycle-events", "Receive workspace creation and state-change events."},
    {"monitor-lifecycle-events", "Receive active monitor addition, change, and removal events."},
    {"input-device-list", "Read detected input devices and their reported capabilities."},
    {"input-device-lifecycle-events", "Receive input-device addition and removal events."},
    {"input-source-list", "List configured XKB input sources and read the confirmed source."},
    {"input-source-selection", "Select configured XKB input sources."},
    {"input-source-lifecycle-events", "Receive changes to the active XKB input source."},
    {"launch-feedback", "Track launch feedback against newly mapped or newly focused windows."},
    {"shortcut-capture", "Capture one normalized keyboard accelerator."},
    {"generic-event-subscriptions", "Subscribe a socket connection to selected native events."},
    {"input-gesture-events", "Receive stable touchpad gesture events."},
    {"shortcut-focus-contexts",
     "Receive a one-use focus grant for a trusted configured command shortcut."},
    {"dynamic-shortcuts", "Register connection-owned momentary global shortcut bindings."},
    {"portal-grants", "Read and revoke Gnoblin portal permission grants."},
    {"permission-policy", "Read the committed portal permission policy."},
    {"animation-preview", "Inspect configured animations and control compositor previews."},
    {"microphone-monitor", "Monitor microphone activity through PipeWire."},
    {"camera-monitor", "Monitor camera activity through PipeWire."},
    {"location-agent", "Monitor GeoClue activity and broker location authorization to Lua."},
};

static const char* native_socket_events[] = {
    "windows",
    "gnoblin.window.created",
    "gnoblin.window.changed",
    "gnoblin.window.focused",
    "gnoblin.window.unfocused",
    "gnoblin.window.attention-changed",
    "gnoblin.window.activation-denied",
    "gnoblin.window.closed",
    "gnoblin.window.drag.started",
    "gnoblin.window.drag.updated",
    "gnoblin.window.drag.ended",
    "gnoblin.window.menu-requested",
    "gnoblin.osd.requested",
    "gnoblin.focus.policy-changed",
    "gnoblin.permission.changed",
    "gnoblin.config.reloaded",
    "gnoblin.config.reload-failed",
    "gnoblin.capability.changed",
    "gnoblin.appearance.color-scheme-changed",
    "gnoblin.privacy.changed",
    "gnoblin.location.authorization-requested",
    "gnoblin.animation.started",
    "gnoblin.animation.finished",
    "gnoblin.workspace.created",
    "gnoblin.workspace.renamed",
    "gnoblin.workspace.changed",
    "gnoblin.workspace.removed",
    "gnoblin.workspace.activated",
    "gnoblin.workspace.window-moved",
    "monitors",
    "gnoblin.monitor.added",
    "gnoblin.monitor.changed",
    "gnoblin.monitor.removed",
    "gnoblin.input.device-added",
    "gnoblin.input.device-removed",
    "gnoblin.input.sources-changed",
    "gnoblin.input.source-changed",
    "gnoblin.input.orientation-lock-changed",
    "gnoblin.input.gesture",
    "gnoblin.launch.changed",
    "gnoblin.session.lock-requested",
    "gnoblin.session.lock-state-changed",
    "gnoblin.session.activity-changed",
    "gnoblin.shortcut.activated",
    "gnoblin.shortcut.binding-activated",
    "gnoblin.shortcut.binding-deactivated",
    "gnoblin.shortcut.session.activated",
    "gnoblin.shortcut.session.key",
    "gnoblin.shortcut.session.ended",
    "gnoblin.portal.grant-added",
    "gnoblin.portal.grant-removed",
    "gnoblin.api.operation-completed",
    "gnoblin.operation.completed",
    NULL,
};

static const char native_policy_introspection[] =
    "<node><interface name='org.gnoblin.Compositor'>"
    "<method name='CheckPermission'>"
    "<arg type='s' direction='in'/><arg type='s' direction='in'/>"
    "<arg type='s' direction='out'/><arg type='s' direction='out'/>"
    "<arg type='as' direction='out'/><arg type='u' direction='out'/>"
    "<arg type='b' direction='out'/>"
    "</method></interface></node>";

static gboolean native_policy_requester_is_portal_backend(GDBusConnection* connection,
                                                          const char* sender) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetNameOwner", g_variant_new("(s)", PORTAL_BACKEND_BUS_NAME), G_VARIANT_TYPE("(s)"),
        G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!owner)
        return FALSE;
    const char* unique_owner;
    g_variant_get(owner, "(&s)", &unique_owner);
    return g_strcmp0(unique_owner, sender) == 0;
}

static void native_policy_method_call(GDBusConnection* connection, const char* sender,
                                      const char* object_path, const char* interface_name,
                                      const char* method_name, GVariant* parameters,
                                      GDBusMethodInvocation* invocation, gpointer user_data) {
    (void)object_path;
    (void)interface_name;
    (void)method_name;
    GnoblinNativeControl* control = user_data;
    if (control->stopping || !native_policy_requester_is_portal_backend(connection, sender)) {
        g_dbus_method_invocation_return_dbus_error(
            invocation, "org.gnoblin.Compositor.Error.AccessDenied",
            "permission decisions are available only to the Gnoblin portal backend");
        return;
    }
    const char *capability, *identity;
    g_variant_get(parameters, "(&s&s)", &capability, &identity);
    if (!gnoblin_permission_capability_supported(capability)) {
        g_dbus_method_invocation_return_dbus_error(invocation,
                                                   "org.gnoblin.Compositor.Error.InvalidCapability",
                                                   "unsupported permission capability");
        return;
    }
    g_autoptr(GVariant) document = native_config_document(control);
    if (!document) {
        g_dbus_method_invocation_return_dbus_error(invocation,
                                                   "org.gnoblin.Compositor.Error.PolicyUnavailable",
                                                   "committed permission policy is unavailable");
        return;
    }
    g_auto(GnoblinPermission) decision =
        gnoblin_permission_policy_evaluate(document, capability, identity);
    static const char* const empty_monitors[] = {NULL};
    const char* const* monitors =
        decision.monitors ? (const char* const*)decision.monitors : empty_monitors;
    g_dbus_method_invocation_return_value(
        invocation,
        g_variant_new("(ss@asub)",
                      decision.level == GNOBLIN_PERMISSION_ASK     ? "ask"
                      : decision.level == GNOBLIN_PERMISSION_ALLOW ? "allow"
                      : decision.level == GNOBLIN_PERMISSION_DENY  ? "deny"
                                                                   : "default",
                      decision.rule ? decision.rule : "", g_variant_new_strv(monitors, -1),
                      decision.devices, decision.clipboard));
}

static const GDBusInterfaceVTable native_policy_vtable = {
    .method_call = native_policy_method_call,
};

static void portal_grants_watch_backend(GnoblinNativeControl* control);
static void portal_grant_fetch_snapshot(GnoblinNativeControl* control);

static gboolean portal_grant_retry_snapshot(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->portal_grant_retry_id = 0;
    portal_grant_fetch_snapshot(control);
    return G_SOURCE_REMOVE;
}

static void portal_grant_schedule_retry(GnoblinNativeControl* control) {
    if (!control || control->stopping || control->portal_grant_retry_id ||
        control->portal_grant_retry_count >= 3 || !control->portal_backend_available)
        return;
    guint delay_seconds = 1u << control->portal_grant_retry_count++;
    control->portal_grant_retry_id =
        g_timeout_add_seconds(delay_seconds, portal_grant_retry_snapshot, control);
}

static GVariant* portal_grant_record(const char* id, const char* kind, const char* requester,
                                     guint32 devices, gboolean clipboard, gboolean screen_streams,
                                     guint64 created_at_ms, guint64 revision) {
    static const struct {
        guint32 bit;
        const char* name;
    } device_names[] = {{1u, "keyboard"}, {2u, "pointer"}, {4u, "touchscreen"}};
    if (!id || !*id || !kind ||
        (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")) || !requester ||
        (devices & ~7u) != 0)
        return NULL;
    GVariantBuilder device_array;
    GVariantBuilder record;
    g_variant_builder_init(&device_array, G_VARIANT_TYPE_STRING_ARRAY);
    for (guint i = 0; i < G_N_ELEMENTS(device_names); i++)
        if (devices & device_names[i].bit)
            g_variant_builder_add(&device_array, "s", device_names[i].name);
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string(kind));
    g_variant_builder_add(&record, "{sv}", "requester", g_variant_new_string(requester));
    g_variant_builder_add(&record, "{sv}", "devices", g_variant_builder_end(&device_array));
    g_variant_builder_add(&record, "{sv}", "clipboard", g_variant_new_boolean(clipboard));
    g_variant_builder_add(&record, "{sv}", "has_screen_streams",
                          g_variant_new_boolean(screen_streams));
    g_variant_builder_add(&record, "{sv}", "created_at",
                          g_variant_new_int64((gint64)MIN(created_at_ms, (guint64)G_MAXINT64)));
    g_variant_builder_add(&record, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static GVariant* portal_grant_record_from_tuple(GVariant* tuple, guint64 revision) {
    const char* id;
    const char* kind;
    const char* requester;
    guint32 devices;
    gboolean clipboard;
    gboolean screen_streams;
    guint64 created_at_ms;
    if (!g_variant_is_of_type(tuple, G_VARIANT_TYPE("(sssubbt)")))
        return NULL;
    g_variant_get(tuple, "(&s&s&subbt)", &id, &kind, &requester, &devices, &clipboard,
                  &screen_streams, &created_at_ms);
    return portal_grant_record(id, kind, requester, devices, clipboard, screen_streams,
                               created_at_ms, revision);
}

static void portal_grant_cache_replace(GnoblinNativeControl* control, GVariant* grants) {
    if (!control || control->stopping)
        return;
    guint64 revision = ++control->portal_grant_revision;
    GVariantBuilder records;
    GVariantBuilder snapshot;
    g_variant_builder_init(&records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; grants && i < g_variant_n_children(grants); i++) {
        g_autoptr(GVariant) tuple = g_variant_get_child_value(grants, i);
        g_autoptr(GVariant) record = portal_grant_record_from_tuple(tuple, revision);
        if (record)
            g_variant_builder_add_value(&records, g_variant_ref(record));
    }
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "grants", g_variant_builder_end(&records));
    g_variant_builder_add(&snapshot, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    g_clear_pointer(&control->portal_grant_snapshot, g_variant_unref);
    control->portal_grant_snapshot = g_variant_ref_sink(g_variant_builder_end(&snapshot));
    native_publish_runtime_snapshot(control, "portal-grants", control->portal_grant_snapshot,
                                    revision);
}

static void portal_grant_cache_apply(GnoblinNativeControl* control, const char* changed_id,
                                     const char* changed_kind, GVariant* replacement) {
    g_autoptr(GVariant) current = control->portal_grant_snapshot
                                      ? g_variant_lookup_value(control->portal_grant_snapshot,
                                                               "grants", G_VARIANT_TYPE("aa{sv}"))
                                      : NULL;
    if (!current) {
        control->portal_grant_revision++;
        native_publish_runtime_snapshot(control, "portal-grants", NULL,
                                        control->portal_grant_revision);
        portal_grant_fetch_snapshot(control);
        return;
    }
    guint64 revision = ++control->portal_grant_revision;
    GVariantBuilder records;
    GVariantBuilder snapshot;
    gboolean replaced = FALSE;
    g_variant_builder_init(&records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; current && i < g_variant_n_children(current); i++) {
        g_autoptr(GVariant) old = g_variant_get_child_value(current, i);
        const char* id = NULL;
        const char* kind = NULL;
        g_variant_lookup(old, "id", "&s", &id);
        g_variant_lookup(old, "kind", "&s", &kind);
        if (g_strcmp0(id, changed_id) == 0 && g_strcmp0(kind, changed_kind) == 0) {
            if (replacement) {
                g_variant_builder_add_value(&records, g_variant_ref(replacement));
                replaced = TRUE;
            }
            continue;
        }
        GVariantBuilder revised;
        GVariantIter fields;
        const char* field_name;
        GVariant* field_value;
        g_variant_builder_init(&revised, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&fields, old);
        while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
            g_autoptr(GVariant) value = field_value;
            if (!g_str_equal(field_name, "revision"))
                g_variant_builder_add(&revised, "{sv}", field_name, g_variant_ref(value));
        }
        g_variant_builder_add(&revised, "{sv}", "revision", g_variant_new_int64((gint64)revision));
        g_variant_builder_add_value(&records, g_variant_builder_end(&revised));
    }
    if (replacement && !replaced)
        g_variant_builder_add_value(&records, g_variant_ref(replacement));
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "grants", g_variant_builder_end(&records));
    g_variant_builder_add(&snapshot, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    g_clear_pointer(&control->portal_grant_snapshot, g_variant_unref);
    control->portal_grant_snapshot = g_variant_ref_sink(g_variant_builder_end(&snapshot));
    native_publish_runtime_snapshot(control, "portal-grants", control->portal_grant_snapshot,
                                    revision);
}

static void publish_portal_grant_event(GnoblinNativeControl* control, const char* event,
                                       GVariant* payload) {
    if (!control || control->stopping || !payload)
        return;
    GVariantBuilder enriched;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_builder_init(&enriched, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&fields, payload);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) value = field_value;
        g_variant_builder_add(&enriched, "{sv}", field_name, g_variant_ref(value));
    }
    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    g_variant_builder_add(&enriched, "{sv}", "name", g_variant_new_string(event));
    g_variant_builder_add(&enriched, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->portal_grant_revision));
    g_variant_builder_add(&enriched, "{sv}", "sequence", g_variant_new_int64(sequence));
    g_variant_builder_add(&enriched, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) enriched_payload = g_variant_ref_sink(g_variant_builder_end(&enriched));
    native_runtime_dispatch_event(control, event, enriched_payload);
    g_autoptr(JsonNode) json = json_from_variant(enriched_payload);
    if (!JSON_NODE_HOLDS_OBJECT(json))
        return;
    publish_native_socket_event(control, json);
}

static void portal_grant_snapshot_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingPortalGrantSnapshot* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), result, &error);
    if (!control->stopping && pending->cache_revision != control->portal_grant_revision) {
        portal_grant_fetch_snapshot(control);
    } else if (!control->stopping && reply &&
               g_variant_is_of_type(reply, G_VARIANT_TYPE("(a(sssubbt))"))) {
        g_autoptr(GVariant) grants = NULL;
        g_variant_get(reply, "(@a(sssubbt))", &grants);
        portal_grant_cache_replace(control, grants);
        control->portal_grant_retry_count = 0;
        if (control->portal_grant_retry_id) {
            g_source_remove(control->portal_grant_retry_id);
            control->portal_grant_retry_id = 0;
        }
    } else if (!control->stopping) {
        if (error)
            g_debug("gnoblin-native-control: portal grant snapshot unavailable: %s",
                    error->message);
        portal_grant_schedule_retry(control);
    }
    control->pending_portal_grant_ops--;
    g_free(pending);
    native_control_maybe_free_stopped(control);
}

static void portal_grant_fetch_snapshot(GnoblinNativeControl* control) {
    if (!control || control->stopping || !control->session_bus ||
        !control->portal_backend_available)
        return;
    PendingPortalGrantSnapshot* pending = g_new0(PendingPortalGrantSnapshot, 1);
    pending->control = control;
    pending->cache_revision = control->portal_grant_revision;
    control->pending_portal_grant_ops++;
    g_dbus_connection_call(
        control->session_bus, PORTAL_BACKEND_BUS_NAME, "/org/gnoblin/Portal/Grants",
        "org.gnoblin.Portal.Grants", "ListPortalGrantDetails", NULL, G_VARIANT_TYPE("(a(sssubbt))"),
        G_DBUS_CALL_FLAGS_NONE, PORTAL_GRANT_TIMEOUT_MS, NULL, portal_grant_snapshot_done, pending);
}

static void portal_backend_signal(GDBusConnection* connection, const char* sender_name,
                                  const char* object_path, const char* interface_name,
                                  const char* signal_name, GVariant* parameters,
                                  gpointer user_data) {
    (void)connection;
    (void)sender_name;
    (void)object_path;
    (void)interface_name;
    GnoblinNativeControl* control = user_data;
    if (control->stopping)
        return;
    if (g_str_equal(signal_name, "GrantAdded")) {
        g_autoptr(GVariant) tuple = g_variant_get_child_value(parameters, 0);
        g_autoptr(GVariant) record =
            portal_grant_record_from_tuple(tuple, control->portal_grant_revision + 1);
        if (!record)
            return;
        const char* id = NULL;
        const char* kind = NULL;
        g_variant_lookup(record, "id", "&s", &id);
        g_variant_lookup(record, "kind", "&s", &kind);
        portal_grant_cache_apply(control, id, kind, record);
        GVariantBuilder payload;
        g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload, "{sv}", "grant", g_variant_ref(record));
        g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
        publish_portal_grant_event(control, "gnoblin.portal.grant-added", event);
    } else if (g_str_equal(signal_name, "GrantRemoved")) {
        const char* id;
        const char* kind;
        if (!g_variant_is_of_type(parameters, G_VARIANT_TYPE("(ss)")))
            return;
        g_variant_get(parameters, "(&s&s)", &id, &kind);
        if (!id || !*id ||
            (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")))
            return;
        portal_grant_cache_apply(control, id, kind, NULL);
        GVariantBuilder payload;
        g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload, "{sv}", "grant_id", g_variant_new_string(id));
        g_variant_builder_add(&payload, "{sv}", "kind", g_variant_new_string(kind));
        g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
        publish_portal_grant_event(control, "gnoblin.portal.grant-removed", event);
    }
}

static void portal_backend_owner_changed(GDBusConnection* connection, const char* sender_name,
                                         const char* object_path, const char* interface_name,
                                         const char* signal_name, GVariant* parameters,
                                         gpointer user_data) {
    (void)connection;
    (void)sender_name;
    (void)object_path;
    (void)interface_name;
    (void)signal_name;
    GnoblinNativeControl* control = user_data;
    const char* name;
    const char* old_owner;
    const char* new_owner;
    g_variant_get(parameters, "(&s&s&s)", &name, &old_owner, &new_owner);
    if (control->stopping || !g_str_equal(name, PORTAL_BACKEND_BUS_NAME))
        return;
    control->portal_backend_available = new_owner && *new_owner;
    if (!control->portal_backend_available) {
        if (control->portal_grant_retry_id) {
            g_source_remove(control->portal_grant_retry_id);
            control->portal_grant_retry_id = 0;
        }
        control->portal_grant_retry_count = 0;
        control->portal_grant_revision++;
        g_clear_pointer(&control->portal_grant_snapshot, g_variant_unref);
        native_publish_runtime_snapshot(control, "portal-grants", NULL,
                                        control->portal_grant_revision);
    } else if (!g_str_equal(old_owner, new_owner)) {
        control->portal_grant_retry_count = 0;
        portal_grant_fetch_snapshot(control);
    }
}

#define SESSION_ACTIVITY_BUS_NAME "org.freedesktop.ScreenSaver"
#define SESSION_ACTIVITY_PATH "/org/gnoblin/SessionIdle"
#define SESSION_ACTIVITY_INTERFACE "org.gnoblin.SessionIdle"

static void native_activity_publish(GnoblinNativeControl* control, gboolean available,
                                    gboolean idle, guint64 threshold_ms, guint64 idle_for_ms) {
    if (!control || control->stopping)
        return;
    if (!available) {
        idle = FALSE;
        idle_for_ms = 0;
    }
    GVariantBuilder snapshot_builder;
    g_variant_builder_init(&snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot_builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle", g_variant_new_boolean(idle));
    g_variant_builder_add(&snapshot_builder, "{sv}", "threshold_ms",
                          g_variant_new_uint64(threshold_ms));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle_for_ms",
                          g_variant_new_uint64(idle_for_ms));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&snapshot_builder));
    if (control->session_activity_snapshot &&
        g_variant_equal(control->session_activity_snapshot, snapshot))
        return;

    g_clear_pointer(&control->session_activity_snapshot, g_variant_unref);
    control->session_activity_snapshot = g_variant_ref(snapshot);
    if (control->session_activity_revision < G_MAXUINT64)
        control->session_activity_revision++;
    native_publish_runtime_snapshot(control, "session-activity", snapshot,
                                    control->session_activity_revision);
    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder event_builder;
    g_variant_builder_init(&event_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(&event_builder, "{sv}", "idle", g_variant_new_boolean(idle));
    g_variant_builder_add(&event_builder, "{sv}", "threshold_ms",
                          g_variant_new_uint64(threshold_ms));
    g_variant_builder_add(&event_builder, "{sv}", "idle_for_ms", g_variant_new_uint64(idle_for_ms));
    g_variant_builder_add(&event_builder, "{sv}", "revision",
                          g_variant_new_uint64(control->session_activity_revision));
    g_variant_builder_add(&event_builder, "{sv}", "sequence", g_variant_new_uint64(sequence));
    g_variant_builder_add(&event_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&event_builder));
    native_runtime_dispatch_event(control, "gnoblin.session.activity-changed", event_payload);

    g_autoptr(JsonNode) event = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(event, object);
    json_object_set_string_member(object, "name", "gnoblin.session.activity-changed");
    json_object_set_boolean_member(object, "available", available);
    json_object_set_boolean_member(object, "idle", idle);
    json_object_set_int_member(object, "threshold_ms", (gint64)threshold_ms);
    json_object_set_int_member(object, "idle_for_ms", (gint64)idle_for_ms);
    json_object_set_int_member(object, "revision", control->session_activity_revision);
    json_object_set_int_member(object, "sequence", sequence);
    json_object_set_int_member(object, "time", time);
    publish_native_socket_event(control, event);
}

typedef struct {
    GnoblinNativeControl* control;
    guint64 generation;
} PendingActivityQuery;

static void native_activity_query_done(GObject* source, GAsyncResult* async_result,
                                       gpointer user_data) {
    PendingActivityQuery* query = user_data;
    GnoblinNativeControl* control = query->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source), async_result, &error);
    if (!control->stopping && query->generation == control->session_activity_generation && reply) {
        gboolean available = FALSE;
        gboolean idle = FALSE;
        guint64 threshold_ms = 0;
        guint64 idle_for_ms = 0;
        g_variant_get(reply, "(bbtt)", &available, &idle, &threshold_ms, &idle_for_ms);
        native_activity_publish(control, available, idle, threshold_ms, idle_for_ms);
    } else if (!control->stopping && query->generation == control->session_activity_generation) {
        native_activity_publish(control, FALSE, FALSE, 120000, 0);
    }
    if (control->pending_activity_queries)
        control->pending_activity_queries--;
    g_free(query);
    native_control_maybe_free_stopped(control);
}

static void native_activity_query(GnoblinNativeControl* control) {
    if (!control || control->stopping || !control->session_bus)
        return;
    PendingActivityQuery* query = g_new0(PendingActivityQuery, 1);
    query->control = control;
    query->generation = control->session_activity_generation;
    control->pending_activity_queries++;
    g_dbus_connection_call(control->session_bus, SESSION_ACTIVITY_BUS_NAME, SESSION_ACTIVITY_PATH,
                           SESSION_ACTIVITY_INTERFACE, "GetActivity", NULL,
                           G_VARIANT_TYPE("(bbtt)"), G_DBUS_CALL_FLAGS_NONE, 1000, NULL,
                           native_activity_query_done, query);
}

static void native_activity_signal(GDBusConnection* connection, const char* sender_name,
                                   const char* object_path, const char* interface_name,
                                   const char* signal_name, GVariant* parameters,
                                   gpointer user_data) {
    (void)connection;
    (void)sender_name;
    (void)object_path;
    (void)interface_name;
    (void)signal_name;
    GnoblinNativeControl* control = user_data;
    if (!g_variant_is_of_type(parameters, G_VARIANT_TYPE("(bbtt)")))
        return;
    control->session_activity_generation++;
    gboolean available = FALSE;
    gboolean idle = FALSE;
    guint64 threshold_ms = 0;
    guint64 idle_for_ms = 0;
    g_variant_get(parameters, "(bbtt)", &available, &idle, &threshold_ms, &idle_for_ms);
    native_activity_publish(control, available, idle, threshold_ms, idle_for_ms);
}

static void native_activity_owner_changed(GDBusConnection* connection, const char* sender_name,
                                          const char* object_path, const char* interface_name,
                                          const char* signal_name, GVariant* parameters,
                                          gpointer user_data) {
    (void)connection;
    (void)sender_name;
    (void)object_path;
    (void)interface_name;
    (void)signal_name;
    GnoblinNativeControl* control = user_data;
    const char* name = NULL;
    const char* old_owner = NULL;
    const char* new_owner = NULL;
    g_variant_get(parameters, "(&s&s&s)", &name, &old_owner, &new_owner);
    (void)old_owner;
    (void)old_owner;
    if (g_str_equal(name, SESSION_ACTIVITY_BUS_NAME)) {
        control->session_activity_generation++;
        if (new_owner && *new_owner)
            native_activity_query(control);
        else
            native_activity_publish(control, FALSE, FALSE, 120000, 0);
    }
}

static void native_activity_watch(GnoblinNativeControl* control) {
    if (!control || !control->session_bus)
        return;
    control->session_activity_subscription_id = g_dbus_connection_signal_subscribe(
        control->session_bus, SESSION_ACTIVITY_BUS_NAME, SESSION_ACTIVITY_INTERFACE,
        "ActivityChanged", SESSION_ACTIVITY_PATH, NULL, G_DBUS_SIGNAL_FLAGS_NONE,
        native_activity_signal, control, NULL);
    control->session_activity_owner_subscription_id = g_dbus_connection_signal_subscribe(
        control->session_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", SESSION_ACTIVITY_BUS_NAME, G_DBUS_SIGNAL_FLAGS_NONE,
        native_activity_owner_changed, control, NULL);
    control->session_activity_generation++;
    native_activity_publish(control, FALSE, FALSE, 120000, 0);
    native_activity_query(control);
}

static void portal_grants_watch_backend(GnoblinNativeControl* control) {
    if (!control || !control->session_bus)
        return;
    control->portal_grant_added_subscription_id = g_dbus_connection_signal_subscribe(
        control->session_bus, PORTAL_BACKEND_BUS_NAME, "org.gnoblin.Portal.Grants", "GrantAdded",
        "/org/gnoblin/Portal/Grants", NULL, G_DBUS_SIGNAL_FLAGS_NONE, portal_backend_signal,
        control, NULL);
    control->portal_grant_removed_subscription_id = g_dbus_connection_signal_subscribe(
        control->session_bus, PORTAL_BACKEND_BUS_NAME, "org.gnoblin.Portal.Grants", "GrantRemoved",
        "/org/gnoblin/Portal/Grants", NULL, G_DBUS_SIGNAL_FLAGS_NONE, portal_backend_signal,
        control, NULL);
    control->portal_owner_subscription_id = g_dbus_connection_signal_subscribe(
        control->session_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", PORTAL_BACKEND_BUS_NAME, G_DBUS_SIGNAL_FLAGS_NONE,
        portal_backend_owner_changed, control, NULL);
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        control->session_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "GetNameOwner", g_variant_new("(s)", PORTAL_BACKEND_BUS_NAME),
        G_VARIANT_TYPE("(s)"), G_DBUS_CALL_FLAGS_NONE, 1000, NULL, NULL);
    control->portal_backend_available = owner != NULL;
    if (control->portal_backend_available)
        portal_grant_fetch_snapshot(control);
}

static gboolean start_native_policy_dbus(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusNodeInfo) node =
        g_dbus_node_info_new_for_xml(native_policy_introspection, &error);
    if (!node) {
        g_warning("gnoblin-native-control: cannot load permission interface: %s", error->message);
        return FALSE;
    }
    control->session_bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (!control->session_bus) {
        g_warning("gnoblin-native-control: permission D-Bus service unavailable: %s",
                  error->message);
        return FALSE;
    }
    native_activity_watch(control);
    g_autoptr(GVariant) request_name = g_dbus_connection_call_sync(
        control->session_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
        "org.freedesktop.DBus", "RequestName", g_variant_new("(su)", NATIVE_POLICY_BUS_NAME, 4u),
        G_VARIANT_TYPE("(u)"), G_DBUS_CALL_FLAGS_NONE, 2000, NULL, &error);
    if (!request_name) {
        g_warning("gnoblin-native-control: cannot own permission D-Bus name: %s", error->message);
        return FALSE;
    }
    guint32 result;
    g_variant_get(request_name, "(u)", &result);
    if (result != 1 && result != 4) {
        g_warning("gnoblin-native-control: permission D-Bus name is already owned");
        return FALSE;
    }
    control->policy_bus_name_owned = TRUE;
    control->policy_dbus_registration_id = g_dbus_connection_register_object(
        control->session_bus, NATIVE_POLICY_OBJECT_PATH, node->interfaces[0], &native_policy_vtable,
        control, NULL, &error);
    if (!control->policy_dbus_registration_id) {
        g_warning("gnoblin-native-control: cannot export permission interface: %s", error->message);
        g_dbus_connection_call(control->session_bus, "org.freedesktop.DBus",
                               "/org/freedesktop/DBus", "org.freedesktop.DBus", "ReleaseName",
                               g_variant_new("(s)", NATIVE_POLICY_BUS_NAME), NULL,
                               G_DBUS_CALL_FLAGS_NONE, -1, NULL, NULL, NULL);
        control->policy_bus_name_owned = FALSE;
        return FALSE;
    }
    portal_grants_watch_backend(control);
    return TRUE;
}

static void stop_native_policy_dbus(GnoblinNativeControl* control) {
    if (control->portal_grant_retry_id) {
        g_source_remove(control->portal_grant_retry_id);
        control->portal_grant_retry_id = 0;
    }
    if (control->ibus_retry_source_id) {
        g_source_remove(control->ibus_retry_source_id);
        control->ibus_retry_source_id = 0;
    }
    if (control->ibus_bus && control->ibus_signal_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->ibus_bus,
                                             control->ibus_signal_subscription_id);
        control->ibus_signal_subscription_id = 0;
    }
    if (control->ibus_bus && control->ibus_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->ibus_bus,
                                             control->ibus_owner_subscription_id);
        control->ibus_owner_subscription_id = 0;
    }
    if (control->ibus_bus)
        g_signal_handlers_disconnect_by_data(control->ibus_bus, control);
    g_clear_object(&control->ibus_bus);
    if (!control->session_bus)
        return;
    if (control->portal_grant_added_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->portal_grant_added_subscription_id);
        control->portal_grant_added_subscription_id = 0;
    }
    if (control->portal_grant_removed_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->portal_grant_removed_subscription_id);
        control->portal_grant_removed_subscription_id = 0;
    }
    if (control->portal_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->portal_owner_subscription_id);
        control->portal_owner_subscription_id = 0;
    }
    if (control->session_activity_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->session_activity_subscription_id);
        control->session_activity_subscription_id = 0;
    }
    if (control->session_activity_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(control->session_bus,
                                             control->session_activity_owner_subscription_id);
        control->session_activity_owner_subscription_id = 0;
    }
    if (control->policy_dbus_registration_id) {
        g_dbus_connection_unregister_object(control->session_bus,
                                            control->policy_dbus_registration_id);
        control->policy_dbus_registration_id = 0;
    }
    if (control->policy_bus_name_owned) {
        g_dbus_connection_call(control->session_bus, "org.freedesktop.DBus",
                               "/org/freedesktop/DBus", "org.freedesktop.DBus", "ReleaseName",
                               g_variant_new("(s)", NATIVE_POLICY_BUS_NAME), NULL,
                               G_DBUS_CALL_FLAGS_NONE, -1, NULL, NULL, NULL);
        control->policy_bus_name_owned = FALSE;
    }
    g_clear_object(&control->session_bus);
    g_clear_pointer(&control->portal_grant_snapshot, g_variant_unref);
    native_publish_runtime_snapshot(control, "portal-grants", NULL, control->portal_grant_revision);
    g_clear_pointer(&control->session_activity_snapshot, g_variant_unref);
}

typedef struct {
    char* type;
    char* id;
    char* layout;
    char* variant;
    char* short_name;
    char* name;
} NativeInputSource;

typedef struct {
    char* json;
    char* comparable_json;
    gboolean focused;
} NativeWindowState;

typedef struct {
    guint64 generation;
    gint64 expires_at_us;
    guint32 timestamp;
    guint64 surface_id;
    guint64 client_id;
    guint64 focus_epoch;
    guint32 modifiers;
    xkb_mod_mask_t allowed_modifier_mask;
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

typedef struct {
    char* json;
} NativeWorkspaceState;

typedef struct {
    char* json;
} NativeMonitorState;

typedef struct {
    char* name;
    const char* category;
    GSubprocess* process;
} RunningCommand;

static void native_shortcut_free(gpointer data) {
    NativeShortcut* shortcut = data;
    g_free(shortcut->name);
    g_free(shortcut->binding);
    g_strfreev(shortcut->bindings);
    g_strfreev(shortcut->argv);
    g_free(shortcut->action_id);
    g_free(shortcut);
}

static GVariant* native_shortcut_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder records;
    g_variant_builder_init(&records, G_VARIANT_TYPE("av"));
    for (guint i = 0; control->shortcuts && i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(shortcut->name));
        if (shortcut->binding_count > 1)
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_strv((const char* const*)shortcut->bindings, -1));
        else if (shortcut->binding)
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_string(shortcut->binding));
        else
            g_variant_builder_add(&record, "{sv}", "binding",
                                  g_variant_new_strv((const char* const*)shortcut->bindings, -1));
        g_variant_builder_add(&record, "{sv}", "enabled", g_variant_new_boolean(shortcut->enabled));
        g_variant_builder_add(&record, "{sv}", "trigger",
                              g_variant_new_string(shortcut->release ? "release" : "press"));
        if (shortcut->action_id) {
            g_variant_builder_add(&record, "{sv}", "action",
                                  g_variant_new_string(shortcut->action_id));
        } else {
            g_variant_builder_add(&record, "{sv}", "command",
                                  g_variant_new_strv((const char* const*)shortcut->argv, -1));
        }
        g_variant_builder_add(&record, "{sv}", "revision",
                              g_variant_new_int64(control->state_revision));
        g_variant_builder_add_value(&records,
                                    g_variant_new_variant(g_variant_builder_end(&record)));
    }
    GVariantBuilder snapshot;
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "shortcuts", g_variant_builder_end(&records));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void native_window_state_free(gpointer data) {
    NativeWindowState* state = data;
    g_free(state->json);
    g_free(state->comparable_json);
    g_free(state);
}

static void native_workspace_state_free(gpointer data) {
    NativeWorkspaceState* state = data;
    g_free(state->json);
    g_free(state);
}

static void native_monitor_state_free(gpointer data) {
    NativeMonitorState* state = data;
    g_free(state->json);
    g_free(state);
}

static void native_snap_target_free(gpointer data) {
    NativeSnapTarget* target = data;
    g_free(target->id);
    g_free(target);
}

static void native_window_drag_free(gpointer data) {
    NativeWindowDrag* drag = data;
    g_free(drag->window_id);
    g_free(drag->monitor_id);
    g_free(drag->committed_target_id);
    g_clear_pointer(&drag->client_tokens, g_hash_table_unref);
    g_clear_pointer(&drag->targets, g_ptr_array_unref);
    g_free(drag);
}

static void native_snap_context_free(gpointer data) {
    NativeSnapContext* context = data;
    g_free(context->window_id);
    g_free(context->monitor_id);
    g_free(context);
}

static char** native_command_argv(GVariant* command) {
    if (!command || !g_variant_is_of_type(command, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(command) == 0)
        return NULL;
    char** argv = g_new0(char*, g_variant_n_children(command) + 1);
    for (gsize item = 0; item < g_variant_n_children(command); item++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(command, item);
        g_autoptr(GVariant) argument = g_variant_get_variant(wrapped);
        if (!g_variant_is_of_type(argument, G_VARIANT_TYPE_STRING)) {
            g_strfreev(argv);
            return NULL;
        }
        argv[item] = g_variant_dup_string(argument, NULL);
    }
    if (!*argv[0]) {
        g_strfreev(argv);
        return NULL;
    }
    return argv;
}

static void native_command_finished(GObject* source, GAsyncResult* result, gpointer user_data) {
    RunningCommand* run = user_data;
    g_autoptr(GError) error = NULL;
    if (!g_subprocess_wait_finish(G_SUBPROCESS(source), result, &error))
        g_warning("gnoblin-%s: %s: %s", run->category, run->name, error->message);
    else if (!g_subprocess_get_successful(run->process))
        g_warning("gnoblin-%s: %s exited unsuccessfully", run->category, run->name);
    g_object_unref(run->process);
    g_free(run->name);
    g_free(run);
}

static void launch_native_command(const char* category, const char* name, char** argv) {
    g_autoptr(GError) error = NULL;
    GSubprocess* process =
        g_subprocess_newv((const char* const*)argv, G_SUBPROCESS_FLAGS_NONE, &error);
    if (!process) {
        g_warning("gnoblin-%s: could not start %s: %s", category, name, error->message);
        return;
    }
    RunningCommand* run = g_new0(RunningCommand, 1);
    run->name = g_strdup(name);
    run->category = category;
    run->process = process;
    g_subprocess_wait_async(process, NULL, native_command_finished, run);
    g_message("gnoblin-%s: started %s", category, name);
}

static void native_shortcut_activated(MetaDisplay* display, guint action, gpointer device,
                                      guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && !shortcut->release)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static void capture_focus_identity(GnoblinNativeControl* control, NativeFocusContext* context,
                                   const ClutterEvent* event) {
    context->modifiers = event ? clutter_event_get_state(event) : 0;
    ClutterKeymap* keymap =
        control && control->input_seat ? clutter_seat_get_keymap(control->input_seat) : NULL;
    if (keymap) {
        xkb_mod_mask_t depressed = 0, latched = 0, locked = 0;
        clutter_keymap_get_modifier_state(keymap, &depressed, &latched, &locked);
        context->allowed_modifier_mask = depressed | latched | locked;
    }
    MetaWaylandSeat* seat =
        control && control->wayland_compositor ? control->wayland_compositor->seat : NULL;
    if (!seat)
        return;
    meta_wayland_seat_get_gnoblin_focus_identity(seat, &context->surface_id, &context->client_id,
                                                 &context->focus_epoch);
}

static void native_trusted_shortcut_activated(MetaDisplay* display, guint action,
                                              const ClutterEvent* event, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !event || !control->shortcuts)
        return;
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return;
    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (clutter_event_type(event) != CLUTTER_KEY_PRESS ||
        (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD)))
        return;

    if (flags & CLUTTER_EVENT_FLAG_REPEATED) {
        dispatch_dynamic_shortcut_repeat(control, action, event);
        return;
    }

    dispatch_dynamic_shortcut_activated(control, action, event);

    guint64 generation = native_config_generation(control);
    if (generation == 0)
        return;
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);

    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (shortcut->overlay || shortcut->release || shortcut->action != action)
            continue;
        if (g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
            break;

        guint64 handle = ++control->next_focus_context_handle;
        if (handle == 0)
            handle = ++control->next_focus_context_handle;
        guint64* key_copy = g_new(guint64, 1);
        *key_copy = handle;
        NativeFocusContext* context = g_new0(NativeFocusContext, 1);
        context->generation = generation;
        context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
        context->timestamp = clutter_event_get_time(event);
        capture_focus_identity(control, context, event);
        g_hash_table_insert(control->focus_contexts, key_copy, context);

        GVariantBuilder payload_builder;
        g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload_builder, "{sv}", "shortcut",
                              g_variant_new_string(shortcut->name));
        g_variant_builder_add(&payload_builder, "{sv}", "trigger", g_variant_new_string("press"));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
        meta_display_dispatch_gnoblin_shortcut_event(display, "gnoblin.shortcut.activated", payload,
                                                     handle, generation, context->expires_at_us);
        if (g_hash_table_contains(control->focus_contexts, &handle))
            publish_shortcut_focus_event(control, shortcut->name, handle, generation,
                                         context->expires_at_us);
    }
}

static gboolean focus_token_random(char token[65]) {
    static const char hex[] = "0123456789abcdef";
    guint8 bytes[32];
    gsize offset = 0;
    while (offset < sizeof(bytes)) {
        ssize_t count = getrandom(bytes + offset, sizeof(bytes) - offset, 0);
        if (count < 0 && errno == EINTR)
            continue;
        if (count <= 0)
            return FALSE;
        offset += (gsize)count;
    }
    for (gsize i = 0; i < sizeof(bytes); i++) {
        token[i * 2] = hex[bytes[i] >> 4];
        token[i * 2 + 1] = hex[bytes[i] & 0x0f];
    }
    token[64] = '\0';
    return TRUE;
}

static gboolean focus_token_exists(GnoblinNativeControl* control, const char* token) {
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (client->focus_grants && g_hash_table_contains(client->focus_grants, token))
            return TRUE;
    }
    return FALSE;
}

static void revoke_focus_grants_for_handle(GnoblinNativeControl* control, guint64 handle) {
    if (!control || !control->clients)
        return;
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (!client->focus_grants)
            continue;
        GHashTableIter grants;
        gpointer grant_value;
        g_hash_table_iter_init(&grants, client->focus_grants);
        while (g_hash_table_iter_next(&grants, NULL, &grant_value)) {
            NativeFocusGrant* grant = grant_value;
            if (grant->handle == handle)
                g_hash_table_iter_remove(&grants);
        }
    }
}

static void revoke_focus_contexts(GnoblinNativeControl* control) {
    if (!control)
        return;
    if (control->focus_contexts)
        g_hash_table_remove_all(control->focus_contexts);
    revoke_text_targets(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    revoke_menu_contexts(control);
    if (!control->clients)
        return;
    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
    }
}

static void native_menu_context_free(gpointer data) {
    NativeMenuContext* context = data;
    if (!context)
        return;
    g_free(context->window_id);
    g_free(context);
}

static void revoke_menu_contexts(GnoblinNativeControl* control) {
    if (!control)
        return;
    if (control->menu_contexts)
        g_hash_table_remove_all(control->menu_contexts);
    if (!control->clients)
        return;
    GHashTableIter clients;
    gpointer value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &value, NULL)) {
        Client* client = value;
        if (client->menu_grants)
            g_hash_table_remove_all(client->menu_grants);
    }
}

static guint64 native_menu_context_create(GnoblinNativeControl* control, const char* window_id,
                                          guint64 socket_owner_client_id, gint64* expires_at_us) {
    if (!control || control->stopping || !control->menu_contexts || !window_id || !*window_id ||
        !control->wayland_compositor ||
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return 0;
    gint64 now = g_get_monotonic_time();
    GHashTableIter stale;
    gpointer stale_value;
    g_hash_table_iter_init(&stale, control->menu_contexts);
    while (g_hash_table_iter_next(&stale, NULL, &stale_value)) {
        NativeMenuContext* old = stale_value;
        if (old->expires_at_us <= now)
            g_hash_table_iter_remove(&stale);
    }
    if (g_hash_table_size(control->menu_contexts) >= MAX_MENU_CONTEXTS)
        return 0;
    guint64 handle = ++control->next_menu_context_handle;
    if (!handle)
        handle = ++control->next_menu_context_handle;
    guint64* key = g_new(guint64, 1);
    *key = handle;
    gint64 expires = now + 5 * G_USEC_PER_SEC;
    NativeMenuContext* context = g_new0(NativeMenuContext, 1);
    context->window_id = g_strdup(window_id);
    context->generation = native_config_generation(control);
    context->expires_at_us = expires;
    context->socket_owner_client_id = socket_owner_client_id;
    g_hash_table_insert(control->menu_contexts, key, context);
    if (expires_at_us)
        *expires_at_us = expires;
    return handle;
}

static void native_menu_context_revoke_client(GnoblinNativeControl* control, guint64 client_id) {
    if (!control || !control->menu_contexts || !client_id)
        return;
    GHashTableIter contexts;
    gpointer value;
    g_hash_table_iter_init(&contexts, control->menu_contexts);
    while (g_hash_table_iter_next(&contexts, NULL, &value)) {
        NativeMenuContext* context = value;
        if (context->socket_owner_client_id == client_id)
            g_hash_table_iter_remove(&contexts);
    }
}

static void native_menu_prune_client_grants(Client* client, gint64 now) {
    if (!client || !client->menu_grants || !client->control)
        return;
    GHashTableIter grants;
    gpointer key, value;
    g_hash_table_iter_init(&grants, client->menu_grants);
    while (g_hash_table_iter_next(&grants, &key, &value)) {
        NativeMenuGrant* grant = value;
        if (grant->expires_at_us <= now) {
            g_hash_table_remove(client->control->menu_contexts, &grant->handle);
            g_hash_table_iter_remove(&grants);
        }
    }
}

static void revoke_text_targets(GnoblinNativeControl* control) {
    if (control && control->text_targets)
        g_hash_table_remove_all(control->text_targets);
}

static void native_socket_revoke_client_tokens(Client* client) {
    if (!client || !client->control)
        return;
    GnoblinNativeControl* control = client->control;
    native_menu_context_revoke_client(control, client->client_id);
    if (client->menu_grants)
        g_hash_table_remove_all(client->menu_grants);
    if (control->text_targets) {
        GHashTableIter targets;
        gpointer value;
        g_hash_table_iter_init(&targets, control->text_targets);
        while (g_hash_table_iter_next(&targets, NULL, &value)) {
            NativeTextTarget* target = value;
            if (target->socket_owner_client_id == client->client_id)
                g_hash_table_iter_remove(&targets);
        }
    }
    if (control->snap_contexts) {
        GHashTableIter contexts;
        gpointer value;
        g_hash_table_iter_init(&contexts, control->snap_contexts);
        while (g_hash_table_iter_next(&contexts, NULL, &value)) {
            NativeSnapContext* context = value;
            if (context->socket_owner_client_id == client->client_id)
                g_hash_table_iter_remove(&contexts);
        }
    }
}

static void prune_focus_contexts(GnoblinNativeControl* control, gint64 now) {
    if (!control || !control->focus_contexts)
        return;
    GHashTableIter contexts;
    gpointer key, value;
    g_hash_table_iter_init(&contexts, control->focus_contexts);
    while (g_hash_table_iter_next(&contexts, &key, &value)) {
        NativeFocusContext* context = value;
        if (context->expires_at_us <= now) {
            guint64 handle = *(guint64*)key;
            g_hash_table_iter_remove(&contexts);
            revoke_focus_grants_for_handle(control, handle);
        }
    }

    if (control->text_targets) {
        GHashTableIter targets;
        gpointer target_value;
        g_hash_table_iter_init(&targets, control->text_targets);
        while (g_hash_table_iter_next(&targets, NULL, &target_value)) {
            NativeTextTarget* target = target_value;
            guint64 surface_id = 0, client_id = 0, focus_epoch = 0;
            graphene_rect_t caret;
            gboolean valid = target->expires_at_us > now &&
                             target->generation == native_config_generation(control) &&
                             control->wayland_compositor && control->wayland_compositor->seat &&
                             control->wayland_compositor->seat->text_input &&
                             !meta_wayland_session_lock_is_active(control->wayland_compositor) &&
                             meta_wayland_text_input_get_gnoblin_state(
                                 control->wayland_compositor->seat->text_input, &surface_id,
                                 &client_id, &focus_epoch, &caret) &&
                             surface_id == target->surface_id && client_id == target->client_id &&
                             focus_epoch == target->focus_epoch;
            if (!valid)
                g_hash_table_iter_remove(&targets);
        }
    }

    if (control->snap_contexts) {
        GHashTableIter snap_contexts;
        gpointer snap_value;
        g_hash_table_iter_init(&snap_contexts, control->snap_contexts);
        while (g_hash_table_iter_next(&snap_contexts, NULL, &snap_value)) {
            NativeSnapContext* context = snap_value;
            if (context->expires_at_us <= now ||
                context->native_generation != native_config_generation(control) ||
                context->settings_revision != native_config_revision(control))
                g_hash_table_iter_remove(&snap_contexts);
        }
    }

    GHashTableIter clients;
    gpointer client_value;
    g_hash_table_iter_init(&clients, control->clients);
    while (g_hash_table_iter_next(&clients, &client_value, NULL)) {
        Client* client = client_value;
        if (!client->focus_grants)
            continue;
        GHashTableIter grants;
        gpointer grant_value;
        g_hash_table_iter_init(&grants, client->focus_grants);
        while (g_hash_table_iter_next(&grants, NULL, &grant_value)) {
            NativeFocusGrant* grant = grant_value;
            if (grant->expires_at_us <= now ||
                !g_hash_table_contains(control->focus_contexts, &grant->handle))
                g_hash_table_iter_remove(&grants);
        }
    }
}

static gboolean focus_context_expiry_tick(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping)
        return G_SOURCE_REMOVE;
    prune_focus_contexts(control, g_get_monotonic_time());
    return G_SOURCE_CONTINUE;
}

static void publish_shortcut_focus_event(GnoblinNativeControl* control, const char* shortcut,
                                         guint64 handle, guint64 generation, gint64 expires_at_us) {
    if (!control || control->stopping || !shortcut ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;

    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 10 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.shortcut.activated"))
            continue;
        prune_focus_contexts(control, g_get_monotonic_time());
        if (!g_hash_table_contains(control->focus_contexts, &handle) ||
            expires_at_us <= g_get_monotonic_time())
            break;
        if (!client->focus_grants || g_hash_table_size(client->focus_grants) >= MAX_FOCUS_CONTEXTS)
            continue;

        char token[65];
        gboolean unique = FALSE;
        for (guint attempt = 0; attempt < 4; attempt++) {
            if (!focus_token_random(token))
                break;
            if (!focus_token_exists(control, token)) {
                unique = TRUE;
                break;
            }
        }
        if (!unique)
            continue;

        NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
        grant->handle = handle;
        grant->generation = generation;
        grant->expires_at_us = expires_at_us;
        g_hash_table_insert(client->focus_grants, g_strdup(token), grant);

        JsonObject* object = json_object_new();
        json_object_set_string_member(object, "event", "gnoblin.shortcut.activated");
        json_object_set_string_member(object, "shortcut", shortcut);
        json_object_set_string_member(object, "trigger", "press");
        json_object_set_string_member(object, "focus_context", token);
        json_object_set_int_member(object, "revision", control->state_revision);
        json_object_set_int_member(object, "sequence", ++control->event_sequence);
        json_object_set_int_member(object, "time", g_get_monotonic_time());
        g_autoptr(JsonNode) event = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(event, object);
        g_autofree char* encoded = json_to_string(event, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
}

static void native_dynamic_shortcut_free(gpointer data) {
    NativeDynamicShortcut* shortcut = data;
    if (!shortcut)
        return;
    if (shortcut->timeout_id)
        g_source_remove(shortcut->timeout_id);
    if (shortcut->trigger_event)
        clutter_event_free(shortcut->trigger_event);
    g_free(shortcut->id);
    g_free(shortcut->accelerator);
    g_free(shortcut->owner_id);
    g_free(shortcut);
}

static gboolean dynamic_shortcut_owner_subscribed(NativeDynamicShortcut* shortcut,
                                                  const char* event) {
    return shortcut && shortcut->client && !shortcut->client->closing &&
           shortcut->client->event_api_minor >= 9 && shortcut->client->event_subscriptions &&
           g_hash_table_contains(shortcut->client->event_subscriptions, event);
}

static void dynamic_shortcut_publish_event(GnoblinNativeControl* control,
                                           NativeDynamicShortcut* shortcut, const char* event,
                                           GVariant* payload) {
    if (!control || !shortcut || !event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return;
    if (shortcut->owner_id && g_str_has_prefix(shortcut->owner_id, "lua:")) {
        native_runtime_dispatch_event(control, event, payload);
        return;
    }
    if (!dynamic_shortcut_owner_subscribed(shortcut, event))
        return;
    g_autoptr(JsonNode) json = json_from_variant(payload);
    if (!json || !JSON_NODE_HOLDS_OBJECT(json))
        return;
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", event);
    g_autofree char* encoded = json_to_string(json, FALSE);
    send_response(shortcut->client, g_strconcat(encoded, "\n", NULL));
}

static gboolean issue_focus_context_for_session_key(GnoblinNativeControl* control,
                                                    NativeDynamicShortcut* shortcut,
                                                    const ClutterEvent* event, guint64* handle_out,
                                                    guint64* generation_out,
                                                    gint64* expires_at_us_out, char token_out[65]) {
    if (handle_out)
        *handle_out = 0;
    if (generation_out)
        *generation_out = 0;
    if (expires_at_us_out)
        *expires_at_us_out = 0;
    if (token_out)
        token_out[0] = '\0';

    if (!control || control->stopping || !control->focus_contexts || !shortcut ||
        !shortcut->active || control->active_shortcut_session != shortcut || !event ||
        (clutter_event_type(event) != CLUTTER_KEY_PRESS &&
         clutter_event_type(event) != CLUTTER_KEY_RELEASE))
        return FALSE;

    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD |
                 CLUTTER_EVENT_FLAG_REPEATED))
        return FALSE;
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor))
        return FALSE;

    Client* client = shortcut->client;
    if ((!client && (!shortcut->owner_id || !g_str_has_prefix(shortcut->owner_id, "lua:"))) ||
        (client && (client->closing || client->api_minor < 68 || !client->focus_grants ||
                    g_hash_table_size(client->focus_grants) >= MAX_FOCUS_CONTEXTS)))
        return FALSE;

    guint64 generation = native_config_generation(control);
    gint64 now = g_get_monotonic_time();
    if (!generation)
        return FALSE;
    prune_focus_contexts(control, now);
    if (g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS)
        return FALSE;

    char token[65] = {0};
    if (client) {
        gboolean unique = FALSE;
        for (guint attempt = 0; attempt < 4; attempt++) {
            if (!focus_token_random(token))
                break;
            if (!focus_token_exists(control, token)) {
                unique = TRUE;
                break;
            }
        }
        if (!unique)
            return FALSE;
    }

    guint64 handle = ++control->next_focus_context_handle;
    if (handle == 0)
        handle = ++control->next_focus_context_handle;
    guint64* key_copy = g_new(guint64, 1);
    *key_copy = handle;
    NativeFocusContext* context = g_new0(NativeFocusContext, 1);
    context->generation = generation;
    context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
    context->timestamp = clutter_event_get_time(event);
    capture_focus_identity(control, context, event);
    g_hash_table_insert(control->focus_contexts, key_copy, context);

    if (client) {
        NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
        grant->handle = handle;
        grant->generation = generation;
        grant->expires_at_us = context->expires_at_us;
        g_hash_table_insert(client->focus_grants, g_strdup(token), grant);
    }

    if (handle_out)
        *handle_out = handle;
    if (generation_out)
        *generation_out = generation;
    if (expires_at_us_out)
        *expires_at_us_out = context->expires_at_us;
    if (token_out && client)
        g_strlcpy(token_out, token, 65);
    return TRUE;
}

static void dynamic_shortcut_end_session(GnoblinNativeControl* control,
                                         NativeDynamicShortcut* shortcut, const char* reason) {
    if (!control || !shortcut || !shortcut->active)
        return;
    shortcut->active = FALSE;
    if (shortcut->timeout_id) {
        g_source_remove(shortcut->timeout_id);
        shortcut->timeout_id = 0;
    }
    if (control->active_shortcut_session == shortcut) {
        if (control->shortcut_session_capture_active) {
            control->shortcut_session_capture_active = FALSE;
            meta_display_stop_native_key_capture(control->display);
            meta_display_unregister_native_key_capture_handler(control->display, control);
        }
        control->active_shortcut_session = NULL;
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&builder, "{sv}", "session_id",
                          g_variant_new_uint64(shortcut->session_id));
    g_variant_builder_add(&builder, "{sv}", "reason",
                          g_variant_new_string(reason ? reason : "cancelled"));
    g_variant_builder_add(&builder, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.ended", payload);
}

static gboolean native_shortcut_session_timeout(gpointer user_data) {
    NativeDynamicShortcut* shortcut = user_data;
    GnoblinNativeControl* control = shortcut ? shortcut->control : NULL;
    if (shortcut)
        shortcut->timeout_id = 0;
    if (control)
        dynamic_shortcut_end_session(control, shortcut, "timed_out");
    return G_SOURCE_REMOVE;
}

static guint dynamic_shortcut_count(GnoblinNativeControl* control, Client* owner) {
    guint count = 0;
    if (!control || !control->dynamic_shortcuts)
        return 0;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (!owner || shortcut->client == owner)
            count++;
    }
    if (control->bare_super_shortcut && (!owner || control->bare_super_shortcut->client == owner))
        count++;
    return count;
}

static NativeDynamicShortcut* find_dynamic_shortcut(Client* owner, const char* id) {
    if (!owner || !owner->control || !owner->control->dynamic_shortcuts || !id)
        return NULL;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, owner->control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->client == owner && g_str_equal(shortcut->id, id))
            return shortcut;
    }
    if (owner->control->bare_super_shortcut &&
        owner->control->bare_super_shortcut->client == owner &&
        g_str_equal(owner->control->bare_super_shortcut->id, id))
        return owner->control->bare_super_shortcut;
    return NULL;
}

static NativeDynamicShortcut* find_runtime_dynamic_shortcut(GnoblinNativeControl* control,
                                                            const char* owner_id, const char* id) {
    if (!control || !owner_id || !id)
        return NULL;
    if (control->bare_super_shortcut &&
        g_str_equal(control->bare_super_shortcut->owner_id, owner_id) &&
        g_str_equal(control->bare_super_shortcut->id, id))
        return control->bare_super_shortcut;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (!shortcut->client && g_str_equal(shortcut->owner_id, owner_id) &&
            g_str_equal(shortcut->id, id))
            return shortcut;
    }
    return NULL;
}

static void remove_dynamic_shortcut(GnoblinNativeControl* control,
                                    NativeDynamicShortcut* shortcut) {
    if (!control || !shortcut)
        return;
    dynamic_shortcut_end_session(control, shortcut, "unbound");
    if (control->bare_super_shortcut == shortcut) {
        control->bare_super_shortcut = NULL;
        native_dynamic_shortcut_free(shortcut);
        return;
    }
    if (!control->dynamic_shortcuts)
        return;
    guint action = shortcut->action;
    meta_display_ungrab_accelerator(control->display, action);
    g_hash_table_remove(control->dynamic_shortcuts, GUINT_TO_POINTER(action));
}

static void clear_client_dynamic_shortcuts(Client* client) {
    if (!client || !client->control || !client->control->dynamic_shortcuts)
        return;
    GnoblinNativeControl* control = client->control;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->client == client) {
            dynamic_shortcut_end_session(control, shortcut, "owner_disconnected");
            meta_display_ungrab_accelerator(control->display, shortcut->action);
            g_hash_table_iter_remove(&iter);
        }
    }
    if (control->bare_super_shortcut && control->bare_super_shortcut->client == client) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        dynamic_shortcut_end_session(control, shortcut, "owner_disconnected");
        control->bare_super_shortcut = NULL;
        native_dynamic_shortcut_free(shortcut);
    }
}

static void clear_runtime_dynamic_shortcuts(GnoblinNativeControl* control, const char* reason) {
    if (!control || !control->dynamic_shortcuts)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->dynamic_shortcuts);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeDynamicShortcut* shortcut = value;
        if (shortcut->owner_id && g_str_has_prefix(shortcut->owner_id, "lua:")) {
            dynamic_shortcut_end_session(control, shortcut, reason);
            meta_display_ungrab_accelerator(control->display, shortcut->action);
            g_hash_table_iter_remove(&iter);
        }
    }
    if (control->bare_super_shortcut && control->bare_super_shortcut->owner_id &&
        g_str_has_prefix(control->bare_super_shortcut->owner_id, "lua:")) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        dynamic_shortcut_end_session(control, shortcut, reason);
        control->bare_super_shortcut = NULL;
        native_dynamic_shortcut_free(shortcut);
    }
}

static void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                                const ClutterEvent* event) {
    if (!control || control->stopping || !event || !control->dynamic_shortcuts ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    NativeDynamicShortcut* shortcut =
        action ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
               : control->bare_super_shortcut;
    if (!shortcut || !shortcut->armed ||
        (shortcut->client && (shortcut->client->closing || !shortcut->client->control ||
                              shortcut->client->api_minor < 11)))
        return;
    Client* client = shortcut->client;
    gboolean starts_session = shortcut->hold_mask || shortcut->modal || shortcut->capture_input;
    gboolean first_activation =
        !starts_session || !shortcut->active || !shortcut->activation_dispatched;
    gboolean wants_binding_event = !client || dynamic_shortcut_owner_subscribed(
                                                  shortcut, "gnoblin.shortcut.binding-activated");
    gboolean wants_session_event =
        !client ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.activated") ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.key") ||
        dynamic_shortcut_owner_subscribed(shortcut, "gnoblin.shortcut.session.ended");
    if ((starts_session && !wants_session_event && !wants_binding_event) ||
        (!starts_session && !wants_binding_event))
        return;
    if (shortcut->trigger_release && !shortcut->releasing) {
        if (shortcut->trigger_event)
            clutter_event_free(shortcut->trigger_event);
        shortcut->trigger_event = clutter_event_copy(event);
        return;
    }
    guint64 generation = native_config_generation(control);
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);
    gboolean issue_focus_context =
        generation != 0 && (!client || wants_binding_event) &&
        g_hash_table_size(control->focus_contexts) < MAX_FOCUS_CONTEXTS &&
        (!client ||
         (client->focus_grants && g_hash_table_size(client->focus_grants) < MAX_FOCUS_CONTEXTS));

    guint64 handle = 0;
    NativeFocusContext* context = NULL;
    if (issue_focus_context) {
        handle = ++control->next_focus_context_handle;
        if (handle == 0)
            handle = ++control->next_focus_context_handle;
        guint64* key_copy = g_new(guint64, 1);
        *key_copy = handle;
        context = g_new0(NativeFocusContext, 1);
        context->generation = generation;
        context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
        context->timestamp = clutter_event_get_time(event);
        capture_focus_identity(control, context, event);
        g_hash_table_insert(control->focus_contexts, key_copy, context);
    }

    guint64 session_id = shortcut->session_id;
    if (starts_session && !shortcut->active) {
        session_id = ++control->next_shortcut_session_id;
        if (session_id == 0)
            session_id = ++control->next_shortcut_session_id;
        shortcut->session_id = session_id;
        shortcut->active = TRUE;
        shortcut->activation_dispatched = FALSE;
        shortcut->held_mask = shortcut->hold_mask;
        control->active_shortcut_session = shortcut;
        shortcut->timeout_id = g_timeout_add_seconds(10, native_shortcut_session_timeout, shortcut);
    }

    char token[65] = {0};
    if (client && wants_binding_event) {
        gboolean unique = FALSE;
        if (issue_focus_context) {
            for (guint attempt = 0; attempt < 4; attempt++) {
                if (!focus_token_random(token))
                    break;
                if (!focus_token_exists(control, token)) {
                    unique = TRUE;
                    break;
                }
            }
        }
        if (unique) {
            NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
            grant->handle = handle;
            grant->generation = generation;
            grant->expires_at_us = context->expires_at_us;
            g_hash_table_insert(client->focus_grants, g_strdup(token), grant);
        } else if (handle) {
            g_hash_table_remove(control->focus_contexts, &handle);
            handle = 0;
            context = NULL;
        }
    }

    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&payload_builder, "{sv}", "accelerator",
                          g_variant_new_string(shortcut->accelerator));
    g_variant_builder_add(&payload_builder, "{sv}", "trigger",
                          g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
    g_variant_builder_add(&payload_builder, "{sv}", "first",
                          g_variant_new_boolean(first_activation));
    g_variant_builder_add(&payload_builder, "{sv}", "modifiers",
                          g_variant_new_uint32(clutter_event_get_state(event)));
    g_variant_builder_add(&payload_builder, "{sv}", "time", g_variant_new_int64(now));
    if (session_id)
        g_variant_builder_add(&payload_builder, "{sv}", "session_id",
                              g_variant_new_uint64(session_id));
    if (handle) {
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_handle",
                              g_variant_new_uint64(handle));
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_generation",
                              g_variant_new_uint64(generation));
        g_variant_builder_add(&payload_builder, "{sv}", "focus_context_expires_at_us",
                              g_variant_new_int64(context->expires_at_us));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    if (!client)
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.binding-activated",
                                       payload);
    if (starts_session) {
        GVariantBuilder session_builder;
        g_variant_builder_init(&session_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&session_builder, "{sv}", "id", g_variant_new_string(shortcut->id));
        g_variant_builder_add(&session_builder, "{sv}", "session_id",
                              g_variant_new_uint64(session_id));
        g_variant_builder_add(&session_builder, "{sv}", "first",
                              g_variant_new_boolean(first_activation));
        g_variant_builder_add(
            &session_builder, "{sv}", "trigger",
            g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
        g_variant_builder_add(&session_builder, "{sv}", "modifiers",
                              g_variant_new_uint32(clutter_event_get_state(event)));
        g_variant_builder_add(&session_builder, "{sv}", "time", g_variant_new_int64(now));
        g_autoptr(GVariant) session_payload =
            g_variant_ref_sink(g_variant_builder_end(&session_builder));
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.activated",
                                       session_payload);
        shortcut->activation_dispatched = TRUE;
        if ((shortcut->modal || shortcut->capture_input) &&
            !control->shortcut_session_capture_active) {
            if (control->shortcut_capture_active || control->active_shortcut_session != shortcut ||
                !meta_display_can_start_native_key_capture(control->display) ||
                !meta_display_register_native_key_capture_handler(
                    control->display, native_shortcut_capture_key, control, NULL)) {
                dynamic_shortcut_end_session(control, shortcut, "preempted");
                return;
            }
            control->shortcut_session_capture_active = TRUE;
            if (!meta_display_start_native_key_capture(control->display)) {
                control->shortcut_session_capture_active = FALSE;
                meta_display_unregister_native_key_capture_handler(control->display, control);
                dynamic_shortcut_end_session(control, shortcut, "preempted");
                return;
            }
        }
    }
    if (client && wants_binding_event) {
        JsonObject* object = json_object_new();
        json_object_set_string_member(object, "event", "gnoblin.shortcut.binding-activated");
        json_object_set_string_member(object, "id", shortcut->id);
        json_object_set_string_member(object, "accelerator", shortcut->accelerator);
        json_object_set_string_member(object, "trigger",
                                      shortcut->trigger_release ? "release" : "press");
        json_object_set_boolean_member(object, "first", first_activation);
        json_object_set_int_member(object, "modifiers", clutter_event_get_state(event));
        if (session_id)
            json_object_set_int_member(object, "session_id", session_id);
        if (token[0])
            json_object_set_string_member(object, "focus_context", token);
        json_object_set_int_member(object, "revision", control->state_revision);
        json_object_set_int_member(object, "sequence", ++control->event_sequence);
        json_object_set_int_member(object, "input_time", clutter_event_get_time(event));
        json_object_set_int_member(object, "time", now);
        g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(root, object);
        g_autofree char* encoded = json_to_string(root, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    shortcut->activated = !shortcut->trigger_release;
}

static void dispatch_dynamic_shortcut_repeat(GnoblinNativeControl* control, guint action,
                                             const ClutterEvent* event) {
    NativeDynamicShortcut* shortcut =
        control && control->dynamic_shortcuts
            ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
            : NULL;
    if (!shortcut || !shortcut->active || !event ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
    g_variant_builder_add(&builder, "{sv}", "session_id",
                          g_variant_new_uint64(shortcut->session_id));
    g_variant_builder_add(&builder, "{sv}", "first", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&builder, "{sv}", "trigger",
                          g_variant_new_string(shortcut->trigger_release ? "release" : "press"));
    g_variant_builder_add(&builder, "{sv}", "modifiers",
                          g_variant_new_uint32(clutter_event_get_state(event)));
    g_variant_builder_add(&builder, "{sv}", "time",
                          g_variant_new_int64(clutter_event_get_time(event)));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.activated",
                                   payload);
}

static void native_shortcut_deactivated(MetaDisplay* display, guint action, gpointer device,
                                        guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    NativeDynamicShortcut* dynamic =
        control && control->dynamic_shortcuts
            ? g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action))
            : NULL;
    if (dynamic && dynamic->trigger_release && dynamic->trigger_event) {
        dynamic->releasing = TRUE;
        dispatch_dynamic_shortcut_activated(control, action, dynamic->trigger_event);
        dynamic->releasing = FALSE;
        clutter_event_free(dynamic->trigger_event);
        dynamic->trigger_event = NULL;
    }
    if (dynamic && !dynamic->trigger_release && dynamic->activated) {
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(dynamic->id));
        g_variant_builder_add(&builder, "{sv}", "accelerator",
                              g_variant_new_string(dynamic->accelerator));
        g_variant_builder_add(&builder, "{sv}", "input_time", g_variant_new_uint32(timestamp));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
        dynamic_shortcut_publish_event(control, dynamic, "gnoblin.shortcut.binding-deactivated",
                                       payload);
        dynamic->activated = FALSE;
    }
    /* A release-triggered binding may only become active in this callback,
     * after the hold key's physical release was already observed. Complete
     * that short session immediately, after publishing activation. */
    if (dynamic && dynamic->trigger_release && dynamic->active && dynamic->hold_mask &&
        !native_shortcut_hold_family_pressed(control, dynamic->hold_mask))
        dynamic_shortcut_end_session(control, dynamic, "released");
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && shortcut->release)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

void gnoblin_native_control_observe_key_event(MetaDisplay* display, const ClutterEvent* event) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !event)
        return;

    ClutterEventType type = clutter_event_type(event);
    if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
        return;
    ClutterEventFlags flags = clutter_event_get_flags(event);
    if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
        return;

    gboolean pressed = type == CLUTTER_KEY_PRESS;
    gboolean* state = NULL;
    switch (clutter_event_get_key_symbol(event)) {
    case XKB_KEY_Super_L:
        state = &control->super_left_pressed;
        break;
    case XKB_KEY_Super_R:
        state = &control->super_right_pressed;
        break;
    case XKB_KEY_Control_L:
        state = &control->control_left_pressed;
        break;
    case XKB_KEY_Control_R:
        state = &control->control_right_pressed;
        break;
    case XKB_KEY_Alt_L:
        state = &control->alt_left_pressed;
        break;
    case XKB_KEY_Alt_R:
        state = &control->alt_right_pressed;
        break;
    default:
        return;
    }
    *state = pressed;

    NativeDynamicShortcut* shortcut = control->active_shortcut_session;
    if (!shortcut || !shortcut->active || !shortcut->hold_mask || shortcut->modal ||
        shortcut->capture_input)
        return;
    if (!native_shortcut_hold_family_pressed(control, shortcut->hold_mask))
        dynamic_shortcut_end_session(control, shortcut, "released");
}

static void native_overlay_key(MetaDisplay* display, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (shortcut->overlay)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static gboolean native_action_target(GVariant* action, const char** group, char** native_name) {
    const char* name = NULL;
    const char* schema = NULL;
    if (g_variant_is_of_type(action, G_VARIANT_TYPE_STRING)) {
        const char* label = g_variant_get_string(action, NULL);
        const char* separator = strchr(label, '.');
        if (!separator)
            return FALSE;
        g_autofree char* namespace = g_strndup(label, separator - label);
        if (g_str_equal(namespace, "wm") || g_str_equal(namespace, "mutter") ||
            g_str_equal(namespace, "wayland"))
            *group = g_intern_string(namespace);
        else
            return FALSE;
        name = separator + 1;
        if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0))
            return FALSE;
        *native_name = g_strdup(name);
        g_strdelimit(*native_name, "_", '-');
        return TRUE;
    }
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(action) != 2 || !g_variant_lookup(action, "schema", "&s", &schema) ||
        !g_variant_lookup(action, "key", "&s", &name) ||
        !g_regex_match_simple("^[a-z0-9]+(?:-[a-z0-9]+)*$", name, 0, 0))
        return FALSE;
    if (g_str_equal(schema, "org.gnome.desktop.wm.keybindings"))
        *group = "wm";
    else if (g_str_equal(schema, "org.gnome.mutter.keybindings"))
        *group = "mutter";
    else if (g_str_equal(schema, "org.gnome.mutter.wayland.keybindings"))
        *group = "wayland";
    else
        return FALSE;
    *native_name = g_strdup(name);
    return TRUE;
}

static GVariant* native_binding_array(GVariant* bindings) {
    if (!bindings || !g_variant_is_of_type(bindings, G_VARIANT_TYPE("av")))
        return NULL;
    g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(bindings) + 1);
    for (gsize i = 0; i < g_variant_n_children(bindings); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(bindings, i);
        g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING) ||
            !*g_variant_get_string(binding, NULL) ||
            strlen(g_variant_get_string(binding, NULL)) > 160)
            return NULL;
        strings[i] = g_variant_dup_string(binding, NULL);
    }
    return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
}

static gboolean merge_native_action(GVariantDict* groups, GVariant* entry, GError** error) {
    g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
    g_autoptr(GVariant) command = g_variant_lookup_value(entry, "command", NULL);
    g_autoptr(GVariant) trigger = g_variant_lookup_value(entry, "trigger", NULL);
    g_autoptr(GVariant) binding = g_variant_lookup_value(entry, "binding", NULL);
    g_autoptr(GVariant) capture = g_variant_lookup_value(entry, "capture-input", NULL);
    g_autoptr(GVariant) native_bindings = NULL;
    g_autoptr(GVariant) existing_group = NULL;
    g_autoptr(GVariant) existing_action = NULL;
    const char* group = NULL;
    g_autofree char* native_name = NULL;
    if (!native_action_target(action, &group, &native_name)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named shortcut action must identify a supported Mutter or Wayland "
                            "keybinding; GNOME Shell actions are not supported");
        return FALSE;
    }
    if (command || trigger ||
        (capture && (!g_variant_is_of_type(capture, G_VARIANT_TYPE_BOOLEAN) ||
                     g_variant_get_boolean(capture)))) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "named built-in actions cannot have command, trigger, or input capture");
        return FALSE;
    }
    native_bindings = native_binding_array(binding);
    if (!native_bindings) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named built-in action needs an array of accelerators");
        return FALSE;
    }
    const char* schema_name = g_str_equal(group, "wm") ? "org.gnome.desktop.wm.keybindings"
                              : g_str_equal(group, "mutter")
                                  ? "org.gnome.mutter.keybindings"
                                  : "org.gnome.mutter.wayland.keybindings";
    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    g_autoptr(GSettingsSchema) schema =
        source ? g_settings_schema_source_lookup(source, schema_name, TRUE) : NULL;
    if (!schema || !g_settings_schema_has_key(schema, native_name)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "unknown native shortcut action: %s.%s", group, native_name);
        return FALSE;
    }
    g_autoptr(GSettingsSchemaKey) schema_key = g_settings_schema_get_key(schema, native_name);
    if (!g_variant_type_equal(g_settings_schema_key_get_value_type(schema_key),
                              G_VARIANT_TYPE_STRING_ARRAY)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "native shortcut action does not accept bindings: %s.%s", group, native_name);
        return FALSE;
    }
    existing_group = g_variant_dict_lookup_value(groups, group, G_VARIANT_TYPE_VARDICT);
    existing_action =
        existing_group ? g_variant_lookup_value(existing_group, native_name, NULL) : NULL;
    if (existing_action) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "native action configured more than once: %s.%s", group, native_name);
        return FALSE;
    }
    GVariantDict actions;
    g_variant_dict_init(&actions, existing_group);
    g_variant_dict_insert_value(&actions, native_name, native_bindings);
    g_autoptr(GVariant) merged_actions = g_variant_ref_sink(g_variant_dict_end(&actions));
    g_variant_dict_insert_value(groups, group, merged_actions);
    return TRUE;
}

static GVariant* merge_native_actions(GVariant* base, GVariant* document, GError** error) {
    g_autoptr(GVariant) shortcuts =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!shortcuts)
        return g_variant_ref(base);
    if (!g_variant_is_of_type(shortcuts, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(shortcuts) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return NULL;
    }
    GVariantDict groups;
    g_variant_dict_init(&groups, base);
    for (gsize i = 0; i < g_variant_n_children(shortcuts); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(shortcuts, i);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut entry %zu must be a table", i + 1);
            g_variant_dict_clear(&groups);
            return NULL;
        }
        g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
        if (action && !merge_native_action(&groups, entry, error)) {
            g_variant_dict_clear(&groups);
            return NULL;
        }
    }
    return g_variant_ref_sink(g_variant_dict_end(&groups));
}

static gboolean apply_native_keybindings(GVariant* document, GError** error) {
    static const struct {
        const char* group;
        const char* schema;
    } schemas[] = {
        {"wm", "org.gnome.desktop.wm.keybindings"},
        {"mutter", "org.gnome.mutter.keybindings"},
        {"wayland", "org.gnome.mutter.wayland.keybindings"},
    };
    g_autoptr(GVariant) configured =
        document ? g_variant_lookup_value(document, "keybindings", NULL) : NULL;
    if (configured && !g_variant_is_of_type(configured, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings must be a table");
        return FALSE;
    }
    GVariantBuilder groups;
    g_autoptr(GVariant) normalized = NULL;
    g_autoptr(GVariant) merged = NULL;
    g_variant_builder_init(&groups, G_VARIANT_TYPE_VARDICT);
    if (configured) {
        GVariantIter iter;
        const char* group;
        GVariant* entries;
        GSettingsSchemaSource* source = g_settings_schema_source_get_default();
        g_variant_iter_init(&iter, configured);
        while (g_variant_iter_next(&iter, "{&sv}", &group, &entries)) {
            g_autoptr(GVariant) group_entries = entries;
            const char* schema_name = NULL;
            for (guint i = 0; i < G_N_ELEMENTS(schemas); i++)
                if (g_str_equal(group, schemas[i].group))
                    schema_name = schemas[i].schema;
            if (!schema_name) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "keybindings.%s is unsupported in the standalone session; use wm, "
                            "mutter, or wayland",
                            group);
                goto invalid_keybindings;
            }
            if (!g_variant_is_of_type(group_entries, G_VARIANT_TYPE_VARDICT)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings.%s must be a table", group);
                goto invalid_keybindings;
            }
            g_autoptr(GSettingsSchema) schema =
                source ? g_settings_schema_source_lookup(source, schema_name, TRUE) : NULL;
            if (!schema) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "keybinding schema %s is not installed", schema_name);
                goto invalid_keybindings;
            }
            GVariantBuilder actions;
            GVariantIter action_iter;
            const char* name;
            GVariant* value;
            g_variant_builder_init(&actions, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&action_iter, group_entries);
            while (g_variant_iter_next(&action_iter, "{&sv}", &name, &value)) {
                g_autoptr(GVariant) bindings = value;
                if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "invalid keybinding name: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autofree char* native_name = g_strdup(name);
                g_strdelimit(native_name, "_", '-');
                if (!g_settings_schema_has_key(schema, native_name)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "unknown keybinding: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autoptr(GSettingsSchemaKey) schema_key =
                    g_settings_schema_get_key(schema, native_name);
                g_autoptr(GVariant) native_bindings = native_binding_array(bindings);
                if (!g_variant_type_equal(g_settings_schema_key_get_value_type(schema_key),
                                          G_VARIANT_TYPE_STRING_ARRAY) ||
                    !native_bindings) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "keybindings.%s.%s needs an array of accelerators", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_variant_builder_add(&actions, "{sv}", native_name,
                                      g_variant_ref(native_bindings));
            }
            g_autoptr(GVariant) native_actions =
                g_variant_ref_sink(g_variant_builder_end(&actions));
            g_variant_builder_add(&groups, "{sv}", group, g_variant_ref(native_actions));
        }
    }
    normalized = g_variant_ref_sink(g_variant_builder_end(&groups));
    merged = merge_native_actions(normalized, document, error);
    if (!merged)
        return FALSE;
    meta_prefs_apply_gnoblin_keybindings(merged);
    return TRUE;

invalid_keybindings:
    g_variant_builder_clear(&groups);
    return FALSE;
}

static gboolean start_native_shortcuts(GnoblinNativeControl* control, GVariant* document,
                                       GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!declarations)
        return TRUE;
    if (!g_variant_is_of_type(declarations, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(declarations) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return FALSE;
    }
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) shortcuts = g_ptr_array_new_with_free_func(native_shortcut_free);
    gboolean has_overlay = FALSE;
    for (gsize index = 0; index < g_variant_n_children(declarations); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) command = NULL;
        g_autoptr(GVariant) binding_value = NULL;
        g_autoptr(GVariant) trigger_value = NULL;
        g_autoptr(GVariant) capture_value = NULL;
        g_autoptr(GVariant) action_value = NULL;
        const char* name = NULL;
        const char* binding = NULL;
        const char* trigger = "press";
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT))
            goto invalid_shortcut;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported = g_str_equal(key, "name") || g_str_equal(key, "binding") ||
                                 g_str_equal(key, "command") || g_str_equal(key, "action") ||
                                 g_str_equal(key, "trigger") || g_str_equal(key, "capture-input");
            g_variant_unref(value);
            if (!supported)
                goto invalid_shortcut;
        }
        if (!g_variant_lookup(entry, "name", "&s", &name) || !*name || strlen(name) > 80 ||
            g_hash_table_contains(names, name))
            goto invalid_shortcut;
        for (const char* character = name; *character; character++)
            if (!g_ascii_isalnum(*character) && *character != '_' && *character != '-')
                goto invalid_shortcut;
        action_value = g_variant_lookup_value(entry, "action", NULL);
        if (action_value) {
            const char* group = NULL;
            g_autofree char* native_name = NULL;
            if (!native_action_target(action_value, &group, &native_name)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "shortcuts.%s.action is unsupported; use a command or a wm, mutter, "
                            "or wayland native action",
                            name ? name : "<unnamed>");
                goto invalid_shortcut;
            }
            binding_value = g_variant_lookup_value(entry, "binding", NULL);
            g_autoptr(GVariant) native_bindings = native_binding_array(binding_value);
            if (!native_bindings)
                goto invalid_shortcut;
            NativeShortcut* shortcut = g_new0(NativeShortcut, 1);
            shortcut->name = g_strdup(name);
            shortcut->action_id = g_strdup_printf("%s.%s", group, native_name);
            shortcut->bindings = g_variant_dup_strv(native_bindings, NULL);
            shortcut->binding_count = g_variant_n_children(native_bindings);
            shortcut->binding = shortcut->bindings[0] ? g_strdup(shortcut->bindings[0]) : NULL;
            shortcut->enabled = shortcut->bindings[0] != NULL;
            g_ptr_array_add(shortcuts, shortcut);
            g_hash_table_add(names, g_strdup(name));
            continue;
        }
        binding_value = g_variant_lookup_value(entry, "binding", NULL);
        if (!binding_value || !g_variant_is_of_type(binding_value, G_VARIANT_TYPE_STRING))
            goto invalid_shortcut;
        binding = g_variant_get_string(binding_value, NULL);
        if (!*binding || strlen(binding) > 160)
            goto invalid_shortcut;
        trigger_value = g_variant_lookup_value(entry, "trigger", NULL);
        if (trigger_value) {
            if (!g_variant_is_of_type(trigger_value, G_VARIANT_TYPE_STRING))
                goto invalid_shortcut;
            trigger = g_variant_get_string(trigger_value, NULL);
            if (!g_str_equal(trigger, "press") && !g_str_equal(trigger, "release"))
                goto invalid_shortcut;
        }
        capture_value = g_variant_lookup_value(entry, "capture-input", NULL);
        if (capture_value) {
            if (!g_variant_is_of_type(capture_value, G_VARIANT_TYPE_BOOLEAN))
                goto invalid_shortcut;
            if (g_variant_get_boolean(capture_value)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "shortcuts.%s.capture-input is unsupported in the standalone session; "
                            "use a command or native action",
                            name);
                goto invalid_shortcut;
            }
        }
        command = g_variant_lookup_value(entry, "command", NULL);
        NativeShortcut* shortcut = g_new0(NativeShortcut, 1);
        shortcut->name = g_strdup(name);
        shortcut->binding = g_strdup(binding);
        shortcut->bindings = g_new0(char*, 2);
        shortcut->bindings[0] = g_strdup(binding);
        shortcut->binding_count = 1;
        shortcut->argv = native_command_argv(command);
        shortcut->enabled = TRUE;
        if (!shortcut->argv) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        shortcut->overlay = g_str_equal(binding, "Super");
        shortcut->release = g_str_equal(trigger, "release");
        if (shortcut->overlay && (!shortcut->release || has_overlay)) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        has_overlay |= shortcut->overlay;
        g_hash_table_add(names, g_strdup(name));
        g_ptr_array_add(shortcuts, shortcut);
        continue;

    invalid_shortcut:
        if (!error || !*error)
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut entry %zu needs a unique name, binding, command array, and valid "
                        "trigger",
                        index + 1);
        return FALSE;
    }
    for (guint i = 0; i < shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(shortcuts, i);
        if (shortcut->overlay)
            continue;
        MetaKeyBindingFlags flags = META_KEY_BINDING_IGNORE_AUTOREPEAT;
        if (shortcut->release)
            flags |= META_KEY_BINDING_TRIGGER_RELEASE;
        shortcut->action =
            meta_display_grab_accelerator(control->display, shortcut->binding, flags);
        if (shortcut->action == META_KEYBINDING_ACTION_NONE) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut '%s' could not claim %s", shortcut->name, shortcut->binding);
            for (guint previous = 0; previous < i; previous++) {
                NativeShortcut* claimed = g_ptr_array_index(shortcuts, previous);
                if (!claimed->overlay)
                    meta_display_ungrab_accelerator(control->display, claimed->action);
            }
            return FALSE;
        }
    }
    control->shortcuts = g_steal_pointer(&shortcuts);
    meta_display_set_gnoblin_shortcut_activated_handler(control->display,
                                                        native_trusted_shortcut_activated, control);
    g_signal_connect(control->display, "accelerator-activated",
                     G_CALLBACK(native_shortcut_activated), control);
    g_signal_connect(control->display, "accelerator-deactivated",
                     G_CALLBACK(native_shortcut_deactivated), control);
    if (has_overlay)
        g_signal_connect(control->display, "overlay-key", G_CALLBACK(native_overlay_key), control);
    g_autoptr(GVariant) shortcut_snapshot = native_shortcut_snapshot(control);
    native_publish_runtime_snapshot(control, "shortcuts", shortcut_snapshot,
                                    control->state_revision);
    return TRUE;
}

typedef enum {
    INPUT_BOOLEAN,
    INPUT_DOUBLE,
    INPUT_MILLISECONDS,
    INPUT_STRING,
    INPUT_STRINGS,
    INPUT_CHOICE,
    INPUT_ACCEL_CURVE,
} InputKind;

typedef struct {
    const char* group;
    const char* name;
    InputKind kind;
    const char* choices;
    double minimum;
    double maximum;
} InputField;

static const InputField input_fields[] = {
    {"mouse", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"mouse", "left-handed", INPUT_BOOLEAN},
    {"mouse", "natural-scroll", INPUT_BOOLEAN},
    {"mouse", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"mouse", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"touchpad", "scroll-speed", INPUT_DOUBLE, NULL, 0, 2},
    {"touchpad", "left-handed", INPUT_CHOICE, "right left mouse"},
    {"touchpad", "natural-scroll", INPUT_BOOLEAN},
    {"touchpad", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"touchpad", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "tap-to-click", INPUT_BOOLEAN},
    {"touchpad", "tap-button-map", INPUT_CHOICE, "default lrm lmr"},
    {"touchpad", "tap-and-drag", INPUT_BOOLEAN},
    {"touchpad", "tap-and-drag-lock", INPUT_BOOLEAN},
    {"touchpad", "disable-while-typing", INPUT_BOOLEAN},
    {"touchpad", "edge-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "two-finger-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "click-method", INPUT_CHOICE, "default none areas fingers"},
    {"keyboard", "repeat", INPUT_BOOLEAN},
    {"keyboard", "delay", INPUT_MILLISECONDS},
    {"keyboard", "repeat-interval", INPUT_MILLISECONDS},
    {"keyboard", "remember-numlock-state", INPUT_BOOLEAN},
    {"keyboard", "numlock-state", INPUT_BOOLEAN},
    {"keyboard", "xkb-options", INPUT_STRINGS},
    {"tablets", "mapping", INPUT_CHOICE, "absolute relative"},
    {"tablets", "left-handed", INPUT_BOOLEAN},
    {"tablets", "keep-aspect", INPUT_BOOLEAN},
    {"styluses", "eraser-button-mode", INPUT_CHOICE, "default button"},
    {"styluses", "eraser-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "eraser-button-keybinding", INPUT_STRING},
    {"styluses", "button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "secondary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "tertiary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "button-keybinding", INPUT_STRING},
    {"styluses", "secondary-button-keybinding", INPUT_STRING},
    {"styluses", "tertiary-button-keybinding", INPUT_STRING},
};

static const InputField* find_input_field(const char* group, const char* name) {
    for (guint i = 0; i < G_N_ELEMENTS(input_fields); i++)
        if (g_str_equal(input_fields[i].group, group) && g_str_equal(input_fields[i].name, name))
            return &input_fields[i];
    return NULL;
}

static GVariant* normalize_input_value(const InputField* field, GVariant* value) {
    if (field->kind == INPUT_ACCEL_CURVE && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        GVariantBuilder curve;
        GVariantIter iter;
        const char* name;
        GVariant* member;
        g_autoptr(GVariant) step_value = NULL;
        g_autoptr(GVariant) points_value = NULL;
        double step;
        GArray* points = g_array_new(FALSE, FALSE, sizeof(double));
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &member)) {
            g_autoptr(GVariant) current = member;
            if (g_str_equal(name, "step"))
                step_value = g_steal_pointer(&current);
            else if (g_str_equal(name, "points"))
                points_value = g_steal_pointer(&current);
            else {
                g_array_unref(points);
                return NULL;
            }
        }
        if (!step_value || !points_value) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_DOUBLE))
            step = g_variant_get_double(step_value);
        else if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_INT64))
            step = (double)g_variant_get_int64(step_value);
        else {
            g_array_unref(points);
            return NULL;
        }
        if (!isfinite(step) || step <= 0) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("ad"))) {
            gsize n_points;
            const double* values =
                g_variant_get_fixed_array(points_value, &n_points, sizeof(double));
            for (gsize i = 0; i < n_points; i++) {
                if (!isfinite(values[i]) || values[i] < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, values[i]);
            }
        } else if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("av"))) {
            for (gsize i = 0; i < g_variant_n_children(points_value); i++) {
                g_autoptr(GVariant) boxed = g_variant_get_child_value(points_value, i);
                g_autoptr(GVariant) point = g_variant_get_variant(boxed);
                double number;
                if (g_variant_is_of_type(point, G_VARIANT_TYPE_DOUBLE))
                    number = g_variant_get_double(point);
                else if (g_variant_is_of_type(point, G_VARIANT_TYPE_INT64))
                    number = (double)g_variant_get_int64(point);
                else {
                    g_array_unref(points);
                    return NULL;
                }
                if (!isfinite(number) || number < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, number);
            }
        } else {
            g_array_unref(points);
            return NULL;
        }
        if (points->len < 2) {
            g_array_unref(points);
            return NULL;
        }
        g_variant_builder_init(&curve, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&curve, "{sv}", "step", g_variant_new_double(step));
        g_variant_builder_add(&curve, "{sv}", "points",
                              g_variant_new_fixed_array(G_VARIANT_TYPE_DOUBLE, points->data,
                                                        points->len, sizeof(double)));
        g_array_unref(points);
        return g_variant_ref_sink(g_variant_builder_end(&curve));
    }
    if (field->kind == INPUT_BOOLEAN && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        return g_variant_ref(value);
    if (field->kind == INPUT_STRING && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return g_variant_ref(value);
    if (field->kind == INPUT_CHOICE && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        g_auto(GStrv) choices = g_strsplit(field->choices, " ", -1);
        if (g_strv_contains((const char* const*)choices, g_variant_get_string(value, NULL)))
            return g_variant_ref(value);
    }
    if (field->kind == INPUT_DOUBLE) {
        double number;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            number = g_variant_get_double(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            number = (double)g_variant_get_int64(value);
        else
            return NULL;
        if (isfinite(number) && number >= field->minimum && number <= field->maximum)
            return g_variant_ref_sink(g_variant_new_double(number));
    }
    if (field->kind == INPUT_MILLISECONDS) {
        gint64 milliseconds;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            milliseconds = g_variant_get_int64(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
            milliseconds = g_variant_get_uint32(value);
        else
            return NULL;
        if (milliseconds >= 1 && milliseconds <= 10000)
            return g_variant_ref_sink(g_variant_new_uint32((guint32)milliseconds));
    }
    if (field->kind == INPUT_STRINGS && g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(value) + 1);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
            g_autoptr(GVariant) item = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING))
                return NULL;
            strings[i] = g_variant_dup_string(item, NULL);
        }
        return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
    }
    return NULL;
}

static GVariant* normalize_input_fields(const char* group, GVariant* fields, GError** error) {
    if (!g_variant_is_of_type(fields, G_VARIANT_TYPE_VARDICT))
        goto invalid_group;
    GVariantBuilder converted;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_builder_init(&converted, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&iter, fields);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) current = value;
        const InputField* field = find_input_field(group, name);
        if (!field) {
            g_variant_builder_clear(&converted);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown input setting: input.%s.%s", group, name);
            return NULL;
        }
        if (g_variant_is_of_type(current, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(current, NULL), "inherit"))
            continue;
        g_autoptr(GVariant) normalized = normalize_input_value(field, current);
        if (!normalized) {
            g_variant_builder_clear(&converted);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "input.%s.%s: invalid value", group, name);
            return NULL;
        }
        g_variant_builder_add(&converted, "{sv}", name, g_variant_ref(normalized));
    }
    return g_variant_ref_sink(g_variant_builder_end(&converted));

invalid_group:
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "input.%s must be a table", group);
    return NULL;
}

static gboolean valid_input_device(const char* group, const char* device) {
    return g_regex_match_simple(g_str_equal(group, "tablets")
                                    ? "^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$"
                                    : "^(?:[0-9a-fA-F]+|default-[0-9a-fA-F]{4}:[0-9a-fA-F]{4})$",
                                device, G_REGEX_OPTIMIZE, 0);
}

static const char* native_orientation_name(MetaOrientation orientation) {
    switch (orientation) {
    case META_ORIENTATION_NORMAL:
        return "normal";
    case META_ORIENTATION_BOTTOM_UP:
        return "bottom-up";
    case META_ORIENTATION_LEFT_UP:
        return "left-up";
    case META_ORIENTATION_RIGHT_UP:
        return "right-up";
    case META_ORIENTATION_UNDEFINED:
    default:
        return "undefined";
    }
}

static GVariant* native_orientation_lock_snapshot(GnoblinNativeControl* control, guint64 revision) {
    MetaOrientationManager* manager = control ? control->orientation_manager : NULL;
    gboolean available = manager && meta_orientation_manager_has_accelerometer(manager);
    const char* source = control && control->orientation_lock_runtime_override ? "runtime"
                         : control && control->orientation_lock_configured     ? "config"
                                                                               : "system";
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(
        &builder, "{sv}", "locked",
        g_variant_new_boolean(available &&
                              meta_orientation_manager_get_orientation_locked(manager)));
    g_variant_builder_add(
        &builder, "{sv}", "orientation",
        g_variant_new_string(
            available ? native_orientation_name(meta_orientation_manager_get_orientation(manager))
                      : "undefined"));
    g_variant_builder_add(&builder, "{sv}", "source", g_variant_new_string(source));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void native_orientation_lock_publish(GnoblinNativeControl* control) {
    if (!control || control->stopping)
        return;

    g_autoptr(GVariant) candidate =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    gboolean initialized = control->orientation_lock_snapshot != NULL;
    gboolean changed =
        !initialized || !g_variant_equal(control->orientation_lock_snapshot, candidate);
    if (!changed)
        return;

    if (control->orientation_lock_revision < G_MAXUINT64)
        control->orientation_lock_revision++;
    g_clear_pointer(&control->orientation_lock_snapshot, g_variant_unref);
    control->orientation_lock_snapshot =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    native_publish_runtime_snapshot(control, "input-orientation-lock",
                                    control->orientation_lock_snapshot,
                                    control->orientation_lock_revision);

    /* The first value seeds the Lua worker; only later changes are events. */
    if (!initialized)
        return;

    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder event_builder;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_builder_init(&event_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_builder, "{sv}", "name",
                          g_variant_new_string("gnoblin.input.orientation-lock-changed"));
    g_variant_iter_init(&fields, control->orientation_lock_snapshot);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) value = field_value;
        g_variant_builder_add(&event_builder, "{sv}", field_name, g_variant_ref(value));
    }
    g_variant_builder_add(&event_builder, "{sv}", "sequence", g_variant_new_uint64(sequence));
    g_variant_builder_add(&event_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&event_builder));
    g_autoptr(JsonNode) json = json_from_variant(event_payload);
    if (JSON_NODE_HOLDS_OBJECT(json))
        publish_native_socket_event(control, json);
    native_runtime_dispatch_event(control, "gnoblin.input.orientation-lock-changed", event_payload);
}

static void native_orientation_manager_notified(GObject* manager, GParamSpec* property,
                                                gpointer user_data) {
    (void)manager;
    (void)property;
    GnoblinNativeControl* control = user_data;
    if (control && !control->orientation_lock_notifications_suppressed)
        native_orientation_lock_publish(control);
}

static void native_orientation_manager_accelerometer_notified(GObject* manager,
                                                              GParamSpec* property,
                                                              gpointer user_data) {
    native_orientation_manager_notified(manager, property, user_data);
}

static void native_orientation_manager_orientation_changed(MetaOrientationManager* manager,
                                                           gpointer user_data) {
    (void)manager;
    GnoblinNativeControl* control = user_data;
    if (control && !control->orientation_lock_notifications_suppressed)
        native_orientation_lock_publish(control);
}

static gboolean apply_native_input(GnoblinNativeControl* control, MetaContext* context,
                                   GVariant* document, GError** error) {
    g_autoptr(GVariant) input = document ? g_variant_lookup_value(document, "input", NULL) : NULL;
    if (!input) {
        control->orientation_lock_configured = FALSE;
        control->orientation_lock_config_value = FALSE;
        control->orientation_lock_runtime_override = FALSE;
        control->orientation_lock_notifications_suppressed = TRUE;
        if (control->orientation_manager)
            meta_orientation_manager_clear_orientation_lock_override(control->orientation_manager);
        control->orientation_lock_notifications_suppressed = FALSE;
        native_orientation_lock_publish(control);
        return TRUE;
    }
    if (!g_variant_is_of_type(input, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input must be a table");
        return FALSE;
    }
    GVariantBuilder converted;
    GVariantIter groups;
    const char* group;
    GVariant* value;
    gboolean orientation_set = FALSE;
    gboolean orientation_locked = FALSE;
    g_autoptr(GVariant) normalized_input = NULL;
    g_autoptr(GVariant) keyboard = NULL;
    g_variant_builder_init(&converted, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&groups, input);
    while (g_variant_iter_next(&groups, "{&sv}", &group, &value)) {
        g_autoptr(GVariant) settings = value;
        if (g_str_equal(group, "orientation-lock")) {
            if (g_variant_is_of_type(settings, G_VARIANT_TYPE_BOOLEAN)) {
                orientation_set = TRUE;
                orientation_locked = g_variant_get_boolean(settings);
            } else if (!g_variant_is_of_type(settings, G_VARIANT_TYPE_STRING) ||
                       !g_str_equal(g_variant_get_string(settings, NULL), "inherit"))
                goto invalid_group;
            continue;
        }
        if (g_str_equal(group, "mouse") || g_str_equal(group, "touchpad") ||
            g_str_equal(group, "keyboard")) {
            g_autoptr(GVariant) normalized = normalize_input_fields(group, settings, error);
            if (!normalized)
                goto invalid_input;
            g_variant_builder_add(&converted, "{sv}", group, g_variant_ref(normalized));
            continue;
        }
        if (g_str_equal(group, "tablets") || g_str_equal(group, "styluses")) {
            if (!g_variant_is_of_type(settings, G_VARIANT_TYPE_VARDICT))
                goto invalid_group;
            GVariantBuilder devices;
            GVariantIter iter;
            const char* device;
            GVariant* fields;
            g_variant_builder_init(&devices, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&iter, settings);
            while (g_variant_iter_next(&iter, "{&sv}", &device, &fields)) {
                g_autoptr(GVariant) device_fields = fields;
                if (!valid_input_device(group, device)) {
                    g_variant_builder_clear(&devices);
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "input.%s: invalid device identifier %s", group, device);
                    goto invalid_input;
                }
                g_autoptr(GVariant) normalized =
                    normalize_input_fields(group, device_fields, error);
                if (!normalized) {
                    g_variant_builder_clear(&devices);
                    goto invalid_input;
                }
                g_variant_builder_add(&devices, "{sv}", device, g_variant_ref(normalized));
            }
            g_autoptr(GVariant) normalized = g_variant_ref_sink(g_variant_builder_end(&devices));
            g_variant_builder_add(&converted, "{sv}", group, g_variant_ref(normalized));
            continue;
        }
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "unknown input group: %s",
                    group);
        goto invalid_input;

    invalid_group:
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "input.%s: invalid value",
                    group);
        goto invalid_input;
    }
    normalized_input = g_variant_ref_sink(g_variant_builder_end(&converted));
    meta_display_apply_gnoblin_input_config(control->display, normalized_input);
    keyboard = g_variant_lookup_value(normalized_input, "keyboard", G_VARIANT_TYPE_VARDICT);
    meta_prefs_apply_gnoblin_keyboard_preferences(keyboard);
    MetaOrientationManager* orientation =
        control->orientation_manager
            ? control->orientation_manager
            : meta_backend_get_orientation_manager(meta_context_get_backend(context));
    control->orientation_lock_configured = orientation_set;
    control->orientation_lock_config_value = orientation_locked;
    control->orientation_lock_runtime_override = FALSE;
    control->orientation_lock_notifications_suppressed = TRUE;
    if (orientation) {
        if (orientation_set)
            meta_orientation_manager_set_orientation_locked(orientation, orientation_locked);
        else
            meta_orientation_manager_clear_orientation_lock_override(orientation);
    }
    control->orientation_lock_notifications_suppressed = FALSE;
    native_orientation_lock_publish(control);
    return TRUE;

invalid_input:
    g_variant_builder_clear(&converted);
    return FALSE;
}

static void client_free(Client* client) {
    native_socket_revoke_client_tokens(client);
    if (client->control)
        g_hash_table_remove(client->control->clients, client);
    g_io_stream_close(G_IO_STREAM(client->connection), NULL, NULL);
    g_clear_object(&client->connection);
    g_string_free(client->request, TRUE);
    g_queue_free_full(client->outgoing, g_free);
    g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
    g_clear_pointer(&client->focus_grants, g_hash_table_unref);
    g_clear_pointer(&client->menu_grants, g_hash_table_unref);
    g_clear_pointer(&client->pending_grant_operations, g_hash_table_unref);
    g_free(client);
}

static void client_maybe_free(Client* client) {
    if (client->closing && !client->reading && !client->writing &&
        client->pending_deferred_requests == 0)
        client_free(client);
}

static void pending_runtime_request_free(gpointer data) {
    PendingRuntimeRequest* pending = data;
    if (!pending)
        return;
    g_free(pending->request_id);
    g_free(pending->legacy_window_action);
    g_free(pending->legacy_window_id);
    g_free(pending);
}

static void native_window_drag_client_disconnected(GnoblinNativeControl* control,
                                                   guint64 client_id);

static void client_close(Client* client) {
    if (!client->closing) {
        client->closing = TRUE;
        clear_client_dynamic_shortcuts(client);
        if (client->control) {
            native_socket_revoke_client_tokens(client);
            native_window_drag_client_disconnected(client->control, client->client_id);
            g_hash_table_remove(client->control->clients, client);
        }
        g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
        if (client->menu_grants)
            g_hash_table_remove_all(client->menu_grants);
        client->control = NULL;
        g_io_stream_close(G_IO_STREAM(client->connection), NULL, NULL);
    }
    client_maybe_free(client);
}

static JsonNode* json_from_variant(GVariant* value) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        return json_from_variant(child);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        JsonObject* object = json_object_new();
        GVariantIter iter;
        const char* key;
        GVariant* child;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            json_object_set_member(object, key, json_from_variant(child));
            g_variant_unref(child);
        }
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, object);
        return node;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_ARRAY)) {
        JsonArray* array = json_array_new();
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            json_array_add_element(array, json_from_variant(child));
        }
        JsonNode* node = json_node_new(JSON_NODE_ARRAY);
        json_node_take_array(node, array);
        return node;
    }
    JsonNode* node = json_node_new(JSON_NODE_VALUE);
    switch (g_variant_classify(value)) {
    case G_VARIANT_CLASS_BOOLEAN:
        json_node_set_boolean(node, g_variant_get_boolean(value));
        break;
    case G_VARIANT_CLASS_DOUBLE:
        json_node_set_double(node, g_variant_get_double(value));
        break;
    case G_VARIANT_CLASS_INT32:
        json_node_set_int(node, g_variant_get_int32(value));
        break;
    case G_VARIANT_CLASS_INT64:
        json_node_set_int(node, g_variant_get_int64(value));
        break;
    case G_VARIANT_CLASS_UINT32:
        json_node_set_int(node, g_variant_get_uint32(value));
        break;
    case G_VARIANT_CLASS_STRING:
        json_node_set_string(node, g_variant_get_string(value, NULL));
        break;
    default:
        json_node_free(node);
        return json_node_new(JSON_NODE_NULL);
    }
    return node;
}

static GVariant* variant_from_json(JsonNode* node) {
    if (JSON_NODE_HOLDS_OBJECT(node)) {
        GVariantBuilder builder;
        JsonObject* object = json_node_get_object(node);
        GList* members = json_object_get_members(object);
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        for (GList* item = members; item; item = item->next) {
            const char* key = item->data;
            GVariant* value = variant_from_json(json_object_get_member(object, key));
            if (value)
                g_variant_builder_add(&builder, "{sv}", key, value);
        }
        g_list_free(members);
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (JSON_NODE_HOLDS_ARRAY(node)) {
        GVariantBuilder builder;
        JsonArray* array = json_node_get_array(node);
        g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
        for (guint i = 0; i < json_array_get_length(array); i++) {
            GVariant* value = variant_from_json(json_array_get_element(array, i));
            if (value)
                g_variant_builder_add_value(&builder, g_variant_new_variant(value));
        }
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (!JSON_NODE_HOLDS_VALUE(node))
        return NULL;
    GType type = json_node_get_value_type(node);
    if (type == G_TYPE_BOOLEAN)
        return g_variant_new_boolean(json_node_get_boolean(node));
    if (type == G_TYPE_DOUBLE || type == G_TYPE_FLOAT)
        return g_variant_new_double(json_node_get_double(node));
    if (type == G_TYPE_INT64 || type == G_TYPE_INT || type == G_TYPE_LONG)
        return g_variant_new_int64(json_node_get_int(node));
    if (type == G_TYPE_STRING)
        return g_variant_new_string(json_node_get_string(node));
    return NULL;
}

static char* encode_response(const char* id, JsonNode* result, const char* message) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "event", message ? "error" : "reply");
    json_object_set_string_member(object, "id", id);
    if (message)
        json_object_set_string_member(object, "message", message);
    else
        json_object_set_member(object, "result", json_node_copy(result));
    g_autofree char* encoded = json_to_string(root, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static char* encode_event_subscription(JsonArray* events, guint api_minor) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "event", "subscribed");
    JsonArray* subscribed = json_array_new();
    for (guint i = 0; i < json_array_get_length(events); i++)
        json_array_add_element(subscribed, json_node_copy(json_array_get_element(events, i)));
    json_object_set_array_member(object, "events", subscribed);
    JsonObject* version = json_object_new();
    json_object_set_int_member(version, "major", GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_object_set_int_member(version, "minor", api_minor);
    json_object_set_object_member(object, "api_version", version);
    g_autofree char* encoded = json_to_string(root, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static void cache_lua_window_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                      guint64 revision);
static void cache_lua_workspace_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                         guint64 revision);

static JsonNode* layer_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                                     GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "layer.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "layers", NULL, control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "layers", NULL, control->state_revision);
        return NULL;
    }
    JsonArray* layers = json_object_get_array_member(json_node_get_object(json), "layers");
    if (!layers) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "layers", NULL, control->state_revision);
        return NULL;
    }
    for (guint i = 0; i < json_array_get_length(layers); i++) {
        JsonNode* record = json_array_get_element(layers, i);
        if (JSON_NODE_HOLDS_OBJECT(record))
            json_object_set_int_member(json_node_get_object(record), "revision",
                                       control->state_revision);
    }
    json_object_set_int_member(json_node_get_object(json), "revision", control->state_revision);
    if (update_lua_snapshot) {
        g_autoptr(GVariant) snapshot = variant_from_json(json);
        native_publish_runtime_snapshot(control, "layers", snapshot, control->state_revision);
    }
    return g_steal_pointer(&json);
}

static GVariant* capability_snapshot_record(GnoblinNativeControl* control,
                                            const NativeCapability* native_capability) {
    GVariantBuilder capability;
    gboolean available = TRUE;
    const char* unavailable_reason = NULL;

    if (g_str_equal(native_capability->id, "microphone-monitor")) {
#ifdef HAVE_REMOTE_DESKTOP
        available = control->privacy_microphone_available;
        if (!available)
            unavailable_reason = "pipewire_unavailable";
#else
        available = FALSE;
        unavailable_reason = "remote_desktop_disabled";
#endif
    } else if (g_str_equal(native_capability->id, "camera-monitor")) {
#ifdef HAVE_REMOTE_DESKTOP
        available = control->privacy_camera_available;
        if (!available)
            unavailable_reason = "pipewire_unavailable";
#else
        available = FALSE;
        unavailable_reason = "remote_desktop_disabled";
#endif
    } else if (g_str_equal(native_capability->id, "location-agent")) {
        available = control->privacy_location_available;
        if (!available)
            unavailable_reason = "geoclue_unavailable";
    }

    g_variant_builder_init(&capability, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&capability, "{sv}", "id", g_variant_new_string(native_capability->id));
    g_variant_builder_add(&capability, "{sv}", "description",
                          g_variant_new_string(native_capability->description));
    g_variant_builder_add(&capability, "{sv}", "available", g_variant_new_boolean(available));
    if (unavailable_reason)
        g_variant_builder_add(&capability, "{sv}", "reason",
                              g_variant_new_string(unavailable_reason));
    g_variant_builder_add(&capability, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&capability));
}

static GVariant* capability_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder capabilities;
    GVariantBuilder snapshot;
    g_variant_builder_init(&capabilities, G_VARIANT_TYPE("av"));
    for (guint i = 0; i < G_N_ELEMENTS(native_capabilities); i++) {
        g_autoptr(GVariant) capability =
            capability_snapshot_record(control, &native_capabilities[i]);
        g_variant_builder_add_value(&capabilities, g_variant_new_variant(capability));
    }
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "capabilities", g_variant_builder_end(&capabilities));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static const NativeCapability* native_capability_by_id(const char* id) {
    for (guint i = 0; i < G_N_ELEMENTS(native_capabilities); i++) {
        if (g_str_equal(native_capabilities[i].id, id))
            return &native_capabilities[i];
    }
    return NULL;
}

static const char* input_device_type_name(ClutterInputDeviceType type) {
    switch (type) {
    case CLUTTER_POINTER_DEVICE:
        return "pointer";
    case CLUTTER_KEYBOARD_DEVICE:
        return "keyboard";
    case CLUTTER_EXTENSION_DEVICE:
        return "extension";
    case CLUTTER_JOYSTICK_DEVICE:
        return "joystick";
    case CLUTTER_TABLET_DEVICE:
        return "tablet";
    case CLUTTER_TOUCHPAD_DEVICE:
        return "touchpad";
    case CLUTTER_TOUCHSCREEN_DEVICE:
        return "touchscreen";
    case CLUTTER_PEN_DEVICE:
        return "pen";
    case CLUTTER_ERASER_DEVICE:
        return "eraser";
    case CLUTTER_CURSOR_DEVICE:
        return "cursor";
    case CLUTTER_PAD_DEVICE:
        return "pad";
    case CLUTTER_N_DEVICE_TYPES:
        break;
    }
    return "unknown";
}

static void add_input_device_capability(GVariantBuilder* capabilities,
                                        ClutterInputCapabilities available,
                                        ClutterInputCapabilities capability, const char* name) {
    if (available & capability)
        g_variant_builder_add(capabilities, "s", name);
}

static GVariant* input_device_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder devices;
    GVariantBuilder snapshot;
    g_autoptr(GList) device_list = NULL;
    g_variant_builder_init(&devices, G_VARIANT_TYPE("av"));

    if (control->input_seat)
        device_list = clutter_seat_list_devices(control->input_seat);

    for (GList* item = device_list; item; item = item->next) {
        ClutterInputDevice* device = item->data;
        GVariantBuilder record;
        GVariantBuilder capabilities;
        ClutterInputCapabilities available = clutter_input_device_get_capabilities(device);
        const char* name = clutter_input_device_get_device_name(device);
        ClutterSeat* seat = clutter_input_device_get_seat(device);
        guint vendor_id = clutter_input_device_get_vendor_id(device);
        guint product_id = clutter_input_device_get_product_id(device);
        char* id = g_hash_table_lookup(control->input_device_ids, device);

        if (!id) {
            id = g_strdup_printf("input:%" G_GUINT64_FORMAT, ++control->next_input_device_id);
            g_hash_table_insert(control->input_device_ids, device, id);
        }

        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(name ? name : ""));
        g_variant_builder_add(&record, "{sv}", "device_type",
                              g_variant_new_string(input_device_type_name(
                                  clutter_input_device_get_device_type(device))));
        if (seat && clutter_seat_get_name(seat))
            g_variant_builder_add(&record, "{sv}", "seat",
                                  g_variant_new_string(clutter_seat_get_name(seat)));
        if (vendor_id)
            g_variant_builder_add(&record, "{sv}", "vendor_id", g_variant_new_uint32(vendor_id));
        if (product_id)
            g_variant_builder_add(&record, "{sv}", "product_id", g_variant_new_uint32(product_id));

        g_variant_builder_init(&capabilities, G_VARIANT_TYPE("as"));
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_POINTER,
                                    "pointer");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_KEYBOARD,
                                    "keyboard");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TOUCHPAD,
                                    "touchpad");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TOUCH,
                                    "touch");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TABLET_TOOL,
                                    "tablet_tool");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TABLET_PAD,
                                    "tablet_pad");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TRACKBALL,
                                    "trackball");
        add_input_device_capability(&capabilities, available, CLUTTER_INPUT_CAPABILITY_TRACKPOINT,
                                    "trackpoint");
        g_variant_builder_add(&record, "{sv}", "capabilities",
                              g_variant_builder_end(&capabilities));
        g_variant_builder_add(&record, "{sv}", "revision",
                              g_variant_new_int64(control->state_revision));
        g_variant_builder_add_value(&devices,
                                    g_variant_new_variant(g_variant_builder_end(&record)));
    }

    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "devices", g_variant_builder_end(&devices));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void native_input_source_free(gpointer data) {
    NativeInputSource* source = data;
    g_free(source->type);
    g_free(source->id);
    g_free(source->layout);
    g_free(source->variant);
    g_free(source->short_name);
    g_free(source->name);
    g_free(source);
}

static NativeInputSource* input_source_new(const char* id) {
    const char* plus = strchr(id, '+');
    g_autofree char* layout = plus ? g_strndup(id, plus - id) : g_strdup(id);
    const char* variant = plus ? plus + 1 : "";
    if (!*layout || (plus && !*variant) || strchr(variant, '+'))
        return NULL;

    struct rxkb_context* registry = rxkb_context_new(RXKB_CONTEXT_LOAD_EXOTIC_RULES);
    if (!registry)
        return NULL;
    if (!rxkb_context_parse(registry, "evdev")) {
        rxkb_context_unref(registry);
        return NULL;
    }
    gboolean found = FALSE;
    for (struct rxkb_layout* candidate = rxkb_layout_first(registry); candidate;
         candidate = rxkb_layout_next(candidate)) {
        if (g_strcmp0(rxkb_layout_get_name(candidate), layout) == 0 &&
            g_strcmp0(rxkb_layout_get_variant(candidate) ? rxkb_layout_get_variant(candidate) : "",
                      variant) == 0) {
            found = TRUE;
            break;
        }
    }
    rxkb_context_unref(registry);
    if (!found)
        return NULL;

    g_autoptr(MetaKeymapDescription) description =
        meta_keymap_description_new_from_rules(NULL, layout, variant, NULL, NULL, NULL);
    if (!description)
        return NULL;
    g_auto(GStrv) display_names = NULL;
    g_auto(GStrv) short_names = NULL;
    g_autoptr(GError) error = NULL;
    struct xkb_keymap* keymap = meta_keymap_description_create_xkb_keymap(
        description, &display_names, &short_names, &error);
    if (!keymap)
        return NULL;
    xkb_keymap_unref(keymap);

    NativeInputSource* source = g_new0(NativeInputSource, 1);
    source->type = g_strdup("xkb");
    source->id = g_strdup(id);
    source->layout = g_steal_pointer(&layout);
    source->variant = g_strdup(variant);
    source->name = g_strdup(display_names && display_names[0] ? display_names[0] : id);
    source->short_name =
        g_strdup(short_names && short_names[0] && *short_names[0] ? short_names[0] : id);
    return source;
}

static NativeInputSource* ibus_input_source_new(const char* id) {
    if (!id || !*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
        return NULL;
    NativeInputSource* source = g_new0(NativeInputSource, 1);
    source->type = g_strdup("ibus");
    source->id = g_strdup(id);
    /* Engine IDs are stable names from IBus. The optional engine metadata is
     * not needed to select an engine and is intentionally not guessed here. */
    source->name = g_strdup(id);
    source->short_name = g_strdup(id);
    return source;
}

static GVariant* input_source_record_variant(NativeInputSource* source, guint64 revision,
                                             gboolean current) {
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "type", g_variant_new_string(source->type));
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(source->id));
    g_variant_builder_add(&record, "{sv}", "short_name", g_variant_new_string(source->short_name));
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(source->name));
    g_variant_builder_add(&record, "{sv}", "current", g_variant_new_boolean(current));
    g_variant_builder_add(&record, "{sv}", "revision", g_variant_new_int64(revision));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static NativeInputSource* input_source_by_id(GnoblinNativeControl* control, const char* id) {
    for (guint i = 0; control->input_sources && i < control->input_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, i);
        if (g_str_equal(source->id, id))
            return source;
    }
    return NULL;
}

static NativeInputSource* ibus_input_source_by_id(GnoblinNativeControl* control, const char* id) {
    for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
        if (g_str_equal(source->id, id))
            return source;
    }
    return NULL;
}

static NativeInputSource* current_input_source(GnoblinNativeControl* control) {
    if (control->current_ibus_source_id) {
        for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
            NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
            if (g_str_equal(source->id, control->current_ibus_source_id))
                return source;
        }
        return NULL;
    }
    if (!control->backend || !control->input_keymap_description ||
        meta_backend_get_keymap_description(control->backend) !=
            control->input_keymap_description ||
        !control->active_input_source_ids)
        return NULL;
    guint group = meta_backend_get_keymap_layout_group(control->backend);
    if (group >= control->active_input_source_ids->len)
        return NULL;
    return input_source_by_id(control, g_ptr_array_index(control->active_input_source_ids, group));
}

static GVariant* input_source_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder sources;
    GVariantBuilder snapshot;
    g_variant_builder_init(&sources, G_VARIANT_TYPE("av"));
    NativeInputSource* current = current_input_source(control);
    for (guint i = 0; control->input_sources && i < control->input_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, i);
        g_variant_builder_add_value(&sources,
                                    g_variant_new_variant(input_source_record_variant(
                                        source, control->state_revision, source == current)));
    }
    for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
        g_variant_builder_add_value(&sources,
                                    g_variant_new_variant(input_source_record_variant(
                                        source, control->state_revision, source == current)));
    }

    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "sources", g_variant_builder_end(&sources));
    if (current)
        g_variant_builder_add(&snapshot, "{sv}", "current",
                              input_source_record_variant(current, control->state_revision, TRUE));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void add_configured_input_source_id(GPtrArray* ids, const char* id) {
    if (!id || !*id)
        return;
    for (guint i = 0; i < ids->len; i++) {
        if (g_str_equal(g_ptr_array_index(ids, i), id))
            return;
    }
    g_ptr_array_add(ids, g_strdup(id));
}

static GPtrArray* read_configured_input_source_ids(GVariant* document, const char* source_type) {
    GPtrArray* ids = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GVariant) config =
        document ? g_variant_lookup_value(document, "input-sources", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) configured =
        config ? g_variant_lookup_value(config, "sources", G_VARIANT_TYPE("av")) : NULL;
    if (configured) {
        for (gsize i = 0; i < g_variant_n_children(configured); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(configured, i);
            g_autoptr(GVariant) record = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* type = NULL;
            const char* id = NULL;
            if (!g_variant_lookup(record, "type", "&s", &type) ||
                !g_variant_lookup(record, "id", "&s", &id) || !g_str_equal(type, source_type))
                continue;
            add_configured_input_source_id(ids, id);
        }
        return ids;
    }
    return ids;
}

static gboolean input_source_id_lists_equal(GPtrArray* first, GPtrArray* second) {
    if (!first || !second || first->len != second->len)
        return FALSE;
    for (guint i = 0; i < first->len; i++) {
        if (!g_str_equal(g_ptr_array_index(first, i), g_ptr_array_index(second, i)))
            return FALSE;
    }
    return TRUE;
}

static gboolean refresh_input_sources(GnoblinNativeControl* control, GVariant* document) {
    g_autoptr(GVariant) current_document = NULL;
    if (!document) {
        current_document = native_config_document(control);
        document = current_document;
    }
    GPtrArray* xkb_ids = read_configured_input_source_ids(document, "xkb");
    GPtrArray* ibus_ids = read_configured_input_source_ids(document, "ibus");
    gboolean changed =
        !input_source_id_lists_equal(control->configured_input_source_ids, xkb_ids) ||
        !input_source_id_lists_equal(control->configured_ibus_source_ids, ibus_ids);
    if (!changed) {
        g_ptr_array_unref(xkb_ids);
        g_ptr_array_unref(ibus_ids);
        return FALSE;
    }
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->configured_ibus_source_ids, g_ptr_array_unref);
    control->configured_input_source_ids = xkb_ids;
    control->configured_ibus_source_ids = ibus_ids;

    GPtrArray* sources = g_ptr_array_new_with_free_func(native_input_source_free);
    for (guint i = 0; i < xkb_ids->len; i++) {
        const char* id = g_ptr_array_index(xkb_ids, i);
        NativeInputSource* source = input_source_new(id);
        if (source)
            g_ptr_array_add(sources, source);
    }
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    control->input_sources = sources;
    sources = g_ptr_array_new_with_free_func(native_input_source_free);
    for (guint i = 0; i < ibus_ids->len; i++) {
        const char* id = g_ptr_array_index(ibus_ids, i);
        NativeInputSource* source = ibus_input_source_new(id);
        if (source)
            g_ptr_array_add(sources, source);
    }
    g_clear_pointer(&control->ibus_sources, g_ptr_array_unref);
    control->ibus_sources = sources;
    return TRUE;
}

static void pending_input_source_free(PendingInputSource* pending) {
    if (!pending)
        return;
    g_clear_pointer(&pending->description, meta_keymap_description_unref);
    g_clear_pointer(&pending->source_ids, g_ptr_array_unref);
    g_free(pending->selected_type);
    g_free(pending->selected_id);
    g_free(pending->method);
    g_free(pending);
}

static void release_input_source_state(GnoblinNativeControl* control) {
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    g_clear_pointer(&control->ibus_sources, g_ptr_array_unref);
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->configured_ibus_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->active_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->input_keymap_description, meta_keymap_description_unref);
    g_clear_object(&control->appearance_settings);
    g_clear_pointer(&control->last_published_input_source, g_free);
    g_clear_pointer(&control->current_ibus_source_id, g_free);
}

static const char* native_operation_error_code(const GError* error) {
    if (!error)
        return "internal";
    if (error->domain == G_DBUS_ERROR) {
        switch (error->code) {
        case G_DBUS_ERROR_SERVICE_UNKNOWN:
        case G_DBUS_ERROR_NAME_HAS_NO_OWNER:
            return "unavailable";
        case G_DBUS_ERROR_UNKNOWN_METHOD:
        case G_DBUS_ERROR_NOT_SUPPORTED:
            return "unsupported";
        case G_DBUS_ERROR_TIMEOUT:
            return "timed_out";
        case G_DBUS_ERROR_FAILED:
            return "not_found";
        default:
            return "internal";
        }
    }
    if (error->domain != G_IO_ERROR)
        return "internal";
    switch (error->code) {
    case G_IO_ERROR_INVALID_ARGUMENT:
        return "invalid_argument";
    case G_IO_ERROR_NOT_FOUND:
        return "not_found";
    case G_IO_ERROR_EXISTS:
        return "stale";
    case G_IO_ERROR_NOT_SUPPORTED:
        return "unsupported";
    case G_IO_ERROR_BUSY:
        return "busy";
    case G_IO_ERROR_PERMISSION_DENIED:
        return "denied";
    case G_IO_ERROR_CANCELLED:
        return "cancelled";
    case G_IO_ERROR_TIMED_OUT:
        return "timed_out";
    case G_IO_ERROR_NOT_CONNECTED:
    case G_IO_ERROR_CLOSED:
        return "unavailable";
    default:
        return "internal";
    }
}

static void dispatch_operation_completion_full(GnoblinNativeControl* control, gint64 request_id,
                                               const char* method, gboolean ok, JsonNode* result,
                                               const char* error_code, const char* message,
                                               gboolean dispatch_lua) {
    if (request_id <= 0)
        return;

    gboolean cancelled_by_restart =
        control->runtime_cancelled_operation_ids &&
        g_hash_table_remove(control->runtime_cancelled_operation_ids, &request_id);
    if (control->runtime_worker_suspended || cancelled_by_restart) {
        ok = FALSE;
        result = NULL;
        error_code = "unavailable";
        message = "Lua worker restarted before the operation completed";
        dispatch_lua = FALSE;
    }

    const char* failure = message && *message ? message : "Operation failed";
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.operation.completed");
    json_object_set_int_member(object, "operation_id", request_id);
    json_object_set_string_member(object, "method", method);
    json_object_set_boolean_member(object, "ok", ok);
    if (ok && result) {
        json_object_set_member(object, "value", json_node_copy(result));
    } else if (!ok) {
        JsonObject* error = json_object_new();
        json_object_set_string_member(error, "code",
                                      error_code && *error_code ? error_code : "internal");
        json_object_set_string_member(error, "message", failure);
        JsonNode* error_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(error_node, error);
        json_object_set_member(object, "error", error_node);
    }
    json_object_set_int_member(object, "revision", control->state_revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    publish_native_socket_event(control, root);
    (void)dispatch_lua;

    if (control->runtime_operation_ids &&
        g_hash_table_remove(control->runtime_operation_ids, &request_id)) {
        GVariantBuilder completion;
        g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(request_id));
        g_variant_builder_add(&completion, "{sv}", "method", g_variant_new_string(method));
        g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(ok));
        if (ok && result) {
            g_autoptr(GVariant) value = variant_from_json(result);
            if (value)
                g_variant_builder_add(&completion, "{sv}", "value", value);
        } else if (!ok) {
            GVariantBuilder error_record;
            g_variant_builder_init(&error_record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(
                &error_record, "{sv}", "code",
                g_variant_new_string(error_code && *error_code ? error_code : "internal"));
            g_variant_builder_add(&error_record, "{sv}", "message", g_variant_new_string(failure));
            g_variant_builder_add(&completion, "{sv}", "error",
                                  g_variant_builder_end(&error_record));
        }
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&completion));
        g_autoptr(GError) completion_error = NULL;
        if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_COMPLETION, (guint64)request_id,
                                 payload, &completion_error)) {
            native_runtime_abort(control);
            g_warning("gnoblin-native-control: cannot return async operation completion: %s",
                      completion_error ? completion_error->message : "unknown error");
        }
    }

    /* Keep the original socket and Lua event payload during client migration. */
    g_autoptr(JsonNode) legacy = json_node_new(JSON_NODE_OBJECT);
    JsonObject* legacy_object = json_object_new();
    json_node_take_object(legacy, legacy_object);
    json_object_set_string_member(legacy_object, "name", "gnoblin.api.operation-completed");
    json_object_set_int_member(legacy_object, "request_id", request_id);
    json_object_set_string_member(legacy_object, "method", method);
    json_object_set_boolean_member(legacy_object, "ok", ok);
    if (ok && result)
        json_object_set_member(legacy_object, "result", json_node_copy(result));
    else if (!ok)
        json_object_set_string_member(legacy_object, "error", failure);
    json_object_set_int_member(legacy_object, "revision", control->state_revision);
    json_object_set_int_member(legacy_object, "sequence", ++control->event_sequence);
    json_object_set_int_member(legacy_object, "time", g_get_monotonic_time());
    publish_native_socket_event(control, legacy);
    /* Public socket subscribers receive the legacy completion name above.
     * Lua completion callbacks are owned by the supervisor process. */
}

static void dispatch_operation_completion(GnoblinNativeControl* control, gint64 request_id,
                                          const char* method, gboolean ok, JsonNode* result,
                                          const char* error_code, const char* message) {
    dispatch_operation_completion_full(control, request_id, method, ok, result, error_code, message,
                                       TRUE);
}

static void dispatch_input_source_operation(GnoblinNativeControl* control, gint64 request_id,
                                            const char* method, const char* source_type,
                                            gboolean ok, const char* selected_id,
                                            const char* error_code, const char* message) {
    g_autoptr(JsonNode) result = NULL;
    if (ok && selected_id) {
        NativeInputSource* source = g_str_equal(source_type, "ibus")
                                        ? ibus_input_source_by_id(control, selected_id)
                                        : input_source_by_id(control, selected_id);
        if (source) {
            g_autoptr(GVariant) record =
                input_source_record_variant(source, control->state_revision, TRUE);
            result = json_from_variant(record);
        }
    }
    dispatch_operation_completion(control, request_id, method, ok, result, error_code, message);
}

static char* ibus_engine_name_from_value(GVariant* value) {
    if (!value)
        return NULL;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unwrapped = g_variant_get_variant(value);
        return ibus_engine_name_from_value(unwrapped);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        const char* name = NULL;
        if (g_variant_lookup(value, "name", "&s", &name) && name && *name)
            return g_strdup(name);
        return NULL;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_TUPLE) && g_variant_n_children(value) >= 3) {
        g_autoptr(GVariant) type_value = g_variant_get_child_value(value, 0);
        if (g_variant_is_of_type(type_value, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(type_value, NULL), "IBusEngineDesc")) {
            g_autoptr(GVariant) name_value = g_variant_get_child_value(value, 2);
            if (g_variant_is_of_type(name_value, G_VARIANT_TYPE_STRING)) {
                const char* name = g_variant_get_string(name_value, NULL);
                return *name ? g_strdup(name) : NULL;
            }
        }
    }
    if (g_variant_is_container(value)) {
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            char* name = ibus_engine_name_from_value(child);
            if (name)
                return name;
        }
    }
    return NULL;
}

static char* ibus_bus_address(void) {
    const char* address = g_getenv("IBUS_ADDRESS");
    if (address && *address)
        return g_strdup(address);

    const char* display = g_getenv("WAYLAND_DISPLAY");
    if (!display || !*display)
        display = g_getenv("DISPLAY");
    if (!display || !*display)
        return NULL;

    g_autofree char* display_name = g_path_get_basename(display);
    if (display_name[0] == ':')
        memmove(display_name, display_name + 1, strlen(display_name));
    g_autofree char* machine_id = NULL;
    if (!g_file_get_contents("/etc/machine-id", &machine_id, NULL, NULL) || !machine_id) {
        g_clear_pointer(&machine_id, g_free);
        if (!g_file_get_contents("/var/lib/dbus/machine-id", &machine_id, NULL, NULL))
            return NULL;
    }
    g_strstrip(machine_id);
    if (!*machine_id)
        return NULL;
    g_autofree char* filename = g_strdup_printf("%s-unix-%s", machine_id, display_name);
    const char* address_dirs[] = {g_get_user_cache_dir(), g_get_user_config_dir()};
    for (guint i = 0; i < G_N_ELEMENTS(address_dirs); i++) {
        g_autofree char* path = g_build_filename(address_dirs[i], "ibus", "bus", filename, NULL);
        g_autofree char* contents = NULL;
        if (!g_file_get_contents(path, &contents, NULL, NULL))
            continue;

        g_auto(GStrv) lines = g_strsplit(contents, "\n", -1);
        for (char** line = lines; *line; line++) {
            if (!g_str_has_prefix(*line, "IBUS_ADDRESS="))
                continue;
            const char* value = *line + strlen("IBUS_ADDRESS=");
            return *value ? g_strdup(value) : NULL;
        }
    }
    g_debug("gnoblin-native-control: IBus address file was not found under the XDG cache or config "
            "directories");
    return NULL;
}

static GDBusConnection* connect_ibus_bus(void) {
    g_autofree char* address = ibus_bus_address();
    if (!address)
        return NULL;

    g_autoptr(GError) error = NULL;
    GDBusConnection* connection =
        g_dbus_connection_new_for_address_sync(address,
                                               G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                                                   G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION,
                                               NULL, NULL, &error);
    if (!connection)
        g_debug("gnoblin-native-control: IBus is unavailable: %s", error->message);
    return connection;
}

static gboolean ibus_retry_connection(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->ibus_retry_source_id = 0;
    if (!control->stopping)
        start_ibus_input_source_tracking(control);
    return G_SOURCE_REMOVE;
}

static void schedule_ibus_connection_retry(GnoblinNativeControl* control) {
    if (control->stopping || control->ibus_bus || control->ibus_retry_source_id)
        return;
    control->ibus_retry_source_id = g_timeout_add_seconds(1, ibus_retry_connection, control);
}

static void ibus_bus_closed(GDBusConnection* connection, gboolean remote_peer_vanished,
                            GError* error, gpointer user_data) {
    (void)remote_peer_vanished;
    (void)error;
    GnoblinNativeControl* control = user_data;
    if (control->stopping || control->ibus_bus != connection)
        return;
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    if (control->ibus_signal_subscription_id) {
        g_dbus_connection_signal_unsubscribe(connection, control->ibus_signal_subscription_id);
        control->ibus_signal_subscription_id = 0;
    }
    if (control->ibus_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(connection, control->ibus_owner_subscription_id);
        control->ibus_owner_subscription_id = 0;
    }
    g_signal_handlers_disconnect_by_data(connection, control);
    g_clear_object(&control->ibus_bus);
    g_clear_pointer(&control->current_ibus_source_id, g_free);
    publish_input_source_changes(control, control->state_revision);
    schedule_ibus_connection_retry(control);
}

static void ibus_global_engine_query_done(GObject* source_object, GAsyncResult* result,
                                          gpointer user_data) {
    PendingIBusQuery* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);
    control->pending_ibus_queries--;
    gboolean current = !control->stopping &&
                       control->ibus_bus == G_DBUS_CONNECTION(source_object) &&
                       control->ibus_owner_generation == pending->ibus_owner_generation &&
                       control->ibus_engine_generation == pending->ibus_engine_generation;
    if (current) {
        g_autofree char* engine = reply ? ibus_engine_name_from_value(reply) : NULL;
        g_free(control->current_ibus_source_id);
        control->current_ibus_source_id = g_steal_pointer(&engine);
        publish_input_source_changes(control, control->state_revision);
    }
    g_free(pending);
    native_control_maybe_free_stopped(control);
}

static void query_ibus_global_engine(GnoblinNativeControl* control) {
    if (!control || control->stopping || !control->ibus_bus)
        return;
    PendingIBusQuery* pending = g_new0(PendingIBusQuery, 1);
    pending->control = control;
    pending->ibus_owner_generation = control->ibus_owner_generation;
    pending->ibus_engine_generation = control->ibus_engine_generation;
    control->pending_ibus_queries++;
    g_dbus_connection_call(control->ibus_bus, IBUS_BUS_NAME, IBUS_OBJECT_PATH, IBUS_INTERFACE,
                           "GetGlobalEngine", NULL, G_VARIANT_TYPE("(v)"), G_DBUS_CALL_FLAGS_NONE,
                           1000, NULL, ibus_global_engine_query_done, pending);
}

static void ibus_global_engine_changed(GDBusConnection* connection, const char* sender_name,
                                       const char* object_path, const char* interface_name,
                                       const char* signal_name, GVariant* parameters,
                                       gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    const char* engine = NULL;
    if (control->stopping || control->ibus_bus != connection)
        return;
    control->ibus_engine_generation++;
    g_variant_get(parameters, "(&s)", &engine);
    g_free(control->current_ibus_source_id);
    control->current_ibus_source_id = *engine ? g_strdup(engine) : NULL;
    publish_input_source_changes(control, control->state_revision);
}

static void ibus_name_owner_changed(GDBusConnection* connection, const char* sender_name,
                                    const char* object_path, const char* interface_name,
                                    const char* signal_name, GVariant* parameters,
                                    gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    const char* name = NULL;
    const char* old_owner = NULL;
    const char* new_owner = NULL;
    g_variant_get(parameters, "(&s&s&s)", &name, &old_owner, &new_owner);
    if (control->stopping || control->ibus_bus != connection || !g_str_equal(name, IBUS_BUS_NAME) ||
        g_str_equal(old_owner, new_owner))
        return;
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    g_clear_pointer(&control->current_ibus_source_id, g_free);
    publish_input_source_changes(control, control->state_revision);
    if (*new_owner)
        query_ibus_global_engine(control);
}

static void start_ibus_input_source_tracking(GnoblinNativeControl* control) {
    if (!control || control->stopping || control->ibus_bus)
        return;
    control->ibus_bus = connect_ibus_bus();
    if (!control->ibus_bus) {
        schedule_ibus_connection_retry(control);
        return;
    }
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    g_signal_connect(control->ibus_bus, "closed", G_CALLBACK(ibus_bus_closed), control);
    control->ibus_signal_subscription_id = g_dbus_connection_signal_subscribe(
        control->ibus_bus, IBUS_BUS_NAME, IBUS_INTERFACE, "GlobalEngineChanged", IBUS_OBJECT_PATH,
        NULL, G_DBUS_SIGNAL_FLAGS_NONE, ibus_global_engine_changed, control, NULL);
    control->ibus_owner_subscription_id = g_dbus_connection_signal_subscribe(
        control->ibus_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", IBUS_BUS_NAME, G_DBUS_SIGNAL_FLAGS_NONE, ibus_name_owner_changed,
        control, NULL);
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        control->ibus_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetNameOwner", g_variant_new("(s)", IBUS_BUS_NAME), G_VARIANT_TYPE("(s)"),
        G_DBUS_CALL_FLAGS_NONE, 250, NULL, NULL);
    if (owner)
        query_ibus_global_engine(control);
}

static void pending_portal_grant_operation_free(PendingPortalGrantOperation* pending) {
    if (!pending)
        return;
    g_free(pending->method);
    g_free(pending->kind);
    g_free(pending->id);
    g_free(pending);
}

static void native_control_maybe_free_stopped(GnoblinNativeControl* control) {
    /* Runtime abort can precede compositor teardown while async callbacks drain. */
    if (!control || !control->stopping || !control->teardown_complete ||
        control->pending_input_source_ops != 0 || control->pending_ibus_queries != 0 ||
        control->pending_portal_grant_ops != 0 || control->pending_thumbnail_count != 0 ||
        control->pending_activity_queries != 0)
        return;
    release_input_source_state(control);
    g_clear_pointer(&control->session_activity_snapshot, g_variant_unref);
    g_free(control);
}

static void clear_pending_grant_delivery(GnoblinNativeControl* control, gint64 request_id) {
    if (!control || request_id <= 0 || !control->clients)
        return;
    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
    GHashTableIter iter;
    gpointer client_value;
    g_hash_table_iter_init(&iter, control->clients);
    while (g_hash_table_iter_next(&iter, &client_value, NULL)) {
        Client* client = client_value;
        if (client->pending_grant_operations)
            g_hash_table_remove(client->pending_grant_operations, key);
    }
}

static GVariant* portal_grant_list_result(GVariant* grants) {
    GVariantBuilder records;
    GVariantBuilder result;
    GVariantIter iter;
    const char* id;
    const char* kind;
    const char* requester;
    guint32 devices;
    gboolean clipboard;
    gboolean screen_streams;

    g_variant_builder_init(&records, G_VARIANT_TYPE("aa{sv}"));
    g_variant_iter_init(&iter, grants);
    while (g_variant_iter_next(&iter, "(&s&s&subb)", &id, &kind, &requester, &devices, &clipboard,
                               &screen_streams)) {
        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string(kind));
        g_variant_builder_add(&record, "{sv}", "requester", g_variant_new_string(requester));
        g_variant_builder_add(&record, "{sv}", "devices", g_variant_new_uint32(devices));
        g_variant_builder_add(&record, "{sv}", "clipboard", g_variant_new_boolean(clipboard));
        g_variant_builder_add(&record, "{sv}", "screenStreams",
                              g_variant_new_boolean(screen_streams));
        g_variant_builder_add_value(&records, g_variant_builder_end(&record));
    }

    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "grants", g_variant_builder_end(&records));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* portal_grant_revoke_result(const char* id) {
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "ok", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void portal_grant_call_done(GObject* source_object, GAsyncResult* result,
                                   gpointer user_data) {
    PendingPortalGrantOperation* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);

    if (control->stopping) {
        clear_pending_grant_delivery(control, pending->operation_id);
    } else if (!reply) {
        gboolean dispatch_lua = native_config_generation(control) == pending->runtime_generation;
        g_autofree char* remote_error = error ? g_dbus_error_get_remote_error(error) : NULL;
        g_autoptr(GError) operation_error = NULL;
        if (remote_error &&
            (g_str_equal(remote_error, "org.freedesktop.DBus.Error.InvalidArgs") ||
             g_str_equal(remote_error, "org.freedesktop.DBus.Error.InvalidArgument")))
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "portal rejected the grant request arguments");
        else if (remote_error &&
                 g_str_equal(remote_error, "org.gnoblin.Portal.Grants.Error.NotFound"))
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "portal grant was not found");
        else if (remote_error && g_str_equal(remote_error, "org.gnoblin.Portal.Grants.Error.Stale"))
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                                "portal grant snapshot is stale; refresh the grant list");
        else if (remote_error &&
                 (g_str_equal(remote_error, "org.freedesktop.DBus.Error.AccessDenied") ||
                  g_str_equal(remote_error, "org.gnoblin.Portal.Grants.Error.AccessDenied")))
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "portal denied the grant request");
        else if (error && (g_error_matches(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT) ||
                           g_error_matches(error, G_IO_ERROR, G_IO_ERROR_CANCELLED)))
            operation_error = g_error_copy(error);
        else {
            g_autofree char* message =
                g_strdup(error ? error->message : "portal grant service is unavailable");
            g_set_error(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED, "%s", message);
        }
        dispatch_operation_completion_full(
            control, pending->operation_id, pending->method, FALSE, NULL,
            native_operation_error_code(operation_error),
            operation_error ? operation_error->message : "portal request failed", dispatch_lua);
    } else {
        gboolean dispatch_lua = native_config_generation(control) == pending->runtime_generation;
        g_autoptr(GVariant) value = NULL;
        if (g_str_equal(pending->method, "grant.list")) {
            g_autoptr(GVariant) grants = NULL;
            if (g_variant_is_of_type(reply, G_VARIANT_TYPE("(a(sssubb))")))
                g_variant_get(reply, "(@a(sssubb))", &grants);
            if (!grants) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "portal returned an invalid grant list");
            } else {
                value = portal_grant_list_result(grants);
            }
        } else if (g_variant_is_of_type(reply, G_VARIANT_TYPE_UNIT)) {
            value = portal_grant_revoke_result(pending->id);
        } else {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "portal returned an invalid revoke response");
        }
        g_autoptr(JsonNode) json_value = value ? json_from_variant(value) : NULL;
        dispatch_operation_completion_full(control, pending->operation_id, pending->method,
                                           value != NULL, json_value,
                                           value ? NULL : native_operation_error_code(error),
                                           value ? NULL : error->message, dispatch_lua);
    }

    control->pending_portal_grant_ops--;
    pending_portal_grant_operation_free(pending);
    native_control_maybe_free_stopped(control);
}

static void portal_grant_bus_ready(GObject* source_object, GAsyncResult* result,
                                   gpointer user_data) {
    PendingPortalGrantOperation* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_finish(result, &error);

    if (control->stopping) {
        clear_pending_grant_delivery(control, pending->operation_id);
        control->pending_portal_grant_ops--;
        pending_portal_grant_operation_free(pending);
        native_control_maybe_free_stopped(control);
        return;
    }
    if (!connection) {
        dispatch_operation_completion_full(control, pending->operation_id, pending->method, FALSE,
                                           NULL, native_operation_error_code(error),
                                           error ? error->message : "session bus is unavailable",
                                           native_config_generation(control) ==
                                               pending->runtime_generation);
        control->pending_portal_grant_ops--;
        pending_portal_grant_operation_free(pending);
        native_control_maybe_free_stopped(control);
        return;
    }

    if (g_str_equal(pending->method, "grant.list")) {
        g_dbus_connection_call(
            connection, "org.freedesktop.impl.portal.desktop.gnoblin", "/org/gnoblin/Portal/Grants",
            "org.gnoblin.Portal.Grants", "ListPortalGrants", NULL, G_VARIANT_TYPE("(a(sssubb))"),
            G_DBUS_CALL_FLAGS_NONE, PORTAL_GRANT_TIMEOUT_MS, NULL, portal_grant_call_done, pending);
    } else if (pending->has_expected_created_at) {
        g_dbus_connection_call(
            connection, PORTAL_BACKEND_BUS_NAME, "/org/gnoblin/Portal/Grants",
            "org.gnoblin.Portal.Grants", "RevokePortalGrantIfCurrent",
            g_variant_new("(sst)", pending->kind, pending->id, pending->expected_created_at_ms),
            G_VARIANT_TYPE_UNIT, G_DBUS_CALL_FLAGS_NONE, PORTAL_GRANT_TIMEOUT_MS, NULL,
            portal_grant_call_done, pending);
    } else {
        g_dbus_connection_call(
            connection, "org.freedesktop.impl.portal.desktop.gnoblin", "/org/gnoblin/Portal/Grants",
            "org.gnoblin.Portal.Grants", "RevokePortalGrant",
            g_variant_new("(ss)", pending->kind, pending->id), G_VARIANT_TYPE_UNIT,
            G_DBUS_CALL_FLAGS_NONE, PORTAL_GRANT_TIMEOUT_MS, NULL, portal_grant_call_done, pending);
    }
}

gboolean gnoblin_native_control_portal_grant_operation(MetaDisplay* display, const char* method,
                                                       GVariant* arguments, gint64 request_id,
                                                       GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !method || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) || request_id <= 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "portal grant operation is unavailable or malformed");
        return FALSE;
    }

    const char* kind = NULL;
    const char* id = NULL;
    guint64 expected_created_at_ms = 0;
    gboolean has_expected_created_at = FALSE;
    if (g_str_equal(method, "grant.list")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "grant.list accepts no arguments");
            return FALSE;
        }
    } else if (g_str_equal(method, "grant.revoke")) {
        GVariant* created_at_value = g_variant_lookup_value(arguments, "created_at", NULL);
        gboolean has_expected = created_at_value != NULL;
        has_expected_created_at = has_expected;
        if (g_variant_n_children(arguments) != (has_expected ? 3u : 2u) ||
            !g_variant_lookup(arguments, "kind", "&s", &kind) ||
            !g_variant_lookup(arguments, "id", "&s", &id) || !*id ||
            (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")) ||
            (has_expected && !g_variant_is_of_type(created_at_value, G_VARIANT_TYPE_INT64)) ||
            (has_expected && g_variant_get_int64(created_at_value) < 0)) {
            g_clear_pointer(&created_at_value, g_variant_unref);
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "grant.revoke requires only a supported kind and opaque id");
            return FALSE;
        }
        if (has_expected)
            expected_created_at_ms = (guint64)g_variant_get_int64(created_at_value);
        g_clear_pointer(&created_at_value, g_variant_unref);
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "unsupported portal grant operation");
        return FALSE;
    }

    PendingPortalGrantOperation* pending = g_new0(PendingPortalGrantOperation, 1);
    pending->control = control;
    pending->operation_id = request_id;
    pending->runtime_generation = native_config_generation(control);
    pending->method = g_strdup(method);
    pending->kind = g_strdup(kind);
    pending->id = g_strdup(id);
    pending->has_expected_created_at = has_expected_created_at;
    pending->expected_created_at_ms = expected_created_at_ms;
    control->pending_portal_grant_ops++;
    g_bus_get(G_BUS_TYPE_SESSION, NULL, portal_grant_bus_ready, pending);
    return TRUE;
}

static void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete,
                                         gboolean ok, const char* accelerator,
                                         const char* error_code, const char* message) {
    if (!control || !control->shortcut_capture_active)
        return;

    control->shortcut_capture_active = FALSE;
    control->shortcut_capture_super_pressed = FALSE;
    if (control->shortcut_capture_timeout_id) {
        g_source_remove(control->shortcut_capture_timeout_id);
        control->shortcut_capture_timeout_id = 0;
    }
    meta_display_stop_native_key_capture(control->display);
    meta_display_unregister_native_key_capture_handler(control->display, control);

    gint64 request_id = control->shortcut_capture_request_id;
    control->shortcut_capture_request_id = 0;
    if (!complete || request_id <= 0 || control->stopping)
        return;

    g_autoptr(JsonNode) result = NULL;
    if (ok && accelerator) {
        JsonObject* result_object = json_object_new();
        json_object_set_string_member(result_object, "accelerator", accelerator);
        result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, result_object);
    }
    dispatch_operation_completion(control, request_id, "shortcut.capture", ok, result, error_code,
                                  message);
}

static gboolean native_shortcut_capture_timed_out(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->shortcut_capture_timeout_id = 0;
    stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "timed_out",
                                 "shortcut capture timed out");
    return G_SOURCE_REMOVE;
}

static gboolean native_keysym_is_modifier(guint keyval) {
    switch (keyval) {
    case XKB_KEY_Shift_L:
    case XKB_KEY_Shift_R:
    case XKB_KEY_Control_L:
    case XKB_KEY_Control_R:
    case XKB_KEY_Caps_Lock:
    case XKB_KEY_Shift_Lock:
    case XKB_KEY_Meta_L:
    case XKB_KEY_Meta_R:
    case XKB_KEY_Alt_L:
    case XKB_KEY_Alt_R:
    case XKB_KEY_Super_L:
    case XKB_KEY_Super_R:
    case XKB_KEY_Hyper_L:
    case XKB_KEY_Hyper_R:
        return TRUE;
    default:
        return FALSE;
    }
}

static gboolean native_shortcut_hold_family_pressed(GnoblinNativeControl* control,
                                                    guint32 hold_mask) {
    if (!control)
        return FALSE;
    if ((hold_mask & CLUTTER_SUPER_MASK) &&
        (control->super_left_pressed || control->super_right_pressed))
        return TRUE;
    if ((hold_mask & CLUTTER_CONTROL_MASK) &&
        (control->control_left_pressed || control->control_right_pressed))
        return TRUE;
    if ((hold_mask & CLUTTER_MOD1_MASK) &&
        (control->alt_left_pressed || control->alt_right_pressed))
        return TRUE;
    return FALSE;
}

static void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (control->shortcut_session_capture_active && control->active_shortcut_session) {
        NativeDynamicShortcut* shortcut = control->active_shortcut_session;
        if (!event) {
            dynamic_shortcut_end_session(control, shortcut, "preempted");
            return;
        }
        ClutterEventType type = clutter_event_type(event);
        if (type != CLUTTER_KEY_PRESS && type != CLUTTER_KEY_RELEASE)
            return;
        ClutterEventFlags flags = clutter_event_get_flags(event);
        if (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD))
            return;
        guint keyval = clutter_event_get_key_symbol(event);
        if (shortcut == control->bare_super_shortcut && type == CLUTTER_KEY_RELEASE &&
            (keyval == XKB_KEY_Super_L || keyval == XKB_KEY_Super_R) &&
            !native_shortcut_hold_family_pressed(control, CLUTTER_SUPER_MASK)) {
            if (shortcut->trigger_event) {
                shortcut->releasing = TRUE;
                dispatch_dynamic_shortcut_activated(control, 0, shortcut->trigger_event);
                shortcut->releasing = FALSE;
                clutter_event_free(shortcut->trigger_event);
                shortcut->trigger_event = NULL;
            }
            dynamic_shortcut_end_session(control, shortcut, "released");
            return;
        }
        guint32 modifiers = clutter_event_get_state(event);
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(shortcut->id));
        g_variant_builder_add(&builder, "{sv}", "session_id",
                              g_variant_new_uint64(shortcut->session_id));
        g_variant_builder_add(&builder, "{sv}", "keyval", g_variant_new_uint32(keyval));
        g_variant_builder_add(&builder, "{sv}", "keycode",
                              g_variant_new_uint32(clutter_event_get_key_code(event)));
        g_variant_builder_add(&builder, "{sv}", "modifiers", g_variant_new_uint32(modifiers));
        g_variant_builder_add(
            &builder, "{sv}", "phase",
            g_variant_new_string(type == CLUTTER_KEY_PRESS ? "press" : "release"));
        g_variant_builder_add(&builder, "{sv}", "time",
                              g_variant_new_int64(clutter_event_get_time(event)));
        guint64 focus_handle = 0;
        guint64 focus_generation = 0;
        gint64 focus_expires_at_us = 0;
        char focus_token[65] = {0};
        if (!(flags & CLUTTER_EVENT_FLAG_REPEATED) &&
            issue_focus_context_for_session_key(control, shortcut, event, &focus_handle,
                                                &focus_generation, &focus_expires_at_us,
                                                focus_token)) {
            if (shortcut->client) {
                g_variant_builder_add(&builder, "{sv}", "focus_context",
                                      g_variant_new_string(focus_token));
            } else {
                g_variant_builder_add(&builder, "{sv}", "focus_context_handle",
                                      g_variant_new_uint64(focus_handle));
                g_variant_builder_add(&builder, "{sv}", "focus_context_generation",
                                      g_variant_new_uint64(focus_generation));
                g_variant_builder_add(&builder, "{sv}", "focus_context_expires_at_us",
                                      g_variant_new_int64(focus_expires_at_us));
            }
        }
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
        dynamic_shortcut_publish_event(control, shortcut, "gnoblin.shortcut.session.key", payload);
        if (type == CLUTTER_KEY_RELEASE && shortcut->hold_mask &&
            !native_shortcut_hold_family_pressed(control, shortcut->hold_mask))
            dynamic_shortcut_end_session(control, shortcut, "released");
        return;
    }
    if (!control->shortcut_capture_active)
        return;
    if (!event) {
        stop_native_shortcut_capture(
            control, TRUE, FALSE, NULL, "busy",
            "shortcut capture cancelled because keyboard input was grabbed");
        return;
    }

    guint keyval = clutter_event_get_key_symbol(event);
    ClutterEventType type = clutter_event_type(event);
    if (type == CLUTTER_KEY_PRESS && keyval == CLUTTER_KEY_Escape) {
        stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "cancelled",
                                     "shortcut capture cancelled");
        return;
    }
    if ((keyval == CLUTTER_KEY_Super_L || keyval == CLUTTER_KEY_Super_R) &&
        type == CLUTTER_KEY_PRESS) {
        control->shortcut_capture_super_pressed = TRUE;
        return;
    }
    if ((keyval == CLUTTER_KEY_Super_L || keyval == CLUTTER_KEY_Super_R) &&
        type == CLUTTER_KEY_RELEASE && control->shortcut_capture_super_pressed) {
        control->shortcut_capture_super_pressed = FALSE;
        if (!(clutter_event_get_state(event) & (CLUTTER_MODIFIER_MASK & ~CLUTTER_SUPER_MASK)))
            stop_native_shortcut_capture(control, TRUE, TRUE, "Super", NULL, NULL);
        return;
    }
    if (type != CLUTTER_KEY_PRESS || native_keysym_is_modifier(keyval))
        return;

    control->shortcut_capture_super_pressed = FALSE;
    g_autofree char* accelerator = meta_accelerator_name(clutter_event_get_state(event), keyval);
    if (!accelerator || !*accelerator)
        return;

    stop_native_shortcut_capture(control, TRUE, TRUE, accelerator, NULL, NULL);
}

gboolean gnoblin_native_control_overlay_modifier_pressed(MetaDisplay* display,
                                                         const ClutterEvent* event) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeDynamicShortcut* shortcut = control ? control->bare_super_shortcut : NULL;
    if (!control || control->stopping || !control->overlay_modifier_hook_available || !shortcut ||
        !shortcut->armed || !shortcut->capture_input || shortcut->active || !event ||
        clutter_event_type(event) != CLUTTER_KEY_PRESS ||
        (clutter_event_get_flags(event) &
         (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD |
          CLUTTER_EVENT_FLAG_REPEATED)))
        return FALSE;
    guint keyval = clutter_event_get_key_symbol(event);
    if (keyval != XKB_KEY_Super_L && keyval != XKB_KEY_Super_R)
        return FALSE;
    if (clutter_event_get_state(event) & (CLUTTER_MODIFIER_MASK & ~CLUTTER_SUPER_MASK))
        return FALSE;
    if (control->shortcut_capture_active || control->active_shortcut_session ||
        !meta_display_can_start_native_key_capture(display) ||
        !meta_display_register_native_key_capture_handler(display, native_shortcut_capture_key,
                                                          control, NULL))
        return FALSE;

    if (!shortcut->session_id) {
        shortcut->session_id = ++control->next_shortcut_session_id;
        if (!shortcut->session_id)
            shortcut->session_id = ++control->next_shortcut_session_id;
    }
    shortcut->active = TRUE;
    shortcut->activation_dispatched = FALSE;
    shortcut->held_mask = CLUTTER_SUPER_MASK;
    shortcut->trigger_event = clutter_event_copy(event);
    shortcut->timeout_id = g_timeout_add_seconds(10, native_shortcut_session_timeout, shortcut);
    control->active_shortcut_session = shortcut;
    control->shortcut_session_capture_active = TRUE;
    if (!meta_display_start_native_key_capture(display)) {
        control->shortcut_session_capture_active = FALSE;
        control->active_shortcut_session = NULL;
        shortcut->active = FALSE;
        if (shortcut->timeout_id) {
            g_source_remove(shortcut->timeout_id);
            shortcut->timeout_id = 0;
        }
        clutter_event_free(shortcut->trigger_event);
        shortcut->trigger_event = NULL;
        meta_display_unregister_native_key_capture_handler(display, control);
        return FALSE;
    }
    return TRUE;
}

void gnoblin_native_control_set_overlay_modifier_hook_available(MetaDisplay* display,
                                                                gboolean available) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control)
        return;
    control->overlay_modifier_hook_available = available;
    if (!available && control->bare_super_shortcut) {
        NativeDynamicShortcut* shortcut = control->bare_super_shortcut;
        dynamic_shortcut_end_session(control, shortcut, "preempted");
        control->bare_super_shortcut = NULL;
        native_dynamic_shortcut_free(shortcut);
    }
}

void gnoblin_native_control_cancel_shortcut_session(MetaDisplay* display, guint64 session_id,
                                                    const char* reason) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || !control->active_shortcut_session || !session_id ||
        control->active_shortcut_session->session_id != session_id)
        return;
    dynamic_shortcut_end_session(control, control->active_shortcut_session,
                                 reason ? reason : "cancelled");
}

gboolean gnoblin_native_control_begin_shortcut_capture(MetaDisplay* display, GVariant* arguments,
                                                       gint64 request_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || request_id <= 0 || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "native shortcut capture is unavailable");
        return FALSE;
    }
    if (control->shortcut_capture_active) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another shortcut capture is active");
        return FALSE;
    }
    g_autoptr(JsonNode) json = json_from_variant(arguments);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture accepts an optional integer timeout");
        return FALSE;
    }
    JsonObject* capture_options = json_node_get_object(json);
    if (json_object_get_size(capture_options) > 1 ||
        (json_object_get_size(capture_options) == 1 &&
         !json_object_has_member(capture_options, "timeout"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture accepts only timeout");
        return FALSE;
    }
    JsonNode* timeout_node = json_object_get_member(capture_options, "timeout");
    if (timeout_node && (!JSON_NODE_HOLDS_VALUE(timeout_node) ||
                         (json_node_get_value_type(timeout_node) != G_TYPE_INT &&
                          json_node_get_value_type(timeout_node) != G_TYPE_INT64))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.capture timeout must be an integer");
        return FALSE;
    }
    gint64 timeout_seconds = 30;
    if (timeout_node)
        timeout_seconds = json_node_get_int(timeout_node);
    if (timeout_seconds < 1 || timeout_seconds > 60) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut capture timeout must be from 1 to 60 seconds");
        return FALSE;
    }
    if (!control->wayland_compositor ||
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "shortcut capture is unavailable while the session is locked");
        return FALSE;
    }
    if (!meta_display_can_start_native_key_capture(display)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "keyboard input is already grabbed or captured");
        return FALSE;
    }
    if (!meta_display_register_native_key_capture_handler(display, native_shortcut_capture_key,
                                                          control, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input capture owner is registered");
        return FALSE;
    }
    control->shortcut_capture_request_id = request_id;
    control->shortcut_capture_active = TRUE;
    if (!meta_display_start_native_key_capture(display)) {
        control->shortcut_capture_active = FALSE;
        control->shortcut_capture_request_id = 0;
        meta_display_unregister_native_key_capture_handler(display, control);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input capture owner is active");
        return FALSE;
    }
    control->shortcut_capture_timeout_id =
        g_timeout_add_seconds((guint)timeout_seconds, native_shortcut_capture_timed_out, control);
    return TRUE;
}

static void native_cancel_window_drags(GnoblinNativeControl* control, const char* reason) {
    if (!control || !control->window_drags)
        return;
    GArray* ids = g_array_new(FALSE, FALSE, sizeof(guint64));
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->window_drags);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        guint64 id = *(guint64*)key;
        g_array_append_val(ids, id);
    }
    for (guint i = 0; i < ids->len; i++)
        gnoblin_native_control_window_drag_end(control->display, g_array_index(ids, guint64, i),
                                               FALSE, reason);
    g_array_unref(ids);
}

static void native_window_drag_client_disconnected(GnoblinNativeControl* control,
                                                   guint64 client_id) {
    if (!control || !control->window_drags)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->window_drags);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeWindowDrag* drag = value;
        if (drag->client_tokens)
            g_hash_table_remove(drag->client_tokens, &client_id);
        if (drag->socket_owner_client_id == client_id) {
            drag->socket_owner_client_id = 0;
            drag->runtime_offer_claimed = FALSE;
            g_ptr_array_set_size(drag->targets, 0);
        }
    }
}

static const char* native_session_lock_state_name(MetaWaylandSessionLockState state) {
    switch (state) {
    case META_WAYLAND_SESSION_LOCK_UNLOCKED:
        return "unlocked";
    case META_WAYLAND_SESSION_LOCK_COVERING:
        return "covering";
    case META_WAYLAND_SESSION_LOCK_LOCKED:
        return "locked";
    case META_WAYLAND_SESSION_LOCK_FAILSAFE:
    default:
        return "failsafe";
    }
}

static GVariant* native_session_lock_snapshot(MetaWaylandCompositor* compositor, guint64 revision) {
    gboolean available = compositor && meta_wayland_session_lock_get_capability(compositor) != 0;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "lock_available", g_variant_new_boolean(available));
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_uint64(revision));
    if (available)
        g_variant_builder_add(&builder, "{sv}", "lock_state",
                              g_variant_new_string(native_session_lock_state_name(
                                  meta_wayland_session_lock_get_state(compositor))));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static void native_session_lock_changed(MetaWaylandCompositor* compositor,
                                        MetaWaylandSessionLockState state, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping)
        return;
    if (state != META_WAYLAND_SESSION_LOCK_UNLOCKED) {
        gnoblin_native_control_revoke_focus_contexts(control->display);
        native_cancel_window_drags(control, "locked");
        g_hash_table_remove_all(control->snap_contexts);
        if (control->active_shortcut_session)
            dynamic_shortcut_end_session(control, control->active_shortcut_session, "locked");
        stop_native_shortcut_capture(user_data, TRUE, FALSE, NULL, "denied",
                                     "shortcut capture cancelled because the session locked");
    }
    const char* state_name = native_session_lock_state_name(state);

    guint64 sequence = ++control->event_sequence;
    control->session_lock_revision = sequence;
    gint64 time = g_get_monotonic_time();
    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "state", g_variant_new_string(state_name));
    g_variant_builder_add(&payload_builder, "{sv}", "revision", g_variant_new_uint64(sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "sequence", g_variant_new_int64(sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "time", g_variant_new_int64(time));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    g_autoptr(GVariant) status_snapshot =
        native_session_lock_snapshot(compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", status_snapshot, sequence);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.session.lock-state-changed");
    json_object_set_string_member(object, "state", state_name);
    json_object_set_int_member(object, "revision", sequence);
    json_object_set_int_member(object, "sequence", sequence);
    json_object_set_int_member(object, "time", time);
    publish_native_socket_event(control, root);
    native_runtime_dispatch_event(control, "gnoblin.session.lock-state-changed", payload);
}

GVariant* gnoblin_native_control_request_session_lock(MetaDisplay* display, GVariant* arguments,
                                                      GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* const event_name = "gnoblin.session.lock-requested";
    guint listeners = 0;
    GHashTableIter iterator;
    gpointer value;

    if (!control || control->stopping || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "session.lock does not accept arguments");
        return NULL;
    }
    if (!control->wayland_compositor ||
        meta_wayland_session_lock_get_capability(control->wayland_compositor) == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "the compositor session-lock protocol is unavailable");
        return NULL;
    }
    if (meta_wayland_session_lock_get_state(control->wayland_compositor) !=
        META_WAYLAND_SESSION_LOCK_UNLOCKED) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "the session is already locked or changing lock state");
        return NULL;
    }

    g_hash_table_iter_init(&iterator, control->clients);
    while (g_hash_table_iter_next(&iterator, NULL, &value)) {
        Client* client = value;
        if (!client->closing && client->api_minor >= 21 && client->event_api_minor >= 21 &&
            client->event_subscriptions &&
            g_hash_table_contains(client->event_subscriptions, event_name))
            listeners++;
    }
    if (listeners == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                            "no session-lock client is subscribed to lock requests");
        return NULL;
    }

    guint64 sequence = ++control->event_sequence;
    gint64 time = g_get_monotonic_time();
    g_autoptr(JsonNode) event = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(event, object);
    json_object_set_string_member(object, "name", event_name);
    json_object_set_int_member(object, "sequence", sequence);
    json_object_set_int_member(object, "time", time);
    publish_native_socket_event(control, event);

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "dispatched", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&result, "{sv}", "subscribers", g_variant_new_uint32(listeners));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void input_source_keymap_set_done(GObject* source_object, GAsyncResult* result,
                                         gpointer user_data) {
    PendingInputSource* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    gboolean ok = meta_backend_set_keymap_finish(META_BACKEND(source_object), result, &error);
    if (control->stopping) {
        control->pending_input_source_ops--;
        pending_input_source_free(pending);
        native_control_maybe_free_stopped(control);
        return;
    }
    control->pending_input_source_ops--;
    NativeInputSource* selected = input_source_by_id(control, pending->selected_id);
    gboolean confirmed =
        ok && selected &&
        meta_backend_get_keymap_description(control->backend) == pending->description &&
        meta_backend_get_keymap_layout_group(control->backend) == pending->group;
    if (confirmed) {
        g_clear_pointer(&control->input_keymap_description, meta_keymap_description_unref);
        control->input_keymap_description = meta_keymap_description_ref(pending->description);
        g_clear_pointer(&control->active_input_source_ids, g_ptr_array_unref);
        control->active_input_source_ids = g_steal_pointer(&pending->source_ids);
        schedule_windows(control);
    }
    if (!confirmed && ok)
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "Mutter did not confirm the requested keymap group");
    publish_input_source_changes(control, control->state_revision);
    dispatch_input_source_operation(control, pending->request_id, pending->method,
                                    pending->selected_type, confirmed,
                                    confirmed ? pending->selected_id : NULL,
                                    confirmed ? NULL : native_operation_error_code(error),
                                    confirmed ? NULL
                                    : error   ? error->message
                                              : "keymap request failed");
    pending_input_source_free(pending);
}

static gboolean select_xkb_source(GnoblinNativeControl* control, const char* id, gint64 request_id,
                                  const char* method, GError** error) {
    NativeInputSource* selected = input_source_by_id(control, id);
    if (!selected) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "XKB input source is not configured or available: %s", id);
        return FALSE;
    }
    if (control->pending_input_source_ops > 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input source selection is still pending");
        return FALSE;
    }
    guint selected_index = 0;
    while (selected_index < control->input_sources->len &&
           g_ptr_array_index(control->input_sources, selected_index) != selected)
        selected_index++;
    guint chunk_start = (selected_index / 4) * 4;
    guint chunk_length = MIN(4, control->input_sources->len - chunk_start);
    GPtrArray* ids = g_ptr_array_new_with_free_func(g_free);
    g_auto(GStrv) layouts = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) variants = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) display_names = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) short_names = g_new0(char*, chunk_length + 1);
    for (guint i = 0; i < chunk_length; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, chunk_start + i);
        layouts[i] = g_strdup(source->layout);
        variants[i] = g_strdup(source->variant);
        display_names[i] = g_strdup(source->name);
        short_names[i] = g_strdup(source->short_name);
        g_ptr_array_add(ids, g_strdup(source->id));
    }
    g_autofree char* layout_list = g_strjoinv(",", layouts);
    g_autofree char* variant_list = g_strjoinv(",", variants);
    g_autofree char* options = NULL;
    g_autoptr(GVariant) document = native_config_document(control);
    g_autoptr(GVariant) input =
        document ? g_variant_lookup_value(document, "input", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) keyboard =
        input ? g_variant_lookup_value(input, "keyboard", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) xkb_options =
        keyboard ? g_variant_lookup_value(keyboard, "xkb-options", G_VARIANT_TYPE("as")) : NULL;
    if (xkb_options) {
        GPtrArray* option_parts = g_ptr_array_new_with_free_func(g_free);
        for (gsize i = 0; i < g_variant_n_children(xkb_options); i++) {
            g_autoptr(GVariant) option = g_variant_get_child_value(xkb_options, i);
            g_ptr_array_add(option_parts, g_strdup(g_variant_get_string(option, NULL)));
        }
        g_ptr_array_add(option_parts, NULL);
        options = g_strjoinv(",", (char**)option_parts->pdata);
        g_ptr_array_unref(option_parts);
    }
    MetaKeymapDescription* description = meta_keymap_description_new_from_rules(
        NULL, layout_list, variant_list, options, display_names, short_names);
    if (!description) {
        g_ptr_array_unref(ids);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "cannot construct the requested XKB keymap");
        return FALSE;
    }
    g_autoptr(GError) compile_error = NULL;
    struct xkb_keymap* keymap =
        meta_keymap_description_create_xkb_keymap(description, NULL, NULL, &compile_error);
    if (!keymap) {
        meta_keymap_description_unref(description);
        g_ptr_array_unref(ids);
        g_propagate_error(error, g_steal_pointer(&compile_error));
        return FALSE;
    }
    xkb_keymap_unref(keymap);
    PendingInputSource* pending = g_new0(PendingInputSource, 1);
    pending->control = control;
    pending->description = description;
    pending->source_ids = ids;
    pending->selected_type = g_strdup("xkb");
    pending->selected_id = g_strdup(id);
    pending->request_id = request_id;
    pending->method = g_strdup(method);
    pending->group = selected_index - chunk_start;
    control->pending_input_source_ops++;
    meta_backend_set_keymap_async(control->backend, description, pending->group, NULL,
                                  input_source_keymap_set_done, pending);
    return TRUE;
}

static void ibus_input_source_set_done(GObject* source_object, GAsyncResult* result,
                                       gpointer user_data) {
    PendingInputSource* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);
    control->pending_input_source_ops--;
    NativeInputSource* source = ibus_input_source_by_id(control, pending->selected_id);
    gboolean current = !control->stopping &&
                       control->ibus_bus == G_DBUS_CONNECTION(source_object) &&
                       control->ibus_owner_generation == pending->ibus_owner_generation;
    gboolean confirmed = current && reply && source;
    if (confirmed) {
        g_free(control->current_ibus_source_id);
        control->current_ibus_source_id = g_strdup(pending->selected_id);
        publish_input_source_changes(control, control->state_revision);
    } else if (!control->stopping && !current) {
        g_clear_error(&error);
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                                    "IBus owner changed before source selection completed");
    } else if (!control->stopping && reply) {
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "IBus source was removed before selection completed");
    }
    if (!control->stopping)
        dispatch_input_source_operation(control, pending->request_id, pending->method, "ibus",
                                        confirmed, confirmed ? pending->selected_id : NULL,
                                        confirmed ? NULL : native_operation_error_code(error),
                                        confirmed ? NULL
                                        : error   ? error->message
                                                  : "IBus selection failed");
    pending_input_source_free(pending);
    native_control_maybe_free_stopped(control);
}

static gboolean select_ibus_source(GnoblinNativeControl* control, const char* id, gint64 request_id,
                                   const char* method, GError** error) {
    if (!ibus_input_source_by_id(control, id)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "IBus input source is not configured: %s", id);
        return FALSE;
    }
    if (control->pending_input_source_ops > 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input source selection is still pending");
        return FALSE;
    }
    if (!control->ibus_bus) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                            "the IBus service is unavailable");
        return FALSE;
    }
    PendingInputSource* pending = g_new0(PendingInputSource, 1);
    pending->control = control;
    pending->selected_type = g_strdup("ibus");
    pending->selected_id = g_strdup(id);
    pending->request_id = request_id;
    pending->method = g_strdup(method);
    pending->ibus_owner_generation = control->ibus_owner_generation;
    control->pending_input_source_ops++;
    g_dbus_connection_call(control->ibus_bus, IBUS_BUS_NAME, IBUS_OBJECT_PATH, IBUS_INTERFACE,
                           "SetGlobalEngine", g_variant_new("(s)", id), G_VARIANT_TYPE_UNIT,
                           G_DBUS_CALL_FLAGS_NONE, 5000, NULL, ibus_input_source_set_done, pending);
    return TRUE;
}

gboolean gnoblin_native_control_select_input_source(MetaDisplay* display, GVariant* arguments,
                                                    gint64 request_id, const char* method,
                                                    GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_INITIALIZED,
                            "native input control is not available");
        return FALSE;
    }

    const char* type = NULL;
    const char* source_id = NULL;
    if (!arguments || !g_variant_lookup(arguments, "type", "&s", &type) ||
        !g_variant_lookup(arguments, "id", "&s", &source_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input source selection requires type and id");
        return FALSE;
    }
    if (g_str_equal(type, "ibus")) {
        return select_ibus_source(control, source_id, request_id, method, error);
    }
    if (!g_str_equal(type, "xkb")) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input source type must be 'xkb' or 'ibus'");
        return FALSE;
    }

    return select_xkb_source(control, source_id, request_id, method, error);
}

static GHashTable* input_device_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* devices = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    if (!snapshot || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return devices;
    JsonArray* records = json_object_get_array_member(json_node_get_object(snapshot), "devices");
    for (guint i = 0; records && i < json_array_get_length(records); i++) {
        JsonNode* record = json_array_get_element(records, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        const char* id =
            json_object_get_string_member_with_default(json_node_get_object(record), "id", NULL);
        if (id && *id)
            g_hash_table_insert(devices, g_strdup(id), json_to_string(record, FALSE));
    }
    return devices;
}

static void native_window_record_add_public_aliases(JsonObject* object);

static JsonNode* window_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                                      GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "window.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "windows", NULL, control->state_revision);
        return NULL;
    }
    if (update_lua_snapshot)
        cache_lua_window_snapshot(control, result, control->state_revision);
    g_autoptr(JsonNode) json = json_from_variant(result);
    if (JSON_NODE_HOLDS_OBJECT(json)) {
        JsonArray* windows = json_object_get_array_member(json_node_get_object(json), "windows");
        for (guint i = 0; windows && i < json_array_get_length(windows); i++) {
            JsonNode* window = json_array_get_element(windows, i);
            if (!JSON_NODE_HOLDS_OBJECT(window))
                continue;
            native_window_record_add_public_aliases(json_node_get_object(window));
        }
    }
    return g_steal_pointer(&json);
}

static char* window_snapshot(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) json = window_snapshot_json(control, FALSE, &error);
    if (!json)
        return encode_response("", NULL, error ? error->message : "window listing unavailable");
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", "windows");
    json_object_set_int_member(object, "revision", control->state_revision);
    g_autofree char* encoded = json_to_string(json, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static const struct {
    const char* native_name;
    const char* lua_name;
} window_property_names[] = {
    {"id", "id"},
    {"title", "title"},
    {"appId", "app_id"},
    {"gtkAppId", "gtk_app_id"},
    {"wmClass", "wm_class"},
    {"ruleAppId", "rule_app_id"},
    {"workspaceId", "workspace_id"},
    {"workspaceNumber", "workspace_number"},
    {"monitorIndex", "monitor_index"},
    {"monitorId", "monitor_id"},
    {"above", "above"},
    {"sticky", "sticky"},
    {"demandsAttention", "demands_attention"},
    {"closable", "closable"},
    {"minimizable", "minimizable"},
    {"maximizable", "maximizable"},
    {"movable", "movable"},
    {"resizable", "resizable"},
    {"role", "role"},
    {"type", "type"},
    {"maximized", "maximized"},
    {"fullscreen", "fullscreen"},
    {"minimized", "minimized"},
    {"lastUserTime", "last_user_time"},
    {"parent", "parent"},
    {"monitor", "monitor"},
    {"geometry", "frame"},
};

static void native_window_record_add_public_aliases(JsonObject* object) {
    for (guint property = 0; property < G_N_ELEMENTS(window_property_names); property++) {
        const char* native_name = window_property_names[property].native_name;
        const char* public_name = window_property_names[property].lua_name;
        if (g_str_equal(native_name, public_name) || !json_object_has_member(object, native_name) ||
            json_object_has_member(object, public_name))
            continue;
        json_object_set_member(object, public_name,
                               json_node_copy(json_object_get_member(object, native_name)));
    }
}

static gboolean window_record_is_modal(JsonObject* record) {
    JsonNode* type = json_object_get_member(record, "type");
    return type && JSON_NODE_HOLDS_VALUE(type) &&
           json_node_get_int(type) == META_WINDOW_MODAL_DIALOG;
}

static JsonNode* lua_window_record(JsonNode* native_record, guint64 revision) {
    JsonObject* source = json_node_get_object(native_record);
    JsonObject* target = json_object_new();
    for (guint i = 0; i < G_N_ELEMENTS(window_property_names); i++) {
        const char* native_name = window_property_names[i].native_name;
        if (json_object_has_member(source, native_name))
            json_object_set_member(target, window_property_names[i].lua_name,
                                   json_node_copy(json_object_get_member(source, native_name)));
    }
    if (json_object_has_member(source, "focused"))
        json_object_set_member(target, "focused",
                               json_node_copy(json_object_get_member(source, "focused")));
    json_object_set_boolean_member(target, "modal", window_record_is_modal(source));
    json_object_set_int_member(target, "revision", revision);
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, target);
    return node;
}

static void cache_lua_window_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                      guint64 revision) {
    g_autoptr(JsonNode) json = json_from_variant(native_snapshot);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        native_publish_runtime_snapshot(control, "windows", NULL, revision);
        return;
    }
    JsonObject* object = json_node_get_object(json);
    JsonNode* native_windows = json_object_get_member(object, "windows");
    if (!native_windows || !JSON_NODE_HOLDS_ARRAY(native_windows)) {
        native_publish_runtime_snapshot(control, "windows", NULL, revision);
        return;
    }

    JsonArray* windows = json_array_new();
    JsonArray* source = json_node_get_array(native_windows);
    for (guint i = 0; i < json_array_get_length(source); i++) {
        JsonNode* native_record = json_array_get_element(source, i);
        if (!JSON_NODE_HOLDS_OBJECT(native_record))
            continue;
        JsonNode* record = lua_window_record(native_record, revision);
        json_array_add_element(windows, record);
    }
    JsonNode* lua_windows = json_node_new(JSON_NODE_ARRAY);
    json_node_take_array(lua_windows, windows);
    json_object_set_member(object, "windows", lua_windows);
    json_object_set_int_member(object, "revision", revision);
    g_autoptr(GVariant) snapshot = variant_from_json(json);
    if (snapshot)
        native_publish_runtime_snapshot(control, "windows", snapshot, revision);
    else
        native_publish_runtime_snapshot(control, "windows", NULL, revision);
}

static void cache_lua_workspace_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                         guint64 revision) {
    g_autoptr(JsonNode) json = json_from_variant(native_snapshot);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        native_publish_runtime_snapshot(control, "workspaces", NULL, revision);
        return;
    }
    JsonObject* object = json_node_get_object(json);
    JsonNode* native_workspaces = json_object_get_member(object, "workspaces");
    if (!native_workspaces || !JSON_NODE_HOLDS_ARRAY(native_workspaces)) {
        native_publish_runtime_snapshot(control, "workspaces", NULL, revision);
        return;
    }

    JsonArray* workspaces = json_array_new();
    JsonArray* source = json_node_get_array(native_workspaces);
    for (guint i = 0; i < json_array_get_length(source); i++) {
        JsonNode* native_record = json_array_get_element(source, i);
        if (!JSON_NODE_HOLDS_OBJECT(native_record))
            continue;
        JsonObject* source_record = json_node_get_object(native_record);
        JsonObject* record = json_object_new();
        GList* members = json_object_get_members(source_record);
        for (GList* item = members; item; item = item->next) {
            const char* name = item->data;
            const char* target_name = g_str_equal(name, "windows") ? "window_count" : name;
            if (!g_str_equal(name, "revision"))
                json_object_set_member(record, target_name,
                                       json_node_copy(json_object_get_member(source_record, name)));
        }
        g_list_free(members);
        json_object_set_int_member(record, "revision", revision);
        JsonNode* record_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(record_node, record);
        json_array_add_element(workspaces, record_node);
    }
    JsonNode* lua_workspaces = json_node_new(JSON_NODE_ARRAY);
    json_node_take_array(lua_workspaces, workspaces);
    json_object_set_member(object, "workspaces", lua_workspaces);
    json_object_set_int_member(object, "revision", revision);
    g_autoptr(GVariant) snapshot = variant_from_json(json);
    if (snapshot)
        native_publish_runtime_snapshot(control, "workspaces", snapshot, revision);
    else
        native_publish_runtime_snapshot(control, "workspaces", NULL, revision);
}

static JsonNode* lua_workspace_record(JsonNode* native_record, guint64 revision) {
    JsonObject* source = json_node_get_object(native_record);
    JsonObject* record = json_object_new();
    GList* members = json_object_get_members(source);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        const char* target_name = g_str_equal(name, "windows") ? "window_count" : name;
        if (!g_str_equal(name, "revision"))
            json_object_set_member(record, target_name,
                                   json_node_copy(json_object_get_member(source, name)));
    }
    g_list_free(members);
    json_object_set_int_member(record, "revision", revision);
    JsonNode* result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, record);
    return result;
}

static gboolean native_event_is_subscribable(const char* name) {
    if (!name)
        return FALSE;
    for (guint i = 0; native_socket_events[i]; i++) {
        if (g_str_equal(name, native_socket_events[i]) && !g_str_equal(name, "windows") &&
            !g_str_equal(name, "monitors"))
            return TRUE;
    }
    return FALSE;
}

static void remove_private_focus_context(JsonNode* node) {
    if (!node)
        return;
    if (JSON_NODE_HOLDS_OBJECT(node)) {
        JsonObject* object = json_node_get_object(node);
        GList* members = json_object_get_members(object);
        for (GList* item = members; item; item = item->next) {
            const char* name = item->data;
            if (g_str_equal(name, "focus_context"))
                json_object_remove_member(object, name);
            else
                remove_private_focus_context(json_object_get_member(object, name));
        }
        g_list_free(members);
    } else if (JSON_NODE_HOLDS_ARRAY(node)) {
        JsonArray* array = json_node_get_array(node);
        for (guint i = 0; i < json_array_get_length(array); i++)
            remove_private_focus_context(json_array_get_element(array, i));
    }
}

static void native_window_event_add_compat_aliases(JsonObject* object) {
    const char* record_names[] = {"window", "last"};
    for (guint i = 0; i < G_N_ELEMENTS(record_names); i++) {
        JsonNode* record = json_object_get_member(object, record_names[i]);
        if (record && JSON_NODE_HOLDS_OBJECT(record))
            native_window_record_add_public_aliases(json_node_get_object(record));
    }

    JsonNode* changed_node = json_object_get_member(object, "changed");
    if (!changed_node || !JSON_NODE_HOLDS_ARRAY(changed_node))
        return;

    JsonArray* source = json_node_get_array(changed_node);
    JsonArray* changed = json_array_new();
    for (guint i = 0; i < json_array_get_length(source); i++) {
        const char* name = json_array_get_string_element(source, i);
        if (!name)
            continue;
        json_array_add_string_element(changed, name);
        for (guint property = 0; property < G_N_ELEMENTS(window_property_names); property++) {
            if (!g_str_equal(window_property_names[property].lua_name, name) ||
                g_str_equal(window_property_names[property].lua_name,
                            window_property_names[property].native_name))
                continue;
            json_array_add_string_element(changed, window_property_names[property].native_name);
            break;
        }
    }
    json_object_set_array_member(object, "changed", changed);
}

static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {
    JsonObject* object = json_node_get_object(payload);
    const char* borrowed_name = json_object_get_string_member_with_default(object, "name", NULL);
    g_autofree char* name = g_strdup(borrowed_name);
    if (!name)
        return;

    g_autoptr(JsonNode) socket_event = json_node_copy(payload);
    JsonObject* socket_object = json_node_get_object(socket_event);
    json_object_set_string_member(socket_object, "event", name);
    json_object_remove_member(socket_object, "name");
    remove_private_focus_context(socket_event);
    if (g_str_has_prefix(name, "gnoblin.window."))
        native_window_event_add_compat_aliases(socket_object);
    g_autofree char* encoded = json_to_string(socket_event, FALSE);
    g_autofree char* line = g_strconcat(encoded, "\n", NULL);
    gint64 completed_operation_id = 0;
    gboolean operation_completion = g_str_equal(name, "gnoblin.operation.completed") &&
                                    json_object_has_member(object, "operation_id");
    if (operation_completion)
        completed_operation_id = json_object_get_int_member(object, "operation_id");
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        gboolean subscribed = client->event_api_minor >= 9 && client->event_subscriptions &&
                              g_hash_table_contains(client->event_subscriptions, name);
        if (g_str_equal(name, "gnoblin.operation.completed") && completed_operation_id > 0 &&
            client->pending_grant_operations) {
            g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, completed_operation_id);
            if (g_hash_table_remove(client->pending_grant_operations, key))
                subscribed = client->api_minor >= 14;
        }
        if (subscribed && g_str_equal(name, "gnoblin.location.authorization-requested") &&
            json_object_has_member(object, "request_id")) {
            gint64 request_id = json_object_get_int_member(object, "request_id");
            PendingLocationAuthorization* pending =
                request_id > 0
                    ? g_hash_table_lookup(control->pending_location_authorizations, &request_id)
                    : NULL;
            if (pending) {
                guint64* client_key = g_new(guint64, 1);
                *client_key = client->client_id;
                g_hash_table_add(pending->recipient_client_ids, client_key);
            }
        }
        if (g_str_equal(name, "gnoblin.api.operation-completed") ||
            g_str_equal(name, "gnoblin.operation.completed")) {
            if (g_str_equal(name, "gnoblin.operation.completed") && client->api_minor < 11)
                subscribed = FALSE;
            if ((client->track_input_sources && client->input_sources_api_minor >= 6) ||
                (client->track_shortcut_capture && client->shortcut_capture_api_minor >= 8))
                subscribed |=
                    client->api_minor >= 11 || g_str_equal(name, "gnoblin.api.operation-completed");
        } else if (g_str_has_prefix(name, "gnoblin.monitor.")) {
            if (client->track_monitors && client->monitors_api_minor >= 1)
                subscribed = TRUE;
        } else if (g_str_has_prefix(name, "gnoblin.input.source")) {
            if (client->track_input_sources && client->input_sources_api_minor >= 6)
                subscribed = TRUE;
        } else if (g_str_has_prefix(name, "gnoblin.input.device-")) {
            if (client->track_input_devices && client->input_devices_api_minor >= 4)
                subscribed = TRUE;
        } else if (g_str_equal(name, "gnoblin.launch.changed")) {
            if (client->track_launches && client->launch_api_minor >= 7)
                subscribed = TRUE;
        } else if (g_str_equal(name, "gnoblin.input.gesture")) {
            /* Gestures are opt-in through the API 1.9 event subscription. */
        } else if ((g_str_has_prefix(name, "gnoblin.window.") ||
                    g_str_has_prefix(name, "gnoblin.workspace.")) &&
                   client->track_windows && client->windows_api_minor >= 1) {
            subscribed = TRUE;
        }
        if ((g_str_equal(name, "gnoblin.window.menu-requested") ||
             g_str_equal(name, "gnoblin.osd.requested")) &&
            client->event_api_minor < 27)
            subscribed = FALSE;
        if (g_str_equal(name, "gnoblin.window.activation-denied") && client->event_api_minor < 69)
            subscribed = FALSE;
        if (subscribed)
            send_response(client, g_strdup(line));
    }
    g_list_free(clients);
}

static void native_publish_request_event(GnoblinNativeControl* control, const char* name,
                                         GVariant* fields) {
    if (!control || control->stopping || !name || !fields ||
        !g_variant_is_of_type(fields, G_VARIANT_TYPE_VARDICT))
        return;
    GVariantBuilder event;
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event, "{sv}", "name", g_variant_new_string(name));
    g_variant_builder_add(&event, "{sv}", "sequence",
                          g_variant_new_uint64(++control->event_sequence));
    g_variant_builder_add(&event, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_variant_iter_init(&iter, fields);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) owned_value = value;
        g_variant_builder_add(&event, "{sv}", key, owned_value);
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&event));
    g_autoptr(JsonNode) json = json_from_variant(payload);
    if (json)
        publish_native_socket_event(control, json);
    native_runtime_dispatch_event(control, name, payload);
}

static GVariant* appearance_snapshot_new(GnoblinNativeControl* control) {
    if (!control || !control->appearance_settings)
        return NULL;
    g_autofree char* color_scheme =
        g_settings_get_string(control->appearance_settings, "color-scheme");
    if (!g_str_equal(color_scheme, "default") && !g_str_equal(color_scheme, "prefer-dark") &&
        !g_str_equal(color_scheme, "prefer-light")) {
        g_warning("gnoblin-native-control: ignoring unsupported desktop color scheme '%s'",
                  color_scheme);
        return NULL;
    }

    GVariantBuilder snapshot;
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "color_scheme", g_variant_new_string(color_scheme));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void appearance_color_scheme_changed(GSettings* settings, const char* key,
                                            gpointer user_data) {
    (void)settings;
    (void)key;
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping)
        return;

    g_autoptr(GVariant) snapshot = appearance_snapshot_new(control);
    if (!snapshot)
        return;
    const char* color_scheme = NULL;
    if (!g_variant_lookup(snapshot, "color_scheme", "&s", &color_scheme))
        return;
    control->appearance_revision++;
    if (control->appearance_revision == 0)
        control->appearance_revision++;
    native_publish_runtime_snapshot(control, "appearance", snapshot, control->appearance_revision);

    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "color_scheme", g_variant_new_string(color_scheme));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.appearance.color-scheme-changed", payload);
}

static GVariant* privacy_snapshot_new(GnoblinNativeControl* control) {
    GVariantBuilder available;
    GVariantBuilder snapshot;
    g_variant_builder_init(&available, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&available, "{sv}", "screen_sharing", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available, "{sv}", "recording", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&available, "{sv}", "microphone_in_use",
                          g_variant_new_boolean(control->privacy_microphone_available));
    g_variant_builder_add(&available, "{sv}", "camera_in_use",
                          g_variant_new_boolean(control->privacy_camera_available));
    g_variant_builder_add(&available, "{sv}", "location_in_use",
                          g_variant_new_boolean(control->privacy_location_available));
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "available", g_variant_builder_end(&available));
    g_variant_builder_add(&snapshot, "{sv}", "screen_sharing",
                          g_variant_new_boolean(control->privacy_screen_sharing));
    g_variant_builder_add(&snapshot, "{sv}", "recording",
                          g_variant_new_boolean(control->privacy_recording));
    if (control->privacy_microphone_available)
        g_variant_builder_add(&snapshot, "{sv}", "microphone_in_use",
                              g_variant_new_boolean(control->privacy_microphone_in_use));
    if (control->privacy_camera_available)
        g_variant_builder_add(&snapshot, "{sv}", "camera_in_use",
                              g_variant_new_boolean(control->privacy_camera_in_use));
    if (control->privacy_location_available)
        g_variant_builder_add(&snapshot, "{sv}", "location_in_use",
                              g_variant_new_boolean(control->privacy_location_in_use));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void publish_privacy_snapshot(GnoblinNativeControl* control, gboolean changed);

#ifdef HAVE_REMOTE_DESKTOP
static gboolean privacy_camera_disable(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->privacy_camera_disable_source_id = 0;
    if (!control->stopping && control->privacy_camera_available && control->privacy_camera_in_use) {
        control->privacy_camera_in_use = FALSE;
        control->privacy_revision++;
        publish_privacy_snapshot(control, TRUE);
    }
    return G_SOURCE_REMOVE;
}

static void privacy_pipewire_state_changed(GnoblinPipewireMonitor* monitor, gboolean available,
                                           gboolean microphone_in_use, gboolean camera_in_use,
                                           gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)monitor;

    if (!control || control->stopping)
        return;
    gboolean microphone_capability_changed = control->privacy_microphone_available != available;
    gboolean camera_capability_changed = control->privacy_camera_available != available;
    gboolean old_microphone_in_use = control->privacy_microphone_in_use;
    gboolean old_camera_in_use = control->privacy_camera_in_use;
    microphone_in_use = available && microphone_in_use;
    camera_in_use = available && camera_in_use;
    if (control->privacy_camera_disable_source_id) {
        g_source_remove(control->privacy_camera_disable_source_id);
        control->privacy_camera_disable_source_id = 0;
    }
    if (!camera_in_use && available && old_camera_in_use) {
        /* Match GNOME Shell's 500 ms disable delay to avoid indicator flicker. */
        control->privacy_camera_disable_source_id =
            g_timeout_add(500, privacy_camera_disable, control);
        camera_in_use = TRUE;
    }
    if (control->privacy_microphone_available == available &&
        control->privacy_camera_available == available &&
        old_microphone_in_use == microphone_in_use && old_camera_in_use == camera_in_use)
        return;

    control->privacy_microphone_available = available;
    control->privacy_microphone_in_use = microphone_in_use;
    control->privacy_camera_available = available;
    control->privacy_camera_in_use = camera_in_use;
    if (microphone_capability_changed || camera_capability_changed) {
        control->state_revision++;
        g_autoptr(GVariant) capabilities = capability_snapshot(control);
        native_publish_runtime_snapshot(control, "capabilities", capabilities,
                                        control->state_revision);

        const char* changed_capabilities[] = {
            microphone_capability_changed ? "microphone-monitor" : NULL,
            camera_capability_changed ? "camera-monitor" : NULL,
        };
        for (guint i = 0; i < G_N_ELEMENTS(changed_capabilities); i++) {
            const NativeCapability* changed =
                changed_capabilities[i] ? native_capability_by_id(changed_capabilities[i]) : NULL;
            if (!changed)
                continue;
            GVariantBuilder event;
            g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
            g_autoptr(GVariant) capability = capability_snapshot_record(control, changed);
            g_variant_builder_add(&event, "{sv}", "capability", capability);
            g_variant_builder_add(&event, "{sv}", "revision",
                                  g_variant_new_int64((gint64)control->state_revision));
            g_autoptr(GVariant) fields = g_variant_ref_sink(g_variant_builder_end(&event));
            native_publish_request_event(control, "gnoblin.capability.changed", fields);
        }
    }
    if (old_microphone_in_use != microphone_in_use || old_camera_in_use != camera_in_use ||
        microphone_capability_changed || camera_capability_changed) {
        control->privacy_revision++;
        publish_privacy_snapshot(control, TRUE);
    }
}
#endif

static void privacy_refresh_state(GnoblinNativeControl* control) {
    gboolean screen_sharing = FALSE;
    gboolean recording = FALSE;
    GHashTableIter iter;
    gpointer key;
    if (!control || control->stopping)
        return;

    g_hash_table_iter_init(&iter, control->privacy_handles);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        gboolean is_recording = FALSE;
        g_object_get(key, "is-recording", &is_recording, NULL);
        if (is_recording)
            recording = TRUE;
        else
            screen_sharing = TRUE;
    }

    if (screen_sharing == control->privacy_screen_sharing &&
        recording == control->privacy_recording)
        return;
    control->privacy_screen_sharing = screen_sharing;
    control->privacy_recording = recording;
    control->privacy_revision++;
    publish_privacy_snapshot(control, TRUE);
}

static void publish_privacy_snapshot(GnoblinNativeControl* control, gboolean changed) {
    if (!control || control->stopping)
        return;

    GVariant* snapshot = privacy_snapshot_new(control);
    g_clear_pointer(&control->privacy_snapshot, g_variant_unref);
    control->privacy_snapshot = snapshot;
    native_publish_runtime_snapshot(control, "privacy", control->privacy_snapshot,
                                    control->privacy_revision);

    if (!changed)
        return;

    GVariantBuilder state_builder;
    GVariantIter state_fields;
    const char* state_field;
    GVariant* state_value;
    g_variant_builder_init(&state_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&state_fields, snapshot);
    while (g_variant_iter_next(&state_fields, "{&sv}", &state_field, &state_value)) {
        g_autoptr(GVariant) value = state_value;
        g_variant_builder_add(&state_builder, "{sv}", state_field, g_variant_ref(value));
    }
    g_variant_builder_add(&state_builder, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->privacy_revision));
    g_autoptr(GVariant) state = g_variant_ref_sink(g_variant_builder_end(&state_builder));
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "state", state);
    g_variant_builder_add(&payload, "{sv}", "name",
                          g_variant_new_string("gnoblin.privacy.changed"));
    g_variant_builder_add(&payload, "{sv}", "revision",
                          g_variant_new_int64((gint64)control->privacy_revision));
    g_variant_builder_add(&payload, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++control->event_sequence));
    g_variant_builder_add(&payload, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) enriched = g_variant_ref_sink(g_variant_builder_end(&payload));
    native_runtime_dispatch_event(control, "gnoblin.privacy.changed", enriched);
    g_autoptr(JsonNode) json = json_from_variant(enriched);
    if (JSON_NODE_HOLDS_OBJECT(json))
        publish_native_socket_event(control, json);
}

static void pending_location_authorization_free(gpointer user_data) {
    PendingLocationAuthorization* pending = user_data;
    if (pending->timeout_source_id)
        g_source_remove(pending->timeout_source_id);
    if (!pending->completed)
        gnoblin_location_request_complete(pending->request, FALSE, 0);
    gnoblin_location_request_unref(pending->request);
    g_clear_pointer(&pending->recipient_client_ids, g_hash_table_unref);
    g_free(pending);
}

static void clear_pending_location_authorizations(GnoblinNativeControl* control) {
    if (control && control->pending_location_authorizations)
        g_hash_table_remove_all(control->pending_location_authorizations);
}

static gboolean pending_location_authorization_timeout(gpointer user_data) {
    PendingLocationAuthorization* pending = user_data;
    pending->timeout_source_id = 0;
    if (pending->control->pending_location_authorizations)
        g_hash_table_remove(pending->control->pending_location_authorizations,
                            &pending->request_id);
    return G_SOURCE_REMOVE;
}

static gboolean location_accuracy_valid(guint accuracy) {
    return accuracy == 0 || accuracy == 1 || accuracy == 4 || accuracy == 5 || accuracy == 6 ||
           accuracy == 8;
}

static void location_authorize_app(GnoblinLocationAgent* agent, const char* app_id,
                                   guint requested_accuracy, GnoblinLocationRequest* request,
                                   gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)agent;
    if (!control || control->stopping || !control->supervised_runtime ||
        control->runtime_worker_suspended || !control->runtime_hello_sent ||
        !control->pending_location_authorizations ||
        g_hash_table_size(control->pending_location_authorizations) >=
            MAX_PENDING_LOCATION_AUTHORIZATIONS ||
        control->next_location_request_id >= G_MAXINT64) {
        gnoblin_location_request_complete(request, FALSE, 0);
        return;
    }

    PendingLocationAuthorization* pending = g_new0(PendingLocationAuthorization, 1);
    pending->control = control;
    pending->request = gnoblin_location_request_ref(request);
    pending->request_id = ++control->next_location_request_id;
    pending->requested_accuracy = requested_accuracy;
    pending->expires_at_us =
        g_get_monotonic_time() + LOCATION_AUTHORIZATION_TIMEOUT_SECONDS * G_USEC_PER_SEC;
    pending->recipient_client_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    guint64* key = g_new(guint64, 1);
    *key = pending->request_id;
    g_hash_table_insert(control->pending_location_authorizations, key, pending);
    pending->timeout_source_id = g_timeout_add_seconds(
        LOCATION_AUTHORIZATION_TIMEOUT_SECONDS, pending_location_authorization_timeout, pending);

    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "request_id",
                          g_variant_new_int64((gint64)pending->request_id));
    g_variant_builder_add(&fields, "{sv}", "app_id", g_variant_new_string(app_id ? app_id : ""));
    g_variant_builder_add(&fields, "{sv}", "requested_accuracy",
                          g_variant_new_uint32(requested_accuracy));
    g_variant_builder_add(&fields, "{sv}", "expires_at_us",
                          g_variant_new_int64(pending->expires_at_us));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.location.authorization-requested", payload);
}

static void privacy_location_state_changed(GnoblinLocationAgent* agent, gboolean available,
                                           gboolean in_use, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    (void)agent;
    if (!control || control->stopping)
        return;
    in_use = available && in_use;
    gboolean availability_changed = control->privacy_location_available != available;
    if (!availability_changed && control->privacy_location_in_use == in_use)
        return;
    control->privacy_location_available = available;
    control->privacy_location_in_use = in_use;
    if (availability_changed) {
        control->state_revision++;
        g_autoptr(GVariant) capabilities = capability_snapshot(control);
        native_publish_runtime_snapshot(control, "capabilities", capabilities,
                                        control->state_revision);
        const NativeCapability* changed = native_capability_by_id("location-agent");
        if (changed) {
            GVariantBuilder event;
            g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
            g_autoptr(GVariant) capability = capability_snapshot_record(control, changed);
            g_variant_builder_add(&event, "{sv}", "capability", capability);
            g_variant_builder_add(&event, "{sv}", "revision",
                                  g_variant_new_int64((gint64)control->state_revision));
            g_autoptr(GVariant) fields = g_variant_ref_sink(g_variant_builder_end(&event));
            native_publish_request_event(control, "gnoblin.capability.changed", fields);
        }
    }
    control->privacy_revision++;
    publish_privacy_snapshot(control, TRUE);
}

static GVariant* native_location_authorize_operation(GnoblinNativeControl* control,
                                                     GVariant* arguments, guint64 client_id,
                                                     GError** error) {
    gint64 signed_request_id = 0;
    gint64 signed_accuracy = 0;
    gboolean allowed = FALSE;
    if (g_variant_n_children(arguments) != 3 ||
        !g_variant_lookup(arguments, "request_id", "x", &signed_request_id) ||
        signed_request_id <= 0 || !g_variant_lookup(arguments, "allow", "b", &allowed) ||
        !g_variant_lookup(arguments, "accuracy", "x", &signed_accuracy) || signed_accuracy < 0 ||
        signed_accuracy > 8 || !location_accuracy_valid((guint)signed_accuracy)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "location.authorize_app requires request_id, allow, and a valid accuracy level");
        return NULL;
    }

    PendingLocationAuthorization* pending =
        g_hash_table_lookup(control->pending_location_authorizations, &signed_request_id);
    if (!pending || pending->completed || pending->expires_at_us <= g_get_monotonic_time()) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "location authorization request is no longer pending");
        return NULL;
    }
    if (client_id && !g_hash_table_contains(pending->recipient_client_ids, &client_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "only a client that received the location request can answer it");
        return NULL;
    }

    guint accuracy = allowed ? (guint)signed_accuracy : 0;
    if ((allowed && (accuracy == 0 || accuracy > pending->requested_accuracy)) ||
        (!allowed && accuracy != 0)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "allowed location accuracy must be nonzero and no greater than requested");
        return NULL;
    }
    pending->completed = TRUE;
    gnoblin_location_request_complete(pending->request, allowed, accuracy);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "request_id", g_variant_new_int64(signed_request_id));
    g_variant_builder_add(&result, "{sv}", "submitted", g_variant_new_boolean(TRUE));
    GVariant* response = g_variant_ref_sink(g_variant_builder_end(&result));
    g_hash_table_remove(control->pending_location_authorizations, &signed_request_id);
    return response;
}

static void privacy_handle_stopped(MetaRemoteAccessHandle* handle, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !g_hash_table_remove(control->privacy_handles, handle))
        return;
    privacy_refresh_state(control);
}

static void privacy_new_handle(MetaRemoteAccessController* controller,
                               MetaRemoteAccessHandle* handle, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !handle ||
        g_hash_table_contains(control->privacy_handles, handle))
        return;

    g_hash_table_add(control->privacy_handles, g_object_ref(handle));
    g_signal_connect(handle, "stopped", G_CALLBACK(privacy_handle_stopped), control);
    privacy_refresh_state(control);
}

static GVariant* native_stop_privacy_sessions(GnoblinNativeControl* control, gboolean recording,
                                              GError** error) {
    g_autoptr(GPtrArray) handles = g_ptr_array_new_with_free_func(g_object_unref);
    GHashTableIter iter;
    gpointer key;
    guint32 requested = 0;
    if (!control || control->stopping || !control->privacy_handles) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "privacy session control is unavailable");
        return NULL;
    }

    g_hash_table_iter_init(&iter, control->privacy_handles);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        gboolean is_recording = FALSE;
        g_object_get(key, "is-recording", &is_recording, NULL);
        if (is_recording == recording)
            g_ptr_array_add(handles, g_object_ref(key));
    }

    for (guint i = 0; i < handles->len; i++) {
        MetaRemoteAccessHandle* handle = g_ptr_array_index(handles, i);
        meta_remote_access_handle_stop(handle);
        requested++;
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "requested", g_variant_new_uint32(requested));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* filter_native_touchpad_gestures(GVariant* gestures);

static gboolean native_rule_get_number(GVariant* record, const char* key, double* number) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, NULL);
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *number = (double)g_variant_get_int32(value);
    else
        return FALSE;
    return isfinite(*number);
}

static gboolean native_rule_get_padding(GVariant* record, double padding[4]) {
    g_autoptr(GVariant) values = g_variant_lookup_value(record, "padding", G_VARIANT_TYPE("av"));
    if (!values || g_variant_n_children(values) != 4)
        return FALSE;

    double parsed[4];
    for (gsize i = 0; i < G_N_ELEMENTS(parsed); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(values, i);
        g_autoptr(GVariant) value = g_variant_get_variant(boxed);
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            parsed[i] = g_variant_get_double(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            parsed[i] = (double)g_variant_get_int64(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
            parsed[i] = (double)g_variant_get_int32(value);
        else
            return FALSE;
        if (!isfinite(parsed[i]) || parsed[i] < -128 || parsed[i] > 128)
            return FALSE;
    }

    memcpy(padding, parsed, sizeof(parsed));
    return TRUE;
}

static gboolean native_rule_get_color(GVariant* record, const char* key, double color[4]) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, G_VARIANT_TYPE_STRING);
    if (!value)
        return FALSE;

    const char* text = g_variant_get_string(value, NULL);
    gsize length = strlen(text);
    if (length != 7 && length != 9)
        return FALSE;
    if (text[0] != '#')
        return FALSE;

    guint8 channels[4] = {0, 0, 0, 255};
    for (guint i = 0; i < (length == 9 ? 4u : 3u); i++) {
        int high = g_ascii_xdigit_value(text[1 + i * 2]);
        int low = g_ascii_xdigit_value(text[2 + i * 2]);
        if (high < 0 || low < 0)
            return FALSE;
        channels[i] = (guint8)(high * 16 + low);
    }
    for (guint i = 0; i < 4; i++)
        color[i] = channels[i] / 255.;
    return TRUE;
}

static gboolean native_rule_get_boolean(GVariant* record, const char* key, const char* legacy_key,
                                        gboolean* value) {
    return g_variant_lookup(record, key, "b", value) ||
           (legacy_key && g_variant_lookup(record, legacy_key, "b", value));
}

enum {
    NATIVE_CORNER_TOOLKIT_NONE = 0,
    NATIVE_CORNER_TOOLKIT_ADWAITA = 1 << 0,
    NATIVE_CORNER_TOOLKIT_HANDY = 1 << 1,
    NATIVE_CORNER_TOOLKIT_PENDING = 1 << 2,
};

typedef struct {
    pid_t pid;
    guint64 start_time;
} NativeCornerProcessIdentity;

typedef struct {
    NativeCornerToolkitCache* cache;
    NativeCornerProcessIdentity identity;
} NativeCornerToolkitProbe;

static guint native_corner_process_identity_hash(gconstpointer data) {
    const NativeCornerProcessIdentity* identity = data;
    guint64 start_time = identity->start_time;
    guint hash = (guint)identity->pid;
    hash = hash * 33u + (guint)start_time;
    hash = hash * 33u + (guint)(start_time >> 32);
    return hash;
}

static gboolean native_corner_process_identity_equal(gconstpointer a, gconstpointer b) {
    const NativeCornerProcessIdentity* left = a;
    const NativeCornerProcessIdentity* right = b;
    return left->pid == right->pid && left->start_time == right->start_time;
}

static gboolean native_corner_process_identity_get(pid_t pid,
                                                   NativeCornerProcessIdentity* identity) {
    g_autofree char* path = g_strdup_printf("/proc/%d/stat", pid);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return FALSE;

    char stat[4096];
    ssize_t count;
    do {
        count = read(fd, stat, sizeof(stat) - 1);
    } while (count < 0 && errno == EINTR);
    close(fd);
    if (count <= 0)
        return FALSE;
    stat[count] = '\0';

    /* comm is parenthesized but may itself contain spaces or ')'. The final
     * ')' separates it from state (field 3); starttime is field 22. */
    char* cursor = strrchr(stat, ')');
    if (!cursor)
        return FALSE;
    cursor++;
    for (guint field = 3; field <= 22; field++) {
        while (g_ascii_isspace(*cursor))
            cursor++;
        if (!*cursor)
            return FALSE;
        char* end = cursor;
        while (*end && !g_ascii_isspace(*end))
            end++;
        if (field == 22) {
            g_autofree char* value = g_strndup(cursor, end - cursor);
            char* parse_end = NULL;
            const guint64 start_time = g_ascii_strtoull(value, &parse_end, 10);
            if (parse_end == value || !parse_end || *parse_end)
                return FALSE;
            identity->pid = pid;
            identity->start_time = start_time;
            return TRUE;
        }
        cursor = end;
    }
    return FALSE;
}

static gpointer native_corner_toolkit_cache_lookup(NativeCornerToolkitCache* cache,
                                                   const NativeCornerProcessIdentity* identity) {
    return cache ? g_hash_table_lookup(cache->entries, identity) : NULL;
}

static void native_corner_toolkit_probe_next(GnoblinNativeControl* control,
                                             const NativeCornerProcessIdentity* after);

static NativeCornerToolkitCache* native_corner_toolkit_cache_ref(NativeCornerToolkitCache* cache) {
    g_atomic_int_inc(&cache->ref_count);
    return cache;
}

static void native_corner_toolkit_cache_unref(NativeCornerToolkitCache* cache) {
    if (!cache || !g_atomic_int_dec_and_test(&cache->ref_count))
        return;
    g_hash_table_unref(cache->entries);
    g_free(cache);
}

static void native_corner_toolkit_cache_shutdown(GnoblinNativeControl* control) {
    if (!control || !control->corner_toolkit_cache)
        return;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    control->corner_toolkit_cache = NULL;
    /* Worker callbacks are dispatched on the compositor's main context. The
     * cache outlives them, but clearing this pointer makes them harmless after
     * control teardown without keeping the whole native control alive. */
    cache->control = NULL;
    native_corner_toolkit_cache_unref(cache);
}

static void native_corner_toolkit_probe_free(gpointer data) {
    NativeCornerToolkitProbe* probe = data;
    native_corner_toolkit_cache_unref(probe->cache);
    g_free(probe);
}

static void native_corner_toolkit_probe_worker(GTask* task, gpointer source_object,
                                               gpointer task_data, GCancellable* cancellable) {
    (void)source_object;
    (void)cancellable;
    NativeCornerToolkitProbe* probe = task_data;
    g_autofree char* path = g_strdup_printf("/proc/%d/maps", probe->identity.pid);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        /* Restricted /proc access is expected in some sandboxes. Match the
         * previous behavior: failed detection leaves clipping enabled. */
        g_task_return_int(task, NATIVE_CORNER_TOOLKIT_NONE);
        return;
    }

    g_autoptr(GByteArray) contents = g_byte_array_sized_new(MAX_CORNER_TOOLKIT_MAPS_BYTES + 1);
    guint8 chunk[8192];
    while (contents->len < MAX_CORNER_TOOLKIT_MAPS_BYTES) {
        const gsize remaining = MAX_CORNER_TOOLKIT_MAPS_BYTES - contents->len;
        const ssize_t count = read(fd, chunk, MIN(sizeof(chunk), remaining));
        if (count > 0) {
            g_byte_array_append(contents, chunk, count);
            continue;
        }
        if (count < 0 && errno == EINTR)
            continue;
        break;
    }
    close(fd);
    g_byte_array_append(contents, (const guint8*)"", 1);

    guint result = NATIVE_CORNER_TOOLKIT_NONE;
    if (strstr((const char*)contents->data, "/libadwaita-1.so"))
        result |= NATIVE_CORNER_TOOLKIT_ADWAITA;
    if (strstr((const char*)contents->data, "/libhandy-1.so"))
        result |= NATIVE_CORNER_TOOLKIT_HANDY;
    g_task_return_int(task, result);
}

static void native_corner_toolkit_probe_finished(GObject* source_object, GAsyncResult* result,
                                                 gpointer user_data) {
    (void)source_object;
    (void)user_data;
    GTask* task = G_TASK(result);
    NativeCornerToolkitProbe* probe = g_task_get_task_data(task);
    NativeCornerToolkitCache* cache = probe->cache;
    GnoblinNativeControl* control = cache->control;
    g_autoptr(GError) error = NULL;
    const gssize probe_result = g_task_propagate_int(task, &error);
    const guint toolkit_flags =
        probe_result >= 0 ? (guint)probe_result : NATIVE_CORNER_TOOLKIT_NONE;

    if (!control || control->stopping || !control->display)
        return;

    gpointer previous = native_corner_toolkit_cache_lookup(cache, &probe->identity);
    if (previous && GPOINTER_TO_UINT(previous) == NATIVE_CORNER_TOOLKIT_PENDING + 1 &&
        cache->pending_count > 0)
        cache->pending_count--;
    NativeCornerProcessIdentity* key = g_new(NativeCornerProcessIdentity, 1);
    *key = probe->identity;
    g_hash_table_replace(cache->entries, key, GUINT_TO_POINTER(toolkit_flags + 1));
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        NativeCornerProcessIdentity current_identity;
        if (meta_window_get_pid(window) == probe->identity.pid &&
            native_corner_process_identity_get(probe->identity.pid, &current_identity) &&
            native_corner_process_identity_equal(&current_identity, &probe->identity))
            native_apply_window_rules(control, window);
    }
    g_slist_free(windows);

    /* A full cache can defer new PIDs while every slot is pending. Walk the
     * current windows again after each completion, starting after this PID,
     * and use the newly completed slot as room for the next pending probe. */
    native_corner_toolkit_probe_next(control, &probe->identity);
}

static gboolean native_corner_toolkit_cache_make_room(NativeCornerToolkitCache* cache) {
    if (g_hash_table_size(cache->entries) < MAX_CORNER_TOOLKIT_CACHE_ENTRIES)
        return TRUE;
    if (cache->pending_count >= MAX_CORNER_TOOLKIT_CACHE_ENTRIES)
        return FALSE;

    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, cache->entries);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        if (GPOINTER_TO_UINT(value) != NATIVE_CORNER_TOOLKIT_PENDING + 1) {
            g_hash_table_iter_remove(&iter);
            return TRUE;
        }
    }
    return FALSE;
}

static gboolean native_corner_toolkit_probe(GnoblinNativeControl* control,
                                            const NativeCornerProcessIdentity* identity) {
    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    if (!cache) {
        cache = g_new0(NativeCornerToolkitCache, 1);
        cache->ref_count = 1;
        cache->control = control;
        cache->entries = g_hash_table_new_full(native_corner_process_identity_hash,
                                               native_corner_process_identity_equal, g_free, NULL);
        control->corner_toolkit_cache = cache;
    }

    if (native_corner_toolkit_cache_lookup(cache, identity) ||
        !native_corner_toolkit_cache_make_room(cache))
        return FALSE;

    NativeCornerProcessIdentity* key = g_new(NativeCornerProcessIdentity, 1);
    *key = *identity;
    g_hash_table_insert(cache->entries, key, GUINT_TO_POINTER(NATIVE_CORNER_TOOLKIT_PENDING + 1));
    cache->pending_count++;
    NativeCornerToolkitProbe* probe = g_new0(NativeCornerToolkitProbe, 1);
    probe->cache = native_corner_toolkit_cache_ref(cache);
    probe->identity = *identity;
    GTask* task = g_task_new(NULL, NULL, native_corner_toolkit_probe_finished, NULL);
    g_task_set_task_data(task, probe, native_corner_toolkit_probe_free);
    g_task_run_in_thread(task, native_corner_toolkit_probe_worker);
    g_object_unref(task);
    return TRUE;
}

static void native_corner_toolkit_probe_next(GnoblinNativeControl* control,
                                             const NativeCornerProcessIdentity* after) {
    if (!control || control->stopping || !control->display)
        return;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    if (!cache)
        return;

    g_autoptr(GPtrArray) identities = g_ptr_array_new_with_free_func(g_free);
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        const pid_t pid = meta_window_get_pid(window);
        if (pid <= 0)
            continue;
        NativeCornerProcessIdentity current;
        if (!native_corner_process_identity_get(pid, &current))
            continue;

        gboolean found = FALSE;
        for (guint i = 0; i < identities->len; i++) {
            if (native_corner_process_identity_equal(g_ptr_array_index(identities, i), &current)) {
                found = TRUE;
                break;
            }
        }
        if (!found) {
            NativeCornerProcessIdentity* copy = g_new(NativeCornerProcessIdentity, 1);
            *copy = current;
            g_ptr_array_add(identities, copy);
        }
    }
    if (identities->len == 0) {
        g_slist_free(windows);
        return;
    }

    guint start = 0;
    for (guint i = 0; i < identities->len; i++) {
        if (native_corner_process_identity_equal(g_ptr_array_index(identities, i), after)) {
            start = (i + 1) % identities->len;
            break;
        }
    }

    for (guint offset = 0; offset < identities->len; offset++) {
        const guint index = (start + offset) % identities->len;
        const NativeCornerProcessIdentity* identity = g_ptr_array_index(identities, index);
        if (native_corner_toolkit_cache_lookup(cache, identity))
            continue;

        NativeCornerProcessIdentity current;
        if (!native_corner_process_identity_get(identity->pid, &current) ||
            !native_corner_process_identity_equal(&current, identity))
            continue;
        const guint pending_before = cache->pending_count;
        for (GSList* link = windows; link; link = link->next) {
            MetaWindow* window = link->data;
            if (meta_window_get_pid(window) == identity->pid)
                native_apply_window_rules(control, window);
            if (cache->pending_count > pending_before)
                break;
        }
        if (cache->pending_count > pending_before)
            break;
    }
    g_slist_free(windows);
}

static gboolean native_corner_toolkit_should_skip(GnoblinNativeControl* control, MetaWindow* window,
                                                  gboolean skip_libadwaita,
                                                  gboolean skip_libhandy) {
    if ((!skip_libadwaita && !skip_libhandy) || !control || control->stopping)
        return FALSE;

    const pid_t pid = meta_window_get_pid(window);
    if (pid <= 0)
        return FALSE;

    NativeCornerProcessIdentity identity;
    if (!native_corner_process_identity_get(pid, &identity))
        return FALSE;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    gpointer encoded = native_corner_toolkit_cache_lookup(cache, &identity);
    if (!encoded) {
        native_corner_toolkit_probe(control, &identity);
        /* Match the old watcher: apply auto mode while detection is pending,
         * then update the actor when the bounded maps scan completes. */
        return FALSE;
    }

    const guint toolkit_flags = GPOINTER_TO_UINT(encoded) - 1;
    if (toolkit_flags == NATIVE_CORNER_TOOLKIT_PENDING)
        return FALSE;

    return (skip_libadwaita && (toolkit_flags & NATIVE_CORNER_TOOLKIT_ADWAITA)) ||
           (skip_libhandy && (toolkit_flags & NATIVE_CORNER_TOOLKIT_HANDY));
}

static gboolean native_window_rule_matches(GVariant* match, MetaWindow* window,
                                           GnoblinNativeControl* control) {
    const char* title = meta_window_get_title(window);
    const char* gtk_app_id = meta_window_get_gtk_application_id(window);
    const char* wm_class = meta_window_get_wm_class(window);
    const char* layer_namespace = g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace");
    const char* rule_app_id = gtk_app_id && *gtk_app_id ? gtk_app_id : (wm_class ? wm_class : "");
    MetaWorkspace* workspace = meta_window_get_workspace(window);
    const char* workspace_id =
        workspace ? g_object_get_data(G_OBJECT(workspace), "gnoblin-native-id") : NULL;
    const int workspace_number = workspace ? meta_workspace_index(workspace) + 1 : 0;
    const gboolean focused = meta_display_get_focus_window(control->display) == window;
    GVariantIter iter;
    const char* key;
    GVariant* value;

    if (!g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    g_variant_iter_init(&iter, match);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean matched = FALSE;

        if (g_str_equal(key, "type")) {
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* type = g_variant_get_string(value, NULL);
                matched = (g_str_equal(type, "window") && !layer_namespace) ||
                          (g_str_equal(type, "layer") && layer_namespace);
            }
        } else if (g_str_equal(key, "layer")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            g_autoptr(GError) error = NULL;
            gboolean pattern_matched = FALSE;
            matched =
                pattern && layer_namespace &&
                gnoblin_lua_pattern_match(pattern, layer_namespace, &pattern_matched, &error) &&
                pattern_matched;
        } else if (g_str_equal(key, "focused")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                      g_variant_get_boolean(value) == focused;
        } else if (g_str_equal(key, "workspace_id") || g_str_equal(key, "workspace-id")) {
            const char* expected = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                       ? g_variant_get_string(value, NULL)
                                       : NULL;
            matched = expected && workspace_id && g_str_equal(expected, workspace_id);
        } else if (g_str_equal(key, "workspace_number") || g_str_equal(key, "workspace-number")) {
            gint64 expected = 0;
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
                expected = g_variant_get_int64(value);
                matched = TRUE;
            } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)) {
                expected = g_variant_get_int32(value);
                matched = TRUE;
            }
            matched = matched && expected == workspace_number;
        } else if (g_str_equal(key, "app-id") || g_str_equal(key, "app_id") ||
                   g_str_equal(key, "title")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            const char* subject = g_str_equal(key, "title") ? (title ? title : "") : rule_app_id;
            g_autoptr(GError) error = NULL;
            gboolean pattern_matched = FALSE;
            matched = pattern &&
                      gnoblin_lua_pattern_match(pattern, subject, &pattern_matched, &error) &&
                      pattern_matched;
        }

        g_variant_unref(value);
        if (!matched)
            return FALSE;
    }
    return TRUE;
}

static void native_shadow_layer_defaults(MetaGnoblinWindowShadowLayer* layer) {
    *layer = (MetaGnoblinWindowShadowLayer){
        .x = 0.,
        .y = 4.,
        .blur = 28.,
        .spread = 4.,
        .opacity = .6,
        .color = {0., 0., 0., 1.},
    };
}

static gboolean native_shadow_layer_parse(GVariant* value, MetaGnoblinWindowShadowLayer* layer,
                                          gboolean initialize) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unboxed = g_variant_get_variant(value);
        return native_shadow_layer_parse(unboxed, layer, initialize);
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    if (initialize)
        native_shadow_layer_defaults(layer);
    double number;
    if (native_rule_get_number(value, "x", &number))
        layer->x = number;
    if (native_rule_get_number(value, "y", &number))
        layer->y = number;
    if (native_rule_get_number(value, "blur", &number))
        layer->blur = number;
    if (native_rule_get_number(value, "spread", &number))
        layer->spread = number;
    if (native_rule_get_number(value, "opacity", &number))
        layer->opacity = number;
    double color[4];
    if (native_rule_get_color(value, "color", color))
        memcpy(layer->color, color, sizeof(color));
    return TRUE;
}

static gboolean native_shadow_layers_parse(GVariant* value, MetaGnoblinWindowShadowLayer layers[4],
                                           guint* n_layers, gboolean merge_single_layer) {
    *n_layers = 0;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unboxed = g_variant_get_variant(value);
        return native_shadow_layers_parse(unboxed, layers, n_layers, merge_single_layer);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
        if (g_variant_get_boolean(value)) {
            native_shadow_layer_defaults(&layers[0]);
            *n_layers = 1;
        }
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        if (!native_shadow_layer_parse(value, &layers[0], !merge_single_layer))
            return FALSE;
        *n_layers = 1;
        return TRUE;
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")) || g_variant_n_children(value) == 0 ||
        g_variant_n_children(value) > 4)
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        if (!native_shadow_layer_parse(boxed, &layers[i], TRUE))
            return FALSE;
        (*n_layers)++;
    }
    return TRUE;
}

static void native_shadow_transition_set_easing(GVariant* record, const char* key,
                                                MetaGnoblinWindowShadowTransition* transition) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, NULL);
    if (!value)
        return;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        transition->easing = g_variant_get_string(value, NULL);
        transition->has_bezier = FALSE;
        return;
    }
    const char* type = NULL;
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(value, "type", "&s", &type) || !g_str_equal(type, "cubic-bezier"))
        return;
    const char* names[] = {"x1", "y1", "x2", "y2"};
    double points[4];
    for (guint i = 0; i < G_N_ELEMENTS(points); i++)
        if (!native_rule_get_number(value, names[i], &points[i]) || points[i] < -2. ||
            points[i] > 2.)
            return;
    if (points[0] > 1. || points[0] < 0. || points[2] > 1. || points[2] < 0.)
        return;
    memcpy(transition->bezier, points, sizeof(points));
    transition->has_bezier = TRUE;
}

static GVariant* native_shadow_animation_merge(GVariant* previous, GVariant* corners) {
    g_autoptr(GVariant) next = g_variant_lookup_value(corners, "shadow_animation", NULL);
    if (!next)
        next = g_variant_lookup_value(corners, "shadow-animation", NULL);
    if (!next)
        return previous ? g_variant_ref(previous) : NULL;
    g_autoptr(GVariant) unboxed = NULL;
    GVariant* next_value = next;
    if (g_variant_is_of_type(next_value, G_VARIANT_TYPE_VARIANT)) {
        unboxed = g_variant_get_variant(next_value);
        next_value = unboxed;
    }
    if (!g_variant_is_of_type(next_value, G_VARIANT_TYPE_VARDICT))
        return previous ? g_variant_ref(previous) : NULL;

    GVariantDict merged;
    g_variant_dict_init(&merged, previous && g_variant_is_of_type(previous, G_VARIANT_TYPE_VARDICT)
                                     ? previous
                                     : NULL);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, next_value);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_variant_dict_insert_value(&merged, key, value);
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_dict_end(&merged));
}

static void native_shadow_animation_resolve(GVariant* document, GVariant* animation,
                                            MetaGnoblinWindowShadowTransition* transition) {
    transition->duration_ms = 0;
    transition->easing = "ease-out-cubic";
    transition->has_bezier = FALSE;
    const char* selected_name = NULL;
    if (animation && g_variant_is_of_type(animation, G_VARIANT_TYPE_VARDICT)) {
        g_variant_lookup(animation, "animation", "&s", &selected_name);
    }

    g_autoptr(GVariant) registrations =
        document ? g_variant_lookup_value(document, "animations", NULL) : NULL;
    gboolean selected_found = !selected_name || g_str_equal(selected_name, "none") ||
                              g_str_equal(selected_name, "gnoblin-shadow-change");
    for (gsize i = 0; registrations && i < g_variant_n_children(registrations); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(registrations, i);
        g_autoptr(GVariant) registration = g_variant_is_of_type(boxed, G_VARIANT_TYPE_VARIANT)
                                               ? g_variant_get_variant(boxed)
                                               : g_variant_ref(boxed);
        const char* name = NULL;
        const char* event = NULL;
        gboolean enabled = TRUE;
        if (!g_variant_is_of_type(registration, G_VARIANT_TYPE_VARDICT) ||
            !g_variant_lookup(registration, "event", "&s", &event) ||
            !g_str_equal(event, "shadow-change"))
            continue;
        g_variant_lookup(registration, "name", "&s", &name);
        g_variant_lookup(registration, "enable", "b", &enabled);
        if (!enabled || (selected_name && g_strcmp0(selected_name, name) != 0))
            continue;
        selected_found = TRUE;
        double duration = 0;
        native_rule_get_number(registration, "duration", &duration);
        if (duration >= 0 && duration <= 2000)
            transition->duration_ms = (guint)duration;
        native_shadow_transition_set_easing(registration, "ease", transition);
        native_shadow_transition_set_easing(registration, "easing", transition);
        break;
    }
    if (!selected_found) {
        transition->duration_ms = 0;
        transition->easing = "ease-out-cubic";
    }
    if (animation) {
        double duration = transition->duration_ms;
        if (native_rule_get_number(animation, "duration", &duration) && duration >= 0 &&
            duration <= 2000)
            transition->duration_ms = (guint)duration;
        native_shadow_transition_set_easing(animation, "ease", transition);
        native_shadow_transition_set_easing(animation, "easing", transition);
    }
}

static void native_window_shader_file_free(gpointer data) {
    NativeWindowShaderFile* file = data;
    if (!file)
        return;
    if (file->reload_timeout_id)
        g_source_remove(file->reload_timeout_id);
    g_clear_object(&file->monitor);
    g_free(file->path);
    g_free(file->source);
    g_free(file);
}

static gboolean native_window_shader_file_reload(NativeWindowShaderFile* file, GError** error) {
    g_autofree char* contents = NULL;
    gsize length = 0;
    if (!g_file_get_contents(file->path, &contents, &length, error))
        return FALSE;
    if (length > 64 * 1024) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA, "shader exceeds 64 KiB");
        return FALSE;
    }
    if (!g_utf8_validate(contents, (gssize)length, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "shader is not valid UTF-8");
        return FALSE;
    }
    if (!strstr(contents, "gnoblin_effect")) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "shader must define gnoblin_effect");
        return FALSE;
    }
    g_free(file->source);
    file->source = g_steal_pointer(&contents);
    return TRUE;
}

static gboolean native_window_shader_file_reload_timeout(gpointer user_data) {
    NativeWindowShaderFile* file = user_data;
    GnoblinNativeControl* control = file->control;
    file->reload_timeout_id = 0;
    if (!control || control->stopping)
        return G_SOURCE_REMOVE;
    g_autoptr(GError) error = NULL;
    if (native_window_shader_file_reload(file, &error))
        native_apply_all_window_rules(control);
    else
        g_warning("gnoblin-shader: keeping previous effect for %s: %s", file->path,
                  error ? error->message : "could not read shader");
    return G_SOURCE_REMOVE;
}

static void native_window_shader_file_changed(GFileMonitor* monitor, GFile* changed_file,
                                              GFile* other_file, GFileMonitorEvent event_type,
                                              gpointer user_data) {
    NativeWindowShaderFile* file = user_data;
    (void)monitor;
    (void)event_type;
    g_autofree char* changed_path = changed_file ? g_file_get_path(changed_file) : NULL;
    g_autofree char* other_path = other_file ? g_file_get_path(other_file) : NULL;
    if (g_strcmp0(changed_path, file->path) != 0 && g_strcmp0(other_path, file->path) != 0)
        return;
    if (!file->control || file->control->stopping)
        return;
    if (file->reload_timeout_id)
        g_source_remove(file->reload_timeout_id);
    file->reload_timeout_id = g_timeout_add(100, native_window_shader_file_reload_timeout, file);
}

static char* native_window_shader_resolve_path(const char* shader) {
    if (!shader || !*shader)
        return NULL;
    if (g_str_has_prefix(shader, "~/")) {
        g_autofree char* expanded = g_build_filename(g_get_home_dir(), shader + 2, NULL);
        return g_canonicalize_filename(expanded, NULL);
    }
    if (g_path_is_absolute(shader))
        return g_canonicalize_filename(shader, NULL);
    g_autofree char* config = NULL;
    const char* configured_path = g_getenv("GNOBLIN_CONFIG");
    if (configured_path && *configured_path)
        config = g_strdup(configured_path);
    else
        config = g_build_filename(g_get_user_config_dir(), "gnoblin", "init.lua", NULL);
    g_autofree char* directory = g_path_get_dirname(config);
    return g_canonicalize_filename(shader, directory);
}

static NativeWindowShaderFile* native_window_shader_file_get(GnoblinNativeControl* control,
                                                             const char* path) {
    NativeWindowShaderFile* file = g_hash_table_lookup(control->window_shader_files, path);
    if (file)
        return file;
    if (g_hash_table_size(control->window_shader_files) >= 128) {
        g_warning("gnoblin-shader: refusing more than 128 shader files in one session");
        return NULL;
    }
    file = g_new0(NativeWindowShaderFile, 1);
    file->control = control;
    file->path = g_strdup(path);
    g_hash_table_insert(control->window_shader_files, g_strdup(path), file);
    g_autoptr(GFile) shader_file = g_file_new_for_path(path);
    g_autoptr(GFile) parent = g_file_get_parent(shader_file);
    if (parent) {
        g_autoptr(GError) monitor_error = NULL;
        file->monitor =
            g_file_monitor_directory(parent, G_FILE_MONITOR_WATCH_MOVES, NULL, &monitor_error);
        if (file->monitor)
            g_signal_connect(file->monitor, "changed",
                             G_CALLBACK(native_window_shader_file_changed), file);
        else
            g_warning("gnoblin-shader: cannot watch %s: %s", path,
                      monitor_error ? monitor_error->message : "monitor unavailable");
    }
    g_autoptr(GError) error = NULL;
    if (!native_window_shader_file_reload(file, &error))
        g_warning("gnoblin-shader: keeping previous effect for %s: %s", path,
                  error ? error->message : "could not read shader");
    return file;
}

static void native_apply_window_rules(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control || !window)
        return;

    MetaWindowActor* actor = meta_window_actor_from_window(window);
    MetaSurfaceActor* surface = actor ? meta_window_actor_get_surface(actor) : NULL;
    if (!surface)
        surface = meta_wayland_layer_shell_get_actor(window);
    if (!surface)
        return;

    double radius = 0;
    double smoothing = 0;
    double border_width = 0;
    double border_color[4] = {128. / 255., 128. / 255., 128. / 255., 1.};
    double padding[4] = {0, 0, 0, 0};
    gboolean keep_maximized = TRUE;
    gboolean keep_fullscreen = FALSE;
    gboolean keep_tiled = FALSE;
    gboolean skip_libadwaita = TRUE;
    gboolean skip_libhandy = FALSE;
    gboolean remove_csd = FALSE;
    gboolean keep_shadow = FALSE;
    gboolean shadow_is_single_layer = FALSE;
    MetaGnoblinWindowShadowLayer shadow_layers[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS] = {0};
    guint n_shadow_layers = 0;
    MetaGnoblinWindowShadowTransition shadow_transition = {
        .duration_ms = 0,
        .easing = "ease-out-cubic",
    };
    g_autoptr(GVariant) shadow_animation_config = NULL;
    g_autofree char* mode = g_strdup("auto");
    g_autofree char* shader_path = NULL;
    g_autoptr(GVariant) shader_uniforms = NULL;
    gboolean shader_selected = FALSE;
    g_autoptr(GVariant) document = native_config_document(control);
    g_autoptr(GVariant) rules =
        document ? g_variant_lookup_value(document, "window-rules", NULL) : NULL;

    for (gsize i = 0; rules && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed_rule = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_is_of_type(boxed_rule, G_VARIANT_TYPE_VARIANT)
                                       ? g_variant_get_variant(boxed_rule)
                                       : g_variant_ref(boxed_rule);
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !native_window_rule_matches(match, window, control))
            continue;

        g_autoptr(GVariant) shader = g_variant_lookup_value(rule, "shader", NULL);
        if (shader && g_variant_is_of_type(shader, G_VARIANT_TYPE_STRING)) {
            shader_selected = TRUE;
            g_free(shader_path);
            const char* configured_shader = g_variant_get_string(shader, NULL);
            shader_path =
                *configured_shader ? native_window_shader_resolve_path(configured_shader) : NULL;
        }
        g_autoptr(GVariant) configured_uniforms =
            g_variant_lookup_value(rule, "shader-uniforms", NULL);
        if (configured_uniforms) {
            g_clear_pointer(&shader_uniforms, g_variant_unref);
            shader_uniforms = g_variant_ref(configured_uniforms);
        }

        g_autoptr(GVariant) corners = g_variant_lookup_value(rule, "corners", NULL);
        if (!corners || !g_variant_is_of_type(corners, G_VARIANT_TYPE_VARDICT))
            continue;

        native_rule_get_number(corners, "radius", &radius);
        native_rule_get_number(corners, "smoothing", &smoothing);
        if (!native_rule_get_number(corners, "border_width", &border_width))
            native_rule_get_number(corners, "border-width", &border_width);
        border_width = CLAMP(border_width, -40., 40.);
        if (!native_rule_get_color(corners, "border_color", border_color))
            native_rule_get_color(corners, "border-color", border_color);
        native_rule_get_padding(corners, padding);
        native_rule_get_boolean(corners, "keep_maximized", "keep-maximized", &keep_maximized);
        native_rule_get_boolean(corners, "keep_fullscreen", "keep-fullscreen", &keep_fullscreen);
        native_rule_get_boolean(corners, "keep_tiled", "keep-tiled", &keep_tiled);
        native_rule_get_boolean(corners, "skip_libadwaita", "skip-libadwaita", &skip_libadwaita);
        native_rule_get_boolean(corners, "skip_libhandy", "skip-libhandy", &skip_libhandy);
        native_rule_get_boolean(corners, "remove_csd", "remove-csd", &remove_csd);
        native_rule_get_boolean(corners, "keep_shadow", "keep-shadow", &keep_shadow);
        g_autoptr(GVariant) shadow = g_variant_lookup_value(corners, "shadow", NULL);
        if (shadow) {
            g_autoptr(GVariant) shadow_unboxed = NULL;
            GVariant* shadow_value = shadow;
            if (g_variant_is_of_type(shadow_value, G_VARIANT_TYPE_VARIANT)) {
                shadow_unboxed = g_variant_get_variant(shadow_value);
                shadow_value = shadow_unboxed;
            }
            const gboolean is_single_layer =
                g_variant_is_of_type(shadow_value, G_VARIANT_TYPE_VARDICT);
            const gboolean merge_single_layer =
                shadow_is_single_layer && is_single_layer && n_shadow_layers > 0;
            MetaGnoblinWindowShadowLayer parsed[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS] = {0};
            MetaGnoblinWindowShadowLayer* target = merge_single_layer ? shadow_layers : parsed;
            guint parsed_count = 0;
            if (native_shadow_layers_parse(shadow_value, target, &parsed_count,
                                           merge_single_layer)) {
                if (!merge_single_layer)
                    memcpy(shadow_layers, parsed, sizeof(shadow_layers));
                n_shadow_layers = parsed_count;
                shadow_is_single_layer = is_single_layer;
            }
        }
        GVariant* merged_animation =
            native_shadow_animation_merge(shadow_animation_config, corners);
        g_clear_pointer(&shadow_animation_config, g_variant_unref);
        shadow_animation_config = merged_animation;
        g_autoptr(GVariant) mode_value = g_variant_lookup_value(corners, "mode", NULL);
        if (mode_value && g_variant_is_of_type(mode_value, G_VARIANT_TYPE_STRING)) {
            g_free(mode);
            mode = g_strdup(g_variant_get_string(mode_value, NULL));
        }
    }

    if (!shader_selected || !shader_path) {
        meta_gnoblin_window_effects_clear_shader(CLUTTER_ACTOR(surface));
    } else {
        NativeWindowShaderFile* shader_file = native_window_shader_file_get(control, shader_path);
        if (shader_file && shader_file->source) {
            MetaGnoblinWindowShaderUniform uniforms[64] = {0};
            guint n_uniforms = 0;
            gboolean uniforms_valid = TRUE;
            if (shader_uniforms && g_variant_is_of_type(shader_uniforms, G_VARIANT_TYPE_VARDICT)) {
                GVariantIter iter;
                const char* name = NULL;
                GVariant* value = NULL;
                g_variant_iter_init(&iter, shader_uniforms);
                while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
                    g_autoptr(GVariant) uniform_value = value;
                    double number = 0;
                    if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_DOUBLE))
                        number = g_variant_get_double(uniform_value);
                    else if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_INT64))
                        number = (double)g_variant_get_int64(uniform_value);
                    else if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_INT32))
                        number = (double)g_variant_get_int32(uniform_value);
                    else
                        uniforms_valid = FALSE;
                    uniforms_valid &= isfinite(number);
                    if (n_uniforms < G_N_ELEMENTS(uniforms)) {
                        uniforms[n_uniforms].name = name;
                        uniforms[n_uniforms].value = number;
                        n_uniforms++;
                    } else {
                        uniforms_valid = FALSE;
                    }
                }
            } else if (shader_uniforms) {
                uniforms_valid = FALSE;
            }
            if (!uniforms_valid) {
                g_warning("gnoblin-shader: keeping previous effect for %s: invalid uniform table",
                          shader_path);
            } else {
                const double surface_width = clutter_actor_get_width(CLUTTER_ACTOR(surface));
                const double surface_height = clutter_actor_get_height(CLUTTER_ACTOR(surface));
                g_autoptr(GError) shader_error = NULL;
                if (!meta_gnoblin_window_effects_set_shader(
                        CLUTTER_ACTOR(surface), shader_file->source, uniforms, n_uniforms,
                        surface_width, surface_height, &shader_error))
                    g_warning("gnoblin-shader: keeping previous effect for %s: %s", shader_path,
                              shader_error ? shader_error->message : "could not apply shader");
            }
        }
    }

    if (!actor)
        return;

    native_shadow_animation_resolve(document, shadow_animation_config, &shadow_transition);

    const MetaMaximizeFlags maximize_flags = meta_window_get_maximize_flags(window);
    const gboolean partially_maximized = !!(maximize_flags & META_MAXIMIZE_HORIZONTAL) !=
                                         !!(maximize_flags & META_MAXIMIZE_VERTICAL);
    const gboolean tiled = meta_window_is_tiled_side_by_side(window) || partially_maximized;
    const MetaWindowType window_type = meta_window_get_window_type(window);
    const gboolean normal =
        (window_type == META_WINDOW_NORMAL || window_type == META_WINDOW_DIALOG ||
         window_type == META_WINDOW_MODAL_DIALOG) &&
        !meta_window_is_override_redirect(window);
    const gboolean allowed = normal && !g_str_equal(mode, "off") &&
                             (!meta_window_is_maximized(window) || keep_maximized) &&
                             (!meta_window_is_fullscreen(window) || keep_fullscreen) &&
                             (!tiled || keep_tiled);
    gboolean clip_enabled = allowed && radius > 0;
    const gboolean border_enabled = allowed && fabs(border_width) > 0.001;
    gboolean has_padding = FALSE;
    for (guint i = 0; i < G_N_ELEMENTS(padding); i++)
        has_padding |= fabs(padding[i]) > 0.001;
    double csd_insets[4] = {0, 0, 0, 0};
    gboolean csd_detected =
        clip_enabled && remove_csd && !has_padding && !window->minimized &&
        meta_gnoblin_window_effects_detect_csd(CLUTTER_ACTOR(actor), csd_insets);
    if (clip_enabled && g_str_equal(mode, "auto") && !csd_detected &&
        native_corner_toolkit_should_skip(control, window, skip_libadwaita, skip_libhandy))
        clip_enabled = FALSE;
    const double effect_radius = clip_enabled ? radius : border_enabled ? 0.5 : 0;
    const gboolean effect_enabled = clip_enabled || border_enabled;
    const double exponent = 2 + CLAMP(smoothing, 0, 1) * 4;
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor))
        meta_window_actor_wayland_set_rounded_clip(
            actor, effect_radius, exponent, g_str_equal(mode, "auto") && !csd_detected, padding);
    else if (META_IS_WINDOW_ACTOR_X11(actor))
        meta_window_actor_x11_set_rounded_clip(META_WINDOW_ACTOR_X11(actor), effect_radius,
                                               exponent, g_str_equal(mode, "auto") && !csd_detected,
                                               padding);
    meta_gnoblin_window_effects_set_rounded_border(CLUTTER_ACTOR(actor),
                                                   effect_enabled ? border_width : 0, border_color);
    const gboolean shadow_state_allowed =
        normal && !g_str_equal(mode, "off") && (!meta_window_is_maximized(window) || keep_shadow) &&
        (!meta_window_is_fullscreen(window) || keep_shadow) && (!tiled || keep_shadow);
    const double actor_width = clutter_actor_get_width(CLUTTER_ACTOR(actor));
    const double actor_height = clutter_actor_get_height(CLUTTER_ACTOR(actor));
    double shadow_geometry[4] = {0., 0., actor_width, actor_height};
    guint shadow_child_index = 0;
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor)) {
        meta_window_actor_wayland_get_surface_container_bounds(actor, shadow_geometry);
        shadow_child_index = meta_window_actor_wayland_get_shadow_child_index(actor);
    } else {
        if (surface && clutter_actor_has_allocation(CLUTTER_ACTOR(surface))) {
            ClutterActorBox surface_box;
            clutter_actor_get_allocation_box(CLUTTER_ACTOR(surface), &surface_box);
            shadow_geometry[0] = surface_box.x1;
            shadow_geometry[1] = surface_box.y1;
            shadow_geometry[2] = surface_box.x2;
            shadow_geometry[3] = surface_box.y2;
        }
    }
    double shadow_bounds[4] = {
        CLAMP(shadow_geometry[0] + padding[3], 0., actor_width),
        CLAMP(shadow_geometry[1] + padding[0], 0., actor_height),
        CLAMP(shadow_geometry[2] - padding[1], 0., actor_width),
        CLAMP(shadow_geometry[3] - padding[2], 0., actor_height),
    };
    meta_gnoblin_window_effects_set_window_shadow(
        CLUTTER_ACTOR(actor), shadow_state_allowed && n_shadow_layers > 0, shadow_bounds, radius,
        exponent, shadow_layers, n_shadow_layers, &shadow_transition, shadow_child_index);
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor))
        meta_window_actor_wayland_set_csd_reconstruction(actor, clip_enabled && csd_detected,
                                                         csd_insets);
    else if (META_IS_WINDOW_ACTOR_X11(actor))
        meta_window_actor_x11_set_csd_reconstruction(META_WINDOW_ACTOR_X11(actor),
                                                     clip_enabled && csd_detected, csd_insets);
}

static void native_apply_all_window_rules(GnoblinNativeControl* control) {
    if (!control || !control->display || !control->windows)
        return;
    GHashTableIter iter;
    gpointer window;
    g_hash_table_iter_init(&iter, control->windows);
    while (g_hash_table_iter_next(&iter, &window, NULL))
        native_apply_window_rules(control, window);
}

static void native_settings_changed(guint64 revision, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !control->display)
        return;
    clear_runtime_dynamic_shortcuts(control, "config_changed");
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session, "config_changed");

    g_autoptr(GVariant) config = native_config_document(control);
    g_autoptr(GVariant) window_management =
        config ? g_variant_lookup_value(config, "window-management", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) compositor_preferences =
        config ? g_variant_lookup_value(config, "compositor", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) input_preferences =
        config ? g_variant_lookup_value(config, "input", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) keyboard_preferences =
        input_preferences
            ? g_variant_lookup_value(input_preferences, "keyboard", G_VARIANT_TYPE_VARDICT)
            : NULL;
    meta_prefs_apply_gnoblin_window_preferences(window_management);
    meta_prefs_apply_gnoblin_compositor_preferences(compositor_preferences);
    meta_prefs_apply_gnoblin_keyboard_preferences(keyboard_preferences);
    native_apply_all_window_rules(control);
    g_autoptr(GVariant) configured_gestures =
        config ? g_variant_lookup_value(config, "touchpad-gestures", G_VARIANT_TYPE("av")) : NULL;
    g_clear_pointer(&control->native_touchpad_gestures, g_variant_unref);
    control->native_touchpad_gestures = filter_native_touchpad_gestures(configured_gestures);
    gnoblin_touchpad_router_reset(control->touchpad_router);

    /* Animation registrations live in the supervisor's Lua state. The cache
     * is updated there independently of the compositor config document. */
    native_publish_runtime_snapshot(control, "animations", NULL, revision);
}

static gboolean native_touchpad_action_supported(GVariant* gesture) {
    g_autoptr(GVariant) action = g_variant_lookup_value(gesture, "action", NULL);
    if (action && g_variant_is_of_type(action, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(action);
        g_variant_unref(g_steal_pointer(&action));
        action = unwrapped;
    }
    if (!action)
        return FALSE;
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_STRING))
        return FALSE;
    const char* name = g_variant_get_string(action, NULL);
    return g_str_equal(name, "workspace.next") || g_str_equal(name, "workspace.previous") ||
           g_str_equal(name, "window.close") || g_str_equal(name, "window.minimize") ||
           g_str_equal(name, "window.toggle-maximize");
}

static GVariant* filter_native_touchpad_gestures(GVariant* gestures) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
    if (gestures && g_variant_is_of_type(gestures, G_VARIANT_TYPE("av"))) {
        for (gsize i = 0; i < g_variant_n_children(gestures); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(gestures, i);
            g_autoptr(GVariant) gesture = g_variant_get_variant(boxed);
            g_autoptr(GVariant) command = g_variant_lookup_value(gesture, "command", NULL);
            gboolean supported = native_touchpad_action_supported(gesture);
            if (command && g_variant_is_of_type(command, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unwrapped = g_variant_get_variant(command);
                g_variant_unref(g_steal_pointer(&command));
                command = unwrapped;
            }
            if (command && g_variant_is_of_type(command, G_VARIANT_TYPE("av")) &&
                g_variant_n_children(command) > 0)
                supported = TRUE;
            if (supported)
                g_variant_builder_add_value(&builder, g_variant_new_variant(gesture));
        }
    }
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static gboolean run_native_touchpad_command(GVariant* gesture, GError** error) {
    g_autoptr(GVariant) command = g_variant_lookup_value(gesture, "command", NULL);
    if (command && g_variant_is_of_type(command, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(command);
        g_variant_unref(g_steal_pointer(&command));
        command = unwrapped;
    }
    if (!command || !g_variant_is_of_type(command, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(command) == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "touchpad gesture command is empty or invalid");
        return FALSE;
    }
    gsize count = g_variant_n_children(command);
    g_auto(GStrv) argv = g_new0(char*, count + 1);
    for (gsize i = 0; i < count; i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(command, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING) ||
            (i == 0 && !*g_variant_get_string(item, NULL))) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                "touchpad gesture command arguments must be strings and argv[0] must not be empty");
            return FALSE;
        }
        argv[i] = g_strdup(g_variant_get_string(item, NULL));
    }
    return g_spawn_async(NULL, argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, error);
}

static char* native_window_id(MetaWindow* window);

static void run_native_touchpad_action(GnoblinNativeControl* control, GVariant* gesture) {
    g_autoptr(GVariant) action = g_variant_lookup_value(gesture, "action", NULL);
    if (action && g_variant_is_of_type(action, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(action);
        g_variant_unref(g_steal_pointer(&action));
        action = unwrapped;
    }
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) arguments = NULL;
    g_autofree char* method = NULL;
    if (!action) {
        if (!run_native_touchpad_command(gesture, &error))
            g_warning("gnoblin-native-control: touchpad command failed: %s", error->message);
        return;
    }
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_STRING))
        return;
    const char* name = g_variant_get_string(action, NULL);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    if (g_str_equal(name, "workspace.next") || g_str_equal(name, "workspace.previous")) {
        if (control->wayland_compositor &&
            meta_wayland_session_lock_is_active(control->wayland_compositor))
            return;
        method = g_strdup(name);
    } else if (g_str_equal(name, "window.close") || g_str_equal(name, "window.minimize") ||
               g_str_equal(name, "window.toggle-maximize")) {
        if (control->wayland_compositor &&
            meta_wayland_session_lock_is_active(control->wayland_compositor))
            return;
        MetaWindow* window = meta_display_get_focus_window(control->display);
        if (!window)
            return;
        g_autofree char* window_id = native_window_id(window);
        if (g_str_equal(name, "window.toggle-maximize")) {
            method = g_strdup("window.set_maximized");
            g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(window_id));
            g_variant_builder_add(&builder, "{sv}", "enabled",
                                  g_variant_new_boolean(!meta_window_is_maximized(window)));
        } else {
            method = g_strdup(name);
            g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(window_id));
        }
    } else {
        return;
    }
    arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, method, arguments, &error);
    if (!result)
        g_warning("gnoblin-native-control: touchpad action %s failed: %s", name,
                  error ? error->message : "native action is unavailable");
}

static gboolean native_config_event(MetaDisplay* display, const char* event, GVariant* document,
                                    GVariant* payload, gpointer user_data) {
    (void)display;
    (void)document;
    GnoblinNativeControl* control = user_data;
    if (!event || !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    if (!g_str_equal(event, "mutter.touchpad.gesture"))
        return FALSE;

    static const struct {
        const char* source;
        const char* target;
        const GVariantType* type;
    } fields[] = {
        {"gesture", "gesture", G_VARIANT_TYPE_STRING},
        {"phase", "phase", G_VARIANT_TYPE_STRING},
        {"fingers", "fingers", G_VARIANT_TYPE_INT64},
        {"time", "input_time", G_VARIANT_TYPE_INT64},
        {"dx", "dx", G_VARIANT_TYPE_DOUBLE},
        {"dy", "dy", G_VARIANT_TYPE_DOUBLE},
        {"scale", "scale", G_VARIANT_TYPE_DOUBLE},
        {"angle_delta", "angle_delta", G_VARIANT_TYPE_DOUBLE},
    };
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.input.gesture");
    for (gsize i = 0; i < G_N_ELEMENTS(fields); i++) {
        g_autoptr(GVariant) value =
            g_variant_lookup_value(payload, fields[i].source, fields[i].type);
        if (value)
            json_object_set_member(object, fields[i].target, json_from_variant(value));
    }
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    gboolean claimed = FALSE;
    if (control->touchpad_router && control->native_touchpad_gestures) {
        const char* context = control->wayland_compositor && meta_wayland_session_lock_is_active(
                                                                 control->wayland_compositor)
                                  ? "unlock-screen"
                                  : "normal";
        g_autoptr(GVariant) matched = NULL;
        claimed = gnoblin_touchpad_router_handle(control->touchpad_router,
                                                 control->native_touchpad_gestures, payload,
                                                 context, &matched);
        if (matched)
            run_native_touchpad_action(control, matched);
    }
    publish_native_socket_event(control, root);
    return claimed;
}

static void native_native_event(MetaDisplay* display, const char* event, GVariant* payload,
                                gpointer user_data) {
    (void)display;
    GnoblinNativeControl* control = user_data;
    if (!event ||
        (!g_str_equal(event, "gnoblin.animation.started") &&
         !g_str_equal(event, "gnoblin.animation.finished") &&
         !g_str_equal(event, "gnoblin.window.activation-denied")) ||
        !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return;

    GVariantBuilder enriched;
    g_variant_builder_init(&enriched, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&enriched, "{sv}", "name", g_variant_new_string(event));
    g_variant_builder_add(&enriched, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_variant_builder_add(&enriched, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    GVariantIter iterator;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iterator, payload);
    while (g_variant_iter_next(&iterator, "{&sv}", &key, &value)) {
        g_variant_builder_add(&enriched, "{sv}", key, value);
        g_variant_unref(value);
    }
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&enriched));
    g_autoptr(JsonNode) root = json_from_variant(event_payload);
    JsonObject* object = json_node_get_object(root);
    /* Keep the animation's event field distinct from the socket event name. */
    JsonNode* animation_event = json_object_get_member(object, "event");
    if (animation_event) {
        json_object_set_member(object, "animation_event", json_node_copy(animation_event));
        json_object_remove_member(object, "event");
    }
    publish_native_socket_event(control, root);
    native_runtime_dispatch_event(control, event, event_payload);
}

static gboolean focus_token_has_valid_shape(const char* token) {
    if (!token || strlen(token) != 64)
        return FALSE;
    for (gsize i = 0; i < 64; i++)
        if (!g_ascii_isxdigit(token[i]))
            return FALSE;
    return TRUE;
}

static gboolean native_socket_has_exact_fields(JsonObject* object, const char* const* fields,
                                               gsize n_fields) {
    if (!object || json_object_get_size(object) != n_fields)
        return FALSE;
    GList* members = json_object_get_members(object);
    gboolean exact = TRUE;
    for (GList* item = members; item && exact; item = item->next) {
        const char* name = item->data;
        exact = FALSE;
        for (gsize i = 0; i < n_fields; i++) {
            if (g_str_equal(name, fields[i])) {
                exact = TRUE;
                break;
            }
        }
    }
    g_list_free(members);
    return exact;
}

static gboolean native_socket_has_nul_escape(const char* data, gsize length) {
    for (gsize i = 0; i < length; i++) {
        if (data[i] != '\\')
            continue;
        gsize slash_count = 1;
        while (i + slash_count < length && data[i + slash_count] == '\\')
            slash_count++;
        gsize escape = i + slash_count;
        if ((slash_count & 1) && escape + 5 < length && data[escape] == 'u' &&
            data[escape + 1] == '0' && data[escape + 2] == '0' && data[escape + 3] == '0' &&
            data[escape + 4] == '0')
            return TRUE;
        i += slash_count - 1;
    }
    return FALSE;
}

static gboolean native_socket_take_focus_grant(Client* client, JsonObject* arguments,
                                               NativeFocusGrant* out_grant, GError** error) {
    JsonNode* token_node = arguments ? json_object_get_member(arguments, "focus_context") : NULL;
    if (!client || client->closing || !client->control || !client->focus_grants || !token_node ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "operation requires a connection-bound focus_context");
        return FALSE;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "focus_context is invalid or revoked");
        return FALSE;
    }
    NativeFocusGrant* stored = g_hash_table_lookup(client->focus_grants, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "focus_context is expired, revoked, or belongs to another connection");
        return FALSE;
    }
    *out_grant = *stored;
    g_hash_table_remove(client->focus_grants, token);
    revoke_focus_grants_for_handle(client->control, out_grant->handle);
    return TRUE;
}

static GVariant* native_socket_focus_window(Client* client, JsonObject* json_arguments,
                                            GError** error) {
    if (!client || client->closing || !client->control || !json_arguments ||
        !client->focus_grants) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus requires a live connection grant");
        return NULL;
    }

    JsonNode* activation_node = json_object_get_member(json_arguments, "activation_token");
    if (activation_node) {
        if (client->api_minor < 32) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "XDG Activation window focus requires API version 1.32");
            return NULL;
        }
        JsonNode* id_node = json_object_get_member(json_arguments, "id");
        if (json_object_get_size(json_arguments) != 2 || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING ||
            !json_node_get_string(id_node)[0] || !JSON_NODE_HOLDS_VALUE(activation_node) ||
            json_node_get_value_type(activation_node) != G_TYPE_STRING ||
            !json_node_get_string(activation_node)[0] ||
            strlen(json_node_get_string(activation_node)) > 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.focus requires exactly id and activation_token strings");
            return NULL;
        }
        if (client->peer_pid <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "window.focus cannot verify the activation token owner");
            return NULL;
        }
        const char* wanted_id = json_node_get_string(id_node);
        MetaWindow* target = native_window_by_stable_id(client->control, wanted_id);
        if (!target || meta_window_is_skip_taskbar(target) ||
            meta_window_is_override_redirect(target)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "window ID not found; list windows first");
            return NULL;
        }
        if (!meta_wayland_activation_focus_window_with_token(
                client->control->wayland_compositor, target, json_node_get_string(activation_node),
                client->peer_pid)) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                "window.focus activation token was invalid, expired, or unavailable");
            return NULL;
        }
        GVariantBuilder result_builder;
        g_variant_builder_init(&result_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result_builder, "{sv}", "id", g_variant_new_string(wanted_id));
        return g_variant_ref_sink(g_variant_builder_end(&result_builder));
    }

    JsonNode* token_node = json_object_get_member(json_arguments, "focus_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus requires a connection-bound focus_context");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus context is invalid or revoked");
        return NULL;
    }
    NativeFocusGrant* stored = g_hash_table_lookup(client->focus_grants, token);
    if (!stored) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "window.focus context is invalid, expired, or belongs to another connection");
        return NULL;
    }

    NativeFocusGrant grant = *stored;
    g_hash_table_remove(client->focus_grants, token);
    revoke_focus_grants_for_handle(client->control, grant.handle);

    gboolean exact_fields = json_object_get_size(json_arguments) == 2;
    GList* members = json_object_get_members(json_arguments);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        if (!g_str_equal(name, "id") && !g_str_equal(name, "focus_context"))
            exact_fields = FALSE;
    }
    g_list_free(members);
    JsonNode* id_node = json_object_get_member(json_arguments, "id");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING || !json_node_get_string(id_node)[0])
        exact_fields = FALSE;

    GVariantBuilder arguments_builder;
    g_variant_builder_init(&arguments_builder, G_VARIANT_TYPE_VARDICT);
    if (exact_fields)
        g_variant_builder_add(&arguments_builder, "{sv}", "id",
                              g_variant_new_string(json_node_get_string(id_node)));
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&arguments_builder));
    return gnoblin_native_control_focus_window(client->control->display, arguments, grant.handle,
                                               grant.generation, error);
}

static GVariant* native_socket_begin_window_grab(Client* client, const char* method,
                                                 JsonObject* json_arguments, GError** error) {
    if (!client || client->closing || !client->control || !json_arguments ||
        !client->focus_grants ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation requires a live connection grant");
        return NULL;
    }
    JsonNode* token_node = json_object_get_member(json_arguments, "focus_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operation requires a connection-bound focus_context");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation context is invalid or revoked");
        return NULL;
    }
    NativeFocusGrant* stored = g_hash_table_lookup(client->focus_grants, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation context is invalid, expired, or belongs "
                            "to another connection");
        return NULL;
    }

    NativeFocusGrant grant = *stored;
    g_hash_table_remove(client->focus_grants, token);
    revoke_focus_grants_for_handle(client->control, grant.handle);

    gboolean resize = g_str_equal(method, "window.begin_resize");
    gboolean exact_fields = json_object_get_size(json_arguments) == (resize ? 3 : 2);
    GList* members = json_object_get_members(json_arguments);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        if (!g_str_equal(name, "id") && !g_str_equal(name, "edge") &&
            !g_str_equal(name, "focus_context"))
            exact_fields = FALSE;
    }
    g_list_free(members);
    JsonNode* id_node = json_object_get_member(json_arguments, "id");
    JsonNode* edge_node = json_object_get_member(json_arguments, "edge");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING || !json_node_get_string(id_node)[0] ||
        (resize && (!edge_node || !JSON_NODE_HOLDS_VALUE(edge_node) ||
                    json_node_get_value_type(edge_node) != G_TYPE_STRING)))
        exact_fields = FALSE;
    GVariantBuilder arguments_builder;
    g_variant_builder_init(&arguments_builder, G_VARIANT_TYPE_VARDICT);
    if (exact_fields) {
        g_variant_builder_add(&arguments_builder, "{sv}", "id",
                              g_variant_new_string(json_node_get_string(id_node)));
    }
    if (exact_fields && resize)
        g_variant_builder_add(&arguments_builder, "{sv}", "edge",
                              g_variant_new_string(json_node_get_string(edge_node)));
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&arguments_builder));
    return gnoblin_native_control_begin_window_grab(client->control->display, method, arguments,
                                                    grant.handle, grant.generation, error);
}

static GVariant* native_socket_begin_menu_window_grab(Client* client, const char* method,
                                                      JsonObject* json_arguments, GError** error) {
    if (!client || client->closing || !client->control || !client->menu_grants || !json_arguments ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu operation requires a live API 1.30 connection");
        return NULL;
    }
    JsonNode* token_node = json_object_get_member(json_arguments, "menu_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu operation requires its menu_context capability");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    NativeMenuGrant* stored =
        focus_token_has_valid_shape(token) ? g_hash_table_lookup(client->menu_grants, token) : NULL;
    if (!stored) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "menu_context is invalid, expired, consumed, or belongs to another connection");
        return NULL;
    }
    NativeMenuGrant grant = *stored;
    g_hash_table_remove(client->menu_grants, token);

    gboolean resize = g_str_equal(method, "window.begin_resize");
    gboolean exact = json_object_get_size(json_arguments) == (resize ? 2 : 1);
    GList* members = json_object_get_members(json_arguments);
    for (GList* item = members; item && exact; item = item->next) {
        const char* name = item->data;
        if (!g_str_equal(name, "menu_context") && !g_str_equal(name, "edge"))
            exact = FALSE;
    }
    g_list_free(members);
    JsonNode* edge = json_object_get_member(json_arguments, "edge");
    if (resize &&
        (!edge || !JSON_NODE_HOLDS_VALUE(edge) || json_node_get_value_type(edge) != G_TYPE_STRING))
        exact = FALSE;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    if (exact && resize)
        g_variant_builder_add(&builder, "{sv}", "edge",
                              g_variant_new_string(json_node_get_string(edge)));
    if (!exact)
        g_variant_builder_add(&builder, "{sv}", "_malformed", g_variant_new_boolean(TRUE));
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    /* The native operation consumes its context even when its arguments are malformed. */
    return gnoblin_native_control_begin_menu_window_grab(client->control->display, method,
                                                         arguments, grant.handle, grant.generation,
                                                         client->client_id, error);
}

static void native_launch_free(gpointer data) {
    NativeLaunch* launch = data;
    g_free(launch->token);
    g_free(launch->application);
    g_free(launch->normalized_application);
    g_free(launch->state);
    g_free(launch->initial_focus_id);
    g_clear_pointer(&launch->initial_window_ids, g_hash_table_unref);
    g_free(launch);
}

static char* normalize_launch_application(const char* value) {
    g_autofree char* normalized = g_ascii_strdown(value ? value : "", -1);
    g_strstrip(normalized);
    if (g_str_has_suffix(normalized, ".desktop"))
        normalized[strlen(normalized) - strlen(".desktop")] = '\0';
    return g_steal_pointer(&normalized);
}

static char* native_window_id(MetaWindow* window) {
    return g_strdup_printf("%u", meta_window_get_stable_sequence(window));
}

void gnoblin_native_control_window_menu_requested(MetaDisplay* display, MetaWindow* window,
                                                  MetaWindowMenuType menu, int x, int y) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* menu_type = menu == META_WINDOW_MENU_WM    ? "wm"
                            : menu == META_WINDOW_MENU_APP ? "app"
                                                           : NULL;
    if (!control || control->stopping || !window || !menu_type)
        return;
    g_autofree char* window_id = native_window_id(window);
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&fields, "{sv}", "menu_type", g_variant_new_string(menu_type));
    g_variant_builder_add(&fields, "{sv}", "x", g_variant_new_int32(x));
    g_variant_builder_add(&fields, "{sv}", "y", g_variant_new_int32(y));
    gint64 runtime_expiry = 0;
    guint64 runtime_handle =
        menu == META_WINDOW_MENU_WM
            ? native_menu_context_create(control, window_id, 0, &runtime_expiry)
            : 0;
    if (runtime_handle) {
        g_variant_builder_add(&fields, "{sv}", "_menu_context_handle",
                              g_variant_new_uint64(runtime_handle));
        g_variant_builder_add(&fields, "{sv}", "_menu_context_generation",
                              g_variant_new_uint64(native_config_generation(control)));
        g_variant_builder_add(&fields, "{sv}", "_menu_context_expires_at_us",
                              g_variant_new_int64(runtime_expiry));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));

    g_autoptr(JsonNode) event = json_from_variant(payload);
    if (!event || !JSON_NODE_HOLDS_OBJECT(event))
        return;
    JsonObject* event_object = json_node_get_object(event);
    json_object_set_string_member(event_object, "name", "gnoblin.window.menu-requested");
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 27 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.window.menu-requested"))
            continue;
        g_autoptr(JsonNode) socket_event = json_node_copy(event);
        JsonObject* socket_object = json_node_get_object(socket_event);
        json_object_remove_member(socket_object, "_menu_context_handle");
        json_object_remove_member(socket_object, "_menu_context_generation");
        json_object_remove_member(socket_object, "_menu_context_expires_at_us");
        native_menu_prune_client_grants(client, g_get_monotonic_time());
        if (menu == META_WINDOW_MENU_WM && client->event_api_minor >= 30 && client->menu_grants &&
            g_hash_table_size(client->menu_grants) < MAX_FOCUS_CONTEXTS) {
            char token[65];
            if (focus_token_random(token)) {
                gint64 expires = 0;
                guint64 handle =
                    native_menu_context_create(control, window_id, client->client_id, &expires);
                if (handle) {
                    NativeMenuGrant* grant = g_new0(NativeMenuGrant, 1);
                    grant->handle = handle;
                    grant->generation = native_config_generation(control);
                    grant->expires_at_us = expires;
                    g_hash_table_insert(client->menu_grants, g_strdup(token), grant);
                    json_object_set_string_member(socket_object, "menu_context", token);
                }
            }
        }
        json_object_set_string_member(socket_object, "event", "gnoblin.window.menu-requested");
        json_object_remove_member(socket_object, "name");
        g_autofree char* encoded = json_to_string(socket_event, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
    if (runtime_handle)
        native_runtime_dispatch_event(control, "gnoblin.window.menu-requested", payload);
}

#define NATIVE_DRAG_MOD_CONTROL (1u << 0)
#define NATIVE_DRAG_MAX_TARGETS 128
#define NATIVE_DRAG_LIFETIME_US (5 * G_USEC_PER_SEC)

static gboolean native_drag_monitor_snapshot(GnoblinNativeControl* control, MetaWindow* window,
                                             char** monitor_id, MtkRectangle* monitor_rect,
                                             MtkRectangle* work_area) {
    int monitor_number = meta_window_get_monitor(window);
    gboolean found = FALSE;
    GHashTableIter iter;
    gpointer value;
    meta_window_get_work_area_for_monitor(window, monitor_number, work_area);
    if (control->monitor_state) {
        g_hash_table_iter_init(&iter, control->monitor_state);
        while (g_hash_table_iter_next(&iter, NULL, &value)) {
            NativeMonitorState* state = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonObject* record = json_node_get_object(json_parser_get_root(parser));
            if (json_object_get_int_member_with_default(record, "index", -1) != monitor_number)
                continue;
            const char* id = json_object_get_string_member_with_default(record, "id", NULL);
            if (!id || !*id)
                continue;
            *monitor_id = g_strdup(id);
            *monitor_rect = (MtkRectangle){
                .x = (int)json_object_get_int_member(record, "x"),
                .y = (int)json_object_get_int_member(record, "y"),
                .width = (int)json_object_get_int_member(record, "width"),
                .height = (int)json_object_get_int_member(record, "height"),
            };
            found = monitor_rect->width > 0 && monitor_rect->height > 0;
            break;
        }
    }
    return found && work_area->width > 0 && work_area->height > 0;
}

static gboolean native_window_drag_refresh(GnoblinNativeControl* control, NativeWindowDrag* drag,
                                           MetaWindow* window, int pointer_x, int pointer_y,
                                           guint32 modifiers) {
    g_autofree char* monitor_id = NULL;
    MtkRectangle monitor_rect, work_area;
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) ||
        !native_drag_monitor_snapshot(control, window, &monitor_id, &monitor_rect, &work_area))
        return FALSE;
    g_autofree char* window_id = native_window_id(window);
    if (drag->window_id && !g_str_equal(drag->window_id, window_id))
        return FALSE;
    g_free(drag->window_id);
    drag->window_id = g_steal_pointer(&window_id);
    g_free(drag->monitor_id);
    drag->monitor_id = g_steal_pointer(&monitor_id);
    meta_window_get_frame_rect(window, &drag->frame);
    drag->monitor = monitor_rect;
    drag->work_area = work_area;
    drag->pointer_x = pointer_x;
    drag->pointer_y = pointer_y;
    drag->modifiers = modifiers;
    drag->maximized = meta_window_is_maximized(window);
    drag->settings_revision = native_config_revision(control);
    drag->expires_at_us = g_get_monotonic_time() + NATIVE_DRAG_LIFETIME_US;
    return TRUE;
}

static GVariant* native_drag_rect_variant(const MtkRectangle* rect) {
    GVariantBuilder value;
    g_variant_builder_init(&value, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&value, "{sv}", "x", g_variant_new_int32(rect->x));
    g_variant_builder_add(&value, "{sv}", "y", g_variant_new_int32(rect->y));
    g_variant_builder_add(&value, "{sv}", "width", g_variant_new_int32(rect->width));
    g_variant_builder_add(&value, "{sv}", "height", g_variant_new_int32(rect->height));
    return g_variant_ref_sink(g_variant_builder_end(&value));
}

static GVariant* native_drag_event_payload(NativeWindowDrag* drag) {
    GVariantBuilder record, pointer, modifiers;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&pointer, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&modifiers, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_uint64(drag->id));
    g_variant_builder_add(&record, "{sv}", "window_id", g_variant_new_string(drag->window_id));
    g_variant_builder_add(&record, "{sv}", "monitor_id", g_variant_new_string(drag->monitor_id));
    g_variant_builder_add(&record, "{sv}", "settings_revision",
                          g_variant_new_uint64(drag->settings_revision));
    g_variant_builder_add(&record, "{sv}", "pointer_x", g_variant_new_int32(drag->pointer_x));
    g_variant_builder_add(&record, "{sv}", "pointer_y", g_variant_new_int32(drag->pointer_y));
    g_variant_builder_add(&record, "{sv}", "maximized", g_variant_new_boolean(drag->maximized));
    g_variant_builder_add(&pointer, "{sv}", "x", g_variant_new_int32(drag->pointer_x));
    g_variant_builder_add(&pointer, "{sv}", "y", g_variant_new_int32(drag->pointer_y));
    g_variant_builder_add(&modifiers, "{sv}", "control",
                          g_variant_new_boolean((drag->modifiers & CLUTTER_CONTROL_MASK) != 0));
    g_variant_builder_add(&modifiers, "{sv}", "shift",
                          g_variant_new_boolean((drag->modifiers & CLUTTER_SHIFT_MASK) != 0));
    g_autoptr(GVariant) monitor = native_drag_rect_variant(&drag->monitor);
    g_autoptr(GVariant) work_area = native_drag_rect_variant(&drag->work_area);
    g_autoptr(GVariant) frame = native_drag_rect_variant(&drag->frame);
    g_variant_builder_add(&record, "{sv}", "monitor", monitor);
    g_variant_builder_add(&record, "{sv}", "work_area", work_area);
    g_variant_builder_add(&record, "{sv}", "frame", frame);
    g_variant_builder_add(&record, "{sv}", "pointer", g_variant_builder_end(&pointer));
    g_variant_builder_add(&record, "{sv}", "modifiers", g_variant_builder_end(&modifiers));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static gboolean rect_contains(const MtkRectangle* outer, const MtkRectangle* inner) {
    return inner->x >= outer->x && inner->y >= outer->y &&
           (gint64)inner->x + inner->width <= (gint64)outer->x + outer->width &&
           (gint64)inner->y + inner->height <= (gint64)outer->y + outer->height;
}

static gboolean native_variant_rect(GVariant* value, MtkRectangle* rect) {
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) &&
           g_variant_n_children(value) == 4 && g_variant_lookup(value, "x", "i", &rect->x) &&
           g_variant_lookup(value, "y", "i", &rect->y) &&
           g_variant_lookup(value, "width", "i", &rect->width) &&
           g_variant_lookup(value, "height", "i", &rect->height) && rect->x >= -100000 &&
           rect->x <= 100000 && rect->y >= -100000 && rect->y <= 100000 && rect->width >= 1 &&
           rect->width <= 32768 && rect->height >= 1 && rect->height <= 32768;
}

static gboolean native_snap_target_parse(GVariant* value, NativeWindowDrag* drag,
                                         NativeSnapTarget** out_target) {
    const char* id = NULL;
    g_autoptr(GVariant) hit = NULL;
    g_autoptr(GVariant) frame = NULL;
    g_autoptr(GVariant) required = NULL;
    g_autoptr(GVariant) forbidden = NULL;
    g_autoptr(GVariant) maximize_value = NULL;
    gboolean maximize = FALSE;
    NativeSnapTarget* target = g_new0(NativeSnapTarget, 1);
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
        goto invalid;
    if (g_variant_n_children(value) < 3 || g_variant_n_children(value) > 6)
        goto invalid;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_iter_init(&fields, value);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) owned_field = field_value;
        if (!g_str_equal(field_name, "id") && !g_str_equal(field_name, "hit") &&
            !g_str_equal(field_name, "frame") && !g_str_equal(field_name, "maximize") &&
            !g_str_equal(field_name, "required_modifiers") &&
            !g_str_equal(field_name, "forbidden_modifiers"))
            goto invalid;
    }
    hit = g_variant_lookup_value(value, "hit", G_VARIANT_TYPE_VARDICT);
    frame = g_variant_lookup_value(value, "frame", G_VARIANT_TYPE_VARDICT);
    required = g_variant_lookup_value(value, "required_modifiers", NULL);
    forbidden = g_variant_lookup_value(value, "forbidden_modifiers", NULL);
    if (!g_variant_lookup(value, "id", "&s", &id) || !id || !*id || strlen(id) > 64 ||
        !g_utf8_validate(id, -1, NULL) || !native_variant_rect(hit, &target->hit) ||
        !native_variant_rect(frame, &target->frame) ||
        !rect_contains(&drag->work_area, &target->hit) ||
        !rect_contains(&drag->work_area, &target->frame) ||
        (required && !g_variant_is_of_type(required, G_VARIANT_TYPE_STRING_ARRAY)) ||
        (forbidden && !g_variant_is_of_type(forbidden, G_VARIANT_TYPE_STRING_ARRAY)))
        goto invalid;
    if (required) {
        for (gsize i = 0; i < g_variant_n_children(required); i++) {
            const char* modifier = NULL;
            g_variant_get_child(required, i, "&s", &modifier);
            if (!g_str_equal(modifier, "control") ||
                (target->required_modifiers & NATIVE_DRAG_MOD_CONTROL))
                goto invalid;
            target->required_modifiers |= NATIVE_DRAG_MOD_CONTROL;
        }
    }
    if (forbidden) {
        for (gsize i = 0; i < g_variant_n_children(forbidden); i++) {
            const char* modifier = NULL;
            g_variant_get_child(forbidden, i, "&s", &modifier);
            if (!g_str_equal(modifier, "control") ||
                (target->forbidden_modifiers & NATIVE_DRAG_MOD_CONTROL))
                goto invalid;
            target->forbidden_modifiers |= NATIVE_DRAG_MOD_CONTROL;
        }
    }
    if ((target->required_modifiers & target->forbidden_modifiers) != 0)
        goto invalid;
    maximize_value = g_variant_lookup_value(value, "maximize", NULL);
    if (maximize_value && !g_variant_lookup(value, "maximize", "b", &maximize))
        goto invalid;
    target->maximize = maximize;
    target->id = g_strdup(id);
    *out_target = target;
    return TRUE;
invalid:
    native_snap_target_free(target);
    return FALSE;
}

static gboolean native_snap_offer_set_targets(NativeWindowDrag* drag, GVariant* targets,
                                              GError** error);

static gboolean native_runtime_window_snap_offer(GnoblinNativeControl* control, GVariant* arguments,
                                                 guint64 owner_generation, GError** error) {
    guint64 drag_id = 0, settings_revision = 0;
    g_autoptr(GVariant) targets =
        g_variant_lookup_value(arguments, "targets", G_VARIANT_TYPE("av"));
    if (!g_variant_lookup(arguments, "drag_id", "t", &drag_id) || !drag_id ||
        !g_variant_lookup(arguments, "settings_revision", "t", &settings_revision) ||
        !owner_generation || owner_generation != control->runtime_generation ||
        settings_revision != native_config_revision(control) || !targets ||
        g_variant_n_children(targets) == 0 ||
        g_variant_n_children(targets) > NATIVE_DRAG_MAX_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap.offer requires a live drag, revision and 1-128 targets");
        return FALSE;
    }
    NativeWindowDrag* drag = g_hash_table_lookup(control->window_drags, &drag_id);
    if (!drag || drag->settings_revision != settings_revision ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->owner_generation != owner_generation || drag->socket_owner_client_id != 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap.offer belongs to an ended drag or stale runtime");
        return FALSE;
    }
    if (!native_snap_offer_set_targets(drag, targets, error))
        return FALSE;
    drag->owner_generation = owner_generation;
    drag->runtime_offer_claimed = TRUE;
    return TRUE;
}

static void native_publish_window_drag_socket_event(GnoblinNativeControl* control,
                                                    NativeWindowDrag* drag, const char* event_name,
                                                    GVariant* payload) {
    g_autoptr(JsonNode) base = json_from_variant(payload);
    if (!JSON_NODE_HOLDS_OBJECT(base))
        return;
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 26 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, event_name))
            continue;
        const char* token = drag && drag->client_tokens
                                ? g_hash_table_lookup(drag->client_tokens, &client->client_id)
                                : NULL;
        if (!token && !g_str_equal(event_name, "gnoblin.window.drag.ended"))
            continue;
        g_autoptr(JsonNode) event = json_node_copy(base);
        JsonObject* object = json_node_get_object(event);
        json_object_set_string_member(object, "event", event_name);
        if (token)
            json_object_set_string_member(object, "drag_token", token);
        g_autofree char* encoded = json_to_string(event, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
}

static gboolean native_snap_offer_set_targets(NativeWindowDrag* drag, GVariant* targets,
                                              GError** error) {
    if (!targets || !g_variant_is_of_type(targets, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(targets) == 0 ||
        g_variant_n_children(targets) > NATIVE_DRAG_MAX_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap.offer requires 1-128 targets");
        return FALSE;
    }
    GPtrArray* parsed = g_ptr_array_new_with_free_func(native_snap_target_free);
    g_autoptr(GHashTable) ids = g_hash_table_new(g_str_hash, g_str_equal);
    for (gsize i = 0; i < g_variant_n_children(targets); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(targets, i);
        g_autoptr(GVariant) record = g_variant_get_variant(boxed);
        NativeSnapTarget* target = NULL;
        if (!native_snap_target_parse(record, drag, &target) ||
            g_hash_table_contains(ids, target ? target->id : "")) {
            native_snap_target_free(target);
            g_ptr_array_unref(parsed);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "window.snap.offer target %zu is invalid or duplicated", i + 1);
            return FALSE;
        }
        g_hash_table_add(ids, target->id);
        g_ptr_array_add(parsed, target);
    }
    g_ptr_array_unref(drag->targets);
    drag->targets = parsed;
    drag->expires_at_us = g_get_monotonic_time() + NATIVE_DRAG_LIFETIME_US;
    return TRUE;
}

guint64 gnoblin_native_control_window_drag_begin(MetaDisplay* display, MetaWindow* window,
                                                 int pointer_x, int pointer_y, guint32 modifiers) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->supervised_runtime ||
        !control->runtime_generation || !window || !control->window_drags)
        return 0;
    native_cancel_window_drags(control, "preempted");
    NativeWindowDrag* drag = g_new0(NativeWindowDrag, 1);
    drag->id = ++control->next_window_drag_id;
    if (!drag->id)
        drag->id = ++control->next_window_drag_id;
    drag->owner_generation = control->runtime_generation;
    drag->targets = g_ptr_array_new_with_free_func(native_snap_target_free);
    drag->client_tokens = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    if (!native_window_drag_refresh(control, drag, window, pointer_x, pointer_y, modifiers)) {
        native_window_drag_free(drag);
        return 0;
    }
    drag->original_frame = drag->frame;
    guint token_count = 0;
    GHashTableIter clients_iter;
    gpointer client_value;
    g_hash_table_iter_init(&clients_iter, control->clients);
    while (g_hash_table_iter_next(&clients_iter, NULL, &client_value) &&
           token_count < MAX_WINDOW_DRAG_CLIENTS) {
        Client* client = client_value;
        if (client->closing || client->api_minor < 26 || client->event_api_minor < 26 ||
            !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.window.drag.started"))
            continue;
        char token[65];
        if (!focus_token_random(token))
            continue;
        guint64* client_key = g_new(guint64, 1);
        *client_key = client->client_id;
        g_hash_table_insert(drag->client_tokens, client_key, g_strdup(token));
        token_count++;
    }
    g_hash_table_remove_all(control->window_drags);
    g_hash_table_insert(control->window_drags, g_memdup2(&drag->id, sizeof(drag->id)), drag);
    g_autoptr(GVariant) payload = native_drag_event_payload(drag);
    native_runtime_dispatch_event(control, "gnoblin.window.drag.started", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.started", payload);
    return drag->id;
}

void gnoblin_native_control_window_drag_update(MetaDisplay* display, guint64 drag_id,
                                               MetaWindow* window, int pointer_x, int pointer_y,
                                               guint32 modifiers) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!drag ||
        !native_window_drag_refresh(control, drag, window, pointer_x, pointer_y, modifiers))
        return;
    g_autoptr(GVariant) payload = native_drag_event_payload(drag);
    native_runtime_dispatch_event(control, "gnoblin.window.drag.updated", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.updated", payload);
}

void gnoblin_native_control_window_drag_end(MetaDisplay* display, guint64 drag_id,
                                            gboolean committed, const char* reason) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!drag)
        return;
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "drag_id", g_variant_new_uint64(drag_id));
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(drag->window_id));
    g_variant_builder_add(&result, "{sv}", "reason", g_variant_new_string(reason));
    g_variant_builder_add(&result, "{sv}", "committed", g_variant_new_boolean(committed));
    if (committed && drag->committed_target_id)
        g_variant_builder_add(&result, "{sv}", "target_id",
                              g_variant_new_string(drag->committed_target_id));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&result));
    native_runtime_dispatch_event(control, "gnoblin.window.drag.ended", payload);
    native_publish_window_drag_socket_event(control, drag, "gnoblin.window.drag.ended", payload);
    g_hash_table_remove(control->window_drags, &drag_id);
}

gboolean gnoblin_native_control_take_window_drag_snap(MetaDisplay* display, guint64 drag_id,
                                                      MetaWindow* window, int release_x,
                                                      int release_y, guint32 modifiers,
                                                      MtkRectangle* out_frame, char** out_target_id,
                                                      gboolean* out_maximize) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!control || control->stopping || !control->supervised_runtime ||
        !control->runtime_generation || !drag || !window ||
        drag->owner_generation != control->runtime_generation ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->settings_revision != native_config_revision(control) ||
        !native_window_drag_refresh(control, drag, window, release_x, release_y, modifiers) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return FALSE;
    guint control_modifier = (modifiers & CLUTTER_CONTROL_MASK) ? NATIVE_DRAG_MOD_CONTROL : 0;
    for (guint i = 0; i < drag->targets->len; i++) {
        NativeSnapTarget* target = g_ptr_array_index(drag->targets, i);
        if (release_x < target->hit.x || release_y < target->hit.y ||
            release_x >= (gint64)target->hit.x + target->hit.width ||
            release_y >= (gint64)target->hit.y + target->hit.height ||
            (control_modifier & target->required_modifiers) != target->required_modifiers ||
            (control_modifier & target->forbidden_modifiers) != 0 ||
            !rect_contains(&drag->work_area, &target->hit) ||
            !rect_contains(&drag->work_area, &target->frame))
            continue;
        *out_frame = target->frame;
        *out_target_id = g_strdup(target->id);
        *out_maximize = target->maximize;
        if (target->maximize) {
            g_hash_table_remove(control->snap_restore_frames, window);
        } else if (!g_hash_table_contains(control->snap_restore_frames, window)) {
            MtkRectangle* original_frame = g_new(MtkRectangle, 1);
            *original_frame = drag->original_frame;
            g_hash_table_insert(control->snap_restore_frames, window, original_frame);
        }
        g_free(drag->committed_target_id);
        drag->committed_target_id = g_strdup(target->id);
        return TRUE;
    }
    return FALSE;
}

GVariant* gnoblin_native_control_focus_window(MetaDisplay* display, GVariant* arguments,
                                              guint64 context_handle, guint64 generation,
                                              GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->focus_contexts || !context_handle) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus has no active native focus context");
        return NULL;
    }

    prune_focus_contexts(control, g_get_monotonic_time());

    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus activation context was already used or revoked");
        return NULL;
    }
    NativeFocusContext context = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time()) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus activation context expired or belongs to an old runtime");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus is unavailable while the session is locked");
        return NULL;
    }

    const char* wanted_id = NULL;
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1 ||
        !g_variant_lookup(arguments, "id", "&s", &wanted_id) || !*wanted_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.focus requires a stable string ID");
        return NULL;
    }
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (g_str_equal(id, wanted_id)) {
            target = window;
            break;
        }
    }
    if (!target) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window ID not found; list windows first");
        return NULL;
    }

    meta_window_activate_with_workspace(target, context.timestamp, NULL);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(wanted_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static gboolean native_text_modifiers_allowed(GnoblinNativeControl* control,
                                              xkb_mod_mask_t allowed) {
    ClutterKeymap* keymap =
        control && control->input_seat ? clutter_seat_get_keymap(control->input_seat) : NULL;
    if (!keymap)
        return FALSE;
    xkb_mod_mask_t depressed = 0, latched = 0, locked = 0;
    clutter_keymap_get_modifier_state(keymap, &depressed, &latched, &locked);
    return ((depressed | latched | locked) & ~allowed) == 0;
}

static gboolean native_text_valid(const char* text) {
    if (!text || !*text || strlen(text) > 256 || !g_utf8_validate(text, -1, NULL))
        return FALSE;
    for (const char* cursor = text; *cursor; cursor = g_utf8_next_char(cursor))
        if (g_unichar_iscntrl(g_utf8_get_char(cursor)))
            return FALSE;
    return TRUE;
}

static GVariant* native_insert_text_owned(MetaDisplay* display, GVariant* arguments,
                                          guint64 socket_owner_client_id, GError** error);

static GVariant* native_socket_create_text_target(Client* client, JsonObject* arguments,
                                                  GError** error) {
    NativeFocusGrant grant = {0};
    if (!native_socket_take_focus_grant(client, arguments, &grant, error))
        return NULL;

    GVariant* result = gnoblin_native_control_create_text_target(
        client->control->display, grant.handle, grant.generation, grant.expires_at_us, error);
    if (!result)
        return NULL;

    const char* token = NULL;
    if (!g_variant_lookup(result, "target", "&s", &token) || !token) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native text target result has no target token");
        return NULL;
    }
    NativeTextTarget* target = g_hash_table_lookup(client->control->text_targets, token);
    if (!target) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native text target was not retained");
        return NULL;
    }
    target->socket_owner_client_id = client->client_id;

    const char* fields[] = {"focus_context"};
    if (!native_socket_has_exact_fields(arguments, fields, G_N_ELEMENTS(fields))) {
        g_hash_table_remove(client->control->text_targets, token);
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.text_target accepts only focus_context");
        return NULL;
    }
    return result;
}

static GVariant* native_socket_insert_text(Client* client, JsonObject* json_arguments,
                                           gboolean has_nul_escape, GError** error) {
    JsonNode* token_node = json_arguments ? json_object_get_member(json_arguments, "target") : NULL;
    if (!client || client->closing || !client->control || !client->control->text_targets ||
        !token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text requires a target token");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget is invalid or revoked");
        return NULL;
    }
    NativeTextTarget* stored = g_hash_table_lookup(client->control->text_targets, token);
    if (!stored || stored->socket_owner_client_id != client->client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget is expired, revoked, or belongs to another connection");
        return NULL;
    }

    if (has_nul_escape) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text rejects embedded NUL characters");
        return NULL;
    }

    const char* fields[] = {"target", "text"};
    JsonNode* text_node = json_object_get_member(json_arguments, "text");
    if (!native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) ||
        !text_node || !JSON_NODE_HOLDS_VALUE(text_node) ||
        json_node_get_value_type(text_node) != G_TYPE_STRING ||
        !native_text_valid(json_node_get_string(text_node))) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "input.insert_text requires 1-256 bytes of UTF-8 text without controls");
        return NULL;
    }

    g_autoptr(JsonNode) arguments_node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(arguments_node, json_object_ref(json_arguments));
    g_autoptr(GVariant) arguments = variant_from_json(arguments_node);
    if (!arguments) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text arguments are invalid");
        return NULL;
    }
    return native_insert_text_owned(client->control->display, arguments, client->client_id, error);
}

static gboolean focus_identity_matches(GnoblinNativeControl* control, guint64 surface_id,
                                       guint64 client_id, guint64 focus_epoch) {
    MetaWaylandSeat* seat =
        control && control->wayland_compositor ? control->wayland_compositor->seat : NULL;
    guint64 current_surface = 0, current_client = 0, current_epoch = 0;
    return seat &&
           meta_wayland_seat_get_gnoblin_focus_identity(seat, &current_surface, &current_client,
                                                        &current_epoch) &&
           current_surface == surface_id && current_client == client_id &&
           current_epoch == focus_epoch;
}

static gboolean native_monitor_lookup(GnoblinNativeControl* control, const char* wanted_id,
                                      int* monitor_number, MtkRectangle* monitor_rect) {
    GHashTableIter iter;
    gpointer value;
    if (!control->monitor_state || !wanted_id)
        return FALSE;
    g_hash_table_iter_init(&iter, control->monitor_state);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeMonitorState* state = value;
        g_autoptr(JsonParser) parser = json_parser_new();
        if (!json_parser_load_from_data(parser, state->json, -1, NULL))
            continue;
        JsonObject* record = json_node_get_object(json_parser_get_root(parser));
        const char* id = json_object_get_string_member_with_default(record, "id", NULL);
        if (!id || !g_str_equal(id, wanted_id))
            continue;
        *monitor_number = (int)json_object_get_int_member(record, "index");
        *monitor_rect = (MtkRectangle){
            .x = (int)json_object_get_int_member(record, "x"),
            .y = (int)json_object_get_int_member(record, "y"),
            .width = (int)json_object_get_int_member(record, "width"),
            .height = (int)json_object_get_int_member(record, "height"),
        };
        return *monitor_number >= 0 && monitor_rect->width > 0 && monitor_rect->height > 0;
    }
    return FALSE;
}

GVariant* gnoblin_native_control_create_snap_context(MetaDisplay* display, guint64 context_handle,
                                                     guint64 generation, gint64 expires_at_us,
                                                     GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !context_handle || !control->focus_contexts ||
        !control->snap_contexts) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context has no live FocusContext");
        return NULL;
    }
    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context FocusContext was already consumed or revoked");
        return NULL;
    }
    NativeFocusContext source = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (source.generation != generation || generation != native_config_generation(control) ||
        source.expires_at_us != expires_at_us || expires_at_us <= g_get_monotonic_time() ||
        !focus_identity_matches(control, source.surface_id, source.client_id, source.focus_epoch) ||
        !native_text_modifiers_allowed(control, source.allowed_modifier_mask) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context FocusContext expired or no longer matches focus");
        return NULL;
    }
    if (g_hash_table_size(control->snap_contexts) >= MAX_SNAP_CONTEXTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "too many active snap contexts; retry after one expires");
        return NULL;
    }
    MetaWindow* window = meta_display_get_focus_window(display);
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) || !meta_window_allows_move(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "focused window cannot be snapped");
        return NULL;
    }
    g_autofree char* window_id = native_window_id(window);
    g_autofree char* monitor_id = NULL;
    MtkRectangle monitor_rect, work_area;
    if (!native_drag_monitor_snapshot(control, window, &monitor_id, &monitor_rect, &work_area)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "focused window has no current monitor");
        return NULL;
    }
    char token[65] = {0};
    gboolean unique = FALSE;
    for (guint attempt = 0; attempt < 4; attempt++) {
        if (!focus_token_random(token))
            break;
        if (!g_hash_table_contains(control->snap_contexts, token)) {
            unique = TRUE;
            break;
        }
    }
    if (!unique) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not allocate one-use SnapContext");
        return NULL;
    }
    NativeSnapContext* snap = g_new0(NativeSnapContext, 1);
    snap->settings_revision = native_config_revision(control);
    snap->native_generation = generation;
    snap->expires_at_us = expires_at_us;
    snap->window_id = g_strdup(window_id);
    snap->monitor_id = g_strdup(monitor_id);
    snap->work_area = work_area;
    meta_window_get_frame_rect(window, &snap->original_frame);
    g_hash_table_insert(control->snap_contexts, g_strdup(token), snap);

    GVariantBuilder result, monitor, area;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&monitor, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&area, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "context", g_variant_new_string(token));
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&result, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&result, "{sv}", "expires_at_us", g_variant_new_int64(expires_at_us));
#define ADD_SNAP_RECT(builder, rect)                                                               \
    g_variant_builder_add((builder), "{sv}", "x", g_variant_new_int32((rect).x));                  \
    g_variant_builder_add((builder), "{sv}", "y", g_variant_new_int32((rect).y));                  \
    g_variant_builder_add((builder), "{sv}", "width", g_variant_new_int32((rect).width));          \
    g_variant_builder_add((builder), "{sv}", "height", g_variant_new_int32((rect).height))
    ADD_SNAP_RECT(&monitor, monitor_rect);
    ADD_SNAP_RECT(&area, work_area);
#undef ADD_SNAP_RECT
    g_variant_builder_add(&result, "{sv}", "monitor", g_variant_builder_end(&monitor));
    g_variant_builder_add(&result, "{sv}", "work_area", g_variant_builder_end(&area));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* native_socket_create_snap_context(Client* client, JsonObject* arguments,
                                                   GError** error) {
    NativeFocusGrant grant = {0};
    if (!native_socket_take_focus_grant(client, arguments, &grant, error))
        return NULL;
    GVariant* result = gnoblin_native_control_create_snap_context(
        client->control->display, grant.handle, grant.generation, grant.expires_at_us, error);
    if (!result)
        return NULL;

    const char* token = NULL;
    if (!g_variant_lookup(result, "context", "&s", &token) || !token) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native SnapContext result has no context token");
        return NULL;
    }
    NativeSnapContext* context = g_hash_table_lookup(client->control->snap_contexts, token);
    if (!context) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native SnapContext was not retained");
        return NULL;
    }
    context->socket_owner_client_id = client->client_id;

    const char* fields[] = {"focus_context"};
    if (!native_socket_has_exact_fields(arguments, fields, G_N_ELEMENTS(fields))) {
        g_hash_table_remove(client->control->snap_contexts, token);
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap_context accepts only focus_context");
        return NULL;
    }
    GVariantBuilder public_result;
    g_variant_builder_init(&public_result, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, result);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) owned_value = value;
        if (!g_str_equal(key, "expires_at_us"))
            g_variant_builder_add(&public_result, "{sv}", key, owned_value);
    }
    g_variant_unref(result);
    return g_variant_ref_sink(g_variant_builder_end(&public_result));
}

static GVariant* native_commit_snap_context_owned(MetaDisplay* display, GVariant* arguments,
                                                  guint64 socket_owner_client_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* token = NULL;
    const char* monitor_id = NULL;
    g_autoptr(GVariant) frame_value = NULL;
    MtkRectangle frame, monitor_rect, work_area;
    if (!control || control->stopping || !control->snap_contexts ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(arguments, "context", "&s", &token) || !token ||
        !focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a SnapContext token");
        return NULL;
    }
    NativeSnapContext* stored = g_hash_table_lookup(control->snap_contexts, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext is expired, consumed or revoked");
        return NULL;
    }
    if (stored->socket_owner_client_id != socket_owner_client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext belongs to another connection");
        return NULL;
    }
    NativeSnapContext context = *stored;
    g_autofree char* window_id = g_strdup(stored->window_id);
    g_hash_table_remove(control->snap_contexts, token);
    if (g_variant_n_children(arguments) != 3 ||
        !g_variant_lookup(arguments, "monitor_id", "&s", &monitor_id) || !monitor_id ||
        !*monitor_id ||
        !(frame_value = g_variant_lookup_value(arguments, "frame", G_VARIANT_TYPE_VARDICT)) ||
        !native_variant_rect(frame_value, &frame)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a monitor_id and valid frame");
        return NULL;
    }
    int monitor_number = -1;
    if (context.settings_revision != native_config_revision(control) ||
        context.native_generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time() ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)) ||
        !native_monitor_lookup(control, monitor_id, &monitor_number, &monitor_rect)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext or target monitor is stale");
        return NULL;
    }
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* window = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* candidate = item->data;
        g_autofree char* id = native_window_id(candidate);
        if (g_str_equal(id, window_id)) {
            window = candidate;
            break;
        }
    }
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) || !meta_window_allows_move(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "SnapContext window is no longer available for moving");
        return NULL;
    }
    meta_window_get_work_area_for_monitor(window, monitor_number, &work_area);
    if (!rect_contains(&work_area, &frame)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "snap frame falls outside the target work area");
        return NULL;
    }
    if (meta_window_get_maximize_flags(window) != 0)
        meta_window_set_unmaximize_flags(window, META_MAXIMIZE_BOTH);
    if (!g_hash_table_contains(control->snap_restore_frames, window)) {
        MtkRectangle* original_frame = g_new(MtkRectangle, 1);
        *original_frame = context.original_frame;
        g_hash_table_insert(control->snap_restore_frames, window, original_frame);
    }
    meta_window_move_resize_frame(window, TRUE, frame.x, frame.y, frame.width, frame.height);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&result, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&result, "{sv}", "committed", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_commit_snap_context(MetaDisplay* display, GVariant* arguments,
                                                     GError** error) {
    return native_commit_snap_context_owned(display, arguments, 0, error);
}

static GVariant* native_socket_commit_snap_context(Client* client, JsonObject* json_arguments,
                                                   GError** error) {
    JsonNode* token_node =
        json_arguments ? json_object_get_member(json_arguments, "context") : NULL;
    if (!client || client->closing || !client->control || !token_node ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.snap requires a context token");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "SnapContext is invalid or revoked");
        return NULL;
    }

    JsonNode* monitor_node = json_object_get_member(json_arguments, "monitor_id");
    JsonNode* frame_node = json_object_get_member(json_arguments, "frame");
    const char* monitor_id = monitor_node && JSON_NODE_HOLDS_VALUE(monitor_node) &&
                                     json_node_get_value_type(monitor_node) == G_TYPE_STRING
                                 ? json_node_get_string(monitor_node)
                                 : NULL;
    const char* fields[] = {"context", "monitor_id", "frame"};
    g_autoptr(GVariant) frame = native_socket_snap_rect(frame_node);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "context", g_variant_new_string(token));
    if (native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) &&
        monitor_id && *monitor_id && frame) {
        g_variant_builder_add(&builder, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
        g_variant_builder_add(&builder, "{sv}", "frame", frame);
    }
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    return native_commit_snap_context_owned(client->control->display, arguments, client->client_id,
                                            error);
}

GVariant* gnoblin_native_control_create_text_target(MetaDisplay* display, guint64 context_handle,
                                                    guint64 generation, gint64 expires_at_us,
                                                    GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->wayland_compositor ||
        !control->wayland_compositor->seat || !control->focus_contexts ||
        !control->wayland_compositor->seat->text_input || !control->text_targets ||
        !context_handle) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "text insertion requires an active Wayland session");
        return NULL;
    }

    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text target requires an unused shortcut FocusContext");
        return NULL;
    }
    NativeFocusContext context = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (g_hash_table_size(control->text_targets) >= MAX_TEXT_TARGETS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "too many active text targets; retry after one expires");
        return NULL;
    }
    if (context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us != expires_at_us || context.expires_at_us <= g_get_monotonic_time() ||
        !focus_identity_matches(control, context.surface_id, context.client_id,
                                context.focus_epoch) ||
        !native_text_modifiers_allowed(control, context.allowed_modifier_mask) ||
        (context.modifiers & (CLUTTER_CONTROL_MASK | CLUTTER_MOD1_MASK | CLUTTER_SHIFT_MASK |
                              CLUTTER_LOCK_MASK | CLUTTER_META_MASK | CLUTTER_HYPER_MASK))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text target FocusContext is expired or no longer matches focus");
        return NULL;
    }
    if (meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "text insertion is unavailable while the session is locked");
        return NULL;
    }

    graphene_rect_t caret;
    guint64 surface_id = 0, client_id = 0, focus_epoch = 0;
    if (!meta_wayland_text_input_get_gnoblin_state(control->wayland_compositor->seat->text_input,
                                                   &surface_id, &client_id, &focus_epoch, &caret) ||
        surface_id != context.surface_id || client_id != context.client_id ||
        focus_epoch != context.focus_epoch) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "focused Wayland client has no active text input");
        return NULL;
    }

    char token[65] = {0};
    gboolean unique = FALSE;
    for (guint attempt = 0; attempt < 4; attempt++) {
        if (!focus_token_random(token))
            break;
        if (!g_hash_table_contains(control->text_targets, token)) {
            unique = TRUE;
            break;
        }
    }
    if (!unique) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not allocate a one-use text target");
        return NULL;
    }
    NativeTextTarget* target = g_new0(NativeTextTarget, 1);
    target->generation = generation;
    target->expires_at_us = context.expires_at_us;
    target->surface_id = surface_id;
    target->client_id = client_id;
    target->focus_epoch = focus_epoch;
    target->allowed_modifier_mask = context.allowed_modifier_mask;
    g_hash_table_insert(control->text_targets, g_strdup(token), target);

    GVariantBuilder result, caret_result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&caret_result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "target", g_variant_new_string(token));
    MetaWaylandSurface* focused_surface =
        meta_wayland_seat_get_input_focus(control->wayland_compositor->seat);
    MetaWindow* focused_window =
        focused_surface ? meta_wayland_surface_get_window(focused_surface) : NULL;
    if (focused_window) {
        g_autofree char* window_id = native_window_id(focused_window);
        g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    }
    g_variant_builder_add(&caret_result, "{sv}", "x", g_variant_new_double(caret.origin.x));
    g_variant_builder_add(&caret_result, "{sv}", "y", g_variant_new_double(caret.origin.y));
    g_variant_builder_add(&caret_result, "{sv}", "width", g_variant_new_double(caret.size.width));
    g_variant_builder_add(&caret_result, "{sv}", "height", g_variant_new_double(caret.size.height));
    g_variant_builder_add(&result, "{sv}", "caret", g_variant_builder_end(&caret_result));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* native_insert_text_owned(MetaDisplay* display, GVariant* arguments,
                                          guint64 socket_owner_client_id, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* token = NULL;
    const char* text = NULL;
    if (!control || !control->text_targets || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(arguments, "target", "&s", &token) || !token || !*token) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text requires a one-use TextTarget and text");
        return NULL;
    }
    NativeTextTarget* stored = g_hash_table_lookup(control->text_targets, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget was consumed, expired, or revoked");
        return NULL;
    }
    if (stored->socket_owner_client_id != socket_owner_client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget belongs to another connection");
        return NULL;
    }
    NativeTextTarget target = *stored;
    g_hash_table_remove(control->text_targets, token);
    if (g_variant_n_children(arguments) != 2 || !g_variant_lookup(arguments, "text", "&s", &text) ||
        !native_text_valid(text)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "inserted text must be 1-256 bytes of UTF-8 without control characters");
        return NULL;
    }
    if (control->stopping || target.generation != native_config_generation(control) ||
        target.expires_at_us <= g_get_monotonic_time() ||
        !native_text_modifiers_allowed(control, target.allowed_modifier_mask) ||
        !control->wayland_compositor || !control->wayland_compositor->seat ||
        !control->wayland_compositor->seat->text_input ||
        meta_wayland_session_lock_is_active(control->wayland_compositor) ||
        !focus_identity_matches(control, target.surface_id, target.client_id, target.focus_epoch) ||
        !meta_wayland_text_input_insert_gnoblin_text(control->wayland_compositor->seat->text_input,
                                                     target.surface_id, target.client_id,
                                                     target.focus_epoch, text)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget no longer matches the active Wayland text input");
        return NULL;
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "inserted", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_insert_text(MetaDisplay* display, GVariant* arguments,
                                             GError** error) {
    return native_insert_text_owned(display, arguments, 0, error);
}

GVariant* gnoblin_native_control_begin_window_grab(MetaDisplay* display, const char* method,
                                                   GVariant* arguments, guint64 context_handle,
                                                   guint64 generation, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->focus_contexts || !context_handle ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation has no active shortcut context");
        return NULL;
    }

    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive window operation context was already used or revoked");
        return NULL;
    }
    NativeFocusContext context = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time()) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operation context expired or belongs to an old runtime");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operations are unavailable while the session is locked");
        return NULL;
    }

    gboolean resize = g_str_equal(method, "window.begin_resize");
    const char* wanted_id = NULL;
    const char* edge = NULL;
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != (resize ? 2 : 1) ||
        !g_variant_lookup(arguments, "id", "&s", &wanted_id) || !*wanted_id ||
        (resize && !g_variant_lookup(arguments, "edge", "&s", &edge))) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "interactive window operation requires a stable window ID and valid arguments");
        return NULL;
    }

    MetaGrabOp op;
    if (!resize) {
        op = META_GRAB_OP_KEYBOARD_MOVING | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_N | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_S | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "east")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_E | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "west")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_W | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north_east")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_NE | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "north_west")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_NW | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south_east")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_SE | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else if (g_str_equal(edge, "south_west")) {
        op = META_GRAB_OP_KEYBOARD_RESIZING_SW | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.begin_resize edge is not a valid ResizeEdge");
        return NULL;
    }

    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (g_str_equal(id, wanted_id)) {
            target = window;
            break;
        }
    }
    if (!target) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window ID not found; list windows first");
        return NULL;
    }
    if ((resize && !meta_window_allows_resize(target)) ||
        (!resize && !meta_window_allows_move(target))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            resize ? "window cannot be resized" : "window cannot be moved");
        return NULL;
    }

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    ClutterBackend* clutter_backend = backend ? meta_backend_get_clutter_backend(backend) : NULL;
    ClutterActor* stage_actor = backend ? meta_backend_get_stage(backend) : NULL;
    if (!clutter_backend || !stage_actor || !CLUTTER_IS_STAGE(stage_actor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "interactive window operations need an active Mutter stage");
        return NULL;
    }
    ClutterSprite* sprite =
        clutter_backend_get_pointer_sprite(clutter_backend, CLUTTER_STAGE(stage_actor));
    if (!meta_window_begin_grab_op(target, op, sprite, context.timestamp, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Mutter could not start the interactive window operation");
        return NULL;
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(wanted_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_native_control_begin_menu_window_grab(MetaDisplay* display, const char* method,
                                                        GVariant* arguments, guint64 context_handle,
                                                        guint64 generation,
                                                        guint64 socket_owner_client_id,
                                                        GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->menu_contexts || !context_handle ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "interactive WM menu operation has no live authority");
        return NULL;
    }

    NativeMenuContext* stored = g_hash_table_lookup(control->menu_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu capability was already used or revoked");
        return NULL;
    }
    NativeMenuContext context = *stored;
    context.window_id = g_strdup(stored->window_id);
    g_hash_table_remove(control->menu_contexts, &context_handle);
    if (context.socket_owner_client_id != socket_owner_client_id ||
        context.generation != generation ||
        context.generation != native_config_generation(control) ||
        context.expires_at_us <= g_get_monotonic_time()) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu capability expired or belongs to another owner/runtime");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_free(context.window_id);
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "interactive window operations are unavailable while the session is locked");
        return NULL;
    }

    gboolean resize = g_str_equal(method, "window.begin_resize");
    const char* edge = NULL;
    gboolean exact = arguments && g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) &&
                     g_variant_n_children(arguments) == (resize ? 1 : 0) &&
                     (!resize || g_variant_lookup(arguments, "edge", "&s", &edge));
    if (!exact) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "WM menu operation arguments are malformed");
        return NULL;
    }

    MetaGrabOp op = META_GRAB_OP_KEYBOARD_MOVING | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
    if (resize) {
        static const struct {
            const char* name;
            MetaGrabOp op;
        } edges[] = {
            {"north", META_GRAB_OP_KEYBOARD_RESIZING_N},
            {"south", META_GRAB_OP_KEYBOARD_RESIZING_S},
            {"east", META_GRAB_OP_KEYBOARD_RESIZING_E},
            {"west", META_GRAB_OP_KEYBOARD_RESIZING_W},
            {"north_east", META_GRAB_OP_KEYBOARD_RESIZING_NE},
            {"north_west", META_GRAB_OP_KEYBOARD_RESIZING_NW},
            {"south_east", META_GRAB_OP_KEYBOARD_RESIZING_SE},
            {"south_west", META_GRAB_OP_KEYBOARD_RESIZING_SW},
        };
        gboolean matched = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(edges); i++) {
            if (g_str_equal(edge, edges[i].name)) {
                op = edges[i].op | META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED;
                matched = TRUE;
                break;
            }
        }
        if (!matched) {
            g_free(context.window_id);
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.begin_resize edge is not a valid ResizeEdge");
            return NULL;
        }
    }

    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (g_str_equal(id, context.window_id)) {
            target = window;
            break;
        }
    }
    if (!target) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "WM menu target is no longer a live window");
        return NULL;
    }
    if ((resize && !meta_window_allows_resize(target)) ||
        (!resize && !meta_window_allows_move(target))) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            resize ? "WM menu target cannot be resized"
                                   : "WM menu target cannot be moved");
        return NULL;
    }

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    ClutterBackend* clutter_backend = backend ? meta_backend_get_clutter_backend(backend) : NULL;
    ClutterActor* stage_actor = backend ? meta_backend_get_stage(backend) : NULL;
    if (!clutter_backend || !stage_actor || !CLUTTER_IS_STAGE(stage_actor)) {
        g_free(context.window_id);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "interactive window operations need an active Mutter stage");
        return NULL;
    }
    ClutterSprite* sprite =
        clutter_backend_get_pointer_sprite(clutter_backend, CLUTTER_STAGE(stage_actor));
    guint32 timestamp = meta_display_get_current_time_roundtrip(display);
    gboolean began = meta_window_begin_grab_op(target, op, sprite, timestamp, NULL);
    g_autofree char* result_id = g_strdup(context.window_id);
    g_free(context.window_id);
    if (!began) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Mutter could not start the WM menu window operation");
        return NULL;
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(result_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

void gnoblin_native_control_revoke_focus_contexts(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    revoke_focus_contexts(control);
}

static JsonNode* native_launch_json(NativeLaunch* launch) {
    JsonObject* object = json_object_new();
    json_object_set_string_member(object, "token", launch->token);
    json_object_set_string_member(object, "application", launch->application);
    json_object_set_int_member(object, "started_at", launch->started_at);
    json_object_set_int_member(object, "timeout_ms", launch->timeout_ms);
    json_object_set_string_member(object, "state", launch->state);
    json_object_set_int_member(object, "revision", launch->revision);
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, object);
    return node;
}

static gboolean native_launch_matches(NativeLaunch* launch, MetaWindow* window) {
    if (meta_window_is_skip_taskbar(window))
        return FALSE;
    const char* values[] = {
        meta_window_get_gtk_application_id(window),
        meta_window_get_wm_class(window),
        meta_window_get_wm_class_instance(window),
    };
    for (guint i = 0; i < G_N_ELEMENTS(values); i++) {
        if (!values[i] || !*values[i])
            continue;
        g_autofree char* normalized = normalize_launch_application(values[i]);
        if (g_str_equal(normalized, launch->normalized_application))
            return TRUE;
    }
    return FALSE;
}

static gboolean native_launch_window_is_mapped(MetaWindow* window) {
    GObject* actor = meta_window_get_compositor_private(window);
    return actor && CLUTTER_IS_ACTOR(actor) && clutter_actor_is_mapped(CLUTTER_ACTOR(actor));
}

static gboolean native_launch_has_pending(GnoblinNativeControl* control) {
    for (guint i = 0; control->launches && i < control->launches->len; i++) {
        NativeLaunch* launch = g_ptr_array_index(control->launches, i);
        if (g_str_equal(launch->state, "pending"))
            return TRUE;
    }
    return FALSE;
}

static void native_launch_update_cursor(GnoblinNativeControl* control) {
    MetaCursorTracker* tracker =
        control->backend ? meta_backend_get_cursor_tracker(control->backend) : NULL;
    if (tracker)
        meta_cursor_tracker_set_gnoblin_launch_cursor(tracker, native_launch_has_pending(control));
}

static void native_launch_changed(GnoblinNativeControl* control, NativeLaunch* launch,
                                  const char* state) {
    g_free(launch->state);
    launch->state = g_strdup(state);
    launch->revision = ++control->launch_revision;
    update_launch_snapshot(control);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.launch.changed");
    json_object_set_int_member(object, "revision", launch->revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_member(object, "launch", native_launch_json(launch));
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, "gnoblin.launch.changed", payload);
    native_launch_update_cursor(control);
}

static gboolean native_launch_observed(NativeLaunch* launch, MetaDisplay* display) {
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    MetaWindow* focused = meta_display_get_focus_window(display);
    g_autofree char* focused_id = focused ? native_window_id(focused) : NULL;
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (!native_launch_window_is_mapped(window) || !native_launch_matches(launch, window))
            continue;
        g_autofree char* id = native_window_id(window);
        if (!g_hash_table_contains(launch->initial_window_ids, id) ||
            (g_strcmp0(focused_id, id) == 0 && g_strcmp0(launch->initial_focus_id, id) != 0))
            return TRUE;
    }
    return FALSE;
}

static gboolean native_launch_tick(gpointer data) {
    GnoblinNativeControl* control = data;
    if (control->stopping || !control->launches) {
        control->launch_tick_id = 0;
        return G_SOURCE_REMOVE;
    }
    gint64 now = g_get_monotonic_time();
    for (guint i = 0; i < control->launches->len; i++) {
        NativeLaunch* launch = g_ptr_array_index(control->launches, i);
        if (!g_str_equal(launch->state, "pending"))
            continue;
        if (now >= launch->deadline_us)
            native_launch_changed(control, launch, "timed_out");
        else if (native_launch_observed(launch, control->display))
            native_launch_changed(control, launch, "started");
    }
    if (!native_launch_has_pending(control)) {
        control->launch_tick_id = 0;
        native_launch_update_cursor(control);
        return G_SOURCE_REMOVE;
    }
    return G_SOURCE_CONTINUE;
}

static NativeLaunch* native_launch_find(GnoblinNativeControl* control, const char* token,
                                        guint* index) {
    for (guint i = 0; control->launches && i < control->launches->len; i++) {
        NativeLaunch* launch = g_ptr_array_index(control->launches, i);
        if (g_str_equal(launch->token, token)) {
            if (index)
                *index = i;
            return launch;
        }
    }
    return NULL;
}

static JsonNode* native_launch_snapshot(GnoblinNativeControl* control) {
    JsonObject* object = json_object_new();
    JsonArray* launches = json_array_new();
    for (guint i = 0; control->launches && i < control->launches->len; i++)
        json_array_add_element(launches,
                               native_launch_json(g_ptr_array_index(control->launches, i)));
    json_object_set_array_member(object, "launches", launches);
    json_object_set_int_member(object, "revision", control->launch_revision);
    JsonNode* result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, object);
    return result;
}

static void update_launch_snapshot(GnoblinNativeControl* control) {
    if (!control)
        return;
    g_autoptr(JsonNode) snapshot = native_launch_snapshot(control);
    g_autoptr(GVariant) value = variant_from_json(snapshot);
    if (value)
        native_publish_runtime_snapshot(control, "launches", value, control->launch_revision);
}

static gboolean native_launch_parse_begin(JsonObject* arguments, const char** token,
                                          const char** application, guint* timeout_ms,
                                          GError** error) {
    JsonNode* token_node = json_object_get_member(arguments, "token");
    JsonNode* application_node = json_object_get_member(arguments, "application");
    JsonNode* timeout_node = json_object_get_member(arguments, "milliseconds");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING || !application_node ||
        !JSON_NODE_HOLDS_VALUE(application_node) ||
        json_node_get_value_type(application_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "launch.begin requires string token and application fields");
        return FALSE;
    }
    *token = json_node_get_string(token_node);
    *application = json_node_get_string(application_node);
    glong token_length = g_utf8_strlen(*token, -1);
    glong application_length = g_utf8_strlen(*application, -1);
    g_autofree char* normalized_application = normalize_launch_application(*application);
    if (token_length <= 0 || token_length > 128 || application_length > 512 ||
        !*normalized_application) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "launch token must be 1–128 characters and application must be "
                            "non-empty and at most 512");
        return FALSE;
    }
    *timeout_ms = 3000;
    if (timeout_node) {
        if (!JSON_NODE_HOLDS_VALUE(timeout_node) ||
            (json_node_get_value_type(timeout_node) != G_TYPE_INT &&
             json_node_get_value_type(timeout_node) != G_TYPE_INT64)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "launch milliseconds must be an integer");
            return FALSE;
        }
        gint64 value = json_node_get_int(timeout_node);
        *timeout_ms = (guint)CLAMP(value, 100, 10000);
    }
    GList* members = json_object_get_members(arguments);
    for (GList* item = members; item; item = item->next) {
        const char* member = item->data;
        if (!g_str_equal(member, "token") && !g_str_equal(member, "application") &&
            !g_str_equal(member, "milliseconds")) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "launch.begin does not accept '%s'", member);
            g_list_free(members);
            return FALSE;
        }
    }
    g_list_free(members);
    return TRUE;
}

static JsonNode* native_launch_begin(GnoblinNativeControl* control, JsonObject* arguments,
                                     GError** error) {
    const char* token;
    const char* application;
    guint timeout_ms;
    if (!native_launch_parse_begin(arguments, &token, &application, &timeout_ms, error))
        return NULL;
    if (!control->backend || !meta_backend_get_cursor_tracker(control->backend)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "Mutter launch cursor support is unavailable");
        return NULL;
    }
    NativeLaunch* launch = native_launch_find(control, token, NULL);
    if (launch) {
        guint index = 0;
        native_launch_find(control, token, &index);
        g_ptr_array_remove_index(control->launches, index);
    }
    if (control->launches->len >= MAX_LAUNCHES) {
        guint remove_index = MAX_LAUNCHES;
        for (guint i = 0; i < control->launches->len; i++) {
            NativeLaunch* previous = g_ptr_array_index(control->launches, i);
            if (!g_str_equal(previous->state, "pending")) {
                remove_index = i;
                break;
            }
        }
        if (remove_index == MAX_LAUNCHES) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "too many pending launches");
            return NULL;
        }
        g_ptr_array_remove_index(control->launches, remove_index);
    }
    launch = g_new0(NativeLaunch, 1);
    launch->token = g_strdup(token);
    launch->application = g_strdup(application);
    launch->normalized_application = normalize_launch_application(application);
    launch->state = g_strdup("pending");
    launch->started_at = g_get_real_time() / 1000;
    launch->timeout_ms = timeout_ms;
    launch->deadline_us = g_get_monotonic_time() + (gint64)timeout_ms * 1000;
    launch->initial_window_ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GList) windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next)
        g_hash_table_add(launch->initial_window_ids, native_window_id(item->data));
    MetaWindow* focused = meta_display_get_focus_window(control->display);
    launch->initial_focus_id = focused ? native_window_id(focused) : NULL;
    g_ptr_array_add(control->launches, launch);
    native_launch_changed(control, launch, "pending");
    if (!control->launch_tick_id)
        control->launch_tick_id = g_timeout_add(100, native_launch_tick, control);
    return native_launch_json(launch);
}

static gboolean native_launch_end(GnoblinNativeControl* control, JsonObject* arguments,
                                  const char** token, GError** error) {
    JsonNode* token_node = json_object_get_member(arguments, "token");
    const char* value = token_node && JSON_NODE_HOLDS_VALUE(token_node) &&
                                json_node_get_value_type(token_node) == G_TYPE_STRING
                            ? json_node_get_string(token_node)
                            : NULL;
    glong token_length = value ? g_utf8_strlen(value, -1) : 0;
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING || !value || token_length <= 0 ||
        token_length > 128 || json_object_get_size(arguments) != 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "launch.end requires only a non-empty token of at most 128 characters");
        return FALSE;
    }
    *token = value;
    NativeLaunch* launch = native_launch_find(control, *token, NULL);
    if (launch && g_str_equal(launch->state, "pending"))
        native_launch_changed(control, launch, "ended");
    return TRUE;
}

GVariant* gnoblin_native_control_dispatch_launch(MetaDisplay* display, const char* method,
                                                 GVariant* arguments, GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || !method || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "native launch feedback is unavailable");
        return NULL;
    }
    g_autoptr(JsonNode) json_arguments = json_from_variant(arguments);
    if (!JSON_NODE_HOLDS_OBJECT(json_arguments)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "launch arguments must be an object");
        return NULL;
    }
    JsonObject* object = json_node_get_object(json_arguments);
    if (g_str_equal(method, "launch.status")) {
        if (json_object_get_size(object) != 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "launch.status does not accept arguments");
            return NULL;
        }
        g_autoptr(JsonNode) snapshot = native_launch_snapshot(control);
        return variant_from_json(snapshot);
    }
    if (g_str_equal(method, "launch.begin")) {
        g_autoptr(JsonNode) launch = native_launch_begin(control, object, error);
        return launch ? variant_from_json(launch) : NULL;
    }
    if (g_str_equal(method, "launch.end")) {
        const char* token = NULL;
        if (!native_launch_end(control, object, &token, error))
            return NULL;
        JsonObject* response = json_object_new();
        json_object_set_boolean_member(response, "ok", TRUE);
        json_object_set_string_member(response, "token", token);
        g_autoptr(JsonNode) json_response = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(json_response, response);
        return variant_from_json(json_response);
    }
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                "unsupported native launch method '%s'", method);
    return NULL;
}

static void dispatch_lua_input_device_event(GnoblinNativeControl* control, guint64 revision,
                                            const char* event, const char* id, JsonNode* device,
                                            JsonNode* last) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.input.device-removed")) {
        json_object_set_string_member(object, "device_id", id);
        json_object_set_member(object, "last", json_node_copy(last));
    } else {
        json_object_set_member(object, "device", json_node_copy(device));
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, event, payload);
}

static void dispatch_lua_input_sources_changed(GnoblinNativeControl* control, guint64 revision) {
    g_autoptr(GVariant) snapshot = input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", snapshot, revision);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.input.sources-changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    g_autoptr(JsonNode) json = json_from_variant(snapshot);
    json_object_set_member(
        object, "sources",
        json_node_copy(json_object_get_member(json_node_get_object(json), "sources")));
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, "gnoblin.input.sources-changed", payload);
}

static void dispatch_lua_input_source_changed(GnoblinNativeControl* control, guint64 revision,
                                              NativeInputSource* source) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.input.source-changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_boolean_member(object, "available", source != NULL);
    if (source) {
        g_autoptr(GVariant) record = input_source_record_variant(source, revision, TRUE);
        json_object_set_member(object, "source", json_from_variant(record));
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, "gnoblin.input.source-changed", payload);
}

static void publish_input_source_changes(GnoblinNativeControl* control, guint64 revision) {
    gboolean sources_changed = refresh_input_sources(control, NULL);
    g_autoptr(GVariant) snapshot = input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", snapshot, revision);
    if (sources_changed && control->input_source_state_initialized)
        dispatch_lua_input_sources_changed(control, revision);
    NativeInputSource* current = current_input_source(control);
    g_autofree char* current_key =
        current ? g_strdup_printf("%s:%s", current->type, current->id) : NULL;
    if (control->pending_input_source_ops == 0) {
        if (control->input_source_state_initialized &&
            g_strcmp0(control->last_published_input_source, current_key) != 0)
            dispatch_lua_input_source_changed(control, revision, current);
        g_free(control->last_published_input_source);
        control->last_published_input_source = g_steal_pointer(&current_key);
    }
    control->input_source_state_initialized = TRUE;
}

static void dispatch_lua_workspace_event(GnoblinNativeControl* control, guint64 revision,
                                         const char* event, const char* id, JsonNode* workspace,
                                         JsonNode* last, const char* previous_id) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.workspace.removed")) {
        json_object_set_string_member(object, "workspace_id", id);
        json_object_set_member(object, "last", lua_workspace_record(last, revision));
    } else {
        json_object_set_member(object, "workspace", lua_workspace_record(workspace, revision));
        if (g_str_equal(event, "gnoblin.workspace.activated") && previous_id)
            json_object_set_string_member(object, "previous_id", previous_id);
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, event, payload);
}

static void dispatch_lua_workspace_changed_event(GnoblinNativeControl* control, guint64 revision,
                                                 JsonNode* workspace, const char* const* changed,
                                                 guint changed_count) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.workspace.changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_member(object, "workspace", lua_workspace_record(workspace, revision));
    JsonArray* changed_array = json_array_new();
    for (guint i = 0; i < changed_count; i++)
        json_array_add_string_element(changed_array, changed[i]);
    json_object_set_array_member(object, "changed", changed_array);
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, "gnoblin.workspace.changed", payload);
}

static void dispatch_lua_window_moved_event(GnoblinNativeControl* control, guint64 revision,
                                            const char* id, const char* from_id,
                                            const char* to_id) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.workspace.window-moved");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_string_member(object, "window_id", id);
    json_object_set_string_member(object, "from_id", from_id);
    json_object_set_string_member(object, "to_id", to_id);
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, "gnoblin.workspace.window-moved", payload);
}

static gboolean workspace_active(JsonNode* node) {
    JsonNode* active = json_object_get_member(json_node_get_object(node), "active");
    return active && JSON_NODE_HOLDS_VALUE(active) && json_node_get_boolean(active);
}

static const char* workspace_id(JsonNode* node) {
    return json_object_get_string_member_with_default(json_node_get_object(node), "id", NULL);
}

static gboolean workspace_int_property_changed(JsonObject* previous, JsonObject* current,
                                               const char* property) {
    JsonNode* before = json_object_get_member(previous, property);
    JsonNode* after = json_object_get_member(current, property);
    if (!before || !after)
        return before != after;
    return json_node_get_int(before) != json_node_get_int(after);
}

static gboolean workspace_boolean_property_changed(JsonObject* previous, JsonObject* current,
                                                   const char* property) {
    JsonNode* before = json_object_get_member(previous, property);
    JsonNode* after = json_object_get_member(current, property);
    if (!before || !after)
        return before != after;
    return json_node_get_boolean(before) != json_node_get_boolean(after);
}

static GHashTable* workspace_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_workspace_state_free);
    if (!JSON_NODE_HOLDS_OBJECT(snapshot))
        return state;
    JsonArray* workspaces =
        json_object_get_array_member(json_node_get_object(snapshot), "workspaces");
    for (guint i = 0; workspaces && i < json_array_get_length(workspaces); i++) {
        JsonNode* record = json_array_get_element(workspaces, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        const char* id = workspace_id(record);
        if (!id || !*id)
            continue;
        NativeWorkspaceState* value = g_new0(NativeWorkspaceState, 1);
        value->json = json_to_string(record, FALSE);
        g_hash_table_insert(state, g_strdup(id), value);
    }
    return state;
}

static void publish_workspace_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                      JsonNode* window_snapshot, guint64 revision) {
    GHashTable* current = workspace_state_from_snapshot(snapshot);
    gboolean dispatch_events = control->workspace_state_initialized;
    const char* previous_active_id = NULL;
    g_autofree char* previous_active_storage = NULL;

    if (dispatch_events) {
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, control->workspace_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWorkspaceState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL) &&
                workspace_active(json_parser_get_root(parser))) {
                previous_active_storage = g_strdup(key);
                previous_active_id = previous_active_storage;
                break;
            }
        }

        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWorkspaceState* state = value;
            NativeWorkspaceState* previous = g_hash_table_lookup(control->workspace_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* record = json_parser_get_root(parser);
            if (!previous) {
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.created", key,
                                             record, NULL, NULL);
            } else {
                g_autoptr(JsonParser) previous_parser = json_parser_new();
                if (json_parser_load_from_data(previous_parser, previous->json, -1, NULL)) {
                    JsonObject* previous_record =
                        json_node_get_object(json_parser_get_root(previous_parser));
                    JsonObject* current_record = json_node_get_object(record);
                    if (g_strcmp0(
                            json_object_get_string_member_with_default(previous_record, "name", ""),
                            json_object_get_string_member_with_default(current_record, "name",
                                                                       "")) != 0)
                        dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.renamed",
                                                     key, record, NULL, NULL);

                    const char* changed[3];
                    guint changed_count = 0;
                    if (workspace_int_property_changed(previous_record, current_record, "number"))
                        changed[changed_count++] = "number";
                    if (workspace_int_property_changed(previous_record, current_record, "windows"))
                        changed[changed_count++] = "window_count";
                    if (workspace_boolean_property_changed(previous_record, current_record,
                                                           "persistent"))
                        changed[changed_count++] = "persistent";
                    if (changed_count)
                        dispatch_lua_workspace_changed_event(control, revision, record, changed,
                                                             changed_count);
                }
            }
            if (workspace_active(record) &&
                (!previous_active_id || !g_str_equal(previous_active_id, key)))
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.activated", key,
                                             record, NULL, previous_active_id);
        }

        g_hash_table_iter_init(&iter, control->workspace_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            NativeWorkspaceState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL))
                dispatch_lua_workspace_event(control, revision, "gnoblin.workspace.removed", key,
                                             NULL, json_parser_get_root(parser), NULL);
        }

        JsonArray* windows =
            JSON_NODE_HOLDS_OBJECT(window_snapshot)
                ? json_object_get_array_member(json_node_get_object(window_snapshot), "windows")
                : NULL;
        for (guint i = 0; windows && i < json_array_get_length(windows); i++) {
            JsonNode* window = json_array_get_element(windows, i);
            if (!JSON_NODE_HOLDS_OBJECT(window))
                continue;
            JsonObject* record = json_node_get_object(window);
            const char* id = json_object_get_string_member_with_default(record, "id", NULL);
            const char* to_id =
                json_object_get_string_member_with_default(record, "workspaceId", NULL);
            NativeWindowState* previous =
                id ? g_hash_table_lookup(control->window_state, id) : NULL;
            if (!previous || !to_id || !*to_id)
                continue;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, previous->json, -1, NULL))
                continue;
            const char* from_id = json_object_get_string_member_with_default(
                json_node_get_object(json_parser_get_root(parser)), "workspaceId", NULL);
            if (from_id && *from_id && !g_str_equal(from_id, to_id))
                dispatch_lua_window_moved_event(control, revision, id, from_id, to_id);
        }
    }

    if (control->workspace_state)
        g_hash_table_unref(control->workspace_state);
    control->workspace_state = current;
    control->workspace_state_initialized = TRUE;
}

static JsonNode* workspace_snapshot_json(GnoblinNativeControl* control, GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "workspace.list", arguments, error);
    if (!result) {
        native_publish_runtime_snapshot(control, "workspaces", NULL, control->state_revision);
        return NULL;
    }
    cache_lua_workspace_snapshot(control, result, control->state_revision);
    g_autoptr(JsonNode) json = json_from_variant(result);
    return g_steal_pointer(&json);
}

static JsonNode* monitor_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                                       GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "monitor.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "monitors", NULL, control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "monitors", NULL, control->state_revision);
        return NULL;
    }
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(json), "monitors");
    if (!monitors) {
        if (update_lua_snapshot)
            native_publish_runtime_snapshot(control, "monitors", NULL, control->state_revision);
        return NULL;
    }
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (JSON_NODE_HOLDS_OBJECT(record))
            json_object_set_int_member(json_node_get_object(record), "revision",
                                       control->state_revision);
    }
    json_object_set_int_member(json_node_get_object(json), "revision", control->state_revision);
    if (update_lua_snapshot) {
        g_autoptr(GVariant) snapshot = variant_from_json(json);
        native_publish_runtime_snapshot(control, "monitors", snapshot, control->state_revision);
    }
    return g_steal_pointer(&json);
}

static char* monitor_snapshot(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) json = monitor_snapshot_json(control, FALSE, &error);
    if (!json)
        return encode_response("", NULL, error ? error->message : "monitor listing unavailable");
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", "monitors");
    g_autofree char* encoded = json_to_string(json, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static GHashTable* monitor_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_monitor_state_free);
    if (!JSON_NODE_HOLDS_OBJECT(snapshot))
        return state;
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(snapshot), "monitors");
    for (guint i = 0; monitors && i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        const char* id =
            json_object_get_string_member_with_default(json_node_get_object(record), "id", NULL);
        if (!id || !*id)
            continue;
        NativeMonitorState* value = g_new0(NativeMonitorState, 1);
        value->json = json_to_string(record, FALSE);
        g_hash_table_insert(state, g_strdup(id), value);
    }
    return state;
}

static char* native_monitor_id_for_index(JsonNode* snapshot, int monitor_index) {
    if (monitor_index < 0 || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return NULL;
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(snapshot), "monitors");
    const char* matched_id = NULL;
    for (guint i = 0; monitors && i < json_array_get_length(monitors); i++) {
        JsonNode* record = json_array_get_element(monitors, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        JsonObject* monitor = json_node_get_object(record);
        if (json_object_get_int_member_with_default(monitor, "index", -1) != monitor_index)
            continue;
        const char* id = json_object_get_string_member_with_default(monitor, "id", NULL);
        if (!id || !*id || matched_id)
            return NULL;
        matched_id = id;
    }
    return g_strdup(matched_id);
}

static gint native_output_name_compare(gconstpointer left, gconstpointer right) {
    return g_strcmp0(*(const char* const*)left, *(const char* const*)right);
}

static GVariant* native_monitor_output_names_for_index(GnoblinNativeControl* control,
                                                       int monitor_index) {
    if (!control->monitor_manager || monitor_index < 0)
        return NULL;

    MetaLogicalMonitor* logical_monitor = NULL;
    GList* logical_monitors = meta_monitor_manager_get_logical_monitors(control->monitor_manager);
    for (GList* item = logical_monitors; item; item = item->next) {
        MetaLogicalMonitor* candidate = item->data;
        if (meta_logical_monitor_get_number(candidate) == monitor_index) {
            logical_monitor = candidate;
            break;
        }
    }
    if (!logical_monitor)
        return NULL;

    GPtrArray* names = g_ptr_array_new_with_free_func(g_free);
    GList* monitors = meta_logical_monitor_get_monitors(logical_monitor);
    for (GList* monitor_item = monitors; monitor_item; monitor_item = monitor_item->next) {
        MetaMonitor* monitor = monitor_item->data;
        if (!meta_monitor_is_active(monitor))
            continue;
        GList* outputs = meta_monitor_get_outputs(monitor);
        for (GList* output_item = outputs; output_item; output_item = output_item->next) {
            const char* name = meta_output_get_name(output_item->data);
            if (name && *name)
                g_ptr_array_add(names, g_strdup(name));
        }
    }
    if (names->len == 0) {
        g_ptr_array_unref(names);
        return NULL;
    }

    g_ptr_array_sort(names, native_output_name_compare);
    GVariantBuilder output_names;
    g_variant_builder_init(&output_names, G_VARIANT_TYPE_STRING_ARRAY);
    const char* previous = NULL;
    for (guint i = 0; i < names->len; i++) {
        const char* name = g_ptr_array_index(names, i);
        if (!previous || !g_str_equal(previous, name))
            g_variant_builder_add(&output_names, "s", name);
        previous = name;
    }
    g_ptr_array_unref(names);
    return g_variant_ref_sink(g_variant_builder_end(&output_names));
}

static void native_show_osd_requested(MetaDisplay* display, gint monitor_index,
                                      const gchar* icon_name, const gchar* message,
                                      gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || control->display != display)
        return;
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) monitor_snapshot = monitor_snapshot_json(control, FALSE, &snapshot_error);
    g_autofree char* monitor_id = native_monitor_id_for_index(monitor_snapshot, monitor_index);
    g_autoptr(GVariant) output_names =
        native_monitor_output_names_for_index(control, monitor_index);
    if (!monitor_id || !output_names) {
        g_debug("gnoblin-native-control: ignoring OSD request for stale or unmapped monitor index "
                "%d%s%s",
                monitor_index, snapshot_error ? ": " : "",
                snapshot_error ? snapshot_error->message : "");
        return;
    }
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&fields, "{sv}", "output_names", output_names);
    if (icon_name)
        g_variant_builder_add(&fields, "{sv}", "icon", g_variant_new_string(icon_name));
    if (message)
        g_variant_builder_add(&fields, "{sv}", "label", g_variant_new_string(message));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.osd.requested", payload);
}

static gboolean monitor_property_changed(JsonObject* previous, JsonObject* current,
                                         const char* property) {
    JsonNode* old_value = json_object_get_member(previous, property);
    JsonNode* new_value = json_object_get_member(current, property);
    if (!old_value || !new_value)
        return old_value != new_value;
    g_autofree char* old_json = json_to_string(old_value, FALSE);
    g_autofree char* new_json = json_to_string(new_value, FALSE);
    return !g_str_equal(old_json, new_json);
}

static void dispatch_lua_monitor_event(GnoblinNativeControl* control, guint64 revision,
                                       const char* event, const char* id, JsonNode* monitor,
                                       JsonNode* last, JsonArray* changed) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.monitor.removed")) {
        json_object_set_string_member(object, "monitor_id", id);
        json_object_set_member(object, "last", json_node_copy(last));
    } else {
        json_object_set_member(object, "monitor", json_node_copy(monitor));
        if (changed)
            json_object_set_array_member(object, "changed", json_array_ref(changed));
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, event, payload);
}

static gboolean publish_monitor_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                        guint64 revision) {
    GHashTable* current = monitor_state_from_snapshot(snapshot);
    gboolean changed_state = !control->monitor_state_initialized;
    if (control->monitor_state_initialized) {
        static const char* properties[] = {
            "id",      "index", "x",    "y",     "width",  "height",       "primary",   "scale",
            "enabled", "name",  "make", "model", "serial", "refresh_rate", "transform",
        };
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeMonitorState* state = value;
            NativeMonitorState* previous = g_hash_table_lookup(control->monitor_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* record = json_parser_get_root(parser);
            if (!previous) {
                changed_state = TRUE;
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.added", key, record,
                                           NULL, NULL);
                continue;
            }
            g_autoptr(JsonParser) previous_parser = json_parser_new();
            if (!json_parser_load_from_data(previous_parser, previous->json, -1, NULL))
                continue;
            JsonObject* old_record = json_node_get_object(json_parser_get_root(previous_parser));
            JsonObject* new_record = json_node_get_object(record);
            JsonArray* changed = json_array_new();
            for (guint i = 0; i < G_N_ELEMENTS(properties); i++) {
                if (monitor_property_changed(old_record, new_record, properties[i]))
                    json_array_add_string_element(changed, properties[i]);
            }
            if (json_array_get_length(changed)) {
                changed_state = TRUE;
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.changed", key,
                                           record, NULL, changed);
            }
            json_array_unref(changed);
        }

        g_hash_table_iter_init(&iter, control->monitor_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            changed_state = TRUE;
            NativeMonitorState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL))
                dispatch_lua_monitor_event(control, revision, "gnoblin.monitor.removed", key, NULL,
                                           json_parser_get_root(parser), NULL);
        }
    }
    if (control->monitor_state)
        g_hash_table_unref(control->monitor_state);
    control->monitor_state = current;
    control->monitor_state_initialized = TRUE;
    return changed_state;
}

static void publish_input_device_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                         guint64 revision) {
    if (!snapshot || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return;

    GHashTable* current = input_device_state_from_snapshot(snapshot);
    if (!control->input_device_state_initialized) {
        if (control->input_device_state)
            g_hash_table_unref(control->input_device_state);
        control->input_device_state = current;
        control->input_device_state_initialized = TRUE;
        return;
    }

    GHashTable* previous = control->input_device_state;
    control->input_device_state = current;
    GHashTableIter iter;
    gpointer key;
    gpointer value;
    g_hash_table_iter_init(&iter, current);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        if (g_hash_table_contains(previous, key))
            continue;
        g_autoptr(JsonParser) parser = json_parser_new();
        if (json_parser_load_from_data(parser, value, -1, NULL))
            dispatch_lua_input_device_event(control, revision, "gnoblin.input.device-added", key,
                                            json_parser_get_root(parser), NULL);
    }

    g_hash_table_iter_init(&iter, previous);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        if (g_hash_table_contains(current, key))
            continue;
        g_autoptr(JsonParser) parser = json_parser_new();
        if (json_parser_load_from_data(parser, value, -1, NULL))
            dispatch_lua_input_device_event(control, revision, "gnoblin.input.device-removed", key,
                                            NULL, json_parser_get_root(parser));
    }
    g_hash_table_unref(previous);
}

static gboolean window_property_equal(JsonObject* first, JsonObject* second, const char* key) {
    JsonNode* a = json_object_get_member(first, key);
    JsonNode* b = json_object_get_member(second, key);
    if (!a || !b)
        return a == b;
    g_autofree char* a_json = json_to_string(a, FALSE);
    g_autofree char* b_json = json_to_string(b, FALSE);
    return g_str_equal(a_json, b_json);
}

static JsonArray* changed_window_properties(JsonNode* previous, JsonNode* current) {
    JsonArray* changed = json_array_new();
    JsonObject* old_record = json_node_get_object(previous);
    JsonObject* new_record = json_node_get_object(current);
    for (guint i = 0; i < G_N_ELEMENTS(window_property_names); i++) {
        const char* native_name = window_property_names[i].native_name;
        if (g_str_equal(native_name, "demandsAttention"))
            continue;
        if (!window_property_equal(old_record, new_record, native_name))
            json_array_add_string_element(changed, window_property_names[i].lua_name);
    }
    if (window_record_is_modal(old_record) != window_record_is_modal(new_record))
        json_array_add_string_element(changed, "modal");
    return changed;
}

static void dispatch_lua_window_event(GnoblinNativeControl* control, guint64 revision,
                                      const char* event, const char* id, JsonNode* window,
                                      JsonNode* last, JsonArray* changed) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.window.created")) {
        json_object_set_member(object, "window", lua_window_record(window, revision));
    } else if (g_str_equal(event, "gnoblin.window.closed")) {
        json_object_set_string_member(object, "window_id", id);
        json_object_set_member(object, "last", lua_window_record(last, revision));
    } else {
        json_object_set_string_member(object, "window_id", id);
        JsonNode* record = lua_window_record(window, revision);
        json_object_set_member(object, "window", record);
        if (g_str_equal(event, "gnoblin.window.attention-changed")) {
            JsonNode* attention =
                json_object_get_member(json_node_get_object(record), "demands_attention");
            if (attention)
                json_object_set_member(object, "demands_attention", json_node_copy(attention));
        }
        if (g_str_equal(event, "gnoblin.window.changed"))
            json_object_set_array_member(object, "changed", json_array_ref(changed));
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, event, payload);
}

static void publish_window_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                   guint64 revision) {
    JsonArray* windows = json_object_get_array_member(json_node_get_object(snapshot), "windows");
    GHashTable* current =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free);
    for (guint i = 0; windows && i < json_array_get_length(windows); i++) {
        JsonNode* node = json_array_get_element(windows, i);
        if (!JSON_NODE_HOLDS_OBJECT(node))
            continue;
        JsonObject* record = json_node_get_object(node);
        const char* id = json_object_get_string_member_with_default(record, "id", NULL);
        if (!id || !*id)
            continue;
        NativeWindowState* state = g_new0(NativeWindowState, 1);
        state->json = json_to_string(node, FALSE);
        g_autoptr(JsonNode) comparable = json_node_copy(node);
        json_object_remove_member(json_node_get_object(comparable), "focused");
        state->comparable_json = json_to_string(comparable, FALSE);
        JsonNode* focused = json_object_get_member(record, "focused");
        state->focused =
            focused && JSON_NODE_HOLDS_VALUE(focused) && json_node_get_boolean(focused);
        g_hash_table_insert(current, g_strdup(id), state);
    }

    gboolean dispatch_events = control->window_state_initialized;
    if (dispatch_events) {
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWindowState* state = value;
            NativeWindowState* previous = g_hash_table_lookup(control->window_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* current_record = json_parser_get_root(parser);
            if (!previous) {
                dispatch_lua_window_event(control, revision, "gnoblin.window.created", key,
                                          current_record, NULL, NULL);
            } else if (g_strcmp0(previous->comparable_json, state->comparable_json) != 0) {
                g_autoptr(JsonParser) previous_parser = json_parser_new();
                if (json_parser_load_from_data(previous_parser, previous->json, -1, NULL)) {
                    JsonNode* previous_record = json_parser_get_root(previous_parser);
                    JsonObject* previous_object = json_node_get_object(previous_record);
                    JsonObject* current_object = json_node_get_object(current_record);
                    JsonArray* changed = changed_window_properties(previous_record, current_record);
                    if (json_array_get_length(changed) > 0)
                        dispatch_lua_window_event(control, revision, "gnoblin.window.changed", key,
                                                  current_record, NULL, changed);
                    if (json_object_has_member(current_object, "demandsAttention") &&
                        !window_property_equal(previous_object, current_object, "demandsAttention"))
                        dispatch_lua_window_event(control, revision,
                                                  "gnoblin.window.attention-changed", key,
                                                  current_record, NULL, NULL);
                    json_array_unref(changed);
                }
            }
        }

        /* Report focus leaving the old window before announcing the new one. */
        g_hash_table_iter_init(&iter, control->window_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWindowState* previous = value;
            NativeWindowState* state = g_hash_table_lookup(current, key);
            if (!previous->focused || !state || state->focused)
                continue;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            dispatch_lua_window_event(control, revision, "gnoblin.window.unfocused", key,
                                      json_parser_get_root(parser), NULL, NULL);
        }

        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeWindowState* state = value;
            NativeWindowState* previous = g_hash_table_lookup(control->window_state, key);
            if (!state->focused || (previous && previous->focused))
                continue;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* current_record = json_parser_get_root(parser);
            dispatch_lua_window_event(control, revision, "gnoblin.window.focused", key,
                                      current_record, NULL, NULL);
        }

        g_hash_table_iter_init(&iter, control->window_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            NativeWindowState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL)) {
                JsonNode* last = json_parser_get_root(parser);
                dispatch_lua_window_event(control, revision, "gnoblin.window.closed", key, NULL,
                                          last, NULL);
            }
        }
    }
    g_hash_table_unref(control->window_state);
    control->window_state = current;
    control->window_state_initialized = TRUE;
}

static gboolean dynamic_shortcut_id_valid(const char* id) {
    if (!id || !*id || strlen(id) > 64)
        return FALSE;
    for (const char* cursor = id; *cursor; cursor++)
        if (!g_ascii_isalnum(*cursor) && *cursor != '_' && *cursor != '-')
            return FALSE;
    return TRUE;
}

static NativeDynamicShortcut* arm_bare_super_shortcut(GnoblinNativeControl* control, const char* id,
                                                      const char* owner_id, Client* client,
                                                      guint64 session_id, GError** error) {
    if (!control || control->stopping || !control->overlay_modifier_hook_available) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "bare Super capture is unavailable in this compositor build");
        return NULL;
    }
    if (!dynamic_shortcut_id_valid(id) || !owner_id || !*owner_id || strlen(owner_id) > 128 ||
        !session_id || control->bare_super_shortcut ||
        dynamic_shortcut_count(control, NULL) >= MAX_DYNAMIC_SHORTCUTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "bare Super capture cannot be armed for this owner");
        return NULL;
    }
    for (const char* cursor = owner_id; *cursor; cursor++)
        if (g_ascii_iscntrl(*cursor)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut owner ID contains a control character");
            return NULL;
        }
    if (client && find_dynamic_shortcut(client, id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                            "shortcut ID is already registered by this connection");
        return NULL;
    }
    NativeDynamicShortcut* shortcut = g_new0(NativeDynamicShortcut, 1);
    shortcut->control = control;
    shortcut->client = client;
    shortcut->client_id = client ? client->client_id : 0;
    shortcut->id = g_strdup(id);
    shortcut->accelerator = g_strdup("Super");
    shortcut->owner_id = g_strdup(owner_id);
    shortcut->session_id = session_id;
    shortcut->trigger_release = TRUE;
    shortcut->capture_input = TRUE;
    shortcut->armed = TRUE;
    control->bare_super_shortcut = shortcut;
    return shortcut;
}

static gboolean dynamic_shortcut_accelerator_valid(const char* accelerator) {
    if (!accelerator || !*accelerator || strlen(accelerator) > 128 ||
        !g_utf8_validate(accelerator, -1, NULL))
        return FALSE;
    for (const char* cursor = accelerator; *cursor; cursor++)
        if (g_ascii_iscntrl(*cursor))
            return FALSE;
    return TRUE;
}

static gboolean dynamic_shortcut_arguments_have_only(JsonObject* arguments, const char* first,
                                                     const char* second) {
    if (!arguments)
        return FALSE;
    GList* members = json_object_get_members(arguments);
    gboolean valid = TRUE;
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        gboolean supported = g_str_equal(name, "hold") || g_str_equal(name, "trigger") ||
                             g_str_equal(name, "mode") || g_str_equal(name, "capture_input");
        if (!g_str_equal(name, first) && (!second || !g_str_equal(name, second)) && !supported) {
            valid = FALSE;
            break;
        }
    }
    g_list_free(members);
    return valid;
}

static char* dynamic_shortcut_bind(Client* client, const char* request_id, JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return encode_response(request_id, NULL, "shortcut.bind requires a live connection");
    if (!dynamic_shortcut_arguments_have_only(arguments, "id", "accelerator"))
        return encode_response(request_id, NULL,
                               "shortcut.bind accepts id, accelerator, hold, trigger, mode, and "
                               "capture_input");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    JsonNode* accelerator_node = json_object_get_member(arguments, "accelerator");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !dynamic_shortcut_id_valid(json_node_get_string(id_node)))
        return encode_response(
            request_id, NULL,
            "shortcut.bind id must contain 1 to 64 letters, digits, underscores, or hyphens");
    if (!accelerator_node || !JSON_NODE_HOLDS_VALUE(accelerator_node) ||
        json_node_get_value_type(accelerator_node) != G_TYPE_STRING ||
        !dynamic_shortcut_accelerator_valid(json_node_get_string(accelerator_node)))
        return encode_response(request_id, NULL,
                               "shortcut.bind accelerator must be a non-empty GTK accelerator "
                               "string of at most 128 bytes");
    const char* id = json_node_get_string(id_node);
    const char* accelerator = json_node_get_string(accelerator_node);
    const char* trigger = "press";
    const char* hold = "none";
    const char* mode = "passive";
    gboolean capture_input = FALSE;
    JsonNode* member = json_object_get_member(arguments, "trigger");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "press") != 0 &&
             g_strcmp0(json_node_get_string(member), "release") != 0))
            return encode_response(request_id, NULL, "trigger must be 'press' or 'release'");
        trigger = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "hold");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "none") != 0 &&
             g_strcmp0(json_node_get_string(member), "super") != 0 &&
             g_strcmp0(json_node_get_string(member), "control") != 0 &&
             g_strcmp0(json_node_get_string(member), "alt") != 0))
            return encode_response(request_id, NULL,
                                   "hold must be 'none', 'super', 'control', or 'alt'");
        hold = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "mode");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING ||
            (g_strcmp0(json_node_get_string(member), "passive") != 0 &&
             g_strcmp0(json_node_get_string(member), "modal") != 0))
            return encode_response(request_id, NULL, "mode must be 'passive' or 'modal'");
        mode = json_node_get_string(member);
    }
    member = json_object_get_member(arguments, "capture_input");
    if (member) {
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_BOOLEAN)
            return encode_response(request_id, NULL, "capture_input must be a boolean");
        capture_input = json_node_get_boolean(member);
    }
    if (client->api_minor < 22 && json_object_get_size(arguments) > 2)
        return encode_response(request_id, NULL,
                               "held and modal shortcut bindings require API version 1.22");
    if (g_str_equal(mode, "modal") && g_str_equal(hold, "none"))
        return encode_response(request_id, NULL, "modal mode requires a held modifier");
    if (capture_input && !g_str_equal(accelerator, "Super"))
        return encode_response(request_id, NULL,
                               "capture_input is only supported for an explicit bare Super "
                               "binding");
    if (g_str_equal(accelerator, "Super") &&
        (!capture_input || !g_str_equal(trigger, "release") || !g_str_equal(hold, "none")))
        return encode_response(request_id, NULL,
                               "bare Super requires capture_input=true, trigger='release', and "
                               "hold='none'");
    GnoblinNativeControl* control = client->control;
    if (find_dynamic_shortcut(client, id))
        return encode_response(request_id, NULL, "shortcut.bind id is already registered");
    if (dynamic_shortcut_count(control, client) >= MAX_DYNAMIC_SHORTCUTS_PER_CLIENT ||
        dynamic_shortcut_count(control, NULL) >= MAX_DYNAMIC_SHORTCUTS)
        return encode_response(request_id, NULL, "dynamic shortcut registration limit reached");

    NativeDynamicShortcut* shortcut = g_new0(NativeDynamicShortcut, 1);
    shortcut->control = control;
    shortcut->client = client;
    shortcut->client_id = client->client_id;
    shortcut->id = g_strdup(id);
    shortcut->accelerator = g_strdup(accelerator);
    shortcut->owner_id = g_strdup_printf("socket:%" G_GUINT64_FORMAT, client->client_id);
    shortcut->trigger_release = g_str_equal(trigger, "release");
    shortcut->modal = g_str_equal(mode, "modal");
    shortcut->capture_input = capture_input;
    shortcut->hold_mask = g_str_equal(hold, "super")     ? CLUTTER_SUPER_MASK
                          : g_str_equal(hold, "control") ? CLUTTER_CONTROL_MASK
                          : g_str_equal(hold, "alt")     ? CLUTTER_MOD1_MASK
                                                         : 0;
    shortcut->armed = TRUE;
    if (g_str_equal(accelerator, "Super")) {
        g_autoptr(GError) arm_error = NULL;
        guint64 session_id = ++control->next_shortcut_session_id;
        if (!session_id)
            session_id = ++control->next_shortcut_session_id;
        native_dynamic_shortcut_free(shortcut);
        g_autofree char* owner_id = g_strdup_printf("socket:%" G_GUINT64_FORMAT, client->client_id);
        shortcut = arm_bare_super_shortcut(control, id, owner_id, client, session_id, &arm_error);
        if (!shortcut)
            return encode_response(request_id, NULL,
                                   arm_error ? arm_error->message
                                             : "bare Super capture could not be armed");
    } else {
        guint action = meta_display_grab_accelerator(control->display, accelerator, 0);
        if (action == META_KEYBINDING_ACTION_NONE) {
            native_dynamic_shortcut_free(shortcut);
            return encode_response(request_id, NULL,
                                   "shortcut.bind accelerator is invalid or already claimed");
        }
        shortcut->action = action;
        g_hash_table_insert(control->dynamic_shortcuts, GUINT_TO_POINTER(action), shortcut);
    }

    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", id);
    json_object_set_string_member(result_object, "accelerator", accelerator);
    json_object_set_string_member(result_object, "trigger", trigger);
    json_object_set_string_member(result_object, "hold", hold);
    json_object_set_string_member(result_object, "mode", mode);
    json_object_set_boolean_member(result_object, "capture_input", capture_input);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(request_id, result, NULL);
}

static char* dynamic_shortcut_unbind(Client* client, const char* request_id,
                                     JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return encode_response(request_id, NULL, "shortcut.unbind requires a live connection");
    if (!arguments || json_object_get_size(arguments) != 1 ||
        !json_object_has_member(arguments, "id"))
        return encode_response(request_id, NULL, "shortcut.unbind accepts only id");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !dynamic_shortcut_id_valid(json_node_get_string(id_node)))
        return encode_response(request_id, NULL, "shortcut.unbind requires a valid id");
    const char* id = json_node_get_string(id_node);
    NativeDynamicShortcut* shortcut = find_dynamic_shortcut(client, id);
    if (!shortcut)
        return encode_response(request_id, NULL,
                               "shortcut.unbind id is not registered by this connection");
    remove_dynamic_shortcut(client->control, shortcut);
    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", id);
    json_object_set_boolean_member(result_object, "unbound", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(request_id, result, NULL);
}

static char* dynamic_shortcut_request_end_session(Client* client, const char* request_id,
                                                  JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return encode_response(request_id, NULL, "shortcut.session.end requires a live connection");
    if (json_object_get_size(arguments) != 2 || !json_object_has_member(arguments, "id") ||
        !json_object_has_member(arguments, "session_id"))
        return encode_response(request_id, NULL,
                               "shortcut.session.end accepts only id and session_id");
    JsonNode* id_node = json_object_get_member(arguments, "id");
    JsonNode* session_node = json_object_get_member(arguments, "session_id");
    if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
        json_node_get_value_type(id_node) != G_TYPE_STRING ||
        !dynamic_shortcut_id_valid(json_node_get_string(id_node)) || !session_node ||
        !JSON_NODE_HOLDS_VALUE(session_node) ||
        json_node_get_value_type(session_node) != G_TYPE_INT64 ||
        json_node_get_int(session_node) <= 0)
        return encode_response(request_id, NULL,
                               "shortcut.session.end requires a valid id and positive session_id");

    const char* binding_id = json_node_get_string(id_node);
    guint64 session_id = (guint64)json_node_get_int(session_node);
    NativeDynamicShortcut* shortcut = find_dynamic_shortcut(client, binding_id);
    if (!shortcut || !shortcut->active || shortcut->session_id != session_id)
        return encode_response(request_id, NULL,
                               "shortcut.session.end session is no longer active for this binding");

    dynamic_shortcut_end_session(client->control, shortcut, "cancelled");
    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", binding_id);
    json_object_set_int_member(result_object, "session_id", (gint64)session_id);
    json_object_set_boolean_member(result_object, "ended", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(request_id, result, NULL);
}

static gboolean native_api_read_method(const char* method) {
    return method &&
           (g_str_equal(method, "window.list") || g_str_equal(method, "windows.list") ||
            g_str_equal(method, "version") || g_str_equal(method, "capabilities.list") ||
            g_str_equal(method, "focus.history") || g_str_equal(method, "settings") ||
            g_str_equal(method, "appearance.color_scheme") || g_str_equal(method, "focus.policy") ||
            g_str_equal(method, "session.activity") || g_str_equal(method, "session.status") ||
            g_str_equal(method, "runtime.status") ||
            g_str_equal(method, "layer.animation_policy") ||
            g_str_equal(method, "workspaces.list") || g_str_equal(method, "monitors.list") ||
            g_str_equal(method, "layers.list") || g_str_equal(method, "launches.list") ||
            g_str_equal(method, "launches.snapshot") || g_str_equal(method, "shortcuts.list") ||
            g_str_equal(method, "shortcuts.actions") ||
            g_str_equal(method, "input.orientation_lock"));
}

static gboolean runtime_config_values_equal(GVariant* left, GVariant* right) {
    if (!left || !right ||
        !g_variant_type_equal(g_variant_get_type(left), g_variant_get_type(right)))
        return FALSE;

    if (g_variant_is_of_type(left, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) left_value = g_variant_get_variant(left);
        g_autoptr(GVariant) right_value = g_variant_get_variant(right);
        return runtime_config_values_equal(left_value, right_value);
    }

    if (g_variant_is_of_type(left, G_VARIANT_TYPE_VARDICT)) {
        if (g_variant_n_children(left) != g_variant_n_children(right))
            return FALSE;

        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, left);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) left_value = value;
            g_autoptr(GVariant) right_value = g_variant_lookup_value(right, key, NULL);
            if (!runtime_config_values_equal(left_value, right_value))
                return FALSE;
        }
        return TRUE;
    }

    if (g_variant_is_container(left)) {
        if (g_variant_n_children(left) != g_variant_n_children(right))
            return FALSE;

        for (gsize i = 0; i < g_variant_n_children(left); i++) {
            g_autoptr(GVariant) left_value = g_variant_get_child_value(left, i);
            g_autoptr(GVariant) right_value = g_variant_get_child_value(right, i);
            if (!runtime_config_values_equal(left_value, right_value))
                return FALSE;
        }
        return TRUE;
    }

    return g_variant_equal(left, right);
}

static gboolean runtime_reload_document_supported(GVariant* current, GVariant* candidate) {
    static const char* const reloadable_settings[] = {
        "animations",        "input",        "input-sources", "permissions",
        "touchpad-gestures", "window-rules", "workspaces",    NULL};
    GVariantIter iter;
    const char* key;
    GVariant* value;

    g_variant_iter_init(&iter, current);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) current_value = value;
        gboolean reloadable = FALSE;
        for (guint i = 0; reloadable_settings[i]; i++)
            reloadable |= g_str_equal(key, reloadable_settings[i]);
        if (reloadable)
            continue;

        g_autoptr(GVariant) candidate_value = g_variant_lookup_value(candidate, key, NULL);
        if (!candidate_value || !runtime_config_values_equal(current_value, candidate_value))
            return FALSE;
    }

    g_variant_iter_init(&iter, candidate);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) candidate_value = value;
        gboolean reloadable = FALSE;
        for (guint i = 0; reloadable_settings[i]; i++)
            reloadable |= g_str_equal(key, reloadable_settings[i]);
        if (reloadable)
            continue;

        g_autoptr(GVariant) current_value = g_variant_lookup_value(current, key, NULL);
        if (!current_value || !runtime_config_values_equal(current_value, candidate_value))
            return FALSE;
    }

    return TRUE;
}

static char* queue_runtime_api_request_internal(Client* client, const char* request_id,
                                                const char* method, GVariant* arguments,
                                                const char* kind, const char* legacy_window_action,
                                                const char* legacy_window_id) {
    GnoblinNativeControl* control = client->control;
    if (control->runtime_worker_suspended)
        return encode_response(request_id, NULL, "Lua worker is restarting");
    guint64 channel_id = ++control->next_runtime_request_id;
    if (channel_id == 0)
        channel_id = ++control->next_runtime_request_id;
    guint64* key = g_new(guint64, 1);
    *key = channel_id;
    PendingRuntimeRequest* pending = g_new0(PendingRuntimeRequest, 1);
    pending->client = client;
    pending->request_id = g_strdup(request_id);
    pending->legacy_window_action = g_strdup(legacy_window_action);
    pending->legacy_window_id = g_strdup(legacy_window_id);
    g_hash_table_insert(control->pending_runtime_requests, key, pending);
    client->pending_deferred_requests++;

    GVariantBuilder request;
    g_variant_builder_init(&request, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&request, "{sv}", "kind", g_variant_new_string(kind));
    g_variant_builder_add(&request, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&request, "{sv}", "arguments", arguments);
    if (g_str_equal(kind, "call"))
        g_variant_builder_add(&request, "{sv}", "client_id",
                              g_variant_new_uint64(client->client_id));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&request));
    g_autoptr(GError) error = NULL;
    if (native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_API_REQUEST, channel_id, payload,
                            &error))
        return NULL;

    g_hash_table_remove(control->pending_runtime_requests, &channel_id);
    client->pending_deferred_requests--;
    client_maybe_free(client);
    return encode_response(request_id, NULL,
                           error ? error->message : "could not queue runtime API request");
}

static char* queue_runtime_api_request(Client* client, const char* request_id, const char* method,
                                       GVariant* arguments, const char* kind) {
    return queue_runtime_api_request_internal(client, request_id, method, arguments, kind, NULL,
                                              NULL);
}

static gboolean json_integer(JsonNode* node, gint64* value) {
    if (!node || !JSON_NODE_HOLDS_VALUE(node))
        return FALSE;
    GType type = json_node_get_value_type(node);
    if (type != G_TYPE_INT && type != G_TYPE_INT64)
        return FALSE;
    *value = json_node_get_int(node);
    return TRUE;
}

static GVariant* native_socket_snap_rect(JsonNode* node) {
    if (!node || !JSON_NODE_HOLDS_OBJECT(node))
        return NULL;
    JsonObject* object = json_node_get_object(node);
    if (json_object_get_size(object) != 4)
        return NULL;
    const char* names[] = {"x", "y", "width", "height"};
    gint32 values[4];
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        gint64 value;
        if (!json_integer(json_object_get_member(object, names[i]), &value) || value < G_MININT32 ||
            value > G_MAXINT32)
            return NULL;
        values[i] = value;
    }
    GVariantBuilder rect;
    g_variant_builder_init(&rect, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < G_N_ELEMENTS(names); i++)
        g_variant_builder_add(&rect, "{sv}", names[i], g_variant_new_int32(values[i]));
    return g_variant_ref_sink(g_variant_builder_end(&rect));
}

static GVariant* native_socket_snap_target(JsonNode* node) {
    if (!node || !JSON_NODE_HOLDS_OBJECT(node))
        return NULL;
    JsonObject* object = json_node_get_object(node);
    if (json_object_get_size(object) < 3 || json_object_get_size(object) > 6)
        return NULL;
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "id") && !g_str_equal(key, "hit") && !g_str_equal(key, "frame") &&
            !g_str_equal(key, "maximize") && !g_str_equal(key, "required_modifiers") &&
            !g_str_equal(key, "forbidden_modifiers")) {
            g_list_free(members);
            return NULL;
        }
    }
    g_list_free(members);
    const char* id = json_object_get_string_member_with_default(object, "id", NULL);
    if (!id || !*id || strlen(id) > 64 || !g_utf8_validate(id, -1, NULL))
        return NULL;
    g_autoptr(GVariant) hit = native_socket_snap_rect(json_object_get_member(object, "hit"));
    g_autoptr(GVariant) frame = native_socket_snap_rect(json_object_get_member(object, "frame"));
    if (!hit || !frame)
        return NULL;
    GVariantBuilder target;
    g_variant_builder_init(&target, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&target, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&target, "{sv}", "hit", hit);
    g_variant_builder_add(&target, "{sv}", "frame", frame);
    const char* modifiers[] = {"required_modifiers", "forbidden_modifiers"};
    for (guint m = 0; m < G_N_ELEMENTS(modifiers); m++) {
        if (!json_object_has_member(object, modifiers[m]))
            continue;
        JsonNode* array_node = json_object_get_member(object, modifiers[m]);
        if (!JSON_NODE_HOLDS_ARRAY(array_node) ||
            json_array_get_length(json_node_get_array(array_node)) > 8) {
            g_variant_builder_clear(&target);
            return NULL;
        }
        GVariantBuilder array;
        g_variant_builder_init(&array, G_VARIANT_TYPE_STRING_ARRAY);
        JsonArray* source = json_node_get_array(array_node);
        for (guint i = 0; i < json_array_get_length(source); i++) {
            JsonNode* item = json_array_get_element(source, i);
            if (!JSON_NODE_HOLDS_VALUE(item) || json_node_get_value_type(item) != G_TYPE_STRING) {
                g_variant_builder_clear(&array);
                g_variant_builder_clear(&target);
                return NULL;
            }
            g_variant_builder_add(&array, "s", json_node_get_string(item));
        }
        g_variant_builder_add(&target, "{sv}", modifiers[m], g_variant_builder_end(&array));
    }
    if (json_object_has_member(object, "maximize")) {
        JsonNode* value = json_object_get_member(object, "maximize");
        if (!JSON_NODE_HOLDS_VALUE(value) || json_node_get_value_type(value) != G_TYPE_BOOLEAN) {
            g_variant_builder_clear(&target);
            return NULL;
        }
        g_variant_builder_add(&target, "{sv}", "maximize",
                              g_variant_new_boolean(json_node_get_boolean(value)));
    }
    return g_variant_ref_sink(g_variant_builder_end(&target));
}

static char* native_socket_window_snap_offer(Client* client, const char* id,
                                             JsonObject* arguments) {
    if (!arguments || json_object_get_size(arguments) != 3 ||
        !json_object_has_member(arguments, "drag_id") ||
        !json_object_has_member(arguments, "drag_token") ||
        !json_object_has_member(arguments, "targets"))
        return encode_response(id, NULL,
                               "window.snap.offer requires drag_id, drag_token and targets");
    gint64 drag_id_value;
    JsonNode* drag_id_node = json_object_get_member(arguments, "drag_id");
    JsonNode* token_node = json_object_get_member(arguments, "drag_token");
    JsonNode* targets_node = json_object_get_member(arguments, "targets");
    if (!json_integer(drag_id_node, &drag_id_value) || drag_id_value <= 0 ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING ||
        !JSON_NODE_HOLDS_ARRAY(targets_node))
        return encode_response(id, NULL, "window.snap.offer arguments are invalid");
    const char* token = json_node_get_string(token_node);
    JsonArray* target_array = json_node_get_array(targets_node);
    if (!token || strlen(token) != 64 || json_array_get_length(target_array) == 0 ||
        json_array_get_length(target_array) > NATIVE_DRAG_MAX_TARGETS)
        return encode_response(id, NULL, "window.snap.offer token or target count is invalid");
    guint64 drag_id = drag_id_value;
    NativeWindowDrag* drag = g_hash_table_lookup(client->control->window_drags, &drag_id);
    const char* expected = drag && drag->client_tokens
                               ? g_hash_table_lookup(drag->client_tokens, &client->client_id)
                               : NULL;
    if (!drag || !expected || !g_str_equal(token, expected) ||
        drag->owner_generation != client->control->runtime_generation ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->settings_revision != native_config_revision(client->control) ||
        (client->control->wayland_compositor &&
         meta_wayland_session_lock_is_active(client->control->wayland_compositor)))
        return encode_response(id, NULL,
                               "window.snap.offer token is stale or not owned by this client");
    if (drag->runtime_offer_claimed && !drag->socket_owner_client_id)
        return encode_response(id, NULL, "window.snap.offer is owned by the Lua runtime");
    if (drag->socket_owner_client_id && drag->socket_owner_client_id != client->client_id)
        return encode_response(id, NULL, "window.snap.offer is owned by another shell client");
    GVariantBuilder targets;
    g_variant_builder_init(&targets, G_VARIANT_TYPE("av"));
    for (guint i = 0; i < json_array_get_length(target_array); i++) {
        g_autoptr(GVariant) target =
            native_socket_snap_target(json_array_get_element(target_array, i));
        if (!target) {
            g_variant_builder_clear(&targets);
            return encode_response(id, NULL, "window.snap.offer contains an invalid target");
        }
        g_variant_builder_add_value(&targets, g_variant_new_variant(target));
    }
    g_autoptr(GVariant) target_values = g_variant_ref_sink(g_variant_builder_end(&targets));
    g_autoptr(GError) error = NULL;
    if (!native_snap_offer_set_targets(drag, target_values, &error))
        return encode_response(id, NULL, error ? error->message : "snap offer was rejected");
    drag->socket_owner_client_id = client->client_id;
    JsonObject* result_object = json_object_new();
    json_object_set_boolean_member(result_object, "accepted", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(id, result, NULL);
}

static void native_socket_consume_rejected_menu_context(Client* client, JsonObject* request) {
    if (!client || !client->control || !client->menu_grants || !request)
        return;
    const char* method = json_object_get_string_member_with_default(request, "method", "");
    if (!g_str_equal(method, "window.begin_move") && !g_str_equal(method, "window.begin_resize"))
        return;
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (!arguments_node || !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return;
    JsonNode* token_node =
        json_object_get_member(json_node_get_object(arguments_node), "menu_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING)
        return;
    const char* token = json_node_get_string(token_node);
    NativeMenuGrant* grant =
        focus_token_has_valid_shape(token) ? g_hash_table_lookup(client->menu_grants, token) : NULL;
    if (!grant)
        return;
    guint64 handle = grant->handle;
    g_hash_table_remove(client->menu_grants, token);
    if (client->control->menu_contexts)
        g_hash_table_remove(client->control->menu_contexts, &handle);
}

static char* handle_request(Client* client, const char* data, gsize length) {
    g_autoptr(JsonParser) parser = json_parser_new();
    g_autoptr(GError) error = NULL;
    const char* id = "";
    if (!json_parser_load_from_data(parser, data, length, &error) ||
        !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
        client->close_after_response = TRUE;
        return encode_response(id, NULL, "invalid JSON request");
    }

    JsonObject* request = json_node_get_object(json_parser_get_root(parser));
    const char* op = json_object_get_string_member_with_default(request, "op", "");
    if (g_str_equal(op, "ping")) {
        JsonNode* id_node = json_object_get_member(request, "id");
        if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING ||
            !json_node_get_string(id_node)[0] || strlen(json_node_get_string(id_node)) > 64)
            return encode_response("", NULL, "invalid request ID");
        const char* ping_id = json_node_get_string(id_node);
        if (json_object_get_size(request) != 2)
            return encode_response(ping_id, NULL, "ping accepts only op and id");
        JsonObject* pong = json_object_new();
        json_object_set_string_member(pong, "pong", "pong");
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, pong);
        return encode_response(ping_id, result, NULL);
    }
    if (client->control && client->control->stopping) {
        JsonNode* id_node = json_object_get_member(request, "id");
        const char* request_id = id_node && JSON_NODE_HOLDS_VALUE(id_node) &&
                                         json_node_get_value_type(id_node) == G_TYPE_STRING
                                     ? json_node_get_string(id_node)
                                     : "";
        return encode_response(request_id, NULL, "Gnoblin compositor control is stopping");
    }
    if (json_object_has_member(request, "api_version") ||
        json_object_has_member(request, "api_major") ||
        json_object_has_member(request, "api_minor")) {
        JsonNode* version_node = json_object_get_member(request, "api_version");
        JsonObject* version =
            JSON_NODE_HOLDS_OBJECT(version_node) ? json_node_get_object(version_node) : NULL;
        JsonNode* major_node = version ? json_object_get_member(version, "major")
                                       : json_object_get_member(request, "api_major");
        JsonNode* minor_node = version ? json_object_get_member(version, "minor")
                                       : json_object_get_member(request, "api_minor");
        if ((version && json_object_get_size(version) != 2) || !major_node || !minor_node ||
            !JSON_NODE_HOLDS_VALUE(major_node) || !JSON_NODE_HOLDS_VALUE(minor_node) ||
            (json_node_get_value_type(major_node) != G_TYPE_INT64 &&
             json_node_get_value_type(major_node) != G_TYPE_INT) ||
            (json_node_get_value_type(minor_node) != G_TYPE_INT64 &&
             json_node_get_value_type(minor_node) != G_TYPE_INT)) {
            if (g_str_equal(op, "api"))
                native_socket_consume_rejected_menu_context(client, request);
            client->close_after_response = TRUE;
            return encode_response("", NULL,
                                   "API version must contain integer major and minor fields");
        }
        gint64 major = json_node_get_int(major_node);
        gint64 minor = json_node_get_int(minor_node);
        if (major != GNOBLIN_NATIVE_CONTROL_API_MAJOR || minor < 0 ||
            minor > GNOBLIN_NATIVE_CONTROL_API_MINOR) {
            if (g_str_equal(op, "api"))
                native_socket_consume_rejected_menu_context(client, request);
            client->close_after_response = TRUE;
            return encode_response("", NULL, "requested compositor API version is unsupported");
        }
        client->api_minor = minor;
    }
    if (g_str_equal(op, "api") && client->api_minor < 30)
        native_socket_consume_rejected_menu_context(client, request);
    if (g_str_equal(op, "events")) {
        if (client->api_minor < 9)
            return encode_response("", NULL, "events subscriptions require API version 1.9");
        JsonNode* events_node = json_object_get_member(request, "events");
        if (!events_node || !JSON_NODE_HOLDS_ARRAY(events_node))
            return encode_response("", NULL, "events must be an array of event names");
        JsonArray* requested = json_node_get_array(events_node);
        if (json_array_get_length(requested) > 64)
            return encode_response("", NULL, "events may contain at most 64 names");
        GHashTable* subscriptions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        for (guint i = 0; i < json_array_get_length(requested); i++) {
            JsonNode* name_node = json_array_get_element(requested, i);
            if (!JSON_NODE_HOLDS_VALUE(name_node) ||
                json_node_get_value_type(name_node) != G_TYPE_STRING) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events entries must be strings");
            }
            const char* name = json_node_get_string(name_node);
            if (!native_event_is_subscribable(name)) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events contains an unsupported event name");
            }
            if (g_str_equal(name, "gnoblin.shortcut.activated") && client->api_minor < 10) {
                g_hash_table_unref(subscriptions);
                return encode_response(
                    "", NULL, "shortcut activation subscriptions require API version 1.10");
            }
            if (g_str_equal(name, "gnoblin.shortcut.binding-activated") && client->api_minor < 11) {
                g_hash_table_unref(subscriptions);
                return encode_response(
                    "", NULL, "dynamic shortcut activation events require API version 1.11");
            }
            if (g_str_equal(name, "gnoblin.shortcut.binding-deactivated") &&
                client->api_minor < 36) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "shortcut deactivation events require API version 1.36");
            }
            if ((g_str_equal(name, "gnoblin.shortcut.session.activated") ||
                 g_str_equal(name, "gnoblin.shortcut.session.key") ||
                 g_str_equal(name, "gnoblin.shortcut.session.ended")) &&
                client->api_minor < 22) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "shortcut session events require API version 1.22");
            }
            if (g_str_equal(name, "gnoblin.focus.policy-changed") && client->api_minor < 13) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "focus policy events require API version 1.13");
            }
            if (g_str_equal(name, "gnoblin.permission.changed") && client->api_minor < 16) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "permission policy events require API version 1.16");
            }
            if (g_str_equal(name, "gnoblin.privacy.changed") && client->api_minor < 17) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "privacy state events require API version 1.17");
            }
            if (g_str_equal(name, "gnoblin.location.authorization-requested") &&
                client->api_minor < 65) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "location authorization events require API version 1.65");
            }
            if (g_str_equal(name, "gnoblin.input.orientation-lock-changed") &&
                client->api_minor < 66) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "orientation lock events require API version 1.66");
            }
            if (g_str_equal(name, "gnoblin.capability.changed") && client->api_minor < 33) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "capability change events require API version 1.33");
            }
            if ((g_str_equal(name, "gnoblin.animation.started") ||
                 g_str_equal(name, "gnoblin.animation.finished")) &&
                client->api_minor < 18) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "animation events require API version 1.18");
            }
            if ((g_str_equal(name, "gnoblin.config.reloaded") ||
                 g_str_equal(name, "gnoblin.config.reload-failed")) &&
                client->api_minor < 20) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "config reload events require API version 1.20");
            }
            if ((g_str_equal(name, "gnoblin.portal.grant-added") ||
                 g_str_equal(name, "gnoblin.portal.grant-removed")) &&
                client->api_minor < 15) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "portal grant events require API version 1.15");
            }
            if (g_str_equal(name, "gnoblin.operation.completed") && client->api_minor < 11) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "gnoblin.operation.completed requires API version 1.11");
            }
            if ((g_str_equal(name, "gnoblin.session.lock-requested") ||
                 g_str_equal(name, "gnoblin.session.lock-state-changed")) &&
                client->api_minor < 21) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "session lock events require API version 1.21");
            }
            if ((g_str_equal(name, "gnoblin.window.drag.started") ||
                 g_str_equal(name, "gnoblin.window.drag.updated") ||
                 g_str_equal(name, "gnoblin.window.drag.ended")) &&
                client->api_minor < 26) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "window drag events require API version 1.26");
            }
            if ((g_str_equal(name, "gnoblin.window.menu-requested") ||
                 g_str_equal(name, "gnoblin.osd.requested")) &&
                client->api_minor < 27) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "shell request events require API version 1.27");
            }
            if (g_str_equal(name, "gnoblin.window.activation-denied") && client->api_minor < 69) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "activation denial events require API version 1.69");
            }
            if (g_str_equal(name, "gnoblin.session.activity-changed") && client->api_minor < 24) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "session activity events require API version 1.24");
            }
            if (g_str_equal(name, "gnoblin.appearance.color-scheme-changed") &&
                client->api_minor < 34) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "appearance change events require API version 1.34");
            }
            if (g_hash_table_contains(subscriptions, name)) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events must not contain duplicate names");
            }
            g_hash_table_add(subscriptions, g_strdup(name));
        }
        native_socket_revoke_client_tokens(client);
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
        native_window_drag_client_disconnected(client->control, client->client_id);
        g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
        client->event_subscriptions = subscriptions;
        client->event_api_minor = client->api_minor;
        return encode_event_subscription(requested, client->event_api_minor);
    }
    if (g_str_equal(op, "windows")) {
        client->windows_api_minor = client->api_minor;
        client->track_windows = TRUE;
        return window_snapshot(client->control);
    }
    if (g_str_equal(op, "monitors")) {
        client->monitors_api_minor = client->api_minor;
        client->track_monitors = TRUE;
        return monitor_snapshot(client->control);
    }
    if (json_object_has_member(request, "id") &&
        JSON_NODE_HOLDS_VALUE(json_object_get_member(request, "id")) &&
        json_node_get_value_type(json_object_get_member(request, "id")) == G_TYPE_STRING)
        id = json_object_get_string_member(request, "id");
    if (!*id || strlen(id) > 64)
        return encode_response("", NULL, "invalid request ID");
    const char* method = json_object_get_string_member_with_default(request, "method", "");
    if (!g_str_equal(op, "api"))
        return encode_response(id, NULL, "unsupported compositor operation");
    if (g_str_equal(method, "layer.list") && client->api_minor < 2)
        return encode_response(id, NULL, "layer.list requires API version 1.2");
    if (g_str_equal(method, "input.devices") && client->api_minor < 3)
        return encode_response(id, NULL, "input.devices requires API version 1.3");
    if ((g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source") ||
         g_str_equal(method, "input.select_source") || g_str_equal(method, "input.select")) &&
        client->api_minor < 6)
        return encode_response(id, NULL, "input source methods require API version 1.6");
    if ((g_str_equal(method, "input.orientation_lock") ||
         g_str_equal(method, "input.set_orientation_lock")) &&
        client->api_minor < 66)
        return encode_response(id, NULL, "orientation lock methods require API version 1.66");
    if (g_str_equal(method, "appearance.color_scheme") && client->api_minor < 70)
        return encode_response(id, NULL, "appearance.color_scheme requires API version 1.70");
    if (g_str_equal(method, "shortcut.actions") && client->api_minor < 5)
        return encode_response(id, NULL, "shortcut.actions requires API version 1.5");
    if (g_str_equal(method, "shortcut.capture") && client->api_minor < 8)
        return encode_response(id, NULL, "shortcut.capture requires API version 1.8");
    if (g_str_equal(method, "shortcut.list") && client->api_minor < 9)
        return encode_response(id, NULL, "shortcut.list requires API version 1.9");
    if (g_str_equal(method, "window.focus") && client->api_minor < 10)
        return encode_response(id, NULL, "window.focus requires API version 1.10");
    if ((g_str_equal(method, "window.begin_move") || g_str_equal(method, "window.begin_resize")) &&
        client->api_minor < 12)
        return encode_response(id, NULL, "interactive window operations require API version 1.12");
    if ((g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) &&
        client->api_minor < 11)
        return encode_response(id, NULL, "dynamic shortcut methods require API version 1.11");
    if (g_str_equal(method, "shortcut.session.end") && client->api_minor < 64)
        return encode_response(id, NULL, "shortcut.session.end requires API version 1.64");
    if (g_str_equal(method, "location.authorize_app") && client->api_minor < 65)
        return encode_response(id, NULL, "location.authorize_app requires API version 1.65");
    if ((g_str_equal(method, "grant.list") || g_str_equal(method, "grant.revoke")) &&
        client->api_minor < 14)
        return encode_response(id, NULL, "portal grant methods require API version 1.14");
    if (g_str_equal(method, "portals.grants") && client->api_minor < 15)
        return encode_response(id, NULL, "portals.grants requires API version 1.15");
    if (g_str_equal(method, "permissions.policy") && client->api_minor < 16)
        return encode_response(id, NULL, "permissions.policy requires API version 1.16");
    if (g_str_equal(method, "privacy.state") && client->api_minor < 17)
        return encode_response(id, NULL, "privacy.state requires API version 1.17");
    if (g_str_equal(method, "session.lock") && client->api_minor < 21)
        return encode_response(id, NULL, "session.lock requires API version 1.21");
    if (g_str_equal(method, "session.activity") && client->api_minor < 24)
        return encode_response(id, NULL, "session.activity requires API version 1.24");
    if (g_str_equal(method, "session.status") && client->api_minor < 29)
        return encode_response(id, NULL, "session.status requires API version 1.29");
    if (g_str_equal(method, "session.logout") && client->api_minor < 32)
        return encode_response(id, NULL, "session.logout requires API version 1.32");
    if (g_str_equal(method, "window.restore_or_minimize") && client->api_minor < 38)
        return encode_response(id, NULL, "window.restore_or_minimize requires API version 1.38");
    if (g_str_equal(method, "launches.snapshot") && client->api_minor < 39)
        return encode_response(id, NULL, "launches.snapshot requires API version 1.39");
    if (g_str_equal(method, "shortcuts.list") && client->api_minor < 40)
        return encode_response(id, NULL, "shortcuts.list requires API version 1.40");
    if (g_str_equal(method, "shortcuts.actions") && client->api_minor < 41)
        return encode_response(id, NULL, "shortcuts.actions requires API version 1.41");
    if ((g_str_equal(method, "privacy.stop_sharing") ||
         g_str_equal(method, "privacy.stop_recording")) &&
        client->api_minor < 31)
        return encode_response(id, NULL, "privacy stop methods require API version 1.31");
    if (g_str_equal(method, "layer.animation_policy") && client->api_minor < 31)
        return encode_response(id, NULL, "layer.animation_policy requires API version 1.31");
    if ((g_str_equal(method, "windows.list") || g_str_equal(method, "workspaces.list") ||
         g_str_equal(method, "monitors.list") || g_str_equal(method, "layers.list") ||
         g_str_equal(method, "launches.list")) &&
        client->api_minor < 37)
        return encode_response(id, NULL, "Lua snapshot collection reads require API version 1.37");
    if (g_str_equal(method, "window.thumbnail") && client->api_minor < 23)
        return encode_response(id, NULL, "window.thumbnail requires API version 1.23");
    if (g_str_equal(method, "window.snap.offer") && client->api_minor < 26)
        return encode_response(id, NULL, "window.snap.offer requires API version 1.26");
    if ((g_str_equal(method, "input.text_target") || g_str_equal(method, "input.insert_text") ||
         g_str_equal(method, "window.snap_context") || g_str_equal(method, "window.snap")) &&
        client->api_minor < 28)
        return encode_response(id, NULL,
                               "text insertion and keyboard snap methods require API version 1.28");
    if (g_str_has_prefix(method, "animation.") && client->api_minor < 18)
        return encode_response(id, NULL, "animation methods require API version 1.18");
    if (native_api_read_method(method) && client->api_minor < 19)
        return encode_response(id, NULL, "shared snapshot reads require API version 1.19");
    if (g_str_equal(method, "runtime.reload_config") && client->api_minor < 20)
        return encode_response(id, NULL, "runtime.reload_config requires API version 1.20");
    if (g_str_equal(method, "runtime.status") && client->api_minor < 67)
        return encode_response(id, NULL, "runtime.status requires API version 1.67");
    if (g_str_has_prefix(method, "launch.") && client->api_minor < 7)
        return encode_response(id, NULL, "launch methods require API version 1.7");
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (arguments_node && !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return encode_response(id, NULL, "arguments must be an object");
    if (g_str_equal(method, "window.list") || g_str_equal(method, "workspace.list")) {
        if (g_str_equal(method, "workspace.list") && arguments_node &&
            json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "workspace.list does not accept arguments");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments) {
            const char* message = g_str_equal(method, "window.list")
                                      ? "window.list arguments are invalid"
                                      : "workspace.list arguments are invalid";
            return encode_response(id, NULL, message);
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "read");
    }
    if (g_str_equal(method, "session.status")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "session.status does not accept arguments");
        g_autoptr(GVariant) lock_snapshot = native_session_lock_snapshot(
            client->control->wayland_compositor, client->control->session_lock_revision);
        gboolean lock_available = FALSE;
        const char* lock_state = NULL;
        guint64 revision = 0;
        g_variant_lookup(lock_snapshot, "lock_available", "b", &lock_available);
        g_variant_lookup(lock_snapshot, "lock_state", "&s", &lock_state);
        g_variant_lookup(lock_snapshot, "revision", "t", &revision);
        JsonObject* result_object = json_object_new();
        json_object_set_string_member(result_object, "state", "running");
        json_object_set_boolean_member(result_object, "lock_available", lock_available);
        json_object_set_int_member(result_object, "revision", (gint64)MIN(revision, G_MAXINT64));
        if (lock_available && lock_state)
            json_object_set_string_member(result_object, "lock_state", lock_state);
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, result_object);
        return encode_response(id, result, NULL);
    }
    if (g_str_equal(method, "window.restore_or_minimize")) {
        JsonObject* json_arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        const char* fields[] = {"id"};
        JsonNode* id_node = json_arguments ? json_object_get_member(json_arguments, "id") : NULL;
        if (!native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) ||
            !id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING || !json_node_get_string(id_node) ||
            !*json_node_get_string(id_node))
            return encode_response(id, NULL,
                                   "window.restore_or_minimize requires only a stable window id");
        g_autoptr(JsonNode) arguments_object = json_node_new(JSON_NODE_OBJECT);
        json_node_set_object(arguments_object, json_object_ref(json_arguments));
        g_autoptr(GVariant) operation_arguments = variant_from_json(arguments_object);
        if (!operation_arguments)
            return encode_response(id, NULL, "window.restore_or_minimize arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, operation_arguments, "call");
    }
    if (g_str_equal(method, "input.text_target") || g_str_equal(method, "input.insert_text") ||
        g_str_equal(method, "window.snap_context") || g_str_equal(method, "window.snap")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        g_autoptr(GError) method_error = NULL;
        g_autoptr(GVariant) result = NULL;
        if (g_str_equal(method, "input.text_target"))
            result = native_socket_create_text_target(client, arguments, &method_error);
        else if (g_str_equal(method, "input.insert_text"))
            result = native_socket_insert_text(
                client, arguments, native_socket_has_nul_escape(data, length), &method_error);
        else if (g_str_equal(method, "window.snap_context"))
            result = native_socket_create_snap_context(client, arguments, &method_error);
        else
            result = native_socket_commit_snap_context(client, arguments, &method_error);
        if (!result)
            return encode_response(
                id, NULL, method_error ? method_error->message : "native API request failed");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "window.snap.offer"))
        return native_socket_window_snap_offer(
            client, id, arguments_node ? json_node_get_object(arguments_node) : NULL);
    if (g_str_equal(method, "runtime.reload_config")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "runtime.reload_config arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "reload");
    }
    if (g_str_equal(method, "runtime.status")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "runtime.status does not accept arguments");
        const char* state = "starting";
        if (!client->control->supervised_runtime || client->control->stopping)
            state = "unavailable";
        else if (client->control->runtime_worker_suspended)
            state = "restarting";
        else if (client->control->runtime_hello_sent)
            state = "running";
        GVariantBuilder status;
        g_variant_builder_init(&status, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&status, "{sv}", "state", g_variant_new_string(state));
        g_variant_builder_add(&status, "{sv}", "generation",
                              g_variant_new_int64((gint64)client->control->runtime_generation));
        g_autoptr(GVariant) result = g_variant_ref_sink(g_variant_builder_end(&status));
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "session.lock")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "session.lock does not accept arguments");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "session.lock arguments are invalid");
        if (client->api_minor >= 49) {
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, arguments, "call");
        }
        g_autoptr(GVariant) result = gnoblin_native_control_request_session_lock(
            client->control->display, arguments, &error);
        if (!result)
            return encode_response(id, NULL,
                                   error ? error->message : "session-lock request failed");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (native_api_read_method(method)) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "layer.list")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "monitor.list")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "privacy.state")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "privacy.state does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "portals.grants")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        const char* kind_filter = NULL;
        if (arguments &&
            (json_object_get_size(arguments) > 1 ||
             (json_object_get_size(arguments) == 1 && !json_object_has_member(arguments, "kind"))))
            return encode_response(id, NULL, "portals.grants accepts only an optional kind");
        if (arguments && json_object_has_member(arguments, "kind")) {
            JsonNode* kind_node = json_object_get_member(arguments, "kind");
            if (!JSON_NODE_HOLDS_VALUE(kind_node) ||
                json_node_get_value_type(kind_node) != G_TYPE_STRING)
                return encode_response(id, NULL, "portal grant kind must be a string");
            kind_filter = json_node_get_string(kind_node);
            if (!g_str_equal(kind_filter, "screen-cast") &&
                !g_str_equal(kind_filter, "remote-desktop"))
                return encode_response(id, NULL,
                                       "portal grant kind must be screen-cast or remote-desktop");
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!read_arguments)
            return encode_response(id, NULL, "portals.grants arguments are invalid");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.list does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.policy")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.policy does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.check")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        if (!arguments || json_object_get_size(arguments) != 2 ||
            !json_object_has_member(arguments, "capability") ||
            !json_object_has_member(arguments, "identity") ||
            !JSON_NODE_HOLDS_VALUE(json_object_get_member(arguments, "capability")) ||
            !JSON_NODE_HOLDS_VALUE(json_object_get_member(arguments, "identity")) ||
            json_node_get_value_type(json_object_get_member(arguments, "capability")) !=
                G_TYPE_STRING ||
            json_node_get_value_type(json_object_get_member(arguments, "identity")) !=
                G_TYPE_STRING)
            return encode_response(id, NULL,
                                   "permissions.check requires string capability and identity");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        g_autoptr(GVariant) read_arguments = variant_from_json(arguments_node);
        if (!read_arguments)
            return encode_response(id, NULL, "permissions.check arguments are invalid");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "window.action")) {
        JsonObject* arguments = arguments_node && JSON_NODE_HOLDS_OBJECT(arguments_node)
                                    ? json_node_get_object(arguments_node)
                                    : NULL;
        JsonNode* action_node = arguments ? json_object_get_member(arguments, "action") : NULL;
        if (action_node && JSON_NODE_HOLDS_VALUE(action_node) &&
            json_node_get_value_type(action_node) == G_TYPE_STRING &&
            g_str_equal(json_node_get_string(action_node), "focus"))
            return encode_response(
                id, NULL,
                "native window.action focus is denied; use window.focus with a live focus_context");

        if (!action_node || !JSON_NODE_HOLDS_VALUE(action_node) ||
            json_node_get_value_type(action_node) != G_TYPE_STRING)
            return encode_response(id, NULL, "window.action requires a string action");

        const char* action = json_node_get_string(action_node);
        gboolean move_action = g_str_equal(action, "move");
        gboolean resize_action = g_str_equal(action, "resize");
        gboolean workspace_action = g_str_equal(action, "workspace");
        gboolean monitor_action = g_str_equal(action, "monitor");
        const char* action_fields[] = {"action", "window"};
        const char* move_fields[] = {"action", "window", "x", "y"};
        const char* move_fields_without_window[] = {"action", "x", "y"};
        const char* resize_fields[] = {"action", "window", "width", "height"};
        const char* resize_fields_without_window[] = {"action", "width", "height"};
        const char* workspace_fields[] = {"action", "window", "workspace"};
        const char* workspace_fields_without_window[] = {"action", "workspace"};
        const char* monitor_fields[] = {"action", "window", "monitor"};
        const char* monitor_fields_without_window[] = {"action", "monitor"};
        gboolean valid_fields =
            !arguments ? FALSE
            : move_action
                ? native_socket_has_exact_fields(arguments, move_fields,
                                                 G_N_ELEMENTS(move_fields)) ||
                      native_socket_has_exact_fields(arguments, move_fields_without_window,
                                                     G_N_ELEMENTS(move_fields_without_window))
            : resize_action
                ? native_socket_has_exact_fields(arguments, resize_fields,
                                                 G_N_ELEMENTS(resize_fields)) ||
                      native_socket_has_exact_fields(arguments, resize_fields_without_window,
                                                     G_N_ELEMENTS(resize_fields_without_window))
            : workspace_action
                ? native_socket_has_exact_fields(arguments, workspace_fields,
                                                 G_N_ELEMENTS(workspace_fields)) ||
                      native_socket_has_exact_fields(arguments, workspace_fields_without_window,
                                                     G_N_ELEMENTS(workspace_fields_without_window))
            : monitor_action
                ? native_socket_has_exact_fields(arguments, monitor_fields,
                                                 G_N_ELEMENTS(monitor_fields)) ||
                      native_socket_has_exact_fields(arguments, monitor_fields_without_window,
                                                     G_N_ELEMENTS(monitor_fields_without_window))
                : json_object_get_size(arguments) == 1 ||
                      native_socket_has_exact_fields(arguments, action_fields,
                                                     G_N_ELEMENTS(action_fields));
        if (!valid_fields)
            return encode_response(
                id, NULL,
                move_action
                    ? "window.action move requires integer x and y and accepts an optional string "
                      "window"
                : resize_action
                    ? "window.action resize requires integer width and height and accepts an "
                      "optional string window"
                : workspace_action
                    ? "window.action workspace requires one workspace selector and accepts an "
                      "optional string window"
                : monitor_action
                    ? "window.action monitor requires an integer monitor index and accepts an "
                      "optional string window"
                    : "window.action accepts only an optional string window");

        JsonNode* window_node = json_object_get_member(arguments, "window");
        if (window_node && (!JSON_NODE_HOLDS_VALUE(window_node) ||
                            json_node_get_value_type(window_node) != G_TYPE_STRING))
            return encode_response(id, NULL, "window.action window must be a stable ID string");

        JsonNode* x_node = move_action ? json_object_get_member(arguments, "x") : NULL;
        JsonNode* y_node = move_action ? json_object_get_member(arguments, "y") : NULL;
        if (move_action && (!x_node || !JSON_NODE_HOLDS_VALUE(x_node) ||
                            (json_node_get_value_type(x_node) != G_TYPE_INT &&
                             json_node_get_value_type(x_node) != G_TYPE_INT64) ||
                            !y_node || !JSON_NODE_HOLDS_VALUE(y_node) ||
                            (json_node_get_value_type(y_node) != G_TYPE_INT &&
                             json_node_get_value_type(y_node) != G_TYPE_INT64)))
            return encode_response(id, NULL, "window.action move requires integer x and y");

        JsonNode* width_node = resize_action ? json_object_get_member(arguments, "width") : NULL;
        JsonNode* height_node = resize_action ? json_object_get_member(arguments, "height") : NULL;
        if (resize_action && (!width_node || !JSON_NODE_HOLDS_VALUE(width_node) ||
                              (json_node_get_value_type(width_node) != G_TYPE_INT &&
                               json_node_get_value_type(width_node) != G_TYPE_INT64) ||
                              !height_node || !JSON_NODE_HOLDS_VALUE(height_node) ||
                              (json_node_get_value_type(height_node) != G_TYPE_INT &&
                               json_node_get_value_type(height_node) != G_TYPE_INT64)))
            return encode_response(id, NULL,
                                   "window.action resize requires integer width and height");

        JsonNode* workspace_node =
            workspace_action ? json_object_get_member(arguments, "workspace") : NULL;
        JsonObject* workspace_selector = workspace_node && JSON_NODE_HOLDS_OBJECT(workspace_node)
                                             ? json_node_get_object(workspace_node)
                                             : NULL;
        const char* workspace_selector_fields[] = {"id", "number"};
        JsonNode* workspace_id_node =
            workspace_selector ? json_object_get_member(workspace_selector, "id") : NULL;
        JsonNode* workspace_number_node =
            workspace_selector ? json_object_get_member(workspace_selector, "number") : NULL;
        gboolean workspace_id = workspace_id_node && JSON_NODE_HOLDS_VALUE(workspace_id_node) &&
                                json_node_get_value_type(workspace_id_node) == G_TYPE_STRING &&
                                json_node_get_string(workspace_id_node) &&
                                *json_node_get_string(workspace_id_node);
        gboolean workspace_number =
            workspace_number_node && JSON_NODE_HOLDS_VALUE(workspace_number_node) &&
            (json_node_get_value_type(workspace_number_node) == G_TYPE_INT ||
             json_node_get_value_type(workspace_number_node) == G_TYPE_INT64) &&
            json_node_get_int(workspace_number_node) > 0;
        if (workspace_action &&
            (!workspace_selector || json_object_get_size(workspace_selector) != 1 ||
             !native_socket_has_exact_fields(workspace_selector, workspace_selector_fields,
                                             G_N_ELEMENTS(workspace_selector_fields)) ||
             workspace_id == workspace_number))
            return encode_response(id, NULL, "window.action workspace requires {id} or {number}");

        JsonNode* monitor_node =
            monitor_action ? json_object_get_member(arguments, "monitor") : NULL;
        if (monitor_action &&
            (!monitor_node || !JSON_NODE_HOLDS_VALUE(monitor_node) ||
             (json_node_get_value_type(monitor_node) != G_TYPE_INT &&
              json_node_get_value_type(monitor_node) != G_TYPE_INT64) ||
             json_node_get_int(monitor_node) < 0 || json_node_get_int(monitor_node) > G_MAXINT))
            return encode_response(id, NULL,
                                   "window.action monitor requires a nonnegative integer index");

        if ((workspace_action || monitor_action) && client->api_minor < 63)
            return encode_response(id, NULL,
                                   "window.action workspace and monitor require API version 1.63");

        static const char* const native_actions[] = {
            "above",   "unabove",  "stick",      "unstick",    "close",        "minimize",
            "restore", "maximize", "unmaximize", "fullscreen", "unfullscreen", NULL};
        if (!g_strv_contains(native_actions, action) &&
            !(resize_action && client->api_minor >= 61) &&
            !(move_action && client->api_minor >= 62) &&
            !((workspace_action || monitor_action) && client->api_minor >= 63))
            return encode_response(
                id, NULL, "unsupported window.action; use a typed window operation when available");

        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");

        const char* target = window_node ? json_node_get_string(window_node) : "active";
        g_autofree char* active_window_id = NULL;
        if (g_str_equal(target, "active")) {
            MetaWindow* active_window = meta_display_get_focus_window(client->control->display);
            if (!active_window)
                return encode_response(id, NULL, "there is no active window");
            active_window_id = native_window_id(active_window);
            target = active_window_id;
        }

        const char* lua_method = method;
        gboolean enabled = TRUE;
        if (g_str_equal(action, "above") || g_str_equal(action, "unabove")) {
            lua_method = "window.set_above";
            enabled = g_str_equal(action, "above");
        } else if (g_str_equal(action, "stick") || g_str_equal(action, "unstick")) {
            lua_method = "window.set_sticky";
            enabled = g_str_equal(action, "stick");
        } else if (g_str_equal(action, "maximize") || g_str_equal(action, "unmaximize")) {
            lua_method = "window.set_maximized";
            enabled = g_str_equal(action, "maximize");
        } else if (g_str_equal(action, "fullscreen") || g_str_equal(action, "unfullscreen")) {
            lua_method = "window.set_fullscreen";
            enabled = g_str_equal(action, "fullscreen");
        } else if (resize_action) {
            lua_method = "window.resize";
        } else if (move_action) {
            lua_method = "window.move";
        } else {
            lua_method = g_str_equal(action, "close")      ? "window.close"
                         : g_str_equal(action, "minimize") ? "window.minimize"
                                                           : "window.unminimize";
        }

        GVariantBuilder typed_arguments;
        g_variant_builder_init(&typed_arguments, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&typed_arguments, "{sv}", "id", g_variant_new_string(target));
        if (g_str_has_prefix(lua_method, "window.set_"))
            g_variant_builder_add(&typed_arguments, "{sv}", "enabled",
                                  g_variant_new_boolean(enabled));
        if (resize_action) {
            g_variant_builder_add(&typed_arguments, "{sv}", "width",
                                  g_variant_new_int64(json_node_get_int(width_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "height",
                                  g_variant_new_int64(json_node_get_int(height_node)));
        }
        if (move_action) {
            g_variant_builder_add(&typed_arguments, "{sv}", "x",
                                  g_variant_new_int64(json_node_get_int(x_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "y",
                                  g_variant_new_int64(json_node_get_int(y_node)));
        }
        if (workspace_action) {
            GVariantBuilder selector;
            g_variant_builder_init(&selector, G_VARIANT_TYPE_VARDICT);
            if (workspace_id)
                g_variant_builder_add(
                    &selector, "{sv}", "id",
                    g_variant_new_string(json_node_get_string(workspace_id_node)));
            else
                g_variant_builder_add(
                    &selector, "{sv}", "number",
                    g_variant_new_int64(json_node_get_int(workspace_number_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "workspace",
                                  g_variant_builder_end(&selector));
            lua_method = "window.move_to_workspace";
        }
        if (monitor_action) {
            g_autoptr(GError) monitor_error = NULL;
            g_autoptr(JsonNode) snapshot =
                monitor_snapshot_json(client->control, FALSE, &monitor_error);
            g_autofree char* monitor_id =
                native_monitor_id_for_index(snapshot, (int)json_node_get_int(monitor_node));
            if (!monitor_id)
                return encode_response(
                    id, NULL, monitor_error ? monitor_error->message : "monitor not found");
            g_variant_builder_add(&typed_arguments, "{sv}", "monitor",
                                  g_variant_new_string(monitor_id));
            lua_method = "window.move_to_monitor";
        }
        g_autoptr(GVariant) operation_arguments =
            g_variant_ref_sink(g_variant_builder_end(&typed_arguments));
        return queue_runtime_api_request_internal(client, id, lua_method, operation_arguments,
                                                  "call", action, target);
    }
    if (g_str_equal(method, "window.match")) {
        JsonObject* arguments_object = arguments_node && JSON_NODE_HOLDS_OBJECT(arguments_node)
                                           ? json_node_get_object(arguments_node)
                                           : NULL;
        if ((arguments_node && !arguments_object) ||
            (arguments_object && json_object_get_size(arguments_object) > 1))
            return encode_response(id, NULL,
                                   "window.match accepts only an optional string window selector");
        if (arguments_object) {
            GList* keys = json_object_get_members(arguments_object);
            gboolean valid =
                !keys || (keys->next == NULL && g_str_equal((const char*)keys->data, "window"));
            g_list_free(keys);
            JsonNode* window_node = json_object_get_member(arguments_object, "window");
            if (!valid || (window_node && (!JSON_NODE_HOLDS_VALUE(window_node) ||
                                           json_node_get_value_type(window_node) != G_TYPE_STRING)))
                return encode_response(
                    id, NULL, "window.match accepts only an optional string window selector");
        }
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "window.match arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "read");
    }
    if (g_str_equal(method, "window.focus")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        g_autoptr(GVariant) result = native_socket_focus_window(client, arguments, &error);
        if (!result)
            return encode_response(id, NULL, error ? error->message : "window.focus was denied");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "window.begin_move") || g_str_equal(method, "window.begin_resize")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        gboolean menu_attempt = arguments && json_object_has_member(arguments, "menu_context");
        if (menu_attempt && client->api_minor < 30)
            return encode_response(id, NULL, "WM menu operations require API version 1.30");
        g_autoptr(GVariant) result =
            menu_attempt ? native_socket_begin_menu_window_grab(client, method, arguments, &error)
                         : native_socket_begin_window_grab(client, method, arguments, &error);
        if (!result)
            return encode_response(
                id, NULL, error ? error->message : "interactive window operation was denied");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "shortcut.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "shortcut.list does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, "shortcuts.list", read_arguments, "read");
    }
    if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        return g_str_equal(method, "shortcut.bind")
                   ? dynamic_shortcut_bind(client, id, arguments)
                   : dynamic_shortcut_unbind(client, id, arguments);
    }
    if (g_str_equal(method, "shortcut.session.end")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        return dynamic_shortcut_request_end_session(client, id, arguments);
    }
    if (g_str_has_prefix(method, "launch.")) {
        client->track_launches = TRUE;
        client->launch_api_minor = client->api_minor;
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) launch_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (g_str_equal(method, "launch.status")) {
            if (!launch_arguments)
                return encode_response(id, NULL, "launch arguments are invalid");
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, launch_arguments, "read");
        }
        if (client->api_minor >= 50 &&
            (g_str_equal(method, "launch.begin") || g_str_equal(method, "launch.end"))) {
            if (!launch_arguments)
                return encode_response(id, NULL, "launch arguments are invalid");
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, launch_arguments, "call");
        }
        g_autoptr(GVariant) result = gnoblin_native_control_dispatch_launch(
            client->control->display, method, launch_arguments, &error);
        if (!result)
            return encode_response(id, NULL,
                                   error ? error->message : "launch feedback unavailable");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "input.devices")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "input.devices does not accept arguments");
        if (client->api_minor >= 4) {
            client->track_input_devices = TRUE;
            client->input_devices_api_minor = client->api_minor;
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "input source reads do not accept arguments");
        client->track_input_sources = TRUE;
        client->input_sources_api_minor = client->api_minor;
        publish_input_source_changes(client->control, client->control->state_revision);
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "shortcut.actions")) {
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!read_arguments)
            return encode_response(id, NULL, "shortcut.actions arguments are invalid");
        return queue_runtime_api_request(client, id, "shortcuts.actions", read_arguments, "read");
    }
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = arguments_node
                                        ? variant_from_json(arguments_node)
                                        : g_variant_ref_sink(g_variant_builder_end(&empty));
    if (!client->control->supervised_runtime)
        return encode_response(id, NULL, "Lua supervisor is not connected");
    return queue_runtime_api_request(client, id, method, arguments, "call");
}

static void write_next(Client* client);

static void write_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingWrite* pending = user_data;
    g_autoptr(GError) error = NULL;
    gboolean written =
        g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, NULL, &error);
    Client* client = pending->client;
    client->writing = FALSE;
    client->pending_bytes -= strlen(pending->response);
    g_free(pending->response);
    g_free(pending);
    if (!written || client->closing) {
        client_close(client);
        return;
    }
    if (client->close_after_response && g_queue_is_empty(client->outgoing)) {
        client_close(client);
        return;
    }
    write_next(client);
    if (!client->writing)
        process_buffer(client);
}

static void write_next(Client* client) {
    if (client->writing || client->closing || g_queue_is_empty(client->outgoing))
        return;
    PendingWrite* pending = g_new0(PendingWrite, 1);
    pending->client = client;
    pending->response = g_queue_pop_head(client->outgoing);
    client->writing = TRUE;
    g_output_stream_write_all_async(g_io_stream_get_output_stream(G_IO_STREAM(client->connection)),
                                    pending->response, strlen(pending->response),
                                    G_PRIORITY_DEFAULT, NULL, write_done, pending);
}

static void send_response(Client* client, char* response) {
    if (client->closing) {
        g_free(response);
        return;
    }
    gsize length = strlen(response);
    if (client->pending_bytes + length > MAX_PENDING_BYTES) {
        g_free(response);
        client_close(client);
        return;
    }
    client->pending_bytes += length;
    g_queue_push_tail(client->outgoing, response);
    write_next(client);
}

static gboolean publish_windows(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    guint64 revision = control->state_revision;
    control->publish_id = 0;
    native_apply_all_window_rules(control);
    g_autoptr(GError) workspace_error = NULL;
    g_autoptr(JsonNode) workspace_json = workspace_snapshot_json(control, &workspace_error);
    if (!workspace_json)
        g_warning("gnoblin-native-control: cannot publish workspaces: %s",
                  workspace_error ? workspace_error->message : "workspace listing unavailable");
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) snapshot_json = window_snapshot_json(control, TRUE, &error);
    g_autoptr(GError) monitor_error = NULL;
    g_autoptr(JsonNode) monitor_json = monitor_snapshot_json(control, TRUE, &monitor_error);
    g_autoptr(GError) layer_error = NULL;
    g_autoptr(JsonNode) layer_json = layer_snapshot_json(control, TRUE, &layer_error);
    g_autoptr(GVariant) capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, revision);
    g_autoptr(GVariant) input_devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", input_devices, revision);
    publish_input_source_changes(control, revision);
    g_autoptr(JsonNode) input_devices_json = json_from_variant(input_devices);
    gboolean monitor_changed = FALSE;
    publish_input_device_changes(control, input_devices_json, revision);
    if (workspace_json)
        publish_workspace_changes(control, workspace_json, snapshot_json, revision);
    if (snapshot_json)
        publish_window_changes(control, snapshot_json, revision);
    else
        g_warning("gnoblin-native-control: cannot publish windows: %s",
                  error ? error->message : "window listing unavailable");
    if (monitor_json)
        monitor_changed = publish_monitor_changes(control, monitor_json, revision);
    else
        g_warning("gnoblin-native-control: cannot publish monitors: %s",
                  monitor_error ? monitor_error->message : "monitor listing unavailable");
    if (!layer_json)
        g_warning("gnoblin-native-control: cannot publish layers: %s",
                  layer_error ? layer_error->message : "layer listing unavailable");
    g_autofree char* snapshot = NULL;
    if (snapshot_json) {
        JsonObject* object = json_node_get_object(snapshot_json);
        json_object_set_string_member(object, "event", "windows");
        json_object_set_int_member(object, "revision", revision);
        g_autofree char* encoded = json_to_string(snapshot_json, FALSE);
        snapshot = g_strconcat(encoded, "\n", NULL);
    } else {
        snapshot = encode_response("", NULL, error ? error->message : "window listing unavailable");
    }
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->track_windows)
            send_response(client, g_strdup(snapshot));
        if (client->track_monitors && monitor_json && monitor_changed) {
            JsonObject* object = json_node_get_object(monitor_json);
            json_object_set_string_member(object, "event", "monitors");
            g_autofree char* encoded = json_to_string(monitor_json, FALSE);
            send_response(client, g_strconcat(encoded, "\n", NULL));
            json_object_remove_member(object, "event");
        }
    }
    g_list_free(clients);
    return G_SOURCE_REMOVE;
}

static void schedule_windows(GnoblinNativeControl* control) {
    if (!control->publish_id) {
        control->state_revision++;
        control->publish_id = g_idle_add(publish_windows, control);
    }
}

static void window_changed(MetaWindow* window, gpointer user_data) {
    native_apply_window_rules(user_data, window);
    schedule_windows(user_data);
}

static gboolean mutter_signal_value_supported(GType type) {
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    return type == G_TYPE_BOOLEAN || type == G_TYPE_CHAR || type == G_TYPE_UCHAR ||
           type == G_TYPE_INT || type == G_TYPE_UINT || type == G_TYPE_LONG ||
           type == G_TYPE_ULONG || type == G_TYPE_INT64 || type == G_TYPE_UINT64 ||
           type == G_TYPE_FLOAT || type == G_TYPE_DOUBLE || type == G_TYPE_STRING ||
           G_TYPE_IS_ENUM(type) || G_TYPE_IS_FLAGS(type);
}

static void add_window_signal_identity(GVariantBuilder* payload, MetaWindow* window);

static gboolean mutter_signal_argument_supported(GType type, const char* source) {
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    return mutter_signal_value_supported(type) ||
           (g_str_equal(source, "workspace") && type == META_TYPE_WINDOW);
}

static GVariant* mutter_signal_value_to_variant(const GValue* value) {
    GType type = G_VALUE_TYPE(value);
    type &= ~G_SIGNAL_TYPE_STATIC_SCOPE;
    if (type == G_TYPE_BOOLEAN)
        return g_variant_new_boolean(g_value_get_boolean(value));
    if (type == G_TYPE_CHAR)
        return g_variant_new_int64(g_value_get_schar(value));
    if (type == G_TYPE_UCHAR)
        return g_variant_new_uint64(g_value_get_uchar(value));
    if (type == G_TYPE_INT)
        return g_variant_new_int64(g_value_get_int(value));
    if (type == G_TYPE_UINT)
        return g_variant_new_uint64(g_value_get_uint(value));
    if (type == G_TYPE_LONG)
        return g_variant_new_int64(g_value_get_long(value));
    if (type == G_TYPE_ULONG)
        return g_variant_new_uint64(g_value_get_ulong(value));
    if (type == G_TYPE_INT64)
        return g_variant_new_int64(g_value_get_int64(value));
    if (type == G_TYPE_UINT64)
        return g_variant_new_uint64(g_value_get_uint64(value));
    if (type == G_TYPE_FLOAT)
        return g_variant_new_double(g_value_get_float(value));
    if (type == G_TYPE_DOUBLE)
        return g_variant_new_double(g_value_get_double(value));
    if (type == G_TYPE_STRING) {
        const char* string = g_value_get_string(value);
        if (string && !g_utf8_validate(string, -1, NULL))
            return NULL;
        return g_variant_new_string(string ? string : "");
    }
    if (G_TYPE_IS_ENUM(type))
        return g_variant_new_int64(g_value_get_enum(value));
    if (G_TYPE_IS_FLAGS(type))
        return g_variant_new_uint64(g_value_get_flags(value));
    return NULL;
}

static GVariant* mutter_signal_argument_to_variant(const GValue* value, const char* source) {
    GType type = G_VALUE_TYPE(value) & ~G_SIGNAL_TYPE_STATIC_SCOPE;
    if (g_str_equal(source, "workspace") && type == META_TYPE_WINDOW) {
        MetaWindow* window = g_value_get_object(value);
        if (!window || !META_IS_WINDOW(window))
            return NULL;
        GVariantBuilder identity;
        g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
        add_window_signal_identity(&identity, window);
        return g_variant_builder_end(&identity);
    }
    return mutter_signal_value_to_variant(value);
}

static void native_mutter_signal_watch_free(gpointer data, GClosure* closure) {
    (void)closure;
    NativeMutterSignalWatch* watch = data;
    g_free(watch->event);
    g_free(watch->source);
    g_free(watch->signal);
    g_free(watch);
}

static void add_window_signal_identity(GVariantBuilder* payload, MetaWindow* window) {
    const char* app_id = meta_window_get_gtk_application_id(window);
    const char* title = meta_window_get_title(window);
    g_variant_builder_add(payload, "{sv}", "window_id",
                          g_variant_new_uint32(meta_window_get_stable_sequence(window)));
    g_variant_builder_add(payload, "{sv}", "app_id", g_variant_new_string(app_id ? app_id : ""));
    g_variant_builder_add(payload, "{sv}", "window_title",
                          g_variant_new_string(title ? title : ""));
}

static void native_mutter_signal_marshal(GClosure* closure, GValue* return_value,
                                         guint n_param_values, const GValue* param_values,
                                         gpointer invocation_hint, gpointer marshal_data) {
    (void)return_value;
    (void)invocation_hint;
    (void)marshal_data;
    NativeMutterSignalWatch* watch = closure->data;
    GSignalQuery query = {0};
    g_signal_query(watch->signal_id, &query);
    if (!watch->control || n_param_values != query.n_params + 1)
        return;
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "source", g_variant_new_string(watch->source));
    g_variant_builder_add(&payload, "{sv}", "signal", g_variant_new_string(watch->signal));
    if (g_str_equal(watch->source, "window") && G_VALUE_HOLDS_OBJECT(&param_values[0])) {
        GObject* object = g_value_get_object(&param_values[0]);
        if (object && META_IS_WINDOW(object))
            add_window_signal_identity(&payload, META_WINDOW(object));
    }
    if (g_str_equal(watch->source, "workspace") && G_VALUE_HOLDS_OBJECT(&param_values[0])) {
        GObject* object = g_value_get_object(&param_values[0]);
        if (object && META_IS_WORKSPACE(object))
            g_variant_builder_add(
                &payload, "{sv}", "workspace_index",
                g_variant_new_int32(meta_workspace_index(META_WORKSPACE(object))));
    }
    for (guint i = 0; i < query.n_params; i++) {
        GType type = query.param_types[i] & ~G_SIGNAL_TYPE_STATIC_SCOPE;
        g_autoptr(GVariant) argument =
            mutter_signal_argument_to_variant(&param_values[i + 1], watch->source);
        if (!argument) {
            g_variant_builder_clear(&payload);
            return;
        }
        g_autofree char* name = g_strdup_printf("arg%u", i);
        g_autofree char* type_name = g_strdup_printf("arg%u_type", i);
        g_variant_builder_add(&payload, "{sv}", name, argument);
        g_variant_builder_add(&payload, "{sv}", type_name, g_variant_new_string(g_type_name(type)));
    }
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    native_runtime_dispatch_event(watch->control, watch->event, event_payload);
}

static void clear_object_signal_watches(GObject* object, GArray* handler_ids) {
    if (!object || !handler_ids)
        return;
    for (guint i = 0; i < handler_ids->len; i++) {
        gulong handler_id = g_array_index(handler_ids, gulong, i);
        if (g_signal_handler_is_connected(object, handler_id))
            g_signal_handler_disconnect(object, handler_id);
    }
    g_array_set_size(handler_ids, 0);
}

static void refresh_object_signal_watches(GnoblinNativeControl* control, GObject* object,
                                          const char* source, GArray* handler_ids) {
    clear_object_signal_watches(object, handler_ids);
    if (!control || !object || !source || !handler_ids || !control->runtime_event_subscriptions)
        return;
    g_autofree char* prefix = g_strdup_printf("mutter.%s.", source);
    GHashTableIter iter;
    gpointer key;
    g_autoptr(GHashTable) connected = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_hash_table_iter_init(&iter, control->runtime_event_subscriptions);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        const char* event = key;
        if (g_str_equal(event, "*")) {
            if (g_str_equal(source, "display"))
                event = "mutter.display.restacked";
            else if (g_str_equal(source, "window"))
                event = "mutter.window.position-changed";
            else if (g_str_equal(source, "workspace-manager"))
                event = "mutter.workspace-manager.active-workspace-changed";
            else if (g_str_equal(source, "backend"))
                event = "mutter.backend.keymap-changed";
            else if (g_str_equal(source, "monitor-manager"))
                event = "mutter.monitor-manager.monitors-changed";
            else if (g_str_equal(source, "cursor-tracker"))
                event = "mutter.cursor-tracker.cursor-changed";
            else
                continue;
        }
        if (!g_str_has_prefix(event, prefix) || !event[strlen(prefix)])
            continue;
        if (g_hash_table_contains(connected, event))
            continue;
        g_hash_table_add(connected, g_strdup(event));
        const char* signal = event + strlen(prefix);
        guint signal_id = 0;
        GQuark detail = 0;
        if (!g_signal_parse_name(signal, G_OBJECT_TYPE(object), &signal_id, &detail, FALSE)) {
            g_warning("gnoblin: cannot subscribe to unknown Mutter event %s", event);
            continue;
        }
        GSignalQuery query = {0};
        g_signal_query(signal_id, &query);
        GType return_type = query.return_type & ~G_SIGNAL_TYPE_STATIC_SCOPE;
        gboolean supported = return_type == G_TYPE_NONE;
        for (guint i = 0; supported && i < query.n_params; i++)
            supported = mutter_signal_argument_supported(query.param_types[i], source);
        if (!supported) {
            g_warning("gnoblin: Mutter event %s has a signal signature that Lua cannot represent",
                      event);
            continue;
        }
        NativeMutterSignalWatch* watch = g_new0(NativeMutterSignalWatch, 1);
        watch->control = control;
        watch->event = g_strdup(event);
        watch->source = g_strdup(source);
        watch->signal = g_strdup(signal);
        watch->signal_id = signal_id;
        GClosure* closure = g_closure_new_simple(sizeof(GClosure), watch);
        g_closure_set_marshal(closure, native_mutter_signal_marshal);
        g_closure_add_finalize_notifier(closure, watch, native_mutter_signal_watch_free);
        gulong handler_id =
            g_signal_connect_closure_by_id(object, signal_id, detail, closure, TRUE);
        g_array_append_val(handler_ids, handler_id);
    }
}

static void clear_display_signal_watches(GnoblinNativeControl* control) {
    if (control && control->display)
        clear_object_signal_watches(G_OBJECT(control->display),
                                    control->display_signal_handler_ids);
}

static void refresh_display_signal_watches(GnoblinNativeControl* control) {
    if (control && control->display)
        refresh_object_signal_watches(control, G_OBJECT(control->display), "display",
                                      control->display_signal_handler_ids);
}

static void clear_workspace_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->workspace_manager)
        clear_object_signal_watches(G_OBJECT(control->workspace_manager),
                                    control->workspace_manager_signal_handler_ids);
}

static void refresh_workspace_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->workspace_manager)
        refresh_object_signal_watches(control, G_OBJECT(control->workspace_manager),
                                      "workspace-manager",
                                      control->workspace_manager_signal_handler_ids);
}

static void clear_all_workspace_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->workspace_signal_handler_ids)
        return;
    GHashTableIter iter;
    gpointer workspace;
    gpointer handler_ids;
    g_hash_table_iter_init(&iter, control->workspace_signal_handler_ids);
    while (g_hash_table_iter_next(&iter, &workspace, &handler_ids))
        clear_object_signal_watches(G_OBJECT(workspace), handler_ids);
    g_hash_table_remove_all(control->workspace_signal_handler_ids);
}

static gboolean workspace_signal_subscribed(GnoblinNativeControl* control) {
    if (!control || !control->runtime_event_subscriptions)
        return FALSE;
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, control->runtime_event_subscriptions);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        const char* event = key;
        if (g_str_equal(event, "*") || g_str_has_prefix(event, "mutter.workspace."))
            return TRUE;
    }
    return FALSE;
}

static void refresh_all_workspace_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->workspace_manager || !control->workspace_signal_handler_ids)
        return;
    clear_all_workspace_signal_watches(control);
    if (!workspace_signal_subscribed(control))
        return;
    int count = meta_workspace_manager_get_n_workspaces(control->workspace_manager);
    for (int index = 0; index < count; index++) {
        MetaWorkspace* workspace =
            meta_workspace_manager_get_workspace_by_index(control->workspace_manager, index);
        if (!workspace)
            continue;
        GArray* handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
        g_hash_table_insert(control->workspace_signal_handler_ids, g_object_ref(workspace),
                            handler_ids);
        refresh_object_signal_watches(control, G_OBJECT(workspace), "workspace", handler_ids);
    }
}

static void clear_backend_signal_watches(GnoblinNativeControl* control) {
    if (control && control->backend)
        clear_object_signal_watches(G_OBJECT(control->backend),
                                    control->backend_signal_handler_ids);
}

static void refresh_backend_signal_watches(GnoblinNativeControl* control) {
    if (control && control->backend)
        refresh_object_signal_watches(control, G_OBJECT(control->backend), "backend",
                                      control->backend_signal_handler_ids);
}

static void clear_monitor_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->monitor_manager)
        clear_object_signal_watches(G_OBJECT(control->monitor_manager),
                                    control->monitor_manager_signal_handler_ids);
}

static void refresh_monitor_manager_signal_watches(GnoblinNativeControl* control) {
    if (control && control->monitor_manager)
        refresh_object_signal_watches(control, G_OBJECT(control->monitor_manager),
                                      "monitor-manager",
                                      control->monitor_manager_signal_handler_ids);
}

static void clear_cursor_tracker_signal_watches(GnoblinNativeControl* control) {
    if (control && control->cursor_tracker)
        clear_object_signal_watches(G_OBJECT(control->cursor_tracker),
                                    control->cursor_tracker_signal_handler_ids);
}

static void refresh_cursor_tracker_signal_watches(GnoblinNativeControl* control) {
    if (control && control->cursor_tracker)
        refresh_object_signal_watches(control, G_OBJECT(control->cursor_tracker), "cursor-tracker",
                                      control->cursor_tracker_signal_handler_ids);
}

static void refresh_window_signal_watches(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control || !control->window_signal_handler_ids || !window)
        return;
    GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
    if (handler_ids)
        refresh_object_signal_watches(control, G_OBJECT(window), "window", handler_ids);
}

static void refresh_all_window_signal_watches(GnoblinNativeControl* control) {
    if (!control || !control->window_signal_handler_ids)
        return;
    GHashTableIter iter;
    gpointer window;
    gpointer handler_ids;
    g_hash_table_iter_init(&iter, control->window_signal_handler_ids);
    while (g_hash_table_iter_next(&iter, &window, &handler_ids))
        refresh_object_signal_watches(control, G_OBJECT(window), "window", handler_ids);
}

static void window_workspace_changed(MetaWindow* window, gpointer user_data) {
    native_apply_window_rules(user_data, window);
    schedule_windows(user_data);
}

static void window_notified(GObject* window, GParamSpec* property, gpointer user_data) {
    native_apply_window_rules(user_data, META_WINDOW(window));
    schedule_windows(user_data);
}

static void window_unmanaged(MetaWindow* window, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
    clear_object_signal_watches(G_OBJECT(window), handler_ids);
    g_hash_table_remove(control->window_signal_handler_ids, window);
    g_signal_handlers_disconnect_by_data(window, control);
    g_hash_table_remove(control->snap_restore_frames, window);
    g_hash_table_remove(control->windows, window);
    schedule_windows(control);
}

static void track_window(GnoblinNativeControl* control, MetaWindow* window) {
    if (g_hash_table_contains(control->windows, window))
        return;
    g_hash_table_add(control->windows, g_object_ref(window));
    GArray* handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    g_hash_table_insert(control->window_signal_handler_ids, window, handler_ids);
    g_signal_connect(window, "notify", G_CALLBACK(window_notified), control);
    g_signal_connect(window, "position-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "size-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "workspace-changed", G_CALLBACK(window_workspace_changed), control);
    g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), control);
    refresh_window_signal_watches(control, window);
    native_apply_window_rules(control, window);
    schedule_windows(control);
}

void gnoblin_native_control_track_layer_window(MetaDisplay* display, MetaWindow* window) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !window)
        return;
    track_window(control, window);
}

static void window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data) {
    track_window(user_data, window);
}

static void display_notified(GObject* display, GParamSpec* property, gpointer user_data) {
    if (g_str_equal(property->name, "focus-window"))
        native_apply_all_window_rules(user_data);
    schedule_windows(user_data);
}

static void display_restacked(MetaDisplay* display, gpointer user_data) {
    schedule_windows(user_data);
}

static void workspace_manager_changed(MetaWorkspaceManager* manager, gpointer user_data) {
    schedule_windows(user_data);
}

static void monitor_manager_changed(MetaMonitorManager* manager, gpointer user_data) {
    schedule_windows(user_data);
}

static void backend_keymap_changed(MetaBackend* backend, gpointer user_data) {
    (void)backend;
    schedule_windows(user_data);
}

static void backend_keymap_layout_group_changed(MetaBackend* backend, guint group,
                                                gpointer user_data) {
    (void)backend;
    (void)group;
    schedule_windows(user_data);
}

static void input_device_added(ClutterSeat* seat, ClutterInputDevice* device, gpointer user_data) {
    (void)seat;
    (void)device;
    schedule_windows(user_data);
}

static void input_device_removed(ClutterSeat* seat, ClutterInputDevice* device,
                                 gpointer user_data) {
    (void)seat;
    GnoblinNativeControl* control = user_data;
    g_hash_table_remove(control->input_device_ids, device);
    schedule_windows(control);
}

static void workspace_manager_index_changed(MetaWorkspaceManager* manager, int index,
                                            gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    refresh_all_workspace_signal_watches(control);
    workspace_manager_changed(manager, control);
}

static void read_request(Client* client);

static void process_buffer(Client* client) {
    if (client->closing || client->writing)
        return;
    const char* newline = memchr(client->request->str, '\n', client->request->len);
    if (newline) {
        gsize line_length = newline - client->request->str;
        if (line_length > MAX_REQUEST_BYTES) {
            client->close_after_response = TRUE;
            send_response(client, encode_response("", NULL, "request is too large"));
            return;
        }
        char* response = handle_request(client, client->request->str, line_length);
        g_string_erase(client->request, 0, line_length + 1);
        if (!response)
            return;
        send_response(client, response);
        return;
    }
    if (client->request->len > MAX_REQUEST_BYTES) {
        client->close_after_response = TRUE;
        send_response(client, encode_response("", NULL, "request is too large"));
        return;
    }
    if (!client->reading)
        read_request(client);
}

static void read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    Client* client = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes =
        g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, &error);
    client->reading = FALSE;
    if (client->closing || !client->control || !bytes || g_bytes_get_size(bytes) == 0) {
        client_close(client);
        return;
    }
    gsize length;
    const char* data = g_bytes_get_data(bytes, &length);
    g_string_append_len(client->request, data, length);
    process_buffer(client);
}

static void read_request(Client* client) {
    client->reading = TRUE;
    g_input_stream_read_bytes_async(g_io_stream_get_input_stream(G_IO_STREAM(client->connection)),
                                    4096, G_PRIORITY_DEFAULT, NULL, read_done, client);
}

static gboolean client_connected(GSocketService* service, GSocketConnection* connection,
                                 GObject* source_object, gpointer user_data) {
    static const char* methods[] = {
        "capabilities.list",
        "focus.history",
        "focus.policy",
        "input.devices",
        "input.sources",
        "input.current_source",
        "input.orientation_lock",
        "input.select",
        "input.set_orientation_lock",
        "input.text_target",
        "input.insert_text",
        "launch.status",
        "launch.begin",
        "launch.end",
        "launches.list",
        "launches.snapshot",
        "shortcuts.list",
        "shortcuts.actions",
        "layer.list",
        "layers.list",
        "monitor.list",
        "monitors.list",
        "settings",
        "shortcut.actions",
        "shortcut.bind",
        "shortcut.capture",
        "shortcut.list",
        "shortcut.session.end",
        "shortcut.unbind",
        "permissions.list",
        "permissions.policy",
        "permissions.check",
        "privacy.state",
        "privacy.stop_sharing",
        "privacy.stop_recording",
        "location.authorize_app",
        "animation.list",
        "animation.get",
        "animation.surfaces",
        "animation.inspect",
        "animation.preview",
        "animation.seek",
        "animation.step",
        "animation.play",
        "animation.pause",
        "animation.stop",
        "grant.list",
        "grant.revoke",
        "portals.grants",
        "window.list",
        "windows.list",
        "window.match",
        "window.thumbnail",
        "window.snap.offer",
        "window.action",
        "window.close",
        "window.minimize",
        "window.toggle_minimize",
        "window.unminimize",
        "window.restore",
        "window.restore_or_minimize",
        "window.set_maximized",
        "window.set_fullscreen",
        "window.set_above",
        "window.set_sticky",
        "window.move",
        "window.resize",
        "window.move_to_workspace",
        "window.move_to_monitor",
        "window.focus",
        "window.snap_context",
        "window.snap",
        "window.begin_move",
        "window.begin_resize",
        "workspace.list",
        "workspaces.list",
        "workspace.create",
        "workspace.rename",
        "workspace.remove",
        "workspace.switch",
        "workspace.next",
        "workspace.previous",
        "workspace.move_active",
        "workspace.move_window",
        "runtime.reload_config",
        "runtime.status",
        "session.lock",
        "session.activity",
        "session.logout",
        "session.status",
        "appearance.color_scheme",
        "layer.animation_policy",
        "version",
        NULL,
    };
    GnoblinNativeControl* control = user_data;
    Client* client = g_new0(Client, 1);
    client->control = control;
    client->client_id = ++control->next_client_id;
    if (client->client_id == 0)
        client->client_id = ++control->next_client_id;
    client->connection = g_object_ref(connection);
    g_autoptr(GError) credentials_error = NULL;
    g_autoptr(GCredentials) credentials =
        g_socket_get_credentials(g_socket_connection_get_socket(connection), &credentials_error);
    if (credentials)
        client->peer_pid = g_credentials_get_unix_pid(credentials, &credentials_error);
    client->request = g_string_new(NULL);
    client->outgoing = g_queue_new();
    client->focus_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    client->menu_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    client->pending_grant_operations = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_hash_table_add(control->clients, client);
    g_autoptr(JsonNode) greeting = json_node_new(JSON_NODE_OBJECT);
    JsonObject* hello = json_object_new();
    json_node_take_object(greeting, hello);
    json_object_set_string_member(hello, "event", "hello");
    json_object_set_int_member(hello, "version", GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_object_set_int_member(hello, "api_major", GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_object_set_int_member(hello, "api_minor", GNOBLIN_NATIVE_CONTROL_API_MINOR);
    json_object_set_int_member(hello, "state_revision", control->state_revision);
    json_object_set_string_member(
        hello, "revision_scope",
        "window-workspace-monitor-layer-input-device-source-capability-state");
    json_object_set_string_member(hello, "revision_semantics", "monotonic-per-session");
    JsonArray* method_array = json_array_new();
    for (guint i = 0; methods[i]; i++)
        json_array_add_string_element(method_array, methods[i]);
    json_object_set_array_member(hello, "methods", method_array);
    JsonArray* event_array = json_array_new();
    for (guint i = 0; native_socket_events[i]; i++)
        json_array_add_string_element(event_array, native_socket_events[i]);
    json_object_set_array_member(hello, "events", event_array);
    JsonArray* capability_array = json_array_new();
    for (guint i = 0; i < G_N_ELEMENTS(native_capabilities); i++)
        json_array_add_string_element(capability_array, native_capabilities[i].id);
    json_object_set_array_member(hello, "capabilities", capability_array);
    json_object_set_array_member(hello, "features", json_array_new());
    g_autofree char* greeting_text = json_to_string(greeting, FALSE);
    send_response(client, g_strconcat(greeting_text, "\n", NULL));
    return TRUE;
}

GVariant* gnoblin_native_control_receive_runtime_config(int runtime_fd, guint64* settings_revision,
                                                        GError** error) {
    g_return_val_if_fail(runtime_fd >= 0, NULL);
    g_return_val_if_fail(error == NULL || *error == NULL, NULL);
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gint64 deadline = g_get_monotonic_time() + 10 * G_USEC_PER_SEC;
    for (;;) {
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        if (!gnoblin_runtime_reader_receive(reader, runtime_fd, &packet, &available, error))
            return NULL;
        if (available) {
            if (packet.type != GNOBLIN_RUNTIME_PACKET_CONFIG || packet.request_id != 0) {
                gnoblin_runtime_packet_clear(&packet);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "expected initial Gnoblin runtime configuration");
                return NULL;
            }
            guint32 version = 0;
            GVariant* document =
                g_variant_lookup_value(packet.payload, "document", G_VARIANT_TYPE_VARDICT);
            guint64 revision = 0;
            guint64 runtime_generation = 0;
            gboolean valid =
                document && g_variant_lookup(packet.payload, "document_version", "u", &version) &&
                version == 1 &&
                g_variant_lookup(packet.payload, "settings_revision", "t", &revision) &&
                revision > 0 &&
                g_variant_lookup(packet.payload, "runtime_generation", "t", &runtime_generation) &&
                runtime_generation > 0 && g_variant_is_normal_form(document);
            gnoblin_runtime_packet_clear(&packet);
            if (!valid) {
                g_clear_pointer(&document, g_variant_unref);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "initial Gnoblin runtime configuration is invalid");
                return NULL;
            }
            if (settings_revision)
                *settings_revision = revision;
            bootstrap_runtime_generation = runtime_generation;
            if (!bootstrap_runtime_cache)
                bootstrap_runtime_cache = gnoblin_runtime_cache_new();
            if (!gnoblin_runtime_cache_replace(bootstrap_runtime_cache, document, revision,
                                               error)) {
                g_variant_unref(document);
                return NULL;
            }
            return document;
        }
        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "timed out waiting for Gnoblin runtime configuration");
            return NULL;
        }
        struct pollfd pfd = {.fd = runtime_fd, .events = POLLIN};
        int result;
        do {
            result = poll(&pfd, 1, (int)MIN(remaining / 1000, G_MAXINT));
        } while (result < 0 && errno == EINTR);
        if (result == 0)
            continue;
        if (result < 0 || (pfd.revents & (POLLERR | POLLHUP | POLLNVAL))) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                        "Gnoblin runtime channel failed during startup: %s",
                        result < 0 ? g_strerror(errno) : "peer closed the channel");
            return NULL;
        }
    }
}

static gboolean native_runtime_write_ready(gint fd, GIOCondition condition, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        control->runtime_write_source_id = 0;
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    g_autoptr(GError) error = NULL;
    if (!gnoblin_runtime_writer_flush(control->runtime_writer, fd, &error)) {
        if (g_error_matches(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK))
            return G_SOURCE_CONTINUE;
        control->runtime_write_source_id = 0;
        g_warning("gnoblin-native-control: private runtime write failed: %s", error->message);
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    control->runtime_write_source_id = 0;
    return G_SOURCE_REMOVE;
}

static gboolean native_runtime_send(GnoblinNativeControl* control, GnoblinRuntimePacketType type,
                                    guint64 request_id, GVariant* payload, GError** error) {
    if (control->runtime_worker_suspended && type != GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED &&
        type != GNOBLIN_RUNTIME_PACKET_ERROR) {
        /* Runtime output is intentionally discarded while no Lua worker owns
         * the session. Resume publishes fresh snapshots after HELLO. */
        return TRUE;
    }
    if (!control->supervised_runtime || control->runtime_fd < 0 || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin runtime channel is unavailable");
        return FALSE;
    }
    if (!gnoblin_runtime_writer_queue(control->runtime_writer, type, request_id, payload, error))
        return FALSE;
    g_autoptr(GError) flush_error = NULL;
    if (!gnoblin_runtime_writer_flush(control->runtime_writer, control->runtime_fd, &flush_error)) {
        if (g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            if (!control->runtime_write_source_id)
                control->runtime_write_source_id =
                    g_unix_fd_add(control->runtime_fd, G_IO_OUT | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                                  native_runtime_write_ready, control);
            return TRUE;
        }
        g_propagate_error(error, g_steal_pointer(&flush_error));
        return FALSE;
    }
    return TRUE;
}

static gboolean native_runtime_send_event_packet(GnoblinNativeControl* control, const char* event,
                                                 GVariant* packet) {
    if (control->runtime_worker_suspended)
        return TRUE;
    if (control->supervised_runtime && !control->runtime_hello_sent &&
        control->pending_runtime_events) {
        g_queue_push_tail(control->pending_runtime_events, g_variant_ref(packet));
        return TRUE;
    }
    g_autoptr(GError) error = NULL;
    if (native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_EVENT, 0, packet, &error))
        return TRUE;
    native_runtime_abort(control);
    g_warning("gnoblin-native-control: could not queue runtime event %s: %s", event,
              error ? error->message : "unknown error");
    return FALSE;
}

static gboolean native_runtime_dispatch_event(GnoblinNativeControl* control, const char* event,
                                              GVariant* payload) {
    if (!control || control->stopping || !event || !*event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    GVariantBuilder event_packet;
    g_variant_builder_init(&event_packet, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&event_packet, "{sv}", "event", g_variant_new_string(event));
    gboolean trusted_binding = g_str_equal(event, "gnoblin.shortcut.binding-activated");
    gboolean session_key = g_str_equal(event, "gnoblin.shortcut.session.key");
    if (g_str_equal(event, "gnoblin.shortcut.activated") || trusted_binding || session_key) {
        gboolean first = FALSE;
        if (trusted_binding && (!g_variant_lookup(payload, "first", "b", &first) || !first))
            return FALSE;
        g_autoptr(GVariant) context_handle =
            g_variant_lookup_value(payload, "focus_context_handle", NULL);
        g_autoptr(GVariant) context_generation =
            g_variant_lookup_value(payload, "focus_context_generation", NULL);
        g_autoptr(GVariant) context_expiry =
            g_variant_lookup_value(payload, "focus_context_expires_at_us", NULL);
        gboolean has_any_context = context_handle || context_generation || context_expiry;
        gboolean has_all_context = context_handle && context_generation && context_expiry;
        if (!has_any_context && (trusted_binding || session_key)) {
            g_variant_builder_add(&event_packet, "{sv}", "payload", payload);
            g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&event_packet));
            return native_runtime_send_event_packet(control, event, packet);
        }
        if (!has_all_context)
            return FALSE;
        guint64 handle = 0;
        guint64 generation = 0;
        gint64 expires_at_us = 0;
        if (!g_variant_lookup(payload, "focus_context_handle", "t", &handle) || !handle ||
            !g_variant_lookup(payload, "focus_context_generation", "t", &generation) ||
            !generation ||
            !g_variant_lookup(payload, "focus_context_expires_at_us", "x", &expires_at_us) ||
            expires_at_us <= g_get_monotonic_time())
            return FALSE;
        GVariantBuilder clean_payload;
        GVariantIter fields;
        const char* field_name;
        GVariant* field_value;
        g_variant_builder_init(&clean_payload, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&fields, payload);
        while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
            g_autoptr(GVariant) value = field_value;
            if (g_str_equal(field_name, "focus_context_handle") ||
                g_str_equal(field_name, "focus_context_generation") ||
                g_str_equal(field_name, "focus_context_expires_at_us"))
                continue;
            g_variant_builder_add(&clean_payload, "{sv}", field_name, value);
        }
        g_variant_builder_add(&event_packet, "{sv}", "payload",
                              g_variant_builder_end(&clean_payload));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_handle",
                              g_variant_new_uint64(handle));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_generation",
                              g_variant_new_uint64(generation));
        g_variant_builder_add(&event_packet, "{sv}", "focus_context_expires_at_us",
                              g_variant_new_int64(expires_at_us));
    } else {
        g_variant_builder_add(&event_packet, "{sv}", "payload", payload);
    }
    g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&event_packet));
    return native_runtime_send_event_packet(control, event, packet);
}

static GVariant* native_runtime_shortcut_operation(GnoblinNativeControl* control,
                                                   const char* method, GVariant* arguments,
                                                   gint64 operation_id, GError** error) {
    const char* id = NULL;
    if (g_str_equal(method, "shortcut.session.end")) {
        gint64 signed_session_id = 0;
        if (!g_variant_lookup(arguments, "id", "&s", &id) || !dynamic_shortcut_id_valid(id) ||
            !g_variant_lookup(arguments, "session_id", "x", &signed_session_id) ||
            signed_session_id <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.session.end requires a valid id and positive session_id");
            return NULL;
        }
        guint64 session_id = (guint64)signed_session_id;
        guint64 generation = native_config_generation(control);
        g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
        NativeDynamicShortcut* shortcut = find_runtime_dynamic_shortcut(control, owner_id, id);
        if (!shortcut || !shortcut->active || shortcut->session_id != session_id) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "shortcut session is no longer active for this Lua binding");
            return NULL;
        }
        dynamic_shortcut_end_session(control, shortcut, "cancelled");
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&result, "{sv}", "session_id", g_variant_new_uint64(session_id));
        g_variant_builder_add(&result, "{sv}", "ended", g_variant_new_boolean(TRUE));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    if (g_str_equal(method, "shortcut.unbind")) {
        if (!g_variant_lookup(arguments, "id", "&s", &id) || !dynamic_shortcut_id_valid(id)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.unbind requires a valid id");
            return NULL;
        }
        guint64 generation = native_config_generation(control);
        g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
        NativeDynamicShortcut* shortcut = find_runtime_dynamic_shortcut(control, owner_id, id);
        if (!shortcut) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "shortcut ID is not registered by this Lua runtime");
            return NULL;
        }
        remove_dynamic_shortcut(control, shortcut);
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&result, "{sv}", "unbound", g_variant_new_boolean(TRUE));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }

    const char* accelerator = NULL;
    const char* hold = "none";
    const char* trigger = "press";
    const char* mode = "passive";
    gboolean capture_input = FALSE;
    if (!g_variant_lookup(arguments, "id", "&s", &id) || !dynamic_shortcut_id_valid(id) ||
        !g_variant_lookup(arguments, "accelerator", "&s", &accelerator) ||
        !dynamic_shortcut_accelerator_valid(accelerator)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.bind requires a valid id and accelerator");
        return NULL;
    }
    g_variant_lookup(arguments, "hold", "&s", &hold);
    g_variant_lookup(arguments, "trigger", "&s", &trigger);
    g_variant_lookup(arguments, "mode", "&s", &mode);
    g_variant_lookup(arguments, "capture_input", "b", &capture_input);
    if (!((g_str_equal(hold, "none")) || g_str_equal(hold, "super") ||
          g_str_equal(hold, "control") || g_str_equal(hold, "alt")) ||
        (!g_str_equal(trigger, "press") && !g_str_equal(trigger, "release")) ||
        (!g_str_equal(mode, "passive") && !g_str_equal(mode, "modal")) ||
        (g_str_equal(mode, "modal") && g_str_equal(hold, "none")) ||
        (capture_input && !g_str_equal(accelerator, "Super")) ||
        (g_str_equal(accelerator, "Super") &&
         (!capture_input || !g_str_equal(trigger, "release") || !g_str_equal(hold, "none")))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcut.bind has an unsupported hold, trigger, mode, or "
                            "capture_input combination");
        return NULL;
    }
    guint64 generation = native_config_generation(control);
    g_autofree char* owner_id = g_strdup_printf("lua:%" G_GUINT64_FORMAT, generation);
    if (!generation || find_runtime_dynamic_shortcut(control, owner_id, id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                            "shortcut ID already exists in this Lua runtime");
        return NULL;
    }
    NativeDynamicShortcut* shortcut = NULL;
    if (g_str_equal(accelerator, "Super")) {
        guint64 session_id = (guint64)operation_id;
        shortcut = arm_bare_super_shortcut(control, id, owner_id, NULL, session_id, error);
        if (!shortcut)
            return NULL;
    } else {
        guint action = meta_display_grab_accelerator(control->display, accelerator, 0);
        if (action == META_KEYBINDING_ACTION_NONE) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                "shortcut.bind accelerator is already claimed");
            return NULL;
        }
        shortcut = g_new0(NativeDynamicShortcut, 1);
        shortcut->control = control;
        shortcut->id = g_strdup(id);
        shortcut->accelerator = g_strdup(accelerator);
        shortcut->owner_id = g_strdup(owner_id);
        shortcut->action = action;
        shortcut->trigger_release = g_str_equal(trigger, "release");
        shortcut->modal = g_str_equal(mode, "modal");
        shortcut->capture_input = capture_input;
        shortcut->hold_mask = g_str_equal(hold, "super")     ? CLUTTER_SUPER_MASK
                              : g_str_equal(hold, "control") ? CLUTTER_CONTROL_MASK
                              : g_str_equal(hold, "alt")     ? CLUTTER_MOD1_MASK
                                                             : 0;
        shortcut->armed = TRUE;
        g_hash_table_insert(control->dynamic_shortcuts, GUINT_TO_POINTER(action), shortcut);
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "accelerator", g_variant_new_string(accelerator));
    g_variant_builder_add(&result, "{sv}", "hold", g_variant_new_string(hold));
    g_variant_builder_add(&result, "{sv}", "trigger", g_variant_new_string(trigger));
    g_variant_builder_add(&result, "{sv}", "mode", g_variant_new_string(mode));
    g_variant_builder_add(&result, "{sv}", "capture_input", g_variant_new_boolean(capture_input));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static gboolean native_runtime_flush_state_snapshots(GnoblinNativeControl* control,
                                                     GError** error) {
    while (control->pending_runtime_states && !g_queue_is_empty(control->pending_runtime_states)) {
        GVariant* payload = g_queue_pop_head(control->pending_runtime_states);
        gboolean sent =
            native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_STATE, 0, payload, error);
        g_variant_unref(payload);
        if (!sent)
            return FALSE;
    }
    return TRUE;
}

static gboolean native_runtime_flush_pending_events(GnoblinNativeControl* control, GError** error) {
    while (control->pending_runtime_events && !g_queue_is_empty(control->pending_runtime_events)) {
        GVariant* packet = g_queue_pop_head(control->pending_runtime_events);
        gboolean sent =
            native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_EVENT, 0, packet, error);
        g_variant_unref(packet);
        if (!sent)
            return FALSE;
    }
    return TRUE;
}

static void native_runtime_clear_queue(GQueue* queue) {
    if (queue)
        g_queue_clear_full(queue, (GDestroyNotify)g_variant_unref);
}

static void native_runtime_fail_pending_requests(GnoblinNativeControl* control,
                                                 const char* reason) {
    if (!control || !control->pending_runtime_requests)
        return;
    g_autoptr(GPtrArray) pending_requests =
        g_ptr_array_new_with_free_func(pending_runtime_request_free);
    GHashTableIter iter;
    gpointer key;
    gpointer value;
    g_hash_table_iter_init(&iter, control->pending_runtime_requests);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        g_ptr_array_add(pending_requests, value);
        g_hash_table_iter_steal(&iter);
        g_free(key);
    }
    for (guint i = 0; i < pending_requests->len; i++) {
        PendingRuntimeRequest* pending = g_ptr_array_index(pending_requests, i);
        Client* client = pending->client;
        if (!client->closing) {
            send_response(client,
                          encode_response(pending->request_id, NULL,
                                          reason ? reason : "Lua worker stopped before replying"));
            process_buffer(client);
            if (client->pending_deferred_requests > 0)
                client->pending_deferred_requests--;
        } else if (client->pending_deferred_requests > 0) {
            client->pending_deferred_requests--;
        }
        client_maybe_free(client);
    }
    if (control->runtime_operation_ids && control->runtime_cancelled_operation_ids) {
        GHashTableIter operation_iter;
        gpointer operation_key;
        g_hash_table_iter_init(&operation_iter, control->runtime_operation_ids);
        while (g_hash_table_iter_next(&operation_iter, &operation_key, NULL)) {
            guint64* cancelled_key = g_new(guint64, 1);
            *cancelled_key = *(guint64*)operation_key;
            g_hash_table_add(control->runtime_cancelled_operation_ids, cancelled_key);
        }
        g_hash_table_remove_all(control->runtime_operation_ids);
    }
}

static gboolean native_runtime_send_worker_suspended(GnoblinNativeControl* control,
                                                     GError** error) {
    guint64 revision = gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation",
                          g_variant_new_uint64(control->runtime_generation));
    g_variant_builder_add(&builder, "{sv}", "operation_id_watermark",
                          g_variant_new_uint64(control->last_runtime_operation_id));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    return native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED, 0, payload, error);
}

static void native_runtime_suspend_worker(GnoblinNativeControl* control) {
    if (!control || control->runtime_worker_suspended)
        return;
    control->runtime_worker_suspended = TRUE;
    control->runtime_hello_sent = FALSE;
    /* The host discards output through the acknowledgement. Drop queued
     * fragments first so an old message cannot continue after SUSPENDED. */
    gnoblin_runtime_writer_reset(control->runtime_writer);
    gnoblin_runtime_reader_reset(control->runtime_reader);
    native_runtime_clear_queue(control->pending_runtime_states);
    native_runtime_clear_queue(control->pending_runtime_events);
    native_runtime_fail_pending_requests(control, "Lua worker restarted before replying");
    native_cancel_window_drags(control, "runtime_restarted");
    /* Do not carry an in-progress gesture from one Lua worker generation to
     * the next. The replacement will receive only complete new gestures. */
    gnoblin_touchpad_router_reset(control->touchpad_router);
    clear_runtime_dynamic_shortcuts(control, "runtime_restarted");
    stop_native_shortcut_capture(control, TRUE, FALSE, NULL, "unavailable",
                                 "shortcut capture cancelled because the Lua worker restarted");
    revoke_focus_contexts(control);
    revoke_menu_contexts(control);
    revoke_text_targets(control);
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    g_autoptr(GError) error = NULL;
    if (!native_runtime_send_worker_suspended(control, &error)) {
        g_warning("gnoblin-native-control: could not acknowledge Lua worker suspension: %s",
                  error ? error->message : "unknown error");
        native_runtime_abort(control);
    }
}

static GHashTable* native_runtime_event_subscriptions_from_payload(GVariant* payload,
                                                                   GError** error) {
    g_autoptr(GVariant) events =
        g_variant_lookup_value(payload, "runtime_events", G_VARIANT_TYPE("as"));
    if (!events) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "runtime configuration is missing Lua event subscriptions");
        return NULL;
    }
    g_auto(GStrv) event_names = g_variant_dup_strv(events, NULL);
    GHashTable* subscriptions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (guint i = 0; event_names[i]; i++) {
        if (!event_names[i][0] || strlen(event_names[i]) > 128 ||
            !g_utf8_validate(event_names[i], -1, NULL) ||
            g_hash_table_contains(subscriptions, event_names[i])) {
            g_hash_table_unref(subscriptions);
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "runtime configuration contains an invalid Lua event name");
            return NULL;
        }
        g_hash_table_add(subscriptions, g_strdup(event_names[i]));
    }
    return subscriptions;
}

static gboolean native_runtime_event_subscriptions_equal(GHashTable* left, GHashTable* right) {
    if (g_hash_table_size(left) != g_hash_table_size(right))
        return FALSE;
    GHashTableIter iter;
    gpointer event_name;
    g_hash_table_iter_init(&iter, left);
    while (g_hash_table_iter_next(&iter, &event_name, NULL)) {
        if (!g_hash_table_contains(right, event_name))
            return FALSE;
    }
    return TRUE;
}

static gboolean native_runtime_resume_snapshot_matches(GnoblinNativeControl* control,
                                                       GVariant* payload) {
    g_autoptr(GVariant) document =
        g_variant_lookup_value(payload, "document", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) current_document =
        gnoblin_runtime_cache_get_document(control->runtime_cache);
    g_autoptr(GError) events_error = NULL;
    g_autoptr(GHashTable) subscriptions =
        native_runtime_event_subscriptions_from_payload(payload, &events_error);
    guint64 revision = 0;
    guint64 generation = 0;
    guint64 operation_id_watermark = 0;
    if (!subscriptions || !control->runtime_event_subscriptions)
        return FALSE;
    return native_runtime_event_subscriptions_equal(subscriptions,
                                                    control->runtime_event_subscriptions) &&
           document && current_document &&
           g_variant_lookup(payload, "settings_revision", "t", &revision) &&
           g_variant_lookup(payload, "runtime_generation", "t", &generation) &&
           g_variant_lookup(payload, "operation_id_watermark", "t", &operation_id_watermark) &&
           revision == gnoblin_runtime_cache_get_settings_revision(control->runtime_cache) &&
           generation == control->runtime_generation &&
           operation_id_watermark == control->last_runtime_operation_id &&
           runtime_config_values_equal(document, current_document);
}

static gboolean native_runtime_reject_resume(GnoblinNativeControl* control, const char* message,
                                             GError** error) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "resume_rejected", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&builder, "{sv}", "message", g_variant_new_string(message));
    g_variant_builder_add(
        &builder, "{sv}", "settings_revision",
        g_variant_new_uint64(gnoblin_runtime_cache_get_settings_revision(control->runtime_cache)));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation",
                          g_variant_new_uint64(control->runtime_generation));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    return native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_ERROR, 0, payload, error);
}

static gboolean native_runtime_republish_full_state(GnoblinNativeControl* control, GError** error) {
    guint64 revision = control->state_revision;
    g_autoptr(GError) snapshot_error = NULL;
    /* These helpers publish normalized Lua snapshots themselves. Re-publishing
     * their native JSON results here would overwrite derived Lua fields. */
    {
        g_autoptr(JsonNode) windows = window_snapshot_json(control, TRUE, &snapshot_error);
        (void)windows;
        if (!windows)
            g_warning("gnoblin-native-control: cannot restore window snapshot: %s",
                      snapshot_error ? snapshot_error->message : "window listing unavailable");
    }

    g_clear_error(&snapshot_error);
    {
        g_autoptr(JsonNode) workspaces = workspace_snapshot_json(control, &snapshot_error);
        (void)workspaces;
        if (!workspaces)
            g_warning("gnoblin-native-control: cannot restore workspace snapshot: %s",
                      snapshot_error ? snapshot_error->message : "workspace listing unavailable");
    }

    g_clear_error(&snapshot_error);
    g_autoptr(JsonNode) monitors = monitor_snapshot_json(control, TRUE, &snapshot_error);
    if (!monitors)
        g_warning("gnoblin-native-control: cannot restore monitor snapshot: %s",
                  snapshot_error ? snapshot_error->message : "monitor listing unavailable");
    g_autoptr(GVariant) monitor_value = monitors ? variant_from_json(monitors) : NULL;
    native_publish_runtime_snapshot(control, "monitors", monitor_value, revision);

    g_clear_error(&snapshot_error);
    g_autoptr(JsonNode) layers = layer_snapshot_json(control, TRUE, &snapshot_error);
    if (!layers)
        g_warning("gnoblin-native-control: cannot restore layer snapshot: %s",
                  snapshot_error ? snapshot_error->message : "layer listing unavailable");
    g_autoptr(GVariant) layer_value = layers ? variant_from_json(layers) : NULL;
    native_publish_runtime_snapshot(control, "layers", layer_value, revision);

    g_autoptr(GVariant) capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, revision);
    g_autoptr(GVariant) devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", devices, revision);
    g_autoptr(GVariant) sources = input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", sources, revision);
    g_autoptr(GVariant) appearance = appearance_snapshot_new(control);
    native_publish_runtime_snapshot(control, "appearance", appearance,
                                    control->appearance_revision);
    g_autoptr(GVariant) orientation_lock =
        native_orientation_lock_snapshot(control, control->orientation_lock_revision);
    native_publish_runtime_snapshot(control, "input-orientation-lock", orientation_lock,
                                    control->orientation_lock_revision);
    g_autoptr(GVariant) shortcuts = native_shortcut_snapshot(control);
    native_publish_runtime_snapshot(control, "shortcuts", shortcuts, revision);
    publish_privacy_snapshot(control, FALSE);
    if (control->portal_grant_snapshot)
        native_publish_runtime_snapshot(control, "portal-grants", control->portal_grant_snapshot,
                                        control->portal_grant_revision);
    if (control->session_activity_snapshot)
        native_publish_runtime_snapshot(control, "session-activity",
                                        control->session_activity_snapshot,
                                        control->session_activity_revision);
    g_autoptr(GVariant) lock_snapshot =
        native_session_lock_snapshot(control->wayland_compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", lock_snapshot,
                                    control->session_lock_revision);
    update_launch_snapshot(control);
    return native_runtime_flush_state_snapshots(control, error);
}

static GVariant* native_set_orientation_lock(GnoblinNativeControl* control, GVariant* arguments,
                                             GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) != 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock requires only value");
        return NULL;
    }
    if (!control->orientation_manager) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "orientation lock is unavailable on this backend");
        return NULL;
    }

    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "value", NULL);
    if (!value) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock requires value");
        return NULL;
    }

    if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
        control->orientation_lock_runtime_override = TRUE;
        control->orientation_lock_notifications_suppressed = TRUE;
        meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                        g_variant_get_boolean(value));
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
               g_str_equal(g_variant_get_string(value, NULL), "inherit")) {
        control->orientation_lock_runtime_override = FALSE;
        control->orientation_lock_notifications_suppressed = TRUE;
        if (control->orientation_lock_configured)
            meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                            control->orientation_lock_config_value);
        else
            meta_orientation_manager_clear_orientation_lock_override(control->orientation_manager);
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.set_orientation_lock value must be boolean or 'inherit'");
        return NULL;
    }

    control->orientation_lock_notifications_suppressed = FALSE;
    native_orientation_lock_publish(control);
    return g_variant_ref(control->orientation_lock_snapshot);
}

static gboolean native_runtime_send_config_result(GnoblinNativeControl* control,
                                                  guint64 transaction_id, gboolean accepted,
                                                  guint64 revision, guint64 generation,
                                                  const char* message, GError** error) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "accepted", g_variant_new_boolean(accepted));
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation", g_variant_new_uint64(generation));
    if (message && *message)
        g_variant_builder_add(&builder, "{sv}", "error", g_variant_new_string(message));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    return native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT, transaction_id,
                               payload, error);
}

static gboolean native_runtime_handle_operation(GnoblinNativeControl* control,
                                                GnoblinRuntimePacket* packet, GError** error) {
    if (!control || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin compositor control is stopping");
        return FALSE;
    }
    gint64 operation_id = 0;
    guint64 client_id = 0;
    const char* method = NULL;
    g_autoptr(GVariant) arguments =
        g_variant_lookup_value(packet->payload, "arguments", G_VARIANT_TYPE_VARDICT);
    if (packet->request_id > G_MAXINT64 ||
        !g_variant_lookup(packet->payload, "request_id", "x", &operation_id) || operation_id <= 0 ||
        (guint64)operation_id != packet->request_id ||
        !g_variant_lookup(packet->payload, "method", "&s", &method) || !method || !*method ||
        !arguments) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "supervisor sent a malformed native operation");
        return FALSE;
    }
    if ((guint64)operation_id <= control->last_runtime_operation_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "supervisor reused a native operation ID");
        return FALSE;
    }
    control->last_runtime_operation_id = (guint64)operation_id;
    g_autoptr(GError) operation_error = NULL;
    g_autoptr(GVariant) result = NULL;
    gboolean pending = FALSE;
    if (g_str_equal(method, "window.thumbnail") || g_str_equal(method, "location.authorize_app"))
        g_variant_lookup(packet->payload, "client_id", "t", &client_id);
    if (g_str_equal(method, "window.snap") || g_str_equal(method, "window.snap_context")) {
        guint64 operation_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &operation_generation) ||
            !operation_generation || operation_generation != control->runtime_generation) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "snap operation belongs to a stale runtime generation");
        }
    }
    if (operation_error) {
        /* Return a normal operation failure; do not tear down the private channel. */
    } else if (g_str_equal(method, "window.snap.offer")) {
        guint64 owner_generation = 0;
        if (!g_variant_lookup(packet->payload, "runtime_generation", "t", &owner_generation) ||
            !native_runtime_window_snap_offer(control, arguments, owner_generation,
                                              &operation_error)) {
            if (!operation_error)
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "window.snap.offer requires the active runtime generation");
        } else {
            GVariantBuilder accepted;
            g_variant_builder_init(&accepted, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&accepted, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&accepted));
        }
    } else if (g_str_equal(method, "window.restore_or_minimize")) {
        result = gnoblin_native_control_restore_or_minimize_window(control->display, arguments,
                                                                   &operation_error);
    } else if (g_str_equal(method, "window.snap")) {
        result = gnoblin_native_control_commit_snap_context(control->display, arguments,
                                                            &operation_error);
    } else if (g_str_equal(method, "window.focus") || g_str_equal(method, "window.snap_context") ||
               g_str_equal(method, "window.begin_move") ||
               g_str_equal(method, "window.begin_resize") ||
               g_str_equal(method, "input.text_target")) {
        guint64 menu_handle = 0;
        guint64 menu_generation = 0;
        gboolean has_menu_handle =
            g_variant_lookup(arguments, "_menu_context_handle", "t", &menu_handle);
        gboolean has_menu_generation =
            g_variant_lookup(arguments, "_menu_context_generation", "t", &menu_generation);
        if ((g_str_equal(method, "window.begin_move") ||
             g_str_equal(method, "window.begin_resize")) &&
            (has_menu_handle || has_menu_generation)) {
            GVariantBuilder clean;
            GVariantIter iter;
            const char* key;
            GVariant* value;
            g_variant_builder_init(&clean, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&iter, arguments);
            while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
                g_autoptr(GVariant) field = value;
                if (g_str_equal(key, "_menu_context_handle") ||
                    g_str_equal(key, "_menu_context_generation"))
                    continue;
                g_variant_builder_add(&clean, "{sv}", key, field);
            }
            g_autoptr(GVariant) menu_arguments = g_variant_ref_sink(g_variant_builder_end(&clean));
            if (!has_menu_handle || !menu_handle || !has_menu_generation || !menu_generation) {
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "operation has malformed WM menu capability metadata");
            } else {
                result = gnoblin_native_control_begin_menu_window_grab(
                    control->display, method, menu_arguments, menu_handle, menu_generation, 0,
                    &operation_error);
            }
        } else {
            guint64 handle = 0;
            guint64 generation = 0;
            gint64 expires_at_us = 0;
            if (!g_variant_lookup(packet->payload, "focus_context_handle", "t", &handle) ||
                !handle ||
                !g_variant_lookup(packet->payload, "focus_context_generation", "t", &generation) ||
                !generation ||
                !g_variant_lookup(packet->payload, "focus_context_expires_at_us", "x",
                                  &expires_at_us) ||
                expires_at_us <= g_get_monotonic_time()) {
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "operation requires a live shortcut focus context");
            } else if (g_str_equal(method, "input.text_target")) {
                result = gnoblin_native_control_create_text_target(
                    control->display, handle, generation, expires_at_us, &operation_error);
            } else if (g_str_equal(method, "window.snap_context")) {
                result = gnoblin_native_control_create_snap_context(
                    control->display, handle, generation, expires_at_us, &operation_error);
            } else if (g_str_equal(method, "window.focus")) {
                result = gnoblin_native_control_focus_window(control->display, arguments, handle,
                                                             generation, &operation_error);
            } else {
                result = gnoblin_native_control_begin_window_grab(
                    control->display, method, arguments, handle, generation, &operation_error);
            }
        }
    } else if (g_str_equal(method, "privacy.stop_sharing") ||
               g_str_equal(method, "privacy.stop_recording")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "privacy stop methods take no arguments");
        } else {
            result = native_stop_privacy_sessions(
                control, g_str_equal(method, "privacy.stop_recording"), &operation_error);
        }
    } else if (g_str_equal(method, "location.authorize_app")) {
        result =
            native_location_authorize_operation(control, arguments, client_id, &operation_error);
    } else if (g_str_equal(method, "input.set_orientation_lock")) {
        result = native_set_orientation_lock(control, arguments, &operation_error);
    } else if (g_str_equal(method, "session.lock")) {
        result = gnoblin_native_control_request_session_lock(control->display, arguments,
                                                             &operation_error);
    } else if (g_str_equal(method, "session.logout")) {
        if (g_variant_n_children(arguments) != 0) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "session.logout takes no arguments");
        } else {
            GVariantBuilder logout_result;
            g_variant_builder_init(&logout_result, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&logout_result, "{sv}", "accepted", g_variant_new_boolean(TRUE));
            result = g_variant_ref_sink(g_variant_builder_end(&logout_result));
        }
    } else if (g_str_equal(method, "input.insert_text")) {
        result = gnoblin_native_control_insert_text(control->display, arguments, &operation_error);
    } else if (g_str_equal(method, "shortcut.session.arm")) {
        const char* binding_id = NULL;
        const char* owner_id = NULL;
        const char* trigger = NULL;
        guint64 session_id = 0;
        guint32 modifiers = 0;
        gboolean capture_input = FALSE;
        if (!g_variant_lookup(arguments, "binding_id", "&s", &binding_id) ||
            !g_variant_lookup(arguments, "owner_id", "&s", &owner_id) ||
            !g_variant_lookup(arguments, "session_id", "t", &session_id) || !session_id ||
            !g_variant_lookup(arguments, "trigger", "&s", &trigger) ||
            !g_str_equal(trigger, "release") ||
            !g_variant_lookup(arguments, "modifiers", "u", &modifiers) || modifiers != 0 ||
            !g_variant_lookup(arguments, "capture_input", "b", &capture_input) || !capture_input) {
            g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.session.arm requires a bare Super release binding");
        } else {
            NativeDynamicShortcut* armed = arm_bare_super_shortcut(
                control, binding_id, owner_id, NULL, session_id, &operation_error);
            if (armed) {
                GVariantBuilder armed_result;
                g_variant_builder_init(&armed_result, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&armed_result, "{sv}", "armed", g_variant_new_boolean(TRUE));
                g_variant_builder_add(&armed_result, "{sv}", "session_id",
                                      g_variant_new_uint64(session_id));
                result = g_variant_ref_sink(g_variant_builder_end(&armed_result));
            }
        }
    } else if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind") ||
               g_str_equal(method, "shortcut.session.end")) {
        result = native_runtime_shortcut_operation(control, method, arguments, operation_id,
                                                   &operation_error);
    } else if (g_str_has_prefix(method, "launch.")) {
        result = gnoblin_native_control_dispatch_launch(control->display, method, arguments,
                                                        &operation_error);
    } else if (g_str_equal(method, "input.select") || g_str_equal(method, "input.select_source") ||
               g_str_equal(method, "shortcut.capture") || g_str_equal(method, "grant.list") ||
               g_str_equal(method, "grant.revoke")) {
        guint64* operation_key = g_new(guint64, 1);
        *operation_key = (guint64)operation_id;
        g_hash_table_add(control->runtime_operation_ids, operation_key);
        if (g_str_equal(method, "input.select") || g_str_equal(method, "input.select_source")) {
            pending = gnoblin_native_control_select_input_source(
                control->display, arguments, operation_id, method, &operation_error);
        } else if (g_str_equal(method, "shortcut.capture")) {
            pending = gnoblin_native_control_begin_shortcut_capture(control->display, arguments,
                                                                    operation_id, &operation_error);
        } else {
            pending = gnoblin_native_control_portal_grant_operation(
                control->display, method, arguments, operation_id, &operation_error);
        }
        if (pending)
            return TRUE;
        g_hash_table_remove(control->runtime_operation_ids, &operation_key[0]);
    } else if (g_str_equal(method, "window.thumbnail")) {
        pending = native_runtime_begin_thumbnail(control, operation_id, arguments, client_id,
                                                 &operation_error);
        if (pending)
            return TRUE;
    } else if (g_str_equal(method, "window.set_maximized")) {
        gboolean enabled = FALSE;
        const char* window_id = NULL;
        if (g_variant_lookup(arguments, "enabled", "b", &enabled) && enabled &&
            g_variant_lookup(arguments, "id", "&s", &window_id) && window_id) {
            MetaWindow* window = native_window_by_stable_id(control, window_id);
            if (window)
                result = meta_gnoblin_dispatch_native_api(control->display, method, arguments,
                                                          &operation_error);
            else
                g_set_error_literal(&operation_error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "window.set_maximized target is unavailable");
            if (result)
                g_hash_table_remove(control->snap_restore_frames, window);
        } else {
            result = meta_gnoblin_dispatch_native_api(control->display, method, arguments,
                                                      &operation_error);
        }
    } else {
        result =
            meta_gnoblin_dispatch_native_api(control->display, method, arguments, &operation_error);
    }
    GVariantBuilder completion;
    g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(operation_id));
    g_variant_builder_add(&completion, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(result != NULL));
    if (result)
        g_variant_builder_add(&completion, "{sv}", "value", result);
    else {
        GVariantBuilder error_record;
        g_variant_builder_init(&error_record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&error_record, "{sv}", "code",
                              g_variant_new_string(native_operation_error_code(operation_error)));
        g_variant_builder_add(&error_record, "{sv}", "message",
                              g_variant_new_string(operation_error ? operation_error->message
                                                                   : "native operation failed"));
        g_variant_builder_add(&completion, "{sv}", "error", g_variant_builder_end(&error_record));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&completion));
    if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_COMPLETION, (guint64)operation_id,
                             payload, error))
        return FALSE;

    g_autoptr(JsonNode) json_result = result ? json_from_variant(result) : NULL;
    dispatch_operation_completion_full(control, operation_id, method, result != NULL, json_result,
                                       native_operation_error_code(operation_error),
                                       operation_error ? operation_error->message : NULL, FALSE);
    return TRUE;
}

static gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (control->stopping) {
        control->runtime_read_source_id = 0;
        return G_SOURCE_REMOVE;
    }
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        control->runtime_read_source_id = 0;
        if (!control->stopping)
            g_warning("gnoblin-native-control: supervisor closed the private runtime channel");
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    for (;;) {
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        g_autoptr(GError) error = NULL;
        if (!gnoblin_runtime_reader_receive(control->runtime_reader, fd, &packet, &available,
                                            &error)) {
            g_warning("gnoblin-native-control: private runtime read failed: %s", error->message);
            native_runtime_abort(control);
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
        if (!available)
            return G_SOURCE_CONTINUE;
        gboolean handled = FALSE;
        if (control->runtime_worker_suspended &&
            packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED) {
            /* A replacement can die before Mutter receives its RESUME. The
             * host cannot tell whether RESUME reached us, so acknowledge a
             * repeated DISCONNECTED while already suspended. */
            if (packet.request_id != 0 || g_variant_n_children(packet.payload) != 0) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid worker disconnect signal");
            } else {
                handled = native_runtime_send_worker_suspended(control, &error);
            }
        } else if (control->runtime_worker_suspended &&
                   packet.type != GNOBLIN_RUNTIME_PACKET_WORKER_RESUME) {
            /* Discard stale frames left by the worker that was just reaped. */
            gnoblin_runtime_packet_clear(&packet);
            continue;
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED) {
            if (packet.request_id != 0 || g_variant_n_children(packet.payload) != 0) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid worker disconnect signal");
            } else {
                native_runtime_suspend_worker(control);
                handled = !control->stopping;
            }
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_WORKER_RESUME) {
            if (!control->runtime_worker_suspended) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "worker sent RESUME while no worker was suspended");
            } else if (packet.request_id != 0 ||
                       !native_runtime_resume_snapshot_matches(control, packet.payload)) {
                handled = native_runtime_reject_resume(
                    control,
                    "replacement worker configuration does not match Mutter's accepted state",
                    &error);
            } else {
                /* Also discard any gesture that began while the worker was
                 * absent, so it cannot cross into the resumed generation. */
                gnoblin_touchpad_router_reset(control->touchpad_router);
                control->runtime_worker_suspended = FALSE;
                GVariantBuilder hello;
                g_variant_builder_init(&hello, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&hello, "{sv}", "role", g_variant_new_string("compositor"));
                g_variant_builder_add(&hello, "{sv}", "document_version", g_variant_new_uint32(1));
                g_autoptr(GVariant) hello_payload =
                    g_variant_ref_sink(g_variant_builder_end(&hello));
                handled = native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_HELLO, 0,
                                              hello_payload, &error);
                if (handled) {
                    control->runtime_hello_sent = TRUE;
                    handled = native_runtime_republish_full_state(control, &error);
                }
            }
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_OPERATION) {
            handled = native_runtime_handle_operation(control, &packet, &error);
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_API_RESPONSE && packet.request_id > 0) {
            PendingRuntimeRequest* pending =
                g_hash_table_lookup(control->pending_runtime_requests, &packet.request_id);
            gboolean ok = FALSE;
            const char* message = NULL;
            g_autoptr(GVariant) result = g_variant_lookup_value(packet.payload, "result", NULL);
            if (!pending || !g_variant_lookup(packet.payload, "ok", "b", &ok) || (ok && !result)) {
                g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid API response");
            } else {
                if (ok && pending->legacy_window_action) {
                    if (!pending->legacy_window_id ||
                        !g_variant_is_of_type(result, G_VARIANT_TYPE_INT64) ||
                        g_variant_get_int64(result) <= 0) {
                        ok = FALSE;
                        message = "Lua returned an invalid window action operation";
                    } else {
                        GVariantBuilder legacy_result;
                        g_variant_builder_init(&legacy_result, G_VARIANT_TYPE_VARDICT);
                        g_variant_builder_add(&legacy_result, "{sv}", "ok",
                                              g_variant_new_boolean(TRUE));
                        g_variant_builder_add(&legacy_result, "{sv}", "pending",
                                              g_variant_new_boolean(TRUE));
                        g_variant_builder_add(&legacy_result, "{sv}", "window",
                                              g_variant_new_string(pending->legacy_window_id));
                        g_variant_builder_add(&legacy_result, "{sv}", "action",
                                              g_variant_new_string(pending->legacy_window_action));
                        g_clear_pointer(&result, g_variant_unref);
                        result = g_variant_ref_sink(g_variant_builder_end(&legacy_result));
                    }
                }
                if (!ok && !message)
                    g_variant_lookup(packet.payload, "error", "&s", &message);
                Client* client = pending->client;
                if (!client->closing) {
                    g_autoptr(JsonNode) json = result ? json_from_variant(result) : NULL;
                    send_response(
                        client,
                        encode_response(pending->request_id, json,
                                        ok ? NULL
                                           : (message ? message : "runtime API request failed")));
                    client->pending_deferred_requests--;
                    process_buffer(client);
                } else if (client->pending_deferred_requests > 0) {
                    client->pending_deferred_requests--;
                }
                g_hash_table_remove(control->pending_runtime_requests, &packet.request_id);
                client_maybe_free(client);
                handled = TRUE;
            }
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_CONFIG) {
            GVariant* document =
                g_variant_lookup_value(packet.payload, "document", G_VARIANT_TYPE_VARDICT);
            guint32 version = 0;
            guint64 revision = 0;
            guint64 runtime_generation = 0;
            guint64 previous_revision =
                gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
            guint64 previous_runtime_generation = control->runtime_generation;
            g_autoptr(GVariant) previous_document =
                gnoblin_runtime_cache_get_document(control->runtime_cache);
            g_autoptr(GVariant) previous_bootstrap_document =
                bootstrap_runtime_cache
                    ? gnoblin_runtime_cache_get_document(bootstrap_runtime_cache)
                    : NULL;
            guint64 previous_bootstrap_revision =
                bootstrap_runtime_cache
                    ? gnoblin_runtime_cache_get_settings_revision(bootstrap_runtime_cache)
                    : 0;
            g_autoptr(GError) config_error = NULL;
            g_autoptr(GHashTable) next_event_subscriptions =
                native_runtime_event_subscriptions_from_payload(packet.payload, &config_error);
            gboolean fields_valid =
                document && next_event_subscriptions &&
                g_variant_lookup(packet.payload, "document_version", "u", &version) &&
                version == 1 &&
                g_variant_lookup(packet.payload, "settings_revision", "t", &revision) &&
                g_variant_lookup(packet.payload, "runtime_generation", "t", &runtime_generation) &&
                runtime_generation > 0;
            gboolean candidate_valid =
                fields_valid && revision >= previous_revision &&
                runtime_generation >= control->runtime_generation &&
                (packet.request_id == 0 ? runtime_generation == control->runtime_generation
                                        : runtime_generation > control->runtime_generation);
            gboolean rollback_failed = FALSE;
            if (candidate_valid) {
                g_autoptr(GVariant) old_workspaces =
                    previous_document
                        ? g_variant_lookup_value(previous_document, "workspaces", NULL)
                        : NULL;
                g_autoptr(GVariant) new_workspaces =
                    g_variant_lookup_value(document, "workspaces", NULL);
                gboolean workspaces_changed =
                    (!old_workspaces != !new_workspaces) ||
                    (old_workspaces && new_workspaces &&
                     !runtime_config_values_equal(old_workspaces, new_workspaces));
                g_autoptr(GVariant) old_input_sources =
                    previous_document
                        ? g_variant_lookup_value(previous_document, "input-sources", NULL)
                        : NULL;
                g_autoptr(GVariant) new_input_sources =
                    g_variant_lookup_value(document, "input-sources", NULL);
                gboolean input_sources_changed =
                    (!old_input_sources != !new_input_sources) ||
                    (old_input_sources && new_input_sources &&
                     !runtime_config_values_equal(old_input_sources, new_input_sources));
                if (revision == previous_revision && previous_document &&
                    !runtime_config_values_equal(previous_document, document)) {
                    g_set_error_literal(
                        &config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                        "supervisor changed configuration without advancing its revision");
                } else {
                    handled = gnoblin_runtime_cache_replace(control->runtime_cache, document,
                                                            revision, &config_error);
                }
                if (handled && revision > previous_revision) {
                    MetaContext* context = meta_backend_get_context(control->backend);
                    gboolean supported =
                        runtime_reload_document_supported(previous_document, document);
                    gboolean apply_ok = supported;
                    if (!supported)
                        g_set_error_literal(
                            &config_error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "candidate changes settings that require a new session");
                    else if (!apply_native_input(control, context, document, &config_error))
                        apply_ok = FALSE;
                    else if (workspaces_changed && !meta_gnoblin_initialize_native_workspaces(
                                                       control->display, &config_error))
                        apply_ok = FALSE;
                    if (!apply_ok) {
                        gnoblin_runtime_cache_replace(control->runtime_cache, previous_document,
                                                      previous_revision, NULL);
                        if (supported && previous_document) {
                            g_autoptr(GError) rollback_error = NULL;
                            if (!apply_native_input(control, context, previous_document,
                                                    &rollback_error))
                                rollback_failed = TRUE;
                            if (workspaces_changed && !meta_gnoblin_initialize_native_workspaces(
                                                          control->display, &rollback_error))
                                rollback_failed = TRUE;
                        }
                        handled = FALSE;
                    }
                }
                if (handled && revision > previous_revision && bootstrap_runtime_cache) {
                    g_autoptr(GError) bootstrap_error = NULL;
                    if (!gnoblin_runtime_cache_replace(bootstrap_runtime_cache, document, revision,
                                                       &bootstrap_error)) {
                        gnoblin_runtime_cache_replace(control->runtime_cache, previous_document,
                                                      previous_revision, NULL);
                        gnoblin_runtime_cache_replace(bootstrap_runtime_cache,
                                                      previous_bootstrap_document,
                                                      previous_bootstrap_revision, NULL);
                        MetaContext* context = meta_backend_get_context(control->backend);
                        g_autoptr(GError) rollback_error = NULL;
                        if (!apply_native_input(control, context, previous_document,
                                                &rollback_error))
                            rollback_failed = TRUE;
                        if (workspaces_changed)
                            if (!meta_gnoblin_initialize_native_workspaces(control->display,
                                                                           &rollback_error))
                                rollback_failed = TRUE;
                        handled = FALSE;
                        if (bootstrap_error)
                            g_propagate_error(&config_error, g_steal_pointer(&bootstrap_error));
                        else
                            g_set_error_literal(
                                &config_error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                "could not update the bootstrap configuration cache");
                    }
                }
                if (handled && runtime_generation > previous_runtime_generation) {
                    native_cancel_window_drags(control, "config_reloaded");
                    revoke_focus_contexts(control);
                }
                if (handled && revision > previous_revision) {
                    native_settings_changed(revision, control);
                    if (input_sources_changed)
                        schedule_windows(control);
                }
                if (handled)
                    control->runtime_generation = runtime_generation;
            } else if (fields_valid) {
                g_set_error_literal(&config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "configuration revision or runtime generation is stale");
            } else if (!config_error) {
                g_set_error_literal(&config_error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "supervisor sent an invalid configuration snapshot");
            }
            if (handled) {
                gboolean events_changed =
                    !control->runtime_event_subscriptions || !next_event_subscriptions ||
                    !native_runtime_event_subscriptions_equal(control->runtime_event_subscriptions,
                                                              next_event_subscriptions);
                if (events_changed) {
                    g_clear_pointer(&control->runtime_event_subscriptions, g_hash_table_unref);
                    control->runtime_event_subscriptions =
                        g_steal_pointer(&next_event_subscriptions);
                    refresh_display_signal_watches(control);
                    refresh_all_window_signal_watches(control);
                    refresh_all_workspace_signal_watches(control);
                    refresh_workspace_manager_signal_watches(control);
                    refresh_backend_signal_watches(control);
                    refresh_monitor_manager_signal_watches(control);
                    refresh_cursor_tracker_signal_watches(control);
                }
            }
            g_clear_pointer(&document, g_variant_unref);
            if (packet.request_id > 0) {
                const char* message = config_error ? config_error->message : NULL;
                handled =
                    native_runtime_send_config_result(control, packet.request_id, handled, revision,
                                                      runtime_generation, message, &error);
            } else if (config_error) {
                g_propagate_error(&error, g_steal_pointer(&config_error));
            }
            if (rollback_failed && !error)
                g_set_error_literal(
                    &error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    "Mutter could not restore the previous configuration after rejection");
        } else if (packet.type == GNOBLIN_RUNTIME_PACKET_SHUTDOWN) {
            handled = TRUE;
            native_runtime_abort(control);
        } else {
            g_set_error_literal(&error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "unexpected packet from Gnoblin supervisor");
        }
        gnoblin_runtime_packet_clear(&packet);
        if (error || !handled) {
            g_warning("gnoblin-native-control: private runtime packet rejected: %s",
                      error ? error->message : "packet was not handled");
            native_runtime_abort(control);
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
        if (control->stopping) {
            control->runtime_read_source_id = 0;
            return G_SOURCE_REMOVE;
        }
    }
}

gboolean gnoblin_native_control_dispatch_runtime_event(MetaDisplay* display, const char* event,
                                                       GVariant* payload, gboolean* claimed,
                                                       GError** error) {
    GnoblinNativeControl* control =
        g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY);
    if (!control || !control->supervised_runtime || !event || !*event || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid supervised runtime event");
        return FALSE;
    }
    if (claimed)
        *claimed = FALSE;
    if (g_str_equal(event, "mutter.touchpad.gesture") &&
        g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT) && claimed)
        *claimed = native_config_event(display, event, NULL, payload, control);
    if (!native_runtime_dispatch_event(control, event, payload)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not queue supervised runtime event");
        return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_native_control_is_session(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!display)
        return bootstrap_runtime_cache != NULL;
    return control && control->supervised_runtime;
}

gboolean gnoblin_native_control_is_supervised(MetaDisplay* display) {
    return gnoblin_native_control_is_session(display);
}

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   int runtime_fd, guint64 settings_revision,
                                                   GError** error) {
    g_autoptr(GVariant) initial_session_lock = NULL;
    g_autoptr(GSocketAddress) bind_address = NULL;
    g_autoptr(GList) windows = NULL;
    g_autofree char* socket_directory = NULL;
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) initial_snapshot = NULL;
    g_autoptr(GError) workspace_error = NULL;
    g_autoptr(JsonNode) initial_workspaces = NULL;
    g_autoptr(GError) monitor_error = NULL;
    g_autoptr(JsonNode) initial_monitors = NULL;
    g_autoptr(GError) layer_error = NULL;
    g_autoptr(GVariant) capabilities = NULL;
    g_autoptr(GVariant) input_devices = NULL;
    g_autoptr(GVariant) input_sources = NULL;
    g_autoptr(GVariant) appearance = NULL;
    g_autoptr(JsonNode) initial_input_devices = NULL;
    if (runtime_fd < 0 || !document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "native control requires a validated Lua-supervisor bootstrap");
        return NULL;
    }
    const char* runtime = g_getenv("XDG_RUNTIME_DIR");
    if (!runtime || !*runtime) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "XDG_RUNTIME_DIR is required for native control");
        return NULL;
    }
    g_autofree char* directory = g_build_filename(runtime, "gnoblin", NULL);
    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno), "cannot create %s: %s",
                    directory, g_strerror(errno));
        return NULL;
    }
    struct stat directory_stat;
    if (lstat(directory, &directory_stat) != 0 || !S_ISDIR(directory_stat.st_mode) ||
        directory_stat.st_uid != getuid() || (directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", directory);
        return NULL;
    }
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    control->runtime_fd = -1;
    control->supervised_runtime = TRUE;
    control->runtime_fd = runtime_fd;
    control->runtime_generation = bootstrap_runtime_generation;
    control->runtime_cache = gnoblin_runtime_cache_new();
    control->runtime_reader = gnoblin_runtime_reader_new();
    control->runtime_writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_cache_replace(control->runtime_cache, document, settings_revision,
                                       error)) {
        gnoblin_runtime_cache_free(control->runtime_cache);
        gnoblin_runtime_reader_free(control->runtime_reader);
        gnoblin_runtime_writer_free(control->runtime_writer);
        g_free(control);
        return NULL;
    }
    control->clients = g_hash_table_new(g_direct_hash, g_direct_equal);
    control->window_shader_files =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_shader_file_free);
    control->pending_runtime_requests =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, pending_runtime_request_free);
    control->pending_location_authorizations = g_hash_table_new_full(
        g_int64_hash, g_int64_equal, g_free, pending_location_authorization_free);
    control->runtime_operation_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    control->runtime_cancelled_operation_ids =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, NULL);
    control->runtime_event_subscriptions =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    control->display_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->workspace_manager_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->workspace_signal_handler_ids = g_hash_table_new_full(
        g_direct_hash, g_direct_equal, g_object_unref, (GDestroyNotify)g_array_unref);
    control->backend_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->monitor_manager_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->cursor_tracker_signal_handler_ids = g_array_new(FALSE, FALSE, sizeof(gulong));
    control->window_signal_handler_ids =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, (GDestroyNotify)g_array_unref);
    control->pending_runtime_states = g_queue_new();
    control->pending_runtime_events = g_queue_new();
    control->privacy_handles =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    control->privacy_revision = 1;
    control->dynamic_shortcuts =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, native_dynamic_shortcut_free);
    control->touchpad_router = gnoblin_touchpad_router_new();
    control->focus_contexts = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    control->menu_contexts =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, native_menu_context_free);
    control->text_targets = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    control->window_drags =
        g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, native_window_drag_free);
    control->snap_contexts =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_snap_context_free);
    control->snap_restore_frames =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
    control->focus_context_timeout_id =
        g_timeout_add_seconds(1, focus_context_expiry_tick, control);
    control->launches = g_ptr_array_new_with_free_func(native_launch_free);
    update_launch_snapshot(control);
    control->wayland_compositor = meta_context_get_wayland_compositor(context);
    if (control->wayland_compositor)
        control->session_lock_callback_id = meta_wayland_session_lock_add_state_changed_callback(
            control->wayland_compositor, native_session_lock_changed, control, NULL);
    initial_session_lock =
        native_session_lock_snapshot(control->wayland_compositor, control->session_lock_revision);
    native_publish_runtime_snapshot(control, "session-lock", initial_session_lock,
                                    control->session_lock_revision);
    control->windows = g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    control->window_state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free);
    control->input_device_ids = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
    control->display = meta_context_get_display(context);
    g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, control);
    MetaBackend* backend = meta_context_get_backend(context);
    control->backend = backend;
    control->orientation_manager = meta_backend_get_orientation_manager(backend);
    control->cursor_tracker = meta_backend_get_cursor_tracker(backend);
    control->remote_access_controller =
        g_object_ref(meta_backend_get_remote_access_controller(backend));
    control->monitor_manager = meta_backend_get_monitor_manager(backend);
    control->input_seat = meta_backend_get_default_seat(backend);
    control->workspace_manager = meta_display_get_workspace_manager(control->display);
    control->path = g_strdup(g_getenv("GNOBLIN_COMPOSITOR_SOCKET"));
    if (!control->path)
        control->path = g_build_filename(directory, "compositor-v1.sock", NULL);
    socket_directory = g_path_get_dirname(control->path);
    if (!meta_gnoblin_initialize_native_workspaces(control->display, error))
        goto fail;
    struct stat socket_directory_stat;
    if (lstat(socket_directory, &socket_directory_stat) != 0 ||
        !S_ISDIR(socket_directory_stat.st_mode) || socket_directory_stat.st_uid != getuid() ||
        (socket_directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", socket_directory);
        goto fail;
    }

    struct stat existing;
    if (lstat(control->path, &existing) == 0) {
        g_autoptr(GSocketClient) probe = NULL;
        g_autoptr(GSocketAddress) address = NULL;
        g_autoptr(GError) probe_error = NULL;
        g_autoptr(GSocketConnection) active = NULL;
        if (!S_ISSOCK(existing.st_mode) || existing.st_uid != getuid()) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "refusing to replace %s",
                        control->path);
            goto fail;
        }
        probe = g_socket_client_new();
        g_socket_client_set_timeout(probe, 1);
        address = g_unix_socket_address_new(control->path);
        active = g_socket_client_connect(probe, G_SOCKET_CONNECTABLE(address), NULL, &probe_error);
        if (active || !g_error_matches(probe_error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                        "Gnoblin compositor socket is already active: %s", control->path);
            goto fail;
        }
        if (g_unlink(control->path) != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "cannot replace stale socket %s: %s", control->path, g_strerror(errno));
            goto fail;
        }
    }
    control->service = g_socket_service_new();
    bind_address = g_unix_socket_address_new(control->path);
    if (!g_socket_listener_add_address(G_SOCKET_LISTENER(control->service), bind_address,
                                       G_SOCKET_TYPE_STREAM, G_SOCKET_PROTOCOL_DEFAULT, NULL, NULL,
                                       error))
        goto fail;
    if (lstat(control->path, &existing) == 0 && S_ISSOCK(existing.st_mode)) {
        control->device = existing.st_dev;
        control->inode = existing.st_ino;
    }
    if (!control->inode || g_chmod(control->path, 0600) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "cannot protect native control socket %s: %s", control->path,
                    g_strerror(errno));
        goto fail;
    }
    g_signal_connect(control->service, "incoming", G_CALLBACK(client_connected), control);
    g_socket_service_start(control->service);
    if (control->supervised_runtime) {
        control->runtime_read_source_id =
            g_unix_fd_add(control->runtime_fd, G_IO_IN | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                          native_runtime_fd_ready, control);
    }
    native_activity_publish(control, FALSE, FALSE, 120000, 0);
    start_native_policy_dbus(control); /* D-Bus absence keeps portal grants fail closed. */
    g_signal_connect(control->display, "gnoblin-native-event", G_CALLBACK(native_native_event),
                     control);
    native_settings_changed(gnoblin_runtime_cache_get_settings_revision(control->runtime_cache),
                            control);
    g_signal_connect(control->display, "window-created", G_CALLBACK(window_created), control);
    g_signal_connect(control->display, "notify::focus-window", G_CALLBACK(display_notified),
                     control);
    g_signal_connect(control->display, "restacked", G_CALLBACK(display_restacked), control);
    g_signal_connect(control->remote_access_controller, "new-handle",
                     G_CALLBACK(privacy_new_handle), control);
    g_signal_connect(control->workspace_manager, "workspace-added",
                     G_CALLBACK(workspace_manager_index_changed), control);
    g_signal_connect(control->workspace_manager, "workspace-removed",
                     G_CALLBACK(workspace_manager_index_changed), control);
    g_signal_connect(control->workspace_manager, "workspaces-reordered",
                     G_CALLBACK(workspace_manager_changed), control);
    g_signal_connect(control->workspace_manager, "active-workspace-changed",
                     G_CALLBACK(workspace_manager_changed), control);
    g_signal_connect(control->workspace_manager, "gnoblin-workspace-state-changed",
                     G_CALLBACK(workspace_manager_changed), control);
    if (control->monitor_manager)
        g_signal_connect(control->monitor_manager, "monitors-changed",
                         G_CALLBACK(monitor_manager_changed), control);
    g_signal_connect(control->backend, "keymap-layout-group-changed",
                     G_CALLBACK(backend_keymap_layout_group_changed), control);
    g_signal_connect(control->backend, "keymap-changed", G_CALLBACK(backend_keymap_changed),
                     control);
    if (control->input_seat) {
        g_signal_connect(control->input_seat, "device-added", G_CALLBACK(input_device_added),
                         control);
        g_signal_connect(control->input_seat, "device-removed", G_CALLBACK(input_device_removed),
                         control);
    }
    GSettingsSchemaSource* schema_source = g_settings_schema_source_get_default();
    GSettingsSchema* appearance_schema =
        schema_source
            ? g_settings_schema_source_lookup(schema_source, "org.gnome.desktop.interface", TRUE)
            : NULL;
    if (appearance_schema && g_settings_schema_has_key(appearance_schema, "color-scheme")) {
        control->appearance_settings = g_settings_new_full(appearance_schema, NULL, NULL);
        g_signal_connect(control->appearance_settings, "changed::color-scheme",
                         G_CALLBACK(appearance_color_scheme_changed), control);
    } else {
        g_warning(
            "gnoblin-native-control: org.gnome.desktop.interface/color-scheme is unavailable; "
            "Lua appearance reads and change events are unavailable");
    }
    if (appearance_schema)
        g_settings_schema_unref(appearance_schema);
    control->appearance_revision = 1;
    appearance = appearance_snapshot_new(control);
    native_publish_runtime_snapshot(control, "appearance", appearance,
                                    control->appearance_revision);
    refresh_input_sources(control, document);
    start_ibus_input_source_tracking(control);
    windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next)
        track_window(control, item->data);
    initial_snapshot = window_snapshot_json(control, TRUE, &snapshot_error);
    if (!initial_snapshot)
        g_warning("gnoblin-native-control: cannot seed Lua window snapshot: %s",
                  snapshot_error ? snapshot_error->message : "window listing unavailable");
    initial_workspaces = workspace_snapshot_json(control, &workspace_error);
    if (!initial_workspaces)
        g_warning("gnoblin-native-control: cannot seed Lua workspace snapshot: %s",
                  workspace_error ? workspace_error->message : "workspace listing unavailable");
    else {
        control->workspace_state = workspace_state_from_snapshot(initial_workspaces);
        control->workspace_state_initialized = TRUE;
    }
    initial_monitors = monitor_snapshot_json(control, TRUE, &monitor_error);
    if (!initial_monitors)
        g_warning("gnoblin-native-control: cannot seed monitor snapshot: %s",
                  monitor_error ? monitor_error->message : "monitor listing unavailable");
    else {
        control->monitor_state = monitor_state_from_snapshot(initial_monitors);
        control->monitor_state_initialized = TRUE;
    }
    g_signal_connect(control->display, "show-osd", G_CALLBACK(native_show_osd_requested), control);
    if (!layer_snapshot_json(control, TRUE, &layer_error))
        g_warning("gnoblin-native-control: cannot seed Lua layer snapshot: %s",
                  layer_error ? layer_error->message : "layer listing unavailable");
    capabilities = capability_snapshot(control);
    native_publish_runtime_snapshot(control, "capabilities", capabilities, control->state_revision);
    input_devices = input_device_snapshot(control);
    native_publish_runtime_snapshot(control, "input-devices", input_devices,
                                    control->state_revision);
    input_sources = input_source_snapshot(control);
    native_publish_runtime_snapshot(control, "input-sources", input_sources,
                                    control->state_revision);
    publish_privacy_snapshot(control, FALSE);
    control->input_source_state_initialized = TRUE;
    NativeInputSource* initial_current_source = current_input_source(control);
    control->last_published_input_source =
        initial_current_source ? g_strdup(initial_current_source->id) : NULL;
    initial_input_devices = json_from_variant(input_devices);
    control->input_device_state = input_device_state_from_snapshot(initial_input_devices);
    control->input_device_state_initialized = TRUE;
#ifdef HAVE_REMOTE_DESKTOP
    control->pipewire_monitor =
        gnoblin_pipewire_monitor_new(NULL, privacy_pipewire_state_changed, control, NULL);
    if (control->pipewire_monitor)
        gnoblin_pipewire_monitor_start(control->pipewire_monitor);
#endif
    control->location_agent = gnoblin_location_agent_new(
        NULL, location_authorize_app, privacy_location_state_changed, control, NULL);
    if (control->location_agent)
        gnoblin_location_agent_start(control->location_agent);
    schedule_windows(control);
    if (control->input_sources && control->input_sources->len > 0 &&
        !control->input_keymap_description) {
        NativeInputSource* first = g_ptr_array_index(control->input_sources, 0);
        if (!select_xkb_source(control, first->id, 0, "input.select_source", error))
            goto fail;
    }
    if (!apply_native_input(control, context, document, error))
        goto fail;
    if (control->orientation_manager) {
        g_signal_connect(control->orientation_manager, "notify::orientation-locked",
                         G_CALLBACK(native_orientation_manager_notified), control);
        g_signal_connect(control->orientation_manager, "notify::has-accelerometer",
                         G_CALLBACK(native_orientation_manager_accelerometer_notified), control);
        g_signal_connect(control->orientation_manager, "orientation-changed",
                         G_CALLBACK(native_orientation_manager_orientation_changed), control);
    }
    native_orientation_lock_publish(control);
    if (!apply_native_keybindings(document, error))
        goto fail;
    if (!start_native_shortcuts(control, document, error))
        goto fail;
    if (control->supervised_runtime) {
        GVariantBuilder hello;
        g_variant_builder_init(&hello, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&hello, "{sv}", "role", g_variant_new_string("compositor"));
        g_variant_builder_add(&hello, "{sv}", "document_version", g_variant_new_uint32(1));
        GVariantBuilder environment;
        g_variant_builder_init(&environment, G_VARIANT_TYPE("a{ss}"));
        const char* display_environment[] = {"WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", NULL};
        for (guint i = 0; display_environment[i]; i++) {
            const char* value = g_getenv(display_environment[i]);
            if (value && *value)
                g_variant_builder_add(&environment, "{ss}", display_environment[i], value);
        }
        g_variant_builder_add(&hello, "{sv}", "environment", g_variant_builder_end(&environment));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&hello));
        if (!native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_HELLO, 0, payload, error))
            goto fail;
        control->runtime_hello_sent = TRUE;
        if (!native_runtime_flush_state_snapshots(control, error) ||
            !native_runtime_flush_pending_events(control, error))
            goto fail;
    }
    return control;

fail:
    gnoblin_native_control_stop(control);
    return NULL;
}

void gnoblin_native_control_stop(GnoblinNativeControl* control) {
    if (!control)
        return;
    control->stopping = TRUE;
    if (control->privacy_camera_disable_source_id) {
        g_source_remove(control->privacy_camera_disable_source_id);
        control->privacy_camera_disable_source_id = 0;
    }
    clear_pending_location_authorizations(control);
    g_clear_pointer(&control->location_agent, gnoblin_location_agent_free);
    control->overlay_modifier_hook_available = FALSE;
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session,
                                     "compositor_stopped");
    native_corner_toolkit_cache_shutdown(control);
#ifdef HAVE_REMOTE_DESKTOP
    g_clear_pointer(&control->pipewire_monitor, gnoblin_pipewire_monitor_free);
#endif
    if (control->runtime_read_source_id) {
        g_source_remove(control->runtime_read_source_id);
        control->runtime_read_source_id = 0;
    }
    if (control->runtime_write_source_id) {
        g_source_remove(control->runtime_write_source_id);
        control->runtime_write_source_id = 0;
    }
    native_runtime_fail_pending_requests(control, "Gnoblin compositor stopped before replying");
    stop_native_policy_dbus(control);
    if (control->display)
        meta_display_set_gnoblin_shortcut_activated_handler(control->display, NULL, NULL);
    revoke_focus_contexts(control);
    revoke_text_targets(control);
    native_cancel_window_drags(control, "compositor_stopped");
    if (control->snap_contexts)
        g_hash_table_remove_all(control->snap_contexts);
    stop_native_shortcut_capture(control, FALSE, FALSE, NULL, NULL, NULL);
    if (control->wayland_compositor && control->session_lock_callback_id) {
        meta_wayland_session_lock_remove_state_changed_callback(control->wayland_compositor,
                                                                control->session_lock_callback_id);
        control->session_lock_callback_id = 0;
    }
    if (control->display &&
        g_object_get_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY) == control)
        g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, NULL);
    clear_display_signal_watches(control);
    if (control->service) {
        g_socket_service_stop(control->service);
        g_clear_object(&control->service);
    }
    if (control->publish_id)
        g_source_remove(control->publish_id);
    if (control->launch_tick_id)
        g_source_remove(control->launch_tick_id);
    if (control->focus_context_timeout_id) {
        g_source_remove(control->focus_context_timeout_id);
        control->focus_context_timeout_id = 0;
    }
    if (control->backend) {
        MetaCursorTracker* tracker = meta_backend_get_cursor_tracker(control->backend);
        if (tracker)
            meta_cursor_tracker_set_gnoblin_launch_cursor(tracker, FALSE);
    }
    if (control->display)
        g_signal_handlers_disconnect_by_data(control->display, control);
    if (control->dynamic_shortcuts) {
        GHashTableIter shortcut_iter;
        gpointer shortcut_value;
        g_hash_table_iter_init(&shortcut_iter, control->dynamic_shortcuts);
        while (g_hash_table_iter_next(&shortcut_iter, NULL, &shortcut_value)) {
            NativeDynamicShortcut* shortcut = shortcut_value;
            meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
        g_hash_table_unref(control->dynamic_shortcuts);
        control->dynamic_shortcuts = NULL;
    }
    if (control->bare_super_shortcut) {
        native_dynamic_shortcut_free(control->bare_super_shortcut);
        control->bare_super_shortcut = NULL;
    }
    if (control->shortcuts) {
        for (guint i = 0; i < control->shortcuts->len; i++) {
            NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
            if (!shortcut->overlay)
                meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
        g_ptr_array_unref(control->shortcuts);
    }
    if (control->windows) {
        GHashTableIter window_iter;
        gpointer window;
        g_hash_table_iter_init(&window_iter, control->windows);
        while (g_hash_table_iter_next(&window_iter, &window, NULL)) {
            GArray* handler_ids = g_hash_table_lookup(control->window_signal_handler_ids, window);
            clear_object_signal_watches(G_OBJECT(window), handler_ids);
            g_signal_handlers_disconnect_by_data(window, control);
        }
        g_hash_table_unref(control->windows);
    }
    g_clear_pointer(&control->window_signal_handler_ids, g_hash_table_unref);
    if (control->workspace_manager)
        clear_workspace_manager_signal_watches(control);
    clear_all_workspace_signal_watches(control);
    clear_backend_signal_watches(control);
    clear_monitor_manager_signal_watches(control);
    clear_cursor_tracker_signal_watches(control);
    if (control->workspace_manager)
        g_signal_handlers_disconnect_by_data(control->workspace_manager, control);
    if (control->monitor_manager)
        g_signal_handlers_disconnect_by_data(control->monitor_manager, control);
    if (control->backend)
        g_signal_handlers_disconnect_by_data(control->backend, control);
    if (control->remote_access_controller)
        g_signal_handlers_disconnect_by_data(control->remote_access_controller, control);
    if (control->orientation_manager)
        g_signal_handlers_disconnect_by_data(control->orientation_manager, control);
    if (control->appearance_settings)
        g_signal_handlers_disconnect_by_data(control->appearance_settings, control);
    if (control->input_seat)
        g_signal_handlers_disconnect_by_data(control->input_seat, control);
    if (control->privacy_handles) {
        GHashTableIter privacy_iter;
        gpointer handle;
        g_hash_table_iter_init(&privacy_iter, control->privacy_handles);
        while (g_hash_table_iter_next(&privacy_iter, &handle, NULL))
            g_signal_handlers_disconnect_by_data(handle, control);
        g_hash_table_unref(control->privacy_handles);
        control->privacy_handles = NULL;
    }
    g_clear_pointer(&control->privacy_snapshot, g_variant_unref);
    g_clear_pointer(&control->orientation_lock_snapshot, g_variant_unref);
    native_publish_runtime_snapshot(control, "privacy", NULL, control->privacy_revision);
    g_clear_object(&control->remote_access_controller);
    if (control->window_state)
        g_hash_table_unref(control->window_state);
    if (control->workspace_state)
        g_hash_table_unref(control->workspace_state);
    if (control->monitor_state)
        g_hash_table_unref(control->monitor_state);
    if (control->input_device_state)
        g_hash_table_unref(control->input_device_state);
    if (control->input_device_ids)
        g_hash_table_unref(control->input_device_ids);
    if (control->launches)
        g_ptr_array_unref(control->launches);
    if (control->focus_contexts)
        g_hash_table_unref(control->focus_contexts);
    if (control->menu_contexts)
        g_hash_table_unref(control->menu_contexts);
    if (control->window_drags)
        g_hash_table_unref(control->window_drags);
    if (control->snap_contexts)
        g_hash_table_unref(control->snap_contexts);
    if (control->snap_restore_frames)
        g_hash_table_unref(control->snap_restore_frames);
    if (control->text_targets)
        g_hash_table_unref(control->text_targets);
    if (control->clients) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, control->clients);
        while (g_hash_table_iter_next(&iter, &value, NULL)) {
            Client* client = value;
            client->control = NULL;
            client_close(client);
        }
        g_hash_table_unref(control->clients);
        control->clients = NULL;
    }
    struct stat current;
    if (control->path && control->inode && lstat(control->path, &current) == 0 &&
        current.st_dev == control->device && current.st_ino == control->inode)
        g_unlink(control->path);
    g_free(control->path);
    g_clear_pointer(&control->native_touchpad_gestures, g_variant_unref);
    g_clear_pointer(&control->touchpad_router, gnoblin_touchpad_router_free);
    g_clear_pointer(&control->runtime_cache, gnoblin_runtime_cache_free);
    g_clear_pointer(&control->pending_runtime_requests, g_hash_table_unref);
    g_clear_pointer(&control->pending_location_authorizations, g_hash_table_unref);
    g_clear_pointer(&control->runtime_operation_ids, g_hash_table_unref);
    g_clear_pointer(&control->runtime_cancelled_operation_ids, g_hash_table_unref);
    g_clear_pointer(&control->runtime_event_subscriptions, g_hash_table_unref);
    g_clear_pointer(&control->display_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->workspace_manager_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->workspace_signal_handler_ids, g_hash_table_unref);
    g_clear_pointer(&control->backend_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->monitor_manager_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->cursor_tracker_signal_handler_ids, g_array_unref);
    g_clear_pointer(&control->runtime_reader, gnoblin_runtime_reader_free);
    g_clear_pointer(&control->runtime_writer, gnoblin_runtime_writer_free);
    if (control->pending_runtime_states) {
        g_queue_clear_full(control->pending_runtime_states, (GDestroyNotify)g_variant_unref);
        g_queue_free(control->pending_runtime_states);
        control->pending_runtime_states = NULL;
    }
    if (control->pending_runtime_events) {
        g_queue_clear_full(control->pending_runtime_events, (GDestroyNotify)g_variant_unref);
        g_queue_free(control->pending_runtime_events);
        control->pending_runtime_events = NULL;
    }
    control->path = NULL;
    control->teardown_complete = TRUE;
    native_control_maybe_free_stopped(control);
}
