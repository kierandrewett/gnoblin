/* Portal grants and session activity: the cached portal grant snapshot and its change events,
 * the grant.list and grant.revoke operations, and the session idle and activity query. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#define PORTAL_GRANT_TIMEOUT_MS 5000

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
    gnoblin_control_publish_runtime_snapshot(control, "portal-grants",
                                             control->portal_grant_snapshot, revision);
}

static void portal_grant_cache_apply(GnoblinNativeControl* control, const char* changed_id,
                                     const char* changed_kind, GVariant* replacement) {
    g_autoptr(GVariant) current = control->portal_grant_snapshot
                                      ? g_variant_lookup_value(control->portal_grant_snapshot,
                                                               "grants", G_VARIANT_TYPE("aa{sv}"))
                                      : NULL;
    if (!current) {
        control->portal_grant_revision++;
        gnoblin_control_publish_runtime_snapshot(control, "portal-grants", NULL,
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
    gnoblin_control_publish_runtime_snapshot(control, "portal-grants",
                                             control->portal_grant_snapshot, revision);
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
    gnoblin_control_dispatch_event(control, event, enriched_payload);
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(enriched_payload);
    if (!JSON_NODE_HOLDS_OBJECT(json))
        return;
    gnoblin_control_publish_socket_event(control, json);
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
    gnoblin_control_maybe_free_stopped(control);
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
        gnoblin_control_publish_runtime_snapshot(control, "portal-grants", NULL,
                                        control->portal_grant_revision);
    } else if (!g_str_equal(old_owner, new_owner)) {
        control->portal_grant_retry_count = 0;
        portal_grant_fetch_snapshot(control);
    }
}

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

void gnoblin_control_portal_grants_watch(GnoblinNativeControl* control) {
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

static void pending_portal_grant_operation_free(PendingPortalGrantOperation* pending) {
    if (!pending)
        return;
    g_free(pending->method);
    g_free(pending->kind);
    g_free(pending->id);
    g_free(pending);
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
        gnoblin_control_clear_pending_grant_delivery(control, pending->operation_id);
    } else if (!reply) {
        gboolean dispatch_lua =
            gnoblin_control_config_generation(control) == pending->runtime_generation;
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
        gnoblin_control_dispatch_operation_completion_full(
            control, pending->operation_id, pending->method, FALSE, NULL,
            gnoblin_control_operation_error_code(operation_error),
            operation_error ? operation_error->message : "portal request failed", dispatch_lua);
    } else {
        gboolean dispatch_lua =
            gnoblin_control_config_generation(control) == pending->runtime_generation;
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
        g_autoptr(JsonNode) json_value = value ? gnoblin_control_json_from_variant(value) : NULL;
        gnoblin_control_dispatch_operation_completion_full(control, pending->operation_id, pending->method,
                                           value != NULL, json_value,
                                           value ? NULL : gnoblin_control_operation_error_code(error),
                                           value ? NULL : error->message, dispatch_lua);
    }

    control->pending_portal_grant_ops--;
    pending_portal_grant_operation_free(pending);
    gnoblin_control_maybe_free_stopped(control);
}

static void portal_grant_bus_ready(GObject* source_object, GAsyncResult* result,
                                   gpointer user_data) {
    PendingPortalGrantOperation* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_finish(result, &error);

    if (control->stopping) {
        gnoblin_control_clear_pending_grant_delivery(control, pending->operation_id);
        control->pending_portal_grant_ops--;
        pending_portal_grant_operation_free(pending);
        gnoblin_control_maybe_free_stopped(control);
        return;
    }
    if (!connection) {
        gnoblin_control_dispatch_operation_completion_full(control, pending->operation_id, pending->method, FALSE,
                                           NULL, gnoblin_control_operation_error_code(error),
                                           error ? error->message : "session bus is unavailable",
                                           gnoblin_control_config_generation(control) ==
                                               pending->runtime_generation);
        control->pending_portal_grant_ops--;
        pending_portal_grant_operation_free(pending);
        gnoblin_control_maybe_free_stopped(control);
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
    pending->runtime_generation = gnoblin_control_config_generation(control);
    pending->method = g_strdup(method);
    pending->kind = g_strdup(kind);
    pending->id = g_strdup(id);
    pending->has_expected_created_at = has_expected_created_at;
    pending->expected_created_at_ms = expected_created_at_ms;
    control->pending_portal_grant_ops++;
    g_bus_get(G_BUS_TYPE_SESSION, NULL, portal_grant_bus_ready, pending);
    return TRUE;
}
