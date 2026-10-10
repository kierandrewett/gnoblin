#include "gnoblin-native-control-private.h"

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

void update_launch_snapshot(GnoblinNativeControl* control) {
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
