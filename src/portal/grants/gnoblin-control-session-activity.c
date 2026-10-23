/* Session activity: the idle and activity snapshot, its change events and the session idle query.
 */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#define SESSION_ACTIVITY_BUS_NAME "org.freedesktop.ScreenSaver"
#define SESSION_ACTIVITY_PATH "/org/gnoblin/SessionIdle"
#define SESSION_ACTIVITY_INTERFACE "org.gnoblin.SessionIdle"

void gnoblin_control_activity_publish(GnoblinNativeControl* control, gboolean available,
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
    gnoblin_control_publish_runtime_snapshot(control, "session-activity", snapshot,
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
    gnoblin_control_dispatch_event(control, "gnoblin.session.activity-changed", event_payload);

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
    gnoblin_control_publish_socket_event(control, event);
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
        gnoblin_control_activity_publish(control, available, idle, threshold_ms, idle_for_ms);
    } else if (!control->stopping && query->generation == control->session_activity_generation) {
        gnoblin_control_activity_publish(control, FALSE, FALSE, 120000, 0);
    }
    if (control->pending_activity_queries)
        control->pending_activity_queries--;
    g_free(query);
    gnoblin_control_maybe_free_stopped(control);
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
    gnoblin_control_activity_publish(control, available, idle, threshold_ms, idle_for_ms);
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
            gnoblin_control_activity_publish(control, FALSE, FALSE, 120000, 0);
    }
}

void gnoblin_control_activity_watch(GnoblinNativeControl* control) {
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
    gnoblin_control_activity_publish(control, FALSE, FALSE, 120000, 0);
    native_activity_query(control);
}
