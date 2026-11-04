/* Easing math and tweens for Lua.
 *
 * gnoblin.ease(name, t, direction?) and gnoblin.lerp(a, b, t) are pure functions. gnoblin.tween()
 * builds a timeline of steps, Godot style: property tracks, intervals and callbacks run one
 * after another, and parallel() makes the next step run with the previous one. play() compiles
 * the timeline into one description that the compositor plays in its render loop
 * (src/control/gnoblin-control-tweens.c), so frames never wait on Lua. A tween drives only the
 * animatable properties a live object kind declares (gnoblin-lua-objects.c); callbacks run when
 * the compositor reaches them.
 *
 * A Tween is a live object (gnoblin-lua-objects.c): its id and status are fields, "finished" runs
 * when the timeline reaches its end, and "closed" runs when the tween has ended, however it
 * ended, with the outcome as its reason. The timeline itself is private to the handle. */
#include "gnoblin-lua-internal.h"

#include "../../control/gnoblin-easing.h"

#define LUA_TWEENS_KEY "gnoblin.tweens"
#define TWEEN_MAX_SECONDS 600.0

static guint64 next_tween_id;

static int check_ease(lua_State* state, int index, const char* fallback) {
    const char* name = luaL_optstring(state, index, fallback);
    int ease = gnoblin_ease_from_name(name);
    if (ease < 0)
        return luaL_error(state, "unknown ease %s", name);
    return ease;
}

static int check_direction(lua_State* state, int index) {
    const char* name = luaL_optstring(state, index, "in_out");
    int direction = gnoblin_ease_direction_from_name(name);
    if (direction < 0)
        return luaL_error(state, "unknown ease direction %s; use in, out or in_out", name);
    return direction;
}

int lua_ease(lua_State* state) {
    int ease = check_ease(state, 1, NULL);
    double t = luaL_checknumber(state, 2);
    int direction = check_direction(state, 3);
    lua_pushnumber(state, gnoblin_ease(ease, direction, t));
    return 1;
}

int lua_lerp(lua_State* state) {
    double a = luaL_checknumber(state, 1);
    double b = luaL_checknumber(state, 2);
    double t = luaL_checknumber(state, 3);
    lua_pushnumber(state, a + (b - a) * t);
    return 1;
}

static LuaConfig* tween_config(void) {
    return active_runtime ? &active_runtime->config : NULL;
}

/* The private table of the Tween that is argument 1: its steps and callbacks. */
static int tween_state(lua_State* state) {
    if (!lua_object_is(state, 1, "Tween"))
        luaL_error(state, "Tween methods are called on a Tween, as tween:method()");
    return lua_object_private(state, 1);
}

/* The status of the Tween at `handle`. */
static const char* handle_status(lua_State* state, int handle) {
    lua_getfield(state, handle, "status");
    const char* status = lua_tostring(state, -1);
    lua_pop(state, 1);
    return status ? status : "idle";
}

/* Move the Tween at `handle` to `status`: its watches and "changed" listeners run. */
static void set_status(lua_State* state, int handle, const char* status) {
    handle = lua_absindex(state, handle);
    int base = lua_gettop(state);
    lua_createtable(state, 0, 1);
    lua_pushstring(state, status);
    lua_setfield(state, -2, "status");
    lua_object_update(state, "Tween", handle, base + 1);
    lua_settop(state, base);
}

static void require_idle(lua_State* state, const char* method) {
    if (!g_str_equal(handle_status(state, 1), "idle"))
        luaL_error(state, "Tween:%s works only before play()", method);
}

/* Append step `step` (top of stack, popped) to the timeline, joining the previous group when
 * parallel() came before it. */
static void add_step(lua_State* state, int tween) {
    lua_getfield(state, tween, "next_parallel");
    gboolean parallel = lua_toboolean(state, -1);
    lua_pop(state, 1);
    lua_getfield(state, tween, "groups");
    int groups = lua_absindex(state, -1);
    lua_Integer count = (lua_Integer)lua_rawlen(state, groups);
    if (!parallel || count == 0) {
        lua_newtable(state);
        lua_rawseti(state, groups, ++count);
    }
    lua_rawgeti(state, groups, count);
    lua_pushvalue(state, -3);
    lua_rawseti(state, -2, (lua_Integer)lua_rawlen(state, -2) + 1);
    lua_pop(state, 3);
    lua_pushboolean(state, FALSE);
    lua_setfield(state, tween, "next_parallel");
}

static double check_seconds(lua_State* state, int index, const char* method) {
    double seconds = luaL_checknumber(state, index);
    if (!(seconds >= 0.0 && seconds <= TWEEN_MAX_SECONDS))
        luaL_error(state, "Tween:%s seconds must be from 0 to %g", method, TWEEN_MAX_SECONDS);
    return seconds;
}

/* property(target, property, to, seconds, ease?, direction?) */
int lua_tween_property(lua_State* state) {
    lua_settop(state, 7); /* Fixed argument slots; the state table goes above them. */
    int tween = tween_state(state);
    require_idle(state, "property");
    const char* property = luaL_checkstring(state, 3);
    const char* target_kind = lua_object_animatable_kind(state, 2, property);
    if (!target_kind) {
        char* animatable = lua_object_describe_animatable();
        lua_pushstring(state, animatable);
        g_free(animatable);
        return luaL_error(state,
                          "Tween:property takes a live object and a property it can "
                          "animate (%s)",
                          lua_tostring(state, -1));
    }
    lua_getfield(state, 2, "alive");
    gboolean alive = lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!alive)
        return luaL_error(state, "Tween:property target has ended");
    double seconds = check_seconds(state, 5, "property");
    check_ease(state, 6, "linear");
    check_direction(state, 7);
    lua_createtable(state, 0, 9);
    int step = lua_absindex(state, -1);
    lua_pushliteral(state, "track");
    lua_setfield(state, step, "kind");
    lua_getfield(state, 2, "id");
    lua_setfield(state, step, "window");
    lua_pushstring(state, target_kind);
    lua_setfield(state, step, "target_kind");
    lua_pushstring(state, property);
    lua_setfield(state, step, "property");
    if (g_str_equal(property, "offset")) {
        if (!lua_istable(state, 4) && !lua_isuserdata(state, 4))
            return luaL_error(state, "Tween:property offset takes {x = number, y = number}");
        const char* axes[2] = {"x", "y"};
        const char* keys[2] = {"to_x", "to_y"};
        for (int axis = 0; axis < 2; axis++) {
            lua_getfield(state, 4, axes[axis]);
            if (lua_type(state, -1) != LUA_TNUMBER)
                return luaL_error(state, "Tween:property offset takes {x = number, y = number}");
            lua_setfield(state, step, keys[axis]);
        }
    } else {
        luaL_checknumber(state, 4);
        lua_pushvalue(state, 4);
        lua_setfield(state, step, "to");
    }
    lua_pushnumber(state, seconds);
    lua_setfield(state, step, "seconds");
    lua_pushstring(state, lua_tostring(state, 6) ? lua_tostring(state, 6) : "linear");
    lua_setfield(state, step, "ease");
    lua_pushstring(state, lua_tostring(state, 7) ? lua_tostring(state, 7) : "in_out");
    lua_setfield(state, step, "direction");
    add_step(state, tween);
    lua_settop(state, 1);
    return 1;
}

int lua_tween_interval(lua_State* state) {
    lua_settop(state, 2);
    int tween = tween_state(state);
    require_idle(state, "interval");
    double seconds = check_seconds(state, 2, "interval");
    lua_createtable(state, 0, 2);
    lua_pushliteral(state, "interval");
    lua_setfield(state, -2, "kind");
    lua_pushnumber(state, seconds);
    lua_setfield(state, -2, "seconds");
    add_step(state, tween);
    lua_settop(state, 1);
    return 1;
}

int lua_tween_callback(lua_State* state) {
    lua_settop(state, 2);
    int tween = tween_state(state);
    require_idle(state, "callback");
    luaL_checktype(state, 2, LUA_TFUNCTION);
    lua_createtable(state, 0, 3);
    lua_pushliteral(state, "callback");
    lua_setfield(state, -2, "kind");
    lua_pushvalue(state, 2);
    lua_setfield(state, -2, "fn");
    lua_pushnumber(state, 0.0);
    lua_setfield(state, -2, "seconds");
    add_step(state, tween);
    lua_settop(state, 1);
    return 1;
}

int lua_tween_parallel(lua_State* state) {
    int tween = tween_state(state);
    require_idle(state, "parallel");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, tween, "next_parallel");
    lua_settop(state, 1);
    return 1;
}

/* Queue a tween wire operation with the argument table on top of the stack; leaves the
 * Operation in its place. */
static void queue_tween_operation(lua_State* state, LuaConfig* config, const char* method) {
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, -2);
    lua_call(state, 1, 1);
}

static void push_tween_arguments(lua_State* state) {
    lua_newtable(state);
    lua_getfield(state, 1, "id");
    lua_setfield(state, -2, "tween_id");
}

/* Completion of tween.play: record the outcome, run "finished" when the timeline reached its end,
 * and end the Tween with the outcome as the reason. Upvalue 1 is the Tween. */
static int lua_tween_ended(lua_State* state) {
    int tween = lua_upvalueindex(1);
    const char* outcome = "cancelled";
    if (lua_istable(state, 1) || lua_isuserdata(state, 1)) {
        lua_getfield(state, 1, "outcome");
        if (lua_type(state, -1) == LUA_TSTRING)
            outcome = lua_tostring(state, -1);
    }
    set_status(state, tween, outcome);
    if (lua_getfield(state, LUA_REGISTRYINDEX, LUA_TWEENS_KEY) == LUA_TTABLE) {
        lua_getfield(state, tween, "id");
        lua_pushnil(state);
        lua_rawset(state, -3);
    }
    lua_pop(state, 1);
    if (g_str_equal(outcome, "finished"))
        lua_object_raise(state, "Tween", tween, "finished");
    /* The compositor cancels every tween when it takes a new configuration: say so. */
    LuaConfig* config = tween_config();
    if (g_str_equal(outcome, "cancelled") && config && config->reload_in_flight)
        outcome = "config_reloaded";
    lua_object_end(state, "Tween", tween, outcome);
    return 0;
}

/* play(): start the timeline, or resume a paused one. Returns the play Operation, which succeeds
 * with "finished", "killed" or "cancelled". */
int lua_tween_play(lua_State* state) {
    LuaConfig* config = tween_config();
    int tween = tween_state(state);
    const char* status = handle_status(state, 1);
    if (g_str_equal(status, "paused")) {
        push_tween_arguments(state);
        queue_tween_operation(state, config, "tween.resume");
        lua_pop(state, 1);
        set_status(state, 1, "playing");
        lua_getfield(state, tween, "operation");
        return 1;
    }
    if (!g_str_equal(status, "idle"))
        return luaL_error(state, "this tween has %s", status);

    /* Compile the groups into tracks and callback times, in milliseconds. */
    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    lua_newtable(state);
    int tracks = lua_absindex(state, -1);
    lua_newtable(state);
    int callback_times = lua_absindex(state, -1);
    lua_newtable(state);
    int callback_fns = lua_absindex(state, -1);
    double start_ms = 0.0;
    lua_getfield(state, tween, "groups");
    int groups = lua_absindex(state, -1);
    for (lua_Integer g = 1; g <= (lua_Integer)lua_rawlen(state, groups); g++) {
        lua_rawgeti(state, groups, g);
        int group = lua_absindex(state, -1);
        double group_ms = 0.0;
        for (lua_Integer s = 1; s <= (lua_Integer)lua_rawlen(state, group); s++) {
            lua_rawgeti(state, group, s);
            int step = lua_absindex(state, -1);
            lua_getfield(state, step, "seconds");
            double duration_ms = lua_tonumber(state, -1) * 1000.0;
            lua_pop(state, 1);
            group_ms = MAX(group_ms, duration_ms);
            lua_getfield(state, step, "kind");
            const char* kind = lua_tostring(state, -1);
            lua_pop(state, 1);
            if (g_str_equal(kind, "track")) {
                lua_createtable(state, 0, 8);
                static const char* const copied[] = {"window", "target_kind", "property",
                                                     "to",     "to_x",        "to_y",
                                                     "ease",   "direction",   NULL};
                for (guint i = 0; copied[i]; i++) {
                    lua_getfield(state, step, copied[i]);
                    lua_setfield(state, -2, copied[i]);
                }
                lua_pushinteger(state, (lua_Integer)start_ms);
                lua_setfield(state, -2, "start_ms");
                lua_pushinteger(state, (lua_Integer)duration_ms);
                lua_setfield(state, -2, "duration_ms");
                lua_rawseti(state, tracks, (lua_Integer)lua_rawlen(state, tracks) + 1);
            } else if (g_str_equal(kind, "callback")) {
                lua_pushinteger(state, (lua_Integer)start_ms);
                lua_rawseti(state, callback_times,
                            (lua_Integer)lua_rawlen(state, callback_times) + 1);
                lua_getfield(state, step, "fn");
                lua_rawseti(state, callback_fns, (lua_Integer)lua_rawlen(state, callback_fns) + 1);
            }
            lua_pop(state, 1);
        }
        start_ms += group_ms;
        lua_pop(state, 1);
    }
    lua_pop(state, 1); /* groups */
    if (lua_rawlen(state, tracks) == 0)
        return luaL_error(state, "a tween needs at least one property track");
    mark_array_table(state, tracks);
    mark_array_table(state, callback_times);
    lua_pushvalue(state, callback_fns);
    lua_setfield(state, tween, "callback_fns");

    lua_getfield(state, 1, "id");
    const char* id = lua_tostring(state, -1);
    lua_pushvalue(state, -1);
    lua_setfield(state, arguments, "tween_id");
    lua_pushinteger(state, (lua_Integer)MAX(start_ms, 1.0));
    lua_setfield(state, arguments, "duration_ms");
    lua_pushvalue(state, tracks);
    lua_setfield(state, arguments, "tracks");
    lua_pushvalue(state, callback_times);
    lua_setfield(state, arguments, "callbacks");
    lua_pushvalue(state, arguments);
    queue_tween_operation(state, config, "tween.play");
    int operation = lua_absindex(state, -1);
    lua_pushvalue(state, operation);
    lua_setfield(state, tween, "operation");
    set_status(state, 1, "playing");

    /* Keep the tween alive while it plays, so its callbacks can run. */
    if (lua_getfield(state, LUA_REGISTRYINDEX, LUA_TWEENS_KEY) != LUA_TTABLE) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, LUA_REGISTRYINDEX, LUA_TWEENS_KEY);
    }
    lua_pushvalue(state, 1);
    lua_setfield(state, -2, id);
    lua_pop(state, 1);

    lua_pushvalue(state, 1);
    lua_pushcclosure(state, lua_tween_ended, 1);
    subscribe_operation(state, config, operation);
    lua_pop(state, 1);
    lua_pushvalue(state, operation);
    return 1;
}

static int lua_tween_control(lua_State* state, const char* method, const char* status) {
    LuaConfig* config = tween_config();
    lua_settop(state, 2);
    tween_state(state);
    const char* current = handle_status(state, 1);
    if (!g_str_equal(current, "playing") && !g_str_equal(current, "paused"))
        return luaL_error(state, "this tween is not playing");
    push_tween_arguments(state);
    if (g_str_equal(method, "tween.seek")) {
        double seconds = check_seconds(state, 2, "seek");
        lua_pushinteger(state, (lua_Integer)(seconds * 1000.0));
        lua_setfield(state, -2, "milliseconds");
    }
    queue_tween_operation(state, config, method);
    if (status)
        set_status(state, 1, status);
    return 1;
}

int lua_tween_pause(lua_State* state) {
    return lua_tween_control(state, "tween.pause", "paused");
}

int lua_tween_seek(lua_State* state) {
    return lua_tween_control(state, "tween.seek", NULL);
}

int lua_tween_kill(lua_State* state) {
    return lua_tween_control(state, "tween.kill", NULL);
}

/* gnoblin.tween(): a new, empty tween. */
int lua_tween_new(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.tween takes no arguments");
    lua_createtable(state, 0, 3);
    int record = lua_absindex(state, -1);
    g_autofree char* id = g_strdup_printf("%" G_GUINT64_FORMAT, ++next_tween_id);
    lua_pushstring(state, id);
    lua_setfield(state, record, "id");
    lua_pushliteral(state, "idle");
    lua_setfield(state, record, "status");
    lua_pushinteger(state, 1);
    lua_setfield(state, record, "revision");
    lua_object_apply(state, "Tween", record);
    lua_object_private(state, -1);
    lua_newtable(state);
    lua_setfield(state, -2, "groups");
    lua_pop(state, 1);
    return 1;
}

/* gnoblin.tween.callback {tween_id, index}: run that callback of the playing tween. */
gboolean lua_tween_dispatch_event(lua_State* state, const char* event, GVariant* payload) {
    if (!g_str_equal(event, "gnoblin.tween.callback"))
        return FALSE;
    const char* id = NULL;
    gint64 index = 0;
    if (!g_variant_lookup(payload, "tween_id", "&s", &id) ||
        !g_variant_lookup(payload, "index", "x", &index))
        return FALSE;
    int base = lua_gettop(state);
    gboolean ran = FALSE;
    if (lua_getfield(state, LUA_REGISTRYINDEX, LUA_TWEENS_KEY) == LUA_TTABLE &&
        lua_getfield(state, -1, id) == LUA_TUSERDATA) {
        int tween = lua_absindex(state, -1);
        lua_object_private(state, tween);
        lua_getfield(state, -1, "callback_fns");
        if (lua_istable(state, -1) && lua_rawgeti(state, -1, index) == LUA_TFUNCTION) {
            lua_pushvalue(state, tween);
            call_handler(state, 1, "Tween callback", "callback");
            ran = TRUE;
        }
    }
    lua_settop(state, base);
    return ran;
}
