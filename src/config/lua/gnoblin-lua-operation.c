/* The Lua Operation object: the handle every asynchronous call returns, its callbacks, the
 * runtime-local operations (timers, map, timeout) and cancellation.
 *
 * A compositor operation is registered in config->operations by its request ID and completes when
 * the compositor's gnoblin.operation.completed event arrives. A local operation is never
 * registered there, because a reload waits until that table is empty and a long timer must not
 * hold a reload back. Local completions queue their callbacks on config->deferred_callbacks, so
 * they run after the current dispatch with the usual instruction limit. */
#include "gnoblin-lua-internal.h"

#define LUA_OPERATION_METATABLE "gnoblin.Operation"
#define MAX_TIMER_SECONDS 86400.0
#define MAX_RUNTIME_TIMERS 1024

typedef struct {
    LuaConfig* config;
    lua_State* state;
    int handle; /* Registry reference to the Operation the timer finishes. */
    guint source;
    gboolean timeout; /* Fail with timed_out instead of succeeding with nil. */
} LuaTimer;

static int lua_operation_unsubscribe(lua_State* state);

guint64 allocate_operation_id(void) {
    if (next_runtime_operation_id >= G_MAXINT64)
        return 0;
    return ++next_runtime_operation_id;
}

static char* operation_key(lua_State* state, int handle) {
    lua_getfield(state, handle, "id");
    char* key = g_strdup_printf("%" G_GUINT64_FORMAT, (guint64)lua_tointeger(state, -1));
    lua_pop(state, 1);
    return key;
}

static gboolean operation_pending(lua_State* state, int handle) {
    lua_getfield(state, handle, "status");
    gboolean pending = lua_isstring(state, -1) && g_str_equal(lua_tostring(state, -1), "pending");
    lua_pop(state, 1);
    return pending;
}

void push_operation_error(lua_State* state, const char* code, const char* message) {
    lua_createtable(state, 0, 2);
    lua_pushstring(state, code);
    lua_setfield(state, -2, "code");
    lua_pushstring(state, message);
    lua_setfield(state, -2, "message");
}

static void push_operation_subscription(lua_State* state, int operation, int callback) {
    operation = lua_absindex(state, operation);
    callback = lua_absindex(state, callback);
    lua_newtable(state);
    lua_pushvalue(state, operation);
    lua_setfield(state, -2, "_operation");
    lua_pushvalue(state, callback);
    lua_setfield(state, -2, "_callback");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "_active");
    lua_pushcfunction(state, lua_operation_unsubscribe);
    lua_setfield(state, -2, "unsubscribe");
}

/* Run each active callback of a finished Operation now, with its value and error. */
void run_operation_callbacks(lua_State* state, int handle) {
    handle = lua_absindex(state, handle);
    lua_getfield(state, handle, "_callbacks");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    int registered = lua_absindex(state, -1);
    lua_newtable(state);
    lua_setfield(state, handle, "_callbacks");
    lua_Integer count = lua_rawlen(state, registered);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, registered, i);
        int subscription = lua_absindex(state, -1);
        lua_getfield(state, subscription, "_active");
        gboolean active = lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (active) {
            lua_pushboolean(state, FALSE);
            lua_setfield(state, subscription, "_active");
            lua_getfield(state, subscription, "_callback");
            lua_getfield(state, handle, "value");
            lua_getfield(state, handle, "error");
            if (lua_pcall(state, 2, 0, 0) != LUA_OK) {
                g_warning("Lua operation completion callback failed: %s", lua_error_text(state));
                lua_pop(state, 1);
            }
        }
        lua_pop(state, 1);
    }
    lua_pop(state, 1);
}

/* Move the active callbacks of a finished Operation to the deferred queue. */
static void queue_operation_callbacks(lua_State* state, LuaConfig* config, int handle) {
    lua_getfield(state, handle, "_callbacks");
    if (lua_istable(state, -1)) {
        int registered = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, registered);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, registered, i);
            g_ptr_array_add(config->deferred_callbacks,
                            GINT_TO_POINTER(luaL_ref(state, LUA_REGISTRYINDEX)));
        }
        lua_newtable(state);
        lua_setfield(state, handle, "_callbacks");
    }
    lua_pop(state, 1);
    schedule_deferred_callbacks(config);
}

static void timer_free(gpointer data) {
    LuaTimer* timer = data;
    if (timer->source)
        g_source_remove(timer->source);
    g_free(timer);
}

/* Finish a pending Operation in the runtime. `value` and `error` are stack indexes, or 0 for nil.
 * Stops the Operation's timer, detaches it from the operation it waits on, and stops waiting for
 * the compositor when it was a compositor operation. */
void finish_operation(lua_State* state, LuaConfig* config, int handle, gboolean ok, int value,
                      int error) {
    handle = lua_absindex(state, handle);
    value = value ? lua_absindex(state, value) : 0;
    error = error ? lua_absindex(state, error) : 0;
    if (!operation_pending(state, handle))
        return;
    g_autofree char* key = operation_key(state, handle);
    LuaTimer* timer = g_hash_table_lookup(config->timers, key);
    if (timer) {
        luaL_unref(state, LUA_REGISTRYINDEX, timer->handle);
        g_hash_table_remove(config->timers, key);
    }
    gpointer reference = g_hash_table_lookup(config->operations, key);
    if (reference) {
        luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
        g_hash_table_remove(config->operations, key);
    }
    lua_getfield(state, handle, "_source");
    if (lua_istable(state, -1)) {
        lua_pushcfunction(state, lua_operation_unsubscribe);
        lua_insert(state, -2);
        lua_call(state, 1, 0);
    } else {
        lua_pop(state, 1);
    }
    lua_pushstring(state, ok ? "succeeded" : "failed");
    lua_setfield(state, handle, "status");
    if (value)
        lua_pushvalue(state, value);
    else
        lua_pushnil(state);
    lua_setfield(state, handle, "value");
    if (error)
        lua_pushvalue(state, error);
    else
        lua_pushnil(state);
    lua_setfield(state, handle, "error");
    queue_operation_callbacks(state, config, handle);
}

static int lua_operation_on_complete(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (operation_pending(state, 1)) {
        lua_getfield(state, 1, "_callbacks");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, 1, "_callbacks");
        }
        int callbacks = lua_absindex(state, -1);
        push_operation_subscription(state, 1, 2);
        lua_pushvalue(state, -1);
        lua_rawseti(state, callbacks, lua_rawlen(state, callbacks) + 1);
        return 1;
    }
    if (!config || !config->deferred_callbacks)
        return luaL_error(state, "deferred operation callbacks are unavailable");
    push_operation_subscription(state, 1, 2);
    lua_pushvalue(state, -1);
    g_ptr_array_add(config->deferred_callbacks,
                    GINT_TO_POINTER(luaL_ref(state, LUA_REGISTRYINDEX)));
    schedule_deferred_callbacks(config);
    return 1;
}

static int lua_operation_unsubscribe(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getfield(state, 1, "_active");
    gboolean active = lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_pushboolean(state, FALSE);
    lua_setfield(state, 1, "_active");
    lua_getfield(state, 1, "_operation");
    lua_getfield(state, -1, "_callbacks");
    if (lua_istable(state, -1)) {
        int callbacks = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, callbacks);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, callbacks, i);
            gboolean match = lua_rawequal(state, 1, -1);
            lua_pop(state, 1);
            if (!match)
                continue;
            for (lua_Integer j = i; j < count; j++) {
                lua_rawgeti(state, callbacks, j + 1);
                lua_rawseti(state, callbacks, j);
            }
            lua_pushnil(state);
            lua_rawseti(state, callbacks, count);
            break;
        }
    }
    return 0;
}

static LuaConfig* runtime_config(lua_State* state, const char* name) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (!config || !config->deferred_callbacks || !config->timers ||
        (!config->dispatching && !config->api_calling))
        luaL_error(state, "%s is available only at runtime", name);
    return config;
}

static void push_operation_metatable(lua_State* state, LuaConfig* config);

/* Push a new pending Operation that the runtime itself finishes. */
void push_local_operation(lua_State* state, LuaConfig* config, const char* method) {
    guint64 id = allocate_operation_id();
    if (!id)
        luaL_error(state, "Gnoblin operation request ID space is exhausted");
    lua_createtable(state, 0, 5);
    lua_pushinteger(state, (lua_Integer)id);
    lua_setfield(state, -2, "id");
    lua_pushstring(state, method);
    lua_setfield(state, -2, "method");
    lua_pushliteral(state, "pending");
    lua_setfield(state, -2, "status");
    push_operation_metatable(state, config);
    lua_setmetatable(state, -2);
}

/* Cancel: stop waiting. A timer stops. An action already sent to the compositor still runs, and
 * its result is discarded. */
static int lua_operation_cancel(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    LuaConfig* config = runtime_config(state, "Operation:cancel");
    if (!operation_pending(state, 1)) {
        lua_pushboolean(state, FALSE);
        return 1;
    }
    push_operation_error(state, "cancelled", "the operation was cancelled");
    finish_operation(state, config, 1, FALSE, 0, -1);
    lua_pushboolean(state, TRUE);
    return 1;
}

/* Callback on a source Operation that finishes the derived Operation in upvalue 2. With a function
 * in upvalue 3 it maps a successful value; without one it passes the result through. */
static int lua_operation_forward(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    int derived = lua_upvalueindex(2);
    lua_settop(state, 2);
    if (!lua_isnil(state, 2)) {
        finish_operation(state, config, derived, FALSE, 0, 2);
        return 0;
    }
    if (lua_isnil(state, lua_upvalueindex(3))) {
        finish_operation(state, config, derived, TRUE, 1, 0);
        return 0;
    }
    lua_pushvalue(state, lua_upvalueindex(3));
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 1, 0) != LUA_OK) {
        push_operation_error(state, "internal", lua_error_text(state));
        finish_operation(state, config, derived, FALSE, 0, -1);
        return 0;
    }
    finish_operation(state, config, derived, TRUE, -1, 0);
    return 0;
}

/* Subscribe the function on top of the stack to the Operation at `operation`. Replaces the function
 * with the subscription. */
void subscribe_operation(lua_State* state, LuaConfig* config, int operation) {
    operation = lua_absindex(state, operation);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_operation_on_complete, 1);
    lua_insert(state, -2);
    lua_pushvalue(state, operation);
    lua_insert(state, -2);
    lua_call(state, 2, 1);
}

/* Push an Operation that finishes when `source` does, mapped through `map` when it is not 0. */
static int push_derived_operation(lua_State* state, LuaConfig* config, int source, int map,
                                  const char* method) {
    source = lua_absindex(state, source);
    map = map ? lua_absindex(state, map) : 0;
    push_local_operation(state, config, method);
    int derived = lua_absindex(state, -1);
    lua_pushlightuserdata(state, config);
    lua_pushvalue(state, derived);
    if (map)
        lua_pushvalue(state, map);
    else
        lua_pushnil(state);
    lua_pushcclosure(state, lua_operation_forward, 3);
    subscribe_operation(state, config, source);
    lua_setfield(state, derived, "_source");
    return derived;
}

static int lua_operation_map(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    if (lua_gettop(state) != 2)
        return luaL_error(state, "Operation:map takes one function");
    LuaConfig* config = runtime_config(state, "Operation:map");
    push_derived_operation(state, config, 1, 2, "operation.map");
    return 1;
}

static gboolean timer_fired(gpointer data);

static double timer_seconds(lua_State* state, int index, const char* name) {
    if (!lua_isnumber(state, index))
        luaL_error(state, "%s requires a number of seconds", name);
    double seconds = lua_tonumber(state, index);
    if (!isfinite(seconds) || seconds < 0 || seconds > MAX_TIMER_SECONDS)
        luaL_error(state, "%s seconds must be from 0 to %d", name, (int)MAX_TIMER_SECONDS);
    return seconds;
}

/* Arm a timer that finishes the Operation at `handle` after `seconds`. */
static void start_timer(lua_State* state, LuaConfig* config, int handle, double seconds,
                        gboolean timeout) {
    handle = lua_absindex(state, handle);
    if (g_hash_table_size(config->timers) >= MAX_RUNTIME_TIMERS)
        luaL_error(state, "too many pending timers");
    LuaTimer* timer = g_new0(LuaTimer, 1);
    timer->config = config;
    timer->state = state;
    timer->timeout = timeout;
    lua_pushvalue(state, handle);
    timer->handle = luaL_ref(state, LUA_REGISTRYINDEX);
    timer->source = g_timeout_add_full(G_PRIORITY_DEFAULT, (guint)(seconds * 1000.0 + 0.5),
                                       timer_fired, timer, NULL);
    g_hash_table_replace(config->timers, operation_key(state, handle), timer);
}

static int lua_operation_timeout(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    if (lua_gettop(state) != 2)
        return luaL_error(state, "Operation:timeout takes a number of seconds");
    double seconds = timer_seconds(state, 2, "Operation:timeout");
    LuaConfig* config = runtime_config(state, "Operation:timeout");
    int derived = push_derived_operation(state, config, 1, 0, "operation.timeout");
    start_timer(state, config, derived, seconds, TRUE);
    lua_settop(state, derived);
    return 1;
}

static int finish_timer(lua_State* state) {
    LuaTimer* timer = lua_touserdata(state, 1);
    LuaConfig* config = timer->config;
    lua_rawgeti(state, LUA_REGISTRYINDEX, timer->handle);
    int handle = lua_absindex(state, -1);
    if (timer->timeout) {
        push_operation_error(state, "timed_out", "the operation did not finish in time");
        finish_operation(state, config, handle, FALSE, 0, -1);
    } else {
        finish_operation(state, config, handle, TRUE, 0, 0);
    }
    return 0;
}

static gboolean timer_fired(gpointer data) {
    LuaTimer* timer = data;
    timer->source = 0; /* The source ends when this returns; finish_operation frees the timer. */
    lua_State* state = timer->state;
    int base = lua_gettop(state);
    lua_pushcfunction(state, finish_timer);
    lua_pushlightuserdata(state, timer);
    if (lua_pcall(state, 1, 0, 0) != LUA_OK)
        g_warning("gnoblin: could not finish a Lua timer: %s", lua_error_text(state));
    lua_settop(state, base);
    return G_SOURCE_REMOVE;
}

int lua_timers_after(lua_State* state) {
    if (lua_gettop(state) != 1)
        return luaL_error(state, "gnoblin.timers.after takes a number of seconds");
    double seconds = timer_seconds(state, 1, "gnoblin.timers.after");
    LuaConfig* config = runtime_config(state, "gnoblin.timers.after");
    push_local_operation(state, config, "timers.after");
    start_timer(state, config, -1, seconds, FALSE);
    return 1;
}

int lua_timers_now(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.timers.now takes no arguments");
    lua_pushnumber(state, (lua_Number)g_get_monotonic_time() / G_USEC_PER_SEC);
    return 1;
}

GHashTable* runtime_timers_new(void) {
    return g_hash_table_new_full(g_str_hash, g_str_equal, g_free, timer_free);
}

static void push_operation_metatable(lua_State* state, LuaConfig* config) {
    if (!luaL_newmetatable(state, LUA_OPERATION_METATABLE))
        return;
    static const struct {
        const char* name;
        lua_CFunction function;
    } methods[] = {
        {"on_complete", lua_operation_on_complete},
        {"map", lua_operation_map},
        {"timeout", lua_operation_timeout},
        {"cancel", lua_operation_cancel},
    };
    lua_createtable(state, 0, G_N_ELEMENTS(methods));
    for (gsize i = 0; i < G_N_ELEMENTS(methods); i++) {
        lua_pushlightuserdata(state, config);
        lua_pushcclosure(state, methods[i].function, 1);
        lua_setfield(state, -2, methods[i].name);
    }
    lua_setfield(state, -2, "__index");
}

void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id, const char* method) {
    lua_createtable(state, 0, 5);
    lua_pushinteger(state, id);
    lua_setfield(state, -2, "id");
    lua_pushstring(state, method);
    lua_setfield(state, -2, "method");
    lua_pushliteral(state, "pending");
    lua_setfield(state, -2, "status");
    push_operation_metatable(state, config);
    lua_setmetatable(state, -2);
    if (config->operations) {
        g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, id);
        lua_pushvalue(state, -1);
        int reference = luaL_ref(state, LUA_REGISTRYINDEX);
        g_hash_table_replace(config->operations, g_strdup(key), GINT_TO_POINTER(reference));
    }
}
