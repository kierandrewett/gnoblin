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

/* Handles keep returned objects alive, including cycles and non-string keys.
 * They are bounded and released with the console state. No metamethod runs. */
static GHashTable *console_handles;
static GVariant *console_details;

static char *console_preview(lua_State *state, int index) {
    switch (lua_type(state, index)) {
    case LUA_TSTRING: {
        size_t length;
        const char *text = lua_tolstring(state, index, &length);
        g_autofree char *valid = g_utf8_make_valid(text, MIN(length, 240));
        g_autofree char *escaped = g_strescape(valid, NULL);
        return g_strdup_printf("\"%s%s\"", escaped, length > 240 ? "…" : "");
    }
    case LUA_TNUMBER:
        return lua_isinteger(state, index) ? g_strdup_printf("%" G_GINT64_FORMAT, (gint64)lua_tointeger(state, index))
            : g_strdup_printf("%.17g", lua_tonumber(state, index));
    case LUA_TBOOLEAN: return g_strdup(lua_toboolean(state, index) ? "true" : "false");
    default: return g_strdup(luaL_typename(state, index));
    }
}

static GVariant *console_describe(lua_State *state, int index) {
    index = lua_absindex(state, index);
    GVariantBuilder value;
    g_variant_builder_init(&value, G_VARIANT_TYPE_VARDICT);
    g_autofree char *summary = console_preview(state, index);
    if (lua_istable(state, index)) {
        GString *text = g_string_new("table {");
        int count = 0;
        lua_pushnil(state);
        while (lua_next(state, index)) {
            if (count == 4) { lua_pop(state, 2); g_string_append(text, ", …"); break; }
            g_autofree char *key = console_preview(state, -2);
            g_autofree char *item = console_preview(state, -1);
            g_string_append_printf(text, "%s%s = %s", count++ ? ", " : "", key, item);
            lua_pop(state, 1);
        }
        g_string_append_c(text, '}');
        g_free(summary);
        summary = g_string_free(text, FALSE);
    }
    g_variant_builder_add(&value, "{sv}", "preview", g_variant_new_string(summary));
    g_variant_builder_add(&value, "{sv}", "kind", g_variant_new_string(luaL_typename(state, index)));
    if (lua_istable(state, index) || lua_isfunction(state, index) || lua_isuserdata(state, index) || lua_isthread(state, index)) {
        const void *identity = lua_topointer(state, index);
        int handle = GPOINTER_TO_INT(g_hash_table_lookup(console_handles, identity));
        if (!handle && g_hash_table_size(console_handles) < 4096) {
            lua_pushvalue(state, index);
            handle = luaL_ref(state, LUA_REGISTRYINDEX);
            g_hash_table_insert(console_handles, (gpointer)identity, GINT_TO_POINTER(handle));
        }
        g_variant_builder_add(&value, "{sv}", "handle", g_variant_new_int32(handle));
        if (!handle)
            g_variant_builder_add(&value, "{sv}", "unavailable", g_variant_new_string("Inspection handle limit reached; reset the console to release objects"));
    }
    return g_variant_builder_end(&value);
}

static void console_property(lua_State *state, GVariantBuilder *rows, const char *name, int index) {
    GVariantBuilder row;
    g_variant_builder_init(&row, G_VARIANT_TYPE_VARDICT);
    g_autofree char *valid = g_utf8_make_valid(name, -1);
    g_variant_builder_add(&row, "{sv}", "name", g_variant_new_string(valid));
    g_variant_builder_add(&row, "{sv}", "value", console_describe(state, index));
    g_variant_builder_add_value(rows, g_variant_builder_end(&row));
}

static int console_capture(lua_State *state) {
    GVariantBuilder details;
    g_variant_builder_init(&details, G_VARIANT_TYPE("aa{sv}"));
    for (int i = 1; i <= lua_gettop(state); i++)
        g_variant_builder_add_value(&details, console_describe(state, i));
    console_details = g_variant_ref_sink(g_variant_builder_end(&details));
    return 0;
}

static int console_inspect(lua_State *state) {
    char *end;
    int handle = (int)g_ascii_strtoll(console_source, &end, 10);
    int offset = *end == ':' ? MAX(0, (int)g_ascii_strtoll(end + 1, NULL, 10)) : 0;
    lua_rawgeti(state, LUA_REGISTRYINDEX, handle);
    if (!(lua_istable(state, -1) || lua_isfunction(state, -1) || lua_isuserdata(state, -1) || lua_isthread(state, -1)))
        return luaL_error(state, "Object is no longer available");
    GVariantBuilder rows;
    g_variant_builder_init(&rows, G_VARIANT_TYPE("aa{sv}"));
    if (lua_istable(state, 1)) {
        int count = 0;
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            if (count >= offset + 100) {
                lua_pop(state, 2);
                GVariantBuilder more;
                g_variant_builder_init(&more, G_VARIANT_TYPE_VARDICT);
                g_variant_builder_add(&more, "{sv}", "name", g_variant_new_string("Show next properties"));
                g_variant_builder_add(&more, "{sv}", "more", g_variant_new_int32(count));
                g_variant_builder_add_value(&rows, g_variant_builder_end(&more));
                break;
            }
            if (count++ >= offset) {
                GVariantBuilder row;
                g_variant_builder_init(&row, G_VARIANT_TYPE_VARDICT);
                g_autofree char *key = console_preview(state, -2);
                g_variant_builder_add(&row, "{sv}", "name", g_variant_new_string(key));
                g_variant_builder_add(&row, "{sv}", "key", console_describe(state, -2));
                g_variant_builder_add(&row, "{sv}", "value", console_describe(state, -1));
                g_variant_builder_add_value(&rows, g_variant_builder_end(&row));
            }
            lua_pop(state, 1);
        }
    } else if (lua_isfunction(state, 1)) {
        lua_Debug info;
        lua_pushvalue(state, 1);
        lua_getinfo(state, ">Su", &info);
        lua_pushstring(state, info.short_src);
        console_property(state, &rows, "[[Source]]", -1); lua_pop(state, 1);
        lua_pushinteger(state, info.linedefined);
        console_property(state, &rows, "[[Line]]", -1); lua_pop(state, 1);
        for (int i = offset + 1; i <= MIN(info.nups, offset + 100); i++) {
            const char *name = lua_getupvalue(state, 1, i);
            if (!name) break;
            console_property(state, &rows, *name ? name : "[[Upvalue]]", -1);
            lua_pop(state, 1);
        }
    }
    if (offset == 0 && lua_getmetatable(state, 1)) {
        console_property(state, &rows, "[[Metatable]]", -1);
        lua_pop(state, 1);
    }
    console_details = g_variant_ref_sink(g_variant_builder_end(&rows));
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
        g_clear_pointer(&console_handles, g_hash_table_unref);
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    console_lines = g_ptr_array_new_with_free_func(g_free);
    g_clear_pointer(&console_details, g_variant_unref);
    int status = LUA_OK;
    gboolean initializing = !console_lua;
    if (initializing) {
        console_handles = g_hash_table_new(g_direct_hash, g_direct_equal);
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
        lua_pushcfunction(console_lua, !strcmp(operation, "inspect") ? console_inspect : !strcmp(operation, "complete") ? console_complete : console_execute);
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
    if (status == LUA_OK && !strcmp(operation, "eval")) {
        int count = lua_gettop(console_lua) - 1;
        lua_pushcfunction(console_lua, console_capture);
        for (int i = 2; i <= count + 1; i++)
            lua_pushvalue(console_lua, i);
        if (lua_pcall(console_lua, count, 0, 1) != LUA_OK) {
            g_autofree char *error = g_utf8_make_valid(lua_error_text(console_lua), -1);
            g_variant_builder_add(&result, "{sv}", "inspectionError", g_variant_new_string(error));
        }
    }
    if (console_details) {
        g_variant_builder_add(&result, "{sv}", "details", console_details);
        g_clear_pointer(&console_details, g_variant_unref);
    }
    lua_settop(console_lua, 0);
    lua_sethook(console_lua, NULL, 0, 0);
done:
    g_variant_builder_add(&result, "{sv}", "values", g_variant_builder_end(&values));
    g_variant_builder_add(&result, "{sv}", "lines", g_variant_new_strv((const char *const *)console_lines->pdata, console_lines->len));
    g_clear_pointer(&console_lines, g_ptr_array_unref);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
