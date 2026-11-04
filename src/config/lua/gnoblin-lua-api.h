#ifndef GNOBLIN_LUA_API_H
#define GNOBLIN_LUA_API_H

/* The table of every function the Lua API installs, and the property schema that the annotations
 * declare. The Lua runtime installs the table (gnoblin-lua-install.c); `gnoblinctl lua` reads the
 * same table to decide which names exist, which are wire operations and which need a runtime. */
#include <glib.h>
#include <lua.h>

#include "gnoblin-api-operations.h"

/* How the value of one positional argument of a record method is checked and passed on. The
 * error text is derived from the kind and the path of the method: "window:move takes integer x
 * and y", "window:thumbnail width must be an integer from 1 to 480". */
typedef enum {
    LUA_ARG_END = 0,
    /* A boolean. */
    LUA_ARG_BOOLEAN,
    /* A Lua integer. */
    LUA_ARG_INTEGER,
    /* A number. */
    LUA_ARG_NUMBER,
    /* A string. */
    LUA_ARG_STRING,
    /* An integer above zero. */
    LUA_ARG_POSITIVE_INTEGER,
    /* An integer from 1 to `limit`. */
    LUA_ARG_SIZE,
    /* A Workspace, a workspace ID string or a workspace number. */
    LUA_ARG_WORKSPACE,
    /* An Output or a monitor ID string. */
    LUA_ARG_MONITOR,
    /* An optional last argument: a table whose only field is this boolean. */
    LUA_ARG_OPTION_BOOLEAN,
    /* Takes no argument; the field is true. */
    LUA_ARG_CONSTANT_TRUE,
    /* Any value but nil. */
    LUA_ARG_ANY,
} LuaApiArgKind;

typedef struct {
    /* The field of the wire arguments the value goes to. NULL: the value picks the operation and
     * is not passed on. */
    const char* field;
    LuaApiArgKind kind;
    lua_Integer limit;
} LuaApiArg;

/* The entries that take no arguments beyond the receiver. */
extern const LuaApiArg lua_no_arguments[];

typedef struct {
    const char* path;
    /* The operation behind the function: the row of the table in gnoblin-api-operations.h. */
    const GnoblinApiOperation* operation;
    lua_CFunction function;
    /* The implementation takes one table of named fields, the form socket clients send. Lua
     * callers pass the annotated parameters positionally (lua_positional_call). */
    gboolean named_arguments;
    /* A record method that has an operation and no function: the positional arguments, as
     * LUA_ARG_END ends the list. NULL for a method that packs its arguments in code. */
    const LuaApiArg* arguments;
} LuaApiEntry;

extern const LuaApiEntry lua_api[];
const LuaApiEntry* lua_api_find_call(const char* method);
gsize lua_api_parameter_names(const char* path, const char** names, gsize capacity);
/* The names of the wire fields that the record methods of `operation` take, as a NULL-ended list
 * in `names`: "id" and then the field of each argument of each method. Returns the count. */
gsize lua_api_operation_fields(const GnoblinApiOperation* operation, const char** names,
                               gsize capacity);
/* The argument that a record method of `operation` passes as wire field `field`, or NULL. */
const LuaApiArg* lua_api_operation_argument(const GnoblinApiOperation* operation,
                                            const char* field);
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
