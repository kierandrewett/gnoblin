#include "gnoblin-native-control-private.h"
#include "compositor/compositor-private.h"

static void native_window_record_add_public_aliases(JsonObject* object);

static void cache_lua_window_snapshot(GnoblinNativeControl* control, GVariant* native_snapshot,
                                      guint64 revision);

JsonNode* window_snapshot_json(GnoblinNativeControl* control, gboolean update_lua_snapshot,
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

char* window_snapshot(GnoblinNativeControl* control) {
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

const WindowPropertyName window_property_names[] = {
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

const gsize window_property_names_count = G_N_ELEMENTS(window_property_names);

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

gboolean window_record_is_modal(JsonObject* record) {
    JsonNode* type = json_object_get_member(record, "type");
    return type && JSON_NODE_HOLDS_VALUE(type) &&
           json_node_get_int(type) == META_WINDOW_MODAL_DIALOG;
}

JsonNode* lua_window_record(JsonNode* native_record, guint64 revision) {
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

gboolean native_event_is_subscribable(const char* name) {
    if (!name)
        return FALSE;
    for (guint i = 0; native_socket_events[i]; i++) {
        if (g_str_equal(name, native_socket_events[i]) && !g_str_equal(name, "windows") &&
            !g_str_equal(name, "monitors"))
            return TRUE;
    }
    return FALSE;
}

gboolean native_client_has_mutter_event_subscriptions(Client* client) {
    if (!client || !client->event_subscriptions)
        return FALSE;

    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, client->event_subscriptions);
    while (g_hash_table_iter_next(&iter, &key, NULL)) {
        const char* event = key;
        if (g_str_has_prefix(event, "mutter."))
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

void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {
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
    /* json_node_copy() shares the object: preserve the mutations above for
     * callers (including Lua), but only encode once a client can receive it. */
    g_autofree char* line = NULL;
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
        if (subscribed && g_str_equal(name, "gnoblin.auth.requested") &&
            json_object_has_member(object, "request_id") && control->pending_auth) {
            gnoblin_control_auth_note_recipient(
                control, json_object_get_int_member(object, "request_id"), client->client_id);
        }
        if (subscribed && g_str_equal(name, "gnoblin.prompt.requested") &&
            json_object_has_member(object, "request_id") && control->pending_prompts) {
            gnoblin_control_prompts_note_recipient(
                control, json_object_get_int_member(object, "request_id"), client->client_id);
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
        if (g_str_has_prefix(name, "gnoblin.layer.") && client->event_api_minor < 71)
            subscribed = FALSE;
        if (g_str_equal(name, "gnoblin.runtime.status-changed") && client->event_api_minor < 72)
            subscribed = FALSE;
        if (g_str_equal(name, "gnoblin.session.state-changed") && client->event_api_minor < 76)
            subscribed = FALSE;
        if (g_str_equal(name, "gnoblin.pointer.locate-requested") && client->event_api_minor < 80)
            subscribed = FALSE;
        if (subscribed) {
            if (!line) {
                g_autofree char* encoded = json_to_string(socket_event, FALSE);
                line = g_strconcat(encoded, "\n", NULL);
            }
            send_response(client, g_strdup(line));
        }
    }
    g_list_free(clients);
}

static gboolean native_socket_has_event_subscriber(GnoblinNativeControl* control,
                                                   const char* event) {
    if (!control || !control->clients || !event)
        return FALSE;

    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->clients);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        Client* client = value;
        if (client->event_api_minor >= 9 && client->event_subscriptions &&
            g_hash_table_contains(client->event_subscriptions, event))
            return TRUE;
    }
    return FALSE;
}

void native_publish_mutter_socket_event(GnoblinNativeControl* control, const char* event,
                                        GVariant* payload) {
    if (!control || !event || !payload || !g_str_has_prefix(event, "mutter.") ||
        !native_socket_has_event_subscriber(control, event))
        return;

    g_autoptr(JsonNode) root = json_from_variant(payload);
    if (!root || !JSON_NODE_HOLDS_OBJECT(root))
        return;

    JsonObject* object = json_node_get_object(root);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    if (!json_object_has_member(object, "time"))
        json_object_set_int_member(object, "time", g_get_monotonic_time());
    publish_native_socket_event(control, root);
}

void dispatch_lua_layer_event(GnoblinNativeControl* control, guint64 revision, const char* event,
                              const char* id, JsonNode* layer, JsonNode* last, JsonArray* changed) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", event);
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    if (g_str_equal(event, "gnoblin.layer.created")) {
        json_object_set_member(object, "layer", json_node_copy(layer));
    } else if (g_str_equal(event, "gnoblin.layer.removed")) {
        json_object_set_string_member(object, "layer_id", id);
        json_object_set_member(object, "last", json_node_copy(last));
    } else {
        json_object_set_string_member(object, "layer_id", id);
        json_object_set_member(object, "layer", json_node_copy(layer));
        json_object_set_array_member(object, "changed", json_array_ref(changed));
    }
    publish_native_socket_event(control, root);
    g_autoptr(GVariant) payload = variant_from_json(root);
    if (payload)
        native_runtime_dispatch_event(control, event, payload);
}

void native_publish_request_event(GnoblinNativeControl* control, const char* name,
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

void gnoblin_control_publish_event(GnoblinNativeControl* control, const char* name,
                                   GVariant* fields) {
    native_publish_request_event(control, name, fields);
}

/* Exported names for static helpers that the split gnoblin-control-*.c files call. */
void gnoblin_control_publish_runtime_snapshot(GnoblinNativeControl* control, const char* name,
                                              GVariant* snapshot, guint64 revision) {
    native_publish_runtime_snapshot(control, name, snapshot, revision);
}

gboolean gnoblin_control_dispatch_event(GnoblinNativeControl* control, const char* event,
                                        GVariant* payload) {
    return native_runtime_dispatch_event(control, event, payload);
}

JsonNode* gnoblin_control_json_from_variant(GVariant* value) {
    return json_from_variant(value);
}

GVariant* gnoblin_control_variant_from_json(JsonNode* node) {
    return variant_from_json(node);
}

GVariant* gnoblin_control_native_config_document(GnoblinNativeControl* control) {
    return native_config_document(control);
}

char* gnoblin_control_native_window_id(MetaWindow* window) {
    return native_window_id(window);
}

void gnoblin_control_schedule_windows(GnoblinNativeControl* control) {
    schedule_windows(control);
}

void gnoblin_control_publish_socket_event(GnoblinNativeControl* control, JsonNode* payload) {
    publish_native_socket_event(control, payload);
}

GVariant* gnoblin_control_capability_snapshot(GnoblinNativeControl* control) {
    return capability_snapshot(control);
}

GVariant* gnoblin_control_capability_snapshot_record(GnoblinNativeControl* control,
                                                     const NativeCapability* native_capability) {
    return capability_snapshot_record(control, native_capability);
}

const NativeCapability* gnoblin_control_capability_by_id(const char* id) {
    return native_capability_by_id(id);
}

const char* native_runtime_status_state(GnoblinNativeControl* control) {
    if (!control || !control->supervised_runtime || control->stopping ||
        control->runtime_recovery_failed)
        return "unavailable";
    if (control->runtime_worker_suspended)
        return "restarting";
    if (control->runtime_hello_sent)
        return "running";
    return "starting";
}

void native_runtime_publish_status(GnoblinNativeControl* control) {
    if (!control || control->stopping)
        return;
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "state",
                          g_variant_new_string(native_runtime_status_state(control)));
    g_variant_builder_add(&fields, "{sv}", "generation",
                          g_variant_new_uint64(control->runtime_generation));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.runtime.status-changed", payload);
}

GVariant* appearance_snapshot_new(GnoblinNativeControl* control) {
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

void appearance_color_scheme_changed(GSettings* settings, const char* key, gpointer user_data) {
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

#ifdef HAVE_REMOTE_DESKTOP

static gboolean privacy_camera_disable(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->privacy_camera_disable_source_id = 0;
    if (!control->stopping && control->privacy_camera_available && control->privacy_camera_in_use) {
        control->privacy_camera_in_use = FALSE;
        control->privacy_revision++;
        gnoblin_control_publish_privacy_snapshot(control, TRUE);
    }
    return G_SOURCE_REMOVE;
}

void privacy_pipewire_state_changed(GnoblinPipewireMonitor* monitor, gboolean available,
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
        gnoblin_control_publish_privacy_snapshot(control, TRUE);
    }
}

#endif

static gboolean location_accuracy_valid(guint accuracy) {
    return accuracy == 0 || accuracy == 1 || accuracy == 4 || accuracy == 5 || accuracy == 6 ||
           accuracy == 8;
}

GVariant* native_location_authorize_operation(GnoblinNativeControl* control, GVariant* arguments,
                                              guint64 client_id, GError** error) {
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

void native_settings_changed(guint64 revision, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || !control->display)
        return;
    gnoblin_control_clear_runtime_dynamic_shortcuts(control, "config_changed");
    gnoblin_control_clear_configured_capture_shortcut(control, "config_changed");
    if (control->active_shortcut_session)
        dynamic_shortcut_end_session(control, control->active_shortcut_session, "config_changed");

    g_autoptr(GVariant) config = native_config_document(control);
    meta_compositor_invalidate_background_blur(meta_display_get_compositor(control->display));
    g_autoptr(GError) capture_error = NULL;
    if (!apply_configured_capture_shortcut(control, config, &capture_error))
        g_warning("gnoblin-native-control: could not apply configured shortcut input capture: %s",
                  capture_error ? capture_error->message : "invalid capture configuration");
    g_autoptr(GVariant) window_management =
        config ? g_variant_lookup_value(config, "window-management", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) compositor_preferences =
        config ? g_variant_lookup_value(config, "compositor", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) input_preferences =
        config ? g_variant_lookup_value(config, "input", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) cursor_preferences =
        config ? g_variant_lookup_value(config, "cursor", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) keyboard_preferences =
        input_preferences
            ? g_variant_lookup_value(input_preferences, "keyboard", G_VARIANT_TYPE_VARDICT)
            : NULL;
    g_autoptr(GVariant) xwayland_preferences =
        config ? g_variant_lookup_value(config, "xwayland", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) monitor_preferences =
        config ? g_variant_lookup_value(config, "monitors", G_VARIANT_TYPE_VARDICT) : NULL;
    native_apply_location_policy(control, config);
    gnoblin_control_auth_apply_config(control, config);
    gnoblin_control_prompts_apply_config(control, config);
    const char* cursor_theme = "default";
    gint64 cursor_size = 24;
    if (cursor_preferences) {
        g_variant_lookup(cursor_preferences, "theme", "&s", &cursor_theme);
        g_variant_lookup(cursor_preferences, "size", "x", &cursor_size);
    }
    meta_prefs_apply_gnoblin_window_preferences(window_management);
    meta_prefs_apply_gnoblin_compositor_preferences(compositor_preferences);
    meta_prefs_apply_gnoblin_keyboard_preferences(keyboard_preferences);
    meta_prefs_set_gnoblin_cursor_config(cursor_theme, (int)cursor_size);
    meta_settings_apply_gnoblin_xwayland_preferences(meta_backend_get_settings(control->backend),
                                                     xwayland_preferences);
    meta_wayland_layer_shell_set_preserve_active_window(
        control->wayland_compositor,
        gnoblin_native_control_get_config_bool(control->display, "layer-shell",
                                               "preserve-active-window", TRUE));
    meta_settings_apply_gnoblin_privacy_screen_preferences(
        meta_backend_get_settings(control->backend), monitor_preferences);
    meta_settings_apply_gnoblin_privacy_screen_runtime_override(
        meta_backend_get_settings(control->backend),
        control->monitor_privacy_screen_runtime_override,
        control->monitor_privacy_screen_runtime_value);
    native_monitor_privacy_screen_publish(control);
    gnoblin_control_apply_all_window_rules(control);
    g_autoptr(GVariant) configured_gestures =
        config ? g_variant_lookup_value(config, "touchpad-gestures", G_VARIANT_TYPE("av")) : NULL;
    g_clear_pointer(&control->native_touchpad_gestures, g_variant_unref);
    control->native_touchpad_gestures = filter_native_touchpad_gestures(configured_gestures);
    gnoblin_touchpad_router_reset(control->touchpad_router);

    /* Animation registrations live in the supervisor's Lua state. The cache
     * is updated there independently of the compositor config document. */
    native_publish_runtime_snapshot(control, "animations", NULL, revision);
}
