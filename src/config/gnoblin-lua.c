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
    GPtrArray* runtime_actions;
    GHashTable *active, *modules;
    char* current_path;
    guint actions_in_dispatch;
    gboolean dispatching;
    gboolean api_calling;
} LuaConfig;

typedef struct {
    LuaConfig config;
    lua_State* state;
    GVariant* document;
    GVariant* pending_document;
} LuaRuntime;

static LuaRuntime* active_runtime;
static LuaRuntime* pending_runtime;
static guint64 next_runtime_operation_id;

static const char* api_methods[] = {
    "workspace.list",
    "workspace.create",
    "workspace.rename",
    "workspace.remove",
    "workspace.switch",
    "workspace.next",
    "workspace.previous",
    "workspace.move_active",
    "workspace.move_window",
    "window.list",
    "window.match",
    "window.action",
    "layer.list",
    "monitor.list",
    "animation.list",
    "animation.surfaces",
    "animation.inspect",
    "animation.preview",
    "animation.seek",
    "animation.step",
    "animation.play",
    "animation.pause",
    "animation.stop",
    "feature.list",
    "feature.show",
    "feature.enable",
    "feature.disable",
    "script.list",
    "input.list",
    "input.current",
    "input.select",
    "privacy.get",
    "permissions.list",
    "permissions.check",
    "grant.list",
    "grant.revoke",
    "launch.status",
    "launch.begin",
    "launch.end",
    "shell.ping",
    "shell.version",
    "shell.status",
    "shell.reload",
    "runtime.reload_config",
    "shortcut.capture",
    NULL,
};

static gboolean append_array_key(const char* key) {
    return key && (!strcmp(key, "autostart") || !strcmp(key, "window-rules") ||
                   !strcmp(key, "shortcuts") || !strcmp(key, "animations") ||
                   !strcmp(key, "rules") || !strcmp(key, "workspaces") ||
                   !strcmp(key, "workspace-names") || !strcmp(key, "workspace-ids") ||
                   !strcmp(key, "xkb-options") || !strcmp(key, "sources"));
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

static void merge_table(lua_State* state, int destination, int source, const char* key,
                        gboolean append_lists);
static gboolean workspace_id_valid(const char* id);

static void remove_named_entry(lua_State* state, int list, const char* name) {
    list = lua_absindex(state, list);
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
}

static gboolean named_entries_key(const char* key) {
    return key && (!strcmp(key, "shortcuts") || !strcmp(key, "autostart"));
}

static gboolean named_entries_map(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index) || is_array(state, index))
        return FALSE;
    lua_pushnil(state);
    if (!lua_next(state, index))
        return FALSE;
    gboolean map = lua_type(state, -2) == LUA_TSTRING;
    lua_pop(state, 2);
    return map;
}

static void merge_named_entries(lua_State* state, int list, int entries) {
    list = lua_absindex(state, list);
    entries = lua_absindex(state, entries);
    lua_pushnil(state);
    while (lua_next(state, entries)) {
        if (lua_type(state, -2) != LUA_TSTRING || !lua_istable(state, -1))
            luaL_error(state, "named shortcuts and autostart entries must be tables");
        const char* name = lua_tostring(state, -2);
        int source = lua_gettop(state);
        lua_getfield(state, source, "name");
        if (!lua_isnil(state, -1) &&
            (lua_type(state, -1) != LUA_TSTRING || strcmp(name, lua_tostring(state, -1))))
            luaL_error(state, "named entry key and name must match");
        lua_pop(state, 1);
        lua_getfield(state, source, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (disabled) {
            remove_named_entry(state, list, name);
            lua_pop(state, 1);
            continue;
        }
        lua_newtable(state);
        int entry = lua_gettop(state);
        merge_table(state, entry, source, NULL, FALSE);
        lua_pushnil(state);
        lua_setfield(state, entry, "enable");
        lua_pushstring(state, name);
        lua_setfield(state, entry, "name");
        lua_Integer count = lua_rawlen(state, list);
        gboolean merged = FALSE;
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, list, i);
            lua_getfield(state, -1, "name");
            gboolean matches =
                lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
            lua_pop(state, 1);
            if (matches) {
                lua_pushnil(state);
                lua_setfield(state, -2, "enable");
                merge_table(state, -1, entry, NULL, FALSE);
                merged = TRUE;
            }
            lua_pop(state, 1);
            if (merged)
                break;
        }
        if (!merged) {
            lua_pushvalue(state, entry);
            lua_rawseti(state, list, count + 1);
        }
        lua_pop(state, 2);
    }
}

static void merge_table(lua_State* state, int destination, int source, const char* key,
                        gboolean append_lists) {
    destination = lua_absindex(state, destination);
    source = lua_absindex(state, source);
    if (key && !strcmp(key, "workspaces") && lua_istable(state, destination) &&
        lua_istable(state, source)) {
        lua_Integer count = lua_rawlen(state, source);
        lua_newtable(state);
        int seen_ids = lua_absindex(state, -1);
        lua_pushnil(state);
        while (lua_next(state, source)) {
            gboolean dense_key = lua_isinteger(state, -2) && lua_tointeger(state, -2) >= 1 &&
                                 lua_tointeger(state, -2) <= count;
            lua_pop(state, 1);
            if (!dense_key)
                luaL_error(state, "workspaces must be a dense array starting at 1");
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, source, i);
            if (!lua_istable(state, -1))
                luaL_error(state, "workspaces must be an array of tables");
            int entry = lua_absindex(state, -1);
            lua_getfield(state, entry, "id");
            const char* id = luaL_checkstring(state, -1);
            if (!workspace_id_valid(id))
                luaL_error(state, "workspace id must start with a letter or number and contain "
                                  "only letters, numbers, '.', '_' or '-' (up to 64 characters)");
            lua_pushvalue(state, -1);
            lua_rawget(state, seen_ids);
            gboolean duplicate = lua_toboolean(state, -1);
            lua_pop(state, 1);
            if (duplicate)
                luaL_error(state, "duplicate workspace id in workspaces array: %s", id);
            lua_pushvalue(state, -1);
            lua_pushboolean(state, TRUE);
            lua_rawset(state, seen_ids);
            lua_pop(state, 1);
            lua_Integer existing_count = lua_rawlen(state, destination);
            gboolean merged = FALSE;
            for (lua_Integer j = 1; j <= existing_count; j++) {
                lua_rawgeti(state, destination, j);
                lua_getfield(state, -1, "id");
                gboolean matches =
                    lua_type(state, -1) == LUA_TSTRING && g_str_equal(lua_tostring(state, -1), id);
                lua_pop(state, 1);
                if (matches) {
                    merge_table(state, -1, entry, NULL, FALSE);
                    merged = TRUE;
                }
                lua_pop(state, 1);
                if (merged)
                    break;
            }
            if (!merged) {
                lua_pushvalue(state, entry);
                lua_rawseti(state, destination, existing_count + 1);
            }
            lua_pop(state, 1);
        }
        lua_pop(state, 1);
        return;
    }
    if (append_lists && append_array_key(key) && lua_istable(state, destination) &&
        lua_istable(state, source)) {
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
        if (named_entries_key(name) && named_entries_map(state, -1)) {
            lua_pushvalue(state, -2);
            lua_rawget(state, destination);
            if (lua_isnil(state, -1)) {
                lua_pop(state, 1);
                lua_newtable(state);
                lua_pushvalue(state, -3);
                lua_pushvalue(state, -2);
                lua_rawset(state, destination);
            }
            if (!lua_istable(state, -1) || named_entries_map(state, -1))
                luaL_error(state, "%s must be a list before named overrides", name);
            merge_named_entries(state, -1, -2);
            lua_pop(state, 1);
        } else if (name && lua_istable(state, -1)) {
            lua_pushvalue(state, -2);
            lua_rawget(state, destination);
            if (lua_istable(state, -1) && !is_array(state, -1) && !is_array(state, -2)) {
                merge_table(state, -1, -2, name, append_lists);
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

static void copy_config_value(lua_State* state, int source, int depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Lua config nesting exceeds 64 levels");
    source = lua_absindex(state, source);
    if (!lua_istable(state, source)) {
        lua_pushvalue(state, source);
        return;
    }
    lua_newtable(state);
    int destination = lua_gettop(state);
    lua_pushnil(state);
    while (lua_next(state, source)) {
        lua_pushvalue(state, -2);
        copy_config_value(state, -2, depth + 1);
        lua_rawset(state, destination);
        lua_pop(state, 1);
    }
    if (marked_array(state, source) && lua_getmetatable(state, source))
        lua_setmetatable(state, destination);
}

static int lua_snapshot(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "snapshot takes no arguments");
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    copy_config_value(state, -1, 0);
    return 1;
}

static int lua_set(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    merge_table(state, -1, 1, NULL, FALSE);
    lua_pop(state, 2);
    return 0;
}
static int lua_legacy_set(lua_State* state) {
    g_warning("gnoblin.set is deprecated and may be removed at any time; migrate to "
              "gnoblin.configure with public snake_case setting names");
    return lua_set(state);
}

static int lua_on(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    if (event_length == 0 || event_length > 128 || memchr(event, '\0', event_length) ||
        !g_utf8_validate(event, event_length, NULL))
        return luaL_error(state, "event name must contain 1 to 128 bytes");
    /* Keep the former built-in event as an input alias while listeners are
     * stored under the canonical event name. */
    if (event_length == strlen("pointer_window_changed") &&
        !memcmp(event, "pointer_window_changed", event_length))
        event = "mutter.wayland.pointer-window-changed";
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    int listeners = lua_absindex(state, -1);
    lua_getfield(state, -1, event);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, listeners, event);
    }
    lua_pushvalue(state, 2);
    lua_rawseti(state, -2, lua_rawlen(state, -2) + 1);
    return 0;
}

static gboolean workspace_id_valid(const char* id) {
    return id && g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, G_REGEX_OPTIMIZE,
                                      G_REGEX_MATCH_NOTEMPTY);
}

static gboolean workspace_selector_id_valid(const char* id) {
    return workspace_id_valid(id) ||
           (id && g_regex_match_simple("^@session-[1-9][0-9]*$", id, G_REGEX_OPTIMIZE,
                                       G_REGEX_MATCH_NOTEMPTY));
}

static guint64 allocate_operation_id(void) {
    if (next_runtime_operation_id >= G_MAXINT64)
        return 0;
    return ++next_runtime_operation_id;
}

static gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed) {
    index = lua_absindex(state, index);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        gboolean found = FALSE;
        if (lua_type(state, -2) == LUA_TSTRING) {
            const char* key = lua_tostring(state, -2);
            for (guint i = 0; allowed[i]; i++)
                found |= g_str_equal(key, allowed[i]);
        }
        lua_pop(state, 1);
        if (!found)
            return FALSE;
    }
    return TRUE;
}

static GVariant* workspace_selector(lua_State* state, int index) {
    luaL_checktype(state, index, LUA_TTABLE);
    index = lua_absindex(state, index);
    static const char* const fields[] = {"id", "number", NULL};
    if (!table_fields_allowed(state, index, fields))
        luaL_error(state, "workspace selector accepts only id or number");
    GVariantBuilder selector;
    g_variant_builder_init(&selector, G_VARIANT_TYPE_VARDICT);
    lua_getfield(state, index, "id");
    gboolean has_id = !lua_isnil(state, -1);
    if (has_id) {
        const char* id = luaL_checkstring(state, -1);
        if (!workspace_selector_id_valid(id))
            luaL_error(state, "workspace id must start with a letter or number and contain only "
                              "letters, numbers, '.', '_' or '-' (up to 64 characters)");
        g_variant_builder_add(&selector, "{sv}", "id", g_variant_new_string(id));
    }
    lua_pop(state, 1);
    lua_getfield(state, index, "number");
    gboolean has_number = !lua_isnil(state, -1);
    if (has_number) {
        if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
            lua_tointeger(state, -1) > 1024)
            luaL_error(state, "workspace number must be an integer from 1 to 1024");
        g_variant_builder_add(&selector, "{sv}", "number",
                              g_variant_new_int64(lua_tointeger(state, -1)));
    }
    lua_pop(state, 1);
    if (has_id == has_number)
        luaL_error(state, "workspace selector must contain exactly one of id or number");
    return g_variant_builder_end(&selector);
}

static void add_workspace_field(lua_State* state, GVariantBuilder* arguments, int table,
                                const char* field) {
    lua_getfield(state, table, field);
    GVariant* selector = workspace_selector(state, -1);
    g_variant_builder_add(arguments, "{sv}", "workspace", selector);
    lua_pop(state, 1);
}

static void add_selector_arguments(lua_State* state, GVariantBuilder* arguments, int table) {
    g_autoptr(GVariant) selector = workspace_selector(state, table);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, selector);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_variant_builder_add(arguments, "{sv}", key, value);
        g_variant_unref(value);
    }
}

static int lua_workspace_action(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (!config || !config->runtime_actions || (!config->dispatching && !config->api_calling))
        return luaL_error(state,
                          "workspace actions are available only while handling a runtime event");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "too many pending workspace actions");

    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    if (g_str_equal(method, "workspace.list") || g_str_equal(method, "workspace.next") ||
        g_str_equal(method, "workspace.previous")) {
        if (lua_gettop(state) != 0)
            return luaL_error(state, "%s takes no arguments", method);
    } else if (g_str_equal(method, "workspace.create")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"id", "name", "activate", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.create accepts only id, name, and activate");
        lua_getfield(state, 1, "id");
        if (!lua_isnil(state, -1)) {
            const char* id = luaL_checkstring(state, -1);
            if (!workspace_id_valid(id))
                return luaL_error(state,
                                  "workspace id must start with a letter or number and contain "
                                  "only letters, numbers, '.', '_' or '-' (up to 64 characters)");
            g_variant_builder_add(&arguments, "{sv}", "id", g_variant_new_string(id));
        }
        lua_pop(state, 1);
        lua_getfield(state, 1, "name");
        const char* name = luaL_checkstring(state, -1);
        if (!*name || !g_utf8_validate(name, -1, NULL) || g_utf8_strlen(name, -1) > 80)
            return luaL_error(state, "workspace name must contain 1 to 80 characters");
        g_variant_builder_add(&arguments, "{sv}", "name", g_variant_new_string(name));
        lua_pop(state, 1);
        lua_getfield(state, 1, "activate");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace activate must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "activate",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.rename")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"id", "number", "name", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.rename accepts only id or number and name");
        add_selector_arguments(state, &arguments, 1);
        lua_getfield(state, 1, "name");
        const char* name = luaL_checkstring(state, -1);
        if (!*name || !g_utf8_validate(name, -1, NULL) || g_utf8_strlen(name, -1) > 80)
            return luaL_error(state, "workspace name must contain 1 to 80 characters");
        g_variant_builder_add(&arguments, "{sv}", "name", g_variant_new_string(name));
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.remove") || g_str_equal(method, "workspace.switch")) {
        add_selector_arguments(state, &arguments, 1);
    } else if (g_str_equal(method, "workspace.move_active")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"workspace", "follow", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.move_active accepts only workspace and follow");
        add_workspace_field(state, &arguments, 1, "workspace");
        lua_getfield(state, 1, "follow");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace move follow must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "follow",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.move_window")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"window", "workspace", "follow", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state,
                              "workspace.move_window accepts only window, workspace, and follow");
        lua_getfield(state, 1, "window");
        const char* window = luaL_checkstring(state, -1);
        if (g_str_equal(window, "active")) {
            g_variant_builder_add(&arguments, "{sv}", "window", g_variant_new_string("active"));
        } else if (*window && g_utf8_validate(window, -1, NULL)) {
            g_variant_builder_add(&arguments, "{sv}", "window", g_variant_new_string(window));
        } else {
            return luaL_error(state, "window must be 'active' or a nonempty window ID");
        }
        lua_pop(state, 1);
        add_workspace_field(state, &arguments, 1, "workspace");
        lua_getfield(state, 1, "follow");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace move follow must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "follow",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else {
        g_variant_builder_clear(&arguments);
        return luaL_error(state, "unknown workspace action");
    }

    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder action;
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    lua_pushinteger(state, request_id);
    return 1;
}

static int lua_generic_api_action(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (!config || !config->runtime_actions || (!config->dispatching && !config->api_calling))
        return luaL_error(state, "Gnoblin session actions are available only at runtime");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "too many pending Gnoblin session actions");

    GVariant* arguments = NULL;
    if (lua_gettop(state) == 0) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        GError* error = NULL;
        arguments = variant_from_lua(state, 1, 0, FALSE, 0, &error);
        if (!arguments) {
            char* message = g_strdup(error ? error->message : "expected a table");
            g_clear_error(&error);
            lua_pushfstring(state, "%s arguments: %s", method, message);
            g_free(message);
            return lua_error(state);
        }
        if (!g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
            g_variant_unref(arguments);
            return luaL_error(state, "%s arguments must be a table", method);
        }
    } else {
        return luaL_error(state, "%s takes no arguments or one argument table", method);
    }

    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder action;
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", arguments);
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    lua_pushinteger(state, request_id);
    return 1;
}

static gboolean is_keybinding_action_name(const char* key) {
    if (!key || !*key)
        return FALSE;
    gboolean previous_underscore = TRUE;
    for (const unsigned char* p = (const unsigned char*)key; *p; p++) {
        if (*p == '_') {
            if (previous_underscore)
                return FALSE;
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
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers") ||
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart"));
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
                                     : keybinding_depth == 1            ? 2
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

static int lua_configure_call(lua_State* state) {
    lua_remove(state, 1);
    return lua_configure(state);
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

static void install_named_view(lua_State* state, const char* key) {
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

static void finish_named_entries(lua_State* state, int config, const char* key) {
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
    lua_pop(state, 1);
}

/* Named commands merge in place; ordered rules always append. */
static int lua_declare(lua_State* state) {
    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && (!strcmp(api, "gnoblin.shortcut") || !strcmp(api, "gnoblin.autostart"))) {
        const char* migration = !strcmp(api, "gnoblin.shortcut")
                                    ? "migrate to gnoblin.configure {shortcuts = {[\"<name>\"] = "
                                      "{binding = ..., command = ...}}}"
                                    : "migrate to gnoblin.configure {autostart = {[\"<name>\"] = "
                                      "{command = ...}}}";
        g_warning("%s is deprecated and may be removed at any time; %s", api, migration);
    }
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

static int lua_remove_declaration(lua_State* state) {
    const char* name = luaL_checkstring(state, 1);
    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && *api) {
        if (!strcmp(api, "gnoblin.remove_shortcut"))
            g_warning("%s is deprecated and may be removed at any time; remove key \"%s\" from "
                      "gnoblin.configure.shortcuts or set "
                      "gnoblin.configure.shortcuts[\"%s\"].enable = false",
                      api, name, name);
        else if (!strcmp(api, "gnoblin.remove_autostart"))
            g_warning("%s is deprecated and may be removed at any time; remove key \"%s\" from "
                      "gnoblin.configure.autostart or set "
                      "gnoblin.configure.autostart[\"%s\"].enable = false",
                      api, name, name);
    }
    push_config_list(state);
    remove_named_entry(state, -1, name);
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
    for (guint i = 0; api_methods[i]; i++) {
        const char* separator = strchr(api_methods[i], '.');
        g_autofree char* domain = g_strndup(api_methods[i], separator - api_methods[i]);
        const char* operation = separator + 1;
        lua_getfield(state, -1, domain);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, domain);
        }
        lua_pushlightuserdata(state, config);
        lua_pushstring(state, api_methods[i]);
        lua_pushcclosure(state,
                         g_str_has_prefix(api_methods[i], "workspace.") ? lua_workspace_action
                                                                        : lua_generic_api_action,
                         2);
        lua_setfield(state, -2, operation);
        lua_pop(state, 1);
    }
    lua_pushcfunction(state, lua_on);
    lua_setfield(state, -2, "on");
    lua_newtable(state);
    lua_setfield(state, -2, "listeners");
    lua_pushcfunction(state, lua_legacy_set);
    lua_setfield(state, -2, "set");
    lua_newtable(state);
    lua_newtable(state);
    lua_pushcfunction(state, lua_configure_call);
    lua_setfield(state, -2, "__call");
    lua_setmetatable(state, -2);
    install_named_view(state, "shortcuts");
    install_named_view(state, "autostart");
    lua_setfield(state, -2, "configure");
    lua_pushcfunction(state, lua_snapshot);
    lua_setfield(state, -2, "snapshot");
    const struct {
        const char *name, *section, *key;
        gboolean named, remove;
    } declarations[] = {
        {"window_rule", "", "window-rules", FALSE, FALSE},
        {"permission_rule", "permissions", "rules", FALSE, FALSE},
        {"shortcut", "", "shortcuts", TRUE, FALSE},
        {"animation", "", "animations", TRUE, FALSE},
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
                   : !strcmp(declarations[i].name, "animation")        ? "gnoblin.animation"
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
    merge_table(state, -2, -1, NULL, TRUE);
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
                merge_table(state, -1, -3, NULL, TRUE);
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
    finish_named_entries(state, -1, "shortcuts");
    finish_named_entries(state, -1, "autostart");
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

static void lua_runtime_free(LuaRuntime* runtime) {
    if (!runtime)
        return;
    if (runtime->state)
        lua_close(runtime->state);
    g_clear_pointer(&runtime->config.active, g_hash_table_unref);
    g_clear_pointer(&runtime->config.modules, g_hash_table_unref);
    g_clear_pointer(&runtime->config.paths, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.directories, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.runtime_actions, g_ptr_array_unref);
    g_clear_pointer(&runtime->document, g_variant_unref);
    g_clear_pointer(&runtime->pending_document, g_variant_unref);
    g_free(runtime->config.current_path);
    g_free(runtime);
}

GVariant* gnoblin_config_load_runtime(const char* path, GPtrArray** paths, GPtrArray** directories,
                                      GError** error) {
    LuaRuntime* runtime = g_new0(LuaRuntime, 1);
    runtime->config.paths = g_ptr_array_new_with_free_func(g_free);
    runtime->config.directories = g_ptr_array_new_with_free_func(g_free);
    runtime->config.runtime_actions =
        g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
    runtime->config.active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    runtime->config.modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    runtime->config.current_path = g_canonicalize_filename(path, NULL);
    runtime->state = new_config_state(&runtime->config);
    if (!runtime->state) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOMEM,
                            "Lua config exceeded memory limit");
        lua_runtime_free(runtime);
        return NULL;
    }
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(runtime->state), &steps, sizeof steps);
    lua_sethook(runtime->state, limit_hook, LUA_MASKCOUNT, 1000);
    EvalRun run = {.config = &runtime->config, .path = path};
    lua_pushlightuserdata(runtime->state, &run);
    lua_pushcclosure(runtime->state, protected_eval, 1);
    if (lua_pcall(runtime->state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(runtime->state));
        lua_pop(runtime->state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
    runtime->document = run.result;
    if (!runtime->document || !gnoblin_config_validate_document(runtime->document, error)) {
        lua_runtime_free(runtime);
        return NULL;
    }
    lua_runtime_free(pending_runtime);
    pending_runtime = runtime;
    if (paths)
        *paths = g_ptr_array_ref(runtime->config.paths);
    if (directories)
        *directories = g_ptr_array_ref(runtime->config.directories);
    return g_variant_ref(runtime->document);
}

GVariant* gnoblin_config_dispatch_event(const char* event, GVariant* payload, GError** error) {
    // Shell is applying a newly parsed configuration before committing it.
    // Compositor preference changes in that window can emit transition events;
    // do not send those to the old Lua runtime or let it mutate the pending one.
    if (pending_runtime)
        return NULL;
    if (!active_runtime)
        return NULL;
    if (!event || !*event || strlen(event) > 128 || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid Gnoblin Lua event or payload");
        return NULL;
    }
    if (active_runtime->config.dispatching || active_runtime->config.api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua runtime operations cannot be dispatched reentrantly");
        return NULL;
    }
    guint action_start = active_runtime->config.runtime_actions->len;
    active_runtime->config.actions_in_dispatch = 0;
    active_runtime->config.dispatching = TRUE;
    lua_State* state = active_runtime->state;
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(state), &steps, sizeof steps);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    gboolean dispatched = FALSE;
    for (int pass = 0; pass < 2; pass++) {
        lua_getfield(state, -1, pass == 0 ? event : "*");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            continue;
        }
        int listeners = lua_gettop(state);
        lua_Integer count = lua_rawlen(state, listeners);
        dispatched |= count > 0;
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, listeners, i);
            push_variant(state, payload);
            lua_pushstring(state, event);
            lua_setfield(state, -2, "name");
            if (lua_pcall(state, 1, 0, 0) != LUA_OK) {
                g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua event '%s' failed: %s",
                            event, lua_error_text(state));
                lua_pop(state, 1);
                lua_pop(state, 3);
                active_runtime->config.dispatching = FALSE;
                g_ptr_array_set_size(active_runtime->config.runtime_actions, action_start);
                active_runtime->config.actions_in_dispatch = 0;
                lua_getglobal(state, "gnoblin");
                push_variant(state, active_runtime->document);
                lua_setfield(state, -2, "config");
                lua_pop(state, 1);
                return NULL;
            }
        }
        lua_pop(state, 1);
    }
    active_runtime->config.dispatching = FALSE;
    active_runtime->config.actions_in_dispatch = 0;
    lua_pop(state, 2);
    if (!dispatched)
        return g_variant_ref(active_runtime->document);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    GVariant* document = variant_from_lua(state, -1, 0, FALSE, 0, error);
    lua_pop(state, 2);
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error)) {
        g_clear_pointer(&document, g_variant_unref);
        g_ptr_array_set_size(active_runtime->config.runtime_actions, action_start);
        active_runtime->config.actions_in_dispatch = 0;
        lua_getglobal(state, "gnoblin");
        push_variant(state, active_runtime->document);
        lua_setfield(state, -2, "config");
        lua_pop(state, 1);
        return NULL;
    }
    g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
    active_runtime->pending_document = g_variant_ref_sink(document);
    return g_variant_ref(active_runtime->pending_document);
}

GVariant* gnoblin_config_drain_runtime_operations(void) {
    GVariantBuilder operations;
    g_variant_builder_init(&operations, G_VARIANT_TYPE("aa{sv}"));
    if (pending_runtime || !active_runtime || active_runtime->config.dispatching ||
        active_runtime->config.api_calling)
        return g_variant_builder_end(&operations);
    GPtrArray* pending = active_runtime->config.runtime_actions;
    for (guint i = 0; i < pending->len; i++)
        g_variant_builder_add_value(&operations, g_variant_ref(g_ptr_array_index(pending, i)));
    g_ptr_array_set_size(pending, 0);
    return g_variant_builder_end(&operations);
}

GVariant* gnoblin_config_call_api(const char* method, GVariant* arguments, GError** error) {
    if (pending_runtime) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API is temporarily unavailable while config reloads");
        return NULL;
    }
    if (!active_runtime || !method || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(
            error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
            "Gnoblin API call requires an active Lua runtime, method, and argument table");
        return NULL;
    }
    gboolean known = FALSE;
    for (guint i = 0; api_methods[i]; i++)
        known |= g_str_equal(api_methods[i], method);
    if (!known) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "unknown Gnoblin API method '%s'",
                    method);
        return NULL;
    }
    LuaConfig* config = &active_runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API calls cannot be dispatched reentrantly");
        return NULL;
    }
    if (config->runtime_actions->len >= 256) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOSPC,
                            "too many pending Gnoblin session actions");
        return NULL;
    }
    const char* separator = strchr(method, '.');
    g_autofree char* domain = g_strndup(method, separator - method);
    const char* operation = separator + 1;
    lua_State* state = active_runtime->state;
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->api_calling = TRUE;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, domain);
    lua_getfield(state, -1, operation);
    if (!lua_isfunction(state, -1)) {
        lua_pop(state, 3);
        config->api_calling = FALSE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Lua API method '%s' is not registered", method);
        return NULL;
    }
    int argument_count = g_variant_n_children(arguments) ? 1 : 0;
    if (argument_count)
        push_variant(state, arguments);
    if (lua_pcall(state, argument_count, 1, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API method '%s' failed: %s",
                    method, lua_error_text(state));
        lua_pop(state, 1);
        lua_pop(state, 2);
        config->api_calling = FALSE;
        g_ptr_array_set_size(config->runtime_actions, action_start);
        config->actions_in_dispatch = 0;
        return NULL;
    }
    gboolean valid_ticket = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0 &&
                            config->runtime_actions->len == action_start + 1;
    lua_pop(state, 1);
    lua_pop(state, 2);
    config->api_calling = FALSE;
    config->actions_in_dispatch = 0;
    if (!valid_ticket) {
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                    "Lua API method '%s' did not produce exactly one operation", method);
        return NULL;
    }
    GVariant* operation_call =
        g_variant_ref(g_ptr_array_index(config->runtime_actions, action_start));
    g_ptr_array_remove_index(config->runtime_actions, action_start);
    return operation_call;
}

void gnoblin_config_finish_load(gboolean commit) {
    if (!pending_runtime)
        return;
    if (commit) {
        lua_runtime_free(active_runtime);
        active_runtime = g_steal_pointer(&pending_runtime);
    } else {
        lua_runtime_free(g_steal_pointer(&pending_runtime));
    }
}

void gnoblin_config_finish_event(gboolean commit) {
    if (!active_runtime || !active_runtime->pending_document)
        return;
    if (commit) {
        g_clear_pointer(&active_runtime->document, g_variant_unref);
        active_runtime->document = g_steal_pointer(&active_runtime->pending_document);
        return;
    }
    lua_State* state = active_runtime->state;
    lua_getglobal(state, "gnoblin");
    push_variant(state, active_runtime->document);
    lua_setfield(state, -2, "config");
    lua_pop(state, 1);
    g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
}

char** gnoblin_config_runtime_events(void) {
    LuaRuntime* runtime = pending_runtime ? pending_runtime : active_runtime;
    if (!runtime)
        return g_new0(char*, 1);
    GPtrArray* events = g_ptr_array_new_with_free_func(g_free);
    lua_State* state = runtime->state;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    lua_pushnil(state);
    while (lua_next(state, -2)) {
        if (lua_type(state, -2) == LUA_TSTRING && lua_istable(state, -1))
            g_ptr_array_add(events, g_strdup(lua_tostring(state, -2)));
        lua_pop(state, 1);
    }
    lua_pop(state, 2);
    g_ptr_array_add(events, NULL);
    return (char**)g_ptr_array_free(events, FALSE);
}

#include "gnoblin-console-lua.inc"
