/* The Lua Process object: gnoblin.process.spawn and the handle it returns.
 *
 * The compositor spawns the child (process.spawn completes with its pid) and reports its end with
 * the runtime event gnoblin.process.exited. The handle's wait() Operation is runtime-local, so a
 * long-lived process never holds a config reload back. A Process belongs to the runtime that
 * spawned it: after a reload the old handle is gone, the child keeps running, and its exit is
 * ignored. */
#include "gnoblin-lua-internal.h"

#include <signal.h>

#define LUA_PROCESS_METATABLE "gnoblin.Process"
#define MAX_PROCESS_STDIN_BYTES (1024 * 1024)
#define MAX_PROCESS_OUTPUT_LIMIT (4 * 1024 * 1024)
#define MAX_PROCESS_TIMEOUT_SECONDS 86400.0

static LuaConfig* process_config(lua_State* state, const char* name) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (!config || !config->processes || !config->runtime_actions ||
        (!config->dispatching && !config->api_calling))
        luaL_error(state, "%s is available only at runtime", name);
    if (config->rejecting_operations)
        luaL_error(state, "runtime operations are unavailable while rejecting a failed event");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        luaL_error(state, "too many pending Gnoblin session actions");
    return config;
}

/* Queue one compositor operation and push its Operation. */
static guint64 queue_operation(lua_State* state, LuaConfig* config, const char* method,
                               GVariant* arguments) {
    guint64 id = allocate_operation_id();
    if (!id) {
        g_variant_unref(g_variant_ref_sink(arguments));
        luaL_error(state, "Gnoblin operation request ID space is exhausted");
    }
    GVariantBuilder action;
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64((gint64)id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", arguments);
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    push_operation_handle(state, config, id, method);
    return id;
}

static const char* stream_option(lua_State* state, int options, const char* key) {
    lua_getfield(state, options, key);
    const char* value = lua_isnil(state, -1) ? "inherit" : lua_tostring(state, -1);
    if (lua_type(state, -1) != LUA_TNIL && lua_type(state, -1) != LUA_TSTRING)
        value = NULL;
    lua_pop(state, 1);
    if (!value || (!g_str_equal(value, "inherit") && !g_str_equal(value, "discard") &&
                   !g_str_equal(value, "capture")))
        luaL_error(state,
                   "gnoblin.process.spawn %s must be \"inherit\", \"discard\" or \"capture\"", key);
    return value;
}

/* The process.spawn arguments from argv (index 1) and the optional options table (index 2). */
static GVariant* spawn_arguments(lua_State* state) {
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "argv",
                          lua_command_argv_variant(state, 1, "gnoblin.process.spawn"));
    if (lua_isnoneornil(state, 2))
        return g_variant_builder_end(&arguments);
    luaL_checktype(state, 2, LUA_TTABLE);
    static const char* const fields[] = {"cwd",    "env",          "stdin",   "stdout",
                                         "stderr", "output_limit", "timeout", NULL};
    if (!table_fields_allowed(state, 2, fields))
        luaL_error(state, "gnoblin.process.spawn options accept cwd, env, stdin, stdout, stderr, "
                          "output_limit and timeout");
    g_variant_builder_add(&arguments, "{sv}", "stdout",
                          g_variant_new_string(stream_option(state, 2, "stdout")));
    g_variant_builder_add(&arguments, "{sv}", "stderr",
                          g_variant_new_string(stream_option(state, 2, "stderr")));

    lua_getfield(state, 2, "cwd");
    if (!lua_isnil(state, -1)) {
        size_t length = 0;
        const char* cwd =
            lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
        if (!cwd || strlen(cwd) != length || !g_path_is_absolute(cwd) ||
            !g_utf8_validate(cwd, -1, NULL))
            luaL_error(state, "gnoblin.process.spawn cwd must be an absolute path");
        g_variant_builder_add(&arguments, "{sv}", "cwd", g_variant_new_string(cwd));
    }
    lua_pop(state, 1);

    lua_getfield(state, 2, "env");
    if (!lua_isnil(state, -1)) {
        if (!lua_istable(state, -1))
            luaL_error(state, "gnoblin.process.spawn env must be a table of strings");
        GVariantBuilder env;
        g_variant_builder_init(&env, G_VARIANT_TYPE("a{ss}"));
        guint count = 0;
        lua_pushnil(state);
        while (lua_next(state, -2)) {
            size_t name_length = 0, value_length = 0;
            const char* name =
                lua_type(state, -2) == LUA_TSTRING ? lua_tolstring(state, -2, &name_length) : NULL;
            const char* value =
                lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &value_length) : NULL;
            if (!name || !value || !name_length || strchr(name, '=') ||
                strlen(name) != name_length || strlen(value) != value_length ||
                !g_utf8_validate(name, -1, NULL) || !g_utf8_validate(value, -1, NULL) ||
                ++count > 128) {
                g_variant_builder_clear(&env);
                luaL_error(state, "gnoblin.process.spawn env holds up to 128 NAME = \"value\" "
                                  "strings; names have no '='");
            }
            g_variant_builder_add(&env, "{ss}", name, value);
            lua_pop(state, 1);
        }
        g_variant_builder_add(&arguments, "{sv}", "env", g_variant_builder_end(&env));
    }
    lua_pop(state, 1);

    lua_getfield(state, 2, "stdin");
    if (!lua_isnil(state, -1)) {
        size_t length = 0;
        const char* bytes =
            lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
        if (!bytes || length > MAX_PROCESS_STDIN_BYTES)
            luaL_error(state, "gnoblin.process.spawn stdin must be a string of at most 1 MiB");
        g_variant_builder_add(&arguments, "{sv}", "stdin",
                              g_variant_new_fixed_array(G_VARIANT_TYPE_BYTE, bytes, length, 1));
    }
    lua_pop(state, 1);

    lua_getfield(state, 2, "output_limit");
    if (!lua_isnil(state, -1)) {
        if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 0 ||
            lua_tointeger(state, -1) > MAX_PROCESS_OUTPUT_LIMIT)
            luaL_error(state, "gnoblin.process.spawn output_limit must be an integer from 0 to %d",
                       MAX_PROCESS_OUTPUT_LIMIT);
        g_variant_builder_add(&arguments, "{sv}", "output_limit",
                              g_variant_new_int64(lua_tointeger(state, -1)));
    }
    lua_pop(state, 1);

    lua_getfield(state, 2, "timeout");
    if (!lua_isnil(state, -1)) {
        double seconds = lua_isnumber(state, -1) ? lua_tonumber(state, -1) : -1;
        if (!isfinite(seconds) || seconds <= 0 || seconds > MAX_PROCESS_TIMEOUT_SECONDS)
            luaL_error(state, "gnoblin.process.spawn timeout must be from 0 to 86400 seconds");
        g_variant_builder_add(&arguments, "{sv}", "timeout_ms",
                              g_variant_new_int64(MAX((gint64)(seconds * 1000.0 + 0.5), 1)));
    }
    lua_pop(state, 1);
    return g_variant_builder_end(&arguments);
}

static void push_process_key(lua_State* state, int process) {
    lua_getfield(state, process, "id");
}

static char* process_key(lua_State* state, int process) {
    push_process_key(state, process);
    char* key = g_strdup_printf("%" G_GUINT64_FORMAT, (guint64)lua_tointeger(state, -1));
    lua_pop(state, 1);
    return key;
}

static void forget_process(lua_State* state, LuaConfig* config, int process) {
    g_autofree char* key = process_key(state, process);
    gpointer reference = g_hash_table_lookup(config->processes, key);
    if (reference) {
        luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
        g_hash_table_remove(config->processes, key);
    }
}

/* Callback on the process.spawn Operation: record the pid, or fail wait() when spawning failed. */
static int lua_process_started(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    int process = lua_upvalueindex(2);
    lua_settop(state, 2);
    lua_getfield(state, process, "_exit");
    int exit = lua_absindex(state, -1);
    if (!lua_isnil(state, 2)) {
        lua_pushliteral(state, "failed");
        lua_setfield(state, process, "status");
        forget_process(state, config, process);
        finish_operation(state, config, exit, FALSE, 0, 2);
        return 0;
    }
    lua_getfield(state, process, "status");
    gboolean starting = g_strcmp0(lua_tostring(state, -1), "starting") == 0;
    lua_pop(state, 1);
    if (!starting)
        return 0; /* The exit event arrived first. */
    lua_getfield(state, 1, "pid");
    lua_setfield(state, process, "pid");
    lua_pushliteral(state, "running");
    lua_setfield(state, process, "status");
    return 0;
}

static int lua_process_wait(lua_State* state) {
    luaL_checkudata(state, 1, LUA_PROCESS_METATABLE);
    if (lua_gettop(state) != 1)
        return luaL_error(state, "Process:wait takes no arguments");
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "_exit");
    return 1;
}

static int lua_process_kill(lua_State* state) {
    luaL_checkudata(state, 1, LUA_PROCESS_METATABLE);
    const char* signal = luaL_optstring(state, 2, "term");
    static const char* const signals[] = {"term", "kill", "int",  "hup", "usr1",
                                          "usr2", "stop", "cont", NULL};
    gboolean known = FALSE;
    for (guint i = 0; signals[i]; i++)
        known |= g_str_equal(signal, signals[i]);
    if (!known || lua_gettop(state) > 2)
        return luaL_error(state, "Process:kill takes one of term, kill, int, hup, usr1, usr2, stop "
                                 "or cont");
    LuaConfig* config = process_config(state, "Process:kill");
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "id", g_variant_new_int64(lua_tointeger(state, -1)));
    g_variant_builder_add(&arguments, "{sv}", "signal", g_variant_new_string(signal));
    queue_operation(state, config, "process.kill", g_variant_builder_end(&arguments));
    return 1;
}

/* Reads come from the backing table, except the private _exit Operation; writes are refused. */
static int lua_process_index(lua_State* state) {
    const char* key = lua_tostring(state, 2);
    if (key && g_str_equal(key, "wait")) {
        lua_pushcfunction(state, lua_process_wait);
        return 1;
    }
    if (key && g_str_equal(key, "kill")) {
        lua_pushvalue(state, lua_upvalueindex(1));
        lua_pushcclosure(state, lua_process_kill, 1);
        return 1;
    }
    if (!key || key[0] == '_') {
        lua_pushnil(state);
        return 1;
    }
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, key);
    return 1;
}

static int lua_process_newindex(lua_State* state) {
    return luaL_error(state, "Process fields are read-only");
}

static int lua_process_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "id");
    lua_pushfstring(state, "Process %I", (LUAI_UACINT)lua_tointeger(state, -1));
    return 1;
}

static void push_process_metatable(lua_State* state, LuaConfig* config) {
    if (!luaL_newmetatable(state, LUA_PROCESS_METATABLE))
        return;
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_process_index, 1);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_process_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_process_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pushliteral(state, "Process");
    lua_setfield(state, -2, "__metatable");
}

int lua_process_spawn(lua_State* state) {
    if (lua_gettop(state) < 1 || lua_gettop(state) > 2)
        return luaL_error(state, "gnoblin.process.spawn takes argv and an optional options table");
    LuaConfig* config = process_config(state, "gnoblin.process.spawn");
    GVariant* arguments = spawn_arguments(state);
    lua_settop(state, 1);
    guint64 id = queue_operation(state, config, "process.spawn", arguments);
    int spawn = lua_absindex(state, -1);

    /* The backing table holds the state; the userdata in front keeps it read-only. */
    lua_createtable(state, 0, 6);
    int backing = lua_absindex(state, -1);
    lua_pushinteger(state, (lua_Integer)id);
    lua_setfield(state, backing, "id");
    lua_newtable(state);
    for (lua_Integer i = 1; i <= (lua_Integer)lua_rawlen(state, 1); i++) {
        lua_rawgeti(state, 1, i);
        lua_rawseti(state, -2, i);
    }
    lua_setfield(state, backing, "argv");
    lua_pushliteral(state, "starting");
    lua_setfield(state, backing, "status");
    push_local_operation(state, config, "process.wait");
    lua_setfield(state, backing, "_exit");

    lua_newuserdatauv(state, 0, 1);
    int process = lua_absindex(state, -1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, process, 1);
    push_process_metatable(state, config);
    lua_setmetatable(state, process);

    g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, id);
    lua_pushvalue(state, backing);
    g_hash_table_replace(config->processes, g_strdup(key),
                         GINT_TO_POINTER(luaL_ref(state, LUA_REGISTRYINDEX)));

    lua_pushlightuserdata(state, config);
    lua_pushvalue(state, backing);
    lua_pushcclosure(state, lua_process_started, 2);
    subscribe_operation(state, config, spawn);
    lua_pop(state, 1);
    lua_pushvalue(state, process);
    return 1;
}

static const char* signal_name(gint64 number) {
    static const struct {
        int number;
        const char* name;
    } names[] = {
        {SIGTERM, "term"}, {SIGKILL, "kill"}, {SIGINT, "int"},   {SIGHUP, "hup"},
        {SIGUSR1, "usr1"}, {SIGUSR2, "usr2"}, {SIGABRT, "abrt"}, {SIGSEGV, "segv"},
        {SIGPIPE, "pipe"}, {SIGQUIT, "quit"}, {SIGBUS, "bus"},   {SIGFPE, "fpe"},
    };
    for (gsize i = 0; i < G_N_ELEMENTS(names); i++)
        if (names[i].number == number)
            return names[i].name;
    return NULL;
}

/* gnoblin.process.exited: finish the Process's wait() Operation with its exit record. */
void process_exited(lua_State* state, LuaConfig* config, GVariant* payload) {
    gint64 id = 0;
    if (!config->processes || !g_variant_lookup(payload, "id", "x", &id) || id <= 0)
        return;
    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, id);
    gpointer reference = g_hash_table_lookup(config->processes, key);
    if (!reference)
        return;
    int base = lua_gettop(state);
    lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    int process = lua_absindex(state, -1);
    luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    g_hash_table_remove(config->processes, key);

    lua_createtable(state, 0, 7);
    int result = lua_absindex(state, -1);
    gint64 number = 0;
    gboolean flag = FALSE;
    if (g_variant_lookup(payload, "exit_code", "x", &number)) {
        lua_pushinteger(state, (lua_Integer)number);
        lua_setfield(state, result, "exit_code");
    } else if (g_variant_lookup(payload, "signal", "x", &number)) {
        const char* name = signal_name(number);
        if (name)
            lua_pushstring(state, name);
        else
            lua_pushinteger(state, (lua_Integer)number);
        lua_setfield(state, result, "signal");
    }
    g_variant_lookup(payload, "timed_out", "b", &flag);
    lua_pushboolean(state, flag);
    lua_setfield(state, result, "timed_out");
    static const char* const streams[] = {"stdout", "stderr"};
    for (guint i = 0; i < G_N_ELEMENTS(streams); i++) {
        g_autoptr(GVariant) bytes =
            g_variant_lookup_value(payload, streams[i], G_VARIANT_TYPE_BYTESTRING);
        if (!bytes)
            continue;
        push_variant(state, bytes);
        lua_setfield(state, result, streams[i]);
        g_autofree char* truncated = g_strdup_printf("%s_truncated", streams[i]);
        flag = FALSE;
        g_variant_lookup(payload, truncated, "b", &flag);
        lua_pushboolean(state, flag);
        lua_setfield(state, result, truncated);
    }
    lua_pushliteral(state, "exited");
    lua_setfield(state, process, "status");
    lua_getfield(state, result, "exit_code");
    lua_setfield(state, process, "exit_code");
    lua_getfield(state, result, "signal");
    lua_setfield(state, process, "signal");
    lua_getfield(state, process, "_exit");
    finish_operation(state, config, -1, TRUE, result, 0);
    lua_settop(state, base);
}
