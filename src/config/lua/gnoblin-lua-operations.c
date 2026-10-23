#include "gnoblin-lua-internal.h"

static GVariant* workspace_selector(lua_State* state, int index, gboolean allow_name);

gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed) {
    index = lua_absindex(state, index);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        gboolean found = FALSE;
        if (lua_type(state, -2) == LUA_TSTRING) {
            const char* key = lua_tostring(state, -2);
            for (guint i = 0; allowed[i]; i++)
                found |= g_str_equal(key, allowed[i]);
        }
        lua_pop(state, 1);
        if (!found)
            return FALSE;
    }
    return TRUE;
}

static gboolean window_operation_valid_args(lua_State* state, const char* method) {
    static const char* const id_only[] = {"id", NULL};
    static const char* const enabled[] = {"id", "enabled", NULL};
    static const char* const move[] = {"id", "x", "y", NULL};
    static const char* const resize[] = {"id", "width", "height", NULL};
    static const char* const workspace[] = {"id", "workspace", "follow", NULL};
    static const char* const monitor[] = {"id", "monitor", NULL};
    static const char* const thumbnail[] = {"id", "width", "height", NULL};
    static const char* const monitor_selector[] = {"id", NULL};
    luaL_checktype(state, 1, LUA_TTABLE);
    const char* const* fields = id_only;
    if (g_str_equal(method, "window.set_maximized") ||
        g_str_equal(method, "window.set_fullscreen") || g_str_equal(method, "window.set_above") ||
        g_str_equal(method, "window.set_sticky"))
        fields = enabled;
    else if (g_str_equal(method, "window.move"))
        fields = move;
    else if (g_str_equal(method, "window.resize"))
        fields = resize;
    else if (g_str_equal(method, "window.move_to_workspace"))
        fields = workspace;
    else if (g_str_equal(method, "window.move_to_monitor"))
        fields = monitor;
    else if (g_str_equal(method, "window.thumbnail"))
        fields = thumbnail;
    if (!table_fields_allowed(state, 1, fields))
        luaL_error(state, "%s arguments contain an unsupported field", method);

    lua_getfield(state, 1, "id");
    if (lua_type(state, -1) != LUA_TSTRING)
        luaL_error(state, "%s requires a string window id", method);
    const char* id = luaL_checkstring(state, -1);
    if (!*id || !g_utf8_validate(id, -1, NULL))
        luaL_error(state, "%s requires a nonempty window id", method);
    lua_pop(state, 1);

    if (fields == enabled) {
        lua_getfield(state, 1, "enabled");
        if (!lua_isboolean(state, -1))
            luaL_error(state, "%s requires boolean enabled", method);
        lua_pop(state, 1);
    } else if (fields == move || fields == resize) {
        const char* first = fields == move ? "x" : "width";
        const char* second = fields == move ? "y" : "height";
        gint64 low = fields == move ? -100000 : 1;
        gint64 high = fields == move ? 100000 : 32768;
        for (guint i = 0; i < 2; i++) {
            lua_getfield(state, 1, i == 0 ? first : second);
            if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < low ||
                lua_tointeger(state, -1) > high)
                luaL_error(state, "%s %s is outside the supported integer range", method,
                           i == 0 ? first : second);
            lua_pop(state, 1);
        }
    } else if (fields == workspace) {
        lua_getfield(state, 1, "workspace");
        GVariant* selector = workspace_selector(state, -1, FALSE);
        g_variant_unref(selector);
        lua_pop(state, 1);
        lua_getfield(state, 1, "follow");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "window.move_to_workspace follow must be a boolean");
        lua_pop(state, 1);
    } else if (fields == monitor) {
        lua_getfield(state, 1, "monitor");
        if (lua_type(state, -1) == LUA_TSTRING) {
            const char* id = lua_tostring(state, -1);
            if (!*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
                luaL_error(state, "window.move_to_monitor requires a valid monitor ID");
        } else if (lua_istable(state, -1)) {
            if (!table_fields_allowed(state, -1, monitor_selector))
                luaL_error(state, "monitor selector accepts only id");
            lua_getfield(state, -1, "id");
            if (lua_type(state, -1) != LUA_TSTRING) {
                luaL_error(state, "monitor selector id must be a string");
            } else {
                const char* id = lua_tostring(state, -1);
                if (!*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
                    luaL_error(state, "window.move_to_monitor requires a valid monitor ID");
            }
            lua_pop(state, 1);
        } else {
            luaL_error(state, "window.move_to_monitor requires a monitor ID or {id = string}");
        }
        lua_pop(state, 1);
    } else if (fields == thumbnail) {
        const char* dimensions[] = {"width", "height"};
        const lua_Integer limits[] = {480, 320};
        for (guint i = 0; i < G_N_ELEMENTS(dimensions); i++) {
            lua_getfield(state, 1, dimensions[i]);
            if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                lua_tointeger(state, -1) > limits[i])
                luaL_error(state, "window.thumbnail %s must be an integer from 1 to %d",
                           dimensions[i], (int)limits[i]);
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

static void input_source_selector_valid_args(lua_State* state) {
    static const char* const fields[] = {"type", "id", NULL};
    luaL_checktype(state, 1, LUA_TTABLE);
    if (!table_fields_allowed(state, 1, fields))
        luaL_error(state, "input source selector accepts only type and id");
    lua_getfield(state, 1, "type");
    if (lua_type(state, -1) != LUA_TSTRING)
        luaL_error(state, "input source selector requires a string type");
    const char* type = lua_tostring(state, -1);
    if (!g_str_equal(type, "xkb") && !g_str_equal(type, "ibus"))
        luaL_error(state, "input source selector type must be 'xkb' or 'ibus'");
    lua_pop(state, 1);
    lua_getfield(state, 1, "id");
    if (lua_type(state, -1) != LUA_TSTRING)
        luaL_error(state, "input source selector requires a string id");
    const char* id = lua_tostring(state, -1);
    if (!*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
        luaL_error(state, "input source selector requires a valid id");
    lua_pop(state, 1);
}

static GVariant* workspace_selector(lua_State* state, int index, gboolean allow_name) {
    luaL_checktype(state, index, LUA_TTABLE);
    index = lua_absindex(state, index);
    static const char* const fields[] = {"id", "number", NULL};
    static const char* const rename_fields[] = {"id", "number", "name", NULL};
    if (!table_fields_allowed(state, index, allow_name ? rename_fields : fields))
        luaL_error(state, "workspace selector accepts only id or number");
    GVariantBuilder selector;
    g_variant_builder_init(&selector, G_VARIANT_TYPE_VARDICT);
    lua_getfield(state, index, "id");
    gboolean has_id = !lua_isnil(state, -1);
    if (has_id) {
        if (lua_type(state, -1) != LUA_TSTRING)
            luaL_error(state, "workspace selector id must be a string");
        const char* id = luaL_checkstring(state, -1);
        if (!workspace_selector_id_valid(id))
            luaL_error(state, "workspace id must start with a letter or number and contain only "
                              "letters, numbers, '.', '_' or '-' (up to 64 characters)");
        g_variant_builder_add(&selector, "{sv}", "id", g_variant_new_string(id));
    }
    lua_pop(state, 1);
    lua_getfield(state, index, "number");
    gboolean has_number = !lua_isnil(state, -1);
    if (has_number) {
        if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
            lua_tointeger(state, -1) > 1024)
            luaL_error(state, "workspace number must be an integer from 1 to 1024");
        g_variant_builder_add(&selector, "{sv}", "number",
                              g_variant_new_int64(lua_tointeger(state, -1)));
    }
    lua_pop(state, 1);
    if (has_id == has_number)
        luaL_error(state, "workspace selector must contain exactly one of id or number");
    return g_variant_builder_end(&selector);
}

static void add_workspace_field(lua_State* state, GVariantBuilder* arguments, int table,
                                const char* field) {
    lua_getfield(state, table, field);
    GVariant* selector = workspace_selector(state, -1, FALSE);
    g_variant_builder_add(arguments, "{sv}", "workspace", selector);
    lua_pop(state, 1);
}

static void add_selector_arguments(lua_State* state, GVariantBuilder* arguments, int table,
                                   gboolean allow_name) {
    g_autoptr(GVariant) selector = workspace_selector(state, table, allow_name);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, selector);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_variant_builder_add(arguments, "{sv}", key, value);
        g_variant_unref(value);
    }
}

int lua_workspace_action(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (config && config->rejecting_operations)
        return luaL_error(state,
                          "runtime operations are unavailable while rejecting a failed event");
    if (!config || !config->runtime_actions || (!config->dispatching && !config->api_calling))
        return luaL_error(state,
                          "workspace actions are available only while handling a runtime event");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "too many pending workspace actions");

    GVariantBuilder arguments;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    if (g_str_equal(method, "workspace.next") || g_str_equal(method, "workspace.previous")) {
        if (lua_gettop(state) != 0)
            return luaL_error(state, "%s takes no arguments", method);
    } else if (g_str_equal(method, "workspace.create")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"id", "name", "activate", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.create accepts only id, name, and activate");
        lua_getfield(state, 1, "id");
        if (!lua_isnil(state, -1)) {
            const char* id = luaL_checkstring(state, -1);
            if (!workspace_id_valid(id))
                return luaL_error(state,
                                  "workspace id must start with a letter or number and contain "
                                  "only letters, numbers, '.', '_' or '-' (up to 64 characters)");
            g_variant_builder_add(&arguments, "{sv}", "id", g_variant_new_string(id));
        }
        lua_pop(state, 1);
        lua_getfield(state, 1, "name");
        const char* name = luaL_checkstring(state, -1);
        if (!*name || !g_utf8_validate(name, -1, NULL) || g_utf8_strlen(name, -1) > 80)
            return luaL_error(state, "workspace name must contain 1 to 80 characters");
        g_variant_builder_add(&arguments, "{sv}", "name", g_variant_new_string(name));
        lua_pop(state, 1);
        lua_getfield(state, 1, "activate");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace activate must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "activate",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.rename")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"id", "number", "name", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.rename accepts only id or number and name");
        add_selector_arguments(state, &arguments, 1, TRUE);
        lua_getfield(state, 1, "name");
        const char* name = luaL_checkstring(state, -1);
        if (!*name || !g_utf8_validate(name, -1, NULL) || g_utf8_strlen(name, -1) > 80)
            return luaL_error(state, "workspace name must contain 1 to 80 characters");
        g_variant_builder_add(&arguments, "{sv}", "name", g_variant_new_string(name));
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.remove") || g_str_equal(method, "workspace.switch")) {
        add_selector_arguments(state, &arguments, 1, FALSE);
    } else if (g_str_equal(method, "workspace.move_active")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"workspace", "follow", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.move_active accepts only workspace and follow");
        add_workspace_field(state, &arguments, 1, "workspace");
        lua_getfield(state, 1, "follow");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace move follow must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "follow",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else if (g_str_equal(method, "workspace.move_window")) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"window", "workspace", "follow", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state,
                              "workspace.move_window accepts only window, workspace, and follow");
        lua_getfield(state, 1, "window");
        const char* window = luaL_checkstring(state, -1);
        if (g_str_equal(window, "active")) {
            g_variant_builder_add(&arguments, "{sv}", "window", g_variant_new_string("active"));
        } else if (*window && g_utf8_validate(window, -1, NULL)) {
            g_variant_builder_add(&arguments, "{sv}", "window", g_variant_new_string(window));
        } else {
            return luaL_error(state, "window must be 'active' or a nonempty window ID");
        }
        lua_pop(state, 1);
        add_workspace_field(state, &arguments, 1, "workspace");
        lua_getfield(state, 1, "follow");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "workspace move follow must be a boolean");
            g_variant_builder_add(&arguments, "{sv}", "follow",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
    } else {
        g_variant_builder_clear(&arguments);
        return luaL_error(state, "unknown workspace action");
    }

    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder action;
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    if (config->api_calling)
        lua_pushinteger(state, request_id); /* legacy socket/C caller contract */
    else
        push_operation_handle(state, config, request_id, method);
    return 1;
}

/* Validates the dense argv array at index and returns it as a floating string-array variant.
 * label names the Lua call in error messages. */
static GVariant* lua_command_argv_variant(lua_State* state, int index, const char* label) {
    index = lua_absindex(state, index);
    lua_Integer count = lua_istable(state, index) ? (lua_Integer)lua_rawlen(state, index) : 0;
    if (count < 1 || !lua_dense_array(state, index, 64)) {
        luaL_error(state, "%s argv must contain 1 to 64 strings", label);
        return NULL;
    }
    gsize bytes = 0;
    GVariantBuilder argv;
    g_variant_builder_init(&argv, G_VARIANT_TYPE_STRING_ARRAY);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, index, i);
        size_t length = 0;
        const char* argument =
            lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
        gboolean valid = argument && memchr(argument, '\0', length) == NULL &&
                         g_utf8_validate(argument, length, NULL);
        gboolean empty_program = i == 1 && length == 0;
        if (valid && !empty_program && (bytes += length + 1) <= 64 * 1024)
            g_variant_builder_add(&argv, "s", argument);
        lua_pop(state, 1);
        if (!valid || empty_program) {
            g_variant_builder_clear(&argv);
            luaL_error(state, "%s argv[1] must be nonempty and every entry UTF-8", label);
            return NULL;
        }
        if (bytes > 64 * 1024) {
            g_variant_builder_clear(&argv);
            luaL_error(state, "%s argv is too large", label);
            return NULL;
        }
    }
    return g_variant_builder_end(&argv);
}

int lua_generic_api_action(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    if (g_str_equal(method, "window.begin_move") || g_str_equal(method, "window.begin_resize"))
        return luaL_error(state, "%s requires a Window record and trusted FocusContext", method);
    if (config && config->rejecting_operations)
        return luaL_error(state,
                          "runtime operations are unavailable while rejecting a failed event");
    if (!config || !config->runtime_actions || (!config->dispatching && !config->api_calling))
        return luaL_error(state, "Gnoblin session actions are available only at runtime");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "too many pending Gnoblin session actions");

    if (g_str_equal(method, "command.run") || g_str_equal(method, "command.capture")) {
        const gboolean capture = g_str_equal(method, "command.capture");
        GVariantBuilder arguments;
        g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
        if (!capture) {
            if (lua_gettop(state) != 1 || !lua_istable(state, 1))
                return luaL_error(state, "commands.run requires one argv array");
            g_variant_builder_add(&arguments, "{sv}", "argv",
                                  lua_command_argv_variant(state, 1, "commands.run"));
        } else {
            static const char* const fields[] = {"argv", "stdin", "timeout", NULL};
            if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
                !table_fields_allowed(state, 1, fields))
                return luaL_error(state, "commands.capture accepts a table with argv, stdin, "
                                         "and timeout");
            lua_getfield(state, 1, "argv");
            g_variant_builder_add(&arguments, "{sv}", "argv",
                                  lua_command_argv_variant(state, -1, "commands.capture"));
            lua_pop(state, 1);
            lua_getfield(state, 1, "stdin");
            if (!lua_isnil(state, -1)) {
                size_t length = 0;
                const char* text =
                    lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
                if (!text || length > 4096 || memchr(text, '\0', length) ||
                    !g_utf8_validate(text, length, NULL))
                    return luaL_error(state, "commands.capture stdin must be UTF-8 text up to "
                                             "4096 bytes");
                g_variant_builder_add(&arguments, "{sv}", "stdin", g_variant_new_string(text));
            }
            lua_pop(state, 1);
            lua_getfield(state, 1, "timeout");
            if (!lua_isnil(state, -1)) {
                if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                    lua_tointeger(state, -1) > 600)
                    return luaL_error(state, "commands.capture timeout must be an integer from "
                                             "1 to 600 seconds");
                g_variant_builder_add(&arguments, "{sv}", "timeout",
                                      g_variant_new_int64(lua_tointeger(state, -1)));
            }
            lua_pop(state, 1);
        }
        guint64 request_id = allocate_operation_id();
        if (!request_id)
            return luaL_error(state, "Gnoblin operation request ID space is exhausted");
        GVariantBuilder action;
        g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
        g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
        g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
        g_ptr_array_add(config->runtime_actions,
                        g_variant_ref_sink(g_variant_builder_end(&action)));
        config->actions_in_dispatch++;
        push_operation_handle(state, config, request_id, method);
        return 1;
    }
    if ((g_str_equal(method, "privacy.stop_sharing") ||
         g_str_equal(method, "privacy.stop_recording")) &&
        lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", method);
    if (g_str_equal(method, "session.lock") && lua_gettop(state) != 0)
        return luaL_error(state, "session.lock takes no arguments");
    if (g_str_equal(method, "session.logout") && lua_gettop(state) != 0)
        return luaL_error(state, "session.logout takes no arguments");
    if ((g_str_equal(method, "session.suspend") || g_str_equal(method, "session.hibernate") ||
         g_str_equal(method, "session.reboot") || g_str_equal(method, "session.poweroff")) &&
        lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", method);
    if (g_str_equal(method, "console.toggle") && lua_gettop(state) != 0)
        return luaL_error(state, "console.toggle takes no arguments");
    if (g_str_equal(method, "location.authorize_app")) {
        static const char* const fields[] = {"request_id", "allow", "accuracy", NULL};
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state,
                              "location.authorize_app requires request_id, allow, and accuracy");
        lua_getfield(state, 1, "request_id");
        gboolean valid_request_id = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0;
        lua_pop(state, 1);
        lua_getfield(state, 1, "allow");
        gboolean valid_allow = lua_isboolean(state, -1);
        gboolean allowed = valid_allow && lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_getfield(state, 1, "accuracy");
        lua_Integer accuracy = lua_isinteger(state, -1) ? lua_tointeger(state, -1) : -1;
        gboolean valid_accuracy = accuracy == 0 || accuracy == 1 || accuracy == 4 ||
                                  accuracy == 5 || accuracy == 6 || accuracy == 8;
        lua_pop(state, 1);
        if (!valid_request_id || !valid_allow || !valid_accuracy || (allowed && accuracy == 0) ||
            (!allowed && accuracy != 0))
            return luaL_error(state,
                              "location.authorize_app needs a positive request_id, boolean allow, "
                              "and accuracy 0, 1, 4, 5, 6, or 8 (0 when denied)");
    }

    if (g_str_has_prefix(method, "auth.")) {
        static const char* const begin_fields[] = {"request_id", "identity", NULL};
        static const char* const respond_fields[] = {"request_id", "response", NULL};
        static const char* const cancel_fields[] = {"request_id", NULL};
        const char* const* fields = g_str_equal(method, "auth.begin")     ? begin_fields
                                    : g_str_equal(method, "auth.respond") ? respond_fields
                                                                          : cancel_fields;
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state, "%s accepts only a table with %s", method,
                              g_str_equal(method, "auth.begin")     ? "request_id and identity"
                              : g_str_equal(method, "auth.respond") ? "request_id and response"
                                                                    : "request_id");
        lua_getfield(state, 1, "request_id");
        gboolean valid_request_id = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0;
        lua_pop(state, 1);
        if (!valid_request_id)
            return luaL_error(state, "%s needs a positive integer request_id", method);
        if (g_str_equal(method, "auth.begin")) {
            lua_getfield(state, 1, "identity");
            gboolean valid_identity =
                lua_isnil(state, -1) || (lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0);
            lua_pop(state, 1);
            if (!valid_identity)
                return luaL_error(state, "auth.begin identity must be a positive integer");
        } else if (g_str_equal(method, "auth.respond")) {
            lua_getfield(state, 1, "response");
            size_t length = 0;
            const char* text =
                lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
            gboolean valid_response = text && length <= 4096 && g_utf8_validate(text, length, NULL);
            lua_pop(state, 1);
            if (!valid_response)
                return luaL_error(state,
                                  "auth.respond needs a UTF-8 response string up to 4096 bytes");
        }
    }

    if (g_str_has_prefix(method, "prompt.")) {
        static const char* const respond_fields[] = {"request_id", "password", "confirmed",
                                                     "choice", NULL};
        static const char* const cancel_fields[] = {"request_id", NULL};
        gboolean respond = g_str_equal(method, "prompt.respond");
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, respond ? respond_fields : cancel_fields))
            return luaL_error(state, "%s accepts only a table with %s", method,
                              respond ? "request_id, password, confirmed, and choice"
                                      : "request_id");
        lua_getfield(state, 1, "request_id");
        gboolean valid_request_id = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0;
        lua_pop(state, 1);
        if (!valid_request_id)
            return luaL_error(state, "%s needs a positive integer request_id", method);
        if (respond) {
            lua_getfield(state, 1, "password");
            size_t length = 0;
            const char* text =
                lua_type(state, -1) == LUA_TSTRING ? lua_tolstring(state, -1, &length) : NULL;
            gboolean valid_password = lua_isnil(state, -1) || (text && length <= 4096 &&
                                                               g_utf8_validate(text, length, NULL));
            lua_pop(state, 1);
            if (!valid_password)
                return luaL_error(
                    state, "prompt.respond password must be a UTF-8 string up to 4096 bytes");
            lua_getfield(state, 1, "confirmed");
            gboolean valid_confirmed = lua_isnil(state, -1) || lua_isboolean(state, -1);
            lua_pop(state, 1);
            lua_getfield(state, 1, "choice");
            gboolean valid_choice = lua_isnil(state, -1) || lua_isboolean(state, -1);
            lua_pop(state, 1);
            if (!valid_confirmed || !valid_choice)
                return luaL_error(state, "prompt.respond confirmed and choice must be booleans");
        }
    }

    if (g_str_equal(method, "input.set_orientation_lock") && config->api_calling) {
        static const char* const fields[] = {"value", NULL};
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state, "input.set_orientation_lock accepts only value");
        lua_getfield(state, 1, "value");
        lua_replace(state, 1);
        lua_settop(state, 1);
    }

    GVariant* arguments = NULL;
    if (g_str_equal(method, "input.set_orientation_lock")) {
        if (lua_gettop(state) != 1 ||
            (!lua_isboolean(state, 1) && !(lua_type(state, 1) == LUA_TSTRING &&
                                           g_str_equal(lua_tostring(state, 1), "inherit"))))
            return luaL_error(state,
                              "input.set_orientation_lock requires true, false, or 'inherit'");
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        if (lua_isboolean(state, 1))
            g_variant_builder_add(&builder, "{sv}", "value",
                                  g_variant_new_boolean(lua_toboolean(state, 1)));
        else
            g_variant_builder_add(&builder, "{sv}", "value", g_variant_new_string("inherit"));
        arguments = g_variant_builder_end(&builder);
    } else if (lua_gettop(state) == 0) {
        if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind") ||
            g_str_equal(method, "shortcut.session.end"))
            return luaL_error(state, "%s requires an argument table", method);
        if (g_str_has_prefix(method, "window.") && !g_str_equal(method, "window.match"))
            return luaL_error(state, "%s requires an argument table", method);
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        if (g_str_has_prefix(method, "window.") && !g_str_equal(method, "window.match"))
            window_operation_valid_args(state, method);
        if (g_str_equal(method, "input.select"))
            input_source_selector_valid_args(state);
        if (g_str_equal(method, "shortcut.capture")) {
            static const char* const fields[] = {"timeout", NULL};
            if (!table_fields_allowed(state, 1, fields))
                return luaL_error(state, "shortcut.capture accepts only timeout");
            lua_getfield(state, 1, "timeout");
            if (!lua_isnil(state, -1) &&
                (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                 lua_tointeger(state, -1) > 60)) {
                lua_pop(state, 1);
                return luaL_error(state,
                                  "shortcut capture timeout must be an integer from 1 to 60");
            }
            lua_pop(state, 1);
        }
        if (g_str_equal(method, "shortcut.bind")) {
            static const char* const fields[] = {"id",   "accelerator",   "hold", "trigger",
                                                 "mode", "capture_input", NULL};
            if (!table_fields_allowed(state, 1, fields))
                return luaL_error(state,
                                  "shortcut.bind accepts id, accelerator, hold, trigger, mode, "
                                  "and capture_input");
            lua_getfield(state, 1, "id");
            gboolean valid_id =
                lua_type(state, -1) == LUA_TSTRING && strlen(lua_tostring(state, -1)) <= 64 &&
                g_regex_match_simple("^[A-Za-z0-9_-]+$", lua_tostring(state, -1), 0, 0);
            lua_pop(state, 1);
            lua_getfield(state, 1, "accelerator");
            gboolean valid_accelerator = lua_type(state, -1) == LUA_TSTRING &&
                                         lua_tostring(state, -1)[0] != '\0' &&
                                         strlen(lua_tostring(state, -1)) <= 128 &&
                                         g_utf8_validate(lua_tostring(state, -1), -1, NULL);
            lua_pop(state, 1);
            if (!valid_id || !valid_accelerator)
                return luaL_error(state, "shortcut.bind requires id and accelerator strings");
            lua_getfield(state, 1, "hold");
            gboolean valid_hold =
                lua_isnil(state, -1) || (lua_type(state, -1) == LUA_TSTRING &&
                                         (g_str_equal(lua_tostring(state, -1), "none") ||
                                          g_str_equal(lua_tostring(state, -1), "super") ||
                                          g_str_equal(lua_tostring(state, -1), "control") ||
                                          g_str_equal(lua_tostring(state, -1), "alt")));
            lua_pop(state, 1);
            lua_getfield(state, 1, "trigger");
            gboolean valid_trigger =
                lua_isnil(state, -1) || (lua_type(state, -1) == LUA_TSTRING &&
                                         (g_str_equal(lua_tostring(state, -1), "press") ||
                                          g_str_equal(lua_tostring(state, -1), "release")));
            lua_pop(state, 1);
            lua_getfield(state, 1, "mode");
            gboolean valid_mode =
                lua_isnil(state, -1) || (lua_type(state, -1) == LUA_TSTRING &&
                                         (g_str_equal(lua_tostring(state, -1), "passive") ||
                                          g_str_equal(lua_tostring(state, -1), "modal")));
            lua_pop(state, 1);
            lua_getfield(state, 1, "capture_input");
            gboolean valid_capture = lua_isnil(state, -1) || lua_isboolean(state, -1);
            lua_pop(state, 1);
            if (!valid_hold || !valid_trigger || !valid_mode || !valid_capture)
                return luaL_error(state, "shortcut.bind has an invalid option value");
        }
        if (g_str_equal(method, "shortcut.unbind")) {
            static const char* const fields[] = {"id", NULL};
            if (!table_fields_allowed(state, 1, fields))
                return luaL_error(state, "shortcut.unbind accepts only id");
        }
        if (g_str_equal(method, "shortcut.session.end")) {
            static const char* const fields[] = {"id", "session_id", NULL};
            if (!table_fields_allowed(state, 1, fields))
                return luaL_error(state, "shortcut.session.end accepts only id and session_id");
            lua_getfield(state, 1, "id");
            gboolean valid_id =
                lua_type(state, -1) == LUA_TSTRING && strlen(lua_tostring(state, -1)) <= 64 &&
                g_regex_match_simple("^[A-Za-z0-9_-]+$", lua_tostring(state, -1), 0, 0);
            lua_pop(state, 1);
            lua_getfield(state, 1, "session_id");
            gboolean valid_session = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0;
            lua_pop(state, 1);
            if (!valid_id || !valid_session)
                return luaL_error(
                    state, "shortcut.session.end requires a valid id and positive session_id");
        }
        GError* error = NULL;
        arguments = variant_from_lua(state, 1, 0, FALSE, 0, &error);
        if (!arguments) {
            char* message = g_strdup(error ? error->message : "expected a table");
            g_clear_error(&error);
            lua_pushfstring(state, "%s arguments: %s", method, message);
            g_free(message);
            return lua_error(state);
        }
        if (!g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
            g_variant_unref(arguments);
            return luaL_error(state, "%s arguments must be a table", method);
        }
    } else if (lua_gettop(state) == 1 && lua_isnil(state, 1) &&
               g_str_equal(method, "shortcut.capture")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else {
        return luaL_error(state, "%s takes no arguments or one argument table", method);
    }

    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder action;
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", arguments);
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    if (config->api_calling)
        lua_pushinteger(state, request_id); /* legacy socket/C caller contract */
    else
        push_operation_handle(state, config, request_id, method);
    return 1;
}
