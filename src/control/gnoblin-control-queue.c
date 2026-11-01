#include "gnoblin-native-control-private.h"
#include "core/stack.h"

gboolean native_api_animation_read_method(const char* method) {
    return method &&
           (g_str_equal(method, "animation.list") || g_str_equal(method, "animation.get") ||
            g_str_equal(method, "animation.surfaces") || g_str_equal(method, "animation.inspect"));
}

gboolean native_api_read_method(const char* method) {
    return method &&
           (g_str_equal(method, "window.list") || g_str_equal(method, "windows.list") ||
            g_str_equal(method, "version") || g_str_equal(method, "capabilities.list") ||
            g_str_equal(method, "focus.history") || g_str_equal(method, "settings") ||
            g_str_equal(method, "config.path") || g_str_equal(method, "config.default") ||
            g_str_equal(method, "appearance.color_scheme") || g_str_equal(method, "focus.policy") ||
            g_str_equal(method, "session.activity") || g_str_equal(method, "session.status") ||
            g_str_equal(method, "runtime.status") ||
            g_str_equal(method, "layer.animation_policy") ||
            g_str_equal(method, "workspaces.list") || g_str_equal(method, "output.list") ||
            g_str_equal(method, "monitors.list") || g_str_equal(method, "outputs.list") ||
            g_str_equal(method, "outputs.renderer") ||
            g_str_equal(method, "monitors.privacy_screen") || g_str_equal(method, "layers.list") ||
            g_str_equal(method, "launches.list") || g_str_equal(method, "launches.snapshot") ||
            g_str_equal(method, "shortcuts.list") || g_str_equal(method, "shortcuts.actions") ||
            g_str_equal(method, "input.orientation_lock"));
}

gboolean runtime_config_values_equal(GVariant* left, GVariant* right) {
    return gnoblin_control_runtime_config_values_equal(left, right);
}

char* queue_runtime_api_request_internal(Client* client, const char* request_id, const char* method,
                                         GVariant* arguments, const char* kind,
                                         const char* legacy_window_action,
                                         const char* legacy_window_id) {
    GnoblinNativeControl* control = client->control;
    if (control->runtime_worker_suspended)
        return encode_response(request_id, NULL, "Lua worker is restarting");
    guint64 channel_id = ++control->next_runtime_request_id;
    if (channel_id == 0)
        channel_id = ++control->next_runtime_request_id;
    guint64* key = g_new(guint64, 1);
    *key = channel_id;
    PendingRuntimeRequest* pending = g_new0(PendingRuntimeRequest, 1);
    pending->client = client;
    pending->request_id = g_strdup(request_id);
    pending->legacy_window_action = g_strdup(legacy_window_action);
    pending->legacy_window_id = g_strdup(legacy_window_id);
    pending->config_reload = g_str_equal(kind, "reload");
    g_hash_table_insert(control->pending_runtime_requests, key, pending);
    client->pending_deferred_requests++;

    GVariantBuilder request;
    g_variant_builder_init(&request, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&request, "{sv}", "kind", g_variant_new_string(kind));
    g_variant_builder_add(&request, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&request, "{sv}", "arguments", arguments);
    if (g_str_equal(kind, "call"))
        g_variant_builder_add(&request, "{sv}", "client_id",
                              g_variant_new_uint64(client->client_id));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&request));
    g_autoptr(GError) error = NULL;
    if (native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_API_REQUEST, channel_id, payload,
                            &error))
        return NULL;

    g_hash_table_remove(control->pending_runtime_requests, &channel_id);
    client->pending_deferred_requests--;
    client_maybe_free(client);
    return encode_response(request_id, NULL,
                           error ? error->message : "could not queue runtime API request");
}

char* queue_runtime_api_request(Client* client, const char* request_id, const char* method,
                                GVariant* arguments, const char* kind) {
    return queue_runtime_api_request_internal(client, request_id, method, arguments, kind, NULL,
                                              NULL);
}

/* The socket protocol is deliberately client-shaped, but the compositor's
 * own diagnostic UI also needs a narrow route into the same supervised Lua
 * runtime. Keep this separate from Client: it has no peer credentials, no
 * JSON response and no ownership relationship with a socket connection. */
static guint64 queue_native_runtime_request(GnoblinNativeControl* control, const char* kind,
                                            const char* method, GVariant* arguments,
                                            GnoblinNativeRuntimeResponseFunc callback,
                                            gpointer user_data, GDestroyNotify destroy,
                                            GError** error) {
    if (!callback) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "a runtime response callback is required");
        return 0;
    }
    if (!control || control->stopping || !control->supervised_runtime || control->runtime_fd < 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin runtime channel is unavailable");
        return 0;
    }
    /* native_runtime_send intentionally drops non-lifecycle packets during a
     * worker handover. A console command must instead fail visibly; silently
     * losing a command makes the panel misleading and leaves its callback
     * outstanding forever. */
    if (control->runtime_worker_suspended || !control->runtime_hello_sent) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                            "Lua runtime is not ready");
        return 0;
    }

    guint64 request_id = ++control->next_runtime_request_id;
    if (request_id == 0)
        request_id = ++control->next_runtime_request_id;
    guint64* key = g_new(guint64, 1);
    *key = request_id;
    PendingRuntimeRequest* pending = g_new0(PendingRuntimeRequest, 1);
    pending->callback = callback;
    pending->config_reload = g_str_equal(kind, "reload");
    pending->callback_data = user_data;
    pending->callback_destroy = destroy;
    g_hash_table_insert(control->pending_runtime_requests, key, pending);

    GVariantBuilder request;
    g_variant_builder_init(&request, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&request, "{sv}", "kind", g_variant_new_string(kind));
    g_variant_builder_add(&request, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&request, "{sv}", "arguments", arguments);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&request));
    if (native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_API_REQUEST, request_id, payload,
                            error))
        return request_id;

    /* The caller retains callback_data when queuing fails. */
    pending->callback_destroy = NULL;
    g_hash_table_remove(control->pending_runtime_requests, &request_id);
    if (error && *error && !g_error_matches(*error, G_IO_ERROR, G_IO_ERROR_NO_SPACE) &&
        !g_error_matches(*error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT))
        native_runtime_abort(control);
    return 0;
}

guint64 gnoblin_native_control_console_eval(MetaDisplay* display, const char* code,
                                            GnoblinNativeRuntimeResponseFunc callback,
                                            gpointer user_data, GDestroyNotify destroy,
                                            GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!code) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua console input is required");
        return 0;
    }
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "code", g_variant_new_string(code));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&arguments));
    return queue_native_runtime_request(control, "console", "console.eval", payload, callback,
                                        user_data, destroy, error);
}

guint64 gnoblin_native_control_console_complete(MetaDisplay* display, const char* source,
                                                GnoblinNativeRuntimeResponseFunc callback,
                                                gpointer user_data, GDestroyNotify destroy,
                                                GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!source || !g_utf8_validate(source, -1, NULL) || strlen(source) > 16 * 1024) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Lua console completion source must be UTF-8 and no more than 16 KiB");
        return 0;
    }
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "code", g_variant_new_string(source));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&arguments));
    return queue_native_runtime_request(control, "console", "console.complete", payload, callback,
                                        user_data, destroy, error);
}

guint64 gnoblin_native_control_reload_runtime_config(MetaDisplay* display,
                                                     GnoblinNativeRuntimeResponseFunc callback,
                                                     gpointer user_data, GDestroyNotify destroy,
                                                     GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&arguments));
    return queue_native_runtime_request(control, "reload", "runtime.reload_config", payload,
                                        callback, user_data, destroy, error);
}

static gboolean json_integer(JsonNode* node, gint64* value) {
    if (!node || !JSON_NODE_HOLDS_VALUE(node))
        return FALSE;
    GType type = json_node_get_value_type(node);
    if (type != G_TYPE_INT && type != G_TYPE_INT64)
        return FALSE;
    *value = json_node_get_int(node);
    return TRUE;
}

GVariant* native_socket_snap_rect(JsonNode* node) {
    if (!node || !JSON_NODE_HOLDS_OBJECT(node))
        return NULL;
    JsonObject* object = json_node_get_object(node);
    if (json_object_get_size(object) != 4)
        return NULL;
    const char* names[] = {"x", "y", "width", "height"};
    gint32 values[4];
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        gint64 value;
        if (!json_integer(json_object_get_member(object, names[i]), &value) || value < G_MININT32 ||
            value > G_MAXINT32)
            return NULL;
        values[i] = value;
    }
    GVariantBuilder rect;
    g_variant_builder_init(&rect, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < G_N_ELEMENTS(names); i++)
        g_variant_builder_add(&rect, "{sv}", names[i], g_variant_new_int32(values[i]));
    return g_variant_ref_sink(g_variant_builder_end(&rect));
}

static GVariant* native_socket_snap_target(JsonNode* node) {
    if (!node || !JSON_NODE_HOLDS_OBJECT(node))
        return NULL;
    JsonObject* object = json_node_get_object(node);
    if (json_object_get_size(object) < 3 || json_object_get_size(object) > 6)
        return NULL;
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "id") && !g_str_equal(key, "hit") && !g_str_equal(key, "frame") &&
            !g_str_equal(key, "maximize") && !g_str_equal(key, "required_modifiers") &&
            !g_str_equal(key, "forbidden_modifiers")) {
            g_list_free(members);
            return NULL;
        }
    }
    g_list_free(members);
    const char* id = json_object_get_string_member_with_default(object, "id", NULL);
    if (!id || !*id || strlen(id) > 64 || !g_utf8_validate(id, -1, NULL))
        return NULL;
    g_autoptr(GVariant) hit = native_socket_snap_rect(json_object_get_member(object, "hit"));
    g_autoptr(GVariant) frame = native_socket_snap_rect(json_object_get_member(object, "frame"));
    if (!hit || !frame)
        return NULL;
    GVariantBuilder target;
    g_variant_builder_init(&target, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&target, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&target, "{sv}", "hit", hit);
    g_variant_builder_add(&target, "{sv}", "frame", frame);
    const char* modifiers[] = {"required_modifiers", "forbidden_modifiers"};
    for (guint m = 0; m < G_N_ELEMENTS(modifiers); m++) {
        if (!json_object_has_member(object, modifiers[m]))
            continue;
        JsonNode* array_node = json_object_get_member(object, modifiers[m]);
        if (!JSON_NODE_HOLDS_ARRAY(array_node) ||
            json_array_get_length(json_node_get_array(array_node)) > 8) {
            g_variant_builder_clear(&target);
            return NULL;
        }
        GVariantBuilder array;
        g_variant_builder_init(&array, G_VARIANT_TYPE_STRING_ARRAY);
        JsonArray* source = json_node_get_array(array_node);
        for (guint i = 0; i < json_array_get_length(source); i++) {
            JsonNode* item = json_array_get_element(source, i);
            if (!JSON_NODE_HOLDS_VALUE(item) || json_node_get_value_type(item) != G_TYPE_STRING) {
                g_variant_builder_clear(&array);
                g_variant_builder_clear(&target);
                return NULL;
            }
            g_variant_builder_add(&array, "s", json_node_get_string(item));
        }
        g_variant_builder_add(&target, "{sv}", modifiers[m], g_variant_builder_end(&array));
    }
    if (json_object_has_member(object, "maximize")) {
        JsonNode* value = json_object_get_member(object, "maximize");
        if (!JSON_NODE_HOLDS_VALUE(value) || json_node_get_value_type(value) != G_TYPE_BOOLEAN) {
            g_variant_builder_clear(&target);
            return NULL;
        }
        g_variant_builder_add(&target, "{sv}", "maximize",
                              g_variant_new_boolean(json_node_get_boolean(value)));
    }
    return g_variant_ref_sink(g_variant_builder_end(&target));
}

char* native_socket_window_snap_offer(Client* client, const char* id, JsonObject* arguments) {
    if (!arguments || json_object_get_size(arguments) != 3 ||
        !json_object_has_member(arguments, "drag_id") ||
        !json_object_has_member(arguments, "drag_token") ||
        !json_object_has_member(arguments, "targets"))
        return encode_response(id, NULL,
                               "window.snap.offer requires drag_id, drag_token and targets");
    gint64 drag_id_value;
    JsonNode* drag_id_node = json_object_get_member(arguments, "drag_id");
    JsonNode* token_node = json_object_get_member(arguments, "drag_token");
    JsonNode* targets_node = json_object_get_member(arguments, "targets");
    if (!json_integer(drag_id_node, &drag_id_value) || drag_id_value <= 0 ||
        !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING ||
        !JSON_NODE_HOLDS_ARRAY(targets_node))
        return encode_response(id, NULL, "window.snap.offer arguments are invalid");
    const char* token = json_node_get_string(token_node);
    JsonArray* target_array = json_node_get_array(targets_node);
    if (!token || strlen(token) != 64 || json_array_get_length(target_array) == 0 ||
        json_array_get_length(target_array) > NATIVE_DRAG_MAX_TARGETS)
        return encode_response(id, NULL, "window.snap.offer token or target count is invalid");
    guint64 drag_id = drag_id_value;
    NativeWindowDrag* drag = g_hash_table_lookup(client->control->window_drags, &drag_id);
    const char* expected = drag && drag->client_tokens
                               ? g_hash_table_lookup(drag->client_tokens, &client->client_id)
                               : NULL;
    if (!drag || !expected || !g_str_equal(token, expected) ||
        drag->owner_generation != client->control->runtime_generation ||
        drag->expires_at_us <= g_get_monotonic_time() ||
        drag->settings_revision != native_config_revision(client->control) ||
        (client->control->wayland_compositor &&
         meta_wayland_session_lock_is_active(client->control->wayland_compositor)))
        return encode_response(id, NULL,
                               "window.snap.offer token is stale or not owned by this client");
    if (drag->runtime_offer_claimed && !drag->socket_owner_client_id)
        return encode_response(id, NULL, "window.snap.offer is owned by the Lua runtime");
    if (drag->socket_owner_client_id && drag->socket_owner_client_id != client->client_id)
        return encode_response(id, NULL, "window.snap.offer is owned by another shell client");
    GVariantBuilder targets;
    g_variant_builder_init(&targets, G_VARIANT_TYPE("av"));
    for (guint i = 0; i < json_array_get_length(target_array); i++) {
        g_autoptr(GVariant) target =
            native_socket_snap_target(json_array_get_element(target_array, i));
        if (!target) {
            g_variant_builder_clear(&targets);
            return encode_response(id, NULL, "window.snap.offer contains an invalid target");
        }
        g_variant_builder_add_value(&targets, g_variant_new_variant(target));
    }
    g_autoptr(GVariant) target_values = g_variant_ref_sink(g_variant_builder_end(&targets));
    g_autoptr(GError) error = NULL;
    if (!gnoblin_control_snap_offer_set_targets(drag, target_values, &error))
        return encode_response(id, NULL, error ? error->message : "snap offer was rejected");
    drag->socket_owner_client_id = client->client_id;
    JsonObject* result_object = json_object_new();
    json_object_set_boolean_member(result_object, "accepted", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(id, result, NULL);
}

void native_socket_consume_rejected_menu_context(Client* client, JsonObject* request) {
    if (!client || !client->control || !client->menu_grants || !request)
        return;
    const char* method = json_object_get_string_member_with_default(request, "method", "");
    if (!g_str_equal(method, "window.begin_move") && !g_str_equal(method, "window.begin_resize"))
        return;
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (!arguments_node || !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return;
    JsonNode* token_node =
        json_object_get_member(json_node_get_object(arguments_node), "menu_context");
    if (!token_node || !JSON_NODE_HOLDS_VALUE(token_node) ||
        json_node_get_value_type(token_node) != G_TYPE_STRING)
        return;
    const char* token = json_node_get_string(token_node);
    NativeMenuGrant* grant =
        focus_token_has_valid_shape(token) ? g_hash_table_lookup(client->menu_grants, token) : NULL;
    if (!grant)
        return;
    guint64 handle = grant->handle;
    g_hash_table_remove(client->menu_grants, token);
    if (client->control->menu_contexts)
        g_hash_table_remove(client->control->menu_contexts, &handle);
}

void native_ui_session_free(gpointer data) {
    NativeUiSession* session = data;
    if (!session)
        return;
    g_clear_pointer(&session->state, json_node_unref);
    g_clear_pointer(&session->surface, g_free);
    g_clear_pointer(&session->companions, g_strfreev);
    g_free(session);
}

static gboolean native_ui_session_name_is_valid(const char* name) {
    if (!name || !*name || strlen(name) > MAX_UI_SESSION_NAME_LENGTH)
        return FALSE;
    for (const char* cursor = name; *cursor; cursor++) {
        if (!g_ascii_isalnum(*cursor) && *cursor != '.' && *cursor != '_' && *cursor != '-')
            return FALSE;
    }
    return TRUE;
}

/* Parse ordering once per publish. Invalid metadata stays shell-owned data.
 * Disjoint namespace sets make declarations independent and prevent cycles. */
static void native_ui_session_parse_order(NativeUiSession* session) {
    g_clear_pointer(&session->surface, g_free);
    g_clear_pointer(&session->companions, g_strfreev);
    session->companions_above = FALSE;
    JsonObject* state = json_node_get_object(session->state);
    JsonNode* visible = json_object_get_member(state, "visible");
    JsonNode* surface = json_object_get_member(state, "surface");
    JsonNode* companions = json_object_get_member(state, "companions");
    JsonNode* above = json_object_get_member(state, "companionsAbove");
    if (!visible || !JSON_NODE_HOLDS_VALUE(visible) ||
        json_node_get_value_type(visible) != G_TYPE_BOOLEAN || !json_node_get_boolean(visible) ||
        !surface || !JSON_NODE_HOLDS_VALUE(surface) ||
        json_node_get_value_type(surface) != G_TYPE_STRING || !companions ||
        !JSON_NODE_HOLDS_ARRAY(companions) ||
        (above &&
         (!JSON_NODE_HOLDS_VALUE(above) || json_node_get_value_type(above) != G_TYPE_BOOLEAN)))
        return;
    const char* name = json_node_get_string(surface);
    JsonArray* members = json_node_get_array(companions);
    guint count = json_array_get_length(members);
    if (!name || !*name || strlen(name) > 128 || !count || count > 32)
        return;
    g_autoptr(GHashTable) seen = g_hash_table_new(g_str_hash, g_str_equal);
    g_autoptr(GPtrArray) parsed = g_ptr_array_new_with_free_func(g_free);
    for (guint i = 0; i < count; i++) {
        JsonNode* member = json_array_get_element(members, i);
        if (!JSON_NODE_HOLDS_VALUE(member) || json_node_get_value_type(member) != G_TYPE_STRING)
            return;
        const char* companion = json_node_get_string(member);
        if (!companion || !*companion || strlen(companion) > 128 || g_str_equal(companion, name))
            return;
        if (g_hash_table_add(seen, (gpointer)companion))
            g_ptr_array_add(parsed, g_strdup(companion));
    }
    g_ptr_array_add(parsed, NULL);
    session->surface = g_strdup(name);
    session->companions = (char**)g_ptr_array_steal(parsed, NULL);
    session->companions_above = above && json_node_get_boolean(above);
}

static void ui_namespace_count(GHashTable* counts, const char* name) {
    guint count = GPOINTER_TO_UINT(g_hash_table_lookup(counts, name));
    g_hash_table_insert(counts, (gpointer)name, GUINT_TO_POINTER(count + 1));
}

static const char* ui_layer_namespace(MetaWindow* window) {
    if (!window || window->unmanaging || !meta_window_is_in_stack(window) ||
        window->type != META_WINDOW_DOCK || window->transient_for ||
        !g_object_get_data(G_OBJECT(window), "gnoblin-layer-surface") ||
        !meta_wayland_surface_get_buffer(meta_window_get_wayland_surface(window)))
        return NULL;
    return g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace");
}

static gint ui_layer_compare_position(gconstpointer left, gconstpointer right) {
    const MetaWindow* a = left;
    const MetaWindow* b = right;
    return (a->stack_position > b->stack_position) - (a->stack_position < b->stack_position);
}

void gnoblin_native_control_constrain_layer_windows(MetaDisplay* display, GList* windows,
                                                    GnoblinLayerConstraintFunc add, gpointer data) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping || !control->ui_sessions ||
        (control->wayland_compositor &&
         meta_wayland_session_lock_is_active(control->wayland_compositor)))
        return;
    g_autoptr(GHashTable) counts = NULL;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, control->ui_sessions);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeUiSession* session = value;
        if (!session->surface)
            continue;
        if (!counts)
            counts = g_hash_table_new(g_str_hash, g_str_equal);
        ui_namespace_count(counts, session->surface);
        for (guint i = 0; session->companions[i]; i++)
            ui_namespace_count(counts, session->companions[i]);
    }
    if (!counts)
        return;
    g_hash_table_iter_init(&iter, control->ui_sessions);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        NativeUiSession* session = value;
        if (!session->surface)
            continue;
        gboolean conflict = GPOINTER_TO_UINT(g_hash_table_lookup(counts, session->surface)) > 1;
        g_autoptr(GHashTable) companions = g_hash_table_new(g_str_hash, g_str_equal);
        for (guint i = 0; session->companions[i]; i++) {
            conflict |= GPOINTER_TO_UINT(g_hash_table_lookup(counts, session->companions[i])) > 1;
            g_hash_table_add(companions, session->companions[i]);
        }
        if (conflict)
            continue;
        for (GList* base_item = windows; base_item; base_item = base_item->next) {
            MetaWindow* base = base_item->data;
            if (g_strcmp0(ui_layer_namespace(base), session->surface) != 0)
                continue;
            GList* selected = NULL;
            for (GList* item = windows; item; item = item->next) {
                MetaWindow* window = item->data;
                const char* name = ui_layer_namespace(window);
                if (window == base || !name || !g_hash_table_contains(companions, name) ||
                    window->layer != base->layer ||
                    meta_window_get_monitor(window) != meta_window_get_monitor(base))
                    continue;
                selected = g_list_prepend(selected, window);
            }
            /* A star of above/base constraints can reverse companions when
             * the base is raised. Preserve their authoritative current order
             * as a chain, including multiple surfaces with one namespace. */
            selected = g_list_sort(selected, ui_layer_compare_position);
            for (GList* item = selected; item; item = item->next) {
                MetaWindow* window = item->data;
                add(session->companions_above ? window : base,
                    session->companions_above ? base : window, data);
                if (item->next)
                    add(item->next->data, window, data);
            }
            g_list_free(selected);
        }
    }
}

static char* native_ui_session_encode_event(const char* event, const char* name, const char* member,
                                            JsonNode* payload) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "event", event);
    json_object_set_string_member(object, "name", name);
    json_object_set_member(object, member, json_node_copy(payload));
    g_autofree char* encoded = json_to_string(root, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static void native_ui_session_send_state(Client* client, const char* name, JsonNode* state) {
    if (!client || client->closing || !client->watch_ui_sessions)
        return;
    send_response(client, native_ui_session_encode_event("ui-state", name, "state", state));
}

static void native_ui_session_broadcast_state(GnoblinNativeControl* control, const char* name,
                                              JsonNode* state) {
    if (!control || control->stopping || !control->clients)
        return;
    GList* clients = g_hash_table_get_values(control->clients);
    for (GList* item = clients; item; item = item->next)
        native_ui_session_send_state(item->data, name, state);
    g_list_free(clients);
}

void native_ui_sessions_client_disconnected(GnoblinNativeControl* control, guint64 client_id) {
    if (!control || !control->ui_sessions || !client_id)
        return;
    GPtrArray* owned_names = g_ptr_array_new_with_free_func(g_free);
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, control->ui_sessions);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        NativeUiSession* session = value;
        if (session->owner_client_id == client_id)
            g_ptr_array_add(owned_names, g_strdup(key));
    }
    g_autoptr(JsonNode) cleared_state = json_node_new(JSON_NODE_NULL);
    gboolean order_changed = FALSE;
    for (guint i = 0; i < owned_names->len; i++) {
        const char* name = g_ptr_array_index(owned_names, i);
        NativeUiSession* session = g_hash_table_lookup(control->ui_sessions, name);
        order_changed |= session->surface != NULL;
        g_hash_table_remove(control->ui_sessions, name);
        native_ui_session_broadcast_state(control, name, cleared_state);
    }
    if (order_changed)
        meta_stack_update_transient(control->display->stack);
    g_ptr_array_unref(owned_names);
}

static gboolean native_ui_session_request_id(JsonObject* request, const char** id) {
    JsonNode* id_node = json_object_get_member(request, "id");
    if (!id_node) {
        *id = "";
        return TRUE;
    }
    if (!JSON_NODE_HOLDS_VALUE(id_node) || json_node_get_value_type(id_node) != G_TYPE_STRING)
        return FALSE;
    *id = json_node_get_string(id_node);
    return **id != '\0' && strlen(*id) <= 64;
}

static char* native_ui_session_reply(const char* id) {
    JsonObject* result_object = json_object_new();
    json_object_set_boolean_member(result_object, "accepted", TRUE);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, result_object);
    return encode_response(id, result, NULL);
}

char* handle_ui_session_request(Client* client, JsonObject* request) {
    const char* id = "";
    if (!client || client->closing || !client->control || !client->control->ui_sessions)
        return encode_response("", NULL, "UI session transport is unavailable");
    JsonNode* op_node = json_object_get_member(request, "op");
    JsonNode* action_node = json_object_get_member(request, "action");
    if (!op_node || !JSON_NODE_HOLDS_VALUE(op_node) ||
        json_node_get_value_type(op_node) != G_TYPE_STRING ||
        !g_str_equal(json_node_get_string(op_node), "ui-session") || !action_node ||
        !JSON_NODE_HOLDS_VALUE(action_node) ||
        json_node_get_value_type(action_node) != G_TYPE_STRING ||
        !native_ui_session_request_id(request, &id))
        return encode_response("", NULL, "invalid UI session request");

    const char* action = json_node_get_string(action_node);
    gboolean has_id = json_object_has_member(request, "id");
    if (g_str_equal(action, "watch")) {
        if (json_object_get_size(request) != (has_id ? 3 : 2))
            return encode_response(id, NULL,
                                   "ui-session watch accepts only op, action, and optional id");
        gboolean first_watch = !client->watch_ui_sessions;
        client->watch_ui_sessions = TRUE;
        if (first_watch) {
            GPtrArray* snapshots = g_ptr_array_new_with_free_func(g_free);
            GHashTableIter iter;
            gpointer key, value;
            g_hash_table_iter_init(&iter, client->control->ui_sessions);
            while (g_hash_table_iter_next(&iter, &key, &value)) {
                NativeUiSession* session = value;
                g_ptr_array_add(snapshots, native_ui_session_encode_event("ui-state", key, "state",
                                                                          session->state));
            }
            for (guint i = 0; i < snapshots->len && !client->closing; i++)
                send_response(client, g_strdup(g_ptr_array_index(snapshots, i)));
            g_ptr_array_unref(snapshots);
        }
        return native_ui_session_reply(id);
    }

    gboolean is_state = g_str_equal(action, "state");
    gboolean is_command = g_str_equal(action, "command");
    if (!is_state && !is_command)
        return encode_response(id, NULL, "ui-session action must be watch, state, or command");
    const char* payload_member = is_state ? "state" : "command";
    if (json_object_get_size(request) != (has_id ? 5 : 4) ||
        !json_object_has_member(request, "name") ||
        !json_object_has_member(request, payload_member))
        return encode_response(id, NULL,
                               "ui-session state and command accept only op, action, name, their "
                               "payload, and optional id");

    JsonNode* name_node = json_object_get_member(request, "name");
    JsonNode* payload = json_object_get_member(request, payload_member);
    if (!name_node || !JSON_NODE_HOLDS_VALUE(name_node) ||
        json_node_get_value_type(name_node) != G_TYPE_STRING ||
        !native_ui_session_name_is_valid(json_node_get_string(name_node)))
        return encode_response(
            id, NULL, "ui-session name must be 1-128 ASCII letters, digits, '.', '_' or '-'");
    if (!payload || !JSON_NODE_HOLDS_OBJECT(payload))
        return encode_response(id, NULL, "ui-session state and command payloads must be objects");
    g_autofree char* encoded_payload = json_to_string(payload, FALSE);
    if (!encoded_payload || strlen(encoded_payload) > MAX_UI_SESSION_PAYLOAD_BYTES)
        return encode_response(id, NULL, "ui-session payload exceeds the 8 KiB limit");
    if (!client->watch_ui_sessions)
        return encode_response(id, NULL, "send ui-session watch before state or command messages");

    const char* name = json_node_get_string(name_node);
    if (is_state) {
        NativeUiSession* session = g_hash_table_lookup(client->control->ui_sessions, name);
        if (session && session->owner_client_id != client->client_id)
            return encode_response(id, NULL,
                                   "ui-session state name is already owned by another connection");
        if (!session && g_hash_table_size(client->control->ui_sessions) >= MAX_UI_SESSIONS)
            return encode_response(id, NULL, "UI session limit reached (64 active names)");
        if (!session) {
            session = g_new0(NativeUiSession, 1);
            session->owner_client_id = client->client_id;
            g_hash_table_insert(client->control->ui_sessions, g_strdup(name), session);
        }
        g_autofree char* previous_surface = g_strdup(session->surface);
        g_auto(GStrv) previous_companions = g_strdupv(session->companions);
        gboolean previous_above = session->companions_above;
        g_clear_pointer(&session->state, json_node_unref);
        session->state = json_node_copy(payload);
        native_ui_session_parse_order(session);
        if (g_strcmp0(previous_surface, session->surface) != 0 ||
            (previous_companions == NULL) != (session->companions == NULL) ||
            (previous_companions && session->companions &&
             !g_strv_equal((const char* const*)previous_companions,
                           (const char* const*)session->companions)) ||
            previous_above != session->companions_above)
            meta_stack_update_transient(client->control->display->stack);
        native_ui_session_broadcast_state(client->control, name, session->state);
        return native_ui_session_reply(id);
    }

    GList* clients = g_hash_table_get_values(client->control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* recipient = item->data;
        if (recipient->watch_ui_sessions && !recipient->closing) {
            g_autofree char* event =
                native_ui_session_encode_event("ui-command", name, "command", payload);
            send_response(recipient, g_strdup(event));
        }
    }
    g_list_free(clients);
    return native_ui_session_reply(id);
}
