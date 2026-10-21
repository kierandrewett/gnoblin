#include "gnoblin-native-control-private.h"

gboolean apply_native_input(GnoblinNativeControl* control, MetaContext* context, GVariant* document,
                            GError** error) {
    g_autoptr(GVariant) input = document ? g_variant_lookup_value(document, "input", NULL) : NULL;
    if (!input) {
        meta_display_apply_gnoblin_input_config(control->display, NULL);
        meta_prefs_apply_gnoblin_input_preferences(control->backend, NULL);
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
            g_str_equal(group, "trackball") || g_str_equal(group, "pointing-stick") ||
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
    meta_prefs_apply_gnoblin_input_preferences(control->backend, normalized_input);
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
    g_free(client);
}

void client_maybe_free(Client* client) {
    if (client->closing && !client->reading && !client->writing &&
        client->pending_deferred_requests == 0)
        client_free(client);
}

void pending_runtime_request_free(gpointer data) {
    PendingRuntimeRequest* pending = data;
    if (!pending)
        return;
    g_free(pending->request_id);
    g_free(pending->legacy_window_action);
    g_free(pending->legacy_window_id);
    if (pending->callback_destroy)
        pending->callback_destroy(pending->callback_data);
    g_free(pending);
}

void client_close(Client* client) {
    if (!client->closing) {
        client->closing = TRUE;
        gnoblin_control_clear_client_dynamic_shortcuts(client);
        if (client->control) {
            GnoblinNativeControl* control = client->control;
            gboolean refresh_mutter_watches = native_client_has_mutter_event_subscriptions(client);
            native_socket_revoke_client_tokens(client);
            native_ui_sessions_client_disconnected(control, client->client_id);
            gnoblin_control_window_drag_client_disconnected(client->control, client->client_id);
            g_hash_table_remove(client->control->clients, client);
            if (refresh_mutter_watches)
                native_refresh_all_mutter_signal_watches(control);
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

JsonNode* json_from_variant(GVariant* value) {
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
    case G_VARIANT_CLASS_UINT64: {
        guint64 number = g_variant_get_uint64(value);
        if (number <= G_MAXINT64)
            json_node_set_int(node, (gint64)number);
        else
            json_node_set_double(node, (double)number);
        break;
    }
    case G_VARIANT_CLASS_STRING:
        json_node_set_string(node, g_variant_get_string(value, NULL));
        break;
    default:
        json_node_free(node);
        return json_node_new(JSON_NODE_NULL);
    }
    return node;
}

GVariant* variant_from_json(JsonNode* node) {
    if (JSON_NODE_HOLDS_OBJECT(node)) {
        GVariantBuilder builder;
        JsonObject* object = json_node_get_object(node);
        GList* members = json_object_get_members(object);
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        for (GList* item = members; item; item = item->next) {
            const char* key = item->data;
            g_autoptr(GVariant) value = variant_from_json(json_object_get_member(object, key));
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
            g_autoptr(GVariant) value = variant_from_json(json_array_get_element(array, i));
            if (value)
                g_variant_builder_add_value(&builder, g_variant_new_variant(value));
        }
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (!JSON_NODE_HOLDS_VALUE(node))
        return NULL;
    GType type = json_node_get_value_type(node);
    if (type == G_TYPE_BOOLEAN)
        return g_variant_ref_sink(g_variant_new_boolean(json_node_get_boolean(node)));
    if (type == G_TYPE_DOUBLE || type == G_TYPE_FLOAT)
        return g_variant_ref_sink(g_variant_new_double(json_node_get_double(node)));
    if (type == G_TYPE_INT64 || type == G_TYPE_INT || type == G_TYPE_LONG)
        return g_variant_ref_sink(g_variant_new_int64(json_node_get_int(node)));
    if (type == G_TYPE_STRING)
        return g_variant_ref_sink(g_variant_new_string(json_node_get_string(node)));
    return NULL;
}

char* encode_response(const char* id, JsonNode* result, const char* message) {
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

char* gnoblin_control_encode_response(const char* id, JsonNode* result, const char* message) {
    return encode_response(id, result, message);
}

char* encode_event_subscription(JsonArray* events, guint api_minor) {
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
