#include "gnoblin-lua-internal.h"

static gboolean is_array(lua_State* state, int index) {
    return marked_array(state, index) || lua_rawlen(state, index) > 0;
}

void remove_named_entry(lua_State* state, int list, const char* name) {
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

int compare_entry_names(gconstpointer left, gconstpointer right) {
    return g_strcmp0(*(const char* const*)left, *(const char* const*)right);
}

/* Entries are appended to an ordered list, and that order reaches the input handlers that Mutter
 * compares with a replacement worker's configuration. Lua randomises its string hash seed for each
 * process, so lua_next order differs between the original worker and its replacement. Visit the
 * names in sorted order instead. */
void merge_named_entries(lua_State* state, int list, int entries) {
    list = lua_absindex(state, list);
    entries = lua_absindex(state, entries);
    g_autoptr(GPtrArray) names = g_ptr_array_new_with_free_func(g_free);
    lua_pushnil(state);
    while (lua_next(state, entries)) {
        if (lua_type(state, -2) != LUA_TSTRING || !lua_istable(state, -1)) {
            g_ptr_array_unref(g_steal_pointer(&names));
            luaL_error(state, "named shortcuts and autostart entries must be tables");
        }
        g_ptr_array_add(names, g_strdup(lua_tostring(state, -2)));
        lua_pop(state, 1);
    }
    g_ptr_array_sort(names, compare_entry_names);
    for (guint position = 0; position < names->len; position++) {
        const char* name = g_ptr_array_index(names, position);
        lua_getfield(state, entries, name);
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

void merge_table(lua_State* state, int destination, int source, const char* key,
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

int lua_array(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_newtable(state);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "__gnoblin_array");
    lua_setmetatable(state, 1);
    lua_settop(state, 1);
    return 1;
}

void copy_config_value(lua_State* state, int source, int depth) {
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

void push_public_settings(lua_State* state, GVariant* value, const char* parent, int depth,
                          int keybinding_depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Gnoblin settings nesting exceeds 64 levels");
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        push_public_settings(state, child, parent, depth, keybinding_depth);
        return;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        lua_newtable(state);
        mark_array_table(state, -1);
        GVariantIter iter;
        GVariant* child;
        guint i = 1;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "v", &child)) {
            push_public_settings(state, child, parent, depth + 1, keybinding_depth);
            lua_rawseti(state, -2, i++);
            g_variant_unref(child);
        }
        return;
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        push_variant(state, value);
        return;
    }

    lua_newtable(state);
    gboolean literal =
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers") ||
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart") ||
                   !strcmp(parent, "interfaces"));
    GVariantIter iter;
    const char* name;
    GVariant* child;
    g_variant_iter_init(&iter, value);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &child)) {
        gboolean action_name = keybinding_depth == 2;
        g_autofree char* public_name = g_strdup(name);
        if (!literal && !action_name)
            g_strdelimit(public_name, "-", '_');
        int child_keybinding_depth = !strcmp(name, "keybindings") ? 1
                                     : keybinding_depth == 1      ? 2
                                                                  : 0;
        push_public_settings(state, child, name, depth + 1, child_keybinding_depth);
        lua_setfield(state, -2, public_name);
        g_variant_unref(child);
    }
}
