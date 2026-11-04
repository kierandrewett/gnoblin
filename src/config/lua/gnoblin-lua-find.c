/* gnoblin.find(kind, filter?): the live objects of one kind that match a filter.
 *
 * One service for every kind that has a list, driven by the property schema of the kind (the
 * annotated fields in src/config/lua/api/): a table filter names declared scalar fields and each
 * handle must equal every value; a function filter is called with each handle and keeps those
 * for which it returns true. The result holds the same live handles the kind's list function
 * returns, in its order. */
#include "gnoblin-lua-internal.h"

/* Each kind that has a list, and the gnoblin path of the function behind that list. */
static const struct {
    const char* kind;
    const char* list;
} listable[] = {
    {"Window", "windows.list"},       {"Workspace", "workspaces.list"},
    {"LayerSurface", "layers.list"},  {"Output", "outputs.list"},
    {"Shortcut", "shortcuts.list"},   {"Launch", "launches.list"},
    {"InputSource", "input.sources"}, {"Process", "process.list"},
};

const char* lua_find_list_path(const char* kind) {
    for (gsize i = 0; i < G_N_ELEMENTS(listable); i++)
        if (g_str_equal(listable[i].kind, kind))
            return listable[i].list;
    return NULL;
}

char* lua_find_kind_names(void) {
    g_autoptr(GPtrArray) names = g_ptr_array_new();
    for (gsize i = 0; i < G_N_ELEMENTS(listable); i++)
        g_ptr_array_add(names, (gpointer)listable[i].kind);
    g_ptr_array_add(names, NULL);
    return g_strjoinv(", ", (gchar**)names->pdata);
}

/* Raise the error for a filter field that `find` cannot match on `kind`. */
static int find_field_error(lua_State* state, const char* kind, const char* field) {
    g_autofree char* names = lua_api_class_scalar_names(kind);
    if (!lua_api_class_has_field(kind, field))
        return luaL_error(state, "a %s has no %s field; gnoblin.find matches: %s", kind, field,
                          names);
    return luaL_error(state, "%s.%s is not a scalar field; gnoblin.find matches: %s", kind, field,
                      names);
}

/* Check the table filter at `filter` against the schema of `kind`. */
static void check_find_filter(lua_State* state, const char* kind, int filter) {
    lua_pushnil(state);
    while (lua_next(state, filter)) {
        if (lua_type(state, -2) != LUA_TSTRING)
            luaL_error(state, "gnoblin.find filter keys are field names of %s", kind);
        const char* field = lua_tostring(state, -2);
        if (!lua_api_class_field_scalar(kind, field))
            find_field_error(state, kind, field);
        int type = lua_type(state, -1);
        if (type != LUA_TSTRING && type != LUA_TNUMBER && type != LUA_TBOOLEAN)
            luaL_error(state, "gnoblin.find compares %s.%s with a string, number or boolean", kind,
                       field);
        lua_pop(state, 1);
    }
}

void lua_find_check(lua_State* state, const char* kind, int filter) {
    gboolean by_function = lua_isfunction(state, filter);
    gboolean by_table = lua_istable(state, filter);
    if (!lua_isnoneornil(state, filter) && !by_function && !by_table)
        luaL_error(state, "gnoblin.find filter must be a table of fields or a function");
    if (by_table)
        check_find_filter(state, kind, lua_absindex(state, filter));
}

/* Whether the handle at `handle` equals every field of the table filter at `filter`. */
static gboolean handle_matches(lua_State* state, int handle, int filter) {
    gboolean matches = TRUE;
    lua_pushnil(state);
    while (matches && lua_next(state, filter)) {
        lua_pushvalue(state, -2);
        lua_gettable(state, handle);
        matches = lua_compare(state, -1, -2, LUA_OPEQ);
        lua_pop(state, 2);
    }
    if (!matches)
        lua_pop(state, 1); /* the key lua_next left */
    return matches;
}

void lua_find_select(lua_State* state, int everything, int filter) {
    everything = lua_absindex(state, everything);
    filter = lua_absindex(state, filter);
    gboolean by_function = lua_isfunction(state, filter);
    gboolean by_table = lua_istable(state, filter);
    lua_createtable(state, 0, 0);
    mark_array_table(state, -1);
    int result = lua_gettop(state);
    lua_Integer count = (lua_Integer)lua_rawlen(state, everything), found = 0;
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, everything, i);
        int handle = lua_gettop(state);
        gboolean keep = TRUE;
        if (by_table) {
            keep = handle_matches(state, handle, filter);
        } else if (by_function) {
            lua_pushvalue(state, filter);
            lua_pushvalue(state, handle);
            lua_call(state, 1, 1);
            keep = lua_toboolean(state, -1);
            lua_pop(state, 1);
        }
        if (keep)
            lua_rawseti(state, result, ++found);
        lua_settop(state, result);
    }
}

int lua_find(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
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

    lua_CFunction list = NULL;
    for (const LuaApiEntry* entry = lua_api; entry->path && !list; entry++)
        if (g_str_equal(entry->path, path))
            list = entry->function;
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, list, 1);
    lua_call(state, 0, 1);
    lua_find_select(state, -1, 2);
    return 1;
}
