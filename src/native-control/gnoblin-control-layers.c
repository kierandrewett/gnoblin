/* Layer tracking and input-device change publishing: the layer snapshot and layer state cache,
 * the changed-layer-property diff, and the input-device snapshot cache and added and removed
 * events. publish_layer_changes and dispatch_lua_layer_event stay in gnoblin-native-control.c
 * because tests read their text there. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include "meta/util.h"

void gnoblin_control_layer_state_free(gpointer data) {
    NativeLayerState* state = data;
    g_free(state->json);
    g_free(state->comparable_json);
    g_free(state);
}

JsonNode* gnoblin_control_layer_snapshot_json(GnoblinNativeControl* control,
                                              gboolean update_lua_snapshot, GError** error) {
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "layer.list", arguments, error);
    if (!result) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "layers", NULL,
                                                     control->state_revision);
        return NULL;
    }

    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json)) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "layers", NULL,
                                                     control->state_revision);
        return NULL;
    }
    JsonArray* layers = json_object_get_array_member(json_node_get_object(json), "layers");
    if (!layers) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "layers", NULL,
                                                     control->state_revision);
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
        g_autoptr(GVariant) snapshot = gnoblin_control_variant_from_json(json);
        gnoblin_control_publish_runtime_snapshot(control, "layers", snapshot,
                                                 control->state_revision);
    }
    return g_steal_pointer(&json);
}

GHashTable* gnoblin_control_input_device_state_from_snapshot(JsonNode* snapshot) {
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
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, event, payload);
}

void gnoblin_control_publish_input_device_changes(GnoblinNativeControl* control, JsonNode* snapshot,
                                                  guint64 revision) {
    if (!snapshot || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return;

    GHashTable* current = gnoblin_control_input_device_state_from_snapshot(snapshot);
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

GHashTable* gnoblin_control_layer_state_from_snapshot(JsonNode* snapshot) {
    GHashTable* state =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, gnoblin_control_layer_state_free);
    if (!snapshot || !JSON_NODE_HOLDS_OBJECT(snapshot))
        return state;

    JsonArray* layers = json_object_get_array_member(json_node_get_object(snapshot), "layers");
    for (guint i = 0; layers && i < json_array_get_length(layers); i++) {
        JsonNode* record = json_array_get_element(layers, i);
        if (!JSON_NODE_HOLDS_OBJECT(record))
            continue;
        JsonObject* object = json_node_get_object(record);
        const char* id = json_object_get_string_member_with_default(object, "id", NULL);
        if (!id || !*id)
            continue;

        NativeLayerState* value = g_new0(NativeLayerState, 1);
        value->json = json_to_string(record, FALSE);
        g_autoptr(JsonNode) comparable = json_node_copy(record);
        json_object_remove_member(json_node_get_object(comparable), "revision");
        value->comparable_json = json_to_string(comparable, FALSE);
        g_hash_table_insert(state, g_strdup(id), value);
    }
    return state;
}

static gboolean layer_property_equal(JsonNode* previous, JsonNode* current) {
    if (!previous || !current)
        return previous == current;
    g_autofree char* previous_json = json_to_string(previous, FALSE);
    g_autofree char* current_json = json_to_string(current, FALSE);
    return g_strcmp0(previous_json, current_json) == 0;
}

JsonArray* gnoblin_control_changed_layer_properties(JsonNode* previous, JsonNode* current) {
    JsonObject* previous_object = json_node_get_object(previous);
    JsonObject* current_object = json_node_get_object(current);
    JsonArray* changed = json_array_new();
    GList* members = json_object_get_members(current_object);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        if (g_str_equal(name, "revision"))
            continue;
        if (!layer_property_equal(json_object_get_member(previous_object, name),
                                  json_object_get_member(current_object, name)))
            json_array_add_string_element(changed, name);
    }
    g_list_free(members);

    members = json_object_get_members(previous_object);
    for (GList* item = members; item; item = item->next) {
        const char* name = item->data;
        if (!g_str_equal(name, "revision") && !json_object_has_member(current_object, name))
            json_array_add_string_element(changed, name);
    }
    g_list_free(members);
    return changed;
}
