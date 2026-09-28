/* The first native owner of compositor-v1: Lua-validated compositor methods. */
#include "config.h"

#include "core/gnoblin-native-control.h"

#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib/gstdio.h>
#include <json-glib/json-glib.h>
#include <errno.h>
#include <math.h>
#include <string.h>
#include <sys/random.h>
#include <sys/stat.h>
#include <unistd.h>
#include <xkbcommon/xkbcommon.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-keymap-description-private.h"
#include "clutter/clutter.h"
#include "core/display-private.h"
#include "core/events.h"
#include "core/util-private.h"
#include "wayland/gnoblin-config.h"
#include "wayland/gnoblin-portal-policy.h"
#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-cursor-tracker.h"
#include "meta/meta-context.h"
#include "meta/meta-wayland-compositor.h"
#include "meta/meta-monitor-manager.h"
#include "meta/meta-orientation-manager.h"
#include "meta/meta-workspace-manager.h"
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
#define MAX_DYNAMIC_SHORTCUTS_PER_CLIENT 32
#define MAX_DYNAMIC_SHORTCUTS 128
#define FOCUS_CONTEXT_LIFETIME_US (5 * G_USEC_PER_SEC)
#define PORTAL_GRANT_TIMEOUT_MS 5000
#define PORTAL_BACKEND_BUS_NAME "org.freedesktop.impl.portal.desktop.gnoblin"
#define NATIVE_POLICY_BUS_NAME "org.gnoblin.Compositor"
#define NATIVE_POLICY_OBJECT_PATH "/org/gnoblin/Compositor"
#define NATIVE_POLICY_INTERFACE "org.gnoblin.Compositor"

struct _GnoblinNativeControl {
    GSocketService* service;
    GDBusConnection* session_bus;
    GHashTable* clients;
    GHashTable* windows;
    GHashTable* window_state;
    GHashTable* workspace_state;
    GHashTable* monitor_state;
    GHashTable* input_device_state;
    GHashTable* input_device_ids;
    GVariant* portal_grant_snapshot;
    GPtrArray* input_sources;
    GPtrArray* configured_input_source_ids;
    GPtrArray* active_input_source_ids;
    GPtrArray* shortcuts;
    GHashTable* dynamic_shortcuts;
    GPtrArray* launches;
    GHashTable* focus_contexts;
    GQueue* policy_events;
    MetaDisplay* display;
    MetaWaylandCompositor* wayland_compositor;
    MetaWorkspaceManager* workspace_manager;
    MetaMonitorManager* monitor_manager;
    ClutterSeat* input_seat;
    MetaBackend* backend;
    MetaKeymapDescription* input_keymap_description;
    GSettings* input_source_settings;
    char* last_published_input_source;
    guint64 next_input_device_id;
    guint publish_id;
    guint launch_tick_id;
    guint shortcut_capture_timeout_id;
    guint focus_context_timeout_id;
    guint policy_event_idle_id;
    guint policy_dbus_registration_id;
    guint portal_grant_added_subscription_id;
    guint portal_grant_removed_subscription_id;
    guint portal_owner_subscription_id;
    guint portal_grant_retry_id;
    gint64 shortcut_capture_request_id;
    gulong session_lock_callback_id;
    guint64 state_revision;
    guint64 launch_revision;
    guint64 portal_grant_revision;
    guint portal_grant_retry_count;
    guint64 event_sequence;
    guint64 next_focus_context_handle;
    gboolean window_state_initialized;
    gboolean workspace_state_initialized;
    gboolean monitor_state_initialized;
    gboolean input_device_state_initialized;
    gboolean input_source_state_initialized;
    gboolean stopping;
    gboolean shortcut_capture_active;
    gboolean shortcut_capture_super_pressed;
    gboolean policy_bus_name_owned;
    gboolean portal_backend_available;
    guint pending_input_source_ops;
    guint pending_portal_grant_ops;
    char* path;
    dev_t device;
    ino_t inode;
};

typedef struct {
    GSocketConnection* connection;
    GnoblinNativeControl* control;
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
    GHashTable* pending_grant_operations;
} Client;

typedef struct {
    Client* client;
    char* response;
} PendingWrite;

typedef struct {
    char* name;
    char** argv;
} NativeAutostart;

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
    Client* client;
    char* id;
    char* accelerator;
    guint action;
} NativeDynamicShortcut;

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
    char* selected_id;
    gint64 request_id;
    char* method;
    guint group;
} PendingInputSource;

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
static void send_response(Client* client, char* response);
static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload);
static void revoke_focus_contexts(GnoblinNativeControl* control);
static void prune_focus_contexts(GnoblinNativeControl* control, gint64 now);
static gboolean focus_context_expiry_tick(gpointer user_data);
static void publish_shortcut_focus_event(GnoblinNativeControl* control, const char* shortcut,
                                         guint64 handle, guint64 generation, gint64 expires_at_us);
static void publish_input_source_changes(GnoblinNativeControl* control, guint64 revision);
static void update_launch_snapshot(GnoblinNativeControl* control);
static void stop_native_shortcut_capture(GnoblinNativeControl* control, gboolean complete,
                                         gboolean ok, const char* accelerator,
                                         const char* error_code, const char* message);
static void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                                const ClutterEvent* event);
static void native_control_maybe_free_stopped(GnoblinNativeControl* control);

static const NativeCapability native_capabilities[] = {
    {"layer-list", "List layer-shell surfaces and their placement."},
    {"monitor-list", "List active logical monitors and their geometry."},
    {"window-list", "Read managed windows and their compositor state."},
    {"window-actions", "Request supported window actions."},
    {"typed-window-operations", "Set window state, geometry, workspace, or monitor."},
    {"window-interactive-grabs",
     "Start a keyboard move or resize with a trusted shortcut context."},
    {"workspace-management", "List and manage native workspaces."},
    {"window-change-events", "Receive changes to managed window properties."},
    {"window-lifecycle-events", "Receive window creation, focus, attention, and close events."},
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
};

static const char* native_socket_events[] = {
    "windows",
    "gnoblin.window.created",
    "gnoblin.window.changed",
    "gnoblin.window.focused",
    "gnoblin.window.unfocused",
    "gnoblin.window.attention-changed",
    "gnoblin.window.closed",
    "gnoblin.focus.policy-changed",
    "gnoblin.permission.changed",
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
    "gnoblin.input.gesture",
    "gnoblin.launch.changed",
    "gnoblin.shortcut.activated",
    "gnoblin.shortcut.binding-activated",
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
    g_autoptr(GVariant) document = gnoblin_config_current_document();
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
    gnoblin_config_update_portal_grant_snapshot(control->portal_grant_snapshot, revision);
}

static void portal_grant_cache_apply(GnoblinNativeControl* control, const char* changed_id,
                                     const char* changed_kind, GVariant* replacement) {
    g_autoptr(GVariant) current = control->portal_grant_snapshot
                                      ? g_variant_lookup_value(control->portal_grant_snapshot,
                                                               "grants", G_VARIANT_TYPE("aa{sv}"))
                                      : NULL;
    if (!current) {
        control->portal_grant_revision++;
        gnoblin_config_update_portal_grant_snapshot(NULL, control->portal_grant_revision);
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
    gnoblin_config_update_portal_grant_snapshot(control->portal_grant_snapshot, revision);
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
    meta_display_dispatch_gnoblin_event(control->display, event, enriched_payload);
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
        gnoblin_config_update_portal_grant_snapshot(NULL, control->portal_grant_revision);
    } else if (!g_str_equal(old_owner, new_owner)) {
        control->portal_grant_retry_count = 0;
        portal_grant_fetch_snapshot(control);
    }
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
    if (!control->session_bus)
        return;
    if (control->portal_grant_retry_id) {
        g_source_remove(control->portal_grant_retry_id);
        control->portal_grant_retry_id = 0;
    }
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
    gnoblin_config_update_portal_grant_snapshot(NULL, control->portal_grant_revision);
}

typedef struct {
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
} NativeFocusContext;

typedef struct {
    guint64 handle;
    guint64 generation;
    gint64 expires_at_us;
} NativeFocusGrant;

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

static void native_autostart_free(gpointer data) {
    NativeAutostart* entry = data;
    g_free(entry->name);
    g_strfreev(entry->argv);
    g_free(entry);
}

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

static gboolean start_native_autostart(GVariant* document, GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "autostart", NULL) : NULL;
    if (!declarations)
        return TRUE;
    if (!g_variant_is_of_type(declarations, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "autostart must be an array of entries");
        return FALSE;
    }

    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) entries = g_ptr_array_new_with_free_func(native_autostart_free);
    for (gsize index = 0; index < g_variant_n_children(declarations); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) when_value = NULL;
        g_autoptr(GVariant) command = NULL;
        const char* name = NULL;
        const char* when = "on_login";
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT))
            goto invalid_entry;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported =
                g_str_equal(key, "name") || g_str_equal(key, "command") || g_str_equal(key, "when");
            g_variant_unref(value);
            if (!supported)
                goto invalid_entry;
        }
        if (!g_variant_lookup(entry, "name", "&s", &name) || !*name ||
            g_utf8_strlen(name, -1) > 80 || g_hash_table_contains(names, name))
            goto invalid_entry;
        when_value = g_variant_lookup_value(entry, "when", NULL);
        if (when_value && (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING) ||
                           !g_str_equal(g_variant_get_string(when_value, NULL), when)))
            goto invalid_entry;
        command = g_variant_lookup_value(entry, "command", NULL);
        NativeAutostart* parsed = g_new0(NativeAutostart, 1);
        parsed->name = g_strdup(name);
        parsed->argv = native_command_argv(command);
        if (!parsed->argv) {
            native_autostart_free(parsed);
            goto invalid_entry;
        }
        g_hash_table_add(names, g_strdup(name));
        g_ptr_array_add(entries, parsed);
        continue;

    invalid_entry:
        g_set_error(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "autostart entry %zu needs a unique name, command array, and optional when='on_login'",
            index + 1);
        return FALSE;
    }

    for (guint index = 0; index < entries->len; index++) {
        NativeAutostart* entry = g_ptr_array_index(entries, index);
        launch_native_command("autostart", entry->name, entry->argv);
    }
    return TRUE;
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
        (flags & (CLUTTER_EVENT_FLAG_SYNTHETIC | CLUTTER_EVENT_FLAG_INPUT_METHOD |
                  CLUTTER_EVENT_FLAG_REPEATED)))
        return;

    dispatch_dynamic_shortcut_activated(control, action, event);

    guint64 generation = gnoblin_config_runtime_generation();
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
        g_hash_table_insert(control->focus_contexts, key_copy, context);

        GVariantBuilder payload_builder;
        g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload_builder, "{sv}", "shortcut",
                              g_variant_new_string(shortcut->name));
        g_variant_builder_add(&payload_builder, "{sv}", "trigger", g_variant_new_string("press"));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
        meta_display_dispatch_gnoblin_shortcut_event(display, "gnoblin.shortcut.activated", payload,
                                                     handle, context->expires_at_us);
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
    g_free(shortcut->id);
    g_free(shortcut->accelerator);
    g_free(shortcut);
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
    return NULL;
}

static void remove_dynamic_shortcut(GnoblinNativeControl* control,
                                    NativeDynamicShortcut* shortcut) {
    if (!control || !shortcut || !control->dynamic_shortcuts)
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
            meta_display_ungrab_accelerator(control->display, shortcut->action);
            g_hash_table_iter_remove(&iter);
        }
    }
}

static void dispatch_dynamic_shortcut_activated(GnoblinNativeControl* control, guint action,
                                                const ClutterEvent* event) {
    if (!control || control->stopping || !event || !control->dynamic_shortcuts ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    NativeDynamicShortcut* shortcut =
        g_hash_table_lookup(control->dynamic_shortcuts, GUINT_TO_POINTER(action));
    if (!shortcut || !shortcut->client || shortcut->client->closing ||
        shortcut->client->api_minor < 11 || !shortcut->client->control ||
        shortcut->client->event_api_minor < 11 || !shortcut->client->event_subscriptions ||
        !g_hash_table_contains(shortcut->client->event_subscriptions,
                               "gnoblin.shortcut.binding-activated"))
        return;
    Client* client = shortcut->client;
    guint64 generation = gnoblin_config_runtime_generation();
    gint64 now = g_get_monotonic_time();
    prune_focus_contexts(control, now);
    if (generation == 0 || g_hash_table_size(control->focus_contexts) >= MAX_FOCUS_CONTEXTS ||
        !client->focus_grants || g_hash_table_size(client->focus_grants) >= MAX_FOCUS_CONTEXTS)
        return;

    guint64 handle = ++control->next_focus_context_handle;
    if (handle == 0)
        handle = ++control->next_focus_context_handle;
    guint64* key_copy = g_new(guint64, 1);
    *key_copy = handle;
    NativeFocusContext* context = g_new0(NativeFocusContext, 1);
    context->generation = generation;
    context->expires_at_us = now + FOCUS_CONTEXT_LIFETIME_US;
    context->timestamp = clutter_event_get_time(event);
    g_hash_table_insert(control->focus_contexts, key_copy, context);

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
    if (!unique) {
        g_hash_table_remove(control->focus_contexts, &handle);
        return;
    }

    NativeFocusGrant* grant = g_new0(NativeFocusGrant, 1);
    grant->handle = handle;
    grant->generation = generation;
    grant->expires_at_us = context->expires_at_us;
    g_hash_table_insert(client->focus_grants, g_strdup(token), grant);

    JsonObject* object = json_object_new();
    json_object_set_string_member(object, "event", "gnoblin.shortcut.binding-activated");
    json_object_set_string_member(object, "id", shortcut->id);
    json_object_set_string_member(object, "accelerator", shortcut->accelerator);
    json_object_set_string_member(object, "trigger", "press");
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

static void native_shortcut_deactivated(MetaDisplay* display, guint action, gpointer device,
                                        guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && shortcut->release)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
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
        if (g_str_equal(namespace, "gnome:shell"))
            *group = "shell";
        else if (g_str_equal(namespace, "wm") || g_str_equal(namespace, "mutter") ||
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
    if (g_str_equal(schema, "org.gnome.shell.keybindings"))
        *group = "shell";
    else if (g_str_equal(schema, "org.gnome.desktop.wm.keybindings"))
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
                            "named shortcut action must identify a supported keybinding");
        return FALSE;
    }
    if (g_str_equal(group, "shell")) {
        g_warning("gnoblin-shortcuts: skipping Shell action %s; it needs the Gnoblin Shell session",
                  native_name);
        return TRUE;
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
                if (g_str_equal(group, "shell")) {
                    g_warning("gnoblin-keybindings: skipping keybindings.shell; it needs the "
                              "Gnoblin Shell session");
                    continue;
                }
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "keybindings.%s is not a native keybinding group", group);
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
            if (!native_action_target(action_value, &group, &native_name))
                goto invalid_shortcut;
            if (!g_str_equal(group, "shell")) {
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
            }
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
                g_warning("gnoblin-shortcuts: skipping '%s'; input capture needs the Gnoblin Shell "
                          "session",
                          name);
                g_hash_table_add(names, g_strdup(name));
                continue;
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
        g_set_error(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "shortcut entry %zu needs a unique name, binding, command array, and valid trigger",
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
    gnoblin_config_update_shortcut_snapshot(shortcut_snapshot, control->state_revision);
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
    if (field->kind == INPUT_MILLISECONDS && g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
        gint64 milliseconds = g_variant_get_int64(value);
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

static gboolean apply_native_input(GnoblinNativeControl* control, MetaContext* context,
                                   GVariant* document, GError** error) {
    g_autoptr(GVariant) input = document ? g_variant_lookup_value(document, "input", NULL) : NULL;
    if (!input)
        return TRUE;
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
        meta_backend_get_orientation_manager(meta_context_get_backend(context));
    if (orientation_set)
        meta_orientation_manager_set_orientation_locked(orientation, orientation_locked);
    else
        meta_orientation_manager_clear_orientation_lock_override(orientation);
    return TRUE;

invalid_input:
    g_variant_builder_clear(&converted);
    return FALSE;
}

static void client_free(Client* client) {
    if (client->control)
        g_hash_table_remove(client->control->clients, client);
    g_io_stream_close(G_IO_STREAM(client->connection), NULL, NULL);
    g_clear_object(&client->connection);
    g_string_free(client->request, TRUE);
    g_queue_free_full(client->outgoing, g_free);
    g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
    g_clear_pointer(&client->focus_grants, g_hash_table_unref);
    g_clear_pointer(&client->pending_grant_operations, g_hash_table_unref);
    g_free(client);
}

static void client_maybe_free(Client* client) {
    if (client->closing && !client->reading && !client->writing)
        client_free(client);
}

static void client_close(Client* client) {
    if (!client->closing) {
        client->closing = TRUE;
        clear_client_dynamic_shortcuts(client);
        if (client->control)
            g_hash_table_remove(client->control->clients, client);
        g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
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

static void cache_lua_window_snapshot(GVariant* native_snapshot, guint64 revision);
static void cache_lua_workspace_snapshot(GVariant* native_snapshot, guint64 revision);

static JsonNode* layer_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                                     GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "layer.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            gnoblin_config_update_layer_snapshot(NULL, control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            gnoblin_config_update_layer_snapshot(NULL, control->state_revision);
        return NULL;
    }
    JsonArray* layers = json_object_get_array_member(json_node_get_object(json), "layers");
    if (!layers) {
        if (update_lua_snapshot)
            gnoblin_config_update_layer_snapshot(NULL, control->state_revision);
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
        gnoblin_config_update_layer_snapshot(snapshot, control->state_revision);
    }
    return g_steal_pointer(&json);
}

static GVariant* capability_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder capabilities;
    GVariantBuilder snapshot;
    g_variant_builder_init(&capabilities, G_VARIANT_TYPE("av"));
    for (guint i = 0; i < G_N_ELEMENTS(native_capabilities); i++) {
        GVariantBuilder capability;
        g_variant_builder_init(&capability, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&capability, "{sv}", "id",
                              g_variant_new_string(native_capabilities[i].id));
        g_variant_builder_add(&capability, "{sv}", "description",
                              g_variant_new_string(native_capabilities[i].description));
        g_variant_builder_add(&capability, "{sv}", "available", g_variant_new_boolean(TRUE));
        g_variant_builder_add(&capability, "{sv}", "revision",
                              g_variant_new_int64(control->state_revision));
        g_variant_builder_add_value(&capabilities,
                                    g_variant_new_variant(g_variant_builder_end(&capability)));
    }
    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "capabilities", g_variant_builder_end(&capabilities));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
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
    source->id = g_strdup(id);
    source->layout = g_steal_pointer(&layout);
    source->variant = g_strdup(variant);
    source->name = g_strdup(display_names && display_names[0] ? display_names[0] : id);
    source->short_name =
        g_strdup(short_names && short_names[0] && *short_names[0] ? short_names[0] : id);
    return source;
}

static GVariant* input_source_record_variant(NativeInputSource* source, guint64 revision,
                                             gboolean current) {
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "type", g_variant_new_string("xkb"));
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

static NativeInputSource* current_input_source(GnoblinNativeControl* control) {
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

static GPtrArray* read_configured_input_source_ids(GnoblinNativeControl* control,
                                                   GVariant* document) {
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
                !g_variant_lookup(record, "id", "&s", &id) || !g_str_equal(type, "xkb"))
                continue;
            add_configured_input_source_id(ids, id);
        }
        return ids;
    }

    if (control->input_source_settings) {
        g_autoptr(GVariant) defaults =
            g_settings_get_value(control->input_source_settings, "sources");
        GVariantIter iter;
        const char* type;
        const char* id;
        g_variant_iter_init(&iter, defaults);
        while (g_variant_iter_next(&iter, "(&s&s)", &type, &id)) {
            if (!g_str_equal(type, "xkb"))
                continue;
            add_configured_input_source_id(ids, id);
        }
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
        current_document = gnoblin_config_current_document();
        document = current_document;
    }
    GPtrArray* ids = read_configured_input_source_ids(control, document);
    if (input_source_id_lists_equal(control->configured_input_source_ids, ids)) {
        g_ptr_array_unref(ids);
        return FALSE;
    }
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    control->configured_input_source_ids = ids;

    GPtrArray* sources = g_ptr_array_new_with_free_func(native_input_source_free);
    for (guint i = 0; i < ids->len; i++) {
        const char* id = g_ptr_array_index(ids, i);
        NativeInputSource* source = input_source_new(id);
        if (source)
            g_ptr_array_add(sources, source);
    }
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    control->input_sources = sources;
    return TRUE;
}

static void pending_input_source_free(PendingInputSource* pending) {
    if (!pending)
        return;
    g_clear_pointer(&pending->description, meta_keymap_description_unref);
    g_clear_pointer(&pending->source_ids, g_ptr_array_unref);
    g_free(pending->selected_id);
    g_free(pending->method);
    g_free(pending);
}

static void release_input_source_state(GnoblinNativeControl* control) {
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->active_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->input_keymap_description, meta_keymap_description_unref);
    g_clear_object(&control->input_source_settings);
    g_clear_pointer(&control->last_published_input_source, g_free);
}

static const char* native_operation_error_code(const GError* error) {
    if (!error || error->domain != G_IO_ERROR)
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
    if (!dispatch_lua)
        return;
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.operation.completed",
                                            payload);

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
    g_autoptr(GVariant) legacy_payload = variant_from_json(legacy);
    if (legacy_payload)
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.api.operation-completed",
                                            legacy_payload);
}

static void dispatch_operation_completion(GnoblinNativeControl* control, gint64 request_id,
                                          const char* method, gboolean ok, JsonNode* result,
                                          const char* error_code, const char* message) {
    dispatch_operation_completion_full(control, request_id, method, ok, result, error_code, message,
                                       TRUE);
}

static void dispatch_input_source_operation(GnoblinNativeControl* control, gint64 request_id,
                                            const char* method, gboolean ok,
                                            const char* selected_id, const char* error_code,
                                            const char* message) {
    g_autoptr(JsonNode) result = NULL;
    if (ok && selected_id) {
        NativeInputSource* source = input_source_by_id(control, selected_id);
        if (source) {
            g_autoptr(GVariant) record =
                input_source_record_variant(source, control->state_revision, TRUE);
            result = json_from_variant(record);
        }
    }
    dispatch_operation_completion(control, request_id, method, ok, result, error_code, message);
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
    if (!control || !control->stopping || control->pending_input_source_ops != 0 ||
        control->pending_portal_grant_ops != 0)
        return;
    release_input_source_state(control);
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
        gboolean dispatch_lua = gnoblin_config_runtime_generation() == pending->runtime_generation;
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
        gboolean dispatch_lua = gnoblin_config_runtime_generation() == pending->runtime_generation;
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
                                           gnoblin_config_runtime_generation() ==
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
    pending->runtime_generation = gnoblin_config_runtime_generation();
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

static void native_shortcut_capture_key(const ClutterEvent* event, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
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
    if (type != CLUTTER_KEY_PRESS || xkb_keysym_is_modifier(keyval))
        return;

    control->shortcut_capture_super_pressed = FALSE;
    g_autofree char* accelerator = meta_accelerator_name(clutter_event_get_state(event), keyval);
    if (!accelerator || !*accelerator)
        return;

    stop_native_shortcut_capture(control, TRUE, TRUE, accelerator, NULL, NULL);
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

static void native_session_lock_changed(MetaWaylandCompositor* compositor,
                                        MetaWaylandSessionLockState state, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (state != META_WAYLAND_SESSION_LOCK_UNLOCKED) {
        gnoblin_native_control_revoke_focus_contexts(control->display);
        stop_native_shortcut_capture(user_data, TRUE, FALSE, NULL, "denied",
                                     "shortcut capture cancelled because the session locked");
    }
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
    dispatch_input_source_operation(control, pending->request_id, pending->method, confirmed,
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
    g_autoptr(GVariant) document = gnoblin_config_current_document();
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
    pending->selected_id = g_strdup(id);
    pending->request_id = request_id;
    pending->method = g_strdup(method);
    pending->group = selected_index - chunk_start;
    control->pending_input_source_ops++;
    meta_backend_set_keymap_async(control->backend, description, pending->group, NULL,
                                  input_source_keymap_set_done, pending);
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
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
            "native input source selection currently supports XKB only; IBus is unavailable");
        return FALSE;
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

static JsonNode* window_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
                                      GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "window.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            gnoblin_config_update_window_snapshot(NULL, control->state_revision);
        return NULL;
    }
    if (update_lua_snapshot)
        cache_lua_window_snapshot(result, control->state_revision);
    g_autoptr(JsonNode) json = json_from_variant(result);
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

static JsonNode* lua_window_record(JsonNode* native_record) {
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
    JsonNode* node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, target);
    return node;
}

static void cache_lua_window_snapshot(GVariant* native_snapshot, guint64 revision) {
    g_autoptr(JsonNode) json = json_from_variant(native_snapshot);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        gnoblin_config_update_window_snapshot(NULL, revision);
        return;
    }
    JsonObject* object = json_node_get_object(json);
    JsonNode* native_windows = json_object_get_member(object, "windows");
    if (!native_windows || !JSON_NODE_HOLDS_ARRAY(native_windows)) {
        gnoblin_config_update_window_snapshot(NULL, revision);
        return;
    }

    JsonArray* windows = json_array_new();
    JsonArray* source = json_node_get_array(native_windows);
    for (guint i = 0; i < json_array_get_length(source); i++) {
        JsonNode* native_record = json_array_get_element(source, i);
        if (!JSON_NODE_HOLDS_OBJECT(native_record))
            continue;
        JsonNode* record = lua_window_record(native_record);
        json_object_set_int_member(json_node_get_object(record), "revision", revision);
        json_array_add_element(windows, record);
    }
    JsonNode* lua_windows = json_node_new(JSON_NODE_ARRAY);
    json_node_take_array(lua_windows, windows);
    json_object_set_member(object, "windows", lua_windows);
    json_object_set_int_member(object, "revision", revision);
    g_autoptr(GVariant) snapshot = variant_from_json(json);
    if (snapshot)
        gnoblin_config_update_window_snapshot(snapshot, revision);
    else
        gnoblin_config_update_window_snapshot(NULL, revision);
}

static void cache_lua_workspace_snapshot(GVariant* native_snapshot, guint64 revision) {
    g_autoptr(JsonNode) json = json_from_variant(native_snapshot);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        gnoblin_config_update_workspace_snapshot(NULL, revision);
        return;
    }
    JsonObject* object = json_node_get_object(json);
    JsonNode* native_workspaces = json_object_get_member(object, "workspaces");
    if (!native_workspaces || !JSON_NODE_HOLDS_ARRAY(native_workspaces)) {
        gnoblin_config_update_workspace_snapshot(NULL, revision);
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
        gnoblin_config_update_workspace_snapshot(snapshot, revision);
    else
        gnoblin_config_update_workspace_snapshot(NULL, revision);
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

static void send_response(Client* client, char* response);

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

static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {
    JsonObject* object = json_node_get_object(payload);
    const char* name = json_object_get_string_member_with_default(object, "name", NULL);
    if (!name)
        return;

    g_autoptr(JsonNode) socket_event = json_node_copy(payload);
    JsonObject* socket_object = json_node_get_object(socket_event);
    json_object_set_string_member(socket_object, "event", name);
    json_object_remove_member(socket_object, "name");
    remove_private_focus_context(socket_event);
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
        if (subscribed)
            send_response(client, g_strdup(line));
    }
    g_list_free(clients);
}

static gboolean dispatch_focus_policy_events(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->policy_event_idle_id = 0;
    guint pending = g_queue_get_length(control->policy_events);

    for (guint i = 0; i < pending && !control->stopping; i++) {
        g_autoptr(GVariant) committed = g_queue_pop_head(control->policy_events);
        if (!committed)
            break;
        const char* event_name = NULL;
        g_autoptr(GVariant) policy =
            g_variant_lookup_value(committed, "policy", G_VARIANT_TYPE_VARDICT);
        gint64 revision = 0;
        if (!policy || !g_variant_lookup(committed, "event", "&s", &event_name) ||
            !g_variant_lookup(committed, "revision", "x", &revision))
            continue;

        GVariantBuilder payload_builder;
        g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload_builder, "{sv}", "policy", policy);
        g_variant_builder_add(&payload_builder, "{sv}", "revision", g_variant_new_int64(revision));
        g_variant_builder_add(&payload_builder, "{sv}", "sequence",
                              g_variant_new_int64(++control->event_sequence));
        g_variant_builder_add(&payload_builder, "{sv}", "time",
                              g_variant_new_int64(g_get_monotonic_time()));
        g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));

        g_autoptr(JsonNode) socket_event = json_from_variant(payload);
        if (JSON_NODE_HOLDS_OBJECT(socket_event)) {
            json_object_set_string_member(json_node_get_object(socket_event), "name", event_name);
            publish_native_socket_event(control, socket_event);
        }

        meta_gnoblin_begin_native_event_batch();
        meta_display_dispatch_gnoblin_event(control->display, event_name, payload);
        meta_gnoblin_end_native_event_batch(control->display);
    }

    if (!control->stopping && !g_queue_is_empty(control->policy_events) &&
        !control->policy_event_idle_id) {
        control->policy_event_idle_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_focus_policy_events, control, NULL);
    }
    return G_SOURCE_REMOVE;
}

static void native_focus_policy_changed(GVariant* policy, guint64 revision, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !policy || !control->policy_events)
        return;

    GVariantBuilder committed_builder;
    g_variant_builder_init(&committed_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&committed_builder, "{sv}", "event",
                          g_variant_new_string("gnoblin.focus.policy-changed"));
    g_variant_builder_add(&committed_builder, "{sv}", "policy", policy);
    g_variant_builder_add(&committed_builder, "{sv}", "revision",
                          g_variant_new_int64((gint64)revision));
    g_queue_push_tail(control->policy_events,
                      g_variant_ref_sink(g_variant_builder_end(&committed_builder)));
    if (!control->policy_event_idle_id)
        control->policy_event_idle_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_focus_policy_events, control, NULL);
}

static void native_permission_policy_changed(GVariant* policy, guint64 revision,
                                             gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !policy || !control->policy_events)
        return;

    GVariantBuilder committed_builder;
    g_variant_builder_init(&committed_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&committed_builder, "{sv}", "event",
                          g_variant_new_string("gnoblin.permission.changed"));
    g_variant_builder_add(&committed_builder, "{sv}", "policy", policy);
    g_variant_builder_add(&committed_builder, "{sv}", "revision",
                          g_variant_new_int64((gint64)revision));
    g_queue_push_tail(control->policy_events,
                      g_variant_ref_sink(g_variant_builder_end(&committed_builder)));
    if (!control->policy_event_idle_id)
        control->policy_event_idle_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_focus_policy_events, control, NULL);
}

static gboolean native_config_event(MetaDisplay* display, const char* event, GVariant* document,
                                    GVariant* payload, gpointer user_data) {
    (void)display;
    (void)document;
    GnoblinNativeControl* control = user_data;
    if (!event || !g_str_equal(event, "mutter.touchpad.gesture") || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
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
    publish_native_socket_event(control, root);
    return FALSE;
}

static gboolean focus_token_has_valid_shape(const char* token) {
    if (!token || strlen(token) != 64)
        return FALSE;
    for (gsize i = 0; i < 64; i++)
        if (!g_ascii_isxdigit(token[i]))
            return FALSE;
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
        context.generation != gnoblin_config_runtime_generation() ||
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
        context.generation != gnoblin_config_runtime_generation() ||
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
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.launch.changed", payload);
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
        gnoblin_config_update_launch_snapshot(value, control->launch_revision);
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
        meta_display_dispatch_gnoblin_event(control->display, event, payload);
}

static void dispatch_lua_input_sources_changed(GnoblinNativeControl* control, guint64 revision) {
    g_autoptr(GVariant) snapshot = input_source_snapshot(control);
    gnoblin_config_update_input_source_snapshot(snapshot, revision);
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
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.input.sources-changed",
                                            payload);
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
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.input.source-changed",
                                            payload);
}

static void publish_input_source_changes(GnoblinNativeControl* control, guint64 revision) {
    gboolean sources_changed = refresh_input_sources(control, NULL);
    g_autoptr(GVariant) snapshot = input_source_snapshot(control);
    gnoblin_config_update_input_source_snapshot(snapshot, revision);
    if (sources_changed && control->input_source_state_initialized)
        dispatch_lua_input_sources_changed(control, revision);
    NativeInputSource* current = current_input_source(control);
    const char* current_id = current ? current->id : NULL;
    if (control->pending_input_source_ops == 0) {
        if (control->input_source_state_initialized &&
            g_strcmp0(control->last_published_input_source, current_id) != 0)
            dispatch_lua_input_source_changed(control, revision, current);
        g_free(control->last_published_input_source);
        control->last_published_input_source = g_strdup(current_id);
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
        meta_display_dispatch_gnoblin_event(control->display, event, payload);
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
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.workspace.changed", payload);
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
        meta_display_dispatch_gnoblin_event(control->display, "gnoblin.workspace.window-moved",
                                            payload);
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
        gnoblin_config_update_workspace_snapshot(NULL, control->state_revision);
        return NULL;
    }
    cache_lua_workspace_snapshot(result, control->state_revision);
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
            gnoblin_config_update_monitor_snapshot(NULL, control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            gnoblin_config_update_monitor_snapshot(NULL, control->state_revision);
        return NULL;
    }
    JsonArray* monitors = json_object_get_array_member(json_node_get_object(json), "monitors");
    if (!monitors) {
        if (update_lua_snapshot)
            gnoblin_config_update_monitor_snapshot(NULL, control->state_revision);
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
        gnoblin_config_update_monitor_snapshot(snapshot, control->state_revision);
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
        meta_display_dispatch_gnoblin_event(control->display, event, payload);
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
        json_object_set_member(object, "window", lua_window_record(window));
    } else if (g_str_equal(event, "gnoblin.window.closed")) {
        json_object_set_string_member(object, "window_id", id);
        json_object_set_member(object, "last", lua_window_record(last));
    } else {
        json_object_set_string_member(object, "window_id", id);
        JsonNode* record = lua_window_record(window);
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
        meta_display_dispatch_gnoblin_event(control->display, event, payload);
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
        meta_gnoblin_begin_native_event_batch();
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
    if (dispatch_events)
        meta_gnoblin_end_native_event_batch(control->display);
}

static JsonNode* shortcut_actions_snapshot(JsonObject* arguments, GError** error) {
    static const struct {
        const char* group;
        const char* schema_id;
    } schemas[] = {
        {"wm", "org.gnome.desktop.wm.keybindings"},
        {"mutter", "org.gnome.mutter.keybindings"},
        {"wayland", "org.gnome.mutter.wayland.keybindings"},
    };
    const char* requested_group = NULL;
    if (arguments && json_object_get_size(arguments)) {
        if (json_object_get_size(arguments) != 1 || !json_object_has_member(arguments, "group")) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.actions accepts only the optional group argument");
            return NULL;
        }
        JsonNode* group_node = json_object_get_member(arguments, "group");
        if (!JSON_NODE_HOLDS_VALUE(group_node) ||
            json_node_get_value_type(group_node) != G_TYPE_STRING) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "shortcut.actions group must be a string");
            return NULL;
        }
        requested_group = json_node_get_string(group_node);
    }
    if (requested_group) {
        gboolean known = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(schemas); i++)
            known |= g_str_equal(requested_group, schemas[i].group);
        if (!known) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                "unknown shortcut action group; expected 'wm', 'mutter', or 'wayland'");
            return NULL;
        }
    }

    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    if (!source) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "GSettings schema source is unavailable");
        return NULL;
    }
    JsonNode* result = json_node_new(JSON_NODE_ARRAY);
    JsonArray* actions = json_array_new();
    json_node_take_array(result, actions);
    for (guint group_index = 0; group_index < G_N_ELEMENTS(schemas); group_index++) {
        const char* group = schemas[group_index].group;
        if (requested_group && !g_str_equal(requested_group, group))
            continue;
        g_autoptr(GSettingsSchema) schema =
            g_settings_schema_source_lookup(source, schemas[group_index].schema_id, TRUE);
        if (!schema) {
            if (requested_group) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "shortcut schema is not installed for group '%s'", group);
                g_clear_pointer(&result, json_node_unref);
                return NULL;
            }
            continue;
        }
        g_auto(GStrv) keys = g_settings_schema_list_keys(schema);
        if (!keys)
            continue;
        g_strv_sort(keys);
        for (guint key_index = 0; keys[key_index]; key_index++) {
            g_autoptr(GSettingsSchemaKey) key = g_settings_schema_get_key(schema, keys[key_index]);
            if (!key || !g_variant_type_equal(g_settings_schema_key_get_value_type(key),
                                              G_VARIANT_TYPE_STRING_ARRAY))
                continue;
            g_autoptr(GVariant) defaults = g_settings_schema_key_get_default_value(key);
            if (!defaults || !g_variant_is_of_type(defaults, G_VARIANT_TYPE_STRING_ARRAY))
                continue;

            g_autofree char* public_key = g_strdup(keys[key_index]);
            g_strdelimit(public_key, "-", '_');
            g_autofree char* id = g_strdup_printf("%s.%s", group, public_key);
            JsonObject* action = json_object_new();
            json_object_set_string_member(action, "id", id);
            json_object_set_string_member(action, "group", group);
            json_object_set_string_member(action, "key", public_key);
            const char* description = g_settings_schema_key_get_description(key);
            if (description && *description)
                json_object_set_string_member(action, "description", description);
            JsonArray* bindings = json_array_new();
            for (gsize binding_index = 0; binding_index < g_variant_n_children(defaults);
                 binding_index++) {
                g_autoptr(GVariant) binding = g_variant_get_child_value(defaults, binding_index);
                json_array_add_string_element(bindings, g_variant_get_string(binding, NULL));
            }
            json_object_set_array_member(action, "default_bindings", bindings);
            JsonNode* action_node = json_node_new(JSON_NODE_OBJECT);
            json_node_take_object(action_node, action);
            json_array_add_element(actions, action_node);
        }
    }
    return result;
}

static gboolean dynamic_shortcut_id_valid(const char* id) {
    if (!id || !*id || strlen(id) > 64)
        return FALSE;
    for (const char* cursor = id; *cursor; cursor++)
        if (!g_ascii_isalnum(*cursor) && *cursor != '_' && *cursor != '-')
            return FALSE;
    return TRUE;
}

static gboolean dynamic_shortcut_accelerator_valid(const char* accelerator) {
    if (!accelerator || !*accelerator || strlen(accelerator) > 128 ||
        !g_utf8_validate(accelerator, -1, NULL) || g_str_equal(accelerator, "Super"))
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
        if (g_str_equal(name, "hold") || g_str_equal(name, "modal") ||
            g_str_equal(name, "captureInput") || g_str_equal(name, "capture_input") ||
            g_str_equal(name, "capture-input") || g_str_equal(name, "trigger")) {
            valid = FALSE;
            break;
        }
        if (!g_str_equal(name, first) && (!second || !g_str_equal(name, second))) {
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
                               "shortcut.bind accepts only id and accelerator; hold, modal, "
                               "trigger, and captureInput are unsupported");
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
                               "string of at most 128 bytes; bare Super is unsupported");
    const char* id = json_node_get_string(id_node);
    const char* accelerator = json_node_get_string(accelerator_node);
    GnoblinNativeControl* control = client->control;
    if (find_dynamic_shortcut(client, id))
        return encode_response(request_id, NULL, "shortcut.bind id is already registered");
    if (dynamic_shortcut_count(control, client) >= MAX_DYNAMIC_SHORTCUTS_PER_CLIENT ||
        dynamic_shortcut_count(control, NULL) >= MAX_DYNAMIC_SHORTCUTS)
        return encode_response(request_id, NULL, "dynamic shortcut registration limit reached");

    guint action = meta_display_grab_accelerator(control->display, accelerator,
                                                 META_KEY_BINDING_IGNORE_AUTOREPEAT);
    if (action == META_KEYBINDING_ACTION_NONE)
        return encode_response(request_id, NULL,
                               "shortcut.bind accelerator is invalid or already claimed");
    NativeDynamicShortcut* shortcut = g_new0(NativeDynamicShortcut, 1);
    shortcut->client = client;
    shortcut->id = g_strdup(id);
    shortcut->accelerator = g_strdup(accelerator);
    shortcut->action = action;
    g_hash_table_insert(control->dynamic_shortcuts, GUINT_TO_POINTER(action), shortcut);

    JsonObject* result_object = json_object_new();
    json_object_set_string_member(result_object, "id", id);
    json_object_set_string_member(result_object, "accelerator", accelerator);
    json_object_set_string_member(result_object, "trigger", "press");
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(request_id, result, NULL);
}

static char* dynamic_shortcut_unbind(Client* client, const char* request_id,
                                     JsonObject* arguments) {
    if (!client || client->closing || !client->control || !arguments)
        return encode_response(request_id, NULL, "shortcut.unbind requires a live connection");
    if (!dynamic_shortcut_arguments_have_only(arguments, "id", NULL))
        return encode_response(request_id, NULL,
                               "shortcut.unbind accepts only id; hold, modal, trigger, and "
                               "captureInput are unsupported");
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

static JsonNode* permission_decision_json(GVariant* document, const char* capability,
                                          const char* identity, GError** error) {
    if (!gnoblin_permission_capability_supported(capability)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "unsupported permission capability");
        return NULL;
    }
    g_auto(GnoblinPermission) decision =
        gnoblin_permission_policy_evaluate(document, capability, identity);
    JsonObject* object = json_object_new();
    json_object_set_string_member(object, "level",
                                  decision.level == GNOBLIN_PERMISSION_ASK     ? "ask"
                                  : decision.level == GNOBLIN_PERMISSION_ALLOW ? "allow"
                                  : decision.level == GNOBLIN_PERMISSION_DENY  ? "deny"
                                                                               : "default");
    json_object_set_string_member(object, "rule", decision.rule ? decision.rule : "");
    JsonArray* monitors = json_array_new();
    for (guint i = 0; decision.monitors && decision.monitors[i]; i++)
        json_array_add_string_element(monitors, decision.monitors[i]);
    json_object_set_array_member(object, "monitors", monitors);
    json_object_set_int_member(object, "devices", decision.devices);
    json_object_set_boolean_member(object, "clipboard", decision.clipboard);
    JsonNode* result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, object);
    return result;
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
            client->close_after_response = TRUE;
            return encode_response("", NULL,
                                   "API version must contain integer major and minor fields");
        }
        gint64 major = json_node_get_int(major_node);
        gint64 minor = json_node_get_int(minor_node);
        if (major != GNOBLIN_NATIVE_CONTROL_API_MAJOR || minor < 0 ||
            minor > GNOBLIN_NATIVE_CONTROL_API_MINOR) {
            client->close_after_response = TRUE;
            return encode_response("", NULL, "requested compositor API version is unsupported");
        }
        client->api_minor = minor;
    }
    const char* op = json_object_get_string_member_with_default(request, "op", "");
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
            if (g_str_equal(name, "gnoblin.focus.policy-changed") && client->api_minor < 13) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "focus policy events require API version 1.13");
            }
            if (g_str_equal(name, "gnoblin.permission.changed") && client->api_minor < 16) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "permission policy events require API version 1.16");
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
            if (g_hash_table_contains(subscriptions, name)) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events must not contain duplicate names");
            }
            g_hash_table_add(subscriptions, g_strdup(name));
        }
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
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
        return encode_response(id, NULL, "native compositor supports only API requests");
    if (g_str_equal(method, "layer.list") && client->api_minor < 2)
        return encode_response(id, NULL, "layer.list requires API version 1.2");
    if (g_str_equal(method, "input.devices") && client->api_minor < 3)
        return encode_response(id, NULL, "input.devices requires API version 1.3");
    if ((g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source") ||
         g_str_equal(method, "input.select_source") || g_str_equal(method, "input.select")) &&
        client->api_minor < 6)
        return encode_response(id, NULL, "input source methods require API version 1.6");
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
    if ((g_str_equal(method, "grant.list") || g_str_equal(method, "grant.revoke")) &&
        client->api_minor < 14)
        return encode_response(id, NULL, "portal grant methods require API version 1.14");
    if (g_str_equal(method, "portals.grants") && client->api_minor < 15)
        return encode_response(id, NULL, "portals.grants requires API version 1.15");
    if (g_str_equal(method, "permissions.policy") && client->api_minor < 16)
        return encode_response(id, NULL, "permissions.policy requires API version 1.16");
    if (g_str_has_prefix(method, "launch.") && client->api_minor < 7)
        return encode_response(id, NULL, "launch methods require API version 1.7");
    if (g_str_equal(method, "shell.ping")) {
        JsonObject* pong = json_object_new();
        json_object_set_string_member(pong, "pong", "pong");
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, pong);
        return encode_response(id, result, NULL);
    }
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (arguments_node && !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return encode_response(id, NULL, "arguments must be an object");
    if (g_str_equal(method, "portals.grants")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        const char* kind_filter = NULL;
        if (arguments && json_object_get_size(arguments) > 1)
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
        if (!client->control->portal_grant_snapshot)
            return encode_response(id, NULL, "native portal grant snapshot is unavailable");
        g_autoptr(GVariant) grants = g_variant_lookup_value(client->control->portal_grant_snapshot,
                                                            "grants", G_VARIANT_TYPE("aa{sv}"));
        g_autoptr(JsonNode) json = grants ? json_from_variant(grants) : NULL;
        if (!JSON_NODE_HOLDS_ARRAY(json))
            return encode_response(id, NULL, "native portal grant snapshot is invalid");
        JsonArray* result_array = json_array_new();
        JsonArray* source = json_node_get_array(json);
        for (guint i = 0; i < json_array_get_length(source); i++) {
            JsonNode* record = json_array_get_element(source, i);
            if (kind_filter &&
                !g_str_equal(kind_filter, json_object_get_string_member_with_default(
                                              json_node_get_object(record), "kind", "")))
                continue;
            json_array_add_element(result_array, json_node_copy(record));
        }
        g_autoptr(JsonNode) result_array_node = json_node_new(JSON_NODE_ARRAY);
        json_node_take_array(result_array_node, result_array);
        return encode_response(id, result_array_node, NULL);
    }
    if (g_str_equal(method, "permissions.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.list does not accept arguments");
        g_autoptr(GVariant) document = gnoblin_config_current_document();
        if (!document)
            return encode_response(id, NULL, "committed permission policy is unavailable");
        g_autofree char* config_path = gnoblin_config_path();
        g_autoptr(GVariant) snapshot = gnoblin_permission_policy_list(document, config_path);
        g_autoptr(JsonNode) json = json_from_variant(snapshot);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "permissions.policy")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.policy does not accept arguments");
        g_autoptr(GVariant) document = gnoblin_config_current_document();
        if (!document)
            return encode_response(id, NULL, "committed permission policy is unavailable");
        g_autoptr(GVariant) snapshot =
            gnoblin_permission_policy_snapshot(document, gnoblin_config_settings_revision());
        g_autoptr(JsonNode) json = snapshot ? json_from_variant(snapshot) : NULL;
        if (!json)
            return encode_response(id, NULL, "committed permission policy is unavailable");
        return encode_response(id, json, NULL);
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
        const char* capability =
            json_node_get_string(json_object_get_member(arguments, "capability"));
        const char* identity = json_node_get_string(json_object_get_member(arguments, "identity"));
        g_autoptr(GVariant) document = gnoblin_config_current_document();
        if (!document)
            return encode_response(id, NULL, "committed permission policy is unavailable");
        g_autoptr(GError) permission_error = NULL;
        g_autoptr(JsonNode) result =
            permission_decision_json(document, capability, identity, &permission_error);
        if (!result)
            return encode_response(
                id, NULL, permission_error ? permission_error->message : "permission check failed");
        return encode_response(id, result, NULL);
    }
    if (g_str_equal(method, "window.action") && arguments_node) {
        JsonNode* action_node =
            json_object_get_member(json_node_get_object(arguments_node), "action");
        if (action_node && JSON_NODE_HOLDS_VALUE(action_node) &&
            json_node_get_value_type(action_node) == G_TYPE_STRING &&
            g_str_equal(json_node_get_string(action_node), "focus"))
            return encode_response(
                id, NULL,
                "native window.action focus is denied; use window.focus with a live focus_context");
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
        g_autoptr(GVariant) result =
            native_socket_begin_window_grab(client, method, arguments, &error);
        if (!result)
            return encode_response(
                id, NULL, error ? error->message : "interactive window operation was denied");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "shortcut.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "shortcut.list does not accept arguments");
        g_autoptr(GVariant) snapshot = native_shortcut_snapshot(client->control);
        g_autoptr(JsonNode) json = json_from_variant(snapshot);
        JsonArray* records = json_object_get_array_member(json_node_get_object(json), "shortcuts");
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_ARRAY);
        json_node_set_array(result, json_array_ref(records));
        return encode_response(id, result, NULL);
    }
    if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        return g_str_equal(method, "shortcut.bind")
                   ? dynamic_shortcut_bind(client, id, arguments)
                   : dynamic_shortcut_unbind(client, id, arguments);
    }
    if (g_str_has_prefix(method, "launch.")) {
        client->track_launches = TRUE;
        client->launch_api_minor = client->api_minor;
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) launch_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
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
        g_autoptr(GVariant) snapshot = input_device_snapshot(client->control);
        g_autoptr(JsonNode) json = json_from_variant(snapshot);
        if (client->api_minor >= 4) {
            client->track_input_devices = TRUE;
            client->input_devices_api_minor = client->api_minor;
        }
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "input source reads do not accept arguments");
        client->track_input_sources = TRUE;
        client->input_sources_api_minor = client->api_minor;
        publish_input_source_changes(client->control, client->control->state_revision);
        g_autoptr(GVariant) snapshot = input_source_snapshot(client->control);
        g_autoptr(JsonNode) json = json_from_variant(snapshot);
        if (g_str_equal(method, "input.current_source")) {
            JsonObject* response = json_object_new();
            json_object_set_boolean_member(
                response, "available",
                json_object_has_member(json_node_get_object(json), "current"));
            if (json_object_has_member(json_node_get_object(json), "current"))
                json_object_set_member(
                    response, "source",
                    json_node_copy(json_object_get_member(json_node_get_object(json), "current")));
            g_autoptr(JsonNode) current = json_node_new(JSON_NODE_OBJECT);
            json_node_take_object(current, response);
            json_object_set_int_member(response, "revision", client->control->state_revision);
            return encode_response(id, current, NULL);
        }
        JsonObject* response = json_object_new();
        json_object_set_member(
            response, "sources",
            json_node_copy(json_object_get_member(json_node_get_object(json), "sources")));
        json_object_set_int_member(response, "revision", client->control->state_revision);
        g_autoptr(JsonNode) sources = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(sources, response);
        return encode_response(id, sources, NULL);
    }
    if (g_str_equal(method, "shortcut.actions")) {
        g_autoptr(JsonNode) json = shortcut_actions_snapshot(
            arguments_node ? json_node_get_object(arguments_node) : NULL, &error);
        if (!json)
            return encode_response(id, NULL,
                                   error ? error->message : "shortcut action listing unavailable");
        return encode_response(id, json, NULL);
    }
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = arguments_node
                                        ? variant_from_json(arguments_node)
                                        : g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) operation = meta_gnoblin_call_config_api(method, arguments, &error);
    if (!operation)
        return encode_response(id, NULL, error ? error->message : "unsupported API method");
    const char* native_method = NULL;
    if (!g_variant_lookup(operation, "method", "&s", &native_method))
        return encode_response(id, NULL, "invalid Lua operation descriptor");
    g_autoptr(GVariant) native_arguments =
        g_variant_lookup_value(operation, "arguments", G_VARIANT_TYPE_VARDICT);
    if (!native_arguments)
        return encode_response(id, NULL, "invalid Lua operation arguments");
    if (g_str_equal(native_method, "input.select")) {
        client->track_input_sources = TRUE;
        client->input_sources_api_minor = client->api_minor;
        gint64 request_id = 0;
        g_variant_lookup(operation, "request_id", "x", &request_id);
        if (!gnoblin_native_control_select_input_source(client->control->display, native_arguments,
                                                        request_id, method, &error))
            dispatch_input_source_operation(client->control, request_id, method, FALSE, NULL,
                                            native_operation_error_code(error),
                                            error ? error->message : "XKB source selection failed");
        g_autoptr(JsonNode) pending = json_from_variant(operation);
        return encode_response(id, pending, NULL);
    }
    if (g_str_equal(native_method, "shortcut.capture")) {
        client->track_shortcut_capture = TRUE;
        client->shortcut_capture_api_minor = client->api_minor;
        gint64 request_id = 0;
        g_variant_lookup(operation, "request_id", "x", &request_id);
        if (!gnoblin_native_control_begin_shortcut_capture(client->control->display,
                                                           native_arguments, request_id, &error))
            return encode_response(id, NULL,
                                   error ? error->message : "shortcut capture could not start");
        g_autoptr(JsonNode) pending = json_from_variant(operation);
        return encode_response(id, pending, NULL);
    }
    if (g_str_equal(native_method, "grant.list") || g_str_equal(native_method, "grant.revoke")) {
        gint64 request_id = 0;
        g_variant_lookup(operation, "request_id", "x", &request_id);
        g_autofree char* operation_key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
        g_hash_table_add(client->pending_grant_operations, g_strdup(operation_key));
        if (!gnoblin_native_control_portal_grant_operation(client->control->display, native_method,
                                                           native_arguments, request_id, &error))
            dispatch_operation_completion(client->control, request_id, method, FALSE, NULL,
                                          native_operation_error_code(error),
                                          error ? error->message
                                                : "portal grant operation could not start");
        g_autoptr(JsonNode) pending = json_from_variant(operation);
        return encode_response(id, pending, NULL);
    }
    g_autoptr(GVariant) result = meta_gnoblin_dispatch_native_api(
        client->control->display, native_method, native_arguments, &error);
    if (!result)
        return encode_response(id, NULL,
                               error ? error->message : "method still needs Gnoblin Shell");
    g_autoptr(JsonNode) json = json_from_variant(result);
    if (g_str_equal(method, "window.list") && JSON_NODE_HOLDS_OBJECT(json))
        json_object_set_int_member(json_node_get_object(json), "revision",
                                   client->control->state_revision);
    if (g_str_equal(method, "layer.list") && JSON_NODE_HOLDS_OBJECT(json)) {
        JsonObject* object = json_node_get_object(json);
        JsonArray* layers = json_object_get_array_member(object, "layers");
        json_object_set_int_member(object, "revision", client->control->state_revision);
        for (guint i = 0; layers && i < json_array_get_length(layers); i++) {
            JsonNode* record = json_array_get_element(layers, i);
            if (JSON_NODE_HOLDS_OBJECT(record))
                json_object_set_int_member(json_node_get_object(record), "revision",
                                           client->control->state_revision);
        }
    }
    return encode_response(id, json, NULL);
}

static void process_buffer(Client* client);
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
    gnoblin_config_update_capability_snapshot(capabilities, revision);
    g_autoptr(GVariant) input_devices = input_device_snapshot(control);
    gnoblin_config_update_input_device_snapshot(input_devices, revision);
    publish_input_source_changes(control, revision);
    g_autoptr(JsonNode) input_devices_json = json_from_variant(input_devices);
    gboolean monitor_changed = FALSE;
    meta_gnoblin_begin_native_event_batch();
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
    meta_gnoblin_end_native_event_batch(control->display);
    return G_SOURCE_REMOVE;
}

static void schedule_windows(GnoblinNativeControl* control) {
    if (!control->publish_id) {
        control->state_revision++;
        control->publish_id = g_idle_add(publish_windows, control);
    }
}

static void window_changed(MetaWindow* window, gpointer user_data) {
    schedule_windows(user_data);
}

static void window_workspace_changed(MetaWindow* window, gpointer user_data) {
    schedule_windows(user_data);
}

static void window_notified(GObject* window, GParamSpec* property, gpointer user_data) {
    schedule_windows(user_data);
}

static void window_unmanaged(MetaWindow* window, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    g_signal_handlers_disconnect_by_data(window, control);
    g_hash_table_remove(control->windows, window);
    schedule_windows(control);
}

static void track_window(GnoblinNativeControl* control, MetaWindow* window) {
    if (g_hash_table_contains(control->windows, window))
        return;
    g_hash_table_add(control->windows, g_object_ref(window));
    g_signal_connect(window, "notify", G_CALLBACK(window_notified), control);
    g_signal_connect(window, "position-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "size-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "workspace-changed", G_CALLBACK(window_workspace_changed), control);
    g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), control);
    schedule_windows(control);
}

static void window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data) {
    track_window(user_data, window);
}

static void display_notified(GObject* display, GParamSpec* property, gpointer user_data) {
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

static void input_source_settings_changed(GSettings* settings, const char* key,
                                          gpointer user_data) {
    (void)settings;
    (void)key;
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
    workspace_manager_changed(manager, user_data);
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
        "input.devices",
        "input.sources",
        "input.current_source",
        "input.select",
        "launch.status",
        "launch.begin",
        "launch.end",
        "layer.list",
        "monitor.list",
        "shortcut.actions",
        "shortcut.bind",
        "shortcut.capture",
        "shortcut.list",
        "shortcut.unbind",
        "permissions.list",
        "permissions.policy",
        "permissions.check",
        "grant.list",
        "grant.revoke",
        "portals.grants",
        "window.list",
        "window.action",
        "window.close",
        "window.minimize",
        "window.toggle_minimize",
        "window.restore",
        "window.set_maximized",
        "window.set_fullscreen",
        "window.set_above",
        "window.set_sticky",
        "window.move",
        "window.resize",
        "window.move_to_workspace",
        "window.move_to_monitor",
        "window.focus",
        "window.begin_move",
        "window.begin_resize",
        "workspace.list",
        "workspace.create",
        "workspace.rename",
        "workspace.remove",
        "workspace.switch",
        "workspace.next",
        "workspace.previous",
        "workspace.move_active",
        "workspace.move_window",
        NULL,
    };
    GnoblinNativeControl* control = user_data;
    Client* client = g_new0(Client, 1);
    client->control = control;
    client->connection = g_object_ref(connection);
    client->request = g_string_new(NULL);
    client->outgoing = g_queue_new();
    client->focus_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
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
    json_object_set_string_member(hello, "revision_scope",
                                  "window-workspace-monitor-layer-input-device-source-state");
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

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   GError** error) {
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
    control->clients = g_hash_table_new(g_direct_hash, g_direct_equal);
    control->dynamic_shortcuts =
        g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, native_dynamic_shortcut_free);
    control->focus_contexts = g_hash_table_new_full(g_int64_hash, g_int64_equal, g_free, g_free);
    control->policy_events = g_queue_new();
    control->focus_context_timeout_id =
        g_timeout_add_seconds(1, focus_context_expiry_tick, control);
    control->launches = g_ptr_array_new_with_free_func(native_launch_free);
    update_launch_snapshot(control);
    control->wayland_compositor = meta_context_get_wayland_compositor(context);
    if (control->wayland_compositor)
        control->session_lock_callback_id = meta_wayland_session_lock_add_state_changed_callback(
            control->wayland_compositor, native_session_lock_changed, control, NULL);
    control->windows = g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    control->window_state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, native_window_state_free);
    control->input_device_ids = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, g_free);
    g_autoptr(GSocketAddress) bind_address = NULL;
    g_autoptr(GList) windows = NULL;
    control->display = meta_context_get_display(context);
    meta_gnoblin_set_native_config_display(control->display);
    g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, control);
    MetaBackend* backend = meta_context_get_backend(context);
    control->backend = backend;
    control->monitor_manager = meta_backend_get_monitor_manager(backend);
    control->input_seat = meta_backend_get_default_seat(backend);
    control->workspace_manager = meta_display_get_workspace_manager(control->display);
    control->path = g_strdup(g_getenv("GNOBLIN_COMPOSITOR_SOCKET"));
    if (!control->path)
        control->path = g_build_filename(directory, "compositor-v1.sock", NULL);
    g_autofree char* socket_directory = g_path_get_dirname(control->path);
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
        if (!S_ISSOCK(existing.st_mode) || existing.st_uid != getuid()) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "refusing to replace %s",
                        control->path);
            goto fail;
        }
        g_autoptr(GSocketClient) probe = g_socket_client_new();
        g_socket_client_set_timeout(probe, 1);
        g_autoptr(GSocketAddress) address = g_unix_socket_address_new(control->path);
        g_autoptr(GError) probe_error = NULL;
        g_autoptr(GSocketConnection) active =
            g_socket_client_connect(probe, G_SOCKET_CONNECTABLE(address), NULL, &probe_error);
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
    start_native_policy_dbus(control); /* D-Bus absence keeps portal grants fail closed. */
    g_signal_connect(control->display, "gnoblin-config-event", G_CALLBACK(native_config_event),
                     control);
    gnoblin_config_set_focus_policy_changed_callback(native_focus_policy_changed, control);
    gnoblin_config_set_permission_policy_changed_callback(native_permission_policy_changed,
                                                          control);
    g_signal_connect(control->display, "window-created", G_CALLBACK(window_created), control);
    g_signal_connect(control->display, "notify::focus-window", G_CALLBACK(display_notified),
                     control);
    g_signal_connect(control->display, "restacked", G_CALLBACK(display_restacked), control);
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
    GSettingsSchema* input_schema =
        schema_source ? g_settings_schema_source_lookup(schema_source,
                                                        "org.gnome.desktop.input-sources", TRUE)
                      : NULL;
    if (input_schema) {
        control->input_source_settings = g_settings_new_full(input_schema, NULL, NULL);
        g_settings_schema_unref(input_schema);
        g_signal_connect(control->input_source_settings, "changed::sources",
                         G_CALLBACK(input_source_settings_changed), control);
    }
    refresh_input_sources(control, document);
    windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next)
        track_window(control, item->data);
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) initial_snapshot = window_snapshot_json(control, TRUE, &snapshot_error);
    if (!initial_snapshot)
        g_warning("gnoblin-native-control: cannot seed Lua window snapshot: %s",
                  snapshot_error ? snapshot_error->message : "window listing unavailable");
    g_autoptr(GError) workspace_error = NULL;
    g_autoptr(JsonNode) initial_workspaces = workspace_snapshot_json(control, &workspace_error);
    if (!initial_workspaces)
        g_warning("gnoblin-native-control: cannot seed Lua workspace snapshot: %s",
                  workspace_error ? workspace_error->message : "workspace listing unavailable");
    else {
        control->workspace_state = workspace_state_from_snapshot(initial_workspaces);
        control->workspace_state_initialized = TRUE;
    }
    g_autoptr(GError) monitor_error = NULL;
    g_autoptr(JsonNode) initial_monitors = monitor_snapshot_json(control, TRUE, &monitor_error);
    if (!initial_monitors)
        g_warning("gnoblin-native-control: cannot seed monitor snapshot: %s",
                  monitor_error ? monitor_error->message : "monitor listing unavailable");
    else {
        control->monitor_state = monitor_state_from_snapshot(initial_monitors);
        control->monitor_state_initialized = TRUE;
    }
    g_autoptr(GError) layer_error = NULL;
    if (!layer_snapshot_json(control, TRUE, &layer_error))
        g_warning("gnoblin-native-control: cannot seed Lua layer snapshot: %s",
                  layer_error ? layer_error->message : "layer listing unavailable");
    g_autoptr(GVariant) capabilities = capability_snapshot(control);
    gnoblin_config_update_capability_snapshot(capabilities, control->state_revision);
    g_autoptr(GVariant) input_devices = input_device_snapshot(control);
    gnoblin_config_update_input_device_snapshot(input_devices, control->state_revision);
    g_autoptr(GVariant) input_sources = input_source_snapshot(control);
    gnoblin_config_update_input_source_snapshot(input_sources, control->state_revision);
    control->input_source_state_initialized = TRUE;
    NativeInputSource* initial_current_source = current_input_source(control);
    control->last_published_input_source =
        initial_current_source ? g_strdup(initial_current_source->id) : NULL;
    g_autoptr(JsonNode) initial_input_devices = json_from_variant(input_devices);
    control->input_device_state = input_device_state_from_snapshot(initial_input_devices);
    control->input_device_state_initialized = TRUE;
    schedule_windows(control);
    if (control->input_sources && control->input_sources->len > 0 &&
        !control->input_keymap_description) {
        NativeInputSource* first = g_ptr_array_index(control->input_sources, 0);
        if (!select_xkb_source(control, first->id, 0, "input.select_source", error))
            goto fail;
    }
    if (!apply_native_input(control, context, document, error))
        goto fail;
    if (!apply_native_keybindings(document, error))
        goto fail;
    if (!start_native_shortcuts(control, document, error))
        goto fail;
    if (!start_native_autostart(document, error))
        goto fail;
    return control;

fail:
    gnoblin_native_control_stop(control);
    return NULL;
}

void gnoblin_native_control_stop(GnoblinNativeControl* control) {
    if (!control)
        return;
    control->stopping = TRUE;
    stop_native_policy_dbus(control);
    if (control->display)
        meta_display_set_gnoblin_shortcut_activated_handler(control->display, NULL, NULL);
    gnoblin_config_set_focus_policy_changed_callback(NULL, NULL);
    gnoblin_config_set_permission_policy_changed_callback(NULL, NULL);
    if (control->policy_event_idle_id) {
        g_source_remove(control->policy_event_idle_id);
        control->policy_event_idle_id = 0;
    }
    if (control->policy_events) {
        g_queue_free_full(control->policy_events, (GDestroyNotify)g_variant_unref);
        control->policy_events = NULL;
    }
    revoke_focus_contexts(control);
    meta_gnoblin_set_native_config_display(NULL);
    stop_native_shortcut_capture(control, FALSE, FALSE, NULL, NULL, NULL);
    if (control->wayland_compositor && control->session_lock_callback_id) {
        meta_wayland_session_lock_remove_state_changed_callback(control->wayland_compositor,
                                                                control->session_lock_callback_id);
        control->session_lock_callback_id = 0;
    }
    if (control->display &&
        g_object_get_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY) == control)
        g_object_set_data(G_OBJECT(control->display), NATIVE_CONTROL_OBJECT_DATA_KEY, NULL);
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
        while (g_hash_table_iter_next(&window_iter, &window, NULL))
            g_signal_handlers_disconnect_by_data(window, control);
        g_hash_table_unref(control->windows);
    }
    if (control->workspace_manager)
        g_signal_handlers_disconnect_by_data(control->workspace_manager, control);
    if (control->monitor_manager)
        g_signal_handlers_disconnect_by_data(control->monitor_manager, control);
    if (control->backend)
        g_signal_handlers_disconnect_by_data(control->backend, control);
    if (control->input_source_settings)
        g_signal_handlers_disconnect_by_data(control->input_source_settings, control);
    if (control->input_seat)
        g_signal_handlers_disconnect_by_data(control->input_seat, control);
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
    control->path = NULL;
    native_control_maybe_free_stopped(control);
}
