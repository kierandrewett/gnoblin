#include "gnoblin-lua-internal.h"

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

/* The shortcuts the configuration declares, as read-only records. */
int lua_shortcuts_configured(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.configured takes no arguments");
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

/* gnoblin.shortcuts.bind: bind the accelerator and return the Shortcut. The Shortcut exists
 * before the compositor confirms the binding, with the values it was asked for. When the
 * binding fails it ends, and its "closed" listeners get the reason. */
int lua_shortcuts_bind(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_settop(state, 1);
    lua_getfield(state, 1, "id");
    if (lua_type(state, 2) == LUA_TSTRING &&
        lua_object_push_registered(state, "Shortcut", lua_tostring(state, 2))) {
        lua_getfield(state, -1, "alive");
        gboolean alive = lua_toboolean(state, -1);
        if (alive)
            return luaL_error(state, "shortcut %s is already bound; unbind it first",
                              lua_tostring(state, 2));
        lua_object_forget(state, "Shortcut", lua_tostring(state, 2));
    }
    lua_settop(state, 1);
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, "shortcut.bind");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_pushvalue(state, 1);
    lua_call(state, 1, 1);
    int operation = lua_gettop(state);

    lua_createtable(state, 0, 6);
    int record = lua_gettop(state);
    static const struct {
        const char* field;
        const char* fallback;
    } defaults[] = {{"id", NULL},
                    {"accelerator", NULL},
                    {"hold", "none"},
                    {"trigger", "press"},
                    {"mode", "passive"}};
    for (gsize i = 0; i < G_N_ELEMENTS(defaults); i++) {
        lua_getfield(state, 1, defaults[i].field);
        if (lua_isnil(state, -1) && defaults[i].fallback) {
            lua_pop(state, 1);
            lua_pushstring(state, defaults[i].fallback);
        }
        lua_setfield(state, record, defaults[i].field);
    }
    lua_getfield(state, 1, "capture_input");
    lua_pushboolean(state, lua_toboolean(state, -1));
    lua_remove(state, -2);
    lua_setfield(state, record, "capture_input");
    push_readonly_copy(state, record);
    lua_object_hold(state, "Shortcut", -1);
    lua_object_end_after(state, config, operation, "Shortcut", -1, FALSE);
    return 1;
}

/* gnoblin.idle.watch: ask the compositor for an idle watch and return the IdleWatch. The watch
 * exists before the compositor confirms it, not idle; when the compositor refuses it, it ends and
 * its "closed" listeners get the reason. Its ID is "idle-" and the ID of the request. */
int lua_idle_watch(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    static const char* const fields[] = {"seconds", NULL};
    if (lua_gettop(state) != 1 || !lua_istable(state, 1) || !table_fields_allowed(state, 1, fields))
        return luaL_error(state, "gnoblin.idle.watch takes the idle time in seconds");
    lua_getfield(state, 1, "seconds");
    double seconds = lua_isnumber(state, -1) ? lua_tonumber(state, -1) : 0;
    if (lua_type(state, -1) != LUA_TNUMBER || !(seconds > 0) || seconds > 86400)
        return luaL_error(state, "gnoblin.idle.watch needs seconds above 0 and at most 86400");
    lua_settop(state, 0);
    lua_createtable(state, 0, 1);
    lua_pushnumber(state, seconds);
    lua_setfield(state, 1, "seconds");
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, "idle.watch");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_pushvalue(state, 1);
    lua_call(state, 1, 1);
    int operation = lua_gettop(state);

    lua_createtable(state, 0, 3);
    int record = lua_gettop(state);
    lua_getfield(state, operation, "id");
    lua_pushfstring(state, "idle-%I", (lua_Integer)lua_tointeger(state, -1));
    lua_setfield(state, record, "id");
    lua_pop(state, 1);
    lua_pushnumber(state, seconds);
    lua_setfield(state, record, "seconds");
    lua_pushboolean(state, FALSE);
    lua_setfield(state, record, "idle");
    push_readonly_copy(state, record);
    lua_object_hold(state, "IdleWatch", -1);
    lua_object_end_after(state, config, operation, "IdleWatch", -1, FALSE);
    return 1;
}

/* gnoblin.shortcuts.list: the Shortcuts this runtime has bound and not unbound. */
int lua_shortcuts_list(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
    lua_object_push_held(state, "Shortcut");
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
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    call_launch_operation(state, "launch.begin", arguments);
    int operation = lua_gettop(state);

    /* The Launch exists before the compositor confirms it, with the values it was asked for; the
     * confirmation and each later change update this same object. Beginning a token again ends
     * the Launch that had it. */
    const char* token = lua_tostring(state, 2);
    if (lua_object_push_registered(state, "Launch", token)) {
        lua_object_end(state, "Launch", -1, "replaced");
        lua_pop(state, 1);
        lua_object_forget(state, "Launch", token);
    }
    lua_createtable(state, 0, 6);
    int record = lua_gettop(state);
    lua_pushvalue(state, 2);
    lua_setfield(state, record, "token");
    lua_pushvalue(state, 3);
    lua_setfield(state, record, "application");
    lua_pushinteger(state, (lua_Integer)(g_get_real_time() / 1000));
    lua_setfield(state, record, "started_at");
    lua_pushinteger(state, lua_isnil(state, 4) ? 3000 : lua_tointeger(state, 4));
    lua_setfield(state, record, "timeout_ms");
    lua_pushliteral(state, "pending");
    lua_setfield(state, record, "state");
    lua_pushinteger(state, 0);
    lua_setfield(state, record, "revision");
    push_readonly_copy(state, record);
    lua_object_end_after(state, config, operation, "Launch", -1, FALSE);
    return 1;
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

/* Overrides: gnoblin.settings.override(path, value) and Window:set_effect(name, value). Both
 * queue one operation and return an Override that lives until the compositor ends it. The
 * compositor owns the rules (gnoblin-control-overrides.c); this file checks what the runtime can
 * check at once, so a mistake is a Lua error at the call and not a late failure. */
#include "../../control/gnoblin-overrides.h"

/* Return `value` with each table key in the document's kebab case, which is the form the
 * window-rule engine reads. The names under shader_uniforms are uniform names and stay as they
 * are. The caller owns the result. */
static GVariant* override_kebab_value(GVariant* value, gboolean literal) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        GVariantBuilder builder;
        GVariantIter iter;
        const char* key = NULL;
        GVariant* child = NULL;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            g_autoptr(GVariant) owned = child;
            g_autofree char* name = g_strdup(key);
            if (!literal)
                g_strdelimit(name, "_", '-');
            g_autoptr(GVariant) converted =
                override_kebab_value(owned, literal || g_str_equal(name, "shader-uniforms"));
            g_variant_builder_add(&builder, "{sv}", name, converted);
        }
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
            g_autoptr(GVariant) inner = g_variant_get_variant(boxed);
            g_autoptr(GVariant) converted = override_kebab_value(inner, literal);
            g_variant_builder_add(&builder, "v", converted);
        }
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    return g_variant_ref(value);
}

static gboolean override_number(GVariant* value, double* number) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = (double)g_variant_get_int64(value);
    else
        return FALSE;
    return isfinite(*number);
}

/* NULL when `table` has only the keys in `allowed`; otherwise the first other key. */
static const char* override_unknown_key(GVariant* table, const char* const* allowed) {
    GVariantIter iter;
    const char* key = NULL;
    g_variant_iter_init(&iter, table);
    while (g_variant_iter_next(&iter, "{&sv}", &key, NULL))
        if (!g_strv_contains(allowed, key))
            return key;
    return NULL;
}

/* An error message for an effect value the rule engine would not accept, or NULL. `value` has
 * kebab-case keys. The caller frees the message. */
static char* override_effect_error(const char* name, GVariant* value) {
    static const char* const corner_keys[] = {"radius",     "smoothing",       "mode",
                                              "padding",    "keep-maximized",  "keep-fullscreen",
                                              "keep-tiled", "skip-libadwaita", "skip-libhandy",
                                              "remove-csd", "border-width",    "border-color",
                                              "shadow",     "keep-shadow",     "shadow-animation",
                                              NULL};
    static const char* const frame_keys[] = {
        "mode",          "extents",    "crop",       "renderer",
        "style",         "background", "foreground", "inactive-background",
        "button-layout", NULL};
    double number = 0;
    if (g_str_equal(name, "opacity")) {
        if (!override_number(value, &number) || number < 0 || number > 1)
            return g_strdup("opacity takes a number from 0 to 1");
    } else if (g_str_equal(name, "blur")) {
        if (!override_number(value, &number) || number < 0 || number > 100)
            return g_strdup("blur takes a number from 0 to 100");
    } else if (g_str_equal(name, "blur_ignore_shadows")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
            return g_strdup("blur_ignore_shadows takes a boolean");
    } else if (g_str_equal(name, "shader")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
            return g_strdup("shader takes a GLSL file path, or \"\" to clear it");
    } else if (g_str_equal(name, "shader_uniforms")) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) ||
            g_variant_n_children(value) > 64)
            return g_strdup("shader_uniforms takes a table of at most 64 numbers");
        GVariantIter iter;
        const char* key = NULL;
        GVariant* child = NULL;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            g_autoptr(GVariant) owned = child;
            if (!override_number(owned, &number))
                return g_strdup_printf("shader_uniforms.%s must be a finite number", key);
        }
    } else if (g_str_equal(name, "frame") || g_str_equal(name, "corners")) {
        gboolean corners = g_str_equal(name, "corners");
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
            return g_strdup_printf("%s takes a table", name);
        const char* unknown = override_unknown_key(value, corners ? corner_keys : frame_keys);
        if (unknown) {
            g_autofree char* public_name = g_strdelimit(g_strdup(unknown), "-", '_');
            return g_strdup_printf("%s has no field %s; see the window rule fields", name,
                                   public_name);
        }
        g_autoptr(GVariant) radius = g_variant_lookup_value(value, "radius", NULL);
        if (corners && radius && (!override_number(radius, &number) || number < 0 || number > 200))
            return g_strdup("corners.radius takes a number from 0 to 200");
    }
    return NULL;
}

/* Make the Override object for `operation` (the Operation at that stack index) and push it. The
 * object exists at once; when the compositor refuses the operation it ends with the reason. */
static int push_override_handle(lua_State* state, LuaConfig* config, int operation,
                                const char* scope, const char* name, const char* window_id,
                                int value) {
    operation = lua_absindex(state, operation);
    value = lua_absindex(state, value);
    lua_createtable(state, 0, 5);
    int record = lua_gettop(state);
    lua_getfield(state, operation, "id");
    lua_pushfstring(state, "override-%I", (lua_Integer)lua_tointeger(state, -1));
    lua_setfield(state, record, "id");
    lua_pop(state, 1);
    lua_pushstring(state, scope);
    lua_setfield(state, record, "scope");
    lua_pushstring(state, name);
    lua_setfield(state, record, "name");
    if (window_id) {
        lua_pushstring(state, window_id);
        lua_setfield(state, record, "window");
    }
    lua_pushvalue(state, value);
    lua_setfield(state, record, "value");
    push_readonly_copy(state, record);
    lua_object_hold(state, "Override", -1);
    lua_object_end_after(state, config, operation, "Override", -1, FALSE);
    return 1;
}

/* A value for the document: the Lua value at `index` as a variant with kebab-case keys (none
 * when `literal`, for the uniform names), or NULL and a message. */
static GVariant* override_value(lua_State* state, int index, gboolean literal, char** message) {
    g_autoptr(GError) error = NULL;
    GVariant* converted = variant_from_lua(state, index, 0, FALSE, 0, &error);
    if (!converted) {
        *message = g_strdup(error ? error->message : "the value cannot be sent to the compositor");
        return NULL;
    }
    g_autoptr(GVariant) owned = g_variant_ref_sink(converted);
    return override_kebab_value(owned, literal);
}

/* Check a setting path and value without raising a Lua error, so every allocation is freed
 * before the caller raises one. On success the document's section and key and the value are
 * set; otherwise the result is the error message. */
static char* settings_override_check(lua_State* state, LuaConfig* config, const char* path,
                                     int value_index, char** section_out, char** key_out,
                                     GVariant** value_out) {
    g_auto(GStrv) parts = g_strsplit(path, ".", -1);
    if (g_strv_length(parts) != 2 || !*parts[0] || !*parts[1])
        return g_strdup_printf("gnoblin.settings.override takes a path such as "
                               "window_management.focus_mode, not \"%s\"",
                               path);
    g_autofree char* section = gnoblin_override_document_name(parts[0]);
    g_autofree char* key = gnoblin_override_document_name(parts[1]);
    g_autofree char* refusal = gnoblin_override_setting_refusal(section, key);
    if (refusal)
        return g_strdup_printf("%s cannot change at runtime: %s. The settings that can are %s",
                               path, refusal, GNOBLIN_OVERRIDE_SETTING_LIST);
    g_autofree char* message = NULL;
    g_autoptr(GVariant) value = override_value(state, value_index, FALSE, &message);
    if (!value)
        return g_strdup_printf("%s: %s", path, message);
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
        !g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
        !g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) &&
        !g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        return g_strdup_printf("%s takes a boolean, a number or a string", path);
    gboolean orientation = g_str_equal(section, "input");
    gboolean privacy = g_str_equal(section, "monitors");
    if ((orientation || privacy) && !g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        return g_strdup_printf("%s takes true or false; revoke the override to follow the "
                               "configuration again",
                               path);
    if (privacy && !any_output_has_privacy_screen(config))
        return g_strdup_printf("%s: no output has a privacy screen", path);
    /* The configuration's own validator checks the value against the committed document. */
    if (!orientation && !privacy) {
        if (!config->settings_document)
            return g_strdup_printf("%s: the configuration is not active yet", path);
        GVariantDict document;
        g_variant_dict_init(&document, config->settings_document);
        g_autoptr(GVariant) current =
            g_variant_dict_lookup_value(&document, section, G_VARIANT_TYPE_VARDICT);
        GVariantDict layer;
        g_variant_dict_init(&layer, current);
        g_variant_dict_insert_value(&layer, key, value);
        g_variant_dict_insert_value(&document, section, g_variant_dict_end(&layer));
        g_autoptr(GVariant) layered = g_variant_ref_sink(g_variant_dict_end(&document));
        g_autoptr(GError) invalid = NULL;
        if (!gnoblin_config_validate_document(layered, &invalid))
            return g_strdup_printf("%s: %s", path, invalid->message);
    }
    *section_out = g_steal_pointer(&section);
    *key_out = g_steal_pointer(&key);
    *value_out = g_steal_pointer(&value);
    return NULL;
}

/* gnoblin.settings.override(path, value). `path` is "section.key" in the public snake_case
 * names, such as "window_management.focus_mode". */
int lua_settings_override(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 2 || lua_type(state, 1) != LUA_TSTRING || lua_isnil(state, 2))
        return luaL_error(state, "gnoblin.settings.override takes a setting path and a value");
    lua_check_operation_context(state, config);
    char* section = NULL;
    char* key = NULL;
    GVariant* value = NULL;
    char* error =
        settings_override_check(state, config, lua_tostring(state, 1), 2, &section, &key, &value);
    if (error) {
        lua_pushstring(state, error);
        g_free(error);
        return lua_error(state);
    }
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "section", g_variant_new_string(section));
    g_variant_builder_add(&arguments, "{sv}", "key", g_variant_new_string(key));
    g_variant_builder_add(&arguments, "{sv}", "value", value);
    g_free(section);
    g_free(key);
    g_variant_unref(value);
    lua_queue_operation(state, config, "settings.override", g_variant_builder_end(&arguments));
    int operation = lua_gettop(state);
    return push_override_handle(state, config, operation, "settings", lua_tostring(state, 1), NULL,
                                2);
}

/* An error message for the effect name and value at these stack indices, or NULL with the
 * value to send. */
static char* window_effect_check(lua_State* state, int name_index, int value_index,
                                 GVariant** value_out) {
    if (lua_type(state, name_index) != LUA_TSTRING || lua_isnil(state, value_index))
        return g_strdup("window:set_effect takes an effect name and a value");
    const char* name = lua_tostring(state, name_index);
    if (!gnoblin_override_effect_known(name))
        return g_strdup_printf("\"%s\" is not a window effect; the effects are %s", name,
                               GNOBLIN_OVERRIDE_EFFECT_LIST);
    g_autofree char* message = NULL;
    g_autoptr(GVariant) value =
        override_value(state, value_index, g_str_equal(name, "shader_uniforms"), &message);
    if (!value)
        return g_strdup_printf("window:set_effect %s: %s", name, message);
    char* invalid = override_effect_error(name, value);
    if (invalid) {
        char* text = g_strdup_printf("window:set_effect: %s", invalid);
        g_free(invalid);
        return text;
    }
    *value_out = g_steal_pointer(&value);
    return NULL;
}

/* Window:set_effect(name, value), called by the Window record method with the window's id. The
 * effect is one the window rules support; it layers over every matching rule until the Override
 * ends. */
int lua_window_set_effect(lua_State* state, LuaConfig* config, const char* window_id,
                          int name_index, int value_index) {
    lua_check_operation_context(state, config);
    GVariant* value = NULL;
    char* error = window_effect_check(state, name_index, value_index, &value);
    if (error) {
        lua_pushstring(state, error);
        g_free(error);
        return lua_error(state);
    }
    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "id", g_variant_new_string(window_id));
    g_variant_builder_add(&arguments, "{sv}", "name",
                          g_variant_new_string(lua_tostring(state, name_index)));
    g_variant_builder_add(&arguments, "{sv}", "value", value);
    g_variant_unref(value);
    lua_queue_operation(state, config, "window.set_effect", g_variant_builder_end(&arguments));
    int operation = lua_gettop(state);
    return push_override_handle(state, config, operation, "window", lua_tostring(state, name_index),
                                window_id, value_index);
}
