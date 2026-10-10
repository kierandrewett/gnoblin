#include "gnoblinctl-internal.h"

static int lua_cli_animation_preview_method(lua_State* state);

/* Nested JSON values are userdata-backed proxies instead of ordinary Lua
 * tables. An empty proxy table would still allow rawset() to shadow fields,
 * while returning the backing table from __pairs would expose mutable state. */
void lua_cli_push_readonly_value(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index)) {
        lua_pushvalue(state, index);
        return;
    }

    lua_newuserdatauv(state, 1, 1);
    int proxy = lua_absindex(state, -1);
    lua_pushvalue(state, index);
    lua_setiuservalue(state, proxy, 1);
    luaL_getmetatable(state, GNOBLINCTL_READONLY_TABLE_METATABLE);
    lua_setmetatable(state, proxy);
}

int lua_cli_readonly_index(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    lua_cli_push_readonly_value(state, -1);
    return 1;
}

int lua_cli_readonly_newindex(lua_State* state) {
    return luaL_error(state, "Snapshot values are read-only");
}

static int lua_cli_readonly_len(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushinteger(state, (lua_Integer)lua_rawlen(state, -1));
    return 1;
}

static int lua_cli_readonly_next(lua_State* state) {
    lua_pushvalue(state, lua_upvalueindex(1));
    int backing = lua_absindex(state, -1);
    lua_pushvalue(state, 2);
    if (!lua_next(state, backing))
        return 0;
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    lua_remove(state, backing);
    return 2;
}

static int lua_cli_readonly_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_cli_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

void register_lua_cli_readonly_table(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_READONLY_TABLE_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_readonly_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_readonly_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_readonly_len);
    lua_setfield(state, -2, "__len");
    lua_pushcfunction(state, lua_cli_readonly_pairs);
    lua_setfield(state, -2, "__pairs");
    lua_pushliteral(state, "read-only snapshot");
    lua_setfield(state, -2, "__metatable");
    lua_pop(state, 1);
}

/* The console must not turn a compositor snapshot into a mutable policy object.
 * Keep the JSON fields in a private backing table and expose the same read-only
 * property/method split as the supervised runtime's Window records. */
int lua_cli_window_index(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    if (!lua_isnil(state, -1)) {
        lua_cli_push_readonly_value(state, -1);
        return 1;
    }
    lua_pop(state, 2);
    lua_getiuservalue(state, 1, 2);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    return 1;
}

int lua_cli_window_newindex(lua_State* state) {
    return luaL_error(state, "Window records are read-only");
}

int lua_cli_window_len(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushinteger(state, (lua_Integer)lua_rawlen(state, -1));
    return 1;
}

int lua_cli_window_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_cli_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

int lua_cli_window_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Window<%s>", id ? id : "unknown");
    return 1;
}

const char* lua_cli_window_id(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_WINDOW_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    if (!id || !*id)
        luaL_error(state, "Window record has no stable id");
    return id;
}

const char* lua_cli_workspace_id(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_WORKSPACE_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    if (!id || !*id)
        luaL_error(state, "Workspace record has no stable id");
    return id;
}

static const char* lua_cli_animation_preview_session(lua_State* state) {
    luaL_checkudata(state, 1, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE);
    lua_getiuservalue(state, 1, 3);
    const char* session = lua_tostring(state, -1);
    if (!session || !*session)
        luaL_error(state, "AnimationPreview record has no session ID");
    return session;
}

static gboolean lua_cli_animation_preview_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "name", "event", "target", "target_type"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* progress = json_object_get_member(object, "progress");
    JsonNode* playing = json_object_get_member(object, "playing");
    JsonNode* revision = json_object_get_member(object, "revision");
    return progress && JSON_NODE_HOLDS_VALUE(progress) &&
           (json_node_get_value_type(progress) == G_TYPE_DOUBLE ||
            json_node_get_value_type(progress) == G_TYPE_INT ||
            json_node_get_value_type(progress) == G_TYPE_INT64) &&
           playing && JSON_NODE_HOLDS_VALUE(playing) &&
           json_node_get_value_type(playing) == G_TYPE_BOOLEAN && revision &&
           JSON_NODE_HOLDS_VALUE(revision) &&
           (json_node_get_value_type(revision) == G_TYPE_INT ||
            json_node_get_value_type(revision) == G_TYPE_INT64);
}

void lua_cli_push_animation_preview_record(lua_State* state, Cli* cli, JsonObject* object,
                                           const char* session) {
    if (!lua_cli_animation_preview_valid(object) || !session || !*session)
        luaL_error(state, "animation preview completion returned an invalid AnimationPreview");

    /* The compositor's session token is an implementation detail used to
     * address subsequent controls. Keep it out of the public read-only record. */
    JsonObject* public_fields = json_object_new();
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "session"))
            json_object_set_member(public_fields, key,
                                   json_node_copy(json_object_get_member(object, key)));
    }
    g_list_free(members);
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, public_fields);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 3);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    static const char* const methods[][2] = {
        {"seek", "animation.seek"},   {"step", "animation.step"}, {"play", "animation.play"},
        {"pause", "animation.pause"}, {"stop", "animation.stop"}, {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_animation_preview_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
    lua_setiuservalue(state, record, 2);
    lua_pushstring(state, session);
    lua_setiuservalue(state, record, 3);
    luaL_getmetatable(state, GNOBLINCTL_ANIMATION_PREVIEW_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

JsonObject* lua_cli_table_object(lua_State* state, int index, const char* description) {
    g_autoptr(GError) conversion_error = NULL;
    g_autoptr(JsonNode) node = lua_to_json(state, index, 0, &conversion_error);
    if (!node)
        luaL_error(state, "%s: %s", description, conversion_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(node))
        luaL_error(state, "%s must be a named table", description);
    return json_object_ref(json_node_get_object(node));
}

static int lua_cli_animation_preview_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* session = g_strdup(lua_cli_animation_preview_session(state));
    lua_settop(state, supplied + 1);

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "session", session);
    if (g_str_equal(method, "animation.seek")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TNUMBER) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        }
        double progress = lua_tonumber(state, 2);
        if (!isfinite(progress) || progress < 0.0 || progress > 1.0) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        }
        json_object_set_double_member(arguments, "progress", progress);
    } else if (g_str_equal(method, "animation.step")) {
        if (supplied != 1 || !lua_isinteger(state, 2) || lua_tointeger(state, 2) < 1 ||
            lua_tointeger(state, 2) > 60000) {
            json_object_unref(arguments);
            return luaL_error(state, "preview:step requires an integer from 1 to 60000");
        }
        json_object_set_int_member(arguments, "milliseconds", lua_tointeger(state, 2));
    } else if (g_str_equal(method, "animation.play") || g_str_equal(method, "animation.pause") ||
               g_str_equal(method, "animation.stop")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method + strlen("animation."));
        }
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported AnimationPreview method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    if (g_str_equal(method, "animation.stop")) {
        json_to_lua(state, result);
        return 1;
    }
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "%s returned an invalid AnimationPreview", method);
    JsonObject* preview = json_node_get_object(result);
    const char* next_session = member_string(preview, "session", NULL);
    if (!next_session)
        next_session = member_string(preview, "id", NULL);
    lua_cli_push_animation_preview_record(state, cli, preview, next_session);
    return 1;
}
