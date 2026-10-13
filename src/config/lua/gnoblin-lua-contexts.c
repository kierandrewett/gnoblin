#include "gnoblin-lua-internal.h"

static int lua_text_target_insert_text(lua_State* state);

int lua_input_text_target(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    LuaFocusContext* context = luaL_testudata(state, 1, LUA_FOCUS_CONTEXT_METATABLE);
    if (lua_gettop(state) != 1 || !context)
        return luaL_error(state, "input.text_target requires a FocusContext from a shortcut event");
    if (!config || !active_runtime || config != &active_runtime->config || !config->dispatching ||
        config->api_calling || context->consumed || !context->handle ||
        context->generation != active_runtime->generation ||
        context->expires_at_us <= g_get_monotonic_time())
        return luaL_error(state, "FocusContext is expired, consumed, or belongs to an old runtime");
    if (config->rejecting_operations || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(
            state, "text target operation is unavailable or the event action limit was reached");
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string("input.text_target"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, request_id);
    LuaFocusOperation* operation = g_new0(LuaFocusOperation, 1);
    operation->handle = context->handle;
    operation->generation = context->generation;
    operation->native_generation = context->native_generation;
    operation->expires_at_us = context->expires_at_us;
    g_hash_table_replace(config->focus_operations, g_strdup(key), operation);
    context->consumed = TRUE;
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, "input.text_target");
    return 1;
}

static int lua_menu_context_begin(lua_State* state, gboolean resize) {
    LuaMenuContext* context = luaL_checkudata(state, 1, LUA_MENU_CONTEXT_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    /* A typed capability is one-use even if the caller supplies malformed arguments. */
    gboolean already_consumed = context->consumed;
    context->consumed = TRUE;
    gboolean malformed = lua_gettop(state) != (resize ? 2 : 1);
    const char* edge = NULL;
    if (resize && !malformed) {
        if (lua_type(state, 2) != LUA_TSTRING)
            malformed = TRUE;
        else
            edge = lua_tostring(state, 2);
    }
    if (already_consumed || !config || !active_runtime || !config->dispatching ||
        config->api_calling || context->generation != active_runtime->generation ||
        !context->native_generation || context->expires_at_us <= g_get_monotonic_time() ||
        context->handle != config->dispatch_menu_handle ||
        context->generation != config->dispatch_menu_generation ||
        context->native_generation != config->dispatch_native_menu_generation ||
        !g_str_equal(context->window_id, config->dispatch_menu_window_id))
        return luaL_error(state, "MenuContext is expired, consumed, or outside its callback scope");
    if (config->rejecting_operations || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(
            state, "MenuContext operation is unavailable or the event action limit was reached");
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    if (resize && edge)
        g_variant_builder_add(&arguments, "{sv}", "edge", g_variant_new_string(edge));
    if (malformed)
        g_variant_builder_add(&arguments, "{sv}", "_malformed", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&arguments, "{sv}", "_menu_context_handle",
                          g_variant_new_uint64(context->handle));
    g_variant_builder_add(&arguments, "{sv}", "_menu_context_generation",
                          g_variant_new_uint64(context->native_generation));
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(
        &action, "{sv}", "method",
        g_variant_new_string(resize ? "window.begin_resize" : "window.begin_move"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id,
                          resize ? "window.begin_resize" : "window.begin_move");
    return 1;
}

int lua_menu_context_begin_move(lua_State* state) {
    return lua_menu_context_begin(state, FALSE);
}

int lua_menu_context_begin_resize(lua_State* state) {
    return lua_menu_context_begin(state, TRUE);
}

int lua_windows_snap_context(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    LuaFocusContext* context = luaL_testudata(state, 1, LUA_FOCUS_CONTEXT_METATABLE);
    if (lua_gettop(state) != 1 || !context)
        return luaL_error(state, "gnoblin.windows.snap_context requires a FocusContext");
    if (!config || !active_runtime || config != &active_runtime->config || !config->dispatching ||
        config->api_calling || context->consumed || !context->handle ||
        context->generation != active_runtime->generation ||
        context->expires_at_us <= g_get_monotonic_time())
        return luaL_error(state, "FocusContext is expired, consumed, or belongs to an old runtime");
    if (config->rejecting_operations || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(
            state, "SnapContext operation is unavailable or the event action limit was reached");
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string("window.snap_context"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, request_id);
    LuaFocusOperation* operation = g_new0(LuaFocusOperation, 1);
    operation->handle = context->handle;
    operation->generation = context->generation;
    operation->native_generation = context->native_generation;
    operation->expires_at_us = context->expires_at_us;
    g_hash_table_replace(config->focus_operations, g_strdup(key), operation);
    context->consumed = TRUE;
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, "window.snap_context");
    return 1;
}

int lua_snap_context_commit(lua_State* state) {
    LuaSnapContext* context = luaL_checkudata(state, 1, LUA_SNAP_CONTEXT_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    static const char* const fields[] = {"monitor_id", "frame", NULL};
    if (lua_gettop(state) != 2 || !lua_istable(state, 2) || !table_fields_allowed(state, 2, fields))
        return luaL_error(state, "SnapContext:commit requires {monitor_id, frame}");
    if (!config || !active_runtime || context->consumed ||
        context->generation != active_runtime->generation || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "SnapContext is consumed, stale, or operations are unavailable");
    lua_getfield(state, 2, "monitor_id");
    const char* monitor_id = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
    if (!monitor_id || !*monitor_id || strlen(monitor_id) > 128 ||
        !g_utf8_validate(monitor_id, -1, NULL))
        return luaL_error(state, "SnapContext:commit requires a valid monitor_id");
    lua_pop(state, 1);
    lua_getfield(state, 2, "frame");
    static const char* const rect_fields[] = {"x", "y", "width", "height", NULL};
    if (!lua_istable(state, -1) || !table_fields_allowed(state, -1, rect_fields))
        return luaL_error(state, "SnapContext:commit frame requires x, y, width and height");
    GVariantBuilder frame;
    g_variant_builder_init(&frame, G_VARIANT_TYPE_VARDICT);
    const char* rect_keys[] = {"x", "y", "width", "height"};
    const lua_Integer lows[] = {-100000, -100000, 1, 1};
    const lua_Integer highs[] = {100000, 100000, 32768, 32768};
    for (guint i = 0; i < G_N_ELEMENTS(rect_keys); i++) {
        lua_getfield(state, -1, rect_keys[i]);
        if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < lows[i] ||
            lua_tointeger(state, -1) > highs[i])
            return luaL_error(state, "SnapContext frame %s is out of range", rect_keys[i]);
        g_variant_builder_add(&frame, "{sv}", rect_keys[i],
                              g_variant_new_int32((gint32)lua_tointeger(state, -1)));
        lua_pop(state, 1);
    }
    lua_pop(state, 1);
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "context", g_variant_new_string(context->token));
    g_variant_builder_add(&arguments, "{sv}", "monitor_id", g_variant_new_string(monitor_id));
    g_variant_builder_add(&arguments, "{sv}", "frame", g_variant_builder_end(&frame));
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string("window.snap"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    context->consumed = TRUE;
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, "window.snap");
    return 1;
}

static gboolean lua_snap_rect(lua_State* state, int index, GVariantBuilder* rect) {
    static const char* const fields[] = {"x", "y", "width", "height", NULL};
    static const char* const keys[] = {"x", "y", "width", "height"};
    static const lua_Integer lows[] = {-100000, -100000, 1, 1};
    static const lua_Integer highs[] = {100000, 100000, 32768, 32768};
    if (!lua_istable(state, index) || !table_fields_allowed(state, index, fields))
        return FALSE;
    index = lua_absindex(state, index);
    g_variant_builder_init(rect, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < G_N_ELEMENTS(keys); i++) {
        lua_getfield(state, index, keys[i]);
        if (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < lows[i] ||
            lua_tointeger(state, -1) > highs[i]) {
            lua_pop(state, 1);
            g_variant_builder_clear(rect);
            return FALSE;
        }
        g_variant_builder_add(rect, "{sv}", keys[i],
                              g_variant_new_int32((gint32)lua_tointeger(state, -1)));
        lua_pop(state, 1);
    }
    return TRUE;
}

gboolean lua_dense_array(lua_State* state, int index, lua_Integer maximum) {
    index = lua_absindex(state, index);
    lua_Integer length = lua_rawlen(state, index);
    if (length > maximum)
        return FALSE;
    lua_Integer entries = 0;
    lua_pushnil(state);
    while (lua_next(state, index)) {
        gboolean valid_key = lua_isinteger(state, -2);
        lua_Integer key = valid_key ? lua_tointeger(state, -2) : 0;
        if (!valid_key || key < 1 || key > length) {
            lua_pop(state, 2);
            return FALSE;
        }
        entries++;
        lua_pop(state, 1);
    }
    return entries == length;
}

int lua_window_drag_offer_targets(lua_State* state) {
    LuaWindowDrag* drag = luaL_checkudata(state, 1, LUA_WINDOW_DRAG_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    if (lua_gettop(state) != 2 || !lua_istable(state, 2) || lua_rawlen(state, 2) < 1 ||
        !lua_dense_array(state, 2, 128))
        return luaL_error(state, "WindowDrag:offer_targets requires 1-128 targets");
    if (!config || !active_runtime || !config->dispatching || config->api_calling ||
        config->rejecting_operations || !config->runtime_actions || drag->id == 0 ||
        drag->generation != active_runtime->generation || drag->ended ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "WindowDrag is stale or offers are unavailable");
    int targets_index = lua_absindex(state, 2);
    GVariantBuilder targets;
    g_variant_builder_init(&targets, G_VARIANT_TYPE("av"));
    g_autoptr(GHashTable) ids = g_hash_table_new(g_str_hash, g_str_equal);
    lua_Integer count = lua_rawlen(state, targets_index);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, targets_index, i);
        int target_index = lua_absindex(state, -1);
        static const char* const target_fields[] = {
            "id", "hit", "frame", "maximize", "required_modifiers", "forbidden_modifiers", NULL};
        if (!lua_istable(state, target_index) ||
            !table_fields_allowed(state, target_index, target_fields))
            return luaL_error(state, "snap target %d has unsupported fields", (int)i);
        lua_getfield(state, target_index, "id");
        const char* id = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        if (!id || !*id || strlen(id) > 64 || !g_utf8_validate(id, -1, NULL) ||
            g_hash_table_contains(ids, id))
            return luaL_error(state, "snap target %d needs a unique nonempty ID up to 64 bytes",
                              (int)i);
        g_hash_table_add(ids, (gpointer)id);
        GVariantBuilder record, hit, frame;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
        lua_pop(state, 1);
        lua_getfield(state, target_index, "hit");
        if (!lua_snap_rect(state, -1, &hit))
            return luaL_error(state, "snap target %d hit rectangle is invalid", (int)i);
        g_variant_builder_add(&record, "{sv}", "hit", g_variant_builder_end(&hit));
        lua_pop(state, 1);
        lua_getfield(state, target_index, "frame");
        if (!lua_snap_rect(state, -1, &frame))
            return luaL_error(state, "snap target %d frame rectangle is invalid", (int)i);
        g_variant_builder_add(&record, "{sv}", "frame", g_variant_builder_end(&frame));
        lua_pop(state, 1);
        lua_getfield(state, target_index, "maximize");
        if (!lua_isnil(state, -1)) {
            if (!lua_isboolean(state, -1))
                return luaL_error(state, "snap target maximize must be boolean");
            g_variant_builder_add(&record, "{sv}", "maximize",
                                  g_variant_new_boolean(lua_toboolean(state, -1)));
        }
        lua_pop(state, 1);
        const char* modifier_fields[] = {"required_modifiers", "forbidden_modifiers"};
        for (guint m = 0; m < G_N_ELEMENTS(modifier_fields); m++) {
            lua_getfield(state, target_index, modifier_fields[m]);
            if (!lua_isnil(state, -1)) {
                if (!lua_istable(state, -1) || !lua_dense_array(state, -1, 1))
                    return luaL_error(state, "snap target modifier arrays accept only control");
                GVariantBuilder list;
                g_variant_builder_init(&list, G_VARIANT_TYPE_STRING_ARRAY);
                for (lua_Integer j = 1; j <= lua_rawlen(state, -1); j++) {
                    lua_rawgeti(state, -1, j);
                    if (lua_type(state, -1) != LUA_TSTRING ||
                        !g_str_equal(lua_tostring(state, -1), "control"))
                        return luaL_error(state, "snap target modifier must be 'control'");
                    g_variant_builder_add(&list, "s", "control");
                    lua_pop(state, 1);
                }
                g_variant_builder_add(&record, "{sv}", modifier_fields[m],
                                      g_variant_builder_end(&list));
            }
            lua_pop(state, 1);
        }
        g_variant_builder_add_value(&targets,
                                    g_variant_new_variant(g_variant_builder_end(&record)));
        lua_pop(state, 1);
    }
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "drag_id", g_variant_new_uint64(drag->id));
    g_variant_builder_add(&arguments, "{sv}", "settings_revision",
                          g_variant_new_uint64(drag->settings_revision));
    g_variant_builder_add(&arguments, "{sv}", "targets", g_variant_builder_end(&targets));
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string("window.snap.offer"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, "window.snap.offer");
    return 1;
}

static int lua_text_target_insert_text(lua_State* state) {
    LuaTextTarget* target = luaL_checkudata(state, 1, LUA_TEXT_TARGET_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    size_t length = 0;
    const char* text = luaL_checklstring(state, 2, &length);
    if (lua_gettop(state) != 2)
        return luaL_error(state, "TextTarget:insert_text takes exactly one string");
    gboolean valid = length > 0 && length <= 256 && !memchr(text, '\0', length) &&
                     g_utf8_validate(text, length, NULL);
    if (valid) {
        const char* cursor = text;
        const char* end = text + length;
        while (cursor < end) {
            if (g_unichar_iscntrl(g_utf8_get_char(cursor))) {
                valid = FALSE;
                break;
            }
            cursor = g_utf8_next_char(cursor);
        }
    }
    gboolean usable = !target->consumed && target->token[0] && target->generation &&
                      active_runtime && target->generation == active_runtime->generation;
    target->consumed = TRUE;
    if (!usable)
        return luaL_error(state, "TextTarget is consumed or belongs to an old runtime");
    if (!valid)
        return luaL_error(state,
                          "inserted text must be 1-256 bytes of UTF-8 without control characters");
    if (!config || !config->dispatching || config->api_calling || config->rejecting_operations ||
        !config->runtime_actions)
        return luaL_error(state,
                          "TextTarget:insert_text is available only in a runtime event callback");
    if (config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "too many pending Gnoblin session actions");
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&arguments, "{sv}", "target", g_variant_new_string(target->token));
    g_variant_builder_add(&arguments, "{sv}", "text", g_variant_new_string(text));
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string("input.insert_text"));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, "input.insert_text");
    return 1;
}

int lua_text_target_index(lua_State* state) {
    LuaTextTarget* target = luaL_checkudata(state, 1, LUA_TEXT_TARGET_METATABLE);
    const char* field = luaL_checkstring(state, 2);
    if (g_str_equal(field, "insert_text")) {
        lua_pushcfunction(state, lua_text_target_insert_text);
    } else if (g_str_equal(field, "window_id") && target->window_id[0]) {
        lua_pushstring(state, target->window_id);
    } else if (g_str_equal(field, "caret") && target->has_caret) {
        lua_createtable(state, 0, 4);
        lua_pushnumber(state, target->x);
        lua_setfield(state, -2, "x");
        lua_pushnumber(state, target->y);
        lua_setfield(state, -2, "y");
        lua_pushnumber(state, target->width);
        lua_setfield(state, -2, "width");
        lua_pushnumber(state, target->height);
        lua_setfield(state, -2, "height");
    } else {
        lua_pushnil(state);
    }
    return 1;
}

int lua_animation_preview_method(lua_State* state) {
    const char* method = lua_tostring(state, lua_upvalueindex(1));
    int supplied = lua_gettop(state) - 1;
    if (lua_gettop(state) < 1 || !lua_isuserdata(state, 1))
        return luaL_error(state, "animation preview methods must be called on a preview record");

    lua_getiuservalue(state, 1, 1);
    if (!lua_istable(state, -1))
        return luaL_error(state, "animation preview record is unavailable");
    lua_getfield(state, -1, "session");
    if (lua_type(state, -1) != LUA_TSTRING)
        return luaL_error(state, "animation preview record has no session ID");

    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    lua_pushvalue(state, -2);
    lua_setfield(state, arguments, "session");

    if (g_str_equal(method, "animation.seek")) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TNUMBER)
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        double progress = lua_tonumber(state, 2);
        if (!isfinite(progress) || progress < 0.0 || progress > 1.0)
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        lua_pushnumber(state, progress);
        lua_setfield(state, arguments, "progress");
    } else if (g_str_equal(method, "animation.step")) {
        if (supplied != 1 || !lua_isinteger(state, 2) || lua_tointeger(state, 2) < 1 ||
            lua_tointeger(state, 2) > 60000)
            return luaL_error(state, "preview:step requires an integer from 1 to 60000");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "milliseconds");
    } else if (g_str_equal(method, "animation.play") || g_str_equal(method, "animation.pause") ||
               g_str_equal(method, "animation.stop")) {
        if (supplied != 0) {
            const char* operation = method + strlen("animation.");
            return luaL_error(state, "preview:%s takes no arguments", operation);
        }
    } else {
        return luaL_error(state, "unsupported animation preview method");
    }

    lua_pushvalue(state, arguments);
    lua_replace(state, 1);
    lua_settop(state, 1);
    lua_pushlightuserdata(state, active_runtime ? &active_runtime->config : NULL);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, 1);
    lua_call(state, 1, 1);
    return 1;
}
