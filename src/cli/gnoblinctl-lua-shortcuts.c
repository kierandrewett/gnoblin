#include "gnoblinctl-internal.h"

int lua_cli_workspace_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Workspace<%s>", id ? id : "unknown");
    return 1;
}

static int lua_cli_workspace_method(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    int supplied = lua_gettop(state) - 1;
    g_autofree char* id = g_strdup(lua_cli_workspace_id(state));
    lua_settop(state, supplied + 1);

    JsonObject* arguments = json_object_new();
    if (g_str_equal(method, "workspace.switch") || g_str_equal(method, "workspace.remove")) {
        if (supplied != 0) {
            json_object_unref(arguments);
            return luaL_error(state, "%s takes no arguments", method);
        }
        json_object_set_string_member(arguments, "id", id);
    } else if (g_str_equal(method, "workspace.rename")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TSTRING || !*lua_tostring(state, 2) ||
            strlen(lua_tostring(state, 2)) > 80) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "workspace.rename requires a nonempty name up to 80 characters");
        }
        json_object_set_string_member(arguments, "id", id);
        json_object_set_string_member(arguments, "name", lua_tostring(state, 2));
    } else if (g_str_equal(method, "workspace.move_window")) {
        if (supplied < 1 || supplied > 2 ||
            (supplied == 2 && !lua_isnil(state, 3) && !lua_istable(state, 3))) {
            json_object_unref(arguments);
            return luaL_error(state,
                              "workspace.move_window requires a window and optional options");
        }
        g_autofree char* window = NULL;
        if (lua_type(state, 2) == LUA_TSTRING)
            window = g_strdup(lua_tostring(state, 2));
        else if (luaL_testudata(state, 2, GNOBLINCTL_WINDOW_RECORD_METATABLE)) {
            lua_getiuservalue(state, 2, 1);
            lua_getfield(state, -1, "id");
            if (lua_isstring(state, -1))
                window = g_strdup(lua_tostring(state, -1));
            lua_pop(state, 2);
        }
        if (!window || !*window) {
            json_object_unref(arguments);
            return luaL_error(state, "workspace.move_window requires a Window record or window ID");
        }
        json_object_set_string_member(arguments, "window", window);
        JsonObject* workspace = json_object_new();
        json_object_set_string_member(workspace, "id", id);
        JsonNode* workspace_node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(workspace_node, workspace);
        json_object_set_member(arguments, "workspace", workspace_node);
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
    } else {
        json_object_unref(arguments);
        return luaL_error(state, "unsupported Workspace method %s", method);
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    json_to_lua(state, result);
    return 1;
}

void lua_cli_push_workspace_record(lua_State* state, Cli* cli, JsonObject* object) {
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
        {"activate", "workspace.switch"},
        {"rename", "workspace.rename"},
        {"remove", "workspace.remove"},
        {"move_here", "workspace.move_window"},
        {NULL, NULL},
    };
    for (guint i = 0; methods[i][0]; i++) {
        lua_pushlightuserdata(state, cli);
        lua_pushstring(state, methods[i][1]);
        lua_pushcclosure(state, lua_cli_workspace_method, 2);
        lua_setfield(state, -2, methods[i][0]);
    }
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_WORKSPACE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

int lua_cli_monitor_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "Monitor<%s>", id ? id : "unknown");
    return 1;
}

void lua_cli_push_monitor_record(lua_State* state, JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    if (!id || !*id)
        luaL_error(state, "gnoblin.monitors returned a monitor without a stable id");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_MONITOR_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

int lua_cli_layer_surface_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushfstring(state, "LayerSurface<%s>", id ? id : "unknown");
    return 1;
}

void lua_cli_push_layer_surface_record(lua_State* state, JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    if (!id || !*id)
        luaL_error(state, "gnoblin.layers.list returned a layer surface without a stable id");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, GNOBLINCTL_LAYER_SURFACE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static gboolean lua_cli_input_source_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "type", "short_name", "name"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* current = json_object_get_member(object, "current");
    return current && JSON_NODE_HOLDS_VALUE(current) &&
           json_node_get_value_type(current) == G_TYPE_BOOLEAN &&
           *member_string(object, "id", "") &&
           (g_str_equal(member_string(object, "type", ""), "xkb") ||
            g_str_equal(member_string(object, "type", ""), "ibus"));
}

static gboolean lua_cli_input_device_valid(JsonObject* object) {
    static const char* const string_fields[] = {"id", "name", "device_type"};
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        JsonNode* field = json_object_get_member(object, string_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_STRING)
            return FALSE;
    }
    JsonNode* capabilities = json_object_get_member(object, "capabilities");
    if (!capabilities || !JSON_NODE_HOLDS_ARRAY(capabilities) || !*member_string(object, "id", ""))
        return FALSE;
    JsonArray* capability_array = json_node_get_array(capabilities);
    for (guint i = 0; i < json_array_get_length(capability_array); i++) {
        JsonNode* capability = json_array_get_element(capability_array, i);
        if (!capability || !JSON_NODE_HOLDS_VALUE(capability) ||
            json_node_get_value_type(capability) != G_TYPE_STRING)
            return FALSE;
    }
    return TRUE;
}

void lua_cli_push_input_record(lua_State* state, JsonObject* object, gint64 revision,
                               gboolean is_device) {
    gboolean valid =
        is_device ? lua_cli_input_device_valid(object) : lua_cli_input_source_valid(object);
    if (!valid)
        luaL_error(state, "input snapshot returned an invalid %s",
                   is_device ? "InputDevice" : "InputSource");

    JsonObject* public_fields = json_object_new();
    GList* members = json_object_get_members(object);
    for (GList* item = members; item; item = item->next) {
        const char* key = item->data;
        if (!g_str_equal(key, "revision"))
            json_object_set_member(public_fields, key,
                                   json_node_copy(json_object_get_member(object, key)));
    }
    g_list_free(members);
    json_object_set_int_member(public_fields, "revision", revision);
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, public_fields);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 2);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    lua_newtable(state);
    lua_setiuservalue(state, record, 2);
    luaL_getmetatable(state, is_device ? GNOBLINCTL_INPUT_DEVICE_RECORD_METATABLE
                                       : GNOBLINCTL_INPUT_SOURCE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static void lua_cli_push_shortcut_record(lua_State* state, JsonObject* object, gboolean is_action) {
    const char* identity = member_string(object, is_action ? "id" : "name", NULL);
    if (!identity || !*identity)
        luaL_error(state, "gnoblin.shortcuts.%s returned a record without a stable %s",
                   is_action ? "actions" : "list", is_action ? "id" : "name");

    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_set_object(node, object);
    json_to_lua(state, node);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, is_action ? GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE
                                       : GNOBLINCTL_SHORTCUT_STATE_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
}

static int lua_cli_shortcut_record_tostring(lua_State* state) {
    gboolean is_action =
        luaL_testudata(state, 1, GNOBLINCTL_SHORTCUT_ACTION_RECORD_METATABLE) != NULL;
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, is_action ? "id" : "name");
    const char* identity = lua_tostring(state, -1);
    lua_pushfstring(state, "%s<%s>", is_action ? "ShortcutAction" : "ShortcutState",
                    identity ? identity : "unknown");
    return 1;
}

static int lua_cli_shortcut_snapshot(lua_State* state, gboolean is_action) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    if (is_action && lua_gettop(state) == 1)
        json_object_set_string_member(arguments, "group", lua_tostring(state, 1));
    const char* method = is_action ? "shortcuts.actions" : "shortcuts.list";
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.%s failed: %s", method, call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.%s returned an invalid snapshot", method);

    JsonArray* entries = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(entries), 0);
    for (guint i = 0; i < json_array_get_length(entries); i++) {
        JsonObject* entry = json_array_get_object_element(entries, i);
        if (!entry)
            return luaL_error(state, "gnoblin.%s returned an invalid record", method);
        lua_cli_push_shortcut_record(state, entry, is_action);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_shortcuts_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
    return lua_cli_shortcut_snapshot(state, FALSE);
}

int lua_cli_shortcuts_actions(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && lua_type(state, 1) != LUA_TSTRING))
        return luaL_error(state, "gnoblin.shortcuts.actions accepts an optional group string");
    return lua_cli_shortcut_snapshot(state, TRUE);
}

int lua_cli_shortcuts_capture(lua_State* state) {
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.shortcuts.capture accepts one optional options table");

    gint timeout_seconds = 30;
    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        json_object_unref(arguments);
        arguments = lua_cli_table_object(state, 1, "shortcut capture options");
        if (json_object_get_size(arguments) > 1 ||
            (json_object_get_size(arguments) == 1 &&
             !json_object_has_member(arguments, "timeout"))) {
            json_object_unref(arguments);
            return luaL_error(state, "gnoblin.shortcuts.capture accepts only timeout");
        }
        JsonNode* timeout = json_object_get_member(arguments, "timeout");
        if (timeout) {
            if (!JSON_NODE_HOLDS_VALUE(timeout) ||
                (json_node_get_value_type(timeout) != G_TYPE_INT &&
                 json_node_get_value_type(timeout) != G_TYPE_INT64)) {
                json_object_unref(arguments);
                return luaL_error(state,
                                  "shortcut capture timeout must be an integer from 1 to 60");
            }
            timeout_seconds = (gint)json_node_get_int(timeout);
            if (timeout_seconds < 1 || timeout_seconds > 60) {
                json_object_unref(arguments);
                return luaL_error(state,
                                  "shortcut capture timeout must be an integer from 1 to 60");
            }
        }
    }

    Cli capture_cli = *(Cli*)lua_touserdata(state, lua_upvalueindex(1));
    capture_cli.timeout = (guint)timeout_seconds;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(&capture_cli, "api", "shortcut.capture", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.shortcuts.capture failed: %s", call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    const char* accelerator = member_string(object, "accelerator", NULL);
    if (!accelerator || !*accelerator)
        return luaL_error(state, "gnoblin.shortcuts.capture returned an invalid result");

    json_to_lua(state, result);
    return 1;
}

static JsonObject* lua_cli_shortcut_owner_call(Cli* cli, const char* method, JsonObject* arguments,
                                               GError** error) {
    if (!cli->lua_shortcut_owner && !cli_lua_shortcut_owner_ensure(cli, error))
        return NULL;
    g_autoptr(JsonNode) result =
        cli_lua_call_compositor_on_subscription(cli->lua_shortcut_owner, method, arguments, error);
    if (!result || !JSON_NODE_HOLDS_OBJECT(result)) {
        if (result)
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA, "%s returned an invalid result",
                        method);
        return NULL;
    }
    return json_object_ref(json_node_get_object(result));
}

static void lua_cli_push_readonly_json_object(lua_State* state, JsonObject* object) {
    g_autoptr(JsonNode) node = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(node, json_object_ref(object));
    json_to_lua(state, node);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
}

int lua_cli_shortcuts_bind(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.shortcuts.bind requires one options table");
    static const char* const allowed[] = {"id",   "accelerator",   "hold", "trigger",
                                          "mode", "capture_input", NULL};
    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "shortcut binding options");
    if (!lua_cli_workspace_object_has_only_keys(arguments, allowed))
        return luaL_error(state, "gnoblin.shortcuts.bind accepts only id, accelerator, hold, "
                                 "trigger, mode, and capture_input");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(GError) error = NULL;
    if (!cli_lua_shortcut_owner_ensure(cli, &error))
        return luaL_error(state, "gnoblin.shortcuts.bind could not create its owner connection: %s",
                          error ? error->message : "unknown error");
    g_autoptr(JsonObject) result =
        lua_cli_shortcut_owner_call(cli, "shortcut.bind", arguments, &error);
    if (!result)
        return luaL_error(state, "gnoblin.shortcuts.bind failed: %s",
                          error ? error->message : "invalid result");
    const char* id = member_string(result, "id", NULL);
    const char* accelerator = member_string(result, "accelerator", NULL);
    if (!id || !*id || !accelerator || !*accelerator)
        return luaL_error(state, "gnoblin.shortcuts.bind returned an invalid binding");
    if (!cli->lua_shortcut_bindings)
        cli->lua_shortcut_bindings = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_hash_table_add(cli->lua_shortcut_bindings, g_strdup(id));
    lua_cli_push_readonly_json_object(state, result);
    return 1;
}

int lua_cli_shortcuts_unbind(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.shortcuts.unbind requires one {id} table");
    static const char* const allowed[] = {"id", NULL};
    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "shortcut unbind options");
    if (!lua_cli_workspace_object_has_only_keys(arguments, allowed))
        return luaL_error(state, "gnoblin.shortcuts.unbind accepts only id");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (!cli->lua_shortcut_owner)
        return luaL_error(state, "gnoblin.shortcuts.unbind has no active owner connection");
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonObject) result =
        lua_cli_shortcut_owner_call(cli, "shortcut.unbind", arguments, &error);
    if (!result)
        return luaL_error(state, "gnoblin.shortcuts.unbind failed: %s",
                          error ? error->message : "invalid result");
    const char* id = member_string(result, "id", NULL);
    if (!id || !json_object_get_boolean_member_with_default(result, "unbound", FALSE))
        return luaL_error(state, "gnoblin.shortcuts.unbind returned an invalid result");
    if (cli->lua_shortcut_bindings)
        g_hash_table_remove(cli->lua_shortcut_bindings, id);
    lua_cli_push_readonly_json_object(state, result);
    return 1;
}

int lua_cli_shortcuts_end_session(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state,
                          "gnoblin.shortcuts.end_session requires one {id, session_id} table");
    static const char* const allowed[] = {"id", "session_id", NULL};
    g_autoptr(JsonObject) arguments = lua_cli_table_object(state, 1, "shortcut session options");
    if (!lua_cli_workspace_object_has_only_keys(arguments, allowed))
        return luaL_error(state, "gnoblin.shortcuts.end_session accepts only id and session_id");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (!cli->lua_shortcut_owner)
        return luaL_error(state, "gnoblin.shortcuts.end_session has no active owner connection");
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonObject) result =
        lua_cli_shortcut_owner_call(cli, "shortcut.session.end", arguments, &error);
    if (!result)
        return luaL_error(state, "gnoblin.shortcuts.end_session failed: %s",
                          error ? error->message : "invalid result");
    const char* id = member_string(result, "id", NULL);
    if (!id || !json_object_get_boolean_member_with_default(result, "ended", FALSE) ||
        json_object_get_int_member_with_default(result, "session_id", 0) <= 0)
        return luaL_error(state, "gnoblin.shortcuts.end_session returned an invalid result");
    lua_cli_push_readonly_json_object(state, result);
    return 1;
}

void register_lua_cli_shortcut_record(lua_State* state, const char* metatable) {
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
    lua_pushcfunction(state, lua_cli_shortcut_record_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

gboolean lua_cli_focus_policy_valid(JsonObject* object) {
    const char* focus_mode = member_string(object, "focus_mode", NULL);
    const char* focus_new_windows = member_string(object, "focus_new_windows", NULL);
    if ((!focus_mode ||
         (!g_str_equal(focus_mode, "click") && !g_str_equal(focus_mode, "hover") &&
          !g_str_equal(focus_mode, "hover-strict") && !g_str_equal(focus_mode, "sloppy") &&
          !g_str_equal(focus_mode, "mouse"))) ||
        (!focus_new_windows ||
         (!g_str_equal(focus_new_windows, "prevent") && !g_str_equal(focus_new_windows, "allow") &&
          !g_str_equal(focus_new_windows, "strict") && !g_str_equal(focus_new_windows, "smart"))))
        return FALSE;

    static const char* const boolean_fields[] = {"raise_on_click", "auto_raise",
                                                 "focus_change_on_pointer_rest"};
    for (guint i = 0; i < G_N_ELEMENTS(boolean_fields); i++) {
        JsonNode* field = json_object_get_member(object, boolean_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            json_node_get_value_type(field) != G_TYPE_BOOLEAN)
            return FALSE;
    }
    const char* const integer_fields[] = {"auto_raise_delay", "revision"};
    for (guint i = 0; i < G_N_ELEMENTS(integer_fields); i++) {
        JsonNode* field = json_object_get_member(object, integer_fields[i]);
        if (!field || !JSON_NODE_HOLDS_VALUE(field) ||
            (json_node_get_value_type(field) != G_TYPE_INT64 &&
             json_node_get_value_type(field) != G_TYPE_INT))
            return FALSE;
    }
    gint64 delay = json_object_get_int_member(object, "auto_raise_delay");
    gint64 revision = json_object_get_int_member(object, "revision");
    return delay >= 0 && delay <= 10000 && revision >= 0;
}

static int lua_cli_focus_policy_tostring(lua_State* state) {
    lua_pushliteral(state, "FocusPolicy");
    return 1;
}

void register_lua_cli_focus_policy_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_FOCUS_POLICY_RECORD_METATABLE)) {
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
    lua_pushcfunction(state, lua_cli_focus_policy_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

static int lua_cli_settings_tostring(lua_State* state) {
    lua_pushliteral(state, "Settings");
    return 1;
}

void register_lua_cli_settings_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_SETTINGS_RECORD_METATABLE)) {
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
    lua_pushcfunction(state, lua_cli_settings_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

int lua_cli_settings_property(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "settings", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.settings failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "gnoblin.settings returned an invalid snapshot");

    JsonObject* object = json_node_get_object(result);
    JsonNode* revision = json_object_get_member(object, "revision");
    if (!revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_INT) ||
        json_node_get_int(revision) < 0)
        return luaL_error(state, "gnoblin.settings returned an invalid Settings revision");

    json_to_lua(state, result);
    int backing = lua_absindex(state, -1);
    lua_newuserdatauv(state, 1, 1);
    int record = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, record, 1);
    luaL_getmetatable(state, GNOBLINCTL_SETTINGS_RECORD_METATABLE);
    lua_setmetatable(state, record);
    lua_remove(state, backing);
    return 1;
}
