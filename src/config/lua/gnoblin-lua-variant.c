#include "gnoblin-lua-internal.h"

static void push_snap_context(lua_State* state, GVariant* value);

typedef struct {
    char* key;
    GVariant* child;
} SortedEntry;

static gint entry_compare(gconstpointer a, gconstpointer b) {
    return strcmp(((const SortedEntry*)a)->key, ((const SortedEntry*)b)->key);
}

GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
                           int keybinding_depth, GError** error) {
    int type = lua_type(state, index);
    if (depth > MAX_CONFIG_DEPTH) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config nesting exceeds 64 levels");
        return NULL;
    }
    if (type == LUA_TUSERDATA) {
        if (!luaL_testudata(state, index, LUA_READONLY_SNAPSHOT_METATABLE)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua values must be read-only Gnoblin snapshots");
            return NULL;
        }
        lua_getiuservalue(state, index, 1);
        GVariant* value =
            variant_from_lua(state, -1, depth + 1, force_array, keybinding_depth, error);
        lua_pop(state, 1);
        return value;
    }
    if (type == LUA_TBOOLEAN)
        return g_variant_new_boolean(lua_toboolean(state, index));
    if (type == LUA_TNUMBER) {
        if (lua_isinteger(state, index))
            return g_variant_new_int64(lua_tointeger(state, index));
        if (!isfinite(lua_tonumber(state, index))) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config numbers must be finite");
            return NULL;
        }
        return g_variant_new_double(lua_tonumber(state, index));
    }
    if (type == LUA_TSTRING) {
        size_t length;
        const char* text = lua_tolstring(state, index, &length);
        if (memchr(text, '\0', length) || !g_utf8_validate(text, length, NULL)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config strings must be UTF-8 without NUL bytes");
            return NULL;
        }
        return g_variant_new_string(text);
    }
    if (type != LUA_TTABLE) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua config cannot contain %s values",
                    lua_typename(state, type));
        return NULL;
    }

    index = lua_absindex(state, index);
    gboolean strings = FALSE, numbers = FALSE;
    lua_pushnil(state);
    while (lua_next(state, index)) {
        if (lua_type(state, -2) == LUA_TSTRING)
            strings = TRUE;
        else if (lua_isinteger(state, -2) && lua_tointeger(state, -2) > 0)
            numbers = TRUE;
        else {
            lua_pop(state, 1);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua config table keys must be strings or positive integers");
            return NULL;
        }
        lua_pop(state, 1);
    }
    if (strings && numbers) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config tables cannot mix map and array keys");
        return NULL;
    }
    gboolean array = marked_array(state, index) || numbers || (!strings && force_array);
    if (array) {
        lua_Integer count = lua_rawlen(state, index);
        GVariantBuilder out;
        g_variant_builder_init(&out, G_VARIANT_TYPE("av"));
        lua_pushnil(state);
        while (lua_next(state, index)) {
            gboolean valid = lua_isinteger(state, -2) && lua_tointeger(state, -2) >= 1 &&
                             lua_tointeger(state, -2) <= count;
            lua_pop(state, 1);
            if (!valid) {
                g_variant_builder_clear(&out);
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua config arrays must be dense and start at 1");
                return NULL;
            }
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, index, i);
            g_autoptr(GVariant) child = variant_from_lua(state, -1, depth + 1, FALSE, FALSE, error);
            lua_pop(state, 1);
            if (!child) {
                g_variant_builder_clear(&out);
                return NULL;
            }
            g_variant_builder_add(&out, "v", g_variant_ref(child));
        }
        return g_variant_builder_end(&out);
    }
    /* Lua visits table keys in hash order, which changes when a table is rebuilt. Emit entries
     * sorted by key so equal documents serialise equally and compare equal. */
    typedef SortedEntry Entry;
    g_autoptr(GArray) entries = g_array_new(FALSE, FALSE, sizeof(Entry));
    gboolean failed = FALSE;
    lua_pushnil(state);
    while (lua_next(state, index)) {
        const char* key = lua_tostring(state, -2);
        gboolean child_array = append_array_key(key) || keybinding_depth == 2;
        int child_keybinding_depth = !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1     ? 2
                                                                 : 0;
        GVariant* child =
            variant_from_lua(state, -1, depth + 1, child_array, child_keybinding_depth, error);
        if (!child) {
            lua_pop(state, 2);
            failed = TRUE;
            break;
        }
        Entry entry = {g_strdup(key), g_variant_ref_sink(child)};
        g_array_append_val(entries, entry);
        lua_pop(state, 1);
    }
    g_array_sort(entries, entry_compare);
    GVariantBuilder out;
    g_variant_builder_init(&out, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < entries->len; i++) {
        Entry* entry = &g_array_index(entries, Entry, i);
        if (!failed)
            g_variant_builder_add(&out, "{sv}", entry->key, entry->child);
        g_free(entry->key);
        g_variant_unref(entry->child);
    }
    if (failed) {
        g_variant_builder_clear(&out);
        return NULL;
    }
    return g_variant_builder_end(&out);
}

void push_variant(lua_State* state, GVariant* value) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        push_variant(state, child);
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        lua_pushboolean(state, g_variant_get_boolean(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        lua_pushinteger(state, g_variant_get_int64(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        lua_pushinteger(state, g_variant_get_int32(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
        lua_pushinteger(state, g_variant_get_uint32(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT64))
        lua_pushinteger(state, (lua_Integer)g_variant_get_uint64(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        lua_pushnumber(state, g_variant_get_double(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        lua_pushstring(state, g_variant_get_string(value, NULL));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING_ARRAY)) {
        lua_newtable(state);
        mark_array_table(state, -1);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            lua_pushstring(state, g_variant_get_string(child, NULL));
            lua_rawseti(state, -2, i + 1);
        }
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE("aa{sv}"))) {
        lua_newtable(state);
        mark_array_table(state, -1);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            push_variant(state, child);
            lua_rawseti(state, -2, i + 1);
        }
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        lua_newtable(state);
        mark_array_table(state, -1);
        GVariantIter iter;
        GVariant* child;
        guint i = 1;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "v", &child)) {
            push_variant(state, child);
            lua_rawseti(state, -2, i++);
            g_variant_unref(child);
        }
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        lua_newtable(state);
        GVariantIter iter;
        const char* key;
        GVariant* child;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            push_variant(state, child);
            lua_setfield(state, -2, key);
            g_variant_unref(child);
        }
    } else
        lua_pushnil(state);
}

static int lua_readonly_newindex(lua_State* state) {
    return luaL_error(state, "Gnoblin snapshots are read-only");
}

static int lua_readonly_index(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    if (!lua_isnil(state, -1))
        return 1;
    lua_pop(state, 2);
    lua_getiuservalue(state, 1, 2);
    lua_pushvalue(state, 2);
    lua_gettable(state, -2);
    return 1;
}

static int lua_readonly_len(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushinteger(state, (lua_Integer)lua_rawlen(state, -1));
    return 1;
}

static int lua_readonly_next(lua_State* state) {
    lua_pushvalue(state, lua_upvalueindex(1));
    lua_pushvalue(state, 2);
    if (!lua_next(state, -2))
        return 0;
    lua_remove(state, -3);
    return 2;
}

/* Methods on snapshot records are thin adapters to the existing typed
 * operation dispatcher. Keeping the ID on the detached record means stale
 * records still fail in the compositor rather than targeting a reused row. */
static int lua_record_method(lua_State* state) {
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    const char* method = lua_tostring(state, lua_upvalueindex(1));
    gboolean workspace = g_str_has_prefix(method, "workspace.");
    gboolean workspace_move = g_str_equal(method, "workspace.move_window");
    gboolean thumbnail = g_str_equal(method, "window.thumbnail");
    lua_getiuservalue(state, 1, 1);
    int backing = lua_absindex(state, -1);
    lua_getfield(state, -1, "id");
    if (lua_type(state, -1) != LUA_TSTRING)
        return luaL_error(state, "snapshot record has no stable id");
    const char* id = lua_tostring(state, -1);
    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    if (!workspace_move) {
        lua_pushstring(state, id);
        lua_setfield(state, arguments, "id");
    }
    lua_remove(state, -2); /* id */
    lua_remove(state, -2); /* backing */
    arguments = lua_gettop(state);

    int supplied = arguments - 2; /* Exclude receiver and constructed argument table. */
    gboolean focus = g_str_equal(method, "window.focus");
    gboolean begin_move = g_str_equal(method, "window.begin_move");
    gboolean begin_resize = g_str_equal(method, "window.begin_resize");
    if (focus || begin_move || begin_resize) {
        int context_index = begin_resize ? 3 : 2;
        LuaFocusContext* context =
            luaL_testudata(state, context_index, LUA_FOCUS_CONTEXT_METATABLE);
        if ((focus && supplied != 1) || (begin_move && supplied != 1) ||
            (begin_resize && supplied != 2) || !context)
            return luaL_error(state, "%s requires a FocusContext from a shortcut event", method);
        if (begin_resize) {
            if (lua_type(state, 2) != LUA_TSTRING)
                return luaL_error(state, "window:begin_resize requires an edge string");
            const char* edge = lua_tostring(state, 2);
            static const char* const edges[] = {"north",      "south",      "east",
                                                "west",       "north_east", "north_west",
                                                "south_east", "south_west", NULL};
            gboolean valid_edge = FALSE;
            for (guint i = 0; edges[i]; i++)
                valid_edge |= g_str_equal(edge, edges[i]);
            if (!valid_edge)
                return luaL_error(state, "window:begin_resize edge is not a valid ResizeEdge");
            lua_pushvalue(state, 2);
            lua_setfield(state, arguments, "edge");
        }
        if (!config || !active_runtime || config != &active_runtime->config ||
            !config->dispatching || config->api_calling)
            return luaL_error(state, "%s is available only in a runtime event callback", method);
        if (context->consumed || !context->handle ||
            context->generation != active_runtime->generation ||
            context->expires_at_us <= g_get_monotonic_time())
            return luaL_error(state,
                              "FocusContext is expired, consumed, or belongs to an old runtime");
        if (!config->runtime_actions || config->runtime_actions->len >= 256 ||
            config->actions_in_dispatch >= 64)
            return luaL_error(state, "too many pending Gnoblin session actions");

        guint64 request_id = allocate_operation_id();
        if (!request_id)
            return luaL_error(state, "Gnoblin operation request ID space is exhausted");
        GVariantBuilder focus_arguments;
        g_variant_builder_init(&focus_arguments, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&focus_arguments, "{sv}", "id", g_variant_new_string(id));
        if (begin_resize)
            g_variant_builder_add(&focus_arguments, "{sv}", "edge",
                                  g_variant_new_string(lua_tostring(state, 2)));
        GVariantBuilder action;
        g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
        g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
        g_variant_builder_add(&action, "{sv}", "arguments",
                              g_variant_builder_end(&focus_arguments));
        g_ptr_array_add(config->runtime_actions,
                        g_variant_ref_sink(g_variant_builder_end(&action)));
        g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, request_id);
        LuaFocusOperation* operation = g_new0(LuaFocusOperation, 1);
        operation->handle = context->handle;
        operation->generation = context->generation;
        operation->native_generation = context->native_generation;
        operation->expires_at_us = context->expires_at_us;
        g_hash_table_replace(config->focus_operations, g_strdup(key), operation);
        context->consumed = TRUE;
        config->actions_in_dispatch++;
        push_operation_handle(state, config, request_id, method);
        return 1;
    }
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.kill") ||
        g_str_equal(method, "window.raise") || g_str_equal(method, "window.lower") ||
        g_str_equal(method, "window.minimize") || g_str_equal(method, "window.toggle_minimize") ||
        g_str_equal(method, "window.restore") || g_str_equal(method, "window.unminimize") ||
        g_str_equal(method, "window.restore_or_minimize") ||
        g_str_equal(method, "workspace.remove") || g_str_equal(method, "workspace.switch")) {
        if (supplied != 0)
            return luaL_error(state, "%s takes no arguments", method);
    } else if (g_str_equal(method, "window.set_above") ||
               g_str_equal(method, "window.set_sticky") ||
               g_str_equal(method, "window.set_maximized") ||
               g_str_equal(method, "window.set_fullscreen")) {
        if (supplied != 1 || !lua_isboolean(state, 2))
            return luaL_error(state, "%s requires one boolean", method);
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "enabled");
    } else if (g_str_equal(method, "window.move") || g_str_equal(method, "window.resize")) {
        if (supplied != 1 || !lua_istable(state, 2))
            return luaL_error(state, "%s requires one geometry table", method);
        static const char* const move_fields[] = {"x", "y", NULL};
        static const char* const resize_fields[] = {"width", "height", NULL};
        if (!table_fields_allowed(state, 2,
                                  g_str_equal(method, "window.move") ? move_fields : resize_fields))
            return luaL_error(state, "%s received an unsupported geometry field", method);
        const char* a = g_str_equal(method, "window.move") ? "x" : "width";
        const char* b = g_str_equal(method, "window.move") ? "y" : "height";
        lua_getfield(state, 2, a);
        lua_setfield(state, arguments, a);
        lua_getfield(state, 2, b);
        lua_setfield(state, arguments, b);
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        if (supplied < 1 || supplied > 2 || !lua_istable(state, 2))
            return luaL_error(state,
                              "window:move_to_workspace requires a selector and optional options");
        static const char* const selector_fields[] = {"id", "number", NULL};
        static const char* const option_fields[] = {"follow", NULL};
        if (!table_fields_allowed(state, 2, selector_fields))
            return luaL_error(state, "window:move_to_workspace selector accepts only id or number");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "workspace");
        if (supplied == 2 && !lua_isnil(state, 3)) {
            if (!lua_istable(state, 3) || !table_fields_allowed(state, 3, option_fields))
                return luaL_error(state, "window:move_to_workspace options accept only follow");
            lua_getfield(state, 3, "follow");
            if (!lua_isnil(state, -1))
                lua_setfield(state, arguments, "follow");
            else
                lua_pop(state, 1);
        }
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        if (supplied != 1)
            return luaL_error(state, "%s requires a monitor selector", method);
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "monitor");
    } else if (thumbnail) {
        if (supplied != 1 || !lua_istable(state, 2))
            return luaL_error(state, "window:thumbnail requires a size table");
        static const char* const size_fields[] = {"width", "height", NULL};
        if (!table_fields_allowed(state, 2, size_fields))
            return luaL_error(state, "window:thumbnail size accepts only width and height");
        const char* dimensions[] = {"width", "height"};
        const lua_Integer limits[] = {480, 320};
        for (guint i = 0; i < G_N_ELEMENTS(dimensions); i++) {
            lua_getfield(state, 2, dimensions[i]);
            if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                lua_tointeger(state, -1) > limits[i])
                return luaL_error(state, "window:thumbnail %s must be an integer from 1 to %d",
                                  dimensions[i], (int)limits[i]);
            lua_setfield(state, arguments, dimensions[i]);
        }
    } else if (g_str_equal(method, "workspace.rename")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TSTRING)
            return luaL_error(state, "workspace:rename requires a name");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "name");
    } else if (g_str_equal(method, "workspace.move_window")) {
        if (supplied < 1 || supplied > 2)
            return luaL_error(state, "workspace:move_here requires a window and optional options");
        if (lua_type(state, 2) == LUA_TSTRING)
            lua_pushvalue(state, 2);
        else if (lua_isuserdata(state, 2))
            lua_getfield(state, 2, "id");
        else
            return luaL_error(state, "workspace:move_here requires a Window record or window ID");
        if (lua_type(state, -1) != LUA_TSTRING)
            return luaL_error(state, "workspace:move_here requires a Window record or window ID");
        lua_setfield(state, arguments, "window");
        lua_newtable(state);
        lua_pushstring(state, id);
        lua_setfield(state, -2, "id");
        lua_setfield(state, arguments, "workspace");
        if (supplied == 2 && !lua_isnil(state, 3)) {
            static const char* const option_fields[] = {"follow", NULL};
            if (!lua_istable(state, 3) || !table_fields_allowed(state, 3, option_fields))
                return luaL_error(state, "workspace:move_here options accept only follow");
            lua_getfield(state, 3, "follow");
            if (!lua_isnil(state, -1))
                lua_setfield(state, arguments, "follow");
            else
                lua_pop(state, 1);
        }
    }
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, workspace ? lua_workspace_action : lua_generic_api_action, 2);
    lua_insert(state, arguments);
    lua_call(state, 1, 1);
    return 1;
}

static void add_record_method(lua_State* state, int method_table, const char* name,
                              const char* operation) {
    lua_pushstring(state, operation);
    lua_pushcclosure(state, lua_record_method, 1);
    lua_setfield(state, method_table, name);
}

static void add_animation_preview_method(lua_State* state, int method_table, const char* name,
                                         const char* operation) {
    lua_pushstring(state, operation);
    lua_pushcclosure(state, lua_animation_preview_method, 1);
    lua_setfield(state, method_table, name);
}

static void add_snapshot_methods(lua_State* state, int backing, int method_table) {
    lua_getfield(state, backing, "id");
    gboolean has_id = lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    if (!has_id)
        return;
    lua_getfield(state, backing, "frame");
    gboolean is_window = lua_istable(state, -1) ||
                         luaL_testudata(state, -1, LUA_READONLY_SNAPSHOT_METATABLE) != NULL;
    lua_pop(state, 1);
    lua_getfield(state, backing, "number");
    gboolean is_workspace = lua_isinteger(state, -1);
    lua_pop(state, 1);
    lua_getfield(state, backing, "session");
    gboolean is_animation_preview = lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    lua_getfield(state, backing, "name");
    is_animation_preview &= lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    lua_getfield(state, backing, "target_type");
    is_animation_preview &= lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    lua_getfield(state, backing, "progress");
    is_animation_preview &= lua_type(state, -1) == LUA_TNUMBER;
    lua_pop(state, 1);
    lua_getfield(state, backing, "playing");
    is_animation_preview &= lua_isboolean(state, -1);
    lua_pop(state, 1);
    if (is_window) {
        const char* methods[][2] = {{"close", "window.close"},
                                    {"kill", "window.kill"},
                                    {"raise", "window.raise"},
                                    {"lower", "window.lower"},
                                    {"minimize", "window.minimize"},
                                    {"toggle_minimize", "window.toggle_minimize"},
                                    {"unminimize", "window.unminimize"},
                                    {"restore", "window.restore"},
                                    {"restore_or_minimize", "window.restore_or_minimize"},
                                    {"set_maximized", "window.set_maximized"},
                                    {"set_fullscreen", "window.set_fullscreen"},
                                    {"set_above", "window.set_above"},
                                    {"set_sticky", "window.set_sticky"},
                                    {"move", "window.move"},
                                    {"resize", "window.resize"},
                                    {"move_to_workspace", "window.move_to_workspace"},
                                    {"move_to_monitor", "window.move_to_monitor"},
                                    {"focus", "window.focus"},
                                    {"begin_move", "window.begin_move"},
                                    {"begin_resize", "window.begin_resize"},
                                    {"thumbnail", "window.thumbnail"},
                                    {NULL, NULL}};
        for (guint i = 0; methods[i][0]; i++)
            add_record_method(state, method_table, methods[i][0], methods[i][1]);
    } else if (is_animation_preview) {
        add_animation_preview_method(state, method_table, "seek", "animation.seek");
        add_animation_preview_method(state, method_table, "step", "animation.step");
        add_animation_preview_method(state, method_table, "play", "animation.play");
        add_animation_preview_method(state, method_table, "pause", "animation.pause");
        add_animation_preview_method(state, method_table, "stop", "animation.stop");
    } else if (is_workspace) {
        add_record_method(state, method_table, "activate", "workspace.switch");
        add_record_method(state, method_table, "rename", "workspace.rename");
        add_record_method(state, method_table, "remove", "workspace.remove");
        add_record_method(state, method_table, "move_here", "workspace.move_window");
    }
}

static int lua_readonly_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

/* Snapshot userdata is detached from the runtime cache and cannot be changed
 * with ordinary assignment or rawset. Nested tables use the same wrapper. */
void push_readonly_copy(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index)) {
        lua_pushvalue(state, index);
        return;
    }

    lua_newuserdatauv(state, sizeof(gpointer), 2);
    int readonly = lua_absindex(state, -1);
    lua_newtable(state); /* private backing table */
    int backing = lua_absindex(state, -1);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        push_readonly_copy(state, -1);
        lua_pushvalue(state, -3);
        lua_insert(state, -2);
        lua_rawset(state, backing);
        lua_pop(state, 1);
    }
    if (marked_array(state, index))
        mark_array_table(state, backing);
    lua_newtable(state); /* Methods stay out of pairs()/snapshot fields. */
    int method_table = lua_absindex(state, -1);
    add_snapshot_methods(state, backing, method_table);
    lua_pushvalue(state, method_table);
    lua_setiuservalue(state, readonly, 2);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, readonly, 1);
    lua_pop(state, 2); /* method table, backing */

    if (luaL_newmetatable(state, LUA_READONLY_SNAPSHOT_METATABLE)) {
        lua_pushcfunction(state, lua_readonly_index);
        lua_setfield(state, -2, "__index");
        lua_pushcfunction(state, lua_readonly_newindex);
        lua_setfield(state, -2, "__newindex");
        lua_pushcfunction(state, lua_readonly_len);
        lua_setfield(state, -2, "__len");
        lua_pushcfunction(state, lua_readonly_pairs);
        lua_setfield(state, -2, "__pairs");
        lua_pushboolean(state, FALSE);
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, readonly);
}

/* Event payloads describe a point-in-time state just like the snapshot API.
 * Keep the event envelope mutable while making every structured field a
 * detached read-only value with any applicable record methods. */
void push_readonly_event_fields(lua_State* state, GVariant* payload, int event_index) {
    event_index = lua_absindex(state, event_index);
    GVariantIter iter;
    const char* key = NULL;
    GVariant* value = NULL;
    g_variant_iter_init(&iter, payload);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        lua_getfield(state, event_index, key);
        if (lua_istable(state, -1)) {
            push_readonly_copy(state, -1);
            lua_setfield(state, event_index, key);
        }
        lua_pop(state, 1);
        g_variant_unref(value);
    }
}

static gboolean variant_is_animation_preview(GVariant* value) {
    const char* session = NULL;
    const char* name = NULL;
    const char* target_type = NULL;
    double progress = 0.0;
    gboolean playing = FALSE;
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) &&
           g_variant_lookup(value, "session", "&s", &session) && session && *session &&
           g_variant_lookup(value, "name", "&s", &name) && name && *name &&
           g_variant_lookup(value, "target_type", "&s", &target_type) && target_type &&
           *target_type && g_variant_lookup(value, "progress", "d", &progress) &&
           isfinite(progress) && g_variant_lookup(value, "playing", "b", &playing);
}

static void push_text_target(lua_State* state, GVariant* value) {
    const char* token = NULL;
    const char* window_id = NULL;
    g_autoptr(GVariant) caret = NULL;
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(value, "target", "&s", &token) || !token || strlen(token) != 64 ||
        !g_str_is_ascii(token)) {
        lua_pushnil(state);
        return;
    }
    caret = g_variant_lookup_value(value, "caret", G_VARIANT_TYPE_VARDICT);
    g_variant_lookup(value, "window_id", "&s", &window_id);
    for (const char* cursor = token; *cursor; cursor++)
        if (!g_ascii_isxdigit(*cursor)) {
            lua_pushnil(state);
            return;
        }
    LuaTextTarget* target = lua_newuserdatauv(state, sizeof(*target), 0);
    memset(target, 0, sizeof(*target));
    g_strlcpy(target->token, token, sizeof(target->token));
    if (window_id && strlen(window_id) < sizeof(target->window_id))
        g_strlcpy(target->window_id, window_id, sizeof(target->window_id));
    target->generation = active_runtime ? active_runtime->generation : 0;
    target->has_caret = caret && g_variant_lookup(caret, "x", "d", &target->x) &&
                        g_variant_lookup(caret, "y", "d", &target->y) &&
                        g_variant_lookup(caret, "width", "d", &target->width) &&
                        g_variant_lookup(caret, "height", "d", &target->height);
    if (luaL_newmetatable(state, LUA_TEXT_TARGET_METATABLE)) {
        lua_pushcfunction(state, lua_text_target_index);
        lua_setfield(state, -2, "__index");
        lua_pushliteral(state, "TextTarget");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

void push_window_drag(lua_State* state, GVariant* value) {
    LuaWindowDrag* drag = lua_newuserdatauv(state, sizeof(*drag), 2);
    memset(drag, 0, sizeof(*drag));
    g_variant_lookup(value, "id", "t", &drag->id);
    g_variant_lookup(value, "settings_revision", "t", &drag->settings_revision);
    drag->generation = active_runtime ? active_runtime->generation : 0;
    push_variant(state, value);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    lua_setiuservalue(state, -2, 1);
    lua_newtable(state);
    lua_pushcfunction(state, lua_window_drag_offer_targets);
    lua_setfield(state, -2, "offer_targets");
    lua_setiuservalue(state, -2, 2);
    if (luaL_newmetatable(state, LUA_WINDOW_DRAG_METATABLE)) {
        lua_pushcfunction(state, lua_readonly_index);
        lua_setfield(state, -2, "__index");
        lua_pushcfunction(state, lua_readonly_newindex);
        lua_setfield(state, -2, "__newindex");
        lua_pushliteral(state, "WindowDrag");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

static void push_snap_context(lua_State* state, GVariant* value) {
    const char* token = NULL;
    if (!value || !g_variant_lookup(value, "context", "&s", &token) || !token ||
        strlen(token) != 64 || !g_str_is_ascii(token)) {
        lua_pushnil(state);
        return;
    }
    LuaSnapContext* context = lua_newuserdatauv(state, sizeof(*context), 2);
    memset(context, 0, sizeof(*context));
    g_strlcpy(context->token, token, sizeof(context->token));
    context->generation = active_runtime ? active_runtime->generation : 0;
    GVariantBuilder public_fields;
    g_variant_builder_init(&public_fields, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* field;
    g_variant_iter_init(&iter, value);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &field)) {
        g_autoptr(GVariant) owned = field;
        if (!g_str_equal(key, "context"))
            g_variant_builder_add(&public_fields, "{sv}", key, field);
    }
    g_autoptr(GVariant) public_value = g_variant_ref_sink(g_variant_builder_end(&public_fields));
    push_variant(state, public_value);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    lua_setiuservalue(state, -2, 1);
    lua_newtable(state);
    lua_pushcfunction(state, lua_snap_context_commit);
    lua_setfield(state, -2, "commit");
    lua_setiuservalue(state, -2, 2);
    if (luaL_newmetatable(state, LUA_SNAP_CONTEXT_METATABLE)) {
        lua_pushcfunction(state, lua_readonly_index);
        lua_setfield(state, -2, "__index");
        lua_pushcfunction(state, lua_readonly_newindex);
        lua_setfield(state, -2, "__newindex");
        lua_pushliteral(state, "SnapContext");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

void push_operation_result(lua_State* state, const char* method, GVariant* value) {
    if (g_str_equal(method, "input.text_target")) {
        push_text_target(state, value);
        return;
    }
    if (g_str_equal(method, "window.snap_context")) {
        push_snap_context(state, value);
        return;
    }
    push_variant(state, value);
    if (!g_str_has_prefix(method, "animation.") || !variant_is_animation_preview(value))
        return;
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
}
