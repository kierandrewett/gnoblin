#ifndef GNOBLIN_LUA_API_H
#define GNOBLIN_LUA_API_H

/* The table of every function the Lua API installs, and the property schema that the annotations
 * declare. The Lua runtime installs the table (gnoblin-lua-install.c); `gnoblinctl lua` reads the
 * same table to decide which names exist, which are wire operations and which need a runtime. */
#include <glib.h>
#include <lua.h>

typedef struct {
    const char* path;
    const char* operation;
    lua_CFunction function;
    /* The implementation takes one table of named fields, the form socket clients send. Lua
     * callers pass the annotated parameters positionally (lua_positional_call). */
    gboolean named_arguments;
} LuaApiEntry;

extern const LuaApiEntry lua_api[];
const LuaApiEntry* lua_api_find_call(const char* method);
gsize lua_api_parameter_names(const char* path, const char** names, gsize capacity);
gboolean lua_api_class_has_field(const char* class_name, const char* field);
gboolean lua_api_class_field_scalar(const char* class_name, const char* field);
char* lua_api_class_scalar_names(const char* class_name);

/* gnoblin.find, shared by the runtime and the console. The kinds that have a list, and the
 * gnoblin path of the function behind it; NULL for another kind. */
const char* lua_find_list_path(const char* kind);
/* The kinds find accepts, comma separated. */
char* lua_find_kind_names(void);
/* Raise an error unless the filter at `filter` (a table, a function, or none) is valid for
 * `kind`. */
void lua_find_check(lua_State* state, const char* kind, int filter);
/* Push the array of the handles in the array at `list` that the filter at `filter` keeps. */
void lua_find_select(lua_State* state, int list, int filter);

#endif
