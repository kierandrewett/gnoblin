/* Session activity and idle watches, on Mutter's core idle monitor.
 *
 * Lua's gnoblin.idle.watch(seconds) is the one mechanism: idle.watch adds an idle watch to the
 * monitor, and a user-active watch after each idle, and the transitions go to the Lua runtime as
 * gnoblin.idle.watch-idle and gnoblin.idle.watch-active. A watch belongs to the runtime
 * generation that made it; a reload, a stopped worker and the compositor's stop remove it. The
 * session-activity snapshot that socket clients read (session.activity and
 * gnoblin.session.activity-changed) is the compositor's own watch of the standard threshold. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include "backends/meta-backend-private.h"
#include "backends/meta-idle-manager.h"
#include "core/gnoblin-native-control-private.h"
#include "meta/meta-idle-monitor.h"

#define SESSION_ACTIVITY_THRESHOLD_MS 120000
#define IDLE_WATCH_MAX 256
#define IDLE_WATCH_MAX_SECONDS 86400

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
    MetaIdleMonitor* monitor;
    char* id;           /* "idle-<operation ID>", Lua's IdleWatch.id; NULL for the session watch */
    guint64 generation; /* the runtime generation that owns it */
    guint64 interval_ms;
    guint idle_watch_id;
    guint active_watch_id;
    gboolean idle;
} IdleWatch;

static MetaIdleMonitor* idle_monitor(GnoblinNativeControl* control) {
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(control->display));
    return meta_idle_manager_get_core_monitor(meta_backend_get_idle_manager(backend));
}

static void idle_watch_free(gpointer data) {
    IdleWatch* watch = data;
    if (watch->idle_watch_id)
        meta_idle_monitor_remove_watch(watch->monitor, watch->idle_watch_id);
    if (watch->active_watch_id)
        meta_idle_monitor_remove_watch(watch->monitor, watch->active_watch_id);
    g_free(watch->id);
    g_free(watch);
}

static GVariant* idle_watch_record(const IdleWatch* watch) {
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(watch->id));
    g_variant_builder_add(&record, "{sv}", "seconds",
                          g_variant_new_double((double)watch->interval_ms / 1000.0));
    g_variant_builder_add(&record, "{sv}", "idle", g_variant_new_boolean(watch->idle));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static void idle_watch_notify(IdleWatch* watch) {
    GnoblinNativeControl* control = watch->control;
    if (control->stopping)
        return;
    if (!watch->id) {
        gint64 idle_for_ms = watch->idle ? meta_idle_monitor_get_idletime(watch->monitor) : 0;
        gnoblin_control_activity_publish(control, TRUE, watch->idle, watch->interval_ms,
                                         (guint64)MAX(idle_for_ms, 0));
        return;
    }
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "watch", idle_watch_record(watch));
    g_variant_builder_add(&payload, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_variant_builder_add(&payload, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
    native_runtime_dispatch_event(
        control, watch->idle ? "gnoblin.idle.watch-idle" : "gnoblin.idle.watch-active", event);
}

static void idle_watch_active(MetaIdleMonitor* monitor, guint watch_id, gpointer user_data) {
    IdleWatch* watch = user_data;
    (void)monitor;
    (void)watch_id;
    watch->active_watch_id = 0; /* A user-active watch ends after it fires. */
    if (!watch->idle)
        return;
    watch->idle = FALSE;
    idle_watch_notify(watch);
}

static void idle_watch_idle(MetaIdleMonitor* monitor, guint watch_id, gpointer user_data) {
    IdleWatch* watch = user_data;
    (void)watch_id;
    if (watch->idle)
        return;
    watch->idle = TRUE;
    if (!watch->active_watch_id)
        watch->active_watch_id =
            meta_idle_monitor_add_user_active_watch(monitor, idle_watch_active, watch, NULL);
    idle_watch_notify(watch);
}

static IdleWatch* idle_watch_new(GnoblinNativeControl* control, guint64 interval_ms) {
    IdleWatch* watch = g_new0(IdleWatch, 1);
    watch->control = control;
    watch->monitor = idle_monitor(control);
    watch->interval_ms = interval_ms;
    watch->idle_watch_id =
        meta_idle_monitor_add_idle_watch(watch->monitor, interval_ms, idle_watch_idle, watch, NULL);
    return watch;
}

/* The compositor's own watch of the standard threshold feeds the session-activity snapshot. */
void gnoblin_control_activity_watch(GnoblinNativeControl* control) {
    if (!control || control->session_activity_watch)
        return;
    IdleWatch* watch = idle_watch_new(control, SESSION_ACTIVITY_THRESHOLD_MS);
    control->session_activity_watch = watch;
    gint64 idle_for_ms = meta_idle_monitor_get_idletime(watch->monitor);
    watch->idle = idle_for_ms >= (gint64)watch->interval_ms;
    if (watch->idle)
        watch->active_watch_id =
            meta_idle_monitor_add_user_active_watch(watch->monitor, idle_watch_active, watch, NULL);
    gnoblin_control_activity_publish(control, TRUE, watch->idle, SESSION_ACTIVITY_THRESHOLD_MS,
                                     watch->idle ? (guint64)idle_for_ms : 0);
}

void gnoblin_control_activity_stop(GnoblinNativeControl* control) {
    g_clear_pointer(&control->session_activity_watch, idle_watch_free);
    g_clear_pointer(&control->idle_watches, g_hash_table_unref);
}

void gnoblin_control_idle_watches_clear(GnoblinNativeControl* control) {
    if (control->idle_watches)
        g_hash_table_remove_all(control->idle_watches);
}

/* idle.watch {seconds}: add a watch for the runtime generation that asked. The result is the new
 * IdleWatch record; the transitions arrive later as runtime events. */
GVariant* gnoblin_control_idle_watch(GnoblinNativeControl* control, gint64 operation_id,
                                     GVariant* arguments, guint64 owner_generation,
                                     GError** error) {
    double seconds = 0;
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "idle.watch belongs to a stale runtime generation");
        return NULL;
    }
    if (!g_variant_lookup(arguments, "seconds", "d", &seconds) || !(seconds > 0) ||
        seconds > IDLE_WATCH_MAX_SECONDS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "idle.watch needs seconds above 0 and at most 86400");
        return NULL;
    }
    if (!control->idle_watches)
        control->idle_watches =
            g_hash_table_new_full(g_str_hash, g_str_equal, NULL, idle_watch_free);
    if (g_hash_table_size(control->idle_watches) >= IDLE_WATCH_MAX) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "a runtime can hold at most 256 idle watches");
        return NULL;
    }
    guint64 interval_ms = MAX((guint64)1, (guint64)(seconds * 1000.0 + 0.5));
    IdleWatch* watch = idle_watch_new(control, interval_ms);
    watch->generation = owner_generation;
    watch->id = g_strdup_printf("idle-%" G_GINT64_FORMAT, operation_id);
    g_hash_table_insert(control->idle_watches, watch->id, watch);
    return idle_watch_record(watch);
}

/* idle.cancel {id}: remove a watch. Only the generation that made it may. */
GVariant* gnoblin_control_idle_cancel(GnoblinNativeControl* control, GVariant* arguments,
                                      guint64 owner_generation, GError** error) {
    const char* id = NULL;
    IdleWatch* watch = NULL;
    if (!g_variant_lookup(arguments, "id", "&s", &id) || !id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "idle.cancel needs an id");
        return NULL;
    }
    watch = control->idle_watches ? g_hash_table_lookup(control->idle_watches, id) : NULL;
    if (!watch || watch->generation != owner_generation) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "idle watch %s has ended", id);
        return NULL;
    }
    g_hash_table_remove(control->idle_watches, id);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "cancelled", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
