#include "gnoblinctl-internal.h"

static gboolean lua_cli_layer_animation_policy_valid(JsonObject* object,
                                                     const char* requested_namespace) {
    if (!g_str_equal(member_string(object, "namespace", ""), requested_namespace))
        return FALSE;
    JsonNode* window_shadow = json_object_get_member(object, "window_shadow");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!window_shadow || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;

    static const char* const phase_names[] = {"enter", "exit"};
    for (guint i = 0; i < G_N_ELEMENTS(phase_names); i++) {
        JsonObject* phase = member_object(object, phase_names[i]);
        if (!phase || !*member_string(phase, "animation", ""))
            return FALSE;
        JsonNode* duration = json_object_get_member(phase, "duration");
        if (duration && (!JSON_NODE_HOLDS_VALUE(duration) ||
                         (json_node_get_value_type(duration) != G_TYPE_INT64 &&
                          json_node_get_value_type(duration) != G_TYPE_INT) ||
                         json_node_get_int(duration) < 0))
            return FALSE;
        JsonNode* easing = json_object_get_member(phase, "easing");
        if (easing) {
            static const char* const curves[] = {"linear",
                                                 "ease-in-quad",
                                                 "ease-out-quad",
                                                 "ease-in-out-cubic",
                                                 "ease-in-cubic",
                                                 "ease-out-cubic",
                                                 "ease-out-expo",
                                                 "ease-out-back",
                                                 NULL};
            if (JSON_NODE_HOLDS_VALUE(easing) &&
                json_node_get_value_type(easing) == G_TYPE_STRING) {
                gboolean known_curve = FALSE;
                const char* curve = json_node_get_string(easing);
                for (guint j = 0; curves[j]; j++)
                    known_curve |= g_str_equal(curve, curves[j]);
                if (!known_curve)
                    return FALSE;
            } else if (JSON_NODE_HOLDS_OBJECT(easing)) {
                JsonObject* bezier = json_node_get_object(easing);
                if (!g_str_equal(member_string(bezier, "type", ""), "cubic-bezier"))
                    return FALSE;
                const char* coordinates[] = {"x1", "y1", "x2", "y2"};
                double values[G_N_ELEMENTS(coordinates)];
                for (guint j = 0; j < G_N_ELEMENTS(coordinates); j++) {
                    JsonNode* coordinate = json_object_get_member(bezier, coordinates[j]);
                    if (!coordinate || !JSON_NODE_HOLDS_VALUE(coordinate))
                        return FALSE;
                    GType type = json_node_get_value_type(coordinate);
                    if (type != G_TYPE_DOUBLE && type != G_TYPE_INT && type != G_TYPE_INT64)
                        return FALSE;
                    values[j] = json_node_get_double(coordinate);
                    if (!isfinite(values[j]))
                        return FALSE;
                }
                if (values[0] < 0 || values[0] > 1 || values[2] < 0 || values[2] > 1)
                    return FALSE;
            } else {
                return FALSE;
            }
        }
    }
    return TRUE;
}

static int lua_cli_layer_animation_policy_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "namespace");
    const char* namespace = lua_tostring(state, -1);
    lua_pushfstring(state, "LayerAnimationPolicy<%s>", namespace ? namespace : "unknown");
    return 1;
}

void register_lua_cli_layer_animation_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_layer_animation_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_layer_animation_policy(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.layers.animation_policy requires one namespace string");
    const char* namespace = lua_tostring(state, 1);
    gsize length = strlen(namespace);
    if (length == 0 || length > 128 || !g_utf8_validate(namespace, length, NULL))
        return luaL_error(state, "layer namespace must be 1 to 128 bytes of UTF-8");

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "namespace", namespace);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "layer.animation_policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.layers.animation_policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_layer_animation_policy_valid(json_node_get_object(result), namespace))
        return luaL_error(state, "gnoblin.layers.animation_policy returned an invalid snapshot");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_LAYER_ANIMATION_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_privacy_state_valid(JsonObject* object) {
    JsonObject* available = member_object(object, "available");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!available || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;

    GList* available_fields = json_object_get_members(available);
    for (GList* item = available_fields; item; item = item->next) {
        const char* key = item->data;
        JsonNode* value = json_object_get_member(available, key);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_BOOLEAN) {
            g_list_free(available_fields);
            return FALSE;
        }
    }
    g_list_free(available_fields);

    GList* fields = json_object_get_members(object);
    for (GList* item = fields; item; item = item->next) {
        const char* key = item->data;
        if (g_str_equal(key, "available") || g_str_equal(key, "revision"))
            continue;
        JsonNode* value = json_object_get_member(object, key);
        JsonNode* source_available = json_object_get_member(available, key);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_BOOLEAN || !source_available ||
            !json_node_get_boolean(source_available)) {
            g_list_free(fields);
            return FALSE;
        }
    }
    g_list_free(fields);

    available_fields = json_object_get_members(available);
    for (GList* item = available_fields; item; item = item->next) {
        const char* key = item->data;
        JsonNode* source_available = json_object_get_member(available, key);
        if (json_node_get_boolean(source_available) && !json_object_has_member(object, key)) {
            g_list_free(available_fields);
            return FALSE;
        }
    }
    g_list_free(available_fields);
    return TRUE;
}

static int lua_cli_privacy_state_tostring(lua_State* state) {
    lua_pushliteral(state, "PrivacyState");
    return 1;
}

void register_lua_cli_privacy_state_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_privacy_state_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_privacy_state(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.privacy.state takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "privacy.state", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.privacy.state failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_privacy_state_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.privacy.state returned an invalid PrivacyState");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PRIVACY_STATE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

int lua_cli_privacy_stop(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", lua_tostring(state, lua_upvalueindex(2)));
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* requested = object ? json_object_get_member(object, "requested") : NULL;
    if (!requested || !JSON_NODE_HOLDS_VALUE(requested) ||
        (json_node_get_value_type(requested) != G_TYPE_INT &&
         json_node_get_value_type(requested) != G_TYPE_INT64) ||
        json_node_get_int(requested) < 0)
        return luaL_error(state, "%s returned an invalid result", method);
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static gboolean lua_cli_location_accuracy_valid(gint64 accuracy) {
    return accuracy == 0 || accuracy == 1 || accuracy == 4 || accuracy == 5 || accuracy == 6 ||
           accuracy == 8;
}

static void cli_lua_location_requests_prune(CliLuaEventSubscription* subscription, gint64 now) {
    if (!subscription || !subscription->location_requests)
        return;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, subscription->location_requests);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        CliLuaLocationRequest* request = value;
        if (!request || request->expires_at_us <= now)
            g_hash_table_iter_remove(&iter);
    }
}

void cli_lua_location_request_track(CliLuaEventSubscription* subscription, JsonObject* event) {
    if (!subscription || !subscription->active ||
        !g_str_equal(subscription->event_name, "gnoblin.location.authorization-requested"))
        return;

    JsonNode* request_id_node = json_object_get_member(event, "request_id");
    JsonNode* expires_node = json_object_get_member(event, "expires_at_us");
    JsonNode* accuracy_node = json_object_get_member(event, "requested_accuracy");
    if (!request_id_node || !expires_node || !accuracy_node ||
        !JSON_NODE_HOLDS_VALUE(request_id_node) || !JSON_NODE_HOLDS_VALUE(expires_node) ||
        !JSON_NODE_HOLDS_VALUE(accuracy_node))
        return;
    GType request_id_type = json_node_get_value_type(request_id_node);
    GType expires_type = json_node_get_value_type(expires_node);
    GType accuracy_type = json_node_get_value_type(accuracy_node);
    if ((request_id_type != G_TYPE_INT && request_id_type != G_TYPE_INT64) ||
        (expires_type != G_TYPE_INT && expires_type != G_TYPE_INT64) ||
        (accuracy_type != G_TYPE_INT && accuracy_type != G_TYPE_INT64))
        return;

    gint64 request_id = json_node_get_int(request_id_node);
    gint64 expires_at_us = json_node_get_int(expires_node);
    gint64 accuracy = json_node_get_int(accuracy_node);
    gint64 now = g_get_monotonic_time();
    if (request_id <= 0 || expires_at_us <= now || !lua_cli_location_accuracy_valid(accuracy))
        return;

    cli_lua_location_requests_prune(subscription, now);
    if (g_hash_table_size(subscription->location_requests) >= 256) {
        GHashTableIter iter;
        gpointer key;
        gpointer value;
        gint64 earliest_id = 0;
        gint64 earliest_expiry = G_MAXINT64;
        g_hash_table_iter_init(&iter, subscription->location_requests);
        while (g_hash_table_iter_next(&iter, &key, &value)) {
            CliLuaLocationRequest* pending = value;
            if (pending && pending->expires_at_us < earliest_expiry) {
                earliest_id = *(gint64*)key;
                earliest_expiry = pending->expires_at_us;
            }
        }
        if (earliest_id)
            g_hash_table_remove(subscription->location_requests, &earliest_id);
    }

    gint64* key = g_new(gint64, 1);
    *key = request_id;
    CliLuaLocationRequest* request = g_new(CliLuaLocationRequest, 1);
    request->expires_at_us = expires_at_us;
    request->requested_accuracy = (guint)accuracy;
    g_hash_table_replace(subscription->location_requests, key, request);
}

static CliLuaEventSubscription* cli_lua_location_request_lookup(Cli* cli, gint64 request_id,
                                                                guint* requested_accuracy) {
    if (!cli || !cli->lua_event_subscriptions || request_id <= 0)
        return NULL;
    gint64 now = g_get_monotonic_time();
    for (guint i = 0; i < cli->lua_event_subscriptions->len; i++) {
        CliLuaEventSubscription* subscription = g_ptr_array_index(cli->lua_event_subscriptions, i);
        if (!subscription->active ||
            !g_str_equal(subscription->event_name, "gnoblin.location.authorization-requested"))
            continue;
        cli_lua_location_requests_prune(subscription, now);
        CliLuaLocationRequest* request =
            g_hash_table_lookup(subscription->location_requests, &request_id);
        if (request) {
            if (requested_accuracy)
                *requested_accuracy = request->requested_accuracy;
            return subscription;
        }
    }
    return NULL;
}

static void cli_lua_location_request_consume(Cli* cli, gint64 request_id) {
    if (!cli || !cli->lua_event_subscriptions)
        return;
    for (guint i = 0; i < cli->lua_event_subscriptions->len; i++) {
        CliLuaEventSubscription* subscription = g_ptr_array_index(cli->lua_event_subscriptions, i);
        if (subscription->location_requests)
            g_hash_table_remove(subscription->location_requests, &request_id);
    }
}

int lua_cli_location_authorize_app(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.location.authorize_app requires one options table");
    g_autoptr(JsonObject) arguments =
        lua_cli_table_object(state, 1, "location authorization options");
    if (json_object_get_size(arguments) != 3 || !json_object_has_member(arguments, "request_id") ||
        !json_object_has_member(arguments, "allow") ||
        !json_object_has_member(arguments, "accuracy"))
        return luaL_error(state,
                          "gnoblin.location.authorize_app accepts request_id, allow, and accuracy");

    JsonNode* request_id_node = json_object_get_member(arguments, "request_id");
    JsonNode* allow_node = json_object_get_member(arguments, "allow");
    JsonNode* accuracy_node = json_object_get_member(arguments, "accuracy");
    if (!request_id_node || !allow_node || !accuracy_node ||
        !JSON_NODE_HOLDS_VALUE(request_id_node) || !JSON_NODE_HOLDS_VALUE(allow_node) ||
        !JSON_NODE_HOLDS_VALUE(accuracy_node))
        return luaL_error(state, "location authorization fields have invalid types");
    GType request_id_type = json_node_get_value_type(request_id_node);
    GType allow_type = json_node_get_value_type(allow_node);
    GType accuracy_type = json_node_get_value_type(accuracy_node);
    if ((request_id_type != G_TYPE_INT && request_id_type != G_TYPE_INT64) ||
        allow_type != G_TYPE_BOOLEAN ||
        (accuracy_type != G_TYPE_INT && accuracy_type != G_TYPE_INT64))
        return luaL_error(state, "location authorization fields have invalid types");

    gint64 request_id = json_node_get_int(request_id_node);
    gboolean allowed = json_node_get_boolean(allow_node);
    gint64 accuracy = json_node_get_int(accuracy_node);
    if (request_id <= 0 || !lua_cli_location_accuracy_valid(accuracy) ||
        (allowed && accuracy == 0) || (!allowed && accuracy != 0))
        return luaL_error(state,
                          "location authorization needs a positive request_id, boolean allow, "
                          "and accuracy 0, 1, 4, 5, 6, or 8 (0 when denied)");

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    guint requested_accuracy = 0;
    CliLuaEventSubscription* subscription =
        cli_lua_location_request_lookup(cli, request_id, &requested_accuracy);
    if (!subscription)
        return luaL_error(state, "location request_id was not received by an active "
                                 "gnoblin.location.authorization-requested subscription");
    if (allowed && (guint)accuracy > requested_accuracy)
        return luaL_error(state,
                          "location accuracy must not exceed the level requested by the app");

    cli_lua_location_request_consume(cli, request_id);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        subscription, "location.authorize_app", arguments, &call_error);
    if (!result)
        return luaL_error(state, "gnoblin.location.authorize_app failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* response_id = response ? json_object_get_member(response, "request_id") : NULL;
    JsonNode* submitted = response ? json_object_get_member(response, "submitted") : NULL;
    if (!response_id || !submitted || !JSON_NODE_HOLDS_VALUE(response_id) ||
        !JSON_NODE_HOLDS_VALUE(submitted) ||
        (json_node_get_value_type(response_id) != G_TYPE_INT &&
         json_node_get_value_type(response_id) != G_TYPE_INT64) ||
        json_node_get_int(response_id) != request_id ||
        json_node_get_value_type(submitted) != G_TYPE_BOOLEAN || !json_node_get_boolean(submitted))
        return luaL_error(state, "gnoblin.location.authorize_app returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_runtime_reload_config(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.runtime.reload_config takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "runtime.reload_config", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.runtime.reload_config failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* ok = object ? json_object_get_member(object, "ok") : NULL;
    JsonNode* action = object ? json_object_get_member(object, "action") : NULL;
    if (!ok || !JSON_NODE_HOLDS_VALUE(ok) || json_node_get_value_type(ok) != G_TYPE_BOOLEAN ||
        !action || !JSON_NODE_HOLDS_VALUE(action) ||
        json_node_get_value_type(action) != G_TYPE_STRING)
        return luaL_error(state, "gnoblin.runtime.reload_config returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_runtime_status(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.runtime.status takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "runtime.status", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.runtime.status failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* state_node = object ? json_object_get_member(object, "state") : NULL;
    JsonNode* generation_node = object ? json_object_get_member(object, "generation") : NULL;
    const char* runtime_state = state_node && JSON_NODE_HOLDS_VALUE(state_node) &&
                                        json_node_get_value_type(state_node) == G_TYPE_STRING
                                    ? json_node_get_string(state_node)
                                    : NULL;
    gboolean known_state =
        runtime_state &&
        (g_str_equal(runtime_state, "starting") || g_str_equal(runtime_state, "running") ||
         g_str_equal(runtime_state, "restarting") || g_str_equal(runtime_state, "unavailable"));
    gboolean generation_valid = generation_node && JSON_NODE_HOLDS_VALUE(generation_node) &&
                                (json_node_get_value_type(generation_node) == G_TYPE_INT64 ||
                                 json_node_get_value_type(generation_node) == G_TYPE_INT) &&
                                json_node_get_int(generation_node) >= 0;
    if (!known_state || !generation_valid)
        return luaL_error(state, "gnoblin.runtime.status returned an invalid RuntimeStatus");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_session_operation(lua_State* state) {
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.%s takes no arguments", method);
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.%s failed: %s", method, call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* status = object ? json_object_get_member(object, "accepted") : NULL;
    if (g_str_equal(method, "session.lock")) {
        status = object ? json_object_get_member(object, "dispatched") : NULL;
        JsonNode* subscribers = object ? json_object_get_member(object, "subscribers") : NULL;
        if (!subscribers || !JSON_NODE_HOLDS_VALUE(subscribers) ||
            (json_node_get_value_type(subscribers) != G_TYPE_INT &&
             json_node_get_value_type(subscribers) != G_TYPE_INT64) ||
            json_node_get_int(subscribers) < 0)
            return luaL_error(state, "gnoblin.%s returned an invalid result", method);
    }
    if (!status || !JSON_NODE_HOLDS_VALUE(status) ||
        json_node_get_value_type(status) != G_TYPE_BOOLEAN || !json_node_get_boolean(status))
        return luaL_error(state, "gnoblin.%s returned an invalid result", method);
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_config_path(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.config.path takes no arguments");
    g_autofree char* path = config_path();
    lua_pushstring(state, path);
    return 1;
}

int lua_cli_config_default(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.config.default takes no arguments");
    g_autofree char* source = default_config(NULL);
    lua_pushstring(state, source);
    return 1;
}

static gboolean lua_cli_capability_valid(JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    const char* description = member_string(object, "description", NULL);
    JsonNode* available = json_object_get_member(object, "available");
    JsonNode* revision = json_object_get_member(object, "revision");
    JsonNode* reason = json_object_get_member(object, "reason");
    return id && *id && description && *description && available &&
           JSON_NODE_HOLDS_VALUE(available) &&
           json_node_get_value_type(available) == G_TYPE_BOOLEAN && revision &&
           JSON_NODE_HOLDS_VALUE(revision) &&
           (json_node_get_value_type(revision) == G_TYPE_INT64 ||
            json_node_get_value_type(revision) == G_TYPE_INT) &&
           json_node_get_int(revision) >= 0 &&
           (!reason ||
            (JSON_NODE_HOLDS_VALUE(reason) && json_node_get_value_type(reason) == G_TYPE_STRING &&
             *json_node_get_string(reason)));
}

static int lua_cli_capability_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Capability<%s>", id ? id : "unknown");
    return 1;
}

void register_lua_cli_capability_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_CAPABILITY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_capability_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static void lua_cli_push_capability_record(lua_State* state, JsonNode* node) {
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_CAPABILITY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

int lua_cli_capabilities_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.capabilities.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "capabilities.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.capabilities.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.capabilities.list returned an invalid snapshot");

    JsonArray* capabilities = json_node_get_array(result);
    guint length = json_array_get_length(capabilities);
    lua_createtable(state, length, 0);
    for (guint i = 0; i < length; i++) {
        JsonNode* node = json_array_get_element(capabilities, i);
        if (!JSON_NODE_HOLDS_OBJECT(node) || !lua_cli_capability_valid(json_node_get_object(node)))
            return luaL_error(state,
                              "gnoblin.capabilities.list returned an invalid Capability record");
        lua_cli_push_capability_record(state, node);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static gboolean lua_cli_permission_policy_valid(JsonObject* object) {
    const char* default_level = member_string(object, "default", NULL);
    JsonArray* rules = json_object_get_array_member(object, "rules");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!default_level ||
        (!g_str_equal(default_level, "inherit") && !g_str_equal(default_level, "default") &&
         !g_str_equal(default_level, "ask") && !g_str_equal(default_level, "deny")) ||
        !rules || !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(rules); i++)
        if (!JSON_NODE_HOLDS_OBJECT(json_array_get_element(rules, i)))
            return FALSE;
    return TRUE;
}

static int lua_cli_permission_policy_tostring(lua_State* state) {
    lua_pushliteral(state, "PermissionPolicy");
    return 1;
}

void register_lua_cli_permission_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_permission_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static gboolean lua_cli_permission_string_array_valid(JsonArray* array) {
    if (!array)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(array); i++) {
        JsonNode* value = json_array_get_element(array, i);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            json_node_get_value_type(value) != G_TYPE_STRING || !json_node_get_string(value)[0])
            return FALSE;
    }
    return TRUE;
}

int lua_cli_permissions_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.list failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonObject* policy = member_object(object, "policy");
    const char* default_level = member_string(policy, "default", NULL);
    JsonNode* path = object ? json_object_get_member(object, "path") : NULL;
    if (!policy || !default_level ||
        (!g_str_equal(default_level, "inherit") && !g_str_equal(default_level, "default") &&
         !g_str_equal(default_level, "ask") && !g_str_equal(default_level, "deny")) ||
        !json_object_get_array_member(policy, "rules") ||
        !lua_cli_permission_string_array_valid(
            object ? json_object_get_array_member(object, "capabilities") : NULL) ||
        !lua_cli_permission_string_array_valid(
            object ? json_object_get_array_member(object, "levels") : NULL) ||
        !path || !JSON_NODE_HOLDS_VALUE(path) || json_node_get_value_type(path) != G_TYPE_STRING)
        return luaL_error(state, "gnoblin.permissions.list returned an invalid snapshot");

    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_version(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.version takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "version", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.version failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    static const char* const fields[] = {"gnoblin",    "gnome",   "mutter",   "lua", "api",
                                         "git_remote", "git_sha", "build_id", NULL};
    for (guint i = 0; fields[i]; i++)
        if (!member_string(object, fields[i], NULL))
            return luaL_error(state, "gnoblin.version returned an invalid Version record");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_permissions_policy(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.policy takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_permission_policy_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.permissions.policy returned an invalid PermissionPolicy");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PERMISSION_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_permission_decision_valid(JsonObject* object) {
    const char* level = member_string(object, "level", NULL);
    JsonNode* rule = json_object_get_member(object, "rule");
    JsonArray* monitors = json_object_get_array_member(object, "monitors");
    JsonArray* devices = json_object_get_array_member(object, "devices");
    JsonNode* clipboard = json_object_get_member(object, "clipboard");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!level ||
        (!g_str_equal(level, "inherit") && !g_str_equal(level, "default") &&
         !g_str_equal(level, "ask") && !g_str_equal(level, "allow") &&
         !g_str_equal(level, "deny")) ||
        !rule || !JSON_NODE_HOLDS_VALUE(rule) || json_node_get_value_type(rule) != G_TYPE_STRING ||
        !monitors || !devices || !clipboard || !JSON_NODE_HOLDS_VALUE(clipboard) ||
        json_node_get_value_type(clipboard) != G_TYPE_BOOLEAN || !revision ||
        !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return FALSE;
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonNode* monitor = json_array_get_element(monitors, i);
        if (!monitor || !JSON_NODE_HOLDS_VALUE(monitor) ||
            json_node_get_value_type(monitor) != G_TYPE_STRING)
            return FALSE;
    }
    for (guint i = 0; i < json_array_get_length(devices); i++) {
        JsonNode* device = json_array_get_element(devices, i);
        const char* name = device && JSON_NODE_HOLDS_VALUE(device) &&
                                   json_node_get_value_type(device) == G_TYPE_STRING
                               ? json_node_get_string(device)
                               : NULL;
        if (!name || (!g_str_equal(name, "keyboard") && !g_str_equal(name, "pointer") &&
                      !g_str_equal(name, "touchscreen")))
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_permission_decision_tostring(lua_State* state) {
    lua_pushliteral(state, "PermissionDecision");
    return 1;
}

void register_lua_cli_permission_decision_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_permission_decision_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_permissions_check(lua_State* state) {
    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 2 && lua_type(state, 1) == LUA_TSTRING &&
        lua_type(state, 2) == LUA_TSTRING) {
        json_object_set_string_member(arguments, "capability", lua_tostring(state, 1));
        json_object_set_string_member(arguments, "identity", lua_tostring(state, 2));
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            if (!key || (!g_str_equal(key, "capability") && !g_str_equal(key, "identity")) ||
                lua_type(state, -1) != LUA_TSTRING) {
                json_object_unref(arguments);
                return luaL_error(
                    state, "gnoblin.permissions.check accepts string capability and identity");
            }
            lua_pop(state, 1);
        }
        lua_getfield(state, 1, "capability");
        const char* capability =
            lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_getfield(state, 1, "identity");
        const char* identity = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        if (!capability || !identity) {
            lua_pop(state, 2);
            json_object_unref(arguments);
            return luaL_error(state,
                              "gnoblin.permissions.check requires string capability and identity");
        }
        json_object_set_string_member(arguments, "capability", capability);
        json_object_set_string_member(arguments, "identity", identity);
        lua_pop(state, 2);
    } else {
        json_object_unref(arguments);
        return luaL_error(state,
                          "gnoblin.permissions.check accepts (capability, identity) or one table");
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "permissions.check", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.permissions.check failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_permission_decision_valid(json_node_get_object(result)))
        return luaL_error(state,
                          "gnoblin.permissions.check returned an invalid PermissionDecision");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_PERMISSION_DECISION_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}
