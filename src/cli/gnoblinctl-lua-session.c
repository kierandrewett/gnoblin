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

/* A Request: what a shell answers. The console makes one for each location authorization
 * request that an event subscription receives, as event.request. The request belongs to the
 * connection that received it; the compositor accepts an answer only from that connection. Only
 * the location kind exists here: the console answers no prompt or authentication request. */
static LuaCliRequest* lua_cli_test_request(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_REQUEST_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliRequest))
        return NULL;
    LuaCliRequest* request = lua_touserdata(state, index);
    return request->magic == LUA_CLI_REQUEST_MAGIC ? request : NULL;
}

gboolean lua_cli_push_request(lua_State* state, CliLuaEventSubscription* subscription,
                              JsonObject* event) {
    if (!subscription || !subscription->active ||
        !g_str_equal(subscription->event_name, "gnoblin.location.authorization-requested"))
        return FALSE;
    JsonNode* request_id_node = json_object_get_member(event, "request_id");
    JsonNode* expires_node = json_object_get_member(event, "expires_at_us");
    JsonNode* accuracy_node = json_object_get_member(event, "requested_accuracy");
    JsonNode* nodes[] = {request_id_node, expires_node, accuracy_node};
    for (guint i = 0; i < G_N_ELEMENTS(nodes); i++)
        if (!nodes[i] || !JSON_NODE_HOLDS_VALUE(nodes[i]) ||
            (json_node_get_value_type(nodes[i]) != G_TYPE_INT &&
             json_node_get_value_type(nodes[i]) != G_TYPE_INT64))
            return FALSE;
    gint64 request_id = json_node_get_int(request_id_node);
    gint64 accuracy = json_node_get_int(accuracy_node);
    if (request_id <= 0 || !lua_cli_location_accuracy_valid(accuracy))
        return FALSE;

    LuaCliRequest* request = lua_newuserdatauv(state, sizeof *request, 1);
    request->magic = LUA_CLI_REQUEST_MAGIC;
    request->subscription = subscription;
    request->request_id = request_id;
    request->expires_at_us = json_node_get_int(expires_node);
    request->requested_accuracy = (guint)accuracy;
    request->answered = FALSE;

    /* The fields a script reads: the event's own, with the ones every Request has. */
    g_autoptr(JsonObject) fields = json_object_new();
    g_autoptr(GList) members = json_object_get_members(event);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "request_id") && !g_str_equal(key, "event") &&
            !g_str_equal(key, "name") && !g_str_equal(key, "sequence") &&
            !g_str_equal(key, "time") && !g_str_equal(key, "kind"))
            json_object_set_member(fields, key, json_node_copy(json_object_get_member(event, key)));
    }
    json_object_set_int_member(fields, "id", request_id);
    json_object_set_string_member(fields, "kind", "location");
    json_object_set_string_member(fields, "status", "pending");
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, fields);
    json_to_lua(state, node);
    lua_setiuservalue(state, -2, 1);
    luaL_getmetatable(state, GNOBLINCTL_REQUEST_METATABLE);
    lua_setmetatable(state, -2);
    return TRUE;
}

/* Answer the location request: `allow`, and the accuracy level when allowed. */
static int lua_cli_request_answer_location(lua_State* state, LuaCliRequest* request, gboolean allow,
                                           gboolean accuracy_given, gint64 accuracy) {
    if (!accuracy_given)
        accuracy = allow ? request->requested_accuracy : 0;
    if (!lua_cli_location_accuracy_valid(accuracy) || (allow && accuracy == 0) ||
        (!allow && accuracy != 0))
        return luaL_error(state, "location authorization needs allow and an accuracy of 1, 4, 5, "
                                 "6 or 8 (0 when denied)");
    if (allow && (guint)accuracy > request->requested_accuracy)
        return luaL_error(state,
                          "location accuracy must not exceed the level requested by the app");
    if (request->answered)
        return luaL_error(state, "this Request was already answered");
    if (request->expires_at_us <= g_get_monotonic_time())
        return luaL_error(state, "this location request has expired");
    if (!request->subscription || !request->subscription->active ||
        !request->subscription->connection)
        return luaL_error(state, "this Request belongs to an event subscription that has ended");

    g_autoptr(JsonObject) arguments = json_object_new();
    json_object_set_int_member(arguments, "request_id", request->request_id);
    json_object_set_boolean_member(arguments, "allow", allow);
    json_object_set_int_member(arguments, "accuracy", accuracy);
    request->answered = TRUE;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        request->subscription, "location.authorize_app", arguments, &call_error);
    if (!result)
        return luaL_error(state, "location.authorize_app failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* response_id = response ? json_object_get_member(response, "request_id") : NULL;
    JsonNode* submitted = response ? json_object_get_member(response, "submitted") : NULL;
    if (!response_id || !submitted || !JSON_NODE_HOLDS_VALUE(response_id) ||
        !JSON_NODE_HOLDS_VALUE(submitted) ||
        (json_node_get_value_type(response_id) != G_TYPE_INT &&
         json_node_get_value_type(response_id) != G_TYPE_INT64) ||
        json_node_get_int(response_id) != request->request_id ||
        json_node_get_value_type(submitted) != G_TYPE_BOOLEAN || !json_node_get_boolean(submitted))
        return luaL_error(state, "location.authorize_app returned an invalid result");
    lua_getiuservalue(state, 1, 1);
    lua_pushliteral(state, "answered");
    lua_setfield(state, -2, "status");
    lua_pop(state, 1);
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

/* Request:answer(allow, accuracy?) */
static int lua_cli_request_answer(lua_State* state) {
    LuaCliRequest* request = lua_cli_test_request(state, 1);
    if (!request)
        return luaL_error(state, "Request:answer requires a Gnoblin Request");
    int arguments = lua_gettop(state) - 1;
    if (arguments < 1 || arguments > 2 || !lua_isboolean(state, 2) ||
        (arguments == 2 && !lua_isinteger(state, 3)))
        return luaL_error(state, "Request:answer takes an allow boolean and an optional accuracy "
                                 "integer");
    return lua_cli_request_answer_location(state, request, lua_toboolean(state, 2), arguments == 2,
                                           arguments == 2 ? lua_tointeger(state, 3) : 0);
}

/* Request:cancel(): deny the location request. */
static int lua_cli_request_cancel(lua_State* state) {
    LuaCliRequest* request = lua_cli_test_request(state, 1);
    if (!request)
        return luaL_error(state, "Request:cancel requires a Gnoblin Request");
    if (lua_gettop(state) != 1)
        return luaL_error(state, "Request:cancel takes no arguments");
    return lua_cli_request_answer_location(state, request, FALSE, FALSE, 0);
}

static int lua_cli_request_index(lua_State* state) {
    if (!lua_cli_test_request(state, 1) || lua_type(state, 2) != LUA_TSTRING) {
        lua_pushnil(state);
        return 1;
    }
    const char* key = lua_tostring(state, 2);
    if (g_str_equal(key, "answer")) {
        lua_pushcfunction(state, lua_cli_request_answer);
        return 1;
    }
    if (g_str_equal(key, "cancel")) {
        lua_pushcfunction(state, lua_cli_request_cancel);
        return 1;
    }
    if (g_str_equal(key, "claim"))
        return luaL_error(state, "a location request has no claim method");
    if (g_str_equal(key, "finished") || g_str_equal(key, "on")) {
        g_autofree char* label = g_strdup_printf("Request:%s", key);
        lua_pushstring(state, label);
        lua_pushcclosure(state, lua_cli_not_available, 1);
        return 1;
    }
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, key);
    return 1;
}

static int lua_cli_request_newindex(lua_State* state) {
    return luaL_error(state, "Request fields are read-only");
}

static int lua_cli_request_tostring(lua_State* state) {
    LuaCliRequest* request = lua_cli_test_request(state, 1);
    lua_pushfstring(state, "Request location %I", (LUAI_UACINT)(request ? request->request_id : 0));
    return 1;
}

void register_lua_cli_request(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_REQUEST_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_request_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_request_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_request_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pushliteral(state, "Request");
    lua_setfield(state, -2, "__metatable");
    lua_pop(state, 1);
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
