#include "gnoblinctl-internal.h"

static int lua_cli_portal_grant_revoke(lua_State* state);

static gboolean lua_cli_session_status_valid(JsonObject* object) {
    const char* session_state = member_string(object, "state", NULL);
    JsonNode* lock_available = json_object_get_member(object, "lock_available");
    if (!session_state || !g_str_equal(session_state, "running") || !lock_available ||
        !JSON_NODE_HOLDS_VALUE(lock_available) ||
        json_node_get_value_type(lock_available) != G_TYPE_BOOLEAN)
        return FALSE;
    JsonNode* revision = json_object_get_member(object, "revision");
    if (revision && (!JSON_NODE_HOLDS_VALUE(revision) ||
                     (json_node_get_value_type(revision) != G_TYPE_INT64 &&
                      json_node_get_value_type(revision) != G_TYPE_INT) ||
                     json_node_get_int(revision) < 0))
        return FALSE;
    JsonNode* lifecycle_state = json_object_get_member(object, "session_state");
    JsonNode* lifecycle_revision = json_object_get_member(object, "session_revision");
    if (!!lifecycle_state != !!lifecycle_revision)
        return FALSE;
    if (lifecycle_state) {
        if (!JSON_NODE_HOLDS_VALUE(lifecycle_state) ||
            json_node_get_value_type(lifecycle_state) != G_TYPE_STRING ||
            !JSON_NODE_HOLDS_VALUE(lifecycle_revision) ||
            (json_node_get_value_type(lifecycle_revision) != G_TYPE_INT64 &&
             json_node_get_value_type(lifecycle_revision) != G_TYPE_INT) ||
            json_node_get_int(lifecycle_revision) < 1)
            return FALSE;
        const char* lifecycle = json_node_get_string(lifecycle_state);
        if (!g_str_equal(lifecycle, "starting") && !g_str_equal(lifecycle, "running") &&
            !g_str_equal(lifecycle, "stopping"))
            return FALSE;
    }
    JsonNode* lock_state_node = json_object_get_member(object, "lock_state");
    if (!json_node_get_boolean(lock_available))
        return lock_state_node == NULL;
    if (!lock_state_node || !JSON_NODE_HOLDS_VALUE(lock_state_node) ||
        json_node_get_value_type(lock_state_node) != G_TYPE_STRING)
        return FALSE;
    const char* lock_state = json_node_get_string(lock_state_node);
    return g_str_equal(lock_state, "unlocked") || g_str_equal(lock_state, "covering") ||
           g_str_equal(lock_state, "locked") || g_str_equal(lock_state, "failsafe");
}

static int lua_cli_session_status_tostring(lua_State* state) {
    lua_pushliteral(state, "SessionStatus");
    return 1;
}

void register_lua_cli_session_status_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_session_status_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_session_status(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.status takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor_with_api_minor(cli, "api", "session.status", arguments, 76, &call_error);
    if (!result && call_error &&
        g_str_equal(call_error->message, "requested compositor API version is unsupported")) {
        g_clear_error(&call_error);
        result = call_compositor_with_api_minor(cli, "api", "session.status", arguments, 29,
                                                &call_error);
    }
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.session.status failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_session_status_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.session.status returned an invalid SessionStatus");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SESSION_STATUS_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static gboolean lua_cli_session_activity_valid(JsonObject* object) {
    static const char* const boolean_fields[] = {"available", "idle"};
    static const char* const integer_fields[] = {"threshold_ms", "idle_for_ms", "revision"};
    for (guint i = 0; i < G_N_ELEMENTS(boolean_fields); i++) {
        JsonNode* field = json_object_get_member(object, boolean_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_BOOLEAN)
            return FALSE;
    }
    for (guint i = 0; i < G_N_ELEMENTS(integer_fields); i++) {
        JsonNode* field = json_object_get_member(object, integer_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            (json_node_get_value_type(field) != G_TYPE_INT64 &&
             json_node_get_value_type(field) != G_TYPE_INT) ||
            json_node_get_int(field) < 0)
            return FALSE;
    }
    return TRUE;
}

static int lua_cli_session_activity_tostring(lua_State* state) {
    lua_pushliteral(state, "SessionActivity");
    return 1;
}

void register_lua_cli_session_activity_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_session_activity_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_session_activity(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.activity takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "session.activity", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.session.activity failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_session_activity_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.session.activity returned an invalid SessionActivity");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SESSION_ACTIVITY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

int lua_cli_focus_policy_property(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.focus.policy takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "focus.policy", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.focus.policy failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.focus.policy returned an invalid snapshot");
    JsonObject* object = json_node_get_object(result);
    if (!lua_cli_focus_policy_valid(object))
        return luaL_error(state, "gnoblin.focus.policy returned an invalid FocusPolicy");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}

static int lua_cli_input_record_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    const char* type = luaL_testudata(state, 1, GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE)
                           ? "InputDevice"
                           : "InputSource";
    lua_pushfstring(state, "%s<%s>", type, id ? id : "unknown");
    return 1;
}

int lua_cli_input_snapshot(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", lua_tostring(state, lua_upvalueindex(2)));
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    g_autoptr(GError) call_error = NULL;
    JsonObject* arguments = json_object_new();
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "%s returned an invalid snapshot", method);

    JsonObject* snapshot = json_node_get_object(result);
    if (g_str_equal(method, "input.orientation_lock")) {
        JsonNode* available = json_object_get_member(snapshot, "available");
        JsonNode* locked = json_object_get_member(snapshot, "locked");
        JsonNode* orientation = json_object_get_member(snapshot, "orientation");
        JsonNode* source = json_object_get_member(snapshot, "source");
        gint64 revision = json_object_get_int_member_with_default(snapshot, "revision", -1);
        if (!available || !JSON_NODE_HOLDS_VALUE(available) ||
            json_node_get_value_type(available) != G_TYPE_BOOLEAN || !locked ||
            !JSON_NODE_HOLDS_VALUE(locked) || json_node_get_value_type(locked) != G_TYPE_BOOLEAN ||
            !orientation || !JSON_NODE_HOLDS_VALUE(orientation) ||
            json_node_get_value_type(orientation) != G_TYPE_STRING || !source ||
            !JSON_NODE_HOLDS_VALUE(source) || json_node_get_value_type(source) != G_TYPE_STRING ||
            revision < 0)
            return luaL_error(state, "%s returned an invalid OrientationLock", method);
        json_to_lua(state, result);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    if (g_str_equal(method, "input.current_source")) {
        JsonNode* available = json_object_get_member(snapshot, "available");
        if (!available || !JSON_NODE_HOLDS_VALUE(available) ||
            json_node_get_value_type(available) != G_TYPE_BOOLEAN)
            return luaL_error(state, "%s returned an invalid availability state", method);
        if (!json_node_get_boolean(available)) {
            lua_pushnil(state);
            return 1;
        }
        /* The reply has no source member while nothing is current, so read it only after the check
         * above. */
        JsonObject* source = json_object_get_object_member(snapshot, "source");
        if (!source)
            return luaL_error(state, "%s omitted its current source", method);
        gint64 revision = json_object_get_int_member_with_default(snapshot, "revision", -1);
        if (revision < 0)
            return luaL_error(state, "%s returned an invalid revision", method);
        lua_cli_push_input_record(state, source, revision, FALSE);
        return 1;
    }

    gboolean is_device = g_str_equal(method, "input.devices");
    const char* records_key = is_device ? "devices" : "sources";
    JsonArray* records = json_object_get_array_member(snapshot, records_key);
    gint64 revision = json_object_get_int_member_with_default(snapshot, "revision", -1);
    if (!records || revision < 0)
        return luaL_error(state, "%s returned an invalid snapshot", method);
    lua_createtable(state, json_array_get_length(records), 0);
    for (guint i = 0; i < json_array_get_length(records); i++) {
        JsonObject* record = json_array_get_object_element(records, i);
        if (!record)
            return luaL_error(state, "%s returned an invalid record", method);
        lua_cli_push_input_record(state, record, revision, is_device);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_appearance_color_scheme(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.appearance.color_scheme takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "appearance.color_scheme", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.appearance.color_scheme failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.appearance.color_scheme returned an invalid result");

    JsonObject* response = json_node_get_object(result);
    JsonNode* available = json_object_get_member(response, "available");
    if (!available || !JSON_NODE_HOLDS_VALUE(available) ||
        json_node_get_value_type(available) != G_TYPE_BOOLEAN)
        return luaL_error(state, "gnoblin.appearance.color_scheme returned invalid availability");
    if (!json_node_get_boolean(available)) {
        if (json_object_has_member(response, "color_scheme"))
            return luaL_error(state, "unavailable appearance result included a color scheme");
        lua_pushnil(state);
        return 1;
    }

    JsonNode* color_scheme_node = json_object_get_member(response, "color_scheme");
    if (!color_scheme_node || !JSON_NODE_HOLDS_VALUE(color_scheme_node) ||
        json_node_get_value_type(color_scheme_node) != G_TYPE_STRING)
        return luaL_error(state, "gnoblin.appearance.color_scheme omitted its string value");
    const char* color_scheme = json_node_get_string(color_scheme_node);
    if (!g_str_equal(color_scheme, "default") && !g_str_equal(color_scheme, "prefer-dark") &&
        !g_str_equal(color_scheme, "prefer-light"))
        return luaL_error(state, "gnoblin.appearance.color_scheme returned an unsupported value");
    lua_pushstring(state, color_scheme);
    return 1;
}

int lua_cli_input_select_source(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "input.select_source requires a {type, id} selector");
    const char* type = NULL;
    const char* id = NULL;
    lua_pushnil(state);
    while (lua_next(state, 1)) {
        if (lua_type(state, -2) != LUA_TSTRING) {
            lua_pop(state, 2);
            return luaL_error(state, "input selector accepts only type and id strings");
        }
        const char* key = lua_tostring(state, -2);
        if (g_str_equal(key, "type") && lua_type(state, -1) == LUA_TSTRING)
            type = lua_tostring(state, -1);
        else if (g_str_equal(key, "id") && lua_type(state, -1) == LUA_TSTRING)
            id = lua_tostring(state, -1);
        else {
            lua_pop(state, 2);
            return luaL_error(state, "input selector accepts only type and id strings");
        }
        lua_pop(state, 1);
    }
    if (!type || !*type || !id || !*id || (!g_str_equal(type, "xkb") && !g_str_equal(type, "ibus")))
        return luaL_error(state, "input.select_source requires type xkb or ibus and a nonempty id");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "type", type);
    json_object_set_string_member(arguments, "id", id);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "input.select", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "input.select_source failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "input.select_source returned an invalid InputSource");
    JsonObject* source = json_node_get_object(result);
    gint64 revision = json_object_get_int_member_with_default(source, "revision", -1);
    if (revision < 0)
        return luaL_error(state, "input.select_source returned an invalid InputSource");
    lua_cli_push_input_record(state, source, revision, FALSE);
    return 1;
}

int lua_cli_input_set_orientation_lock(lua_State* state) {
    if (lua_gettop(state) != 1 || (!lua_isboolean(state, 1) &&
                                   !(lua_type(state, 1) == LUA_TSTRING && lua_isstring(state, 1) &&
                                     g_str_equal(lua_tostring(state, 1), "inherit"))))
        return luaL_error(state, "input.set_orientation_lock requires true, false, or 'inherit'");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    if (lua_isboolean(state, 1))
        json_object_set_boolean_member(arguments, "value", lua_toboolean(state, 1));
    else
        json_object_set_string_member(arguments, "value", "inherit");
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "input.set_orientation_lock", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "input.set_orientation_lock failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "input.set_orientation_lock returned an invalid snapshot");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

static gboolean lua_cli_portal_grant_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "kind", "requester"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    const char* kind = member_string(object, "kind", NULL);
    if (!member_string(object, "id", NULL) || !*member_string(object, "id", "") ||
        !member_string(object, "requester", NULL) ||
        (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")))
        return FALSE;

    JsonNode* devices = json_object_get_member(object, "devices");
    JsonNode* clipboard = json_object_get_member(object, "clipboard");
    JsonNode* streams = json_object_get_member(object, "has_screen_streams");
    JsonNode* created_at = json_object_get_member(object, "created_at");
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!devices || !JSON_NODE_HOLDS_ARRAY(devices) || !clipboard ||
        !JSON_NODE_HOLDS_VALUE(clipboard) ||
        json_node_get_value_type(clipboard) != G_TYPE_BOOLEAN || !streams ||
        !JSON_NODE_HOLDS_VALUE(streams) || json_node_get_value_type(streams) != G_TYPE_BOOLEAN ||
        !created_at || !JSON_NODE_HOLDS_VALUE(created_at) ||
        (json_node_get_value_type(created_at) != G_TYPE_INT64 &&
         json_node_get_value_type(created_at) != G_TYPE_INT) ||
        !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT))
        return FALSE;

    JsonArray* device_array = json_node_get_array(devices);
    for (guint i = 0; i < json_array_get_length(device_array); i++) {
        JsonNode* device = json_array_get_element(device_array, i);
        if (!device || !JSON_NODE_HOLDS_VALUE(device) ||
            json_node_get_value_type(device) != G_TYPE_STRING)
            return FALSE;
    }
    return TRUE;
}

static void lua_cli_push_portal_grant_record(lua_State* state, Cli* cli, JsonObject* object) {
    if (!lua_cli_portal_grant_valid(object))
        luaL_error(state, "gnoblin.portals.grants returned an invalid PortalGrant");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, cli);
    lua_pushcclosure(state, lua_cli_portal_grant_revoke, 1);
    lua_setfield(state, -2, "revoke");
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_portal_grant_revoke(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    luaL_checkudata(state, 1, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE);
    if (lua_gettop(state) != 1)
        return luaL_error(state, "PortalGrant:revoke takes no arguments");

    lua_getiuservalue(state, 1, 1);
    JsonObject* arguments = json_object_new();
    const char* kind = NULL;
    const char* id = NULL;
    lua_getfield(state, -1, "kind");
    kind = lua_tostring(state, -1);
    if (kind)
        json_object_set_string_member(arguments, "kind", kind);
    lua_pop(state, 1);
    lua_getfield(state, -1, "id");
    id = lua_tostring(state, -1);
    if (id)
        json_object_set_string_member(arguments, "id", id);
    lua_pop(state, 1);
    lua_getfield(state, -1, "created_at");
    if (lua_isinteger(state, -1))
        json_object_set_int_member(arguments, "created_at", lua_tointeger(state, -1));
    else {
        json_object_unref(arguments);
        return luaL_error(state, "PortalGrant record has no creation timestamp");
    }
    lua_pop(state, 2);

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "grant.revoke", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "grant.revoke failed: %s", call_error->message);
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* ok = response ? json_object_get_member(response, "ok") : NULL;
    if (!ok || !JSON_NODE_HOLDS_VALUE(ok) || json_node_get_value_type(ok) != G_TYPE_BOOLEAN ||
        !json_node_get_boolean(ok) || !g_str_equal(member_string(response, "id", ""), id))
        return luaL_error(state, "grant.revoke returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_grant_revoke(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.grant.revoke requires an options table");
    static const char* const allowed_fields[] = {"kind", "id", "created_at", NULL};
    g_autoptr(JsonObject) options = lua_cli_table_object(state, 1, "grant options");
    const char* kind = member_string(options, "kind", NULL);
    const char* id = member_string(options, "id", NULL);
    JsonNode* created_at = json_object_get_member(options, "created_at");
    if (!lua_cli_workspace_object_has_only_keys(options, allowed_fields) || !kind ||
        (!g_str_equal(kind, "screen-cast") && !g_str_equal(kind, "remote-desktop")) || !id ||
        !*id ||
        (created_at && (!JSON_NODE_HOLDS_VALUE(created_at) ||
                        (json_node_get_value_type(created_at) != G_TYPE_INT &&
                         json_node_get_value_type(created_at) != G_TYPE_INT64) ||
                        json_node_get_int(created_at) < 0)))
        return luaL_error(state, "gnoblin.grant.revoke received invalid grant options");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "kind", kind);
    json_object_set_string_member(arguments, "id", id);
    if (created_at)
        json_object_set_member(arguments, "created_at", json_node_copy(created_at));
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "grant.revoke", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.grant.revoke failed: %s", call_error->message);
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* ok = response ? json_object_get_member(response, "ok") : NULL;
    if (!ok || !JSON_NODE_HOLDS_VALUE(ok) || json_node_get_value_type(ok) != G_TYPE_BOOLEAN ||
        !json_node_get_boolean(ok) || !g_str_equal(member_string(response, "id", ""), id))
        return luaL_error(state, "gnoblin.grant.revoke returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_portal_grants(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.portals.grants accepts one optional filter table");

    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            gboolean valid = lua_type(state, -2) == LUA_TSTRING &&
                             g_str_equal(lua_tostring(state, -2), "kind") &&
                             lua_type(state, -1) == LUA_TSTRING &&
                             (g_str_equal(lua_tostring(state, -1), "screen-cast") ||
                              g_str_equal(lua_tostring(state, -1), "remote-desktop"));
            if (!valid) {
                json_object_unref(arguments);
                return luaL_error(
                    state, "portal grant filter accepts only kind = screen-cast or remote-desktop");
            }
            json_object_set_string_member(arguments, "kind", lua_tostring(state, -1));
            lua_pop(state, 1);
        }
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "portals.grants", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.portals.grants failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.portals.grants returned an invalid snapshot");

    JsonArray* grants = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(grants), 0);
    for (guint i = 0; i < json_array_get_length(grants); i++) {
        JsonObject* grant = json_array_get_object_element(grants, i);
        if (!grant)
            return luaL_error(state, "gnoblin.portals.grants returned an invalid record");
        lua_cli_push_portal_grant_record(state, cli, grant);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_layers_list(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.layers.list accepts one optional filter table");

    JsonObject* arguments = NULL;
    if (lua_gettop(state) == 1) {
        static const char* fields[] = {"monitor_id", "namespace", "layer"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || lua_type(state, -1) != LUA_TSTRING)
                return luaL_error(state, "gnoblin.layers.list filter fields must be monitor_id, "
                                         "namespace, or layer strings");
            lua_pop(state, 1);
        }
        arguments = lua_cli_table_object(state, 1, "layer filter");
    } else {
        arguments = json_object_new();
    }

    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "layers.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.layers.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.layers.list returned an invalid snapshot");

    JsonArray* layers = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(layers), 0);
    for (guint i = 0; i < json_array_get_length(layers); i++) {
        JsonObject* layer = json_array_get_object_element(layers, i);
        if (!layer)
            return luaL_error(state,
                              "gnoblin.layers.list returned an invalid layer surface record");
        lua_cli_push_layer_surface_record(state, layer);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

void register_lua_cli_layer_surface_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_window_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_layer_surface_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_animation_preview_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "AnimationPreview<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_animation_preview_newindex(lua_State* state) {
    return luaL_error(state, "AnimationPreview records are read-only");
}

void register_lua_cli_animation_preview_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_animation_preview_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_animation_preview_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_portal_grant_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "PortalGrant<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_portal_grant_newindex(lua_State* state) {
    return luaL_error(state, "PortalGrant records are read-only");
}

void register_lua_cli_portal_grant_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_PORTAL_GRANT_RECORD_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_portal_grant_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_portal_grant_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

void register_lua_cli_input_record(lua_State* state, const char* metatable) {
    if (!luaL_newmetatable(state, metatable)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_window_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_window_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_window_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushcfunction(state, lua_cli_input_record_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}
