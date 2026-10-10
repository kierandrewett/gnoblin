#include "gnoblin-lua-internal.h"

/* Named views expose the actual loaded entries, while the stored document
 * keeps the existing ordered-list representation. */
static int lua_named_entry_index(lua_State* state) {
    const char* key = luaL_checkstring(state, 2);
    g_autofree char* internal = g_strdup(key);
    g_strdelimit(internal, "_", '-');
    lua_pushstring(state, internal);
    lua_rawget(state, lua_upvalueindex(1));
    if (!strcmp(key, "enable") && lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_pushboolean(state, TRUE);
    }
    return 1;
}

static int lua_named_entry_assign(lua_State* state) {
    const char* key = luaL_checkstring(state, 2);
    if (!strcmp(key, "name"))
        return luaL_error(state, "named entry name cannot be changed");
    g_autofree char* internal = g_strdup(key);
    g_strdelimit(internal, "_", '-');
    lua_pushstring(state, internal);
    lua_pushvalue(state, 3);
    lua_rawset(state, lua_upvalueindex(1));
    return 0;
}

static int lua_named_entry_pairs(lua_State* state) {
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_pushnil(state);
    while (lua_next(state, lua_upvalueindex(1))) {
        if (lua_type(state, -2) == LUA_TSTRING) {
            g_autofree char* public_key = g_strdup(lua_tostring(state, -2));
            g_strdelimit(public_key, "-", '_');
            lua_pushvalue(state, -1);
            lua_setfield(state, entries, public_key);
        }
        lua_pop(state, 1);
    }
    lua_getfield(state, entries, "enable");
    gboolean has_enable = !lua_isnil(state, -1);
    lua_pop(state, 1);
    if (!has_enable) {
        lua_pushboolean(state, TRUE);
        lua_setfield(state, entries, "enable");
    }
    lua_getglobal(state, "next");
    lua_pushvalue(state, entries);
    lua_pushnil(state);
    return 3;
}

static void push_named_entry_proxy(lua_State* state, int entry) {
    entry = lua_absindex(state, entry);
    lua_newtable(state);
    lua_newtable(state);
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_index, 1);
    lua_setfield(state, -2, "__index");
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_assign, 1);
    lua_setfield(state, -2, "__newindex");
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_pairs, 1);
    lua_setfield(state, -2, "__pairs");
    lua_setmetatable(state, -2);
}

static int lua_named_view_index(lua_State* state) {
    const char* name = luaL_checkstring(state, 2);
    push_config_list(state);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches) {
            push_named_entry_proxy(state, -1);
            return 1;
        }
        lua_pop(state, 1);
    }
    lua_pushnil(state);
    return 1;
}

static int lua_named_view_pairs(lua_State* state) {
    push_config_list(state);
    int list = lua_gettop(state);
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        if (lua_type(state, -1) == LUA_TSTRING) {
            const char* name = lua_tostring(state, -1);
            push_named_entry_proxy(state, -2);
            lua_setfield(state, entries, name);
        }
        lua_pop(state, 2);
    }
    lua_getglobal(state, "next");
    lua_pushvalue(state, entries);
    lua_pushnil(state);
    return 3;
}

static int lua_named_view_assign(lua_State* state) {
    const char* name = luaL_checkstring(state, 2);
    luaL_checktype(state, 3, LUA_TTABLE);
    push_settings(state, 3, NULL, 0, 0);
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_pushvalue(state, -2);
    lua_setfield(state, entries, name);
    push_config_list(state);
    merge_named_entries(state, -1, entries);
    return 0;
}

void install_named_view(lua_State* state, const char* key) {
    lua_newtable(state);
    lua_newtable(state);
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_index, 2);
    lua_setfield(state, -2, "__index");
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_pairs, 2);
    lua_setfield(state, -2, "__pairs");
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_assign, 2);
    lua_setfield(state, -2, "__newindex");
    lua_setmetatable(state, -2);
    lua_setfield(state, -2, key);
}

/* Mirrors parse_autostart_entries in gnoblin-runtime.c. The session guardian rejects the same
 * entries, so a bad entry fails the reload here with the entry named, and the previous config stays
 * active. */
static void validate_autostart_entries(lua_State* state, int list, lua_Integer count) {
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        if (!lua_istable(state, -1))
            luaL_error(state, "autostart entry %d must be a table", (int)i);
        int entry = lua_gettop(state);
        lua_getfield(state, entry, "name");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1) ||
            g_utf8_strlen(lua_tostring(state, -1), -1) > 80)
            luaL_error(state, "autostart entry %d needs a name of 1 to 80 characters", (int)i);
        const char* name = lua_tostring(state, -1);
        for (lua_Integer j = 1; j < i; j++) {
            lua_rawgeti(state, list, j);
            lua_getfield(state, -1, "name");
            if (lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1)))
                luaL_error(state, "autostart name \"%s\" is used twice", name);
            lua_pop(state, 2);
        }
        lua_pushnil(state);
        while (lua_next(state, entry)) {
            if (lua_type(state, -2) != LUA_TSTRING)
                luaL_error(state, "autostart \"%s\" has a field that is not named", name);
            const char* field = lua_tostring(state, -2);
            if (strcmp(field, "name") && strcmp(field, "command") && strcmp(field, "when"))
                luaL_error(
                    state,
                    "autostart \"%s\" has an unsupported field \"%s\"; use name, command or when",
                    name, field);
            lua_pop(state, 1);
        }
        lua_getfield(state, entry, "when");
        if (!lua_isnil(state, -1) &&
            (lua_type(state, -1) != LUA_TSTRING || strcmp(lua_tostring(state, -1), "on_login")))
            luaL_error(state, "autostart \"%s\" when must be \"on_login\"", name);
        lua_pop(state, 1);
        lua_getfield(state, entry, "command");
        if (!lua_istable(state, -1) || lua_rawlen(state, -1) == 0)
            luaL_error(state, "autostart \"%s\" needs a command list with at least one item", name);
        for (lua_Integer arg = 1; arg <= (lua_Integer)lua_rawlen(state, -1); arg++) {
            lua_rawgeti(state, -1, arg);
            if (lua_type(state, -1) != LUA_TSTRING || (arg == 1 && !lua_rawlen(state, -1)))
                luaL_error(state, "autostart \"%s\" command item %d must be a string%s", name,
                           (int)arg, arg == 1 ? " that is not empty" : "");
            lua_pop(state, 1);
        }
        lua_settop(state, entry - 1);
    }
}

void finish_named_entries(lua_State* state, int config, const char* key) {
    config = lua_absindex(state, config);
    lua_getfield(state, config, key);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), next = 1;
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_pushnil(state);
        lua_setfield(state, -2, "enable");
        if (disabled)
            lua_pop(state, 1);
        else
            lua_rawseti(state, list, next++);
    }
    for (lua_Integer i = next; i <= count; i++) {
        lua_pushnil(state);
        lua_rawseti(state, list, i);
    }
    if (!strcmp(key, "autostart"))
        validate_autostart_entries(state, list, next - 1);
    lua_pop(state, 1);
}

/* Named commands merge in place; ordered rules always append. */
int lua_declare(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    push_settings(state, 1, NULL, 0, 0);
    int entry = lua_gettop(state);
    const char* name = NULL;
    if (lua_toboolean(state, lua_upvalueindex(3))) {
        lua_getfield(state, entry, "name");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
            return luaL_error(state, "declaration needs a nonempty string name");
        name = lua_tostring(state, -1);
        lua_getfield(state, entry, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            return luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_pushnil(state);
        lua_setfield(state, entry, "enable");
        if (disabled) {
            push_config_list(state);
            remove_named_entry(state, -1, name);
            return 0;
        }
    }
    push_config_list(state);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; name && i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches) {
            lua_pushnil(state);
            lua_setfield(state, -2, "enable");
            merge_table(state, -1, entry, NULL, FALSE);
            return 0;
        }
        lua_pop(state, 1);
    }
    lua_pushvalue(state, entry);
    lua_rawseti(state, list, count + 1);
    return 0;
}

static char* embedded_relative_path(const char* current_path, const char* pattern, GError** error) {
    static const char prefix[] = "/gnoblin-builtin/";
    if (g_path_is_absolute(pattern) || g_str_has_prefix(pattern, "~/")) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "embedded defaults may only load bundled relative paths");
        return NULL;
    }
    if (!g_str_has_prefix(current_path, prefix)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "embedded default config has an invalid current path");
        return NULL;
    }
    g_autofree char* current_relative = g_strdup(current_path + strlen(prefix));
    g_autofree char* directory = g_path_get_dirname(current_relative);
    g_autofree char* combined = g_build_filename(directory, pattern, NULL);
    if (strstr(combined, "..")) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "embedded default config attempted to leave its bundle");
        return NULL;
    }
    while (g_str_has_prefix(combined, "./"))
        memmove(combined, combined + 2, strlen(combined + 2) + 1);
    return g_steal_pointer(&combined);
}

static gboolean load_embedded_pattern(lua_State* state, LuaConfig* config, const char* pattern,
                                      GError** error) {
    g_autofree char* relative = embedded_relative_path(config->current_path, pattern, error);
    if (!relative)
        return FALSE;
    gsize count = 0;
    const GnoblinConfigDefaultFile* files = gnoblin_config_default_files(&count);
    gboolean matched = FALSE;
    for (gsize i = 0; i < count; i++) {
        if (fnmatch(relative, files[i].path, FNM_PATHNAME) != 0)
            continue;
        matched = TRUE;
        if (!evaluate_embedded_path(state, config, files[i].path, error))
            return FALSE;
    }
    if (!matched) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_NOENT,
                    "embedded default config file does not exist: %s", relative);
        return FALSE;
    }
    return TRUE;
}

int lua_config_load(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    if (config->ignore_config_loads) {
        lua_pushboolean(state, TRUE);
        return 1;
    }
    const char* pattern = luaL_checkstring(state, 1);
    g_autoptr(GError) error = NULL;
    if (config->embedded_defaults) {
        if (!load_embedded_pattern(state, config, pattern, &error))
            return luaL_error(state, "%s", error->message);
        lua_pushboolean(state, TRUE);
        return 1;
    }
    g_autoptr(GPtrArray) paths =
        gnoblin_config_expand_paths(config->current_path, pattern, config->directories, &error);
    if (!paths)
        return luaL_error(state, "%s", error->message);
    for (guint i = 0; i < paths->len; i++) {
        if (evaluate_path(state, config, g_ptr_array_index(paths, i), FALSE, &error))
            continue;
        if (!config->skip_failed_files)
            return luaL_error(state, "%s", error->message);
        if (!config->skipped_files)
            config->skipped_files = g_ptr_array_new_with_free_func(g_free);
        g_ptr_array_add(config->skipped_files, g_strdup_printf("ignored file %s", error->message));
        g_clear_error(&error);
    }
    lua_pushboolean(state, TRUE);
    return 1;
}
