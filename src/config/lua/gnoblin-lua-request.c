/* The Lua Request object: one shape for everything a shell answers.
 *
 * Passphrase prompts, polkit authentication, location authorization and session-lock requests
 * each arrive as their own event. The dispatcher attaches a Request to that event as
 * `event.request`, and to the follow-up events of the same request. Every Request has an `id`, a
 * `kind`, a `status` and `finished()`, an Operation that succeeds once with the outcome. Answer
 * methods exist only on the kinds that take them, and queue the existing wire operations, so the
 * compositor's per-kind validation, timeouts and recipient checks still apply.
 *
 * Kinds and their members:
 *   password, confirm  answer(...), cancel()           finished: answered | cancelled | timeout
 *   authentication     claim(identity?), answer(text),
 *                      cancel(), on("prompt"|"message") finished: authorized | denied | cancelled
 *   location           answer(allow, accuracy?), cancel() finished: allowed | denied | timeout
 *   lock               (none)                          finished: locked | timeout */
#include "gnoblin-lua-internal.h"

#define LUA_REQUEST_METATABLE "gnoblin.Request"
#define LOCK_REQUEST_SECONDS 30.0

typedef enum { FAMILY_PROMPT, FAMILY_AUTH, FAMILY_LOCATION, FAMILY_LOCK } RequestFamily;

static const char* const family_names[] = {"prompt", "auth", "location", "lock"};

static char* request_key(RequestFamily family, gint64 id) {
    return g_strdup_printf("%s:%" G_GINT64_FORMAT, family_names[family], id);
}

/* Push the backing table of a registered request, or nothing and FALSE. */
static gboolean push_request_backing(lua_State* state, LuaConfig* config, RequestFamily family,
                                     gint64 id) {
    g_autofree char* key = request_key(family, id);
    gpointer reference = config->requests ? g_hash_table_lookup(config->requests, key) : NULL;
    if (!reference)
        return FALSE;
    lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    return TRUE;
}

static void forget_request(lua_State* state, LuaConfig* config, RequestFamily family, gint64 id) {
    g_autofree char* key = request_key(family, id);
    gpointer reference = g_hash_table_lookup(config->requests, key);
    if (!reference)
        return;
    luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    g_hash_table_remove(config->requests, key);
}

/* Finish the request whose backing table is at `backing` with `result`, once. */
static void finish_request(lua_State* state, LuaConfig* config, int backing, const char* result) {
    backing = lua_absindex(state, backing);
    lua_getfield(state, backing, "status");
    gboolean finished = g_strcmp0(lua_tostring(state, -1), "finished") == 0;
    lua_pop(state, 1);
    if (finished)
        return;
    lua_pushliteral(state, "finished");
    lua_setfield(state, backing, "status");
    lua_pushstring(state, result);
    lua_setfield(state, backing, "result");
    lua_getfield(state, backing, "_family");
    RequestFamily family = (RequestFamily)lua_tointeger(state, -1);
    lua_getfield(state, backing, "id");
    gint64 id = lua_tointeger(state, -1);
    lua_pop(state, 2);
    lua_getfield(state, backing, "_finished");
    lua_pushstring(state, result);
    finish_operation(state, config, -2, TRUE, -1, 0);
    lua_pop(state, 2);
    forget_request(state, config, family, id);
}

static const char* backing_kind(lua_State* state, int backing) {
    lua_getfield(state, backing, "kind");
    const char* kind = lua_tostring(state, -1);
    lua_pop(state, 1); /* The string stays alive in the backing table. */
    return kind ? kind : "";
}

/* Callback on a request's finished() Operation: record an outcome that a timer delivered. */
static int lua_request_settled(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    int backing = lua_upvalueindex(2);
    lua_getfield(state, backing, "status");
    gboolean finished = g_strcmp0(lua_tostring(state, -1), "finished") == 0;
    lua_pop(state, 1);
    if (finished)
        return 0;
    lua_pushliteral(state, "finished");
    lua_setfield(state, backing, "status");
    lua_pushvalue(state, 1);
    lua_setfield(state, backing, "result");
    lua_getfield(state, backing, "_family");
    lua_getfield(state, backing, "id");
    forget_request(state, config, (RequestFamily)lua_tointeger(state, -2),
                   lua_tointeger(state, -1));
    return 0;
}

static int request_backing(lua_State* state) {
    luaL_checkudata(state, 1, LUA_REQUEST_METATABLE);
    lua_getiuservalue(state, 1, 1);
    return lua_absindex(state, -1);
}

/* Queue a wire operation through the generic action, which validates its arguments. The argument
 * table is on top of the stack; leaves the Operation in its place. */
static void queue_request_operation(lua_State* state, LuaConfig* config, const char* method) {
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, -2);
    lua_call(state, 1, 1);
}

static void push_request_arguments(lua_State* state, int backing) {
    lua_newtable(state);
    lua_getfield(state, backing, "id");
    lua_setfield(state, -2, "request_id");
}

static void mark_answered(lua_State* state, int backing) {
    lua_getfield(state, backing, "status");
    gboolean pending = g_strcmp0(lua_tostring(state, -1), "finished") != 0;
    lua_pop(state, 1);
    if (pending) {
        lua_pushliteral(state, "answered");
        lua_setfield(state, backing, "status");
    }
}

/* Callback on a location answer: the request is decided once the compositor accepted it. */
static int lua_location_answered(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_isnil(state, 2))
        finish_request(state, config, lua_upvalueindex(2),
                       lua_tostring(state, lua_upvalueindex(3)));
    return 0;
}

static int lua_request_answer(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    int arguments = lua_gettop(state) - 1;
    int backing = request_backing(state);
    const char* kind = backing_kind(state, backing);
    push_request_arguments(state, backing);
    int table = lua_absindex(state, -1);
    const char* method = NULL;
    const char* location_result = NULL;
    if (g_str_equal(kind, "password") || g_str_equal(kind, "confirm")) {
        gboolean password = g_str_equal(kind, "password");
        if (arguments < 1 || arguments > 2 ||
            (password ? lua_type(state, 2) != LUA_TSTRING : !lua_isboolean(state, 2)) ||
            (arguments == 2 && !lua_isboolean(state, 3)))
            return luaL_error(state, password ? "Request:answer takes a password string and an "
                                                "optional choice boolean"
                                              : "Request:answer takes a confirmed boolean and an "
                                                "optional choice boolean");
        lua_pushvalue(state, 2);
        lua_setfield(state, table, password ? "password" : "confirmed");
        if (arguments == 2) {
            lua_pushvalue(state, 3);
            lua_setfield(state, table, "choice");
        }
        method = "prompt.respond";
    } else if (g_str_equal(kind, "authentication")) {
        if (arguments != 1 || lua_type(state, 2) != LUA_TSTRING)
            return luaL_error(state, "Request:answer takes the response string for the current "
                                     "prompt");
        lua_pushvalue(state, 2);
        lua_setfield(state, table, "response");
        method = "auth.respond";
    } else if (g_str_equal(kind, "location")) {
        if (arguments < 1 || arguments > 2 || !lua_isboolean(state, 2) ||
            (arguments == 2 && !lua_isinteger(state, 3)))
            return luaL_error(state, "Request:answer takes an allow boolean and an optional "
                                     "accuracy integer");
        gboolean allow = lua_toboolean(state, 2);
        lua_pushboolean(state, allow);
        lua_setfield(state, table, "allow");
        if (arguments == 2)
            lua_pushvalue(state, 3);
        else if (allow)
            lua_getfield(state, backing, "requested_accuracy");
        else
            lua_pushinteger(state, 0);
        lua_setfield(state, table, "accuracy");
        method = "location.authorize_app";
        location_result = allow ? "allowed" : "denied";
    } else {
        return luaL_error(state, "this Request takes no answer");
    }
    queue_request_operation(state, config, method);
    mark_answered(state, backing);
    if (location_result) {
        int operation = lua_absindex(state, -1);
        lua_pushlightuserdata(state, config);
        lua_pushvalue(state, backing);
        lua_pushstring(state, location_result);
        lua_pushcclosure(state, lua_location_answered, 3);
        subscribe_operation(state, config, operation);
        lua_pop(state, 1);
    }
    return 1;
}

static int lua_request_cancel(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1)
        return luaL_error(state, "Request:cancel takes no arguments");
    int backing = request_backing(state);
    const char* kind = backing_kind(state, backing);
    if (g_str_equal(kind, "location")) {
        lua_settop(state, 1);
        lua_pushboolean(state, FALSE);
        return lua_request_answer(state);
    }
    push_request_arguments(state, backing);
    queue_request_operation(state, config,
                            g_str_equal(kind, "authentication") ? "auth.cancel" : "prompt.cancel");
    mark_answered(state, backing);
    return 1;
}

static int lua_request_claim(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 2 || (lua_gettop(state) == 2 && !lua_isinteger(state, 2)))
        return luaL_error(state, "Request:claim takes an optional identity number");
    int backing = request_backing(state);
    push_request_arguments(state, backing);
    if (lua_gettop(state) >= 3 && lua_isinteger(state, 2)) {
        lua_pushvalue(state, 2);
        lua_setfield(state, -2, "identity");
    }
    queue_request_operation(state, config, "auth.begin");
    lua_pushliteral(state, "claimed");
    lua_setfield(state, backing, "status");
    return 1;
}

static int lua_request_finished(lua_State* state) {
    if (lua_gettop(state) != 1)
        return luaL_error(state, "Request:finished takes no arguments");
    int backing = request_backing(state);
    lua_getfield(state, backing, "_finished");
    return 1;
}

/* on("prompt" | "message", fn): run fn(request) for each later prompt or message. */
static int lua_request_on(lua_State* state) {
    int backing = request_backing(state);
    const char* signal = luaL_checkstring(state, 2);
    luaL_checktype(state, 3, LUA_TFUNCTION);
    if (!g_str_equal(signal, "prompt") && !g_str_equal(signal, "message"))
        return luaL_error(state, "Request:on takes \"prompt\" or \"message\"");
    lua_getfield(state, backing, "_signals");
    lua_getfield(state, -1, signal);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, signal);
    }
    lua_pushvalue(state, 3);
    lua_rawseti(state, -2, (lua_Integer)lua_rawlen(state, -2) + 1);
    return 0;
}

static gboolean kind_has(const char* kind, const char* member) {
    gboolean answerable = g_str_equal(kind, "password") || g_str_equal(kind, "confirm") ||
                          g_str_equal(kind, "authentication") || g_str_equal(kind, "location");
    if (g_str_equal(member, "answer") || g_str_equal(member, "cancel"))
        return answerable;
    if (g_str_equal(member, "claim") || g_str_equal(member, "on"))
        return g_str_equal(kind, "authentication");
    return g_str_equal(member, "finished");
}

static int lua_request_index(lua_State* state) {
    const char* key = lua_tostring(state, 2);
    lua_getiuservalue(state, 1, 1);
    int backing = lua_absindex(state, -1);
    static const struct {
        const char* name;
        lua_CFunction function;
    } methods[] = {
        {"answer", lua_request_answer}, {"cancel", lua_request_cancel},
        {"claim", lua_request_claim},   {"finished", lua_request_finished},
        {"on", lua_request_on},
    };
    for (gsize i = 0; key && i < G_N_ELEMENTS(methods); i++) {
        if (!g_str_equal(key, methods[i].name))
            continue;
        if (!kind_has(backing_kind(state, backing), key))
            return luaL_error(state, "a %s request has no %s method", backing_kind(state, backing),
                              key);
        lua_pushvalue(state, lua_upvalueindex(1));
        lua_pushcclosure(state, methods[i].function, 1);
        return 1;
    }
    if (!key || key[0] == '_') {
        lua_pushnil(state);
        return 1;
    }
    lua_getfield(state, backing, key);
    return 1;
}

static int lua_request_newindex(lua_State* state) {
    return luaL_error(state, "Request fields are read-only");
}

static int lua_request_tostring(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_getfield(state, -1, "kind");
    lua_getfield(state, -2, "id");
    lua_pushfstring(state, "Request %s %I", lua_tostring(state, -2),
                    (LUAI_UACINT)lua_tointeger(state, -1));
    return 1;
}

static void push_request_metatable(lua_State* state, LuaConfig* config) {
    if (!luaL_newmetatable(state, LUA_REQUEST_METATABLE))
        return;
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_request_index, 1);
    lua_setfield(state, -2, "__index");
    lua_pushcfunction(state, lua_request_newindex);
    lua_setfield(state, -2, "__newindex");
    lua_pushcfunction(state, lua_request_tostring);
    lua_setfield(state, -2, "__tostring");
    lua_pushliteral(state, "Request");
    lua_setfield(state, -2, "__metatable");
}

/* Copy the payload's detail fields into the backing table. */
static void copy_details(lua_State* state, int backing, GVariant* payload) {
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_iter_init(&iter, payload);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) owned = value;
        if (g_str_equal(name, "request_id") || g_str_equal(name, "name") ||
            g_str_equal(name, "sequence") || g_str_equal(name, "time") || g_str_equal(name, "kind"))
            continue;
        push_variant(state, owned);
        lua_setfield(state, backing, name);
    }
}

/* Create and register a Request; leaves its backing table on the stack. */
static void create_request(lua_State* state, LuaConfig* config, RequestFamily family, gint64 id,
                           const char* kind, GVariant* payload) {
    lua_createtable(state, 0, 12);
    int backing = lua_absindex(state, -1);
    lua_pushinteger(state, (lua_Integer)id);
    lua_setfield(state, backing, "id");
    lua_pushstring(state, kind);
    lua_setfield(state, backing, "kind");
    lua_pushliteral(state, "pending");
    lua_setfield(state, backing, "status");
    lua_pushinteger(state, family);
    lua_setfield(state, backing, "_family");
    lua_newtable(state);
    lua_setfield(state, backing, "_signals");
    copy_details(state, backing, payload);
    push_local_operation(state, config, "request.finished");
    /* A timer can finish the Operation directly; record the outcome on the request then too. */
    lua_pushlightuserdata(state, config);
    lua_pushvalue(state, backing);
    lua_pushcclosure(state, lua_request_settled, 2);
    subscribe_operation(state, config, -2);
    lua_pop(state, 1);
    lua_setfield(state, backing, "_finished");
    lua_newuserdatauv(state, 0, 1);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, -2, 1);
    push_request_metatable(state, config);
    lua_setmetatable(state, -2);
    lua_setfield(state, backing, "_object");
    g_autofree char* key = request_key(family, id);
    forget_request(state, config, family, id);
    lua_pushvalue(state, backing);
    g_hash_table_replace(config->requests, g_steal_pointer(&key),
                         GINT_TO_POINTER(luaL_ref(state, LUA_REGISTRYINDEX)));
}

static void run_signal(lua_State* state, int backing, const char* signal) {
    lua_getfield(state, backing, "_signals");
    lua_getfield(state, -1, signal);
    if (lua_istable(state, -1)) {
        int callbacks = lua_absindex(state, -1);
        for (lua_Integer i = 1; i <= (lua_Integer)lua_rawlen(state, callbacks); i++) {
            lua_rawgeti(state, callbacks, i);
            lua_getfield(state, backing, "_object");
            if (lua_pcall(state, 1, 0, 0) != LUA_OK) {
                g_warning("Lua request %s callback failed: %s", signal, lua_error_text(state));
                lua_pop(state, 1);
            }
        }
    }
    lua_pop(state, 2);
}

static void finish_lock_requests(lua_State* state, LuaConfig* config, const char* result) {
    g_autoptr(GPtrArray) keys = g_ptr_array_new();
    GHashTableIter iter;
    gpointer key;
    g_hash_table_iter_init(&iter, config->requests);
    while (g_hash_table_iter_next(&iter, &key, NULL))
        if (g_str_has_prefix(key, "lock:"))
            g_ptr_array_add(keys, g_strdup(key));
    for (guint i = 0; i < keys->len; i++) {
        g_autofree char* owned = g_ptr_array_index(keys, i);
        gpointer reference = g_hash_table_lookup(config->requests, owned);
        if (!reference)
            continue;
        lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
        finish_request(state, config, -1, result);
        lua_pop(state, 1);
    }
}

/* Called for every runtime event before its listeners run. Returns a registry reference to the
 * Request to attach as event.request, or LUA_NOREF. */
int request_for_event(lua_State* state, LuaConfig* config, const char* event, GVariant* payload) {
    if (!config->requests || !config->timers)
        return LUA_NOREF;
    int base = lua_gettop(state);
    gint64 id = 0;
    g_variant_lookup(payload, "request_id", "x", &id);
    gboolean attach = FALSE;
    if (g_str_equal(event, "gnoblin.prompt.requested") && id > 0) {
        const char* kind = "password";
        g_variant_lookup(payload, "kind", "&s", &kind);
        create_request(state, config, FAMILY_PROMPT, id,
                       g_str_equal(kind, "confirm") ? "confirm" : "password", payload);
        attach = TRUE;
    } else if (g_str_equal(event, "gnoblin.auth.requested") && id > 0) {
        create_request(state, config, FAMILY_AUTH, id, "authentication", payload);
        attach = TRUE;
    } else if (g_str_equal(event, "gnoblin.location.authorization-requested") && id > 0) {
        create_request(state, config, FAMILY_LOCATION, id, "location", payload);
        gint64 expires_at_us = 0;
        g_variant_lookup(payload, "expires_at_us", "x", &expires_at_us);
        double seconds = (double)(expires_at_us - g_get_monotonic_time()) / G_USEC_PER_SEC;
        lua_getfield(state, -1, "_finished");
        finish_operation_after(state, config, -1, CLAMP(seconds, 0.0, 600.0), "timeout");
        lua_pop(state, 1);
        attach = TRUE;
    } else if (g_str_equal(event, "gnoblin.session.lock-requested")) {
        gint64 sequence = 0;
        g_variant_lookup(payload, "sequence", "x", &sequence);
        create_request(state, config, FAMILY_LOCK, sequence, "lock", payload);
        lua_getfield(state, -1, "_finished");
        finish_operation_after(state, config, -1, LOCK_REQUEST_SECONDS, "timeout");
        lua_pop(state, 1);
        attach = TRUE;
    } else if ((g_str_equal(event, "gnoblin.auth.prompt") ||
                g_str_equal(event, "gnoblin.auth.message")) &&
               push_request_backing(state, config, FAMILY_AUTH, id)) {
        copy_details(state, lua_absindex(state, -1), payload);
        run_signal(state, lua_absindex(state, -1),
                   g_str_equal(event, "gnoblin.auth.prompt") ? "prompt" : "message");
        attach = TRUE;
    } else if ((g_str_equal(event, "gnoblin.prompt.finished") &&
                push_request_backing(state, config, FAMILY_PROMPT, id)) ||
               (g_str_equal(event, "gnoblin.auth.finished") &&
                push_request_backing(state, config, FAMILY_AUTH, id))) {
        const char* result = "cancelled";
        g_variant_lookup(payload, "result", "&s", &result);
        lua_pushvalue(state, -1);
        finish_request(state, config, -1, result);
        lua_pop(state, 1);
        attach = TRUE;
    } else if (g_str_equal(event, "gnoblin.session.lock-state-changed")) {
        const char* lock_state = NULL;
        if (g_variant_lookup(payload, "state", "&s", &lock_state) &&
            g_str_equal(lock_state, "locked"))
            finish_lock_requests(state, config, "locked");
    }
    int reference = LUA_NOREF;
    if (attach) {
        lua_getfield(state, -1, "_object");
        reference = luaL_ref(state, LUA_REGISTRYINDEX);
    }
    lua_settop(state, base);
    return reference;
}

GHashTable* runtime_requests_new(void) {
    return g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
}
