#include "gnoblin-lua-internal.h"

/* Grant: the proof that a user gesture just happened, scoped to the operations it may start.
 *
 * The compositor mints one proof per gesture (a key press that matched a shortcut, a pointer
 * binding, a click an input handler saw, a window menu request). A script never holds the proof
 * itself, only a Grant that names it. A Grant lists the operations it may take and may name one
 * window. `narrow` returns a Grant with a subset of both; nothing returns a wider one. Each
 * operation of a gesture can run once, whichever Grant takes it, so one key press can focus a
 * window and then begin a move. Timers and events that are not gestures carry no Grant. */

static const struct {
    const char* name;
    LuaGrantScope scope;
} grant_scopes[] = {
    {"focus", LUA_GRANT_FOCUS},
    {"begin_move", LUA_GRANT_BEGIN_MOVE},
    {"begin_resize", LUA_GRANT_BEGIN_RESIZE},
    {"snap_context", LUA_GRANT_SNAP_CONTEXT},
    {"text_target", LUA_GRANT_TEXT_TARGET},
};

static const char* grant_scope_name(LuaGrantScope scope) {
    for (guint i = 0; i < G_N_ELEMENTS(grant_scopes); i++)
        if (grant_scopes[i].scope == scope)
            return grant_scopes[i].name;
    return "operation";
}

static int lua_grant_tostring(lua_State* state) {
    lua_pushliteral(state, "Grant");
    return 1;
}

static int lua_grant_newindex(lua_State* state) {
    return luaL_error(state, "Grant values are read-only");
}

static void new_grant(lua_State* state, const LuaGrant* grant) {
    LuaGrant* copy = lua_newuserdatauv(state, sizeof(*copy), 0);
    *copy = *grant;
    if (luaL_newmetatable(state, LUA_GRANT_METATABLE)) {
        lua_pushcfunction(state, lua_grant_tostring);
        lua_setfield(state, -2, "__tostring");
        lua_pushcfunction(state, lua_grant_index);
        lua_setfield(state, -2, "__index");
        lua_pushcfunction(state, lua_grant_newindex);
        lua_setfield(state, -2, "__newindex");
        lua_pushliteral(state, "Grant");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

void push_grant(lua_State* state, guint64 handle, guint64 generation, guint64 native_generation,
                gint64 expires_at_us, gboolean menu, const char* window_id) {
    LuaGrant grant = {.handle = handle,
                      .generation = generation,
                      .native_generation = native_generation,
                      .expires_at_us = expires_at_us,
                      .scope =
                          menu ? (LUA_GRANT_BEGIN_MOVE | LUA_GRANT_BEGIN_RESIZE) : LUA_GRANT_ALL,
                      .menu = menu};
    g_strlcpy(grant.window_id, window_id ? window_id : "", sizeof(grant.window_id));
    new_grant(state, &grant);
}

static guint64 grant_spent_key(const LuaGrant* grant) {
    return grant->handle | (grant->menu ? G_GUINT64_CONSTANT(1) << 63 : 0);
}

/* What the gesture behind `grant` has spent. */
static guint grant_used(LuaConfig* config, const LuaGrant* grant) {
    if (!config || !config->grant_spent)
        return 0;
    guint64 key = grant_spent_key(grant);
    LuaGrantSpent* spent = g_hash_table_lookup(config->grant_spent, &key);
    return spent ? spent->used : 0;
}

static gboolean grant_is_live(const LuaGrant* grant) {
    return active_runtime && grant->handle && grant->generation == active_runtime->generation &&
           grant->native_generation && grant->expires_at_us > g_get_monotonic_time();
}

static LuaGrantSpent* grant_spent_for_update(LuaConfig* config, const LuaGrant* grant) {
    guint64 key = grant_spent_key(grant);
    LuaGrantSpent* spent = g_hash_table_lookup(config->grant_spent, &key);
    if (spent)
        return spent;
    /* A gesture lives five seconds: forget the ones that ended. */
    GHashTableIter iter;
    gpointer value;
    gint64 now = g_get_monotonic_time();
    g_hash_table_iter_init(&iter, config->grant_spent);
    while (g_hash_table_iter_next(&iter, NULL, &value))
        if (((LuaGrantSpent*)value)->expires_at_us <= now)
            g_hash_table_iter_remove(&iter);
    spent = g_new0(LuaGrantSpent, 1);
    spent->expires_at_us = grant->expires_at_us;
    guint64* key_copy = g_new(guint64, 1);
    *key_copy = key;
    g_hash_table_insert(config->grant_spent, key_copy, spent);
    return spent;
}

static void lua_grant_take(lua_State* state, LuaConfig* config, int index, LuaGrantScope scope,
                           const char* method, const char* window_id, LuaFocusOperation* out) {
    LuaGrant* grant = luaL_testudata(state, index, LUA_GRANT_METATABLE);
    if (!grant) {
        luaL_error(state, "%s requires a Grant from a shortcut, pointer or window menu event",
                   method);
        return;
    }
    if (!config || !active_runtime || config != &active_runtime->config || !config->dispatching ||
        config->api_calling) {
        luaL_error(state, "%s is available only in a runtime event callback", method);
        return;
    }
    if (!grant_is_live(grant)) {
        luaL_error(state, "%s: the Grant is expired or belongs to an old runtime", method);
        return;
    }
    /* A menu request is a Grant only inside the callback that received it. */
    if (grant->menu && grant->handle != config->dispatch_menu_handle) {
        luaL_error(state, "%s: a window menu Grant works only in the event callback that got it",
                   method);
        return;
    }
    if (!(grant->scope & scope)) {
        luaL_error(state, "%s: this Grant has no %s scope", method, grant_scope_name(scope));
        return;
    }
    if (grant->window_id[0] && window_id && !g_str_equal(grant->window_id, window_id)) {
        luaL_error(state, "%s: this Grant is narrowed to window %s, not window %s", method,
                   grant->window_id, window_id);
        return;
    }
    LuaGrantSpent* spent = grant_spent_for_update(config, grant);
    if (spent->used & scope) {
        luaL_error(state, "%s: this gesture already used its %s scope", method,
                   grant_scope_name(scope));
        return;
    }
    /* A menu request offers one move or one resize, whichever comes first. */
    spent->used |= grant->menu ? LUA_GRANT_ALL : scope;
    *out = (LuaFocusOperation){.handle = grant->handle,
                               .generation = grant->generation,
                               .native_generation = grant->native_generation,
                               .expires_at_us = grant->expires_at_us};
    g_strlcpy(out->window_id, grant->window_id, sizeof(out->window_id));
}

/* Queue `method` for the Grant at `index` and push its Operation. `window_id` names the window the
 * operation acts on (NULL when the compositor picks it) and `edge` is the resize edge. */
static int lua_grant_operation(lua_State* state, LuaConfig* config, int index, LuaGrantScope scope,
                               const char* method, const char* window_id, const char* edge) {
    if (!config || config->rejecting_operations || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "%s is unavailable or the event action limit was reached", method);
    LuaFocusOperation operation;
    lua_grant_take(state, config, index, scope, method, window_id, &operation);
    LuaGrant* grant = lua_touserdata(state, index);
    guint64 request_id = allocate_operation_id();
    if (!request_id)
        return luaL_error(state, "Gnoblin operation request ID space is exhausted");
    GVariantBuilder arguments, action;
    g_variant_builder_init(&arguments, G_VARIANT_TYPE_VARDICT);
    if (grant->menu) {
        /* The compositor holds the window of a menu request; the call names only the grant. */
        g_variant_builder_add(&arguments, "{sv}", "_menu_context_handle",
                              g_variant_new_uint64(grant->handle));
        g_variant_builder_add(&arguments, "{sv}", "_menu_context_generation",
                              g_variant_new_uint64(grant->native_generation));
    } else if (window_id) {
        g_variant_builder_add(&arguments, "{sv}", "id", g_variant_new_string(window_id));
    }
    if (edge)
        g_variant_builder_add(&arguments, "{sv}", "edge", g_variant_new_string(edge));
    g_variant_builder_init(&action, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&action, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&action, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&action, "{sv}", "arguments", g_variant_builder_end(&arguments));
    g_ptr_array_add(config->runtime_actions, g_variant_ref_sink(g_variant_builder_end(&action)));
    if (!grant->menu) {
        g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, request_id);
        LuaFocusOperation* stored = g_memdup2(&operation, sizeof(operation));
        g_hash_table_replace(config->focus_operations, g_strdup(key), stored);
    }
    config->actions_in_dispatch++;
    push_operation_handle(state, config, request_id, method);
    return 1;
}

static gboolean grant_window_argument(lua_State* state, int index, char* out) {
    if (lua_isnoneornil(state, index))
        return TRUE;
    const char* id = NULL;
    if (lua_type(state, index) == LUA_TSTRING) {
        id = lua_tostring(state, index);
    } else if (lua_type(state, index) == LUA_TTABLE || lua_type(state, index) == LUA_TUSERDATA) {
        lua_getfield(state, index, "id");
        id = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_pop(state, 1);
    }
    if (!id || !*id || strlen(id) >= GNOBLIN_GRANT_WINDOW_ID_SIZE)
        return FALSE;
    g_strlcpy(out, id, GNOBLIN_GRANT_WINDOW_ID_SIZE);
    return TRUE;
}

/* grant:narrow(scope, window?): a Grant for fewer operations and, optionally, one window. */
static int lua_grant_narrow(lua_State* state) {
    LuaGrant* grant = luaL_checkudata(state, 1, LUA_GRANT_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    if (lua_gettop(state) < 2 || lua_gettop(state) > 3)
        return luaL_error(state, "Grant:narrow takes operations and an optional window");
    guint requested = 0;
    int count = 0;
    if (lua_type(state, 2) == LUA_TSTRING) {
        lua_pushvalue(state, 2);
        count = 1;
    } else if (lua_type(state, 2) == LUA_TTABLE &&
               lua_dense_array(state, 2, G_N_ELEMENTS(grant_scopes)) && lua_rawlen(state, 2) > 0) {
        count = (int)lua_rawlen(state, 2);
        for (int i = 1; i <= count; i++)
            lua_rawgeti(state, 2, i);
    } else {
        return luaL_error(state, "Grant:narrow needs an operation name or a list of names");
    }
    for (int i = 0; i < count; i++) {
        const char* name =
            lua_type(state, -count + i) == LUA_TSTRING ? lua_tostring(state, -count + i) : NULL;
        guint bit = 0;
        for (guint s = 0; name && s < G_N_ELEMENTS(grant_scopes); s++)
            if (g_str_equal(name, grant_scopes[s].name))
                bit = grant_scopes[s].scope;
        if (!bit)
            return luaL_error(state,
                              "Grant:narrow: %s is not an operation (focus, begin_move, "
                              "begin_resize, snap_context, text_target)",
                              name ? name : "a non-string");
        requested |= bit;
    }
    lua_pop(state, count);
    if (!grant_is_live(grant))
        return luaL_error(state, "Grant:narrow: the Grant is expired or belongs to an old runtime");
    guint used = grant_used(config, grant);
    for (guint s = 0; s < G_N_ELEMENTS(grant_scopes); s++) {
        guint bit = grant_scopes[s].scope;
        if (!(requested & bit))
            continue;
        if (!(grant->scope & bit))
            return luaL_error(state, "Grant:narrow cannot widen: this Grant has no %s scope",
                              grant_scopes[s].name);
        if (used & bit)
            return luaL_error(state, "Grant:narrow: this gesture already used its %s scope",
                              grant_scopes[s].name);
    }
    LuaGrant narrowed = *grant;
    narrowed.scope = requested;
    char window_id[GNOBLIN_GRANT_WINDOW_ID_SIZE] = {0};
    if (!grant_window_argument(state, 3, window_id))
        return luaL_error(state, "Grant:narrow needs a Window or a window ID for its window");
    if (window_id[0]) {
        if (grant->window_id[0] && !g_str_equal(grant->window_id, window_id))
            return luaL_error(state, "Grant:narrow cannot widen: this Grant is for window %s",
                              grant->window_id);
        g_strlcpy(narrowed.window_id, window_id, sizeof(narrowed.window_id));
    }
    new_grant(state, &narrowed);
    return 1;
}

/* grant.scope, window, expires_at_us, alive and :narrow(). */
int lua_grant_index(lua_State* state) {
    LuaGrant* grant = luaL_checkudata(state, 1, LUA_GRANT_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    const char* key = lua_type(state, 2) == LUA_TSTRING ? lua_tostring(state, 2) : NULL;
    if (!key) {
        lua_pushnil(state);
    } else if (g_str_equal(key, "narrow")) {
        lua_pushcfunction(state, lua_grant_narrow);
    } else if (g_str_equal(key, "scope")) {
        guint remaining = grant_is_live(grant) ? grant->scope & ~grant_used(config, grant) : 0;
        lua_newtable(state);
        mark_array_table(state, -1);
        lua_Integer next = 1;
        for (guint s = 0; s < G_N_ELEMENTS(grant_scopes); s++) {
            if (!(remaining & grant_scopes[s].scope))
                continue;
            lua_pushstring(state, grant_scopes[s].name);
            lua_rawseti(state, -2, next++);
        }
    } else if (g_str_equal(key, "window")) {
        if (!grant->window_id[0] || !config ||
            !lua_push_window_by_id(state, config, grant->window_id))
            lua_pushnil(state);
    } else if (g_str_equal(key, "expires_at_us")) {
        lua_pushinteger(state, grant->expires_at_us);
    } else if (g_str_equal(key, "alive")) {
        lua_pushboolean(state,
                        grant_is_live(grant) && (grant->scope & ~grant_used(config, grant)) != 0);
    } else {
        lua_pushnil(state);
    }
    return 1;
}

int lua_input_text_target(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1)
        return luaL_error(state, "input.text_target takes one Grant");
    return lua_grant_operation(state, config, 1, LUA_GRANT_TEXT_TARGET, "input.text_target", NULL,
                               NULL);
}

int lua_windows_snap_context(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1)
        return luaL_error(state, "gnoblin.windows.snap_context takes one Grant");
    return lua_grant_operation(state, config, 1, LUA_GRANT_SNAP_CONTEXT, "window.snap_context",
                               NULL, NULL);
}

/* window:focus(grant), :begin_move(grant) and :begin_resize(edge, grant), called from a record
 * method with the Window's ID. */
int lua_window_grant_operation(lua_State* state, LuaConfig* config, const char* method,
                               const char* window_id, int grant_index, const char* edge) {
    LuaGrantScope scope =
        gnoblin_api_operation_is(method, GNOBLIN_OP_WINDOW_FOCUS)        ? LUA_GRANT_FOCUS
        : gnoblin_api_operation_is(method, GNOBLIN_OP_WINDOW_BEGIN_MOVE) ? LUA_GRANT_BEGIN_MOVE
                                                                         : LUA_GRANT_BEGIN_RESIZE;
    return lua_grant_operation(state, config, grant_index, scope, method, window_id, edge);
}

int lua_snap_context_commit(lua_State* state) {
    LuaSnapContext* context = luaL_checkudata(state, 1, LUA_SNAP_CONTEXT_METATABLE);
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    if (lua_gettop(state) != 6)
        return luaL_error(state,
                          "SnapContext:commit takes a monitor (an Output or its ID), x, y, width "
                          "and height");
    if (!config || !active_runtime || context->consumed ||
        context->generation != active_runtime->generation || !config->runtime_actions ||
        config->runtime_actions->len >= 256 || config->actions_in_dispatch >= 64)
        return luaL_error(state, "SnapContext is consumed, stale, or operations are unavailable");
    if (!lua_push_monitor_id(state, 2))
        return luaL_error(state, "SnapContext:commit takes an Output or a monitor ID");
    const char* monitor_id = lua_tostring(state, -1);
    if (!monitor_id || !*monitor_id || strlen(monitor_id) > 128 ||
        !g_utf8_validate(monitor_id, -1, NULL))
        return luaL_error(state, "SnapContext:commit requires a valid monitor ID");
    GVariantBuilder frame;
    g_variant_builder_init(&frame, G_VARIANT_TYPE_VARDICT);
    const char* rect_keys[] = {"x", "y", "width", "height"};
    const lua_Integer lows[] = {-100000, -100000, 1, 1};
    const lua_Integer highs[] = {100000, 100000, 32768, 32768};
    for (guint i = 0; i < G_N_ELEMENTS(rect_keys); i++) {
        int argument = 3 + (int)i;
        if (!lua_isinteger(state, argument) || lua_tointeger(state, argument) < lows[i] ||
            lua_tointeger(state, argument) > highs[i]) {
            g_variant_builder_clear(&frame);
            return luaL_error(state, "SnapContext %s must be an integer from %d to %d",
                              rect_keys[i], (int)lows[i], (int)highs[i]);
        }
        g_variant_builder_add(&frame, "{sv}", rect_keys[i],
                              g_variant_new_int32((gint32)lua_tointeger(state, argument)));
    }
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
    lua_getfield(state, -1, "id");
    if (lua_type(state, -1) != LUA_TSTRING)
        return luaL_error(state, "animation preview record has no ID");

    lua_newtable(state);
    int arguments = lua_absindex(state, -1);
    lua_pushvalue(state, -2);
    lua_setfield(state, arguments, "session"); /* the wire call names the preview "session" */

    const GnoblinApiOperation* operation_row = gnoblin_api_operation_find(method);
    GnoblinApiOp id = operation_row ? operation_row->id : GNOBLIN_OP_COUNT;
    if (id == GNOBLIN_OP_ANIMATION_SEEK) {
        if (supplied != 1 || lua_type(state, 2) != LUA_TNUMBER)
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        double progress = lua_tonumber(state, 2);
        if (!isfinite(progress) || progress < 0.0 || progress > 1.0)
            return luaL_error(state, "preview:seek requires a number from 0 to 1");
        lua_pushnumber(state, progress);
        lua_setfield(state, arguments, "progress");
    } else if (id == GNOBLIN_OP_ANIMATION_STEP) {
        if (supplied != 1 || !lua_isinteger(state, 2) || lua_tointeger(state, 2) < 1 ||
            lua_tointeger(state, 2) > 60000)
            return luaL_error(state, "preview:step requires an integer from 1 to 60000");
        lua_pushvalue(state, 2);
        lua_setfield(state, arguments, "milliseconds");
    } else if (id == GNOBLIN_OP_ANIMATION_PLAY || id == GNOBLIN_OP_ANIMATION_PAUSE ||
               id == GNOBLIN_OP_ANIMATION_STOP) {
        if (supplied != 0) {
            const char* operation =
                method + strlen(gnoblin_api_operation_name(GNOBLIN_OP_ANIMATION_FAMILY));
            return luaL_error(state, "preview:%s takes no arguments", operation);
        }
    } else {
        return luaL_error(state, "unsupported animation preview method");
    }

    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_pushvalue(state, arguments);
    lua_call(state, 1, 1);
    /* The preview ends when the compositor has stopped it, or when a call on it fails because it
     * is gone. */
    if (config)
        lua_object_end_after(state, config, lua_gettop(state), "AnimationPreview", 1,
                             id == GNOBLIN_OP_ANIMATION_STOP);
    return 1;
}
