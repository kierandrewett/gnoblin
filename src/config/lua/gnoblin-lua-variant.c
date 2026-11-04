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
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_BYTESTRING)) {
        /* Raw bytes, such as process output, which need not be UTF-8. */
        gsize length = 0;
        const void* bytes = g_variant_get_fixed_array(value, &length, 1);
        lua_pushlstring(state, length ? bytes : "", length);
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING_ARRAY)) {
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
    gboolean thumbnail = g_str_equal(method, "window.thumbnail");
    /* A launch is named by its token, every other record by its id. */
    const char* key_field = g_str_has_prefix(method, "launch.") ? "token" : "id";
    lua_getiuservalue(state, 1, 1);
    int backing = lua_absindex(state, -1);
    lua_getfield(state, -1, key_field);
    if (lua_type(state, -1) != LUA_TSTRING)
        return luaL_error(state, "snapshot record has no stable %s", key_field);
    const char* id = lua_tostring(state, -1);
    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    lua_pushstring(state, id);
    lua_setfield(state, arguments, key_field);
    if (g_str_equal(method, "input.select")) {
        /* An input source is named by its type and id. */
        lua_getfield(state, backing, "type");
        lua_setfield(state, arguments, "type");
    }
    lua_remove(state, -2); /* id */
    lua_remove(state, -2); /* backing */
    arguments = lua_gettop(state);

    int supplied = arguments - 2; /* Exclude receiver and constructed argument table. */
    gboolean focus = g_str_equal(method, "window.focus");
    gboolean begin_move = g_str_equal(method, "window.begin_move");
    gboolean begin_resize = g_str_equal(method, "window.begin_resize");
    if (focus || begin_move || begin_resize) {
        if ((focus && supplied != 1) || (begin_move && supplied != 1) ||
            (begin_resize && supplied != 2))
            return luaL_error(state,
                              "%s requires %sa Grant from a shortcut, pointer or window "
                              "menu event as its last argument",
                              method, begin_resize ? "an edge and " : "");
        const char* edge = NULL;
        if (begin_resize) {
            if (lua_type(state, 2) != LUA_TSTRING)
                return luaL_error(state, "window:begin_resize requires an edge string");
            edge = lua_tostring(state, 2);
            static const char* const edges[] = {"north",      "south",      "east",
                                                "west",       "north_east", "north_west",
                                                "south_east", "south_west", NULL};
            gboolean valid_edge = FALSE;
            for (guint i = 0; edges[i]; i++)
                valid_edge |= g_str_equal(edge, edges[i]);
            if (!valid_edge)
                return luaL_error(state, "window:begin_resize edge is not a valid ResizeEdge");
        }
        return lua_window_grant_operation(state, config, method, id, begin_resize ? 3 : 2, edge);
    }
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.kill") ||
        g_str_equal(method, "window.raise") || g_str_equal(method, "window.lower") ||
        g_str_equal(method, "window.restore_or_minimize") ||
        g_str_equal(method, "window.present") || g_str_equal(method, "workspace.remove") ||
        g_str_equal(method, "workspace.switch") || g_str_equal(method, "shortcut.unbind") ||
        g_str_equal(method, "launch.end") || g_str_equal(method, "input.select") ||
        g_str_equal(method, "idle.cancel") || g_str_equal(method, "override.revoke") ||
        g_str_equal(method, "input.grab.renew") || g_str_equal(method, "input.grab.release")) {
        if (supplied != 0)
            return luaL_error(state, "%s takes no arguments", method);
    } else if (g_str_equal(method, "window.set_minimized")) {
        /* Minimizing and restoring are two compositor operations; the boolean picks one. */
        if (supplied != 1 || !lua_isboolean(state, 2))
            return luaL_error(state, "%s requires one boolean", method);
        method = lua_toboolean(state, 2) ? "window.minimize" : "window.unminimize";
    } else if (g_str_equal(method, "window.set_above") ||
               g_str_equal(method, "window.set_sticky") ||
               g_str_equal(method, "window.set_maximized") ||
               g_str_equal(method, "window.set_fullscreen")) {
        if (supplied != 1 || !lua_isboolean(state, 2))
            return luaL_error(state, "%s requires one boolean", method);
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "enabled");
    } else if (g_str_equal(method, "window.move") || g_str_equal(method, "window.resize")) {
        gboolean move = g_str_equal(method, "window.move");
        if (supplied != 2 || !lua_isinteger(state, 2) || !lua_isinteger(state, 3))
            return luaL_error(state, move ? "window:move takes integer x and y"
                                          : "window:resize takes integer width and height");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, move ? "x" : "width");
        lua_pushvalue(state, 3);
        lua_setfield(state, arguments, move ? "y" : "height");
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        if (supplied < 1 || supplied > 2)
            return luaL_error(state,
                              "window:move_to_workspace takes a workspace and optional options");
        static const char* const option_fields[] = {"follow", NULL};
        if (!lua_push_workspace_selector(state, 2))
            return luaL_error(state, "window:move_to_workspace takes a workspace ID string or "
                                     "number");
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
    } else if (g_str_equal(method, "window.set_effect")) {
        /* Returns an Override, not the Operation (gnoblin-lua-state-queries.c). */
        if (supplied != 2)
            return luaL_error(state, "window:set_effect takes an effect name and a value");
        return lua_window_set_effect(state, config, id, 2, 3);
    } else if (g_str_equal(method, "window.set_visual")) {
        /* set_opacity(v), set_scale(s), set_rotation(degrees), set_offset(x, y) and
         * reset_visuals() share the operation; the compositor checks the ranges. */
        const char* name = lua_tostring(state, lua_upvalueindex(2));
        static const struct {
            const char* method;
            const char* fields[2];
        } visuals[] = {
            {"set_opacity", {"opacity", NULL}},   {"set_scale", {"scale", NULL}},
            {"set_rotation", {"rotation", NULL}}, {"set_offset", {"offset_x", "offset_y"}},
            {"reset_visuals", {NULL, NULL}},
        };
        gboolean known = FALSE;
        for (guint i = 0; name && i < G_N_ELEMENTS(visuals); i++) {
            if (!g_str_equal(name, visuals[i].method))
                continue;
            known = TRUE;
            int count = visuals[i].fields[1] ? 2 : visuals[i].fields[0] ? 1 : 0;
            if (supplied != count)
                return luaL_error(state, "window:%s takes %d number%s", name, count,
                                  count == 1 ? "" : "s");
            for (int field = 0; field < count; field++) {
                if (lua_type(state, 2 + field) != LUA_TNUMBER)
                    return luaL_error(state, "window:%s takes numbers", name);
                lua_pushvalue(state, 2 + field);
                lua_setfield(state, arguments, visuals[i].fields[field]);
            }
            if (count == 0) {
                lua_pushboolean(state, TRUE);
                lua_setfield(state, arguments, "reset");
            }
        }
        if (!known)
            return luaL_error(state, "unknown window visual method");
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TSTRING)
            return luaL_error(state, "window:move_to_monitor takes a monitor ID");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "monitor");
    } else if (thumbnail) {
        if (supplied != 2)
            return luaL_error(state, "window:thumbnail takes width and height");
        const char* dimensions[] = {"width", "height"};
        const lua_Integer limits[] = {480, 320};
        for (guint i = 0; i < G_N_ELEMENTS(dimensions); i++) {
            if (!lua_isinteger(state, 2 + (int)i) || lua_tointeger(state, 2 + (int)i) < 1 ||
                lua_tointeger(state, 2 + (int)i) > limits[i])
                return luaL_error(state, "window:thumbnail %s must be an integer from 1 to %d",
                                  dimensions[i], (int)limits[i]);
            lua_pushvalue(state, 2 + (int)i);
            lua_setfield(state, arguments, dimensions[i]);
        }
    } else if (g_str_equal(method, "workspace.rename")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TSTRING)
            return luaL_error(state, "workspace:rename requires a name");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "name");
    } else if (g_str_equal(method, "shortcut.session.end")) {
        if (supplied != 1 || !lua_isinteger(state, 2) || lua_tointeger(state, 2) <= 0)
            return luaL_error(state, "shortcut:end_session takes a positive integer session ID");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "session_id");
    }
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, workspace ? lua_workspace_action : lua_generic_api_action, 2);
    lua_insert(state, arguments);
    lua_call(state, 1, 1);
    /* A shortcut ends when the compositor has unbound it. */
    if (config && g_str_equal(method, "shortcut.unbind"))
        lua_object_end_after(state, config, -1, "Shortcut", 1, TRUE);
    /* An idle watch ends when the compositor has removed it. */
    if (config && g_str_equal(method, "idle.cancel"))
        lua_object_end_after(state, config, -1, "IdleWatch", 1, TRUE);
    return 1;
}

/* Add the lua_api methods of one record class, such as Window, to a record's method table. Each
 * gets its wire operation as upvalue 1, or the class name when it has none, and its own name as
 * upvalue 2, which tells apart methods that share one operation. */
static void add_record_methods(lua_State* state, int method_table, const char* class) {
    gsize length = strlen(class);
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (strncmp(entry->path, class, length) != 0 || entry->path[length] != ':')
            continue;
        lua_pushstring(state, entry->operation ? entry->operation : class);
        lua_pushstring(state, entry->path + length + 1);
        lua_pushcclosure(state, entry->function ? entry->function : lua_record_method, 2);
        lua_setfield(state, method_table, entry->path + length + 1);
    }
}

/* The Lua type of field `name` of the record at `backing`. */
static int backing_field_type(lua_State* state, int backing, const char* name) {
    int type = lua_getfield(state, backing, name);
    lua_pop(state, 1);
    return type;
}

/* Install the record methods for the kind of record `backing` holds and return that kind, or
 * NULL for plain data. */
static const char* add_snapshot_methods(lua_State* state, int backing, int method_table) {
    /* A launch is named by its token and has a launch state. */
    if (backing_field_type(state, backing, "token") == LUA_TSTRING &&
        backing_field_type(state, backing, "application") == LUA_TSTRING &&
        backing_field_type(state, backing, "state") == LUA_TSTRING &&
        backing_field_type(state, backing, "timeout_ms") == LUA_TNUMBER) {
        add_record_methods(state, method_table, "Launch");
        lua_object_add_methods(state, method_table, "Launch");
        return "Launch";
    }
    lua_getfield(state, backing, "id");
    gboolean has_id = lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    if (!has_id)
        return NULL;
    lua_getfield(state, backing, "frame");
    gboolean is_window = lua_istable(state, -1) ||
                         luaL_testudata(state, -1, LUA_READONLY_SNAPSHOT_METATABLE) != NULL;
    lua_pop(state, 1);
    lua_getfield(state, backing, "number");
    gboolean is_workspace = lua_isinteger(state, -1);
    lua_pop(state, 1);
    lua_getfield(state, backing, "namespace");
    gboolean is_layer = lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    lua_getfield(state, backing, "layer");
    is_layer &= lua_type(state, -1) == LUA_TSTRING;
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
    /* An output is a display ("virtual" is reserved) with a current mode. */
    lua_getfield(state, backing, "kind");
    gboolean is_output =
        lua_type(state, -1) == LUA_TSTRING && (g_str_equal(lua_tostring(state, -1), "display") ||
                                               g_str_equal(lua_tostring(state, -1), "virtual"));
    lua_pop(state, 1);
    lua_getfield(state, backing, "mode");
    is_output &= lua_istable(state, -1) ||
                 luaL_testudata(state, -1, LUA_READONLY_SNAPSHOT_METATABLE) != NULL;
    lua_pop(state, 1);
    /* A shortcut binding has an accelerator and a hold; an input source has a type and whether it
     * is current. */
    gboolean is_shortcut = backing_field_type(state, backing, "accelerator") == LUA_TSTRING &&
                           backing_field_type(state, backing, "hold") == LUA_TSTRING &&
                           backing_field_type(state, backing, "capture_input") == LUA_TBOOLEAN;
    lua_getfield(state, backing, "type");
    gboolean is_input_source = lua_type(state, -1) == LUA_TSTRING &&
                               (g_str_equal(lua_tostring(state, -1), "xkb") ||
                                g_str_equal(lua_tostring(state, -1), "ibus")) &&
                               backing_field_type(state, backing, "name") == LUA_TSTRING &&
                               backing_field_type(state, backing, "current") == LUA_TBOOLEAN;
    lua_pop(state, 1);
    /* An idle watch has the seconds it waits for and whether the session is idle for them. */
    gboolean is_idle_watch = backing_field_type(state, backing, "seconds") == LUA_TNUMBER &&
                             backing_field_type(state, backing, "idle") == LUA_TBOOLEAN;
    /* An override names its scope, and the setting or effect it changes. */
    lua_getfield(state, backing, "scope");
    gboolean is_override = lua_type(state, -1) == LUA_TSTRING &&
                           (g_str_equal(lua_tostring(state, -1), "settings") ||
                            g_str_equal(lua_tostring(state, -1), "window")) &&
                           backing_field_type(state, backing, "name") == LUA_TSTRING;
    lua_pop(state, 1);
    /* A keyboard grab has the idle timeout of its lease and names the device it holds. */
    lua_getfield(state, backing, "device");
    gboolean is_grab = lua_type(state, -1) == LUA_TSTRING &&
                       g_str_equal(lua_tostring(state, -1), "keyboard") &&
                       backing_field_type(state, backing, "timeout") == LUA_TNUMBER;
    lua_pop(state, 1);
    const char* kind = is_override            ? "Override"
                       : is_grab              ? "Grab"
                       : is_window            ? "Window"
                       : is_animation_preview ? "AnimationPreview"
                       : is_workspace         ? "Workspace"
                       : is_layer             ? "LayerSurface"
                       : is_output            ? "Output"
                       : is_shortcut          ? "Shortcut"
                       : is_input_source      ? "InputSource"
                       : is_idle_watch        ? "IdleWatch"
                                              : NULL;
    if (kind)
        add_record_methods(state, method_table, kind);
    if (lua_object_kind_is_live(kind))
        lua_object_add_methods(state, method_table, kind);
    return kind;
}

static int lua_readonly_pairs(lua_State* state) {
    lua_getiuservalue(state, 1, 1);
    lua_pushcclosure(state, lua_readonly_next, 1);
    lua_pushnil(state);
    lua_pushnil(state);
    return 3;
}

/* Snapshot userdata cannot be changed with ordinary assignment or rawset; nested tables use the
 * same wrapper. A window, workspace, layer surface or output record becomes the live handle for
 * its id (gnoblin-lua-objects.c): the one userdata per object, moved to the newest state. Its
 * memory block holds whether the object has ended. */
void push_readonly_copy(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index)) {
        lua_pushvalue(state, index);
        return;
    }

    gboolean* closed = lua_newuserdatauv(state, sizeof(gboolean), 2);
    *closed = FALSE;
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
    const char* kind = add_snapshot_methods(state, backing, method_table);
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
    if (lua_object_kind_is_live(kind))
        lua_object_intern(state, kind, readonly);
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
    /* These return the live object they started or changed. */
    gboolean live_result = g_str_equal(method, "shortcut.bind") ||
                           g_str_equal(method, "launch.begin") ||
                           g_str_equal(method, "input.select");
    if (!live_result &&
        (!g_str_has_prefix(method, "animation.") || !variant_is_animation_preview(value)))
        return;
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
}
