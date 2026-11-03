#include "gnoblin-lua-internal.h"

int lua_focus_policy(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.focus.policy takes no arguments");
    lua_pushvalue(state, lua_upvalueindex(1));
    return 1;
}

int lua_gnoblin_index(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    if (lua_type(state, 2) != LUA_TSTRING) {
        lua_pushnil(state);
        return 1;
    }
    const char* property = lua_tostring(state, 2);
    static const struct {
        const char* name;
        const char* replacement;
    } removed_apis[] = {
        {"shortcut", "use gnoblin.configure.shortcuts.name = {binding = ..., command = ...}"},
        {"autostart", "use gnoblin.configure.autostart.name = {command = ...}"},
        {"remove_shortcut", "set gnoblin.configure.shortcuts.name.enable = false"},
        {"remove_autostart", "set gnoblin.configure.autostart.name.enable = false"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(removed_apis); i++)
        if (g_str_equal(property, removed_apis[i].name))
            return luaL_error(state, "gnoblin.%s was removed; %s", removed_apis[i].name,
                              removed_apis[i].replacement);
    if (g_str_equal(property, "focus")) {
        if (!config || !config->settings_document) {
            return luaL_error(state,
                              "gnoblin.focus.policy is unavailable before the config is active");
        }

        push_public_settings(state, config->settings_document, NULL, 0, 0);
        lua_getfield(state, -1, "window_management");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
        }
        int window_management = lua_absindex(state, -1);
        lua_createtable(state, 0, 7);
        int policy = lua_absindex(state, -1);
        static const struct {
            const char* name;
            const char* string_default;
            gboolean boolean_default;
            lua_Integer integer_default;
            enum { FOCUS_STRING, FOCUS_BOOLEAN, FOCUS_INTEGER } kind;
        } fields[] = {
            {"focus_mode", "click", FALSE, 0, FOCUS_STRING},
            {"focus_new_windows", "prevent", FALSE, 0, FOCUS_STRING},
            {"raise_on_click", NULL, TRUE, 0, FOCUS_BOOLEAN},
            {"auto_raise", NULL, FALSE, 0, FOCUS_BOOLEAN},
            {"focus_change_on_pointer_rest", NULL, FALSE, 0, FOCUS_BOOLEAN},
            {"auto_raise_delay", NULL, FALSE, 500, FOCUS_INTEGER},
        };
        for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
            lua_getfield(state, window_management, fields[i].name);
            if (lua_isnil(state, -1)) {
                lua_pop(state, 1);
                if (fields[i].kind == FOCUS_STRING)
                    lua_pushstring(state, fields[i].string_default);
                else if (fields[i].kind == FOCUS_BOOLEAN)
                    lua_pushboolean(state, fields[i].boolean_default);
                else
                    lua_pushinteger(state, fields[i].integer_default);
            }
            lua_setfield(state, policy, fields[i].name);
        }
        lua_pushinteger(state, (lua_Integer)config->settings_revision);
        lua_setfield(state, policy, "revision");
        push_readonly_copy(state, policy);
        int readonly_policy = lua_absindex(state, -1);
        lua_createtable(state, 0, 2);
        lua_pushvalue(state, readonly_policy);
        lua_pushcclosure(state, lua_focus_policy, 1);
        lua_setfield(state, -2, "policy");
        lua_pushlightuserdata(state, config);
        lua_pushcclosure(state, lua_focus_history, 1);
        lua_setfield(state, -2, "history");
        return 1;
    }
    if (!g_str_equal(property, "settings")) {
        lua_pushnil(state);
        return 1;
    }
    if (!config || !config->settings_document)
        return luaL_error(state, "gnoblin.settings is unavailable before the config is active");

    GVariant* settings = config->settings_document;
    push_public_settings(state, settings, NULL, 0, 0);
    lua_pushinteger(state, (lua_Integer)config->settings_revision);
    lua_setfield(state, -2, "revision");
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_snapshot(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "snapshot takes no arguments");
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    copy_config_value(state, -1, 0);
    return 1;
}

int lua_set(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    merge_table(state, -1, 1, NULL, FALSE);
    lua_pop(state, 2);
    return 0;
}

static int lua_subscription_unsubscribe(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getfield(state, 1, "_active");
    gboolean active = lua_isnil(state, -1) || lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_pushboolean(state, FALSE);
    lua_setfield(state, 1, "_active");
    lua_getfield(state, 1, "_event");
    lua_getfield(state, 1, "_callback");
    const char* event = lua_tostring(state, -2);
    if (!event)
        return 0;
    int callback = lua_absindex(state, -1);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    lua_getfield(state, -1, event);
    if (lua_istable(state, -1)) {
        int list = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, list);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, list, i);
            gboolean match = lua_rawequal(state, callback, -1);
            lua_pop(state, 1);
            if (!match)
                continue;
            lua_getglobal(state, "table");
            lua_getfield(state, -1, "remove");
            lua_pushvalue(state, list);
            lua_pushinteger(state, i);
            lua_call(state, 2, 0);
            break;
        }
    }
    return 0;
}

static int lua_listener_wrapper(lua_State* state) {
    lua_getfield(state, lua_upvalueindex(1), "_active");
    gboolean active = lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_getfield(state, lua_upvalueindex(1), "_user_callback");
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 0, 0) != LUA_OK) {
        record_runtime_callback_error("event callback", lua_error_text(state));
        lua_pushboolean(state, FALSE);
        lua_setfield(state, lua_upvalueindex(1), "_active");
        return lua_error(state);
    }
    return 0;
}

int lua_on(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    if (event_length == 0 || event_length > 128 || memchr(event, '\0', event_length) ||
        !g_utf8_validate(event, event_length, NULL))
        return luaL_error(state, "event name must contain 1 to 128 bytes");
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
    lua_newtable(state);
    int subscription = lua_absindex(state, -1);
    lua_pushstring(state, event);
    lua_setfield(state, subscription, "_event");
    lua_pushvalue(state, 2);
    lua_setfield(state, subscription, "_user_callback");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, subscription, "_active");
    lua_pushcfunction(state, lua_subscription_unsubscribe);
    lua_setfield(state, subscription, "unsubscribe");
    lua_pushvalue(state, subscription);
    lua_pushcclosure(state, lua_listener_wrapper, 1);
    lua_pushvalue(state, -1);
    lua_setfield(state, subscription, "_callback");
    lua_rawseti(state, -3, lua_rawlen(state, -3) + 1);
    lua_pushvalue(state, subscription);
    return 1;
}

int lua_mutter_on(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    if (event_length <= strlen("mutter.") || !g_str_has_prefix(event, "mutter."))
        return luaL_error(state, "Mutter event names must start with 'mutter.'");
    return lua_on(state);
}

static int lua_once_callback(lua_State* state) {
    lua_getfield(state, lua_upvalueindex(1), "fired");
    if (lua_toboolean(state, -1))
        return 0;
    lua_pop(state, 1);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, lua_upvalueindex(1), "fired");
    lua_getfield(state, lua_upvalueindex(1), "subscription");
    if (lua_istable(state, -1)) {
        lua_getfield(state, -1, "unsubscribe");
        lua_pushvalue(state, -2);
        lua_call(state, 1, 0);
    }
    lua_pop(state, 1);
    lua_getfield(state, lua_upvalueindex(1), "callback");
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 0, 0) != LUA_OK)
        return lua_error(state);
    return 0;
}

int lua_once(lua_State* state) {
    luaL_checktype(state, 2, LUA_TFUNCTION);
    lua_newtable(state);
    lua_pushvalue(state, 2);
    lua_setfield(state, -2, "callback");
    lua_pushboolean(state, FALSE);
    lua_setfield(state, -2, "fired");
    lua_pushvalue(state, -1);
    lua_pushcclosure(state, lua_once_callback, 1);
    lua_remove(state, 2);
    lua_remove(state, 2);
    lua_pushcfunction(state, lua_on);
    lua_insert(state, 1);
    lua_call(state, 2, 1);
    lua_getfield(state, 1, "_user_callback");
    lua_getupvalue(state, -1, 1);
    if (lua_istable(state, -1)) {
        lua_pushvalue(state, 1);
        lua_setfield(state, -2, "subscription");
    }
    lua_settop(state, 1);
    return 1;
}

int lua_mutter_once(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    if (event_length <= strlen("mutter.") || !g_str_has_prefix(event, "mutter."))
        return luaL_error(state, "Mutter event names must start with 'mutter.'");
    return lua_once(state);
}

gboolean workspace_id_valid(const char* id) {
    return id && g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, G_REGEX_OPTIMIZE,
                                      G_REGEX_MATCH_NOTEMPTY);
}

gboolean workspace_selector_id_valid(const char* id) {
    return workspace_id_valid(id) ||
           (id && g_regex_match_simple("^@session-[1-9][0-9]*$", id, G_REGEX_OPTIMIZE,
                                       G_REGEX_MATCH_NOTEMPTY));
}
