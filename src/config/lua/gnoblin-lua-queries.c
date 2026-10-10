#include "gnoblin-lua-internal.h"

static gboolean safe_module_name(const char* name) {
    if (!name[0] || strstr(name, ".."))
        return FALSE;
    for (const char* p = name; *p; p++)
        if (!g_ascii_isalnum(*p) && *p != '_' && *p != '-' && *p != '.')
            return FALSE;
    return TRUE;
}

int lua_require(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    const char* name = luaL_checkstring(state, 1);
    if (!strcmp(name, "gnoblin")) {
        lua_getglobal(state, "gnoblin");
        return 1;
    }
    if (!safe_module_name(name))
        return luaL_error(state, "invalid Lua module name: %s", name);
    gpointer saved = g_hash_table_lookup(config->modules, name);
    if (saved) {
        lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(saved));
        return 1;
    }
    g_autofree char* relative = g_strconcat(name, ".lua", NULL);
    g_autofree char* path = resolve_path(config->current_path, relative);
    if (!g_file_test(path, G_FILE_TEST_IS_REGULAR)) {
        g_autofree char* under_lua = g_build_filename("lua", relative, NULL);
        g_free(path);
        path = resolve_path(config->current_path, under_lua);
    }
    add_path(config->paths, path);
    if (!g_file_test(path, G_FILE_TEST_IS_REGULAR))
        return luaL_error(state, "Lua module not found: %s", name);
    if (g_hash_table_contains(config->active, path))
        return luaL_error(state, "%s: config cycle", path);
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    int status = luaL_loadfilex(state, path, "t");
    if (status == LUA_OK)
        status = lua_pcall(state, 0, 1, 0);
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    if (status != LUA_OK)
        return lua_error(state);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_pushboolean(state, TRUE);
    }
    int reference = luaL_ref(state, LUA_REGISTRYINDEX);
    g_hash_table_insert(config->modules, g_strdup(name), GINT_TO_POINTER(reference));
    lua_rawgeti(state, LUA_REGISTRYINDEX, reference);
    return 1;
}

static GVariant* lua_window_array(LuaConfig* config) {
    if (!config || !config->window_snapshot)
        return NULL;
    return g_variant_lookup_value(config->window_snapshot, "windows", G_VARIANT_TYPE("av"));
}

static gboolean focus_history_contains(GPtrArray* history, const char* id) {
    for (guint i = 0; history && i < history->len; i++)
        if (g_str_equal(g_ptr_array_index(history, i), id))
            return TRUE;
    return FALSE;
}

static void focus_history_touch(LuaConfig* config, const char* id) {
    if (!config->focus_history_ids)
        config->focus_history_ids = g_ptr_array_new_with_free_func(g_free);
    for (guint i = 0; i < config->focus_history_ids->len; i++) {
        if (g_str_equal(g_ptr_array_index(config->focus_history_ids, i), id)) {
            g_ptr_array_remove_index(config->focus_history_ids, i);
            break;
        }
    }
    g_ptr_array_insert(config->focus_history_ids, 0, g_strdup(id));
}

static void focus_history_remove(LuaConfig* config, const char* id) {
    if (!config->focus_history_ids)
        return;
    for (guint i = config->focus_history_ids->len; i > 0; i--)
        if (g_str_equal(g_ptr_array_index(config->focus_history_ids, i - 1), id))
            g_ptr_array_remove_index(config->focus_history_ids, i - 1);
}

void focus_history_reconcile_snapshot(LuaConfig* config, GVariant* snapshot) {
    if (!config->focus_history_ids)
        config->focus_history_ids = g_ptr_array_new_with_free_func(g_free);
    if (!snapshot) {
        g_ptr_array_set_size(config->focus_history_ids, 0);
        return;
    }

    g_autoptr(GVariant) windows = g_variant_lookup_value(snapshot, "windows", G_VARIANT_TYPE("av"));
    if (!windows) {
        g_ptr_array_set_size(config->focus_history_ids, 0);
        return;
    }

    g_autoptr(GHashTable) live_ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autofree char* focused_id = NULL;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        const char* id = NULL;
        gboolean focused = FALSE;
        if (!g_variant_lookup(window, "id", "&s", &id) || !id || !*id)
            continue;
        if (!g_hash_table_contains(live_ids, id)) {
            g_hash_table_add(live_ids, g_strdup(id));
            if (!focus_history_contains(config->focus_history_ids, id))
                g_ptr_array_add(config->focus_history_ids, g_strdup(id));
        }
        if (!focused_id && g_variant_lookup(window, "focused", "b", &focused) && focused)
            focused_id = g_strdup(id);
    }

    for (guint i = config->focus_history_ids->len; i > 0; i--) {
        const char* id = g_ptr_array_index(config->focus_history_ids, i - 1);
        if (!g_hash_table_contains(live_ids, id))
            g_ptr_array_remove_index(config->focus_history_ids, i - 1);
    }
    if (focused_id)
        focus_history_touch(config, focused_id);
}

static gboolean focus_history_snapshot_has_id(LuaConfig* config, const char* id) {
    g_autoptr(GVariant) windows = lua_window_array(config);
    for (gsize i = 0; windows && i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        const char* window_id = NULL;
        if (g_variant_lookup(window, "id", "&s", &window_id) && g_str_equal(window_id, id))
            return TRUE;
    }
    return FALSE;
}

void focus_history_update_event(LuaConfig* config, const char* event, GVariant* payload) {
    const char* id = NULL;
    if (g_str_equal(event, "gnoblin.window.closed")) {
        if (g_variant_lookup(payload, "window_id", "&s", &id) && id && *id)
            focus_history_remove(config, id);
        return;
    }
    if (!g_str_equal(event, "gnoblin.window.focused") ||
        !g_variant_lookup(payload, "window_id", "&s", &id) || !id || !*id)
        return;

    g_autoptr(GVariant) window = g_variant_lookup_value(payload, "window", G_VARIANT_TYPE_VARDICT);
    const char* record_id = NULL;
    gboolean focused = FALSE;
    if (window && g_variant_lookup(window, "id", "&s", &record_id) && g_str_equal(record_id, id) &&
        g_variant_lookup(window, "focused", "b", &focused) && focused &&
        focus_history_snapshot_has_id(config, id))
        focus_history_touch(config, id);
}

static gboolean focus_history_matches_filter(GVariant* window, lua_State* state, int filter) {
    static const char* fields[] = {"workspace_id", "monitor_id"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            const char* expected = lua_tostring(state, -1);
            const char* actual = NULL;
            gboolean matches = expected && g_variant_lookup(window, fields[i], "&s", &actual) &&
                               g_str_equal(actual, expected);
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

int lua_focus_history(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.focus.history takes no arguments or one filter table");
    lua_Integer limit = 50;
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"workspace_id", "monitor_id", "limit"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || (g_str_equal(key, "limit")
                               ? (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                                  lua_tointeger(state, -1) > 256)
                               : lua_type(state, -1) != LUA_TSTRING))
                return luaL_error(state, "invalid gnoblin.focus.history filter field or value");
            lua_pop(state, 1);
        }
        lua_getfield(state, 1, "limit");
        if (!lua_isnil(state, -1))
            limit = lua_tointeger(state, -1);
        lua_pop(state, 1);
    }
    if (!config || !config->window_snapshot || !config->focus_history_ids)
        return luaL_error(state, "native focus history is unavailable");

    g_autoptr(GVariant) windows = lua_window_array(config);
    guint result_limit = (guint)limit;
    lua_newtable(state);
    mark_array_table(state, -1);
    guint result_index = 1;
    for (guint history_index = 0;
         windows && history_index < config->focus_history_ids->len && result_index <= result_limit;
         history_index++) {
        const char* wanted_id = g_ptr_array_index(config->focus_history_ids, history_index);
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* id = NULL;
            if (!g_variant_lookup(window, "id", "&s", &id) || !g_str_equal(id, wanted_id))
                continue;
            if (!lua_istable(state, 1) || focus_history_matches_filter(window, state, 1)) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                lua_rawseti(state, -2, result_index++);
            }
            break;
        }
    }
    return 1;
}

static gboolean window_matches_filter(GVariant* window, lua_State* state, int filter) {
    static const char* fields[] = {"app_id", "title", "focused", "workspace_id", "monitor_id"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            g_autoptr(GVariant) value = g_variant_lookup_value(window, fields[i], NULL);
            gboolean matches = FALSE;
            if (i == 2) {
                matches = value && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                          lua_isboolean(state, -1) &&
                          g_variant_get_boolean(value) == lua_toboolean(state, -1);
            } else if (lua_type(state, -1) == LUA_TSTRING && value &&
                       g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* expected = lua_tostring(state, -1);
                const char* actual = g_variant_get_string(value, NULL);
                if (i == 1) {
                    g_autofree char* folded_expected = g_utf8_casefold(expected, -1);
                    g_autofree char* folded_actual = g_utf8_casefold(actual, -1);
                    matches = folded_expected && folded_actual &&
                              strstr(folded_actual, folded_expected) != NULL;
                } else {
                    matches = g_str_equal(actual, expected);
                }
            }
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

int lua_windows_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.windows.list takes no arguments or one filter table");
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"app_id", "title", "focused", "workspace_id", "monitor_id"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_tostring(state, -2);
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || (g_str_equal(key, "focused") ? !lua_isboolean(state, -1)
                                                       : lua_type(state, -1) != LUA_TSTRING))
                return luaL_error(state, "invalid gnoblin.windows.list filter field or value");
            lua_pop(state, 1);
        }
    }
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");

    g_autoptr(GVariant) windows = lua_window_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    if (!windows)
        return 1;
    guint result_index = 1;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        if (lua_istable(state, 1) && !window_matches_filter(window, state, 1))
            continue;
        push_variant(state, window);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, result_index++);
    }
    return 1;
}

int lua_windows_focused(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.windows.focused takes no arguments");
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");
    g_autoptr(GVariant) windows = lua_window_array(config);
    if (windows) {
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            gboolean focused = FALSE;
            if (g_variant_lookup(window, "focused", "b", &focused) && focused) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

int lua_windows_by_id(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.windows.by_id requires one string ID");
    const char* id = lua_tostring(state, 1);
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");
    g_autoptr(GVariant) windows = lua_window_array(config);
    if (windows) {
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* window_id = NULL;
            if (g_variant_lookup(window, "id", "&s", &window_id) && g_str_equal(id, window_id)) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static GVariant* lua_workspace_array(LuaConfig* config) {
    if (!config || !config->workspace_snapshot)
        return NULL;
    return g_variant_lookup_value(config->workspace_snapshot, "workspaces", G_VARIANT_TYPE("av"));
}

int lua_workspaces_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.list takes no arguments");
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    if (!workspaces)
        return 1;
    for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
        g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                            ? g_variant_get_variant(wrapped)
                                            : g_variant_ref(wrapped);
        push_variant(state, workspace);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_workspaces_active(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.active takes no arguments");
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    if (workspaces) {
        for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                                ? g_variant_get_variant(wrapped)
                                                : g_variant_ref(wrapped);
            gboolean active = FALSE;
            if (g_variant_lookup(workspace, "active", "b", &active) && active) {
                push_variant(state, workspace);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

int lua_workspaces_by_id(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.workspaces.by_id requires one string ID");
    const char* id = lua_tostring(state, 1);
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    if (workspaces) {
        for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                                ? g_variant_get_variant(wrapped)
                                                : g_variant_ref(wrapped);
            const char* workspace_id = NULL;
            if (g_variant_lookup(workspace, "id", "&s", &workspace_id) &&
                g_str_equal(id, workspace_id)) {
                push_variant(state, workspace);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static GVariant* lua_monitor_array(LuaConfig* config) {
    if (!config || !config->monitor_snapshot)
        return NULL;
    return g_variant_lookup_value(config->monitor_snapshot, "monitors", G_VARIANT_TYPE("av"));
}

int lua_monitors_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.list takes no arguments");
    if (!config || !config->monitor_snapshot)
        return luaL_error(state, "native monitor snapshot is unavailable");

    g_autoptr(GVariant) monitors = lua_monitor_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; monitors && i < g_variant_n_children(monitors); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(monitors, i);
        g_autoptr(GVariant) monitor = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(wrapped)
                                          : g_variant_ref(wrapped);
        push_variant(state, monitor);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_monitors_primary(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.primary takes no arguments");
    if (!config || !config->monitor_snapshot)
        return luaL_error(state, "native monitor snapshot is unavailable");

    g_autoptr(GVariant) monitors = lua_monitor_array(config);
    for (gsize i = 0; monitors && i < g_variant_n_children(monitors); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(monitors, i);
        g_autoptr(GVariant) monitor = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(wrapped)
                                          : g_variant_ref(wrapped);
        gboolean primary = FALSE;
        if (g_variant_lookup(monitor, "primary", "b", &primary) && primary) {
            push_variant(state, monitor);
            push_readonly_copy(state, -1);
            lua_remove(state, -2);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

int lua_monitors_privacy_screen(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.privacy_screen takes no arguments");
    if (!config || !config->monitor_privacy_screen_snapshot)
        return luaL_error(state, "native monitor privacy-screen snapshot is unavailable");
    push_variant(state, config->monitor_privacy_screen_snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_monitors_set_privacy_screen(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (config && config->api_calling && lua_gettop(state) == 1 && lua_istable(state, 1)) {
        static const char* const fields[] = {"value", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "monitors.set_privacy_screen accepts only value");
        lua_getfield(state, 1, "value");
        lua_replace(state, 1);
        lua_settop(state, 1);
    }
    if (lua_gettop(state) != 1 ||
        (!lua_isboolean(state, 1) &&
         !(lua_type(state, 1) == LUA_TSTRING && g_str_equal(lua_tostring(state, 1), "inherit"))))
        return luaL_error(state,
                          "gnoblin.monitors.set_privacy_screen requires true, false, or 'inherit'");

    lua_newtable(state);
    lua_pushvalue(state, 1);
    lua_setfield(state, -2, "value");
    int arguments = lua_absindex(state, -1);
    return call_launch_operation(state, "monitors.set_privacy_screen", arguments);
}

static GVariant* lua_layer_array(LuaConfig* config) {
    if (!config || !config->layer_snapshot)
        return NULL;
    return g_variant_lookup_value(config->layer_snapshot, "layers", G_VARIANT_TYPE("av"));
}

static gboolean layer_matches_filter(GVariant* layer, lua_State* state, int filter) {
    static const char* fields[] = {"monitor_id", "namespace", "layer"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            const char* expected = lua_tostring(state, -1);
            const char* actual = NULL;
            gboolean matches = expected && g_variant_lookup(layer, fields[i], "&s", &actual) &&
                               g_str_equal(actual, expected);
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

int lua_layers_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.layers.list takes no arguments or one filter table");
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"monitor_id", "namespace", "layer"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || lua_type(state, -1) != LUA_TSTRING)
                return luaL_error(state, "invalid gnoblin.layers.list filter field or value");
            lua_pop(state, 1);
        }
    }
    if (!config || !config->layer_snapshot)
        return luaL_error(state, "native layer snapshot is unavailable");

    g_autoptr(GVariant) layers = lua_layer_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    guint result_index = 1;
    for (gsize i = 0; layers && i < g_variant_n_children(layers); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(layers, i);
        g_autoptr(GVariant) layer = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                        ? g_variant_get_variant(wrapped)
                                        : g_variant_ref(wrapped);
        if (lua_istable(state, 1) && !layer_matches_filter(layer, state, 1))
            continue;
        push_variant(state, layer);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, result_index++);
    }
    return 1;
}
