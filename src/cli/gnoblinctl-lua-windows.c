#include "gnoblinctl-internal.h"

static JsonNode* lua_cli_monitor_snapshot(lua_State* state, Cli* cli, const char* operation) {
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    JsonNode* result = call_compositor(cli, "api", "monitors.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        luaL_error(state, "gnoblin.monitors.%s failed: %s", operation, call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result)) {
        json_node_unref(result);
        luaL_error(state, "gnoblin.monitors.%s returned an invalid snapshot", operation);
    }
    return result;
}

int lua_cli_monitors_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_monitor_snapshot(state, cli, "list");
    JsonArray* monitors = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(monitors), 0);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonObject* monitor = json_array_get_object_element(monitors, i);
        if (!monitor)
            return luaL_error(state, "gnoblin.monitors.list returned an invalid monitor record");
        lua_cli_push_monitor_record(state, monitor);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_monitors_primary(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.primary takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_monitor_snapshot(state, cli, "primary");
    JsonArray* monitors = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonObject* monitor = json_array_get_object_element(monitors, i);
        if (monitor && json_object_get_boolean_member_with_default(monitor, "primary", FALSE)) {
            lua_cli_push_monitor_record(state, monitor);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static gboolean lua_cli_monitor_privacy_screen_snapshot_valid(JsonObject* snapshot) {
    static const char* const snapshot_fields[] = {"requested_enabled", "source", "revision",
                                                  "monitors", NULL};
    static const char* const monitor_fields[] = {"id", "available", "enabled", "locked", NULL};
    if (!snapshot || !lua_cli_workspace_object_has_only_keys(snapshot, snapshot_fields))
        return FALSE;
    JsonNode* requested = json_object_get_member(snapshot, "requested_enabled");
    JsonNode* source = json_object_get_member(snapshot, "source");
    JsonNode* revision = json_object_get_member(snapshot, "revision");
    JsonNode* monitors_node = json_object_get_member(snapshot, "monitors");
    const char* source_name =
        source && JSON_NODE_HOLDS_VALUE(source) && json_node_get_value_type(source) == G_TYPE_STRING
            ? json_node_get_string(source)
            : NULL;
    if (!requested || !JSON_NODE_HOLDS_VALUE(requested) ||
        json_node_get_value_type(requested) != G_TYPE_BOOLEAN || !source_name ||
        !(g_str_equal(source_name, "runtime") || g_str_equal(source_name, "config") ||
          g_str_equal(source_name, "system")) ||
        !revision || !JSON_NODE_HOLDS_VALUE(revision) ||
        (json_node_get_value_type(revision) != G_TYPE_INT &&
         json_node_get_value_type(revision) != G_TYPE_INT64 &&
         json_node_get_value_type(revision) != G_TYPE_UINT &&
         json_node_get_value_type(revision) != G_TYPE_UINT64) ||
        (json_node_get_value_type(revision) != G_TYPE_UINT &&
         json_node_get_value_type(revision) != G_TYPE_UINT64 && json_node_get_int(revision) < 0) ||
        !monitors_node || !JSON_NODE_HOLDS_ARRAY(monitors_node))
        return FALSE;

    JsonArray* monitors = json_node_get_array(monitors_node);
    for (guint i = 0; i < json_array_get_length(monitors); i++) {
        JsonNode* monitor_node = json_array_get_element(monitors, i);
        if (!monitor_node || !JSON_NODE_HOLDS_OBJECT(monitor_node))
            return FALSE;
        JsonObject* monitor = json_node_get_object(monitor_node);
        if (!lua_cli_workspace_object_has_only_keys(monitor, monitor_fields))
            return FALSE;
        const char* id = member_string(monitor, "id", NULL);
        JsonNode* available = json_object_get_member(monitor, "available");
        if (!id || !*id || !available || !JSON_NODE_HOLDS_VALUE(available) ||
            json_node_get_value_type(available) != G_TYPE_BOOLEAN)
            return FALSE;
        gboolean supports_privacy_screen = json_node_get_boolean(available);
        const char* const state_fields[] = {"enabled", "locked", NULL};
        for (guint field = 0; state_fields[field]; field++) {
            JsonNode* value = json_object_get_member(monitor, state_fields[field]);
            if (supports_privacy_screen) {
                if (!value || !JSON_NODE_HOLDS_VALUE(value) ||
                    json_node_get_value_type(value) != G_TYPE_BOOLEAN)
                    return FALSE;
            } else if (value) {
                return FALSE;
            }
        }
    }
    return TRUE;
}

int lua_cli_monitors_privacy_screen(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.privacy_screen takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "monitors.privacy_screen", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.monitors.privacy_screen failed: %s",
                          call_error ? call_error->message : "unknown compositor error");
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_monitor_privacy_screen_snapshot_valid(json_node_get_object(result)))
        return luaL_error(state, "gnoblin.monitors.privacy_screen returned an invalid snapshot");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_monitors_set_privacy_screen(lua_State* state) {
    if (lua_gettop(state) != 1 ||
        (!lua_isboolean(state, 1) &&
         !(lua_type(state, 1) == LUA_TSTRING && g_str_equal(lua_tostring(state, 1), "inherit"))))
        return luaL_error(state,
                          "gnoblin.monitors.set_privacy_screen requires true, false, or 'inherit'");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    if (lua_isboolean(state, 1))
        json_object_set_boolean_member(arguments, "value", lua_toboolean(state, 1));
    else
        json_object_set_string_member(arguments, "value", "inherit");
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "monitors.set_privacy_screen", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.monitors.set_privacy_screen failed: %s",
                          call_error ? call_error->message : "unknown compositor error");
    if (!JSON_NODE_HOLDS_OBJECT(result) ||
        !lua_cli_monitor_privacy_screen_snapshot_valid(json_node_get_object(result)))
        return luaL_error(state,
                          "gnoblin.monitors.set_privacy_screen returned an invalid snapshot");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_workspaces_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.list returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(workspaces), 0);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        if (!workspace)
            return luaL_error(state,
                              "gnoblin.workspaces.list returned an invalid workspace record");
        lua_cli_push_workspace_record(state, cli, workspace);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_workspaces_active(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.active takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.active failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.active returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        JsonNode* active = workspace ? json_object_get_member(workspace, "active") : NULL;
        if (active && JSON_NODE_HOLDS_VALUE(active) &&
            json_node_get_value_type(active) == G_TYPE_BOOLEAN && json_node_get_boolean(active)) {
            lua_cli_push_workspace_record(state, cli, workspace);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

int lua_cli_workspaces_by_id(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1))
        return luaL_error(state,
                          "gnoblin.workspaces.by_id requires one stable workspace ID string");
    const char* id = lua_tostring(state, 1);
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "workspaces.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.workspaces.by_id failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.workspaces.by_id returned an invalid snapshot");
    JsonArray* workspaces = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(workspaces); i++) {
        JsonObject* workspace = json_array_get_object_element(workspaces, i);
        if (workspace && g_str_equal(member_string(workspace, "id", ""), id)) {
            lua_cli_push_workspace_record(state, cli, workspace);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

gboolean lua_cli_workspace_object_has_only_keys(JsonObject* object, const char* const* allowed) {
    GList* members = json_object_get_members(object);
    gboolean valid = TRUE;
    for (GList* item = members; item && valid; item = item->next) {
        gboolean found = FALSE;
        for (guint i = 0; allowed[i]; i++)
            found |= g_str_equal(item->data, allowed[i]);
        valid = found;
    }
    g_list_free(members);
    return valid;
}

static gboolean lua_cli_workspace_selector_valid(JsonObject* object) {
    JsonNode* id = object ? json_object_get_member(object, "id") : NULL;
    JsonNode* number = object ? json_object_get_member(object, "number") : NULL;
    if (!!id == !!number)
        return FALSE;
    if (id)
        return JSON_NODE_HOLDS_VALUE(id) && json_node_get_value_type(id) == G_TYPE_STRING &&
               *json_node_get_string(id) != '\0';
    return JSON_NODE_HOLDS_VALUE(number) &&
           (json_node_get_value_type(number) == G_TYPE_INT ||
            json_node_get_value_type(number) == G_TYPE_INT64) &&
           json_node_get_int(number) > 0;
}

static gboolean lua_cli_workspace_boolean_member_valid(JsonObject* object, const char* name) {
    JsonNode* value = json_object_get_member(object, name);
    return !value ||
           (JSON_NODE_HOLDS_VALUE(value) && json_node_get_value_type(value) == G_TYPE_BOOLEAN);
}

static gboolean lua_cli_workspace_string_member_valid(JsonObject* object, const char* name) {
    JsonNode* value = json_object_get_member(object, name);
    return !value ||
           (JSON_NODE_HOLDS_VALUE(value) && json_node_get_value_type(value) == G_TYPE_STRING);
}

static gboolean lua_cli_workspace_record_valid(JsonObject* object) {
    const char* id = member_string(object, "id", NULL);
    const char* name = member_string(object, "name", NULL);
    JsonNode* number = object ? json_object_get_member(object, "number") : NULL;
    JsonNode* windows = object ? json_object_get_member(object, "windows") : NULL;
    JsonNode* active = object ? json_object_get_member(object, "active") : NULL;
    JsonNode* persistent = object ? json_object_get_member(object, "persistent") : NULL;
    return id && *id && name && number && JSON_NODE_HOLDS_VALUE(number) &&
           (json_node_get_value_type(number) == G_TYPE_INT ||
            json_node_get_value_type(number) == G_TYPE_INT64) &&
           json_node_get_int(number) > 0 && windows && JSON_NODE_HOLDS_VALUE(windows) &&
           (json_node_get_value_type(windows) == G_TYPE_INT ||
            json_node_get_value_type(windows) == G_TYPE_INT64) &&
           json_node_get_int(windows) >= 0 && active && JSON_NODE_HOLDS_VALUE(active) &&
           json_node_get_value_type(active) == G_TYPE_BOOLEAN && persistent &&
           JSON_NODE_HOLDS_VALUE(persistent) &&
           json_node_get_value_type(persistent) == G_TYPE_BOOLEAN;
}

static int lua_cli_workspace_operation_error(lua_State* state, JsonObject* arguments,
                                             const char* message) {
    json_object_unref(arguments);
    return luaL_error(state, "%s", message);
}

int lua_cli_workspaces_operation(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    gboolean takes_no_arguments =
        g_str_equal(method, "workspace.next") || g_str_equal(method, "workspace.previous");
    if (takes_no_arguments ? lua_gettop(state) != 0
                           : lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "%s has invalid arguments", method);

    JsonObject* arguments = takes_no_arguments
                                ? json_object_new()
                                : lua_cli_table_object(state, 1, "workspace options");
    gboolean valid = FALSE;
    static const char* const create_fields[] = {"id", "name", "activate", NULL};
    static const char* const selector_fields[] = {"id", "number", NULL};
    static const char* const rename_fields[] = {"id", "number", "name", NULL};
    static const char* const move_active_fields[] = {"workspace", "follow", NULL};
    static const char* const move_window_fields[] = {"window", "workspace", "follow", NULL};

    if (g_str_equal(method, "workspace.create")) {
        const char* name = member_string(arguments, "name", NULL);
        const char* id = member_string(arguments, "id", NULL);
        valid = lua_cli_workspace_object_has_only_keys(arguments, create_fields) && name && *name &&
                g_utf8_validate(name, -1, NULL) && g_utf8_strlen(name, -1) <= 80 &&
                lua_cli_workspace_string_member_valid(arguments, "id") &&
                (!id || g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, 0, 0)) &&
                lua_cli_workspace_boolean_member_valid(arguments, "activate");
    } else if (g_str_equal(method, "workspace.rename")) {
        const char* name = member_string(arguments, "name", NULL);
        valid = lua_cli_workspace_object_has_only_keys(arguments, rename_fields) &&
                lua_cli_workspace_selector_valid(arguments) && name && *name &&
                lua_cli_workspace_string_member_valid(arguments, "name") &&
                g_utf8_validate(name, -1, NULL) && g_utf8_strlen(name, -1) <= 80;
    } else if (g_str_equal(method, "workspace.remove") || g_str_equal(method, "workspace.switch")) {
        valid = lua_cli_workspace_object_has_only_keys(arguments, selector_fields) &&
                lua_cli_workspace_selector_valid(arguments);
    } else if (g_str_equal(method, "workspace.move_active") ||
               g_str_equal(method, "workspace.move_window")) {
        gboolean moving_active = g_str_equal(method, "workspace.move_active");
        JsonObject* workspace = member_object(arguments, "workspace");
        const char* window = member_string(arguments, "window", NULL);
        valid = lua_cli_workspace_object_has_only_keys(
                    arguments, moving_active ? move_active_fields : move_window_fields) &&
                workspace && json_object_get_size(workspace) == 1 &&
                lua_cli_workspace_selector_valid(workspace) &&
                lua_cli_workspace_boolean_member_valid(arguments, "follow") &&
                (moving_active || lua_cli_workspace_string_member_valid(arguments, "window")) &&
                (moving_active || (window && *window && g_utf8_validate(window, -1, NULL)));
    } else if (takes_no_arguments) {
        valid = TRUE;
    }
    if (!valid)
        return lua_cli_workspace_operation_error(state, arguments,
                                                 "workspace operation received invalid arguments");

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "%s failed: %s", method, call_error->message);
    JsonObject* object = JSON_NODE_HOLDS_OBJECT(result) ? json_node_get_object(result) : NULL;
    if (g_str_equal(method, "workspace.move_active") ||
        g_str_equal(method, "workspace.move_window")) {
        JsonObject* workspace = member_object(object, "workspace");
        const char* window = member_string(object, "window", NULL);
        JsonNode* follow = object ? json_object_get_member(object, "follow") : NULL;
        if (!lua_cli_workspace_record_valid(workspace) || !window || !*window || !follow ||
            !JSON_NODE_HOLDS_VALUE(follow) || json_node_get_value_type(follow) != G_TYPE_BOOLEAN)
            return luaL_error(state, "%s returned an invalid WorkspaceMove", method);
        json_to_lua(state, result);
        lua_cli_push_readonly_value(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    if (!lua_cli_workspace_record_valid(object)) {
        g_autofree char* encoded = json_to_string(result, TRUE);
        return luaL_error(state, "%s returned an invalid Workspace record: %s", method, encoded);
    }
    lua_cli_push_workspace_record(state, cli, object);
    return 1;
}

int lua_cli_windows_list(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.windows.list accepts one optional filter table");
    JsonObject* arguments =
        lua_gettop(state) ? lua_cli_table_object(state, 1, "window filter") : json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.list failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.list returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(windows), 0);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window)
            return luaL_error(state, "gnoblin.windows.list returned an invalid window record");
        lua_cli_push_window_record(state, cli, window);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_windows_snap_context(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1)
        return luaL_error(state, "gnoblin.windows.snap_context requires a FocusContext");
    LuaCliFocusContext* context = lua_cli_test_focus_context(state, 1);
    if (!context)
        return luaL_error(state, "gnoblin.windows.snap_context requires a FocusContext");
    if (context->consumed)
        return luaL_error(state, "gnoblin.windows.snap_context cannot reuse a consumed context");
    if (!context->subscription || !context->subscription->active ||
        !context->subscription->connection || context->subscription->cli != cli)
        return luaL_error(state,
                          "gnoblin.windows.snap_context requires its active event connection");

    JsonObject* arguments = json_object_new();
    json_object_set_string_member(arguments, "focus_context", context->token);
    context->consumed = TRUE;
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = cli_lua_call_compositor_on_subscription(
        context->subscription, "window.snap_context", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "window.snap_context failed: %s",
                          call_error ? call_error->message : "compositor request failed");
    if (!JSON_NODE_HOLDS_OBJECT(result))
        return luaL_error(state, "window.snap_context returned an invalid result");
    lua_cli_push_snap_context(state, context->subscription, json_node_get_object(result));
    return 1;
}

int lua_cli_focus_history(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.focus.history accepts one optional filter table");
    JsonObject* arguments = lua_gettop(state)
                                ? lua_cli_table_object(state, 1, "focus history filter")
                                : json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "focus.history", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.focus.history failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.focus.history returned an invalid snapshot");

    JsonArray* windows = json_node_get_array(result);
    lua_createtable(state, json_array_get_length(windows), 0);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (!window || !member_string(window, "id", NULL) || !*member_string(window, "id", ""))
            return luaL_error(state, "gnoblin.focus.history returned an invalid Window record");
        lua_cli_push_window_record(state, cli, window);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_cli_windows_focused(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.windows.focused takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    json_object_set_boolean_member(arguments, "focused", TRUE);
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.focused failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.focused returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    if (json_array_get_length(windows) == 0) {
        lua_pushnil(state);
        return 1;
    }
    JsonObject* window = json_array_get_object_element(windows, 0);
    if (!window)
        return luaL_error(state, "gnoblin.windows.focused returned an invalid window record");
    lua_cli_push_window_record(state, cli, window);
    return 1;
}

int lua_cli_windows_by_id(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.windows.by_id requires one stable window ID string");
    const char* id = lua_tostring(state, 1);
    if (!*id)
        return luaL_error(state, "gnoblin.windows.by_id requires one stable window ID string");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result =
        call_compositor(cli, "api", "windows.list", arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "gnoblin.windows.by_id failed: %s", call_error->message);
    if (!JSON_NODE_HOLDS_ARRAY(result))
        return luaL_error(state, "gnoblin.windows.by_id returned an invalid snapshot");
    JsonArray* windows = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(windows); i++) {
        JsonObject* window = json_array_get_object_element(windows, i);
        if (window && g_str_equal(member_string(window, "id", ""), id)) {
            lua_cli_push_window_record(state, cli, window);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

void cli_lua_event_subscription_detach(CliLuaEventSubscription* subscription) {
    if (!subscription || !subscription->active)
        return;
    subscription->active = FALSE;
    if (subscription->location_requests)
        g_hash_table_remove_all(subscription->location_requests);
    if (subscription->source_id) {
        g_source_remove(subscription->source_id);
        subscription->source_id = 0;
    }
    if (subscription->pending_source_id) {
        g_source_remove(subscription->pending_source_id);
        subscription->pending_source_id = 0;
    }
    if (subscription->connection) {
        if (!subscription->shortcut_owner)
            g_io_stream_close(G_IO_STREAM(subscription->connection), NULL, NULL);
        g_clear_object(&subscription->connection);
    }
    if (subscription->shortcut_owner_transport && subscription->cli &&
        subscription->cli->lua_shortcut_owner == subscription)
        subscription->cli->lua_shortcut_owner = NULL;
    if (!subscription->shortcut_owner_transport && subscription->cli &&
        subscription->cli->active_lua_event_subscriptions > 0) {
        subscription->cli->active_lua_event_subscriptions--;
        if (subscription->cli->active_lua_event_subscriptions == 0 &&
            subscription->cli->lua_event_loop &&
            g_main_loop_is_running(subscription->cli->lua_event_loop))
            g_main_loop_quit(subscription->cli->lua_event_loop);
    }
    if (!subscription->dispatching && subscription->callback_ref != LUA_NOREF) {
        luaL_unref(subscription->cli->lua_state, LUA_REGISTRYINDEX, subscription->callback_ref);
        subscription->callback_ref = LUA_NOREF;
    }
}

gboolean cli_lua_shortcut_owner_dispatch(CliLuaEventSubscription* owner, JsonNode* event) {
    if (!owner || !owner->cli || !event || !JSON_NODE_HOLDS_OBJECT(event))
        return FALSE;
    const char* name = member_string(json_node_get_object(event), "event", "");
    Cli* cli = owner->cli;
    if (!cli->lua_event_subscriptions)
        return TRUE;
    for (guint i = 0; i < cli->lua_event_subscriptions->len; i++) {
        CliLuaEventSubscription* listener = g_ptr_array_index(cli->lua_event_subscriptions, i);
        if (listener != owner && listener->shortcut_owner && listener->active &&
            !listener->once_fired && g_str_equal(listener->event_name, name))
            cli_lua_event_dispatch(listener, event);
    }
    return TRUE;
}

void cli_lua_event_subscription_free(gpointer data) {
    CliLuaEventSubscription* subscription = data;
    if (!subscription)
        return;
    cli_lua_event_subscription_detach(subscription);
    if (subscription->callback_ref != LUA_NOREF && subscription->cli &&
        subscription->cli->lua_state) {
        luaL_unref(subscription->cli->lua_state, LUA_REGISTRYINDEX, subscription->callback_ref);
        subscription->callback_ref = LUA_NOREF;
    }
    g_clear_object(&subscription->connection);
    g_clear_pointer(&subscription->location_requests, g_hash_table_unref);
    if (subscription->pending)
        g_string_free(subscription->pending, TRUE);
    g_free(subscription->event_name);
    g_free(subscription);
}

void lua_cli_push_focus_context(lua_State* state, CliLuaEventSubscription* subscription,
                                const char* token) {
    LuaCliFocusContext* context = lua_newuserdatauv(state, sizeof *context, 0);
    context->magic = LUA_CLI_FOCUS_CONTEXT_MAGIC;
    context->subscription = subscription;
    context->token = g_strdup(token);
    context->consumed = FALSE;
    luaL_getmetatable(state, GNOBLINCTL_FOCUS_CONTEXT_METATABLE);
    lua_setmetatable(state, -2);
}

void lua_cli_push_menu_context(lua_State* state, CliLuaEventSubscription* subscription,
                               const char* token) {
    LuaCliMenuContext* context = lua_newuserdatauv(state, sizeof *context, 0);
    context->magic = LUA_CLI_MENU_CONTEXT_MAGIC;
    context->subscription = subscription;
    context->token = g_strdup(token);
    context->consumed = FALSE;
    luaL_getmetatable(state, GNOBLINCTL_MENU_CONTEXT_METATABLE);
    lua_setmetatable(state, -2);
}

static JsonNode* lua_cli_output_snapshot(lua_State* state, Cli* cli, const char* operation) {
    JsonObject* arguments = json_object_new();
    g_autoptr(GError) error = NULL;
    JsonNode* result = call_compositor(cli, "api", "outputs.list", arguments, &error);
    json_object_unref(arguments);
    if (!result)
        luaL_error(state, "gnoblin.outputs.%s failed: %s", operation,
                   error ? error->message : "unknown error");
    if (!JSON_NODE_HOLDS_ARRAY(result)) {
        json_node_unref(result);
        luaL_error(state, "gnoblin.outputs.%s returned an invalid snapshot", operation);
    }
    return result;
}

int lua_cli_outputs_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.outputs.list takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_output_snapshot(state, cli, "list");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_cli_outputs_find(lua_State* state, gboolean by_id) {
    const char* id = NULL;
    if (by_id && (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING ||
                  !(id = lua_tostring(state, 1)) || !*id))
        return luaL_error(state, "gnoblin.outputs.by_id requires one nonempty output ID");
    if (!by_id && lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.outputs.primary takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    g_autoptr(JsonNode) result = lua_cli_output_snapshot(state, cli, by_id ? "by_id" : "primary");
    JsonArray* outputs = json_node_get_array(result);
    for (guint i = 0; i < json_array_get_length(outputs); i++) {
        JsonObject* output = json_array_get_object_element(outputs, i);
        if (output &&
            ((by_id && g_str_equal(member_string(output, "id", ""), id)) ||
             (!by_id && json_object_get_boolean_member_with_default(output, "primary", FALSE)))) {
            JsonNode* copy = json_node_copy(json_array_get_element(outputs, i));
            json_to_lua(state, copy);
            json_node_unref(copy);
            lua_cli_push_readonly_value(state, -1);
            lua_remove(state, -2);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

int lua_cli_outputs_primary(lua_State* state) {
    return lua_cli_outputs_find(state, FALSE);
}
int lua_cli_outputs_by_id(lua_State* state) {
    return lua_cli_outputs_find(state, TRUE);
}
int lua_cli_outputs_renderer(lua_State* state) {
    if (lua_gettop(state))
        return luaL_error(state, "gnoblin.outputs.renderer takes no arguments");
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    JsonObject* args = json_object_new();
    g_autoptr(GError) error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", "outputs.renderer", args, &error);
    json_object_unref(args);
    if (!result)
        return luaL_error(state, "gnoblin.outputs.renderer failed: %s",
                          error ? error->message : "unknown error");
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}
