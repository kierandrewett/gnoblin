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

/* The table that a window operation takes: the id of the window and the fields that the record
 * methods of the operation declare (lua_api[]). The compositor checks the ranges of the visual
 * transforms; Lua checks the types. */
static gboolean window_operation_valid_args(lua_State* state,
                                            const GnoblinApiOperation* operation) {
    const char* method = operation->name;
    const char* fields[8];
    static const char* const monitor_selector[] = {"id", NULL};
    lua_api_operation_fields(operation, fields, G_N_ELEMENTS(fields));
    luaL_checktype(state, 1, LUA_TTABLE);
    if (!table_fields_allowed(state, 1, fields))
        luaL_error(state, "%s arguments contain an unsupported field", method);

    lua_getfield(state, 1, "id");
    if (lua_type(state, -1) != LUA_TSTRING)
        luaL_error(state, "%s requires a string window id", method);
    const char* id = luaL_checkstring(state, -1);
    if (!*id || !g_utf8_validate(id, -1, NULL))
        luaL_error(state, "%s requires a nonempty window id", method);
    lua_pop(state, 1);

    switch (operation->id) {
    case GNOBLIN_OP_WINDOW_SET_MAXIMIZED:
    case GNOBLIN_OP_WINDOW_SET_FULLSCREEN:
    case GNOBLIN_OP_WINDOW_SET_ABOVE:
    case GNOBLIN_OP_WINDOW_SET_STICKY:
        lua_getfield(state, 1, fields[1]);
        if (!lua_isboolean(state, -1))
            luaL_error(state, "%s requires boolean %s", method, fields[1]);
        lua_pop(state, 1);
        break;
    case GNOBLIN_OP_WINDOW_SET_VISUAL:
        /* The compositor checks the ranges; Lua checks the types. */
        for (guint i = 1; fields[i]; i++) {
            lua_getfield(state, 1, fields[i]);
            gboolean is_reset = g_str_equal(fields[i], "reset");
            if (!lua_isnil(state, -1) &&
                (is_reset ? !lua_isboolean(state, -1) : lua_type(state, -1) != LUA_TNUMBER))
                luaL_error(state, "%s %s must be %s", method, fields[i],
                           is_reset ? "a boolean" : "a number");
            lua_pop(state, 1);
        }
        break;
    case GNOBLIN_OP_WINDOW_MOVE:
    case GNOBLIN_OP_WINDOW_RESIZE: {
        gboolean move = operation->id == GNOBLIN_OP_WINDOW_MOVE;
        gint64 low = move ? -100000 : 1;
        gint64 high = move ? 100000 : 32768;
        for (guint i = 1; i <= 2; i++) {
            lua_getfield(state, 1, fields[i]);
            if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < low ||
                lua_tointeger(state, -1) > high)
                luaL_error(state, "%s %s is outside the supported integer range", method,
                           fields[i]);
            lua_pop(state, 1);
        }
        break;
    }
    case GNOBLIN_OP_WINDOW_MOVE_TO_WORKSPACE: {
        lua_getfield(state, 1, fields[1]);
        GVariant* selector = workspace_selector(state, -1, FALSE);
        g_variant_unref(selector);
        lua_pop(state, 1);
        lua_getfield(state, 1, fields[2]);
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "%s %s must be a boolean", method, fields[2]);
        lua_pop(state, 1);
        break;
    }
    case GNOBLIN_OP_WINDOW_MOVE_TO_MONITOR:
        lua_getfield(state, 1, fields[1]);
        if (lua_type(state, -1) == LUA_TSTRING) {
            const char* id = lua_tostring(state, -1);
            if (!*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
                luaL_error(state, "%s requires a valid monitor ID", method);
        } else if (lua_istable(state, -1)) {
            if (!table_fields_allowed(state, -1, monitor_selector))
                luaL_error(state, "monitor selector accepts only id");
            lua_getfield(state, -1, "id");
            if (lua_type(state, -1) != LUA_TSTRING) {
                luaL_error(state, "monitor selector id must be a string");
            } else {
                const char* id = lua_tostring(state, -1);
                if (!*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
                    luaL_error(state, "%s requires a valid monitor ID", method);
            }
            lua_pop(state, 1);
        } else {
            luaL_error(state, "%s requires a monitor ID or {id = string}", method);
        }
        lua_pop(state, 1);
        break;
    case GNOBLIN_OP_WINDOW_THUMBNAIL: {
        /* The limits are those of the record method's arguments (lua_api[]). */
        for (guint i = 1; i <= 2; i++) {
            lua_Integer limit = lua_api_operation_argument(operation, fields[i])->limit;
            lua_getfield(state, 1, fields[i]);
            if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                lua_tointeger(state, -1) > limit)
                luaL_error(state, "%s %s must be an integer from 1 to %d", method, fields[i],
                           (int)limit);
            lua_pop(state, 1);
        }
        break;
    }
    default:
        break;
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

/* Push the selector table for a Workspace, a workspace ID string or a workspace number. Returns
 * FALSE and pushes nothing for any other value. */
gboolean lua_push_workspace_selector(lua_State* state, int index) {
    index = lua_absindex(state, index);
    gboolean handle = lua_object_is(state, index, "Workspace");
    if (handle) {
        lua_getfield(state, index, "id");
        if (lua_type(state, -1) != LUA_TSTRING) {
            lua_pop(state, 1);
            luaL_error(state, "this Workspace has no ID");
        }
    } else if (lua_type(state, index) != LUA_TSTRING && !lua_isinteger(state, index)) {
        return FALSE;
    } else {
        lua_pushvalue(state, index);
    }
    lua_createtable(state, 0, 1);
    lua_insert(state, -2);
    lua_setfield(state, -2, lua_type(state, -1) == LUA_TSTRING ? "id" : "number");
    return TRUE;
}

/* Push the monitor ID that the value at `index` names: an Output (the logical monitor that shows
 * it) or a monitor ID string. Returns FALSE and pushes nothing for any other value. */
gboolean lua_push_monitor_id(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (lua_type(state, index) == LUA_TSTRING) {
        lua_pushvalue(state, index);
        return TRUE;
    }
    if (!lua_object_is(state, index, "Output"))
        return FALSE;
    lua_getfield(state, index, "monitor_id");
    if (lua_type(state, -1) != LUA_TSTRING)
        luaL_error(state, "this Output is not enabled, so it shows no monitor");
    return TRUE;
}

static GVariant* workspace_selector(lua_State* state, int index, gboolean allow_name) {
    index = lua_absindex(state, index);
    if (lua_push_workspace_selector(state, index)) {
        GVariant* selector = workspace_selector(state, -1, FALSE);
        lua_pop(state, 1);
        return selector;
    }
    luaL_checktype(state, index, LUA_TTABLE);
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

/* Add id or number for the workspace that `table` names: either its `workspace` field (an ID
 * string, a number or a selector table, as positional Lua calls pass it) or its own top-level id
 * or number fields. */
static void add_selector_arguments(lua_State* state, GVariantBuilder* arguments, int table,
                                   gboolean allow_name) {
    table = lua_absindex(state, table);
    luaL_checktype(state, table, LUA_TTABLE);
    lua_getfield(state, table, "workspace");
    gboolean nested = !lua_isnil(state, -1);
    if (nested) {
        static const char* const fields[] = {"workspace", NULL};
        static const char* const rename_fields[] = {"workspace", "name", NULL};
        if (!table_fields_allowed(state, table, allow_name ? rename_fields : fields))
            luaL_error(state, "workspace selector accepts only one workspace");
    }
    g_autoptr(GVariant) selector = nested ? workspace_selector(state, -1, FALSE)
                                          : workspace_selector(state, table, allow_name);
    lua_pop(state, 1);
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
    const GnoblinApiOperation* operation = gnoblin_api_operation_find(method);
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
    GnoblinApiOp workspace_id = operation ? operation->id : GNOBLIN_OP_COUNT;
    if (workspace_id == GNOBLIN_OP_WORKSPACE_NEXT ||
        workspace_id == GNOBLIN_OP_WORKSPACE_PREVIOUS) {
        if (lua_gettop(state) != 0)
            return luaL_error(state, "%s takes no arguments", method);
    } else if (workspace_id == GNOBLIN_OP_WORKSPACE_CREATE) {
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
    } else if (workspace_id == GNOBLIN_OP_WORKSPACE_RENAME) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"id", "number", "workspace", "name", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "workspace.rename accepts only a workspace and a name");
        add_selector_arguments(state, &arguments, 1, TRUE);
        lua_getfield(state, 1, "name");
        const char* name = luaL_checkstring(state, -1);
        if (!*name || !g_utf8_validate(name, -1, NULL) || g_utf8_strlen(name, -1) > 80)
            return luaL_error(state, "workspace name must contain 1 to 80 characters");
        g_variant_builder_add(&arguments, "{sv}", "name", g_variant_new_string(name));
        lua_pop(state, 1);
    } else if (workspace_id == GNOBLIN_OP_WORKSPACE_REMOVE ||
               workspace_id == GNOBLIN_OP_WORKSPACE_SWITCH) {
        add_selector_arguments(state, &arguments, 1, FALSE);
    } else if (workspace_id == GNOBLIN_OP_WORKSPACE_MOVE_ACTIVE) {
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
    } else if (workspace_id == GNOBLIN_OP_WORKSPACE_MOVE_WINDOW) {
        luaL_checktype(state, 1, LUA_TTABLE);
        static const char* const fields[] = {"window", "workspace", "follow", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state,
                              "workspace.move_window accepts only window, workspace, and follow");
        lua_getfield(state, 1, "window");
        if (lua_isuserdata(state, -1)) {
            lua_getfield(state, -1, "id");
            lua_remove(state, -2);
        }
        if (lua_type(state, -1) != LUA_TSTRING)
            return luaL_error(state, "window must be a Window, 'active' or a window ID");
        const char* window = lua_tostring(state, -1);
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
GVariant* lua_command_argv_variant(lua_State* state, int index, const char* label) {
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

/* Raise the Lua error that every operation shares when it cannot be queued now. */
void lua_check_operation_context(lua_State* state, LuaConfig* config) {
    if (config && config->rejecting_operations)
        luaL_error(state, "runtime operations are unavailable while rejecting a failed event");
    if (!config || !config->runtime_actions || (!config->dispatching && !config->api_calling))
        luaL_error(state, "Gnoblin session actions are available only at runtime");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        luaL_error(state, "too many pending Gnoblin session actions");
}

/* Queue `method` with `arguments` (an a{sv} variant, consumed) for the compositor and push its
 * Operation. */
int lua_queue_operation(lua_State* state, LuaConfig* config, const char* method,
                        GVariant* arguments) {
    guint64 request_id = allocate_operation_id();
    if (!request_id) {
        g_variant_unref(g_variant_ref_sink(arguments));
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    }
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

int lua_generic_api_action(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* method = lua_tostring(state, lua_upvalueindex(2));
    const GnoblinApiOperation* operation = gnoblin_api_operation_find(method);
    GnoblinApiOp id = operation ? operation->id : GNOBLIN_OP_COUNT;
    if (id == GNOBLIN_OP_WINDOW_BEGIN_MOVE || id == GNOBLIN_OP_WINDOW_BEGIN_RESIZE)
        return luaL_error(state, "%s requires a Window record and a Grant", method);
    lua_check_operation_context(state, config);

    if (operation && (operation->flags & GN_OP_NOTHING) && lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", method);
    gboolean window_call = gnoblin_api_operation_in_family(method, GNOBLIN_OP_WINDOW_FAMILY) &&
                           id != GNOBLIN_OP_WINDOW_MATCH;
    if (id == GNOBLIN_OP_LOCATION_AUTHORIZE_APP) {
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

    if (operation && operation->dispatch == GN_DISPATCH_AUTH) {
        static const char* const begin_fields[] = {"request_id", "identity", NULL};
        static const char* const respond_fields[] = {"request_id", "response", NULL};
        static const char* const cancel_fields[] = {"request_id", NULL};
        const char* const* fields = id == GNOBLIN_OP_AUTH_BEGIN     ? begin_fields
                                    : id == GNOBLIN_OP_AUTH_RESPOND ? respond_fields
                                                                    : cancel_fields;
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state, "%s accepts only a table with %s", method,
                              id == GNOBLIN_OP_AUTH_BEGIN     ? "request_id and identity"
                              : id == GNOBLIN_OP_AUTH_RESPOND ? "request_id and response"
                                                              : "request_id");
        lua_getfield(state, 1, "request_id");
        gboolean valid_request_id = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0;
        lua_pop(state, 1);
        if (!valid_request_id)
            return luaL_error(state, "%s needs a positive integer request_id", method);
        if (id == GNOBLIN_OP_AUTH_BEGIN) {
            lua_getfield(state, 1, "identity");
            gboolean valid_identity =
                lua_isnil(state, -1) || (lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0);
            lua_pop(state, 1);
            if (!valid_identity)
                return luaL_error(state, "auth.begin identity must be a positive integer");
        } else if (id == GNOBLIN_OP_AUTH_RESPOND) {
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

    if (operation && operation->dispatch == GN_DISPATCH_PROMPT) {
        static const char* const respond_fields[] = {"request_id", "password", "confirmed",
                                                     "choice", NULL};
        static const char* const cancel_fields[] = {"request_id", NULL};
        gboolean respond = id == GNOBLIN_OP_PROMPT_RESPOND;
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

    if (id == GNOBLIN_OP_INPUT_SET_ORIENTATION_LOCK && config->api_calling) {
        static const char* const fields[] = {"value", NULL};
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state, "input.set_orientation_lock accepts only value");
        lua_getfield(state, 1, "value");
        lua_replace(state, 1);
        lua_settop(state, 1);
    }

    GVariant* arguments = NULL;
    if (id == GNOBLIN_OP_INPUT_SET_ORIENTATION_LOCK) {
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
        if ((operation && operation->dispatch == GN_DISPATCH_SHORTCUT) || window_call)
            return luaL_error(state, "%s requires an argument table", method);
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        if (window_call)
            window_operation_valid_args(state, operation);
        if (id == GNOBLIN_OP_INPUT_SELECT)
            input_source_selector_valid_args(state);
        if (id == GNOBLIN_OP_SHORTCUT_CAPTURE) {
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
        if (id == GNOBLIN_OP_SHORTCUT_BIND) {
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
        if (id == GNOBLIN_OP_SHORTCUT_UNBIND) {
            static const char* const fields[] = {"id", NULL};
            if (!table_fields_allowed(state, 1, fields))
                return luaL_error(state, "shortcut.unbind accepts only id");
        }
        if (id == GNOBLIN_OP_SHORTCUT_SESSION_END) {
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
    } else if (lua_gettop(state) == 1 && lua_isnil(state, 1) && id == GNOBLIN_OP_SHORTCUT_CAPTURE) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else {
        return luaL_error(state, "%s takes no arguments or one argument table", method);
    }

    return lua_queue_operation(state, config, method, arguments);
}
