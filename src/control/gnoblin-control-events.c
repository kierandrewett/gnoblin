#include "gnoblin-native-control-private.h"

static gboolean native_touchpad_action_supported(GVariant* gesture) {
    g_autoptr(GVariant) action = g_variant_lookup_value(gesture, "action", NULL);
    if (action && g_variant_is_of_type(action, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(action);
        g_variant_unref(g_steal_pointer(&action));
        action = unwrapped;
    }
    if (!action)
        return FALSE;
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_STRING))
        return FALSE;
    const char* name = g_variant_get_string(action, NULL);
    return g_str_equal(name, "workspace.next") || g_str_equal(name, "workspace.previous") ||
           g_str_equal(name, "window.close") || g_str_equal(name, "window.minimize") ||
           g_str_equal(name, "window.toggle-maximize");
}

GVariant* filter_native_touchpad_gestures(GVariant* gestures) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
    if (gestures && g_variant_is_of_type(gestures, G_VARIANT_TYPE("av"))) {
        for (gsize i = 0; i < g_variant_n_children(gestures); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(gestures, i);
            g_autoptr(GVariant) gesture = g_variant_get_variant(boxed);
            g_autoptr(GVariant) command = g_variant_lookup_value(gesture, "command", NULL);
            gboolean supported = native_touchpad_action_supported(gesture);
            if (command && g_variant_is_of_type(command, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unwrapped = g_variant_get_variant(command);
                g_variant_unref(g_steal_pointer(&command));
                command = unwrapped;
            }
            if (command && g_variant_is_of_type(command, G_VARIANT_TYPE("av")) &&
                g_variant_n_children(command) > 0)
                supported = TRUE;
            if (supported)
                g_variant_builder_add_value(&builder, g_variant_new_variant(gesture));
        }
    }
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static gboolean run_native_touchpad_command(GVariant* gesture, GError** error) {
    g_autoptr(GVariant) command = g_variant_lookup_value(gesture, "command", NULL);
    if (command && g_variant_is_of_type(command, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(command);
        g_variant_unref(g_steal_pointer(&command));
        command = unwrapped;
    }
    if (!command || !g_variant_is_of_type(command, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(command) == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "touchpad gesture command is empty or invalid");
        return FALSE;
    }
    gsize count = g_variant_n_children(command);
    g_auto(GStrv) argv = g_new0(char*, count + 1);
    for (gsize i = 0; i < count; i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(command, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING) ||
            (i == 0 && !*g_variant_get_string(item, NULL))) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                "touchpad gesture command arguments must be strings and argv[0] must not be empty");
            return FALSE;
        }
        argv[i] = g_strdup(g_variant_get_string(item, NULL));
    }
    return gnoblin_control_queue_command("touchpad", argv[0], argv, error);
}

static void run_native_touchpad_action(GnoblinNativeControl* control, GVariant* gesture) {
    g_autoptr(GVariant) action = g_variant_lookup_value(gesture, "action", NULL);
    if (action && g_variant_is_of_type(action, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(action);
        g_variant_unref(g_steal_pointer(&action));
        action = unwrapped;
    }
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) arguments = NULL;
    g_autofree char* method = NULL;
    if (!action) {
        if (!run_native_touchpad_command(gesture, &error))
            g_warning("gnoblin-native-control: touchpad command failed: %s", error->message);
        return;
    }
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_STRING))
        return;
    const char* name = g_variant_get_string(action, NULL);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    if (g_str_equal(name, "workspace.next") || g_str_equal(name, "workspace.previous")) {
        if (control->wayland_compositor &&
            meta_wayland_session_lock_is_active(control->wayland_compositor))
            return;
        method = g_strdup(name);
    } else if (g_str_equal(name, "window.close") || g_str_equal(name, "window.minimize") ||
               g_str_equal(name, "window.toggle-maximize")) {
        if (control->wayland_compositor &&
            meta_wayland_session_lock_is_active(control->wayland_compositor))
            return;
        MetaWindow* window = meta_display_get_focus_window(control->display);
        if (!window)
            return;
        g_autofree char* window_id = native_window_id(window);
        if (g_str_equal(name, "window.toggle-maximize")) {
            method = g_strdup("window.set_maximized");
            g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(window_id));
            g_variant_builder_add(&builder, "{sv}", "enabled",
                                  g_variant_new_boolean(!meta_window_is_maximized(window)));
        } else {
            method = g_strdup(name);
            g_variant_builder_add(&builder, "{sv}", "id", g_variant_new_string(window_id));
        }
    } else {
        return;
    }
    arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, method, arguments, &error);
    if (!result)
        g_warning("gnoblin-native-control: touchpad action %s failed: %s", name,
                  error ? error->message : "native action is unavailable");
}

gboolean native_config_event(MetaDisplay* display, const char* event, GVariant* document,
                             GVariant* payload, gpointer user_data) {
    (void)display;
    (void)document;
    GnoblinNativeControl* control = user_data;
    if (!event || !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    if (!g_str_equal(event, "mutter.touchpad.gesture"))
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
    gboolean claimed = FALSE;
    if (control->touchpad_router && control->native_touchpad_gestures) {
        const char* context = control->wayland_compositor && meta_wayland_session_lock_is_active(
                                                                 control->wayland_compositor)
                                  ? "locked"
                                  : "unlocked";
        g_autoptr(GVariant) matched = NULL;
        claimed = gnoblin_touchpad_router_handle(control->touchpad_router,
                                                 control->native_touchpad_gestures, payload,
                                                 context, &matched);
        if (matched)
            run_native_touchpad_action(control, matched);
    }
    publish_native_socket_event(control, root);
    return claimed;
}

void native_native_event(MetaDisplay* display, const char* event, GVariant* payload,
                         gpointer user_data) {
    (void)display;
    GnoblinNativeControl* control = user_data;
    if (!event ||
        (!g_str_equal(event, "gnoblin.animation.started") &&
         !g_str_equal(event, "gnoblin.animation.finished") &&
         !g_str_equal(event, "gnoblin.window.activation-denied")) ||
        !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return;

    GVariantBuilder enriched;
    g_variant_builder_init(&enriched, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&enriched, "{sv}", "name", g_variant_new_string(event));
    g_variant_builder_add(&enriched, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_variant_builder_add(&enriched, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    GVariantIter iterator;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iterator, payload);
    while (g_variant_iter_next(&iterator, "{&sv}", &key, &value)) {
        g_variant_builder_add(&enriched, "{sv}", key, value);
        g_variant_unref(value);
    }
    g_autoptr(GVariant) event_payload = g_variant_ref_sink(g_variant_builder_end(&enriched));
    g_autoptr(JsonNode) root = json_from_variant(event_payload);
    JsonObject* object = json_node_get_object(root);
    /* Keep the animation's event field distinct from the socket event name. */
    JsonNode* animation_event = json_object_get_member(object, "event");
    if (animation_event) {
        json_object_set_member(object, "animation_event", json_node_copy(animation_event));
        json_object_remove_member(object, "event");
    }
    publish_native_socket_event(control, root);
    native_runtime_dispatch_event(control, event, event_payload);
}

gboolean focus_token_has_valid_shape(const char* token) {
    if (!token || strlen(token) != 64)
        return FALSE;
    for (gsize i = 0; i < 64; i++)
        if (!g_ascii_isxdigit(token[i]))
            return FALSE;
    return TRUE;
}

gboolean native_socket_has_exact_fields(JsonObject* object, const char* const* fields,
                                        gsize n_fields) {
    if (!object || json_object_get_size(object) != n_fields)
        return FALSE;
    GList* members = json_object_get_members(object);
    gboolean exact = TRUE;
    for (GList* item = members; item && exact; item = item->next) {
        const char* name = item->data;
        exact = FALSE;
        for (gsize i = 0; i < n_fields; i++) {
            if (g_str_equal(name, fields[i])) {
                exact = TRUE;
                break;
            }
        }
    }
    g_list_free(members);
    return exact;
}

gboolean native_socket_has_nul_escape(const char* data, gsize length) {
    for (gsize i = 0; i < length; i++) {
        if (data[i] != '\\')
            continue;
        gsize slash_count = 1;
        while (i + slash_count < length && data[i + slash_count] == '\\')
            slash_count++;
        gsize escape = i + slash_count;
        if ((slash_count & 1) && escape + 5 < length && data[escape] == 'u' &&
            data[escape + 1] == '0' && data[escape + 2] == '0' && data[escape + 3] == '0' &&
            data[escape + 4] == '0')
            return TRUE;
        i += slash_count - 1;
    }
    return FALSE;
}

gboolean native_socket_take_focus_grant(Client* client, JsonObject* arguments,
                                        NativeFocusGrant* out_grant, GError** error) {
    JsonNode* token_node = arguments ? json_object_get_member(arguments, "focus_context") : NULL;
    if (!client || client->closing || !client->control || !client->focus_grants || !token_node ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "operation requires a connection-bound focus_context");
        return FALSE;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "focus_context is invalid or revoked");
        return FALSE;
    }
    NativeFocusGrant* stored = g_hash_table_lookup(client->focus_grants, token);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "focus_context is expired, revoked, or belongs to another connection");
        return FALSE;
    }
    *out_grant = *stored;
    g_hash_table_remove(client->focus_grants, token);
    revoke_focus_grants_for_handle(client->control, out_grant->handle);
    return TRUE;
}

GVariant* native_socket_focus_window(Client* client, JsonObject* json_arguments, GError** error) {
    if (!client || client->closing || !client->control || !json_arguments ||
        !client->focus_grants) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus requires a live connection grant");
        return NULL;
    }

    JsonNode* activation_node = json_object_get_member(json_arguments, "activation_token");
    if (activation_node) {
        if (client->api_minor < 32) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "XDG Activation window focus requires API version 1.32");
            return NULL;
        }
        JsonNode* id_node = json_object_get_member(json_arguments, "id");
        if (json_object_get_size(json_arguments) != 2 || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING ||
            !json_node_get_string(id_node)[0] || !JSON_NODE_HOLDS_VALUE(activation_node) ||
            json_node_get_value_type(activation_node) != G_TYPE_STRING ||
            !json_node_get_string(activation_node)[0] ||
            strlen(json_node_get_string(activation_node)) > 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.focus requires exactly id and activation_token strings");
            return NULL;
        }
        if (client->peer_pid <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "window.focus cannot verify the activation token owner");
            return NULL;
        }
        const char* wanted_id = json_node_get_string(id_node);
        MetaWindow* target = native_window_by_stable_id(client->control, wanted_id);
        if (!target || meta_window_is_skip_taskbar(target) ||
            meta_window_is_override_redirect(target)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "window ID not found; list windows first");
            return NULL;
        }
        if (!meta_wayland_activation_focus_window_with_token(
                client->control->wayland_compositor, target, json_node_get_string(activation_node),
                client->peer_pid)) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                "window.focus activation token was invalid, expired, or unavailable");
            return NULL;
        }
        GVariantBuilder result_builder;
        g_variant_builder_init(&result_builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result_builder, "{sv}", "id", g_variant_new_string(wanted_id));
        return g_variant_ref_sink(g_variant_builder_end(&result_builder));
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

GVariant* native_socket_begin_window_grab(Client* client, const char* method,
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

GVariant* native_socket_begin_menu_window_grab(Client* client, const char* method,
                                               JsonObject* json_arguments, GError** error) {
    if (!client || client->closing || !client->control || !client->menu_grants || !json_arguments ||
        (!g_str_equal(method, "window.begin_move") &&
         !g_str_equal(method, "window.begin_resize"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu operation requires a live API 1.30 connection");
        return NULL;
    }
    JsonNode* token_node = json_object_get_member(json_arguments, "menu_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "WM menu operation requires its menu_context capability");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    NativeMenuGrant* stored =
        focus_token_has_valid_shape(token) ? g_hash_table_lookup(client->menu_grants, token) : NULL;
    if (!stored) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
            "menu_context is invalid, expired, consumed, or belongs to another connection");
        return NULL;
    }
    NativeMenuGrant grant = *stored;
    g_hash_table_remove(client->menu_grants, token);

    gboolean resize = g_str_equal(method, "window.begin_resize");
    gboolean exact = json_object_get_size(json_arguments) == (resize ? 2 : 1);
    GList* members = json_object_get_members(json_arguments);
    for (GList* item = members; item && exact; item = item->next) {
        const char* name = item->data;
        if (!g_str_equal(name, "menu_context") && !g_str_equal(name, "edge"))
            exact = FALSE;
    }
    g_list_free(members);
    JsonNode* edge = json_object_get_member(json_arguments, "edge");
    if (resize &&
        (!edge || !JSON_NODE_HOLDS_VALUE(edge) || json_node_get_value_type(edge) != G_TYPE_STRING))
        exact = FALSE;
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    if (exact && resize)
        g_variant_builder_add(&builder, "{sv}", "edge",
                              g_variant_new_string(json_node_get_string(edge)));
    if (!exact)
        g_variant_builder_add(&builder, "{sv}", "_malformed", g_variant_new_boolean(TRUE));
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&builder));
    /* The native operation consumes its context even when its arguments are malformed. */
    return gnoblin_native_control_begin_menu_window_grab(client->control->display, method,
                                                         arguments, grant.handle, grant.generation,
                                                         client->client_id, error);
}

void native_launch_free(gpointer data) {
    NativeLaunch* launch = data;
    g_free(launch->token);
    g_free(launch->application);
    g_free(launch->normalized_application);
    g_free(launch->state);
    g_free(launch->initial_focus_id);
    g_clear_pointer(&launch->initial_window_ids, g_hash_table_unref);
    g_free(launch);
}

char* normalize_launch_application(const char* value) {
    g_autofree char* normalized = g_ascii_strdown(value ? value : "", -1);
    g_strstrip(normalized);
    if (g_str_has_suffix(normalized, ".desktop"))
        normalized[strlen(normalized) - strlen(".desktop")] = '\0';
    return g_steal_pointer(&normalized);
}

char* native_window_id(MetaWindow* window) {
    return g_strdup_printf("%u", meta_window_get_stable_sequence(window));
}

void gnoblin_native_control_window_menu_requested(MetaDisplay* display, MetaWindow* window,
                                                  MetaWindowMenuType menu, int x, int y) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    const char* menu_type = menu == META_WINDOW_MENU_WM    ? "wm"
                            : menu == META_WINDOW_MENU_APP ? "app"
                                                           : NULL;
    if (!control || control->stopping || !window || !menu_type)
        return;
    g_autofree char* window_id = native_window_id(window);
    GVariantBuilder fields;
    g_variant_builder_init(&fields, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&fields, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&fields, "{sv}", "menu_type", g_variant_new_string(menu_type));
    g_variant_builder_add(&fields, "{sv}", "x", g_variant_new_int32(x));
    g_variant_builder_add(&fields, "{sv}", "y", g_variant_new_int32(y));
    gint64 runtime_expiry = 0;
    guint64 runtime_handle =
        menu == META_WINDOW_MENU_WM
            ? native_menu_context_create(control, window_id, 0, &runtime_expiry)
            : 0;
    if (runtime_handle) {
        g_variant_builder_add(&fields, "{sv}", "_menu_context_handle",
                              g_variant_new_uint64(runtime_handle));
        g_variant_builder_add(&fields, "{sv}", "_menu_context_generation",
                              g_variant_new_uint64(native_config_generation(control)));
        g_variant_builder_add(&fields, "{sv}", "_menu_context_expires_at_us",
                              g_variant_new_int64(runtime_expiry));
    }
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&fields));

    g_autoptr(JsonNode) event = json_from_variant(payload);
    if (!event || !JSON_NODE_HOLDS_OBJECT(event))
        return;
    JsonObject* event_object = json_node_get_object(event);
    json_object_set_string_member(event_object, "name", "gnoblin.window.menu-requested");
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->closing || client->event_api_minor < 27 || !client->event_subscriptions ||
            !g_hash_table_contains(client->event_subscriptions, "gnoblin.window.menu-requested"))
            continue;
        g_autoptr(JsonNode) socket_event = json_node_copy(event);
        JsonObject* socket_object = json_node_get_object(socket_event);
        json_object_remove_member(socket_object, "_menu_context_handle");
        json_object_remove_member(socket_object, "_menu_context_generation");
        json_object_remove_member(socket_object, "_menu_context_expires_at_us");
        native_menu_prune_client_grants(client, g_get_monotonic_time());
        if (menu == META_WINDOW_MENU_WM && client->event_api_minor >= 30 && client->menu_grants &&
            g_hash_table_size(client->menu_grants) < MAX_FOCUS_CONTEXTS) {
            char token[65];
            if (focus_token_random(token)) {
                gint64 expires = 0;
                guint64 handle =
                    native_menu_context_create(control, window_id, client->client_id, &expires);
                if (handle) {
                    NativeMenuGrant* grant = g_new0(NativeMenuGrant, 1);
                    grant->handle = handle;
                    grant->generation = native_config_generation(control);
                    grant->expires_at_us = expires;
                    g_hash_table_insert(client->menu_grants, g_strdup(token), grant);
                    json_object_set_string_member(socket_object, "menu_context", token);
                }
            }
        }
        json_object_set_string_member(socket_object, "event", "gnoblin.window.menu-requested");
        json_object_remove_member(socket_object, "name");
        g_autofree char* encoded = json_to_string(socket_event, FALSE);
        send_response(client, g_strconcat(encoded, "\n", NULL));
    }
    g_list_free(clients);
    if (runtime_handle)
        native_runtime_dispatch_event(control, "gnoblin.window.menu-requested", payload);
}

/* The drag snap constants live in gnoblin-control-internal.h:
 * #define NATIVE_DRAG_MOD_CONTROL. */

gboolean gnoblin_native_control_take_window_drag_snap(MetaDisplay* display, guint64 drag_id,
                                                      MetaWindow* window, int release_x,
                                                      int release_y, guint32 modifiers,
                                                      MtkRectangle* out_frame, char** out_target_id,
                                                      gboolean* out_maximize) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    NativeWindowDrag* drag = control && control->window_drags
                                 ? g_hash_table_lookup(control->window_drags, &drag_id)
                                 : NULL;
    if (!control || control->stopping || !control->supervised_runtime ||
        !control->runtime_generation || !drag || !window ||
        drag->owner_generation != control->runtime_generation ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->settings_revision != native_config_revision(control) ||
        !gnoblin_control_window_drag_refresh(control, drag, window, release_x, release_y,
                                             modifiers) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return FALSE;
    guint control_modifier = (modifiers & CLUTTER_CONTROL_MASK) ? NATIVE_DRAG_MOD_CONTROL : 0;
    for (guint i = 0; i < drag->targets->len; i++) {
        NativeSnapTarget* target = g_ptr_array_index(drag->targets, i);
        if (release_x < target->hit.x || release_y < target->hit.y ||
            release_x >= (gint64)target->hit.x + target->hit.width ||
            release_y >= (gint64)target->hit.y + target->hit.height ||
            (control_modifier & target->required_modifiers) != target->required_modifiers ||
            (control_modifier & target->forbidden_modifiers) != 0 ||
            !gnoblin_control_rect_contains(&drag->work_area, &target->hit) ||
            !gnoblin_control_rect_contains(&drag->work_area, &target->frame))
            continue;
        *out_frame = target->frame;
        *out_target_id = g_strdup(target->id);
        *out_maximize = target->maximize;
        if (target->maximize) {
            g_hash_table_remove(control->snap_restore_frames, window);
        } else if (!g_hash_table_contains(control->snap_restore_frames, window)) {
            MtkRectangle* original_frame = g_new(MtkRectangle, 1);
            *original_frame = drag->original_frame;
            g_hash_table_insert(control->snap_restore_frames, window, original_frame);
        }
        g_free(drag->committed_target_id);
        drag->committed_target_id = g_strdup(target->id);
        return TRUE;
    }
    return FALSE;
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
        context.generation != native_config_generation(control) ||
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

gboolean native_text_modifiers_allowed(GnoblinNativeControl* control, xkb_mod_mask_t allowed) {
    ClutterKeymap* keymap =
        control && control->input_seat ? clutter_seat_get_keymap(control->input_seat) : NULL;
    if (!keymap)
        return FALSE;
    xkb_mod_mask_t depressed = 0, latched = 0, locked = 0;
    clutter_keymap_get_modifier_state(keymap, &depressed, &latched, &locked);
    return ((depressed | latched | locked) & ~allowed) == 0;
}

gboolean native_text_valid(const char* text) {
    if (!text || !*text || strlen(text) > 256 || !g_utf8_validate(text, -1, NULL))
        return FALSE;
    for (const char* cursor = text; *cursor; cursor = g_utf8_next_char(cursor))
        if (g_unichar_iscntrl(g_utf8_get_char(cursor)))
            return FALSE;
    return TRUE;
}

GVariant* native_socket_create_text_target(Client* client, JsonObject* arguments, GError** error) {
    NativeFocusGrant grant = {0};
    if (!native_socket_take_focus_grant(client, arguments, &grant, error))
        return NULL;

    GVariant* result = gnoblin_native_control_create_text_target(
        client->control->display, grant.handle, grant.generation, grant.expires_at_us, error);
    if (!result)
        return NULL;

    const char* token = NULL;
    if (!g_variant_lookup(result, "target", "&s", &token) || !token) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native text target result has no target token");
        return NULL;
    }
    NativeTextTarget* target = g_hash_table_lookup(client->control->text_targets, token);
    if (!target) {
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "native text target was not retained");
        return NULL;
    }
    target->socket_owner_client_id = client->client_id;

    const char* fields[] = {"focus_context"};
    if (!native_socket_has_exact_fields(arguments, fields, G_N_ELEMENTS(fields))) {
        g_hash_table_remove(client->control->text_targets, token);
        g_variant_unref(result);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.text_target accepts only focus_context");
        return NULL;
    }
    return result;
}

GVariant* native_socket_insert_text(Client* client, JsonObject* json_arguments,
                                    gboolean has_nul_escape, GError** error) {
    JsonNode* token_node = json_arguments ? json_object_get_member(json_arguments, "target") : NULL;
    if (!client || client->closing || !client->control || !client->control->text_targets ||
        !token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text requires a target token");
        return NULL;
    }
    const char* token = json_node_get_string(token_node);
    if (!focus_token_has_valid_shape(token)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget is invalid or revoked");
        return NULL;
    }
    NativeTextTarget* stored = g_hash_table_lookup(client->control->text_targets, token);
    if (!stored || stored->socket_owner_client_id != client->client_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "TextTarget is expired, revoked, or belongs to another connection");
        return NULL;
    }

    if (has_nul_escape) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text rejects embedded NUL characters");
        return NULL;
    }

    const char* fields[] = {"target", "text"};
    JsonNode* text_node = json_object_get_member(json_arguments, "text");
    if (!native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) ||
        !text_node || !JSON_NODE_HOLDS_VALUE(text_node) ||
        json_node_get_value_type(text_node) != G_TYPE_STRING ||
        !native_text_valid(json_node_get_string(text_node))) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "input.insert_text requires 1-256 bytes of UTF-8 text without controls");
        return NULL;
    }

    g_autoptr(JsonNode) arguments_node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(arguments_node, json_object_ref(json_arguments));
    g_autoptr(GVariant) arguments = variant_from_json(arguments_node);
    if (!arguments) {
        g_hash_table_remove(client->control->text_targets, token);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input.insert_text arguments are invalid");
        return NULL;
    }
    return native_insert_text_owned(client->control->display, arguments, client->client_id, error);
}

gboolean focus_identity_matches(GnoblinNativeControl* control, guint64 surface_id,
                                guint64 client_id, guint64 focus_epoch) {
    MetaWaylandSeat* seat =
        control && control->wayland_compositor ? control->wayland_compositor->seat : NULL;
    guint64 current_surface = 0, current_client = 0, current_epoch = 0;
    return seat &&
           meta_wayland_seat_get_gnoblin_focus_identity(seat, &current_surface, &current_client,
                                                        &current_epoch) &&
           current_surface == surface_id && current_client == client_id &&
           current_epoch == focus_epoch;
}

gboolean native_monitor_lookup(GnoblinNativeControl* control, const char* wanted_id,
                               int* monitor_number, MtkRectangle* monitor_rect) {
    GHashTableIter iter;
    gpointer value;
    if (!control->monitor_state || !wanted_id)
        return FALSE;
    g_hash_table_iter_init(&iter, control->monitor_state);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeMonitorState* state = value;
        g_autoptr(JsonParser) parser = json_parser_new();
        if (!json_parser_load_from_data(parser, state->json, -1, NULL))
            continue;
        JsonObject* record = json_node_get_object(json_parser_get_root(parser));
        const char* id = json_object_get_string_member_with_default(record, "id", NULL);
        if (!id || !g_str_equal(id, wanted_id))
            continue;
        *monitor_number = (int)json_object_get_int_member(record, "index");
        *monitor_rect = (MtkRectangle){
            .x = (int)json_object_get_int_member(record, "x"),
            .y = (int)json_object_get_int_member(record, "y"),
            .width = (int)json_object_get_int_member(record, "width"),
            .height = (int)json_object_get_int_member(record, "height"),
        };
        return *monitor_number >= 0 && monitor_rect->width > 0 && monitor_rect->height > 0;
    }
    return FALSE;
}

GVariant* gnoblin_native_control_create_snap_context(MetaDisplay* display, guint64 context_handle,
                                                     guint64 generation, gint64 expires_at_us,
                                                     GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !context_handle || !control->focus_contexts ||
        !control->snap_contexts) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context has no live FocusContext");
        return NULL;
    }
    prune_focus_contexts(control, g_get_monotonic_time());
    NativeFocusContext* stored = g_hash_table_lookup(control->focus_contexts, &context_handle);
    if (!stored) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context FocusContext was already consumed or revoked");
        return NULL;
    }
    NativeFocusContext source = *stored;
    g_hash_table_remove(control->focus_contexts, &context_handle);
    revoke_focus_grants_for_handle(control, context_handle);
    if (source.generation != generation || generation != native_config_generation(control) ||
        source.expires_at_us != expires_at_us || expires_at_us <= g_get_monotonic_time() ||
        !focus_identity_matches(control, source.surface_id, source.client_id, source.focus_epoch) ||
        !native_text_modifiers_allowed(control, source.allowed_modifier_mask) ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.snap_context FocusContext expired or no longer matches focus");
        return NULL;
    }
    if (g_hash_table_size(control->snap_contexts) >= MAX_SNAP_CONTEXTS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "too many active snap contexts; retry after one expires");
        return NULL;
    }
    MetaWindow* window = meta_display_get_focus_window(display);
    if (!window || meta_window_is_skip_taskbar(window) ||
        meta_window_is_override_redirect(window) || !meta_window_allows_move(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "focused window cannot be snapped");
        return NULL;
    }
    g_autofree char* window_id = native_window_id(window);
    g_autofree char* monitor_id = NULL;
    MtkRectangle monitor_rect, work_area;
    if (!gnoblin_control_drag_monitor_snapshot(control, window, &monitor_id, &monitor_rect,
                                               &work_area)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "focused window has no current monitor");
        return NULL;
    }
    char token[65] = {0};
    gboolean unique = FALSE;
    for (guint attempt = 0; attempt < 4; attempt++) {
        if (!focus_token_random(token))
            break;
        if (!g_hash_table_contains(control->snap_contexts, token)) {
            unique = TRUE;
            break;
        }
    }
    if (!unique) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "could not allocate one-use SnapContext");
        return NULL;
    }
    NativeSnapContext* snap = g_new0(NativeSnapContext, 1);
    snap->settings_revision = native_config_revision(control);
    snap->native_generation = generation;
    snap->expires_at_us = expires_at_us;
    snap->window_id = g_strdup(window_id);
    snap->monitor_id = g_strdup(monitor_id);
    snap->work_area = work_area;
    meta_window_get_frame_rect(window, &snap->original_frame);
    g_hash_table_insert(control->snap_contexts, g_strdup(token), snap);

    GVariantBuilder result, monitor, area;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&monitor, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&area, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "context", g_variant_new_string(token));
    g_variant_builder_add(&result, "{sv}", "window_id", g_variant_new_string(window_id));
    g_variant_builder_add(&result, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&result, "{sv}", "expires_at_us", g_variant_new_int64(expires_at_us));
#define ADD_SNAP_RECT(builder, rect)                                                               \
    g_variant_builder_add((builder), "{sv}", "x", g_variant_new_int32((rect).x));                  \
    g_variant_builder_add((builder), "{sv}", "y", g_variant_new_int32((rect).y));                  \
    g_variant_builder_add((builder), "{sv}", "width", g_variant_new_int32((rect).width));          \
    g_variant_builder_add((builder), "{sv}", "height", g_variant_new_int32((rect).height))
    ADD_SNAP_RECT(&monitor, monitor_rect);
    ADD_SNAP_RECT(&area, work_area);
#undef ADD_SNAP_RECT
    g_variant_builder_add(&result, "{sv}", "monitor", g_variant_builder_end(&monitor));
    g_variant_builder_add(&result, "{sv}", "work_area", g_variant_builder_end(&area));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
