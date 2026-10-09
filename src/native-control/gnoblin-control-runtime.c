/* Runtime worker protocol helpers: reload document comparison, resume matching, config results,
 * queue flushing and request cancel. The framed send path, the abort path and the operation
 * dispatcher stay in gnoblin-native-control.c with the state they share. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <string.h>

gboolean gnoblin_control_xwayland_server_flags_changed(GVariant* current, GVariant* candidate) {
    g_autoptr(GVariant) current_xwayland =
        current ? g_variant_lookup_value(current, "xwayland", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) candidate_xwayland =
        candidate ? g_variant_lookup_value(candidate, "xwayland", G_VARIANT_TYPE_VARDICT) : NULL;
    gboolean current_byte_swapped = FALSE;
    gboolean candidate_byte_swapped = FALSE;
    if (current_xwayland)
        g_variant_lookup(current_xwayland, "allow-byte-swapped-clients", "b",
                         &current_byte_swapped);
    if (candidate_xwayland)
        g_variant_lookup(candidate_xwayland, "allow-byte-swapped-clients", "b",
                         &candidate_byte_swapped);
    if (current_byte_swapped != candidate_byte_swapped)
        return TRUE;

    g_autoptr(GVariant) current_extensions =
        current_xwayland ? g_variant_lookup_value(current_xwayland, "disable-extensions", NULL)
                         : NULL;
    g_autoptr(GVariant) candidate_extensions =
        candidate_xwayland ? g_variant_lookup_value(candidate_xwayland, "disable-extensions", NULL)
                           : NULL;
    gboolean current_empty = !current_extensions || g_variant_n_children(current_extensions) == 0;
    gboolean candidate_empty =
        !candidate_extensions || g_variant_n_children(candidate_extensions) == 0;
    return current_empty != candidate_empty ||
           (!current_empty && !gnoblin_control_runtime_config_values_equal(current_extensions,
                                                           candidate_extensions));
}

gboolean gnoblin_control_runtime_reload_documents_valid(GVariant* current, GVariant* candidate) {
    /* The Lua loader has already validated both snapshots. Do not maintain a
     * second, incomplete list of settings that can reload: native settings
     * are reapplied from the candidate document and the Lua runtime is
     * replaced transactionally below. */
    return current && candidate &&
           g_variant_is_of_type(current, G_VARIANT_TYPE_VARDICT) &&
           g_variant_is_of_type(candidate, G_VARIANT_TYPE_VARDICT);
}

gboolean gnoblin_control_runtime_send_config_result(GnoblinNativeControl* control,
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
    return gnoblin_control_runtime_send(control, GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT, transaction_id,
                               payload, error);
}

void gnoblin_control_runtime_clear_queue(GQueue* queue) {
    if (queue)
        g_queue_clear_full(queue, (GDestroyNotify)g_variant_unref);
}

gboolean gnoblin_control_runtime_event_subscriptions_equal(GHashTable* left, GHashTable* right) {
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

void gnoblin_control_runtime_add_display_environment(GVariantBuilder* hello) {
    GVariantBuilder environment;
    g_variant_builder_init(&environment, G_VARIANT_TYPE("a{ss}"));
    const char* names[] = {"WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", NULL};
    for (guint i = 0; names[i]; i++) {
        const char* value = g_getenv(names[i]);
        if (value && *value)
            g_variant_builder_add(&environment, "{ss}", names[i], value);
    }
    g_variant_builder_add(hello, "{sv}", "environment", g_variant_builder_end(&environment));
}

static gboolean config_values_equal_at(GVariant* left, GVariant* right, GString* path);

gboolean gnoblin_control_runtime_resume_snapshot_matches(GnoblinNativeControl* control,
                                                       GVariant* payload) {
    g_autoptr(GVariant) document =
        g_variant_lookup_value(payload, "document", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) current_document =
        gnoblin_runtime_cache_get_document(control->runtime_cache);
    g_autoptr(GError) events_error = NULL;
    g_autoptr(GHashTable) subscriptions =
        gnoblin_control_runtime_event_subscriptions_from_payload(payload, &events_error);
    guint64 revision = 0;
    guint64 generation = 0;
    guint64 operation_id_watermark = 0;
    guint64 current_revision = gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
    /* Name the first field that differs. The rejection otherwise only says that the state does not match. */
    const char* mismatch = NULL;
    if (!subscriptions || !control->runtime_event_subscriptions)
        mismatch = "event subscriptions are missing";
    else if (!gnoblin_control_runtime_event_subscriptions_equal(subscriptions,
                                                                 control->runtime_event_subscriptions))
        mismatch = "event subscriptions differ";
    else if (!document || !current_document)
        mismatch = "the document is missing";
    else if (!g_variant_lookup(payload, "settings_revision", "t", &revision) ||
             !g_variant_lookup(payload, "runtime_generation", "t", &generation) ||
             !g_variant_lookup(payload, "operation_id_watermark", "t", &operation_id_watermark))
        mismatch = "a counter is missing";
    else if (revision != current_revision)
        mismatch = "the settings revision differs";
    else if (generation != control->runtime_generation)
        mismatch = "the runtime generation differs";
    else if (operation_id_watermark != control->last_runtime_operation_id)
        mismatch = "the operation id watermark differs";
    g_autoptr(GString) difference = g_string_new("");
    if (!mismatch && !config_values_equal_at(document, current_document, difference))
        mismatch = "the document values differ";
    if (!mismatch)
        return TRUE;
    if (difference->len)
        g_message("gnoblin-native-control: first document difference: %s", difference->str);
    g_warning("gnoblin-native-control: runtime resume does not match the accepted state: %s "
              "(worker revision %" G_GUINT64_FORMAT " generation %" G_GUINT64_FORMAT
              " watermark %" G_GUINT64_FORMAT "; accepted revision %" G_GUINT64_FORMAT
              " generation %" G_GUINT64_FORMAT " watermark %" G_GUINT64_FORMAT ")",
              mismatch, revision, generation, operation_id_watermark, current_revision,
              control->runtime_generation, (guint64)control->last_runtime_operation_id);
    return FALSE;
}

gboolean gnoblin_control_runtime_reject_resume(GnoblinNativeControl* control, const char* message,
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
    return gnoblin_control_runtime_send(control, GNOBLIN_RUNTIME_PACKET_ERROR, 0, payload, error);
}

gboolean gnoblin_native_control_cancel_runtime_request(MetaDisplay* display, guint64 request_id) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || !request_id || !control->pending_runtime_requests)
        return FALSE;
    PendingRuntimeRequest* pending =
        g_hash_table_lookup(control->pending_runtime_requests, &request_id);
    if (!pending || pending->discard_response)
        return FALSE;
    pending->discard_response = TRUE;
    if (pending->callback_destroy)
        pending->callback_destroy(pending->callback_data);
    pending->callback = NULL;
    pending->callback_data = NULL;
    pending->callback_destroy = NULL;
    return TRUE;
}

/* Compares two configuration values. When PATH is set, it receives the key path of the first difference and both
 * values, so a rejected worker resume can say what differed. */
static gboolean config_values_equal_at(GVariant* left, GVariant* right, GString* path) {
    if (!left || !right) {
        if (path)
            g_string_append_printf(path, " [%s]", !left ? "missing in the worker" : "missing in the accepted state");
        return FALSE;
    }
    if (!g_variant_type_equal(g_variant_get_type(left), g_variant_get_type(right))) {
        if (path)
            g_string_append_printf(path, " [worker type %s, accepted type %s]",
                                   g_variant_get_type_string(left), g_variant_get_type_string(right));
        return FALSE;
    }

    if (g_variant_is_of_type(left, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) left_value = g_variant_get_variant(left);
        g_autoptr(GVariant) right_value = g_variant_get_variant(right);
        return config_values_equal_at(left_value, right_value, path);
    }

    if (g_variant_is_of_type(left, G_VARIANT_TYPE_VARDICT)) {
        if (g_variant_n_children(left) != g_variant_n_children(right)) {
            if (path)
                g_string_append_printf(path, " [worker has %" G_GSIZE_FORMAT " keys, accepted has %" G_GSIZE_FORMAT "]",
                                       g_variant_n_children(left), g_variant_n_children(right));
            return FALSE;
        }

        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, left);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) left_value = value;
            g_autoptr(GVariant) right_value = g_variant_lookup_value(right, key, NULL);
            gsize length = path ? path->len : 0;
            if (path)
                g_string_append_printf(path, "/%s", key);
            if (!config_values_equal_at(left_value, right_value, path))
                return FALSE;
            if (path)
                g_string_truncate(path, length);
        }
        return TRUE;
    }

    if (g_variant_is_container(left)) {
        if (g_variant_n_children(left) != g_variant_n_children(right)) {
            if (path)
                g_string_append_printf(path, " [worker has %" G_GSIZE_FORMAT " items, accepted has %" G_GSIZE_FORMAT "]",
                                       g_variant_n_children(left), g_variant_n_children(right));
            return FALSE;
        }

        for (gsize i = 0; i < g_variant_n_children(left); i++) {
            g_autoptr(GVariant) left_value = g_variant_get_child_value(left, i);
            g_autoptr(GVariant) right_value = g_variant_get_child_value(right, i);
            gsize length = path ? path->len : 0;
            if (path)
                g_string_append_printf(path, "[%" G_GSIZE_FORMAT "]", i);
            if (!config_values_equal_at(left_value, right_value, path))
                return FALSE;
            if (path)
                g_string_truncate(path, length);
        }
        return TRUE;
    }

    if (g_variant_equal(left, right))
        return TRUE;
    if (path) {
        g_autofree char* left_text = g_variant_print(left, FALSE);
        g_autofree char* right_text = g_variant_print(right, FALSE);
        g_string_append_printf(path, " [worker %s, accepted %s]", left_text, right_text);
    }
    return FALSE;
}

gboolean gnoblin_control_runtime_config_values_equal(GVariant* left, GVariant* right) {
    return config_values_equal_at(left, right, NULL);
}

gboolean gnoblin_control_runtime_flush_state_snapshots(GnoblinNativeControl* control,
                                                     GError** error) {
    while (control->pending_runtime_states && !g_queue_is_empty(control->pending_runtime_states)) {
        GVariant* payload = g_queue_pop_head(control->pending_runtime_states);
        gboolean sent =
            gnoblin_control_runtime_send(control, GNOBLIN_RUNTIME_PACKET_STATE, 0, payload, error);
        g_variant_unref(payload);
        if (!sent)
            return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_control_runtime_flush_pending_events(GnoblinNativeControl* control, GError** error) {
    while (control->pending_runtime_events && !g_queue_is_empty(control->pending_runtime_events)) {
        GVariant* packet = g_queue_pop_head(control->pending_runtime_events);
        gboolean sent =
            gnoblin_control_runtime_send(control, GNOBLIN_RUNTIME_PACKET_EVENT, 0, packet, error);
        g_variant_unref(packet);
        if (!sent)
            return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_control_runtime_send_worker_suspended(GnoblinNativeControl* control,
                                                     GError** error) {
    guint64 revision = gnoblin_runtime_cache_get_settings_revision(control->runtime_cache);
    g_autoptr(GVariant) document = gnoblin_runtime_cache_get_document(control->runtime_cache);
    if (!document) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "cannot suspend Lua worker without an accepted configuration document");
        return FALSE;
    }
    GVariantBuilder events;
    GHashTableIter subscriptions;
    gpointer event_name;
    g_variant_builder_init(&events, G_VARIANT_TYPE("as"));
    if (control->runtime_event_subscriptions) {
        g_hash_table_iter_init(&subscriptions, control->runtime_event_subscriptions);
        while (g_hash_table_iter_next(&subscriptions, &event_name, NULL))
            g_variant_builder_add(&events, "s", (const char*)event_name);
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation",
                          g_variant_new_uint64(control->runtime_generation));
    g_variant_builder_add(&builder, "{sv}", "operation_id_watermark",
                          g_variant_new_uint64(control->last_runtime_operation_id));
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_variant_builder_add(&builder, "{sv}", "runtime_events", g_variant_builder_end(&events));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&builder));
    return gnoblin_control_runtime_send(control, GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED, 0, payload, error);
}

GHashTable* gnoblin_control_runtime_event_subscriptions_from_payload(GVariant* payload,
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
