#include "gnoblinctl-internal.h"

static void lua_cli_set_window_id(JsonObject* arguments, const char* id) {
    json_object_set_string_member(arguments, "id", id);
}

/* A Grant here wraps the compositor's one-use token for a gesture. The wire has no narrowing, so
 * `narrow` is enforced in this process; the compositor still takes each operation once. */
static const struct {
    const char* name;
    LuaCliGrantScope scope;
} lua_cli_grant_scopes[] = {
    {"focus", LUA_CLI_GRANT_FOCUS},
    {"begin_move", LUA_CLI_GRANT_BEGIN_MOVE},
    {"begin_resize", LUA_CLI_GRANT_BEGIN_RESIZE},
    {"snap_context", LUA_CLI_GRANT_SNAP_CONTEXT},
    {"text_target", LUA_CLI_GRANT_TEXT_TARGET},
};

static const char* lua_cli_grant_scope_name(LuaCliGrantScope scope) {
    for (guint i = 0; i < G_N_ELEMENTS(lua_cli_grant_scopes); i++)
        if (lua_cli_grant_scopes[i].scope == scope)
            return lua_cli_grant_scopes[i].name;
    return "operation";
}

LuaCliGrant* lua_cli_test_grant(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_GRANT_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliGrant))
        return NULL;
    LuaCliGrant* grant = lua_touserdata(state, index);
    return grant->magic == LUA_CLI_GRANT_MAGIC && grant->subscription && grant->token &&
                   grant->spent
               ? grant
               : NULL;
}

static void lua_cli_new_grant(lua_State* state, const LuaCliGrant* source) {
    LuaCliGrant* grant = lua_newuserdatauv(state, sizeof *grant, 0);
    *grant = *source;
    grant->token = g_strdup(source->token);
    grant->spent->refs++;
    luaL_getmetatable(state, GNOBLINCTL_GRANT_METATABLE);
    lua_setmetatable(state, -2);
}

void lua_cli_push_grant(lua_State* state, CliLuaEventSubscription* subscription, const char* token,
                        gboolean menu, const char* window_id) {
    LuaCliGrantSpent* spent = g_new0(LuaCliGrantSpent, 1);
    LuaCliGrant grant = {.magic = LUA_CLI_GRANT_MAGIC,
                         .subscription = subscription,
                         .token = (char*)token,
                         .menu = menu,
                         .scope = menu ? (LUA_CLI_GRANT_BEGIN_MOVE | LUA_CLI_GRANT_BEGIN_RESIZE)
                                       : LUA_CLI_GRANT_ALL,
                         .spent = spent};
    g_strlcpy(grant.window_id, window_id ? window_id : "", sizeof grant.window_id);
    lua_cli_new_grant(state, &grant);
}

static gboolean lua_cli_resize_edge_valid(const char* edge) {
    return edge && (g_str_equal(edge, "north") || g_str_equal(edge, "south") ||
                    g_str_equal(edge, "east") || g_str_equal(edge, "west") ||
                    g_str_equal(edge, "north_east") || g_str_equal(edge, "north_west") ||
                    g_str_equal(edge, "south_east") || g_str_equal(edge, "south_west"));
}

/* The string field `field` of the record at `index` when it is a record of `metatable`; NULL for
 * any other value. */
static const char* lua_cli_record_string(lua_State* state, int index, const char* metatable,
                                         const char* field) {
    if (!luaL_testudata(state, index, metatable))
        return NULL;
    lua_getiuservalue(state, index, 1);
    lua_getfield(state, -1, field);
    const char* value = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
    lua_pop(state, 2); /* the string stays alive in the record */
    return value;
}

/* The monitor ID that the value at `index` names: an Output (the monitor that shows it) or a
 * monitor ID string. NULL for any other value. */
static const char* lua_cli_monitor_argument(lua_State* state, int index) {
    if (lua_type(state, index) == LUA_TSTRING)
        return *lua_tostring(state, index) ? lua_tostring(state, index) : NULL;
    const char* monitor =
        lua_cli_record_string(state, index, GNOBLINCTL_OUTPUT_RECORD_METATABLE, "monitor_id");
    if (!monitor && luaL_testudata(state, index, GNOBLINCTL_OUTPUT_RECORD_METATABLE))
        luaL_error(state, "this Output is not enabled, so it shows no monitor");
    return monitor && *monitor ? monitor : NULL;
}

/* Take `scope` for `method` from the Grant at `index`. */
static LuaCliGrant* lua_cli_grant_take(lua_State* state, Cli* cli, int index,
                                       LuaCliGrantScope scope, const char* method,
                                       const char* window_id) {
    LuaCliGrant* grant = lua_cli_test_grant(state, index);
    if (!grant) {
        luaL_error(state, "%s requires a Grant received from a Gnoblin event", method);
        return NULL;
    }
    CliLuaEventSubscription* subscription = grant->subscription;
    if (!subscription->active || !subscription->connection || subscription->cli != cli) {
        luaL_error(state, "%s requires a Grant from an active event subscription", method);
        return NULL;
    }
    if (!(grant->scope & scope)) {
        luaL_error(state, "%s: this Grant has no %s scope", method,
                   lua_cli_grant_scope_name(scope));
        return NULL;
    }
    if (grant->window_id[0] && window_id && !g_str_equal(grant->window_id, window_id)) {
        luaL_error(state, "%s: this Grant is narrowed to window %s, not window %s", method,
                   grant->window_id, window_id);
        return NULL;
    }
    if (grant->spent->used & scope) {
        luaL_error(state, "%s: this gesture already used its %s scope", method,
                   lua_cli_grant_scope_name(scope));
        return NULL;
    }
    /* The compositor takes the scope on the first valid request, even when the action is
     * denied, so never offer local reuse. A menu request offers one move or one resize. */
    grant->spent->used |= grant->menu ? LUA_CLI_GRANT_ALL : scope;
    return grant;
}

JsonNode* lua_cli_grant_request(lua_State* state, Cli* cli, int index, LuaCliGrantScope scope,
                                const char* method, const char* window_id, const char* edge,
                                CliLuaEventSubscription** subscription) {
    LuaCliGrant* grant = lua_cli_grant_take(state, cli, index, scope, method, window_id);
    JsonObject* arguments = json_object_new();
    if (grant->menu) {
        json_object_set_string_member(arguments, "menu_context", grant->token);
    } else {
        json_object_set_string_member(arguments, "focus_context", grant->token);
        if (window_id)
            lua_cli_set_window_id(arguments, window_id);
    }
    if (edge)
        json_object_set_string_member(arguments, "edge", edge);
    GError* error = NULL;
    CliLuaEventSubscription* owner = grant->subscription;
    JsonNode* result =
        grant->menu
            ? cli_lua_call_compositor_direct_on_subscription(owner, method, arguments, &error)
            : cli_lua_call_compositor_on_subscription(owner, method, arguments, &error);
    json_object_unref(arguments);
    if (!result) {
        char message[512];
        g_snprintf(message, sizeof message, "%s",
                   error ? error->message : "unknown compositor error");
        g_clear_error(&error);
        luaL_error(state, "%s failed: %s", method, message);
        return NULL;
    }
    if (subscription)
        *subscription = owner;
    return result;
}

static gboolean lua_cli_grant_window_argument(lua_State* state, int index, char* out) {
    if (lua_isnoneornil(state, index))
        return TRUE;
    const char* id = NULL;
    if (lua_type(state, index) == LUA_TSTRING) {
        id = lua_tostring(state, index);
    } else if (lua_type(state, index) == LUA_TTABLE || lua_type(state, index) == LUA_TUSERDATA) {
        lua_getfield(state, index, "id");
        id = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_pop(state, 1);
    }
    if (!id || !*id || strlen(id) >= sizeof((LuaCliGrant){0}.window_id))
        return FALSE;
    g_strlcpy(out, id, sizeof((LuaCliGrant){0}.window_id));
    return TRUE;
}

/* grant:narrow(scope, window?): a Grant for fewer operations and, optionally, one window. */
static int lua_cli_grant_narrow(lua_State* state) {
    LuaCliGrant* grant = lua_cli_test_grant(state, 1);
    if (!grant)
        return luaL_error(state, "Grant:narrow called on a forged or invalid value");
    if (lua_gettop(state) < 2 || lua_gettop(state) > 3)
        return luaL_error(state, "Grant:narrow takes operations and an optional window");
    guint requested = 0;
    int count = 0;
    if (lua_type(state, 2) == LUA_TSTRING) {
        lua_pushvalue(state, 2);
        count = 1;
    } else if (lua_type(state, 2) == LUA_TTABLE && lua_rawlen(state, 2) > 0 &&
               lua_rawlen(state, 2) <= G_N_ELEMENTS(lua_cli_grant_scopes)) {
        count = (int)lua_rawlen(state, 2);
        for (int i = 1; i <= count; i++)
            lua_rawgeti(state, 2, i);
    } else {
        return luaL_error(state, "Grant:narrow needs an operation name or a list of names");
    }
    for (int i = 0; i < count; i++) {
        const char* name =
            lua_type(state, -count + i) == LUA_TSTRING ? lua_tostring(state, -count + i) : NULL;
        guint bit = 0;
        for (guint s = 0; name && s < G_N_ELEMENTS(lua_cli_grant_scopes); s++)
            if (g_str_equal(name, lua_cli_grant_scopes[s].name))
                bit = lua_cli_grant_scopes[s].scope;
        if (!bit)
            return luaL_error(state,
                              "Grant:narrow: %s is not an operation (focus, begin_move, "
                              "begin_resize, snap_context, text_target)",
                              name ? name : "a non-string");
        requested |= bit;
    }
    lua_pop(state, count);
    for (guint s = 0; s < G_N_ELEMENTS(lua_cli_grant_scopes); s++) {
        guint bit = lua_cli_grant_scopes[s].scope;
        if (!(requested & bit))
            continue;
        if (!(grant->scope & bit))
            return luaL_error(state, "Grant:narrow cannot widen: this Grant has no %s scope",
                              lua_cli_grant_scopes[s].name);
        if (grant->spent->used & bit)
            return luaL_error(state, "Grant:narrow: this gesture already used its %s scope",
                              lua_cli_grant_scopes[s].name);
    }
    LuaCliGrant narrowed = *grant;
    narrowed.scope = requested;
    char window_id[sizeof grant->window_id] = {0};
    if (!lua_cli_grant_window_argument(state, 3, window_id))
        return luaL_error(state, "Grant:narrow needs a Window or a window ID for its window");
    if (window_id[0]) {
        if (grant->window_id[0] && !g_str_equal(grant->window_id, window_id))
            return luaL_error(state, "Grant:narrow cannot widen: this Grant is for window %s",
                              grant->window_id);
        g_strlcpy(narrowed.window_id, window_id, sizeof narrowed.window_id);
    }
    lua_cli_new_grant(state, &narrowed);
    return 1;
}

static int lua_cli_grant_index(lua_State* state) {
    LuaCliGrant* grant = lua_cli_test_grant(state, 1);
    if (!grant)
        return 0;
    const char* key = lua_type(state, 2) == LUA_TSTRING ? lua_tostring(state, 2) : NULL;
    if (!key)
        return 0;
    if (g_str_equal(key, "narrow")) {
        lua_pushcfunction(state, lua_cli_grant_narrow);
        return 1;
    }
    gboolean live = grant->subscription->active && grant->subscription->connection;
    if (g_str_equal(key, "scope")) {
        guint remaining = live ? grant->scope & ~grant->spent->used : 0;
        lua_newtable(state);
        lua_Integer next = 1;
        for (guint s = 0; s < G_N_ELEMENTS(lua_cli_grant_scopes); s++) {
            if (!(remaining & lua_cli_grant_scopes[s].scope))
                continue;
            lua_pushstring(state, lua_cli_grant_scopes[s].name);
            lua_rawseti(state, -2, next++);
        }
        return 1;
    }
    if (g_str_equal(key, "window")) {
        if (!grant->window_id[0]) {
            lua_pushnil(state);
            return 1;
        }
        lua_getglobal(state, "gnoblin");
        lua_getfield(state, -1, "windows");
        lua_getfield(state, -1, "by_id");
        lua_pushstring(state, grant->window_id);
        lua_call(state, 1, 1);
        return 1;
    }
    if (g_str_equal(key, "alive")) {
        lua_pushboolean(state, live && (grant->scope & ~grant->spent->used) != 0);
        return 1;
    }
    return 0;
}

static int lua_cli_grant_gc(lua_State* state) {
    LuaCliGrant* grant = luaL_testudata(state, 1, GNOBLINCTL_GRANT_METATABLE);
    if (!grant || lua_rawlen(state, 1) != sizeof(LuaCliGrant) ||
        grant->magic != LUA_CLI_GRANT_MAGIC)
        return 0;
    g_clear_pointer(&grant->token, g_free);
    if (grant->spent && --grant->spent->refs == 0)
        g_free(grant->spent);
    grant->spent = NULL;
    grant->subscription = NULL;
    grant->magic = 0;
    return 0;
}

static int lua_cli_grant_newindex(lua_State* state) {
    return luaL_error(state, "Grant values are read-only");
}

static int lua_cli_grant_tostring(lua_State* state) {
    lua_pushstring(state, lua_cli_test_grant(state, 1) ? "Grant" : "Grant<invalid>");
    return 1;
}

void register_lua_cli_grant(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_GRANT_METATABLE)) {
        lua_pop(state, 1);
        return;
    }
    lua_pushcfunction(state, lua_cli_grant_index);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_cli_grant_gc);
    lua_setfield(state, -2, "__gc");
    lua_pushcfunction(state, lua_cli_grant_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_cli_grant_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_window_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_window_id(state));
    lua_settop(state, supplied + 1);

    gboolean uses_grant = g_str_equal(method, "window.focus") ||
                          g_str_equal(method, "window.begin_move") ||
                          g_str_equal(method, "window.begin_resize");
    if (uses_grant) {
        gboolean resize = g_str_equal(method, "window.begin_resize");
        if (supplied != (resize ? 2 : 1))
            return luaL_error(state, "%s requires %sa Grant as its final argument", method,
                              resize ? "an edge and " : "");
        const char* edge = NULL;
        if (resize) {
            edge = lua_type(state, 2) == LUA_TSTRING ? lua_tostring(state, 2) : NULL;
            if (!lua_cli_resize_edge_valid(edge))
                return luaL_error(state, "window.begin_resize requires a valid ResizeEdge");
        }
        LuaCliGrantScope scope = g_str_equal(method, "window.focus") ? LUA_CLI_GRANT_FOCUS
                                 : resize                            ? LUA_CLI_GRANT_BEGIN_RESIZE
                                                                     : LUA_CLI_GRANT_BEGIN_MOVE;
        g_autoptr(JsonNode) grant_result =
            lua_cli_grant_request(state, cli, resize ? 3 : 2, scope, method, id, edge, NULL);
        json_to_lua(state, grant_result);
        return 1;
    }

    JsonObject* arguments = json_object_new();
    lua_cli_set_window_id(arguments, id);
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.kill") ||
        g_str_equal(method, "window.raise") || g_str_equal(method, "window.lower") ||
        g_str_equal(method, "window.restore_or_minimize")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method);
        }
    } else if (g_str_equal(method, "window.set_minimized")) {
        /* Minimizing and restoring are two compositor operations; the boolean picks one. */
        if (supplied != 1 || !lua_isboolean(state, 2)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires one boolean", method);
        }
        method = lua_toboolean(state, 2) ? "window.minimize" : "window.unminimize";
    } else if (g_str_equal(method, "window.set_maximized") ||
               g_str_equal(method, "window.set_fullscreen") ||
               g_str_equal(method, "window.set_above") ||
               g_str_equal(method, "window.set_sticky")) {
        if (supplied != 1 || !lua_isboolean(state, 2)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires one boolean", method);
        }
        json_object_set_boolean_member(arguments, "enabled", lua_toboolean(state, 2));
    } else if (g_str_equal(method, "window.move") || g_str_equal(method, "window.resize") ||
               g_str_equal(method, "window.thumbnail")) {
        if (supplied != 2 || !lua_isinteger(state, 2) || !lua_isinteger(state, 3)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires integer %s and %s", method,
                              g_str_equal(method, "window.move") ? "x" : "width",
                              g_str_equal(method, "window.move") ? "y" : "height");
        }
        json_object_set_int_member(arguments, g_str_equal(method, "window.move") ? "x" : "width",
                                   lua_tointeger(state, 2));
        json_object_set_int_member(arguments, g_str_equal(method, "window.move") ? "y" : "height",
                                   lua_tointeger(state, 3));
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        /* A Workspace names itself; a string or a number names a workspace by ID or position. */
        const char* workspace_id =
            lua_cli_record_string(state, 2, GNOBLINCTL_WORKSPACE_RECORD_METATABLE, "id");
        if (!workspace_id && lua_type(state, 2) == LUA_TSTRING)
            workspace_id = lua_tostring(state, 2);
        gboolean id_selector = workspace_id && *workspace_id;
        gboolean number_selector = lua_isinteger(state, 2) && lua_tointeger(state, 2) > 0;
        if (supplied < 1 || supplied > 2 || (!id_selector && !number_selector) ||
            (supplied == 2 && !lua_isnil(state, 3) && !lua_istable(state, 3))) {
            json_object_unref(arguments);
            return luaL_error(state, "window:move_to_workspace takes a Workspace, a workspace ID "
                                     "string or a number, and optional options");
        }
        g_autoptr(JsonObject) selector = json_object_new();
        if (id_selector)
            json_object_set_string_member(selector, "id", workspace_id);
        else
            json_object_set_int_member(selector, "number", lua_tointeger(state, 2));
        JsonNode* workspace = json_node_new(JSON_NODE_OBJECT);
        json_node_set_object(workspace, selector);
        json_object_set_member(arguments, "workspace", workspace);
        if (supplied == 2 && !lua_isnil(state, 3)) {
            g_autoptr(JsonObject) options = lua_cli_table_object(state, 3, "workspace options");
            JsonNode* follow = json_object_get_member(options, "follow");
            if (json_object_get_size(options) != 1 || !follow || !JSON_NODE_HOLDS_VALUE(follow) ||
                json_node_get_value_type(follow) != G_TYPE_BOOLEAN) {
                json_object_unref(arguments);
                return luaL_error(state, "window:move_to_workspace options accept only follow");
            }
            json_object_set_boolean_member(arguments, "follow", json_node_get_boolean(follow));
        }
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        const char* monitor = lua_cli_monitor_argument(state, 2);
        if (supplied != 1 || !monitor) {
            json_object_unref(arguments);
            return luaL_error(state, "window:move_to_monitor takes an Output or a monitor ID");
        }
        json_object_set_string_member(arguments, "monitor", monitor);
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported Window method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    JsonNode* raw_result = call_compositor(cli, "api", method, arguments, &call_error);
    g_autoptr(JsonNode) result = raw_result;
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    json_to_lua(state, result);
    return 1;
}

/* window:set_opacity(v), set_scale(s), set_rotation(degrees), set_offset(x, y) and
 * reset_visuals(): one window.set_visual call each, as in the runtime. Upvalue 3 names the
 * method; the compositor checks the ranges. */
int lua_cli_window_visual(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* name = lua_tostring(state, lua_upvalueindex(3));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_window_id(state));
    static const struct {
        const char* method;
        const char* fields[2];
    } visuals[] = {
        {"set_opacity", {"opacity", NULL}},   {"set_scale", {"scale", NULL}},
        {"set_rotation", {"rotation", NULL}}, {"set_offset", {"offset_x", "offset_y"}},
        {"reset_visuals", {NULL, NULL}},
    };
    for (guint i = 0; i < G_N_ELEMENTS(visuals); i++) {
        if (!g_str_equal(name, visuals[i].method))
            continue;
        int count = visuals[i].fields[1] ? 2 : visuals[i].fields[0] ? 1 : 0;
        if (supplied != count)
            return luaL_error(state, "window:%s takes %d number%s", name, count,
                              count == 1 ? "" : "s");
        JsonObject* arguments = json_object_new();
        lua_cli_set_window_id(arguments, id);
        for (int field = 0; field < count; field++) {
            if (lua_type(state, 2 + field) != LUA_TNUMBER) {
                json_object_unref(arguments);
                return luaL_error(state, "window:%s takes numbers", name);
            }
            json_object_set_double_member(arguments, visuals[i].fields[field],
                                          lua_tonumber(state, 2 + field));
        }
        if (count == 0)
            json_object_set_boolean_member(arguments, "reset", TRUE);
        g_autoptr(GError) call_error = NULL;
        g_autoptr(JsonNode) result =
            call_compositor(cli, "api", "window.set_visual", arguments, &call_error);
        json_object_unref(arguments);
        if (!result)
            return luaL_error(state, "window:%s failed: %s", name, call_error->message);
        json_to_lua(state, result);
        /* The compositor answers with offset_x and offset_y. A Window reads its offset as {x, y},
         * so the result says it the same way, as in the runtime. */
        lua_createtable(state, 0, 2);
        lua_getfield(state, -2, "offset_x");
        lua_setfield(state, -2, "x");
        lua_getfield(state, -2, "offset_y");
        lua_setfield(state, -2, "y");
        lua_setfield(state, -2, "offset");
        lua_pushnil(state);
        lua_setfield(state, -2, "offset_x");
        lua_pushnil(state);
        lua_setfield(state, -2, "offset_y");
        return 1;
    }
    return luaL_error(state, "unknown window visual method %s", name);
}

void lua_cli_push_window_record(lua_State* state, Cli* cli, JsonObject* object) {
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_cli_add_record_methods(state, cli, "Window");
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_WINDOW_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static LuaCliSnapContext* lua_cli_test_snap_context(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_SNAP_CONTEXT_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliSnapContext))
        return NULL;
    LuaCliSnapContext* context = lua_touserdata(state, index);
    return context->magic == LUA_CLI_SNAP_CONTEXT_MAGIC ? context : NULL;
}

static int lua_cli_snap_context_commit(lua_State* state) {
    LuaCliSnapContext* context = lua_cli_test_snap_context(state, 1);
    if (!context)
        return luaL_error(state, "SnapContext:commit requires a Gnoblin SnapContext");
    if (lua_gettop(state) != 6)
        return luaL_error(state, "SnapContext:commit takes a monitor (an Output or its ID), x, y, "
                                 "width and height");
    if (context->consumed)
        return luaL_error(state, "SnapContext:commit cannot reuse a consumed context");
    if (!context->subscription || !context->subscription->active ||
        !context->subscription->connection)
        return luaL_error(state, "SnapContext:commit requires its active event connection");

    const char* monitor_id = lua_cli_monitor_argument(state, 2);
    if (!monitor_id || strlen(monitor_id) > 128 || !g_utf8_validate(monitor_id, -1, NULL))
        return luaL_error(state, "SnapContext:commit takes an Output or a monitor ID");

    static const char* const rect_keys[] = {"x", "y", "width", "height"};
    static const gint64 lows[] = {-100000, -100000, 1, 1};
    static const gint64 highs[] = {100000, 100000, 32768, 32768};
    g_autoptr(JsonObject) frame = json_object_new();
    for (guint i = 0; i < G_N_ELEMENTS(rect_keys); i++) {
        int argument = 3 + (int)i;
        if (!lua_isinteger(state, argument) || lua_tointeger(state, argument) < lows[i] ||
            lua_tointeger(state, argument) > highs[i])
            return luaL_error(state, "SnapContext:commit %s must be an integer from %d to %d",
                              rect_keys[i], (int)lows[i], (int)highs[i]);
        json_object_set_int_member(frame, rect_keys[i], lua_tointeger(state, argument));
    }

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "context", context->token);
    json_object_set_string_member(arguments, "monitor_id", monitor_id);
    JsonNode* frame_node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(frame_node, frame);
    json_object_set_member(arguments, "frame", frame_node);

    context->consumed = TRUE;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        context->subscription, "window.snap", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "window.snap failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    if (!response || !member_string(response, "window_id", NULL) ||
        !member_string(response, "monitor_id", NULL) ||
        !json_object_get_boolean_member_with_default(response, "committed", FALSE))
        return luaL_error(state, "window.snap returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_snap_context_gc(lua_State* state) {
    LuaCliSnapContext* context = lua_cli_test_snap_context(state, 1);
    if (!context)
        return 0;
    g_clear_pointer(&context->token, g_free);
    context->subscription = NULL;
    context->magic = 0;
    return 0;
}

int lua_cli_snap_context_newindex(lua_State* state) {
    return luaL_error(state, "SnapContext values are read-only");
}

int lua_cli_snap_context_tostring(lua_State* state) {
    if (!lua_cli_test_snap_context(state, 1)) {
        lua_pushliteral(state, "SnapContext<invalid>");
        return 1;
    }
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "window_id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "SnapContext<%s>", id ? id : "unknown");
    return 1;
}

static gboolean lua_cli_snap_rect_valid(JsonObject* rectangle) {
    static const char* const fields[] = {"x", "y", "width", "height", NULL};
    if (!rectangle || json_object_get_size(rectangle) != 4 ||
        !lua_cli_workspace_object_has_only_keys(rectangle, fields))
        return FALSE;
    const char* keys[] = {"x", "y", "width", "height"};
    for (guint i = 0; i < G_N_ELEMENTS(keys); i++) {
        JsonNode* value = json_object_get_member(rectangle, keys[i]);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            (json_node_get_value_type(value) != G_TYPE_INT &&
             json_node_get_value_type(value) != G_TYPE_INT64) ||
            (i >= 2 && json_node_get_int(value) <= 0))
            return FALSE;
    }
    return TRUE;
}

static gboolean lua_cli_token_valid(const char* token) {
    if (!token || strlen(token) != 64)
        return FALSE;
    for (guint i = 0; i < 64; i++)
        if (!g_ascii_isxdigit(token[i]))
            return FALSE;
    return TRUE;
}

void lua_cli_push_snap_context(lua_State* state, CliLuaEventSubscription* subscription,
                               JsonObject* response) {
    if (!member_string(response, "window_id", NULL) ||
        !member_string(response, "monitor_id", NULL) ||
        !lua_cli_snap_rect_valid(member_object(response, "monitor")) ||
        !lua_cli_snap_rect_valid(member_object(response, "work_area"))) {
        luaL_error(state, "window.snap_context returned an invalid result");
        return;
    }
    const char* token = member_string(response, "context", NULL);
    if (!lua_cli_token_valid(token)) {
        luaL_error(state, "window.snap_context returned an invalid context token");
        return;
    }

    JsonObject* public_fields = json_object_new();
    GList* members = json_object_get_members(response);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "context") && !g_str_equal(key, "expires_at_us"))
            json_object_set_member(public_fields, key,
                                   json_node_copy(json_object_get_member(response, key)));
    }
    g_list_free(members);
    g_autoptr(JsonNode) public_node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(public_node, public_fields);
    json_to_lua(state, public_node);
    int backing = lua_absindex(state, -1);

    LuaCliSnapContext* context = lua_newuserdatauv(state, sizeof *context, 2);
    context->magic = LUA_CLI_SNAP_CONTEXT_MAGIC;
    context->subscription = subscription;
    context->token = g_strdup(token);
    context->consumed = FALSE;
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, -2, 1);
    lua_newtable(state);
    lua_pushcfunction(state, lua_cli_snap_context_commit);
    lua_setfield(state, -2, "commit");
    lua_setiuservalue(state, -2, 2);
    luaL_getmetatable(state, GNOBLINCTL_SNAP_CONTEXT_METATABLE);
    lua_setmetatable(state, -2);
    lua_remove(state, backing);
}

static LuaCliTextTarget* lua_cli_test_text_target(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_TEXT_TARGET_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliTextTarget))
        return NULL;
    LuaCliTextTarget* target = lua_touserdata(state, index);
    return target->magic == LUA_CLI_TEXT_TARGET_MAGIC ? target : NULL;
}

static int lua_cli_text_target_insert_text(lua_State* state) {
    LuaCliTextTarget* target = lua_cli_test_text_target(state, 1);
    if (!target)
        return luaL_error(state, "TextTarget:insert_text requires a Gnoblin TextTarget");
    if (lua_gettop(state) != 2 || lua_type(state, 2) != LUA_TSTRING)
        return luaL_error(state, "TextTarget:insert_text takes exactly one string");
    if (target->consumed)
        return luaL_error(state, "TextTarget is consumed");
    if (!target->subscription || !target->subscription->active ||
        !target->subscription->connection) {
        target->consumed = TRUE;
        return luaL_error(state, "TextTarget:insert_text requires its active event connection");
    }

    size_t length = 0;
    const char* text = lua_tolstring(state, 2, &length);
    gboolean valid = text && length > 0 && length <= 256 && !memchr(text, '\0', length) &&
                     g_utf8_validate(text, length, NULL);
    if (valid) {
        const char* cursor = text;
        const char* end = text + length;
        while (cursor < end) {
            if (g_unichar_iscntrl(g_utf8_get_char(cursor))) {
                valid = FALSE;
                break;
            }
            cursor = g_utf8_next_char(cursor);
        }
    }
    target->consumed = TRUE;
    if (!valid)
        return luaL_error(state,
                          "inserted text must be 1-256 bytes of UTF-8 without control characters");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "target", target->token);
    json_object_set_string_member(arguments, "text", text);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        target->subscription, "input.insert_text", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "input.insert_text failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    JsonObject* response = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    JsonNode* inserted = response ? json_object_get_member(response, "inserted") : NULL;
    if (!inserted || !JSON_NODE_HOLDS_VALUE(inserted) ||
        json_node_get_value_type(inserted) != G_TYPE_BOOLEAN || !json_node_get_boolean(inserted))
        return luaL_error(state, "input.insert_text returned an invalid result");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_text_target_gc(lua_State* state) {
    LuaCliTextTarget* target = lua_cli_test_text_target(state, 1);
    if (!target)
        return 0;
    g_clear_pointer(&target->token, g_free);
    target->subscription = NULL;
    target->magic = 0;
    return 0;
}

int lua_cli_text_target_newindex(lua_State* state) {
    return luaL_error(state, "TextTarget values are read-only");
}

int lua_cli_text_target_tostring(lua_State* state) {
    if (!lua_cli_test_text_target(state, 1)) {
        lua_pushliteral(state, "TextTarget<invalid>");
        return 1;
    }
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "window_id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "TextTarget<%s>", id ? id : "unknown");
    return 1;
}

static gboolean lua_cli_text_caret_valid(JsonObject* caret) {
    static const char* const fields[] = {"x", "y", "width", "height", NULL};
    if (!caret || json_object_get_size(caret) != 4 ||
        !lua_cli_workspace_object_has_only_keys(caret, fields))
        return FALSE;
    const char* keys[] = {"x", "y", "width", "height"};
    for (guint i = 0; i < G_N_ELEMENTS(keys); i++) {
        JsonNode* value = json_object_get_member(caret, keys[i]);
        if (!value || !JSON_NODE_HOLDS_VALUE(value))
            return FALSE;
        GType type = json_node_get_value_type(value);
        if (type != G_TYPE_INT && type != G_TYPE_INT64 && type != G_TYPE_DOUBLE)
            return FALSE;
        double number = json_node_get_double(value);
        if (!isfinite(number) || (i >= 2 && number < 0))
            return FALSE;
    }
    return TRUE;
}

static void lua_cli_push_text_target(lua_State* state, CliLuaEventSubscription* subscription,
                                     JsonObject* response) {
    static const char* const response_fields[] = {"target", "window_id", "caret", NULL};
    const char* token = member_string(response, "target", NULL);
    const char* window_id = member_string(response, "window_id", NULL);
    JsonNode* window_id_node = json_object_get_member(response, "window_id");
    JsonNode* caret_node = json_object_get_member(response, "caret");
    JsonObject* caret = member_object(response, "caret");
    if (!lua_cli_workspace_object_has_only_keys(response, response_fields) ||
        !lua_cli_token_valid(token) ||
        (window_id_node &&
         (!JSON_NODE_HOLDS_VALUE(window_id_node) ||
          json_node_get_value_type(window_id_node) != G_TYPE_STRING || !window_id || !*window_id ||
          strlen(window_id) >= 128 || !g_utf8_validate(window_id, -1, NULL))) ||
        (caret_node && (!JSON_NODE_HOLDS_OBJECT(caret_node) || !lua_cli_text_caret_valid(caret)))) {
        luaL_error(state, "input.text_target returned an invalid TextTarget");
        return;
    }

    JsonObject* public_fields = json_object_new();
    if (window_id)
        json_object_set_string_member(public_fields, "window_id", window_id);
    if (caret_node)
        json_object_set_member(public_fields, "caret", json_node_copy(caret_node));
    g_autoptr(JsonNode) public_node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(public_node, public_fields);
    json_to_lua(state, public_node);
    int backing = lua_absindex(state, -1);

    LuaCliTextTarget* target = lua_newuserdatauv(state, sizeof *target, 2);
    target->magic = LUA_CLI_TEXT_TARGET_MAGIC;
    target->subscription = subscription;
    target->token = g_strdup(token);
    target->consumed = FALSE;
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, -2, 1);
    lua_newtable(state);
    lua_pushcfunction(state, lua_cli_text_target_insert_text);
    lua_setfield(state, -2, "insert_text");
    lua_setiuservalue(state, -2, 2);
    luaL_getmetatable(state, GNOBLINCTL_TEXT_TARGET_METATABLE);
    lua_setmetatable(state, -2);
    lua_remove(state, backing);
}

int lua_cli_input_text_target(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1)
        return luaL_error(state, "gnoblin.input.text_target takes one Grant");
    CliLuaEventSubscription* subscription = NULL;
    g_autoptr(JsonNode) result = lua_cli_grant_request(
        state, cli, 1, LUA_CLI_GRANT_TEXT_TARGET, "input.text_target", NULL, NULL, &subscription);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "input.text_target returned an invalid result");
    lua_cli_push_text_target(state, subscription, json_node_get_object(result));
    return 1;
}
