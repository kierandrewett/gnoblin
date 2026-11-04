#include "gnoblinctl-internal.h"

#include "../control/gnoblin-easing.h"

/* The console's Lua API. The names come from lua_api[], the table the Lua runtime installs, so a
 * name exists in `gnoblinctl lua` exactly when it exists in a configuration. This file says which
 * of them the console runs itself:
 *
 *   - A function in console_functions or console_methods has a console implementation. It calls
 *     the compositor, which runs the same operations as the runtime, and it returns the completed
 *     value, because the console has no Operation to hand back.
 *   - Any other name in lua_api[] needs a runtime that the console does not have: a timer, a
 *     process, a listener, a tween, a config document. It raises one error that says so.
 *   - A name that lua_api[] does not hold is an unknown name, and the error says so.
 *
 * The audit script (scripts/audit-lua-annotations.py) checks that every name below is in
 * lua_api[]. */

typedef struct {
    const char* path;
    lua_CFunction function;
} ConsoleFunction;

/* gnoblin.<path>: functions the console runs. Upvalues: the Cli, the wire operation (the path when
 * lua_api[] names none) and the path. */
static const ConsoleFunction console_functions[] = {
    {"version", lua_cli_version},
    {"on", lua_cli_events_subscribe},
    {"once", lua_cli_events_subscribe},
    {"find", lua_cli_find},
    {"ease", lua_cli_ease},
    {"lerp", lua_cli_lerp},
    {"config.path", lua_cli_config_path},
    {"config.default", lua_cli_config_default},
    {"config.restore_default", lua_cli_session_operation},
    {"windows.list", lua_cli_windows_list},
    {"windows.focused", lua_cli_windows_focused},
    {"windows.by_id", lua_cli_windows_by_id},
    {"windows.snap_context", lua_cli_windows_snap_context},
    {"workspaces.list", lua_cli_workspaces_list},
    {"workspaces.active", lua_cli_workspaces_active},
    {"workspaces.by_id", lua_cli_workspaces_by_id},
    {"workspaces.create", lua_cli_workspaces_operation},
    {"workspaces.next", lua_cli_workspaces_operation},
    {"workspaces.previous", lua_cli_workspaces_operation},
    {"outputs.list", lua_cli_outputs_list},
    {"outputs.primary", lua_cli_outputs_primary},
    {"outputs.by_id", lua_cli_outputs_by_id},
    {"outputs.renderer", lua_cli_outputs_renderer},
    {"layers.list", lua_cli_layers_list},
    {"layers.animation_policy", lua_cli_layer_animation_policy},
    {"capabilities.list", lua_cli_capabilities_list},
    {"appearance.color_scheme", lua_cli_appearance_color_scheme},
    {"animations.list", lua_cli_animations_list},
    {"animations.get", lua_cli_animations_get},
    {"input.devices", lua_cli_input_snapshot},
    {"input.sources", lua_cli_input_snapshot},
    {"input.current_source", lua_cli_input_snapshot},
    {"input.orientation_lock", lua_cli_input_snapshot},
    {"input.text_target", lua_cli_input_text_target},
    {"shortcuts.actions", lua_cli_shortcuts_actions},
    {"shortcuts.configured", lua_cli_shortcuts_configured},
    {"shortcuts.list", lua_cli_shortcuts_list},
    {"shortcuts.capture", lua_cli_shortcuts_capture},
    {"shortcuts.bind", lua_cli_shortcuts_bind},
    {"launches.list", lua_cli_launches_read},
    {"launches.begin", lua_cli_launches_begin},
    {"privacy.state", lua_cli_privacy_state},
    {"privacy.stop_sharing", lua_cli_privacy_stop},
    {"privacy.stop_recording", lua_cli_privacy_stop},
    {"session.status", lua_cli_session_status},
    {"session.lock", lua_cli_session_operation},
    {"session.logout", lua_cli_session_operation},
    {"session.suspend", lua_cli_session_operation},
    {"session.hibernate", lua_cli_session_operation},
    {"session.reboot", lua_cli_session_operation},
    {"session.poweroff", lua_cli_session_operation},
    {"runtime.status", lua_cli_runtime_status},
    {"runtime.reload_config", lua_cli_runtime_reload_config},
    {"console.toggle", lua_cli_wire_call},
};

/* The names the runtime installs outside lua_api[] (the audit script lists them as structural).
 * `configure`, `window_rule` and `animation` declare settings in a config document, which the
 * console does not have; a null function marks that. */
static const ConsoleFunction console_extras[] = {
    {"focus.policy", lua_cli_focus_policy_property},
    {"focus.history", lua_cli_focus_history},
    {"configure", NULL},
    {"window_rule", NULL},
    {"animation", NULL},
};

typedef struct {
    const char* path; /* Class:method */
    lua_CFunction function;
} ConsoleMethod;

/* Class:method: methods the console runs on a record it made. Upvalues: the Cli, the wire
 * operation (empty when lua_api[] names none) and the method name. A method of lua_api[] that is
 * not listed raises the not-available error. */
static const ConsoleMethod console_methods[] = {
    {"Window:close", lua_cli_window_method},
    {"Window:kill", lua_cli_window_method},
    {"Window:raise", lua_cli_window_method},
    {"Window:lower", lua_cli_window_method},
    {"Window:set_minimized", lua_cli_window_method},
    {"Window:restore_or_minimize", lua_cli_window_method},
    {"Window:set_maximized", lua_cli_window_method},
    {"Window:set_fullscreen", lua_cli_window_method},
    {"Window:set_above", lua_cli_window_method},
    {"Window:set_sticky", lua_cli_window_method},
    {"Window:move", lua_cli_window_method},
    {"Window:resize", lua_cli_window_method},
    {"Window:move_to_workspace", lua_cli_window_method},
    {"Window:move_to_monitor", lua_cli_window_method},
    {"Window:focus", lua_cli_window_method},
    {"Window:begin_move", lua_cli_window_method},
    {"Window:begin_resize", lua_cli_window_method},
    {"Window:thumbnail", lua_cli_window_method},
    {"Window:set_opacity", lua_cli_window_visual},
    {"Window:set_scale", lua_cli_window_visual},
    {"Window:set_rotation", lua_cli_window_visual},
    {"Window:set_offset", lua_cli_window_visual},
    {"Window:reset_visuals", lua_cli_window_visual},
    {"Window:inspect_animation", lua_cli_record_animation},
    {"Window:preview_animation", lua_cli_record_animation},
    {"LayerSurface:inspect_animation", lua_cli_record_animation},
    {"LayerSurface:preview_animation", lua_cli_record_animation},
    {"Workspace:activate", lua_cli_workspace_method},
    {"Workspace:rename", lua_cli_workspace_method},
    {"Workspace:remove", lua_cli_workspace_method},
    {"AnimationPreview:seek", lua_cli_animation_preview_method},
    {"AnimationPreview:step", lua_cli_animation_preview_method},
    {"AnimationPreview:play", lua_cli_animation_preview_method},
    {"AnimationPreview:pause", lua_cli_animation_preview_method},
    {"AnimationPreview:stop", lua_cli_animation_preview_method},
    {"InputSource:select", lua_cli_input_source_select},
    {"Shortcut:unbind", lua_cli_shortcut_method},
    {"Shortcut:end_session", lua_cli_shortcut_method},
    {"Launch:finish", lua_cli_launch_finish},
};

/* Names whose error says more than the common one. */
static const struct {
    const char* label;
    const char* hint;
} not_available_hints[] = {
    {"gnoblin.idle.watch", "read the idle sample with `gnoblinctl session activity`"},
    {"gnoblin.settings.override",
     "an override ends with the runtime that holds it; use `gnoblinctl input orientation-lock` "
     "for the orientation lock"},
    {"Window:set_effect", "an effect override ends with the runtime that holds it"},
    {"gnoblin.input.grab", "a grab needs a runtime to receive its keys"},
};

/* Raise the error for a name that needs a runtime. Upvalue 1 is the name as Lua writes it. */
int lua_cli_not_available(lua_State* state) {
    const char* label = lua_tostring(state, lua_upvalueindex(1));
    for (guint i = 0; i < G_N_ELEMENTS(not_available_hints); i++)
        if (g_str_equal(label, not_available_hints[i].label))
            return luaL_error(state,
                              "%s is not available in gnoblinctl lua because the console has no "
                              "runtime: %s",
                              label, not_available_hints[i].hint);
    return luaL_error(
        state, "%s is not available in gnoblinctl lua because the console has no runtime", label);
}

static void push_not_available(lua_State* state, const char* label) {
    lua_pushstring(state, label);
    lua_pushcclosure(state, lua_cli_not_available, 1);
}

/* A table that raises the not-available error for a call, a read and a write: the value of a name
 * that is a table in a configuration, such as gnoblin.configure. */
static void push_not_available_table(lua_State* state, const char* label) {
    static const char* const events[] = {"__call", "__index", "__newindex"};
    lua_newtable(state);
    lua_newtable(state);
    for (guint i = 0; i < G_N_ELEMENTS(events); i++) {
        push_not_available(state, label);
        lua_setfield(state, -2, events[i]);
    }
    lua_setmetatable(state, -2);
}

static void push_closure(lua_State* state, Cli* cli, lua_CFunction function, const char* method,
                         const char* name) {
    lua_pushlightuserdata(state, cli);
    lua_pushstring(state, method);
    lua_pushstring(state, name);
    lua_pushcclosure(state, function, 3);
}

static const LuaApiEntry* find_entry(const char* path) {
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (g_str_equal(entry->path, path))
            return entry;
    return NULL;
}

static lua_CFunction console_function(const char* path) {
    for (guint i = 0; i < G_N_ELEMENTS(console_functions); i++)
        if (g_str_equal(console_functions[i].path, path))
            return console_functions[i].function;
    return NULL;
}

/* Whether some name starts with `path` and a dot: `path` is a namespace such as gnoblin.windows. */
static gboolean is_namespace(const char* path) {
    gsize length = strlen(path);
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (!strchr(entry->path, ':') && g_str_has_prefix(entry->path, path) &&
            entry->path[length] == '.')
            return TRUE;
    for (guint i = 0; i < G_N_ELEMENTS(console_extras); i++)
        if (g_str_has_prefix(console_extras[i].path, path) && console_extras[i].path[length] == '.')
            return TRUE;
    return FALSE;
}

/* Push the function for gnoblin.<path>: the console's own, or the not-available error. Returns
 * FALSE and pushes nothing when lua_api[] has no such function. */
gboolean lua_cli_push_api_function(lua_State* state, Cli* cli, const char* path) {
    const LuaApiEntry* entry = find_entry(path);
    for (guint i = 0; !entry && i < G_N_ELEMENTS(console_extras); i++)
        if (g_str_equal(console_extras[i].path, path)) {
            lua_CFunction function = console_extras[i].function;
            g_autofree char* label = g_strdup_printf("gnoblin.%s", path);
            if (function)
                push_closure(state, cli, function, path, path);
            else
                push_not_available(state, label);
            return TRUE;
        }
    if (!entry)
        return FALSE;
    lua_CFunction function = console_function(path);
    if (function) {
        push_closure(state, cli, function, entry->operation ? entry->operation->name : path, path);
    } else {
        g_autofree char* label = g_strdup_printf("gnoblin.%s", path);
        push_not_available(state, label);
    }
    return TRUE;
}

/* Add the methods that lua_api[] declares for `class_name` to the table on top of the stack. */
void lua_cli_add_record_methods(lua_State* state, Cli* cli, const char* class_name) {
    gsize length = strlen(class_name);
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (strncmp(entry->path, class_name, length) != 0 || entry->path[length] != ':')
            continue;
        const char* name = entry->path + length + 1;
        lua_CFunction function = NULL;
        for (guint i = 0; i < G_N_ELEMENTS(console_methods) && !function; i++)
            if (g_str_equal(console_methods[i].path, entry->path))
                function = console_methods[i].function;
        if (function)
            push_closure(state, cli, function, entry->operation ? entry->operation->name : "",
                         name);
        else
            push_not_available(state, entry->path);
        lua_setfield(state, -2, name);
    }
}

/* A call that the compositor runs by name: the positional arguments, named by the annotations,
 * make the wire's table of fields. Upvalues: the Cli, the wire operation and the path. */
int lua_cli_wire_call(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    const char* path = lua_tostring(state, lua_upvalueindex(3));
    const char* names[8];
    int count = (int)lua_api_parameter_names(path, names, G_N_ELEMENTS(names));
    int supplied = lua_gettop(state);
    if (supplied > count)
        return luaL_error(state, "gnoblin.%s takes at most %d argument%s", path, count,
                          count == 1 ? "" : "s");

    g_autoptr(JsonObject) arguments = json_object_new();
    for (int i = 1; i <= supplied; i++) {
        if (lua_isnil(state, i))
            continue;
        g_autoptr(GError) conversion_error = NULL;
        g_autoptr(JsonNode) encoded = lua_to_json(state, i, 0, &conversion_error);
        if (!encoded)
            return luaL_error(state, "gnoblin.%s argument %d: %s", path, i,
                              conversion_error->message);
        if (g_str_equal(names[i - 1], "options")) {
            if (!JSON_NODE_HOLDS_OBJECT(encoded))
                return luaL_error(state, "gnoblin.%s options must be a table", path);
            g_autoptr(GList) members = json_object_get_members(json_node_get_object(encoded));
            for (GList* item = members; item; item = item->next)
                json_object_set_member(arguments, item->data,
                                       json_node_copy(json_object_get_member(
                                           json_node_get_object(encoded), item->data)));
        } else {
            json_object_set_member(arguments, names[i - 1], json_node_copy(encoded));
        }
    }
    g_autoptr(GError) call_error = NULL;
    g_autoptr(JsonNode) result = call_compositor(cli, "api", method, arguments, &call_error);
    if (!result)
        return luaL_error(state, "gnoblin.%s failed: %s", path, call_error->message);
    json_to_lua(state, result);
    lua_cli_push_readonly_value(state, -1);
    lua_remove(state, -2);
    return 1;
}

/* gnoblin.ease and gnoblin.lerp are pure functions; they share their curves with the runtime and
 * the compositor. */
int lua_cli_ease(lua_State* state) {
    if (lua_gettop(state) < 2 || lua_gettop(state) > 3)
        return luaL_error(state, "gnoblin.ease takes a name, a progress and an optional direction");
    const char* name = luaL_checkstring(state, 1);
    int ease = gnoblin_ease_from_name(name);
    if (ease < 0)
        return luaL_error(state, "unknown ease %s", name);
    double t = luaL_checknumber(state, 2);
    const char* direction_name = luaL_optstring(state, 3, "in_out");
    int direction = gnoblin_ease_direction_from_name(direction_name);
    if (direction < 0)
        return luaL_error(state, "unknown ease direction %s; use in, out or in_out",
                          direction_name);
    lua_pushnumber(state, gnoblin_ease(ease, direction, t));
    return 1;
}

int lua_cli_lerp(lua_State* state) {
    if (lua_gettop(state) != 3)
        return luaL_error(state, "gnoblin.lerp takes a, b and t");
    double a = luaL_checknumber(state, 1);
    double b = luaL_checknumber(state, 2);
    double t = luaL_checknumber(state, 3);
    lua_pushnumber(state, a + (b - a) * t);
    return 1;
}

/* gnoblin.find(kind, filter?): the runtime's service, over the lists that the console reads. */
int lua_cli_find(lua_State* state) {
    Cli* cli = lua_touserdata(state, lua_upvalueindex(1));
    const char* kind = luaL_checkstring(state, 1);
    if (lua_gettop(state) > 2)
        return luaL_error(state, "gnoblin.find takes a kind and an optional filter");
    const char* path = lua_find_list_path(kind);
    if (!path) {
        g_autofree char* names = lua_find_kind_names();
        return luaL_error(state, "gnoblin.find cannot list %s; kinds: %s", kind, names);
    }
    lua_settop(state, 2); /* the filter slot exists, nil when absent */
    lua_find_check(state, kind, 2);
    lua_cli_push_api_function(state, cli, path);
    lua_call(state, 0, 1);
    lua_find_select(state, -1, 2);
    return 1;
}

/* gnoblin.config holds path(), default() and restore_default() and nothing else, as in a
 * configuration. Upvalue 1 is the namespace table. */
static int config_index(lua_State* state) {
    lua_pushvalue(state, 2);
    lua_rawget(state, lua_upvalueindex(1));
    if (!lua_isnil(state, -1))
        return 1;
    return luaL_error(state, "gnoblin.config holds path(), default() and restore_default(), not "
                             "the settings. Read gnoblin.settings; a console has no config being "
                             "loaded");
}

static int config_newindex(lua_State* state) {
    return luaL_error(state, "gnoblin.config cannot be assigned to. Change settings with "
                             "gnoblin.configure in a configuration; a console has no runtime to "
                             "hold one");
}

static void push_namespace(lua_State* state, Cli* cli, const char* path) {
    lua_newtable(state);
    lua_newtable(state);
    lua_pushstring(state, path);
    lua_pushlightuserdata(state, cli);
    lua_pushcclosure(state, lua_api_index, 2);
    lua_setfield(state, -2, "__index");
    lua_setmetatable(state, -2);
}

int lua_api_index(lua_State* state) {
    const char* prefix = lua_tostring(state, lua_upvalueindex(1));
    Cli* cli = lua_touserdata(state, lua_upvalueindex(2));
    if (lua_type(state, 2) != LUA_TSTRING) {
        lua_pushnil(state);
        return 1;
    }
    const char* name = lua_tostring(state, 2);
    g_autofree char* path = *prefix ? g_strdup_printf("%s.%s", prefix, name) : g_strdup(name);

    if (g_str_equal(path, "settings")) {
        push_closure(state, cli, lua_cli_settings_property, path, path);
        lua_call(state, 0, 1);
        return 1;
    }
    if (g_str_equal(path, "configure")) {
        push_not_available_table(state, "gnoblin.configure");
        return 1;
    }
    if (g_str_equal(path, "config")) {
        /* The functions of lua_api[] under config, in a table that refuses anything else. */
        lua_newtable(state);
        int methods = lua_absindex(state, -1);
        for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
            if (g_str_has_prefix(entry->path, "config.") &&
                lua_cli_push_api_function(state, cli, entry->path))
                lua_setfield(state, methods, entry->path + strlen("config."));
        lua_newtable(state); /* the table Lua sees */
        lua_newtable(state); /* its metatable */
        lua_pushvalue(state, methods);
        lua_pushcclosure(state, config_index, 1);
        lua_setfield(state, -2, "__index");
        lua_pushcfunction(state, config_newindex);
        lua_setfield(state, -2, "__newindex");
        lua_setmetatable(state, -2);
        lua_remove(state, methods);
        return 1;
    }
    if (lua_cli_push_api_function(state, cli, path))
        return 1;
    if (is_namespace(path)) {
        push_namespace(state, cli, path);
        return 1;
    }
    return luaL_error(state, "gnoblin.%s is not part of the Lua API", path);
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
