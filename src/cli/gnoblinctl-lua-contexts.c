#include "gnoblinctl-internal.h"

static void lua_cli_set_window_id(JsonObject* arguments, const char* id) {
    json_object_set_string_member(arguments, "id", id);
}

LuaCliFocusContext* lua_cli_test_focus_context(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_FOCUS_CONTEXT_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliFocusContext))
        return NULL;
    LuaCliFocusContext* context = lua_touserdata(state, index);
    return context->magic == LUA_CLI_FOCUS_CONTEXT_MAGIC ? context : NULL;
}

LuaCliMenuContext* lua_cli_test_menu_context(lua_State* state, int index) {
    if (!luaL_testudata(state, index, GNOBLINCTL_MENU_CONTEXT_METATABLE) ||
        lua_rawlen(state, index) != sizeof(LuaCliMenuContext))
        return NULL;
    LuaCliMenuContext* context = lua_touserdata(state, index);
    return context->magic == LUA_CLI_MENU_CONTEXT_MAGIC && context->subscription && context->token
               ? context
               : NULL;
}

static gboolean lua_cli_resize_edge_valid(const char* edge) {
    return edge && (g_str_equal(edge, "north") || g_str_equal(edge, "south") ||
                    g_str_equal(edge, "east") || g_str_equal(edge, "west") ||
                    g_str_equal(edge, "north_east") || g_str_equal(edge, "north_west") ||
                    g_str_equal(edge, "south_east") || g_str_equal(edge, "south_west"));
}

static int lua_cli_menu_context_begin(lua_State* state, gboolean resize) {
    LuaCliMenuContext* context = lua_cli_test_menu_context(state, 1);
    if (!context)
        return luaL_error(state, "MenuContext method called on a forged or invalid value");
    if (lua_gettop(state) != (resize ? 2 : 1))
        return luaL_error(state, "MenuContext:%s takes %s", resize ? "begin_resize" : "begin_move",
                          resize ? "one ResizeEdge" : "no arguments");
    if (context->consumed)
        return luaL_error(state, "MenuContext has already been consumed");
    CliLuaEventSubscription* subscription = context->subscription;
    if (!subscription || !subscription->active || !subscription->connection || !subscription->cli)
        return luaL_error(state, "MenuContext requires its active event subscription");

    const char* edge = NULL;
    if (resize) {
        size_t edge_length = 0;
        edge = lua_type(state, 2) == LUA_TSTRING ? lua_tolstring(state, 2, &edge_length) : NULL;
        if (!lua_cli_resize_edge_valid(edge) || edge_length != strlen(edge))
            return luaL_error(state, "MenuContext:begin_resize requires a valid ResizeEdge");
    }

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "menu_context", context->token);
    if (resize)
        json_object_set_string_member(arguments, "edge", edge);
    context->consumed = TRUE;

    GError* error = NULL;
    JsonNode* result = cli_lua_call_compositor_direct_on_subscription(
        subscription, resize ? "window.begin_resize" : "window.begin_move", arguments, &error);
    json_object_unref(arguments);
    if (!result) {
        char message[512];
        g_snprintf(message, sizeof message, "%s",
                   error ? error->message : "unknown compositor error");
        g_clear_error(&error);
        return luaL_error(state, "MenuContext operation failed: %s", message);
    }

    json_to_lua(state, result);
    json_node_unref(result);
    return 1;
}

int lua_cli_menu_context_begin_move(lua_State* state) {
    return lua_cli_menu_context_begin(state, FALSE);
}

int lua_cli_menu_context_begin_resize(lua_State* state) {
    return lua_cli_menu_context_begin(state, TRUE);
}

static int lua_cli_window_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_window_id(state));
    lua_settop(state, supplied + 1);

    gboolean uses_focus_context = g_str_equal(method, "window.focus") ||
                                  g_str_equal(method, "window.begin_move") ||
                                  g_str_equal(method, "window.begin_resize");
    LuaCliFocusContext* focus_context = NULL;
    if (uses_focus_context) {
        int context_index = g_str_equal(method, "window.begin_resize") ? 3 : 2;
        int expected_arguments = g_str_equal(method, "window.begin_resize") ? 2 : 1;
        if (supplied != expected_arguments) {
            return luaL_error(state, "%s requires %sFocusContext as its final argument", method,
                              g_str_equal(method, "window.begin_resize") ? "an edge and a " : "a ");
        }
        focus_context = lua_cli_test_focus_context(state, context_index);
        if (!focus_context)
            return luaL_error(state, "%s requires a FocusContext received from a Gnoblin event",
                              method);
        if (focus_context->consumed)
            return luaL_error(state, "%s cannot reuse a consumed FocusContext", method);
        if (!focus_context->subscription || !focus_context->subscription->active ||
            !focus_context->subscription->connection || focus_context->subscription->cli != cli)
            return luaL_error(state, "%s requires a FocusContext from an active event subscription",
                              method);
    }

    JsonObject* arguments = json_object_new();
    lua_cli_set_window_id(arguments, id);
    if (uses_focus_context) {
        json_object_set_string_member(arguments, "focus_context", focus_context->token);
        if (g_str_equal(method, "window.begin_resize")) {
            const char* edge = lua_type(state, 2) == LUA_TSTRING ? lua_tostring(state, 2) : NULL;
            if (!lua_cli_resize_edge_valid(edge)) {
                json_object_unref(arguments);
                return luaL_error(state, "window.begin_resize requires a valid ResizeEdge");
            }
            json_object_set_string_member(arguments, "edge", edge);
        }
    } else if (g_str_equal(method, "window.close") || g_str_equal(method, "window.kill") ||
               g_str_equal(method, "window.raise") || g_str_equal(method, "window.lower") ||
               g_str_equal(method, "window.minimize") ||
               g_str_equal(method, "window.toggle_minimize") ||
               g_str_equal(method, "window.unminimize") || g_str_equal(method, "window.restore") ||
               g_str_equal(method, "window.restore_or_minimize")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method);
        }
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
        if (supplied != 1 || !lua_istable(state, 2)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires one table", method);
        }
        g_autoptr(JsonObject) values = lua_cli_table_object(state, 2, method);
        const char* first = g_str_equal(method, "window.move") ? "x" : "width";
        const char* second = g_str_equal(method, "window.move") ? "y" : "height";
        JsonNode* first_value = json_object_get_member(values, first);
        JsonNode* second_value = json_object_get_member(values, second);
        if (json_object_get_size(values) != 2 || !first_value || !second_value ||
            !JSON_NODE_HOLDS_VALUE(first_value) || !JSON_NODE_HOLDS_VALUE(second_value) ||
            (json_node_get_value_type(first_value) != G_TYPE_INT &&
             json_node_get_value_type(first_value) != G_TYPE_INT64) ||
            (json_node_get_value_type(second_value) != G_TYPE_INT &&
             json_node_get_value_type(second_value) != G_TYPE_INT64)) {
            json_object_unref(arguments);
            return luaL_error(state, "%s requires integer %s and %s", method, first, second);
        }
        json_object_set_member(arguments, first, json_node_copy(first_value));
        json_object_set_member(arguments, second, json_node_copy(second_value));
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        if (supplied < 1 || supplied > 2 || !lua_istable(state, 2) ||
            (supplied == 2 && !lua_isnil(state, 3) && !lua_istable(state, 3))) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "window.move_to_workspace requires a selector and optional options");
        }
        g_autoptr(JsonObject) selector = lua_cli_table_object(state, 2, "workspace selector");
        if (json_object_get_size(selector) != 1 || (!json_object_has_member(selector, "id") &&
                                                    !json_object_has_member(selector, "number"))) {
            json_object_unref(arguments);
            return luaL_error(state, "workspace selector accepts exactly id or number");
        }
        JsonNode* workspace = json_node_new(JSON_NODE_OBJECT);
        json_node_set_object(workspace, selector);
        json_object_set_member(arguments, "workspace", workspace);
        if (supplied == 2 && !lua_isnil(state, 3)) {
            g_autoptr(JsonObject) options = lua_cli_table_object(state, 3, "workspace options");
            JsonNode* follow = json_object_get_member(options, "follow");
            if (json_object_get_size(options) != 1 || !follow || !JSON_NODE_HOLDS_VALUE(follow) ||
                json_node_get_value_type(follow) != G_TYPE_BOOLEAN) {
                json_object_unref(arguments);
                return luaL_error(state, "workspace options accept only boolean follow");
            }
            json_object_set_boolean_member(arguments, "follow", json_node_get_boolean(follow));
        }
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        if (supplied != 1) {
            json_object_unref(arguments);
            return luaL_error(state, "window.move_to_monitor requires a monitor selector");
        }
        g_autofree char* monitor = lua_isstring(state, 2) ? g_strdup(lua_tostring(state, 2)) : NULL;
        if (!monitor && lua_istable(state, 2)) {
            lua_getfield(state, 2, "id");
            if (lua_isstring(state, -1))
                monitor = g_strdup(lua_tostring(state, -1));
            lua_pop(state, 1);
        }
        if (!monitor || !*monitor) {
            json_object_unref(arguments);
            return luaL_error(state, "window.move_to_monitor requires a monitor ID");
        }
        json_object_set_string_member(arguments, "monitor", monitor);
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported Window method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    JsonNode* raw_result;
    if (focus_context) {
        /* The server consumes a focus grant on the first valid request, even
         * when the requested action is denied, so never offer local reuse. */
        focus_context->consumed = TRUE;
        raw_result = cli_lua_call_compositor_on_subscription(focus_context->subscription, method,
                                                             arguments, &call_error);
    } else {
        raw_result = call_compositor(cli, "api", method, arguments, &call_error);
    }
    g_autoptr(JsonNode) result = raw_result;
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    json_to_lua(state, result);
    return 1;
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
    static const char* const methods[][2] = {
        {"close", "window.close"},
        {"kill", "window.kill"},
        {"raise", "window.raise"},
        {"lower", "window.lower"},
        {"minimize", "window.minimize"},
        {"toggle_minimize", "window.toggle_minimize"},
        {"unminimize", "window.unminimize"},
        {"restore", "window.restore"},
        {"restore_or_minimize", "window.restore_or_minimize"},
        {"set_maximized", "window.set_maximized"},
        {"set_fullscreen", "window.set_fullscreen"},
        {"set_above", "window.set_above"},
        {"set_sticky", "window.set_sticky"},
        {"move", "window.move"},
        {"resize", "window.resize"},
        {"move_to_workspace", "window.move_to_workspace"},
        {"move_to_monitor", "window.move_to_monitor"},
        {"focus", "window.focus"},
        {"begin_move", "window.begin_move"},
        {"begin_resize", "window.begin_resize"},
        {"thumbnail", "window.thumbnail"},
        {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_window_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
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
    if (lua_gettop(state) != 2 || !lua_istable(state, 2))
        return luaL_error(state, "SnapContext:commit requires {monitor_id, frame}");
    if (context->consumed)
        return luaL_error(state, "SnapContext:commit cannot reuse a consumed context");
    if (!context->subscription || !context->subscription->active ||
        !context->subscription->connection)
        return luaL_error(state, "SnapContext:commit requires its active event connection");

    g_autoptr(JsonObject) target = lua_cli_table_object(state, 2, "SnapContext target");
    static const char* const target_fields[] = {"monitor_id", "frame", NULL};
    static const char* const frame_fields[] = {"x", "y", "width", "height", NULL};
    if (!lua_cli_workspace_object_has_only_keys(target, target_fields))
        return luaL_error(state, "SnapContext:commit accepts only monitor_id and frame");
    const char* monitor_id = member_string(target, "monitor_id", NULL);
    if (!monitor_id || !*monitor_id || strlen(monitor_id) > 128 ||
        !g_utf8_validate(monitor_id, -1, NULL))
        return luaL_error(state, "SnapContext:commit requires a valid monitor_id");
    JsonObject* frame = member_object(target, "frame");
    if (!frame || !lua_cli_workspace_object_has_only_keys(frame, frame_fields))
        return luaL_error(state, "SnapContext:commit frame requires x, y, width and height");

    const char* rect_keys[] = {"x", "y", "width", "height"};
    const gint64 lows[] = {-100000, -100000, 1, 1};
    const gint64 highs[] = {100000, 100000, 32768, 32768};
    for (guint i = 0; i < G_N_ELEMENTS(rect_keys); i++) {
        JsonNode* value = json_object_get_member(frame, rect_keys[i]);
        if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
            (json_node_get_value_type(value) != G_TYPE_INT &&
             json_node_get_value_type(value) != G_TYPE_INT64) ||
            json_node_get_int(value) < lows[i] || json_node_get_int(value) > highs[i])
            return luaL_error(state, "SnapContext:commit frame %s is out of range", rect_keys[i]);
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
    LuaCliFocusContext* context = lua_cli_test_focus_context(state, 1);
    if (lua_gettop(state) != 1 || !context)
        return luaL_error(state,
                          "gnoblin.input.text_target requires a FocusContext from a Gnoblin event");
    if (context->consumed)
        return luaL_error(state, "gnoblin.input.text_target cannot reuse a consumed FocusContext");
    if (!context->subscription || !context->subscription->active ||
        !context->subscription->connection || context->subscription->cli != cli)
        return luaL_error(state, "gnoblin.input.text_target requires its active event connection");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "focus_context", context->token);
    context->consumed = TRUE;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        context->subscription, "input.text_target", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "input.text_target failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "input.text_target returned an invalid result");
    lua_cli_push_text_target(state, context->subscription, json_node_get_object(result));
    return 1;
}
