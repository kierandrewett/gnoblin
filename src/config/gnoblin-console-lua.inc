/* Persistent console state. Shares the config API, but never applies settings. */
#include "gnoblin-console-lua.h"

static lua_State *console_lua;
static LuaConfig console_config;
static GPtrArray *console_lines;
static const char *console_source;

static int console_print(lua_State *state) {
    if (console_lines->len >= 200)
        return 0;
    GString *line = g_string_new(NULL);
    for (int i = 1; i <= lua_gettop(state); i++) {
        if (i > 1)
            g_string_append_c(line, '\t');
        const char *text = lua_tostring(state, i);
        if (text)
            g_string_append_len(line, text, MIN(strlen(text), 4096));
        else if (lua_isboolean(state, i))
            g_string_append(line, lua_toboolean(state, i) ? "true" : "false");
        else
            g_string_append(line, luaL_typename(state, i));
        if (line->len >= 8192)
            break;
    }
    g_ptr_array_add(console_lines, g_utf8_make_valid(line->str, line->len));
    g_string_free(line, TRUE);
    return 0;
}

static int console_traceback(lua_State *state) {
    luaL_traceback(state, state, lua_tostring(state, 1), 1);
    return 1;
}

static int console_init(lua_State *state) {
    install_api(state, &console_config);
    lua_pushcfunction(state, console_print);
    lua_setglobal(state, "print");
    GError *error = NULL;
    if (!evaluate_path(state, &console_config, console_config.current_path, TRUE, &error)) {
        lua_pushstring(state, error->message);
        g_clear_error(&error);
        return lua_error(state);
    }
    return 0;
}

static int console_execute(lua_State *state) {
    const char *source = console_source;
    lua_pushstring(state, source);
    lua_pushfstring(state, "return %s", source);
    int status = luaL_loadbufferx(state, lua_tostring(state, -1),
                                  lua_rawlen(state, -1), "=console", "t");
    if (status != LUA_OK) {
        lua_pop(state, 1);
        status = luaL_loadbufferx(state, source, strlen(source), "=console", "t");
    }
    if (status != LUA_OK)
        return lua_error(state);
    lua_call(state, 0, LUA_MULTRET);
    /* Source and attempted expression occupy the first two stack slots. */
    return lua_gettop(state) - 2;
}

static int console_complete(lua_State *state) {
    const char *source = console_source;
    g_auto(GStrv) parts = g_strsplit(source, ".", -1);
    lua_pushglobaltable(state);
    guint last = g_strv_length(parts) - 1;
    for (guint i = 0; i < last; i++) {
        if (!lua_istable(state, -1))
            return 0;
        lua_pushstring(state, parts[i]);
        lua_rawget(state, -2);
        lua_remove(state, -2);
    }
    if (!lua_istable(state, -1))
        return 0;
    lua_pushnil(state);
    while (lua_next(state, -2)) {
        if (lua_type(state, -2) == LUA_TSTRING) {
            const char *key = lua_tostring(state, -2);
            if (g_str_has_prefix(key, parts[last]) && console_lines->len < 200)
                g_ptr_array_add(console_lines, g_utf8_make_valid(key, -1));
        }
        lua_pop(state, 1);
    }
    return 0;
}

GVariant *gnoblin_console_lua(const char *operation, const char *source) {
    GVariantBuilder result, values;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&values, G_VARIANT_TYPE_STRING_ARRAY);
    if (!strcmp(operation, "reset")) {
        if (console_lua)
            lua_close(console_lua);
        console_lua = NULL;
        g_clear_pointer(&console_config.paths, g_ptr_array_unref);
        g_clear_pointer(&console_config.active, g_hash_table_unref);
        g_clear_pointer(&console_config.modules, g_hash_table_unref);
        g_clear_pointer(&console_config.current_path, g_free);
        console_config.allocated = 0;
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    console_lines = g_ptr_array_new_with_free_func(g_free);
    int status = LUA_OK;
    gboolean initializing = !console_lua;
    if (initializing) {
        console_config.paths = g_ptr_array_new_with_free_func(g_free);
        console_config.active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        console_config.modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        console_config.current_path = gnoblin_config_path();
        console_lua = lua_newstate(limited_alloc, &console_config);
    }
    if (!console_lua) {
        g_variant_builder_add(&result, "{sv}", "error", g_variant_new_string("Lua memory limit exceeded"));
        goto done;
    }
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(console_lua), &steps, sizeof steps);
    lua_sethook(console_lua, limit_hook, LUA_MASKCOUNT, 1000);
    lua_settop(console_lua, 0);
    lua_pushcfunction(console_lua, console_traceback);
    if (initializing) {
        lua_pushcfunction(console_lua, console_init);
        status = lua_pcall(console_lua, 0, 0, 1);
    }
    if (status == LUA_OK) {
        lua_pushcfunction(console_lua, !strcmp(operation, "complete") ? console_complete : console_execute);
        console_source = source;
        status = lua_pcall(console_lua, 0, LUA_MULTRET, 1);
        console_source = NULL;
    }
    if (status != LUA_OK) {
        g_autofree char *error = g_utf8_make_valid(lua_error_text(console_lua), -1);
        g_variant_builder_add(&result, "{sv}", "error", g_variant_new_string(error));
    } else {
        for (int i = 2; i <= lua_gettop(console_lua); i++) {
            g_autofree char *number = NULL;
            if (lua_isinteger(console_lua, i))
                number = g_strdup_printf("%" G_GINT64_FORMAT, (gint64)lua_tointeger(console_lua, i));
            else if (lua_type(console_lua, i) == LUA_TNUMBER)
                number = g_strdup_printf("%.17g", lua_tonumber(console_lua, i));
            const char *value = number ? number : (lua_type(console_lua, i) == LUA_TSTRING ? lua_tostring(console_lua, i) : NULL);
            if (!value)
                value = lua_isboolean(console_lua, i) ? (lua_toboolean(console_lua, i) ? "true" : "false") : luaL_typename(console_lua, i);
            g_autofree char *valid = g_utf8_make_valid(value, -1);
            g_variant_builder_add(&values, "s", valid);
        }
    }
    lua_settop(console_lua, 0);
    lua_sethook(console_lua, NULL, 0, 0);
done:
    g_variant_builder_add(&result, "{sv}", "values", g_variant_builder_end(&values));
    g_variant_builder_add(&result, "{sv}", "lines", g_variant_new_strv((const char *const *)console_lines->pdata, console_lines->len));
    g_clear_pointer(&console_lines, g_ptr_array_unref);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
