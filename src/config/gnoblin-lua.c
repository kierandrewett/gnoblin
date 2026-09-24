/* Restricted, single-state Lua configuration evaluator. */
#include "gnoblin-config.h"

#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <math.h>
#include <string.h>

#define MAX_CONFIG_BYTES (8 * 1024 * 1024)
#define MAX_CONFIG_STEPS 1000000
#define MAX_CONFIG_DEPTH 64
#define MAX_CONFIG_FILES 32

typedef struct {
    gsize allocated;
    GPtrArray *paths, *directories;
    GHashTable *active, *modules;
    char* current_path;
} LuaConfig;

static gboolean append_array_key(const char* key) {
    return key && (!strcmp(key, "autostart") || !strcmp(key, "window-rules") ||
                   !strcmp(key, "shortcuts") || !strcmp(key, "rules") ||
                   !strcmp(key, "workspace-names") || !strcmp(key, "xkb-options") ||
                   !strcmp(key, "sources"));
}

static void* limited_alloc(void* opaque, void* pointer, size_t old, size_t size) {
    LuaConfig* config = opaque;
    if (!pointer)
        old = 0;
    if (!size) {
        config->allocated -= MIN(config->allocated, old);
        g_free(pointer);
        return NULL;
    }
    if (size > old && size - old > MAX_CONFIG_BYTES - config->allocated)
        return NULL;
    pointer = g_try_realloc(pointer, size);
    if (pointer)
        config->allocated = config->allocated - old + size;
    return pointer;
}

static lua_State* new_config_state(LuaConfig* config) {
#if LUA_VERSION_NUM >= 505
    return lua_newstate(limited_alloc, config, g_random_int());
#else
    return lua_newstate(limited_alloc, config);
#endif
}

static void limit_hook(lua_State* state, lua_Debug* debug) {
    int* remaining = (int*)lua_getextraspace(state);
    (void)debug;
    if (--*remaining <= 0)
        luaL_error(state, "configuration instruction limit exceeded");
}

static void add_path(GPtrArray* paths, const char* path) {
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    for (guint i = 0; i < paths->len; i++)
        if (!strcmp(g_ptr_array_index(paths, i), canonical))
            return;
    g_ptr_array_add(paths, g_steal_pointer(&canonical));
}

static char* resolve_path(const char* current, const char* given) {
    if (g_str_has_prefix(given, "~/"))
        return g_canonicalize_filename(given + 2, g_get_home_dir());
    if (g_path_is_absolute(given))
        return g_canonicalize_filename(given, NULL);
    g_autofree char* directory = g_path_get_dirname(current);
    return g_canonicalize_filename(given, directory);
}

static LuaConfig* config_from_upvalue(lua_State* state) {
    return lua_touserdata(state, lua_upvalueindex(1));
}
static const char* lua_error_text(lua_State* state) {
    return lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1)
                                              : "Lua configuration failed";
}

static gboolean marked_array(lua_State* state, int index) {
    gboolean result = FALSE;
    if (!lua_getmetatable(state, index))
        return FALSE;
    lua_pushstring(state, "__gnoblin_array");
    lua_rawget(state, -2);
    result = lua_toboolean(state, -1);
    lua_pop(state, 2);
    return result;
}

static GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
                                  int keybinding_depth, GError** error) {
    int type = lua_type(state, index);
    if (depth > MAX_CONFIG_DEPTH) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config nesting exceeds 64 levels");
        return NULL;
    }
    if (type == LUA_TBOOLEAN)
        return g_variant_new_boolean(lua_toboolean(state, index));
    if (type == LUA_TNUMBER) {
        if (lua_isinteger(state, index))
            return g_variant_new_int64(lua_tointeger(state, index));
        if (!isfinite(lua_tonumber(state, index))) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config numbers must be finite");
            return NULL;
        }
        return g_variant_new_double(lua_tonumber(state, index));
    }
    if (type == LUA_TSTRING) {
        size_t length;
        const char* text = lua_tolstring(state, index, &length);
        if (memchr(text, '\0', length) || !g_utf8_validate(text, length, NULL)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config strings must be UTF-8 without NUL bytes");
            return NULL;
        }
        return g_variant_new_string(text);
    }
    if (type != LUA_TTABLE) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua config cannot contain %s values",
                    lua_typename(state, type));
        return NULL;
    }

    index = lua_absindex(state, index);
    gboolean strings = FALSE, numbers = FALSE;
    lua_pushnil(state);
    while (lua_next(state, index)) {
        if (lua_type(state, -2) == LUA_TSTRING)
            strings = TRUE;
        else if (lua_isinteger(state, -2) && lua_tointeger(state, -2) > 0)
            numbers = TRUE;
        else {
            lua_pop(state, 1);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config table keys must be strings or positive integers");
            return NULL;
        }
        lua_pop(state, 1);
    }
    if (strings && numbers) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config tables cannot mix map and array keys");
        return NULL;
    }
    gboolean array = marked_array(state, index) || numbers || (!strings && force_array);
    if (array) {
        lua_Integer count = lua_rawlen(state, index);
        GVariantBuilder out;
        g_variant_builder_init(&out, G_VARIANT_TYPE("av"));
        lua_pushnil(state);
        while (lua_next(state, index)) {
            gboolean valid = lua_isinteger(state, -2) && lua_tointeger(state, -2) >= 1 &&
                             lua_tointeger(state, -2) <= count;
            lua_pop(state, 1);
            if (!valid) {
                g_variant_builder_clear(&out);
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua config arrays must be dense and start at 1");
                return NULL;
            }
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, index, i);
            g_autoptr(GVariant) child = variant_from_lua(state, -1, depth + 1, FALSE, FALSE, error);
            lua_pop(state, 1);
            if (!child) {
                g_variant_builder_clear(&out);
                return NULL;
            }
            g_variant_builder_add(&out, "v", g_variant_ref(child));
        }
        return g_variant_builder_end(&out);
    }
    GVariantBuilder out;
    g_variant_builder_init(&out, G_VARIANT_TYPE_VARDICT);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        const char* key = lua_tostring(state, -2);
        gboolean child_array = append_array_key(key) || keybinding_depth == 2;
        int child_keybinding_depth = !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1     ? 2
                                                                 : 0;
        g_autoptr(GVariant) child =
            variant_from_lua(state, -1, depth + 1, child_array, child_keybinding_depth, error);
        if (!child) {
            lua_pop(state, 1);
            g_variant_builder_clear(&out);
            return NULL;
        }
        g_variant_builder_add(&out, "{sv}", key, g_variant_ref(child));
        lua_pop(state, 1);
    }
    return g_variant_builder_end(&out);
}

static void push_variant(lua_State* state, GVariant* value) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        push_variant(state, child);
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        lua_pushboolean(state, g_variant_get_boolean(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        lua_pushinteger(state, g_variant_get_int64(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        lua_pushnumber(state, g_variant_get_double(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        lua_pushstring(state, g_variant_get_string(value, NULL));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        lua_newtable(state);
        GVariantIter iter;
        GVariant* child;
        guint i = 1;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "v", &child)) {
            push_variant(state, child);
            lua_rawseti(state, -2, i++);
            g_variant_unref(child);
        }
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        lua_newtable(state);
        GVariantIter iter;
        const char* key;
        GVariant* child;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            push_variant(state, child);
            lua_setfield(state, -2, key);
            g_variant_unref(child);
        }
    } else
        lua_pushnil(state);
}

static gboolean is_array(lua_State* state, int index) {
    return marked_array(state, index) || lua_rawlen(state, index) > 0;
}

static void merge_table(lua_State* state, int destination, int source, const char* key) {
    destination = lua_absindex(state, destination);
    source = lua_absindex(state, source);
    if (append_array_key(key) && lua_istable(state, destination) && lua_istable(state, source)) {
        lua_Integer offset = lua_rawlen(state, destination), count = lua_rawlen(state, source);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, source, i);
            lua_rawseti(state, destination, offset + i);
        }
        return;
    }
    lua_pushnil(state);
    while (lua_next(state, source)) {
        const char* name = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
        if (name && lua_istable(state, -1)) {
            lua_pushvalue(state, -2);
            lua_rawget(state, destination);
            if (lua_istable(state, -1) && !is_array(state, -1) && !is_array(state, -2)) {
                merge_table(state, -1, -2, name);
                lua_pop(state, 1);
            } else {
                lua_pop(state, 1);
                lua_pushvalue(state, -2);
                lua_pushvalue(state, -2);
                lua_rawset(state, destination);
            }
        } else {
            lua_pushvalue(state, -2);
            lua_pushvalue(state, -2);
            lua_rawset(state, destination);
        }
        lua_pop(state, 1);
    }
}

static int lua_array(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_newtable(state);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "__gnoblin_array");
    lua_setmetatable(state, 1);
    lua_settop(state, 1);
    return 1;
}

static int lua_set(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    merge_table(state, -1, 1, NULL);
    lua_pop(state, 2);
    return 0;
}
static int lua_legacy_set(lua_State* state) {
    g_warning("gnoblin.set is deprecated and may be removed at any time; migrate to "
              "gnoblin.configure with public snake_case setting names");
    return lua_set(state);
}


static gboolean is_keybinding_action_name(const char* key) {
    if (!key || !*key) return FALSE;
    gboolean previous_underscore = TRUE;
    for (const unsigned char* p = (const unsigned char*)key; *p; p++) {
        if (*p == '_') {
            if (previous_underscore) return FALSE;
            previous_underscore = TRUE;
        } else if (g_ascii_islower(*p) || g_ascii_isdigit(*p))
            previous_underscore = FALSE;
        else
            return FALSE;
    }
    return !previous_underscore;
}

/* Public declarations use Lua identifiers. Keep keybinding action names in
 * snake_case until the shell maps them to their native GSettings names. */
static void push_settings(lua_State* state, int source, const char* parent, int depth,
                          int keybinding_depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Lua config nesting exceeds 64 levels");
    luaL_checkstack(state, 6, "settings nesting");
    source = lua_absindex(state, source);
    if (!lua_istable(state, source)) {
        lua_pushvalue(state, source);
        return;
    }
    lua_newtable(state);
    int destination = lua_gettop(state);
    gboolean literal =
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers"));
    lua_pushnil(state);
    while (lua_next(state, source)) {
        gboolean keybinding_action = keybinding_depth == 2 && lua_type(state, -2) == LUA_TSTRING;
        if (keybinding_action && !is_keybinding_action_name(lua_tostring(state, -2)))
            luaL_error(state, "keybinding action names must use snake_case");
        if (lua_type(state, -2) == LUA_TSTRING && !literal && !keybinding_action) {
            char* key = g_strdup(lua_tostring(state, -2));
            g_strdelimit(key, "_", '-');
            lua_pushstring(state, key);
            g_free(key);
        } else {
            lua_pushvalue(state, -2);
        }
        lua_pushvalue(state, -1);
        lua_rawget(state, destination);
        if (!lua_isnil(state, -1))
            luaL_error(state, "duplicate setting after snake_case conversion");
        lua_pop(state, 1);
        const char* key = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        int child_keybinding_depth = key && !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1               ? 2
                                                                           : 0;
        push_settings(state, -2, key, depth + 1, child_keybinding_depth);
        lua_rawset(state, destination);
        lua_pop(state, 1);
    }
    if (marked_array(state, source) && lua_getmetatable(state, source))
        lua_setmetatable(state, destination);
}

static int lua_configure(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    push_settings(state, 1, NULL, 0, 0);
    lua_replace(state, 1);
    return lua_set(state);
}

static void push_config_list(lua_State* state) {
    const char* section = lua_tostring(state, lua_upvalueindex(1));
    const char* key = lua_tostring(state, lua_upvalueindex(2));
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        luaL_error(state, "gnoblin.config must be a table");
    if (section[0]) {
        lua_getfield(state, -1, section);
        if (lua_isnil(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, section);
        }
        if (!lua_istable(state, -1))
            luaL_error(state, "%s must be a table", section);
    }
    lua_getfield(state, -1, key);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, key);
    }
    if (!lua_istable(state, -1))
        luaL_error(state, "%s must be a list", key);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), entries = 0;
    lua_pushnil(state);
    while (lua_next(state, list)) {
        if (!lua_isinteger(state, -2) || lua_tointeger(state, -2) < 1 ||
            lua_tointeger(state, -2) > count || !lua_istable(state, -1))
            luaL_error(state, "%s must be a dense list of tables", key);
        entries++;
        lua_pop(state, 1);
    }
    if (entries != count)
        luaL_error(state, "%s must be a dense list of tables", key);
}

/* Named commands merge in place; ordered rules always append. */
    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && *api) {
        const char* migration =
            !strcmp(api, "gnoblin.shortcut")
                ? "migrate to gnoblin.configure {shortcuts = {{name = ..., binding = ..., command "
                  "= ...}}}"
                : "migrate to gnoblin.configure {autostart = {{name = ..., command = ...}}}";
        g_warning("%s is deprecated and may be removed at any time; %s", api, migration);
    }
static int lua_declare(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    push_settings(state, 1, NULL, 0, 0);
    int entry = lua_gettop(state);
    const char* name = NULL;
    if (lua_toboolean(state, lua_upvalueindex(3))) {
        lua_getfield(state, entry, "name");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
            return luaL_error(state, "declaration needs a nonempty string name");
        name = lua_tostring(state, -1);
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
            merge_table(state, -1, entry, NULL);
            return 0;
        }
        lua_pop(state, 1);
    }
    lua_pushvalue(state, entry);
    lua_rawseti(state, list, count + 1);
    return 0;
}

    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && *api) {
        const char* migration =
            !strcmp(api, "gnoblin.remove_shortcut")
                ? "migrate by removing that name from gnoblin.configure {shortcuts = {...}}"
                : "migrate by removing that name from gnoblin.configure {autostart = {...}}";
        g_warning("%s is deprecated and may be removed at any time; %s", api, migration);
    }
static int lua_remove_declaration(lua_State* state) {
    const char* name = luaL_checkstring(state, 1);
    push_config_list(state);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), next = 1;
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches)
            lua_pop(state, 1);
        else
            lua_rawseti(state, list, next++);
    }
    for (lua_Integer i = next; i <= count; i++) {
        lua_pushnil(state);
        lua_rawseti(state, list, i);
    }
    return 0;
}

static gboolean evaluate_path(lua_State*, LuaConfig*, const char*, gboolean, GError**);

static int lua_config_load(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    const char* pattern = luaL_checkstring(state, 1);
    g_autoptr(GError) error = NULL;
    g_autoptr(GPtrArray) paths =
        gnoblin_config_expand_paths(config->current_path, pattern, config->directories, &error);
    if (!paths)
        return luaL_error(state, "%s", error->message);
    for (guint i = 0; i < paths->len; i++)
        if (!evaluate_path(state, config, g_ptr_array_index(paths, i), FALSE, &error))
            return luaL_error(state, "%s", error->message);
    return 0;
}

static gboolean safe_module_name(const char* name) {
    if (!name[0] || strstr(name, ".."))
        return FALSE;
    for (const char* p = name; *p; p++)
        if (!g_ascii_isalnum(*p) && *p != '_' && *p != '-' && *p != '.')
            return FALSE;
    return TRUE;
}

static int lua_require(lua_State* state) {
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

static void install_api(lua_State* state, LuaConfig* config) {
    luaL_openlibs(state);
    const char* blocked[] = {"os", "io", "debug", "package", "dofile", "loadfile", NULL};
    for (guint i = 0; blocked[i]; i++) {
        lua_pushnil(state);
        lua_setglobal(state, blocked[i]);
    }
    lua_newtable(state);
    lua_newtable(state);
    lua_setfield(state, -2, "config");
    lua_pushcfunction(state, lua_legacy_set);
    lua_setfield(state, -2, "set");
    lua_pushcfunction(state, lua_configure);
    lua_setfield(state, -2, "configure");
    const struct {
        const char *name, *section, *key;
        gboolean named, remove;
    } declarations[] = {
        {"window_rule", "", "window-rules", FALSE, FALSE},
        {"permission_rule", "permissions", "rules", FALSE, FALSE},
        {"shortcut", "", "shortcuts", TRUE, FALSE},
        {"autostart", "", "autostart", TRUE, FALSE},
        {"remove_shortcut", "", "shortcuts", TRUE, TRUE},
        {"remove_autostart", "", "autostart", TRUE, TRUE},
    };
    for (guint i = 0; i < G_N_ELEMENTS(declarations); i++) {
        lua_pushstring(state, declarations[i].section);
        lua_pushstring(state, declarations[i].key);
        lua_pushboolean(state, declarations[i].named);
        lua_pushstring(
            state, !strcmp(declarations[i].name, "shortcut")           ? "gnoblin.shortcut"
                   : !strcmp(declarations[i].name, "autostart")        ? "gnoblin.autostart"
                   : !strcmp(declarations[i].name, "remove_shortcut")  ? "gnoblin.remove_shortcut"
                   : !strcmp(declarations[i].name, "remove_autostart") ? "gnoblin.remove_autostart"
                                                                       : "");
        lua_pushcclosure(state, declarations[i].remove ? lua_remove_declaration : lua_declare, 4);
        lua_setfield(state, -2, declarations[i].name);
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_config_load, 1);
    lua_setfield(state, -2, "load");
    lua_pushcfunction(state, lua_array);
    lua_setfield(state, -2, "array");
    lua_setglobal(state, "gnoblin");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_require, 1);
    lua_setglobal(state, "require");
}

static gboolean merge_variant(lua_State* state, GVariant* document, GError** error) {
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "loaded config must be a table");
        return FALSE;
    }
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    push_variant(state, document);
    merge_table(state, -2, -1, NULL);
    lua_pop(state, 3);
    return TRUE;
}

static gboolean evaluate_toml(lua_State* state, LuaConfig* config, const char* path,
                              GError** error) {
    g_autofree char* contents = NULL;
    if (!g_file_get_contents(path, &contents, NULL, error))
        return FALSE;
    g_autoptr(GVariant) document = gnoblin_config_parse_toml(contents, error);
    if (!document)
        return FALSE;
    return merge_variant(state, document, error);
}

static gboolean evaluate_path(lua_State* state, LuaConfig* config, const char* given, gboolean root,
                              GError** error) {
    g_autofree char* path = g_canonicalize_filename(given, NULL);
    add_path(config->paths, path);
    if (!g_file_test(path, G_FILE_TEST_EXISTS)) {
        if (root)
            return TRUE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_NOENT, "%s: config file does not exist",
                    path);
        return FALSE;
    }
    if (g_hash_table_contains(config->active, path)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: config cycle", path);
        return FALSE;
    }
    if (g_hash_table_size(config->active) >= MAX_CONFIG_FILES) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "config nesting exceeds 32 files");
        return FALSE;
    }
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    gboolean ok;
    if (!g_str_has_suffix(path, ".lua")) {
        ok = evaluate_toml(state, config, path, error);
    } else {
        int status = luaL_loadfilex(state, path, "t");
        if (status == LUA_OK)
            status = lua_pcall(state, 0, 1, 0);
        if (status != LUA_OK) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: %s", path,
                        lua_error_text(state));
            lua_pop(state, 1);
            ok = FALSE;
        } else if (lua_isnil(state, -1)) {
            lua_pop(state, 1);
            ok = TRUE;
        } else if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config must return a table or nil");
            ok = FALSE;
        } else {
            g_autoptr(GError) conversion = NULL;
            g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, FALSE, FALSE, &conversion);
            ok = value != NULL && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT);
            if (ok) {
                lua_getglobal(state, "gnoblin");
                lua_getfield(state, -1, "config");
                merge_table(state, -1, -3, NULL);
                lua_pop(state, 3);
            } else {
                lua_pop(state, 1);
                if (conversion)
                    g_propagate_error(error, g_steal_pointer(&conversion));
                else
                    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                        "Lua config must return a table of settings");
            }
        }
    }
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    return ok;
}

typedef struct {
    LuaConfig* config;
    const char* path;
    GVariant* result;
    GError* error;
} EvalRun;

static int protected_eval(lua_State* state) {
    EvalRun* run = lua_touserdata(state, lua_upvalueindex(1));

    install_api(state, run->config);
    if (!evaluate_path(state, run->config, run->path, TRUE, &run->error))
        return 0;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    run->result = variant_from_lua(state, -1, 0, FALSE, 0, &run->error);
    lua_pop(state, 2);
    return 0;
}

GVariant* gnoblin_config_evaluate_file(const char* path, GPtrArray* paths, GPtrArray* directories,
                                       GError** error) {
    LuaConfig config = {.paths = paths ? paths : g_ptr_array_new_with_free_func(g_free),
                        .directories = directories,
                        .active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .current_path = g_canonicalize_filename(path, NULL)};
    lua_State* state = new_config_state(&config);
    EvalRun run = {.config = &config, .path = path};
    int steps = MAX_CONFIG_STEPS / 1000;
    if (!state) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOMEM,
                            "Lua config exceeded memory limit");
        goto out;
    }
    memcpy(lua_getextraspace(state), &steps, sizeof steps);
    lua_sethook(state, limit_hook, LUA_MASKCOUNT, 1000);
    lua_pushlightuserdata(state, &run);
    lua_pushcclosure(state, protected_eval, 1);
    if (lua_pcall(state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(state));
        lua_pop(state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
out:
    if (state)
        lua_close(state);
    g_hash_table_unref(config.active);
    g_hash_table_unref(config.modules);
    g_free(config.current_path);
    if (!paths)
        g_ptr_array_free(config.paths, TRUE);
    return run.result;
}

#include "gnoblin-console-lua.inc"
