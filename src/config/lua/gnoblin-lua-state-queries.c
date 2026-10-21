#include "gnoblin-lua-internal.h"
#include "../../session/gnoblin-session-class.h"

int lua_capabilities_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.capabilities.list takes no arguments");
    if (!config || !config->capability_snapshot)
        return luaL_error(state, "native capability snapshot is unavailable");

    g_autoptr(GVariant) capabilities =
        g_variant_lookup_value(config->capability_snapshot, "capabilities", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; capabilities && i < g_variant_n_children(capabilities); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(capabilities, i);
        g_autoptr(GVariant) capability = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
        push_variant(state, capability);
        lua_pushinteger(state, (lua_Integer)config->capability_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_permissions_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.list takes no arguments");
    if (!config || !config->settings_document)
        return luaL_error(state, "committed permission policy is unavailable");
    g_autofree char* path = gnoblin_config_path();
    g_autoptr(GVariant) snapshot = gnoblin_permission_policy_list(config->settings_document, path);
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_permissions_policy(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.policy takes no arguments");
    if (!config || !config->settings_document)
        return luaL_error(state, "committed permission policy is unavailable");
    g_autoptr(GVariant) snapshot =
        gnoblin_permission_policy_snapshot(config->settings_document, config->settings_revision);
    if (!snapshot)
        return luaL_error(state, "committed permission policy is unavailable");
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_privacy_state(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.privacy.state takes no arguments");
    if (!config || !config->privacy_snapshot)
        return luaL_error(state, "native privacy snapshot is unavailable");

    g_autoptr(GVariant) available =
        g_variant_lookup_value(config->privacy_snapshot, "available", G_VARIANT_TYPE_VARDICT);
    if (!available)
        return luaL_error(state, "native privacy snapshot is unavailable");
    GVariantBuilder state_builder;
    g_variant_builder_init(&state_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&state_builder, "{sv}", "available", available);
    static const char* const fields[] = {"screen_sharing", "recording",       "microphone_in_use",
                                         "camera_in_use",  "location_in_use", NULL};
    for (guint i = 0; fields[i]; i++) {
        g_autoptr(GVariant) value =
            g_variant_lookup_value(config->privacy_snapshot, fields[i], G_VARIANT_TYPE_BOOLEAN);
        if (value)
            g_variant_builder_add(&state_builder, "{sv}", fields[i], value);
    }
    g_variant_builder_add(&state_builder, "{sv}", "revision",
                          g_variant_new_uint64(config->privacy_revision));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&state_builder));
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_session_activity(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.activity takes no arguments");
    if (!config || !config->session_activity_snapshot)
        return luaL_error(state, "native session activity snapshot is unavailable");
    gboolean available = FALSE;
    gboolean idle = FALSE;
    guint64 threshold_ms = 0;
    guint64 idle_for_ms = 0;
    g_variant_lookup(config->session_activity_snapshot, "available", "b", &available);
    g_variant_lookup(config->session_activity_snapshot, "idle", "b", &idle);
    g_variant_lookup(config->session_activity_snapshot, "threshold_ms", "t", &threshold_ms);
    g_variant_lookup(config->session_activity_snapshot, "idle_for_ms", "t", &idle_for_ms);
    if (available && idle && config->session_activity_sampled_at_us > 0) {
        guint64 elapsed_ms =
            (guint64)MAX(0, g_get_monotonic_time() - config->session_activity_sampled_at_us) / 1000;
        idle_for_ms =
            G_MAXUINT64 - idle_for_ms < elapsed_ms ? G_MAXUINT64 : idle_for_ms + elapsed_ms;
    }
    GVariantBuilder snapshot_builder;
    g_variant_builder_init(&snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot_builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle", g_variant_new_boolean(idle));
    g_variant_builder_add(&snapshot_builder, "{sv}", "threshold_ms",
                          g_variant_new_uint64(threshold_ms));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle_for_ms",
                          g_variant_new_uint64(idle_for_ms));
    g_variant_builder_add(&snapshot_builder, "{sv}", "revision",
                          g_variant_new_uint64(config->session_activity_revision));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&snapshot_builder));
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

/* The class is fixed for the life of the compositor process, so it needs no config snapshot. */
int lua_session_class(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.class takes no arguments");
    lua_pushstring(state, gnoblin_session_class_name(gnoblin_session_class_from_environment()));
    return 1;
}

int lua_session_status(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.status takes no arguments");
    gboolean lock_available = FALSE;
    const char* lock_state = NULL;
    const char* session_state = NULL;
    if (config && config->session_lock_snapshot) {
        g_variant_lookup(config->session_lock_snapshot, "lock_available", "b", &lock_available);
        g_variant_lookup(config->session_lock_snapshot, "lock_state", "&s", &lock_state);
    }
    gboolean has_session_lifecycle =
        config && config->session_lifecycle_snapshot &&
        g_variant_lookup(config->session_lifecycle_snapshot, "state", "&s", &session_state);
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state", g_variant_new_string("running"));
    g_variant_builder_add(&builder, "{sv}", "lock_available",
                          g_variant_new_boolean(lock_available));
    g_variant_builder_add(&builder, "{sv}", "revision",
                          g_variant_new_uint64(config ? config->session_lock_revision : 0));
    if (lock_available && lock_state)
        g_variant_builder_add(&builder, "{sv}", "lock_state", g_variant_new_string(lock_state));
    if (has_session_lifecycle) {
        g_variant_builder_add(&builder, "{sv}", "session_state",
                              g_variant_new_string(session_state));
        g_variant_builder_add(&builder, "{sv}", "session_revision",
                              g_variant_new_uint64(config->session_lifecycle_revision));
    }
    g_autoptr(GVariant) status = g_variant_ref_sink(g_variant_builder_end(&builder));
    push_variant(state, status);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_runtime_status(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.runtime.status takes no arguments");
    if (!active_runtime)
        return luaL_error(state, "Lua runtime status is unavailable");
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state", g_variant_new_string("running"));
    g_variant_builder_add(&builder, "{sv}", "generation",
                          g_variant_new_uint64(active_runtime->generation));
    g_autoptr(GVariant) status = g_variant_ref_sink(g_variant_builder_end(&builder));
    push_variant(state, status);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_permissions_check(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* capability = NULL;
    const char* identity = NULL;
    if (lua_gettop(state) == 2 && lua_type(state, 1) == LUA_TSTRING &&
        lua_type(state, 2) == LUA_TSTRING) {
        capability = lua_tostring(state, 1);
        identity = lua_tostring(state, 2);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        static const char* const fields[] = {"capability", "identity", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "gnoblin.permissions.check accepts capability and identity");
        lua_getfield(state, 1, "capability");
        capability = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_getfield(state, 1, "identity");
        identity = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
    } else {
        return luaL_error(state, "gnoblin.permissions.check requires capability and identity");
    }
    if (!capability || !identity) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state,
                          "gnoblin.permissions.check requires string capability and identity");
    }
    if (!gnoblin_permission_capability_supported(capability)) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state, "unsupported permission capability");
    }
    if (!config || !config->settings_document) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state, "committed permission policy is unavailable");
    }
    g_auto(GnoblinPermission) decision =
        gnoblin_permission_policy_evaluate(config->settings_document, capability, identity);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(
        &result, "{sv}", "level",
        g_variant_new_string(decision.level == GNOBLIN_PERMISSION_ASK     ? "ask"
                             : decision.level == GNOBLIN_PERMISSION_ALLOW ? "allow"
                             : decision.level == GNOBLIN_PERMISSION_DENY  ? "deny"
                                                                          : "inherit"));
    g_variant_builder_add(&result, "{sv}", "rule",
                          g_variant_new_string(decision.rule ? decision.rule : ""));
    static const char* const empty_monitors[] = {NULL};
    const char* const* monitor_values =
        decision.monitors ? (const char* const*)decision.monitors : empty_monitors;
    g_variant_builder_add(&result, "{sv}", "monitors", g_variant_new_strv(monitor_values, -1));
    GVariantBuilder devices;
    g_variant_builder_init(&devices, G_VARIANT_TYPE_STRING_ARRAY);
    if (decision.devices & 1)
        g_variant_builder_add(&devices, "s", "keyboard");
    if (decision.devices & 2)
        g_variant_builder_add(&devices, "s", "pointer");
    if (decision.devices & 4)
        g_variant_builder_add(&devices, "s", "touchscreen");
    g_variant_builder_add(&result, "{sv}", "devices", g_variant_builder_end(&devices));
    g_variant_builder_add(&result, "{sv}", "clipboard", g_variant_new_boolean(decision.clipboard));
    g_variant_builder_add(&result, "{sv}", "revision",
                          g_variant_new_uint64(config->settings_revision));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&result));
    if (lua_gettop(state) == 3)
        lua_pop(state, 2);
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_input_devices(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.input.devices takes no arguments");
    if (!config || !config->input_device_snapshot)
        return luaL_error(state, "native input device snapshot is unavailable");

    g_autoptr(GVariant) devices =
        g_variant_lookup_value(config->input_device_snapshot, "devices", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    for (gsize i = 0; devices && i < g_variant_n_children(devices); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(devices, i);
        g_autoptr(GVariant) device = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        push_variant(state, device);
        lua_pushinteger(state, (lua_Integer)config->input_device_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_input_sources(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.input.sources takes no arguments");
    if (!config || !config->input_source_snapshot)
        return luaL_error(state, "native XKB input source snapshot is unavailable");

    g_autoptr(GVariant) sources =
        g_variant_lookup_value(config->input_source_snapshot, "sources", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    for (gsize i = 0; sources && i < g_variant_n_children(sources); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(sources, i);
        g_autoptr(GVariant) source = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        push_variant(state, source);
        lua_pushinteger(state, (lua_Integer)config->input_source_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

int lua_input_current_source(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.input.current_source takes no arguments");
    if (!config || !config->input_source_snapshot)
        return luaL_error(state, "native XKB input source snapshot is unavailable");

    g_autoptr(GVariant) source =
        g_variant_lookup_value(config->input_source_snapshot, "current", G_VARIANT_TYPE_VARDICT);
    if (!source) {
        lua_pushnil(state);
        return 1;
    }
    push_variant(state, source);
    lua_pushinteger(state, (lua_Integer)config->input_source_revision);
    lua_setfield(state, -2, "revision");
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_appearance_color_scheme(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.appearance.color_scheme takes no arguments");
    if (!config || !config->appearance_snapshot_received)
        return luaL_error(state, "native appearance snapshot is not ready");
    if (!config->appearance_snapshot) {
        lua_pushnil(state);
        return 1;
    }
    const char* color_scheme = NULL;
    if (!g_variant_lookup(config->appearance_snapshot, "color_scheme", "&s", &color_scheme) ||
        (!g_str_equal(color_scheme, "default") && !g_str_equal(color_scheme, "prefer-dark") &&
         !g_str_equal(color_scheme, "prefer-light")))
        return luaL_error(state, "native appearance snapshot has an invalid color scheme");
    lua_pushstring(state, color_scheme);
    return 1;
}

int lua_input_orientation_lock(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.input.orientation_lock takes no arguments");
    if (!config || !config->orientation_lock_snapshot)
        return luaL_error(state, "native orientation lock snapshot is unavailable");
    push_variant(state, config->orientation_lock_snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_shortcuts_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
    if (!config || !config->shortcut_snapshot)
        return luaL_error(state, "native shortcut snapshot is unavailable");

    g_autoptr(GVariant) shortcuts =
        g_variant_lookup_value(config->shortcut_snapshot, "shortcuts", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; shortcuts && i < g_variant_n_children(shortcuts); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(shortcuts, i);
        g_autoptr(GVariant) shortcut = g_variant_get_variant(wrapped);
        push_variant(state, shortcut);
        lua_pushinteger(state, (lua_Integer)config->shortcut_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static void push_launches_array(lua_State* state, LuaConfig* config) {
    g_autoptr(GVariant) launches =
        g_variant_lookup_value(config->launch_snapshot, "launches", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; launches && i < g_variant_n_children(launches); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(launches, i);
        g_autoptr(GVariant) launch = g_variant_get_variant(wrapped);
        push_variant(state, launch);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
}

int lua_launches_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.launches.list takes no arguments");
    if (!config || !config->launch_snapshot)
        return luaL_error(state, "native launch snapshot is unavailable");
    push_launches_array(state, config);
    return 1;
}

int lua_launches_snapshot(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.launches.snapshot takes no arguments");
    if (!config || !config->launch_snapshot)
        return luaL_error(state, "native launch snapshot is unavailable");

    lua_newtable(state);
    push_launches_array(state, config);
    lua_setfield(state, -2, "launches");
    lua_pushinteger(state, config->launch_revision);
    lua_setfield(state, -2, "revision");
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

int lua_portal_grants(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* kind_filter = NULL;
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.portals.grants takes no arguments or a filter table");
    if (lua_istable(state, 1)) {
        static const char* const fields[] = {"kind", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "gnoblin.portals.grants filter accepts only kind");
        lua_getfield(state, 1, "kind");
        if (!lua_isnil(state, -1) && lua_type(state, -1) != LUA_TSTRING)
            return luaL_error(state, "portal grant kind must be a string");
        kind_filter = lua_tostring(state, -1);
        lua_pop(state, 1);
    }
    if (kind_filter && !g_str_equal(kind_filter, "screen-cast") &&
        !g_str_equal(kind_filter, "remote-desktop"))
        return luaL_error(state, "grant kind must be screen-cast or remote-desktop");
    if (!config || !config->portal_grant_snapshot)
        return luaL_error(state, "native portal grant snapshot is unavailable");

    g_autoptr(GVariant) grants =
        g_variant_lookup_value(config->portal_grant_snapshot, "grants", G_VARIANT_TYPE("aa{sv}"));
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; grants && i < g_variant_n_children(grants); i++) {
        g_autoptr(GVariant) grant = g_variant_get_child_value(grants, i);
        const char* kind = NULL;
        g_variant_lookup(grant, "kind", "&s", &kind);
        if (kind_filter && (!kind || !g_str_equal(kind_filter, kind)))
            continue;
        push_variant(state, grant);
        lua_pushinteger(state, (lua_Integer)config->portal_grant_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, lua_rawlen(state, -2) + 1);
    }
    return 1;
}

int call_launch_operation(lua_State* state, const char* method, int arguments) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, arguments);
    lua_call(state, 1, 1);
    return 1;
}

int lua_launches_begin(lua_State* state) {
    if (lua_gettop(state) != 1 || !lua_istable(state, 1))
        return luaL_error(state, "gnoblin.launches.begin requires an options table");
    static const char* const fields[] = {"token", "application", "timeout_ms", NULL};
    if (!table_fields_allowed(state, 1, fields))
        return luaL_error(state,
                          "gnoblin.launches.begin accepts token, application, and timeout_ms");

    lua_getfield(state, 1, "token");
    if (lua_type(state, -1) != LUA_TSTRING || !*lua_tostring(state, -1) ||
        g_utf8_strlen(lua_tostring(state, -1), -1) > 128)
        return luaL_error(state,
                          "launch token must be a non-empty string of at most 128 characters");
    lua_getfield(state, 1, "application");
    if (lua_type(state, -1) != LUA_TSTRING || !*lua_tostring(state, -1) ||
        g_utf8_strlen(lua_tostring(state, -1), -1) > 512)
        return luaL_error(
            state, "launch application must be a non-empty string of at most 512 characters");
    lua_getfield(state, 1, "timeout_ms");
    if (!lua_isnil(state, -1) && (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 100 ||
                                  lua_tointeger(state, -1) > 10000))
        return luaL_error(state, "launch timeout_ms must be an integer from 100 to 10000");
    int timeout = lua_absindex(state, -1);

    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    lua_pushvalue(state, -4); /* token */
    lua_setfield(state, arguments, "token");
    lua_pushvalue(state, -3); /* application */
    lua_setfield(state, arguments, "application");
    if (!lua_isnil(state, timeout)) {
        lua_pushvalue(state, timeout);
        lua_setfield(state, arguments, "milliseconds");
    }
    lua_settop(state, arguments);
    return call_launch_operation(state, "launch.begin", arguments);
}

int lua_launches_finish(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1) ||
        g_utf8_strlen(lua_tostring(state, 1), -1) > 128)
        return luaL_error(
            state, "gnoblin.launches.finish requires a non-empty token of at most 128 characters");
    lua_newtable(state);
    lua_pushvalue(state, 1);
    lua_setfield(state, -2, "token");
    int arguments = lua_absindex(state, -1);
    return call_launch_operation(state, "launch.end", arguments);
}

int lua_shortcut_actions(lua_State* state) {
    static const char* groups[] = {"wm", "mutter", "wayland"};
    if (lua_gettop(state) > 1 ||
        (lua_gettop(state) == 1 && !lua_isnil(state, 1) && lua_type(state, 1) != LUA_TSTRING))
        return luaL_error(state, "gnoblin.shortcuts.actions takes an optional group string");

    const char* requested_group = lua_isnoneornil(state, 1) ? NULL : lua_tostring(state, 1);
    if (requested_group) {
        gboolean known = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(groups); i++)
            known |= g_str_equal(requested_group, groups[i]);
        if (!known)
            return luaL_error(
                state, "unknown shortcut action group; expected 'wm', 'mutter', or 'wayland'");
    }

    g_autoptr(GError) catalog_error = NULL;
    g_autoptr(GnoblinKeybindingCatalog) catalog = gnoblin_keybinding_catalog_load(&catalog_error);
    if (!catalog)
        return luaL_error(state, "%s", catalog_error->message);

    lua_newtable(state);
    int result = lua_absindex(state, -1);
    guint result_index = 1;
    for (guint group_index = 0; group_index < G_N_ELEMENTS(groups); group_index++) {
        const char* group = groups[group_index];
        if (requested_group && !g_str_equal(requested_group, group))
            continue;

        const GPtrArray* actions = gnoblin_keybinding_catalog_get_group(catalog, group);
        for (guint action_index = 0; actions && action_index < actions->len; action_index++) {
            const GnoblinKeybindingAction* action =
                g_ptr_array_index((GPtrArray*)actions, action_index);
            g_autofree char* public_key = g_strdup(action->key);
            g_strdelimit(public_key, "-", '_');
            g_autofree char* id = g_strdup_printf("%s.%s", group, public_key);

            lua_newtable(state);
            lua_pushstring(state, id);
            lua_setfield(state, -2, "id");
            lua_pushstring(state, group);
            lua_setfield(state, -2, "group");
            lua_pushstring(state, public_key);
            lua_setfield(state, -2, "key");
            if (action->description && *action->description) {
                lua_pushstring(state, action->description);
                lua_setfield(state, -2, "description");
            }
            lua_newtable(state);
            mark_array_table(state, -1);
            for (gsize binding_index = 0; action->default_bindings[binding_index];
                 binding_index++) {
                lua_pushstring(state, action->default_bindings[binding_index]);
                lua_rawseti(state, -2, binding_index + 1);
            }
            lua_setfield(state, -2, "default_bindings");
            push_readonly_copy(state, -1);
            lua_remove(state, -2);
            lua_rawseti(state, result, result_index++);
        }
    }

    push_readonly_copy(state, result);
    lua_remove(state, result);
    return 1;
}
