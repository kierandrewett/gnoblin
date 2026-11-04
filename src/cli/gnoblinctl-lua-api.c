#include "gnoblinctl-internal.h"

static int lua_api_call(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "Gnoblin API methods accept one optional argument table");

    JsonObject* arguments = json_object_new();
    if (lua_gettop(state) == 1) {
        g_autoptr(GError) conversion_error = NULL;
        g_autoptr(JsonNode) encoded = lua_to_json(state, 1, 0, &conversion_error);
        if (!encoded)
            return luaL_error(state, "invalid API arguments: %s", conversion_error->message);
        if (!JSON_NODE_HOLDS_OBJECT(encoded))
            return luaL_error(state, "Gnoblin API arguments must be a named Lua table");
        json_object_unref(arguments);
        arguments = json_object_ref(json_node_get_object(encoded));
    }

    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    json_object_unref(arguments);
    if (!result)
        return luaL_error(state, "Gnoblin API call failed: %s", call_error->message);
    json_to_lua(state, result);
    return 1;
}

static int lua_cli_runtime_scope_error(lua_State* state) {
    const char* method = lua_tostring(state, lua_upvalueindex(1));
    if (g_str_has_prefix(method, "gnoblin.shortcuts."))
        return luaL_error(
            state,
            "%s requires the supervised Lua runtime; terminal shortcut bindings are owned by "
            "a persistent runtime connection",
            method);
    return luaL_error(
        state, "%s requires the supervised Lua runtime; gnoblinctl lua cannot subscribe to events",
        method);
}

static void lua_cli_push_runtime_scope_error(lua_State* state, const char* method) {
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_cli_runtime_scope_error, 1);
}

int lua_api_index(lua_State* state) {
    const char* prefix = lua_tostring(state, lua_upvalueindex(1));
    const char* name = luaL_checkstring(state, 2);
    if (!*prefix) {
        if (g_str_equal(name, "on") || g_str_equal(name, "once")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushboolean(state, g_str_equal(name, "once"));
            lua_pushcclosure(state, lua_cli_events_subscribe, 2);
            return 1;
        }
        if (g_str_equal(name, "version")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_version, 1);
            return 1;
        }
        if (g_str_equal(name, "settings")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_settings_property, 1);
            lua_call(state, 0, 1);
            return 1;
        }
        lua_newtable(state);
        lua_newtable(state);
        lua_pushstring(state, name);
        lua_pushvalue(state, lua_upvalueindex(2));
        lua_pushcclosure(state, lua_api_index, 2);
        lua_setfield(state, -2, "__index");
        lua_setmetatable(state, -2);
        return 1;
    }
    if (g_str_equal(prefix, "windows")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_windows_list, 1);
        else if (g_str_equal(name, "focused"))
            lua_pushcclosure(state, lua_cli_windows_focused, 1);
        else if (g_str_equal(name, "by_id"))
            lua_pushcclosure(state, lua_cli_windows_by_id, 1);
        else if (g_str_equal(name, "snap_context"))
            lua_pushcclosure(state, lua_cli_windows_snap_context, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "focused") ||
            g_str_equal(name, "by_id") || g_str_equal(name, "snap_context"))
            return 1;
    }
    if (g_str_equal(prefix, "workspaces")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_workspaces_list, 1);
        else if (g_str_equal(name, "active"))
            lua_pushcclosure(state, lua_cli_workspaces_active, 1);
        else if (g_str_equal(name, "by_id"))
            lua_pushcclosure(state, lua_cli_workspaces_by_id, 1);
        else {
            const char* method = NULL;
            if (g_str_equal(name, "create"))
                method = "workspace.create";
            else if (g_str_equal(name, "next"))
                method = "workspace.next";
            else if (g_str_equal(name, "previous"))
                method = "workspace.previous";
            if (method) {
                lua_pop(state, 1);
                lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
                lua_pushstring(state, method);
                lua_pushcclosure(state, lua_cli_workspaces_operation, 2);
                return 1;
            }
            lua_pop(state, 1);
        }
        if (g_str_equal(name, "list") || g_str_equal(name, "active") || g_str_equal(name, "by_id"))
            return 1;
    }
    if (g_str_equal(prefix, "outputs")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_outputs_list, 1);
        else if (g_str_equal(name, "primary"))
            lua_pushcclosure(state, lua_cli_outputs_primary, 1);
        else if (g_str_equal(name, "by_id"))
            lua_pushcclosure(state, lua_cli_outputs_by_id, 1);
        else if (g_str_equal(name, "renderer"))
            lua_pushcclosure(state, lua_cli_outputs_renderer, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "primary") ||
            g_str_equal(name, "by_id") || g_str_equal(name, "renderer"))
            return 1;
    }
    if (g_str_equal(prefix, "layers")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        if (g_str_equal(name, "list"))
            lua_pushcclosure(state, lua_cli_layers_list, 1);
        else if (g_str_equal(name, "animation_policy"))
            lua_pushcclosure(state, lua_cli_layer_animation_policy, 1);
        else
            lua_pop(state, 1);
        if (g_str_equal(name, "list") || g_str_equal(name, "animation_policy"))
            return 1;
    }
    if (g_str_equal(prefix, "appearance") && g_str_equal(name, "color_scheme")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_appearance_color_scheme, 1);
        return 1;
    }
    if (g_str_equal(prefix, "privacy") && g_str_equal(name, "state")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_privacy_state, 1);
        return 1;
    }
    if (g_str_equal(prefix, "location") && g_str_equal(name, "authorize_app")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_location_authorize_app, 1);
        return 1;
    }
    if (g_str_equal(prefix, "privacy") &&
        (g_str_equal(name, "stop_sharing") || g_str_equal(name, "stop_recording"))) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushfstring(state, "privacy.%s", name);
        lua_pushcclosure(state, lua_cli_privacy_stop, 2);
        return 1;
    }
    if (g_str_equal(prefix, "runtime") && g_str_equal(name, "reload_config")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_runtime_reload_config, 1);
        return 1;
    }
    if (g_str_equal(prefix, "runtime") && g_str_equal(name, "status")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_runtime_status, 1);
        return 1;
    }
    if (g_str_equal(prefix, "config")) {
        if (g_str_equal(name, "path")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_config_path, 1);
            return 1;
        }
        if (g_str_equal(name, "default")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_config_default, 1);
            return 1;
        }
        if (g_str_equal(name, "restore_default")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushliteral(state, "config.restore_default");
            lua_pushcclosure(state, lua_cli_session_operation, 2);
            return 1;
        }
    }
    if (g_str_equal(prefix, "session") &&
        (g_str_equal(name, "lock") || g_str_equal(name, "logout") || g_str_equal(name, "suspend") ||
         g_str_equal(name, "hibernate") || g_str_equal(name, "reboot") ||
         g_str_equal(name, "poweroff"))) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushfstring(state, "session.%s", name);
        lua_pushcclosure(state, lua_cli_session_operation, 2);
        return 1;
    }
    if (g_str_equal(prefix, "capabilities") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_capabilities_list, 1);
        return 1;
    }
    if (g_str_equal(prefix, "session") && g_str_equal(name, "status")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_session_status, 1);
        return 1;
    }
    if (g_str_equal(prefix, "animations") && g_str_equal(name, "preview")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_animations_preview, 1);
        return 1;
    }
    if (g_str_equal(prefix, "animations") &&
        (g_str_equal(name, "list") || g_str_equal(name, "get") || g_str_equal(name, "inspect") ||
         g_str_equal(name, "surfaces"))) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_CFunction function = g_str_equal(name, "list")      ? lua_cli_animations_list
                                 : g_str_equal(name, "get")     ? lua_cli_animations_get
                                 : g_str_equal(name, "inspect") ? lua_cli_animations_inspect
                                                                : lua_cli_animations_surfaces;
        lua_pushcclosure(state, function, 1);
        return 1;
    }
    if (g_str_equal(prefix, "launches") && g_str_equal(name, "list")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_launches_read, 1);
        return 1;
    }
    if (g_str_equal(prefix, "launches") && g_str_equal(name, "begin")) {
        lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
        lua_pushcclosure(state, lua_cli_launches_begin, 1);
        return 1;
    }
    if (g_str_equal(prefix, "input")) {
        if (g_str_equal(name, "text_target")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_input_text_target, 1);
            return 1;
        }
        if (g_str_equal(name, "devices") || g_str_equal(name, "sources") ||
            g_str_equal(name, "current_source") || g_str_equal(name, "orientation_lock")) {
            g_autofree char* method = g_strdup_printf("input.%s", name);
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushstring(state, method);
            lua_pushcclosure(state, lua_cli_input_snapshot, 2);
            return 1;
        }
    }
    if (g_str_equal(prefix, "focus")) {
        if (g_str_equal(name, "policy")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_focus_policy_property, 1);
            return 1;
        }
        if (g_str_equal(name, "history")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_focus_history, 1);
            return 1;
        }
    }
    if (g_str_equal(prefix, "shortcuts")) {
        if (g_str_equal(name, "bind")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_shortcuts_bind, 1);
            return 1;
        }
        if (g_str_equal(name, "list") || g_str_equal(name, "configured") ||
            g_str_equal(name, "actions")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_CFunction function = g_str_equal(name, "list") ? lua_cli_shortcuts_list
                                     : g_str_equal(name, "configured")
                                         ? lua_cli_shortcuts_configured
                                         : lua_cli_shortcuts_actions;
            lua_pushcclosure(state, function, 1);
            return 1;
        }
        if (g_str_equal(name, "capture")) {
            lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
            lua_pushcclosure(state, lua_cli_shortcuts_capture, 1);
            return 1;
        }
    }
    if (g_str_equal(prefix, "idle"))
        return luaL_error(state,
                          "gnoblin.idle.%s needs a configuration to hold the watch; a gnoblinctl "
                          "script ends before the session is idle",
                          name);
    static const char* const removed[][2] = {
        {"monitor.list", "gnoblin.outputs.list"},
        {"monitors.list", "gnoblin.outputs.list"},
        {"monitors.primary", "gnoblin.outputs.primary"},
        {"monitors.privacy_screen", "output.privacy_screen"},
        {"monitors.set_privacy_screen",
         "gnoblin.settings.override(\"monitors.privacy_screen\", value) in a configuration"},
        {"outputs.set_privacy_screen",
         "gnoblin.settings.override(\"monitors.privacy_screen\", value) in a configuration"},
        {"input.set_orientation_lock",
         "gnoblin.settings.override(\"input.orientation_lock\", value) in a configuration, or "
         "`gnoblinctl input orientation-lock`"},
        {"layer.list", "gnoblin.layers.list"},
        {"input.list", "gnoblin.input.sources"},
        {"input.current", "gnoblin.input.current_source"},
        {"events.on", "gnoblin.on"},
        {"events.once", "gnoblin.once"},
        {"workspaces.rename", "workspace:rename"},
        {"workspaces.remove", "workspace:remove"},
        {"workspaces.activate", "workspace:activate"},
        {"workspaces.move_active", "window:move_to_workspace"},
        {"workspaces.move_window", "window:move_to_workspace"},
        {"animations.seek", "preview:seek"},
        {"animations.step", "preview:step"},
        {"animations.play", "preview:play"},
        {"animations.pause", "preview:pause"},
        {"animations.stop", "preview:stop"},
        {"input.select_source", "source:select"},
        {"launches.snapshot", "gnoblin.launches.list"},
        {"launches.end", "launch:finish"},
        {"launches.finish", "launch:finish"},
        {"shortcuts.unbind", "shortcut:unbind"},
        {"shortcuts.end_session", "shortcut:end_session"},
        {"session.activity",
         "gnoblin.idle.watch in a configuration, or `gnoblinctl session activity`"},
        {NULL, NULL},
    };
    g_autofree char* qualified = g_strdup_printf("%s.%s", prefix, name);
    for (guint i = 0; removed[i][0]; i++)
        if (g_str_equal(qualified, removed[i][0]))
            return luaL_error(state, "gnoblin.%s was removed; use %s", qualified, removed[i][1]);
    g_autofree char* method = g_strdup_printf("%s.%s", prefix, name);
    lua_pushlightuserdata(state, lua_touserdata(state, lua_upvalueindex(2)));
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_api_call, 2);
    return 1;
}

void register_lua_cli_window_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_WINDOW_RECORD_METATABLE)) {
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
    lua_pushcfunction(state, lua_cli_window_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

void register_lua_cli_workspace_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_WORKSPACE_RECORD_METATABLE)) {
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
    lua_pushcfunction(state, lua_cli_workspace_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

void register_lua_cli_output_record(lua_State* state) {
    if (!luaL_newmetatable(state, GNOBLINCTL_OUTPUT_RECORD_METATABLE)) {
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
    lua_pushcfunction(state, lua_cli_output_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pop(state, 1);
}

void print_lua_error(lua_State* state, const char* prefix) {
    const char* message = lua_tostring(state, -1);
    g_printerr("%s%s\n", prefix, message ? message : "Lua evaluation failed");
    lua_pop(state, 1);
}
