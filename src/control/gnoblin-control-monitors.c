#include "gnoblin-native-control-private.h"
#include "meta/meta-backend.h"
#include "meta/meta-monitor.h"
#include "meta/meta-monitor-manager.h"
#include "meta/meta-logical-monitor.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-logical-monitor-private.h"
#include "clutter/clutter-backend.h"
#include "cogl/cogl.h"
#include "cogl/driver/gl/cogl-driver-gl-private.h"
#include <GL/gl.h>
#include <string.h>

/* Provided by the optional VR extension when it is built in; absent otherwise. */
extern GVariant* gnoblin_vr_output_record(void) __attribute__((weak));

char* monitor_snapshot(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) json = gnoblin_control_monitor_snapshot_json(control, FALSE, &error);
    if (!json)
        return encode_response("", NULL, error ? error->message : "monitor listing unavailable");
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", "monitors");
    g_autofree char* encoded = json_to_string(json, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

JsonNode* gnoblin_control_output_snapshot_json(GnoblinNativeControl* control,
                                               gboolean update_lua_snapshot, GError** error) {
    g_autoptr(GVariant) result = gnoblin_control_output_list(control->display, error);
    if (!result) {
        if (update_lua_snapshot)
            gnoblin_control_publish_runtime_snapshot(control, "outputs", NULL,
                                                     control->state_revision);
        return NULL;
    }
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(result);
    if (!JSON_NODE_HOLDS_OBJECT(json))
        return NULL;
    JsonObject* object = json_node_get_object(json);
    json_object_set_int_member(object, "revision", control->state_revision);
    JsonArray* outputs = json_object_get_array_member(object, "outputs");
    for (guint i = 0; outputs && i < json_array_get_length(outputs); i++) {
        JsonNode* item = json_array_get_element(outputs, i);
        if (JSON_NODE_HOLDS_OBJECT(item))
            json_object_set_int_member(json_node_get_object(item), "revision",
                                       control->state_revision);
    }
    if (update_lua_snapshot) {
        g_autoptr(GVariant) snapshot = gnoblin_control_variant_from_json(json);
        gnoblin_control_publish_runtime_snapshot(control, "outputs", snapshot,
                                                 control->state_revision);
    }
    return g_steal_pointer(&json);
}

char* native_monitor_id_for_index(JsonNode* snapshot, int monitor_index) {
    return gnoblin_control_monitor_id_for_index(snapshot, monitor_index);
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

void native_show_osd_requested(MetaDisplay* display, gint monitor_index, const gchar* icon_name,
                               const gchar* message, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (!control || control->stopping || control->display != display)
        return;
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) monitor_snapshot =
        gnoblin_control_monitor_snapshot_json(control, FALSE, &snapshot_error);
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

void gnoblin_native_control_locate_pointer(MetaDisplay* display) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->input_seat)
        return;
    /* Mutter calls the plugin for the locate-pointer key whether or not the feature is on. */
    if (!gnoblin_native_control_get_config_bool(display, "compositor", "locate-pointer", FALSE))
        return;
    graphene_point_t pointer = GRAPHENE_POINT_INIT(0, 0);
    if (!clutter_seat_query_state(control->input_seat, NULL, &pointer, NULL))
        return;
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "x", g_variant_new_double(pointer.x));
    g_variant_builder_add(&fields, "{sv}", "y", g_variant_new_double(pointer.y));
    /* The monitor fields are optional: a monitor change can race with the key press. */
    gint monitor_index = meta_display_get_current_monitor(display);
    g_autoptr(GError) snapshot_error = NULL;
    g_autoptr(JsonNode) monitor_snapshot =
        gnoblin_control_monitor_snapshot_json(control, FALSE, &snapshot_error);
    g_autofree char* monitor_id = native_monitor_id_for_index(monitor_snapshot, monitor_index);
    if (monitor_id)
        g_variant_builder_add(&fields, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_autoptr(GVariant) output_names =
        native_monitor_output_names_for_index(control, monitor_index);
    if (output_names)
        g_variant_builder_add(&fields, "{sv}", "output_names", output_names);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.pointer.locate-requested", payload);
}

static void native_pad_help_add_button(GVariantBuilder* buttons, MetaDisplay* display,
                                       ClutterInputDevice* pad, gint button) {
    GVariantBuilder record;
    g_autofree char* label = meta_display_get_pad_button_label(display, pad, button);
    gint mode_group = clutter_input_device_get_mode_switch_button_group(pad, button);

    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "button", g_variant_new_int32(button));
    if (label)
        g_variant_builder_add(&record, "{sv}", "label", g_variant_new_string(label));
    if (mode_group >= 0)
        g_variant_builder_add(&record, "{sv}", "mode_group", g_variant_new_int32(mode_group));
    g_variant_builder_add_value(buttons, g_variant_builder_end(&record));
}

static void native_pad_help_add_feature_label(GVariantBuilder* record, MetaDisplay* display,
                                              ClutterInputDevice* pad, MetaPadFeatureType feature,
                                              MetaPadDirection direction, gint feature_number,
                                              const char* field) {
    g_autofree char* label =
        meta_display_get_pad_feature_label(display, pad, feature, direction, feature_number);
    if (label)
        g_variant_builder_add(record, "{sv}", field, g_variant_new_string(label));
}

static void native_pad_help_add_features(GVariantBuilder* features, MetaDisplay* display,
                                         ClutterInputDevice* pad, MetaPadFeatureType feature,
                                         gint feature_count, const char* type_name,
                                         MetaPadDirection positive_direction,
                                         const char* positive_label,
                                         MetaPadDirection negative_direction,
                                         const char* negative_label) {
    for (gint index = 0; index < feature_count; index++) {
        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "type", g_variant_new_string(type_name));
        g_variant_builder_add(&record, "{sv}", "index", g_variant_new_int32(index));
        native_pad_help_add_feature_label(&record, display, pad, feature, positive_direction, index,
                                          positive_label);
        native_pad_help_add_feature_label(&record, display, pad, feature, negative_direction, index,
                                          negative_label);
        g_variant_builder_add_value(features, g_variant_builder_end(&record));
    }
}

ClutterActor* native_show_pad_osd_requested(MetaDisplay* display, ClutterInputDevice* pad,
                                            GSettings* settings, const gchar* layout_path,
                                            gboolean edition_mode, gint monitor_index,
                                            gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    GVariantBuilder fields;
    GVariantBuilder buttons;
    GVariantBuilder mode_groups;
    GVariantBuilder features;
    g_autoptr(GVariant) device = NULL;
    g_autoptr(GVariant) button_records = NULL;
    g_autoptr(GVariant) mode_records = NULL;
    g_autoptr(GVariant) feature_records = NULL;
    g_autoptr(JsonNode) monitor_snapshot = NULL;
    g_autoptr(GVariant) output_names = NULL;
    g_autoptr(GError) snapshot_error = NULL;
    g_autofree char* monitor_id = NULL;

    (void)settings;
    (void)layout_path;
    if (!control || control->stopping || control->display != display || !pad)
        return NULL;

    device = input_device_record(control, pad);
    g_variant_builder_init(&buttons, G_VARIANT_TYPE("aa{sv}"));
    for (gint button = 0; button < clutter_input_device_get_n_buttons(pad); button++)
        native_pad_help_add_button(&buttons, display, pad, button);
    button_records = g_variant_ref_sink(g_variant_builder_end(&buttons));

    g_variant_builder_init(&mode_groups, G_VARIANT_TYPE("aa{sv}"));
    for (gint group = 0; group < clutter_input_device_get_n_mode_groups(pad); group++) {
        GVariantBuilder group_record;
        GVariantBuilder switch_buttons;
        g_variant_builder_init(&group_record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_init(&switch_buttons, G_VARIANT_TYPE("av"));
        for (gint button = 0; button < clutter_input_device_get_n_buttons(pad); button++) {
            if (clutter_input_device_get_mode_switch_button_group(pad, button) == group)
                g_variant_builder_add(&switch_buttons, "v", g_variant_new_int32(button));
        }
        g_variant_builder_add(&group_record, "{sv}", "group", g_variant_new_int32(group));
        g_variant_builder_add(
            &group_record, "{sv}", "mode_count",
            g_variant_new_int32(clutter_input_device_get_group_n_modes(pad, group)));
        g_variant_builder_add(&group_record, "{sv}", "switch_buttons",
                              g_variant_builder_end(&switch_buttons));
        g_variant_builder_add_value(&mode_groups, g_variant_builder_end(&group_record));
    }
    mode_records = g_variant_ref_sink(g_variant_builder_end(&mode_groups));

    g_variant_builder_init(&features, G_VARIANT_TYPE("aa{sv}"));
    native_pad_help_add_features(&features, display, pad, META_PAD_FEATURE_RING,
                                 clutter_input_device_get_n_rings(pad), "ring",
                                 META_PAD_DIRECTION_CW, "clockwise_label", META_PAD_DIRECTION_CCW,
                                 "counterclockwise_label");
    native_pad_help_add_features(
        &features, display, pad, META_PAD_FEATURE_STRIP, clutter_input_device_get_n_strips(pad),
        "strip", META_PAD_DIRECTION_UP, "up_label", META_PAD_DIRECTION_DOWN, "down_label");
    native_pad_help_add_features(&features, display, pad, META_PAD_FEATURE_DIAL,
                                 clutter_input_device_get_n_dials(pad), "dial",
                                 META_PAD_DIRECTION_CW, "clockwise_label", META_PAD_DIRECTION_CCW,
                                 "counterclockwise_label");
    feature_records = g_variant_ref_sink(g_variant_builder_end(&features));

    monitor_snapshot = gnoblin_control_monitor_snapshot_json(control, FALSE, &snapshot_error);
    if (monitor_snapshot)
        monitor_id = native_monitor_id_for_index(monitor_snapshot, monitor_index);
    if (monitor_id)
        output_names = native_monitor_output_names_for_index(control, monitor_index);
    if (!monitor_id)
        g_debug("gnoblin-native-control: pad-help request has no stable monitor for index %d%s%s",
                monitor_index, snapshot_error ? ": " : "",
                snapshot_error ? snapshot_error->message : "");

    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "device", device);
    if (monitor_id)
        g_variant_builder_add(&fields, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    if (output_names)
        g_variant_builder_add(&fields, "{sv}", "output_names", output_names);
    g_variant_builder_add(&fields, "{sv}", "edition_mode", g_variant_new_boolean(edition_mode));
    g_variant_builder_add(&fields, "{sv}", "buttons", button_records);
    g_variant_builder_add(&fields, "{sv}", "mode_groups", mode_records);
    g_variant_builder_add(&fields, "{sv}", "features", feature_records);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));
    native_publish_request_event(control, "gnoblin.input.pad-help-requested", payload);

    /* A shell owns the overlay; returning NULL leaves Mutter free to emit the
     * next request instead of tracking an actor it does not own. */
    return NULL;
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

gboolean gnoblin_control_monitor_property_changed(JsonObject* previous, JsonObject* current,
                                                  const char* property) {
    return monitor_property_changed(previous, current, property);
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
    for (guint i = 0; i < window_property_names_count; i++) {
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

void publish_layer_changes(GnoblinNativeControl* control, JsonNode* snapshot, guint64 revision) {
    GHashTable* current = gnoblin_control_layer_state_from_snapshot(snapshot);
    if (control->layer_state_initialized) {
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        g_hash_table_iter_init(&iter, current);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            NativeLayerState* state = value;
            NativeLayerState* previous = g_hash_table_lookup(control->layer_state, key);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, state->json, -1, NULL))
                continue;
            JsonNode* layer = json_parser_get_root(parser);
            if (!previous) {
                dispatch_lua_layer_event(control, revision, "gnoblin.layer.created", key, layer,
                                         NULL, NULL);
                continue;
            }
            if (g_strcmp0(previous->comparable_json, state->comparable_json) == 0)
                continue;

            g_autoptr(JsonParser) previous_parser = json_parser_new();
            if (!json_parser_load_from_data(previous_parser, previous->json, -1, NULL))
                continue;
            JsonArray* changed = gnoblin_control_changed_layer_properties(
                json_parser_get_root(previous_parser), layer);
            if (json_array_get_length(changed) > 0)
                dispatch_lua_layer_event(control, revision, "gnoblin.layer.changed", key, layer,
                                         NULL, changed);
            json_array_unref(changed);
        }

        g_hash_table_iter_init(&iter, control->layer_state);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            if (g_hash_table_contains(current, key))
                continue;
            NativeLayerState* previous = value;
            g_autoptr(JsonParser) parser = json_parser_new();
            if (json_parser_load_from_data(parser, previous->json, -1, NULL))
                dispatch_lua_layer_event(control, revision, "gnoblin.layer.removed", key, NULL,
                                         json_parser_get_root(parser), NULL);
        }
    }

    if (control->layer_state)
        g_hash_table_unref(control->layer_state);
    control->layer_state = current;
    control->layer_state_initialized = TRUE;
}

void publish_window_changes(GnoblinNativeControl* control, JsonNode* snapshot, guint64 revision) {
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
        /* json_node_copy() shares the object. Give the comparison its own
         * member table so removing fields cannot mutate the snapshot; nested
         * values are only serialized, so they can remain shared. */
        g_autoptr(JsonNode) comparable = json_node_new(JSON_NODE_OBJECT);
        JsonObject* comparable_record = json_object_new();
        JsonObjectIter member_iter;
        const char* member_name;
        JsonNode* member_value;
        json_object_iter_init_ordered(&member_iter, record);
        while (json_object_iter_next_ordered(&member_iter, &member_name, &member_value))
            json_object_set_member(comparable_record, member_name, json_node_ref(member_value));
        json_node_take_object(comparable, comparable_record);
        json_object_remove_member(json_node_get_object(comparable), "focused");
        json_object_remove_member(json_node_get_object(comparable), "revision");
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

static GVariant* output_renderer(MetaDisplay* display) {
    static char* cached_name;
    static char* cached_vendor;
    static char* cached_api;
    static gboolean cached_accelerated;
    static gboolean cached;
    if (!cached) {
        MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
        ClutterBackend* clutter = backend ? meta_backend_get_clutter_backend(backend) : NULL;
        CoglContext* context = clutter ? clutter_backend_get_cogl_context(clutter) : NULL;
        CoglRenderer* renderer = context ? cogl_context_get_renderer(context) : NULL;
        CoglDriver* driver = context ? cogl_context_get_driver(context) : NULL;
        if (!renderer || !COGL_IS_DRIVER_GL(driver))
            return NULL;
        const char* name = (const char*)glGetString(GL_RENDERER);
        const char* vendor = (const char*)glGetString(GL_VENDOR);
        if (!name || !*name)
            return NULL;
        cached_name = g_strdup(name);
        cached_vendor = g_strdup(vendor);
        CoglDriverId driver_id = cogl_renderer_get_driver_id(renderer);
        cached_api = g_strdup(driver_id == COGL_DRIVER_ID_GLES2 ? "gles2"
                              : driver_id == COGL_DRIVER_ID_GL3 ? "gl"
                                                                : "other");
        cached_accelerated = cogl_renderer_is_hardware_accelerated(renderer);
        g_autofree char* folded = g_ascii_strdown(cached_name, -1);
        if (strstr(folded, "llvmpipe") || strstr(folded, "softpipe") || strstr(folded, "swrast") ||
            strstr(folded, "software rasterizer"))
            cached_accelerated = FALSE;
        cached = TRUE;
    }
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(cached_name));
    if (cached_vendor && *cached_vendor)
        g_variant_builder_add(&record, "{sv}", "vendor", g_variant_new_string(cached_vendor));
    g_variant_builder_add(&record, "{sv}", "api", g_variant_new_string(cached_api));
    g_variant_builder_add(&record, "{sv}", "accelerated",
                          g_variant_new_boolean(cached_accelerated));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static const char* output_transform_name(MtkMonitorTransform transform) {
    switch (transform) {
    case MTK_MONITOR_TRANSFORM_NORMAL:
        return "normal";
    case MTK_MONITOR_TRANSFORM_90:
        return "90";
    case MTK_MONITOR_TRANSFORM_180:
        return "180";
    case MTK_MONITOR_TRANSFORM_270:
        return "270";
    case MTK_MONITOR_TRANSFORM_FLIPPED:
        return "flipped";
    case MTK_MONITOR_TRANSFORM_FLIPPED_90:
        return "flipped-90";
    case MTK_MONITOR_TRANSFORM_FLIPPED_180:
        return "flipped-180";
    case MTK_MONITOR_TRANSFORM_FLIPPED_270:
        return "flipped-270";
    }
    return "normal";
}

GVariant* gnoblin_control_output_list(MetaDisplay* display, GError** error) {
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    MetaMonitorManager* manager = backend ? meta_backend_get_monitor_manager(backend) : NULL;
    if (!manager) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_INITIALIZED,
                            "output enumeration is unavailable before monitor initialization");
        return NULL;
    }

    GVariantBuilder outputs;
    g_autoptr(GVariant) renderer = output_renderer(display);
    g_variant_builder_init(&outputs, G_VARIANT_TYPE("aa{sv}"));
    GList* logicals = meta_monitor_manager_get_logical_monitors(manager);
    for (GList* item = meta_monitor_manager_get_monitors(manager); item; item = item->next) {
        MetaMonitor* monitor = item->data;
        GList* connectors = meta_monitor_get_outputs(monitor);
        MetaMonitorMode* mode = meta_monitor_get_current_mode(monitor);
        if (!mode)
            mode = meta_monitor_get_preferred_mode(monitor);
        int width = 0, height = 0;
        if (mode)
            meta_monitor_mode_get_resolution(mode, &width, &height);
        for (GList* connector = connectors; connector; connector = connector->next) {
            MetaOutput* output = connector->data;
            const char* id = meta_output_get_name(output);
            if (!id || !*id)
                continue;
            MetaLogicalMonitor* logical = NULL;
            for (GList* lm = logicals; lm && !logical; lm = lm->next) {
                GList* members = meta_logical_monitor_get_monitors(lm->data);
                if (g_list_find(members, monitor))
                    logical = lm->data;
            }
            GVariantBuilder record;
            g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
            g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string("display"));
            if (renderer)
                g_variant_builder_add(&record, "{sv}", "renderer", renderer);
            g_variant_builder_add(&record, "{sv}", "enabled",
                                  g_variant_new_boolean(logical != NULL));
            g_variant_builder_add(
                &record, "{sv}", "primary",
                g_variant_new_boolean(logical && meta_logical_monitor_is_primary(logical)));
            const char* display_name = meta_monitor_get_display_name(monitor);
            g_variant_builder_add(
                &record, "{sv}", "name",
                g_variant_new_string(display_name && *display_name ? display_name : id));
            if (meta_monitor_get_vendor(monitor))
                g_variant_builder_add(&record, "{sv}", "make",
                                      g_variant_new_string(meta_monitor_get_vendor(monitor)));
            if (meta_monitor_get_product(monitor))
                g_variant_builder_add(&record, "{sv}", "model",
                                      g_variant_new_string(meta_monitor_get_product(monitor)));
            if (meta_monitor_get_serial(monitor))
                g_variant_builder_add(&record, "{sv}", "serial",
                                      g_variant_new_string(meta_monitor_get_serial(monitor)));
            GVariantBuilder mode_record;
            g_variant_builder_init(&mode_record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&mode_record, "{sv}", "width", g_variant_new_int32(width));
            g_variant_builder_add(&mode_record, "{sv}", "height", g_variant_new_int32(height));
            g_variant_builder_add(
                &mode_record, "{sv}", "refresh_rate",
                g_variant_new_double(mode ? meta_monitor_mode_get_refresh_rate(mode) : 0));
            g_variant_builder_add(&record, "{sv}", "mode", g_variant_builder_end(&mode_record));
            if (logical) {
                MtkRectangle rect = meta_logical_monitor_get_layout(logical);
                const char* logical_id = id;
                for (GList* member = meta_logical_monitor_get_monitors(logical); member;
                     member = member->next)
                    for (GList* member_output = meta_monitor_get_outputs(member->data);
                         member_output; member_output = member_output->next) {
                        const char* member_id = meta_output_get_name(member_output->data);
                        if (member_id && g_strcmp0(member_id, logical_id) < 0)
                            logical_id = member_id;
                    }
                g_variant_builder_add(&record, "{sv}", "monitor_id",
                                      g_variant_new_string(logical_id));
                g_variant_builder_add(&record, "{sv}", "x", g_variant_new_int32(rect.x));
                g_variant_builder_add(&record, "{sv}", "y", g_variant_new_int32(rect.y));
                g_variant_builder_add(
                    &record, "{sv}", "scale",
                    g_variant_new_double(meta_logical_monitor_get_scale(logical)));
                g_variant_builder_add(&record, "{sv}", "transform",
                                      g_variant_new_string(output_transform_name(
                                          meta_logical_monitor_get_transform(logical))));
            } else {
                g_variant_builder_add(&record, "{sv}", "scale", g_variant_new_double(1.0));
                g_variant_builder_add(&record, "{sv}", "transform", g_variant_new_string("normal"));
            }
            g_variant_builder_add_value(&outputs, g_variant_builder_end(&record));
        }
    }
    if (gnoblin_vr_output_record) {
        g_autoptr(GVariant) headset = gnoblin_vr_output_record();
        if (headset)
            g_variant_builder_add_value(&outputs, headset);
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "outputs", g_variant_builder_end(&outputs));
    if (renderer)
        g_variant_builder_add(&result, "{sv}", "renderer", renderer);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
