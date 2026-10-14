#include "gnoblin-lua-internal.h"

LuaRuntime* active_runtime;

LuaRuntime* pending_runtime;

LuaRuntime* deferred_runtime;

guint64 next_runtime_operation_id;

guint64 next_settings_revision;

guint64 next_runtime_generation;

gboolean revision_counters_seeded;

static char* runtime_callback_error;

GnoblinConfigRuntimeWakeupFunc runtime_wakeup_callback;

gpointer runtime_wakeup_data;

GnoblinConfigFocusPolicyChangedFunc focus_policy_changed_callback;

gpointer focus_policy_changed_data;

GnoblinConfigPermissionPolicyChangedFunc permission_policy_changed_callback;

gpointer permission_policy_changed_data;

GnoblinConfigSettingsChangedFunc settings_changed_callback;

gpointer settings_changed_data;

static int lua_focus_context_tostring(lua_State* state) {
    lua_pushliteral(state, "FocusContext");
    return 1;
}

static int lua_menu_context_tostring(lua_State* state) {
    lua_pushliteral(state, "MenuContext");
    return 1;
}

static int lua_menu_context_index(lua_State* state) {
    const char* key = lua_tostring(state, 2);
    if (key && g_str_equal(key, "begin_move"))
        lua_pushcfunction(state, lua_menu_context_begin_move);
    else if (key && g_str_equal(key, "begin_resize"))
        lua_pushcfunction(state, lua_menu_context_begin_resize);
    else
        lua_pushnil(state);
    return 1;
}

void push_menu_context(lua_State* state, guint64 handle, guint64 generation,
                       guint64 native_generation, gint64 expires_at_us, const char* window_id) {
    LuaMenuContext* context = lua_newuserdatauv(state, sizeof(*context), 0);
    *context = (LuaMenuContext){.handle = handle,
                                .generation = generation,
                                .native_generation = native_generation,
                                .expires_at_us = expires_at_us};
    g_strlcpy(context->window_id, window_id ? window_id : "", sizeof(context->window_id));
    if (luaL_newmetatable(state, LUA_MENU_CONTEXT_METATABLE)) {
        lua_pushcfunction(state, lua_menu_context_tostring);
        lua_setfield(state, -2, "__tostring");
        lua_pushcfunction(state, lua_menu_context_index);
        lua_setfield(state, -2, "__index");
        lua_pushliteral(state, "MenuContext");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

void push_focus_context(lua_State* state, guint64 handle, guint64 generation,
                        guint64 native_generation, gint64 expires_at_us) {
    LuaFocusContext* context = lua_newuserdatauv(state, sizeof(*context), 0);
    *context = (LuaFocusContext){.handle = handle,
                                 .generation = generation,
                                 .native_generation = native_generation,
                                 .expires_at_us = expires_at_us,
                                 .consumed = FALSE};
    if (luaL_newmetatable(state, LUA_FOCUS_CONTEXT_METATABLE)) {
        lua_pushcfunction(state, lua_focus_context_tostring);
        lua_setfield(state, -2, "__tostring");
        lua_pushliteral(state, "FocusContext");
        lua_setfield(state, -2, "__metatable");
    }
    lua_setmetatable(state, -2);
}

const char* api_methods[] = {
    "workspace.create",
    "workspace.rename",
    "workspace.remove",
    "workspace.switch",
    "workspace.next",
    "workspace.previous",
    "workspace.move_active",
    "workspace.move_window",
    "window.close",
    "window.kill",
    "window.minimize",
    "window.toggle_minimize",
    "window.unminimize",
    "window.restore",
    "window.restore_or_minimize",
    "window.set_maximized",
    "window.set_fullscreen",
    "window.set_above",
    "window.set_sticky",
    "window.move",
    "window.resize",
    "window.move_to_workspace",
    "window.move_to_monitor",
    "window.focus",
    "window.begin_move",
    "window.begin_resize",
    "window.match",
    "window.thumbnail",
    "layer.list",
    "monitor.list",
    "monitors.set_privacy_screen",
    "animation.list",
    "animation.get",
    "animation.surfaces",
    "animation.inspect",
    "animation.preview",
    "animation.seek",
    "animation.step",
    "animation.play",
    "animation.pause",
    "animation.stop",
    "input.list",
    "input.current",
    "input.select",
    "input.text_target",
    "input.sources",
    "input.current_source",
    "input.set_orientation_lock",
    "privacy.get",
    "privacy.stop_sharing",
    "privacy.stop_recording",
    "location.authorize_app",
    "auth.begin",
    "auth.respond",
    "auth.cancel",
    "prompt.respond",
    "prompt.cancel",
    "permissions.list",
    "permissions.policy",
    "permissions.check",
    "grant.list",
    "grant.revoke",
    "launch.status",
    "launch.begin",
    "launch.end",
    "session.lock",
    "session.activity",
    "session.status",
    "session.logout",
    "session.suspend",
    "session.hibernate",
    "session.reboot",
    "session.poweroff",
    "runtime.reload_config",
    "config.restore_default",
    "runtime.status",
    "shortcut.capture",
    "shortcut.bind",
    "shortcut.unbind",
    "shortcut.session.end",
    "command.run",
    "command.capture",
    "console.toggle",
    NULL,
};

/* Public plural collection names are aliases over the operation names that
 * the compositor currently registers. Keep the mapping here so Lua-facing
 * names can evolve without making the wire API depend on presentation. */
const ApiAlias api_aliases[] = {
    {"workspaces.create", "workspace.create"},
    {"workspaces.rename", "workspace.rename"},
    {"workspaces.remove", "workspace.remove"},
    {"workspaces.activate", "workspace.switch"},
    {"workspaces.next", "workspace.next"},
    {"workspaces.previous", "workspace.previous"},
    {"workspaces.move_active", "workspace.move_active"},
    {"workspaces.move_window", "workspace.move_window"},
    {"input.select_source", "input.select"},
    {"commands.run", "command.run"},
    {"commands.capture", "command.capture"},
    {"animations.surfaces", "animation.surfaces"},
    {"animations.inspect", "animation.inspect"},
    {"animations.preview", "animation.preview"},
    {"animations.seek", "animation.seek"},
    {"animations.step", "animation.step"},
    {"animations.play", "animation.play"},
    {"animations.pause", "animation.pause"},
    {"animations.stop", "animation.stop"},
};

const gsize api_aliases_count = G_N_ELEMENTS(api_aliases);

gboolean append_array_key(const char* key) {
    return key &&
           (!strcmp(key, "autostart") || !strcmp(key, "window-rules") ||
            !strcmp(key, "shortcuts") || !strcmp(key, "animations") || !strcmp(key, "rules") ||
            !strcmp(key, "workspaces") || !strcmp(key, "xkb-options") || !strcmp(key, "sources"));
}

static void* limited_alloc(void* opaque, void* pointer, size_t old, size_t size) {
    LuaConfig* config = opaque;
    if (!pointer)
        old = 0;
    if (!size) {
        config->allocated -= MIN(config->allocated, old);
        g_free(pointer);
        return NULL;
    }
    if (size > old && size - old > MAX_CONFIG_BYTES - config->allocated)
        return NULL;
    pointer = g_try_realloc(pointer, size);
    if (pointer)
        config->allocated = config->allocated - old + size;
    return pointer;
}

lua_State* new_config_state(LuaConfig* config) {
#if LUA_VERSION_NUM >= 505
    return lua_newstate(limited_alloc, config, g_random_int());
#else
    return lua_newstate(limited_alloc, config);
#endif
}

void limit_hook(lua_State* state, lua_Debug* debug) {
    int* remaining = (int*)lua_getextraspace(state);
    (void)debug;
    if (--*remaining <= 0)
        luaL_error(state, "configuration instruction limit exceeded");
}

void add_path(GPtrArray* paths, const char* path) {
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    for (guint i = 0; i < paths->len; i++)
        if (!strcmp(g_ptr_array_index(paths, i), canonical))
            return;
    g_ptr_array_add(paths, g_steal_pointer(&canonical));
}

char* resolve_path(const char* current, const char* given) {
    if (g_str_has_prefix(given, "~/"))
        return g_canonicalize_filename(given + 2, g_get_home_dir());
    if (g_path_is_absolute(given))
        return g_canonicalize_filename(given, NULL);
    g_autofree char* directory = g_path_get_dirname(current);
    return g_canonicalize_filename(given, directory);
}

LuaConfig* config_from_upvalue(lua_State* state) {
    return lua_touserdata(state, lua_upvalueindex(1));
}

const char* lua_error_text(lua_State* state) {
    return lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1)
                                              : "Lua configuration failed";
}

void record_runtime_callback_error(const char* scope, const char* detail) {
    g_free(runtime_callback_error);
    runtime_callback_error =
        g_strdup_printf("Lua %s was disabled after an error: %s", scope ? scope : "callback",
                        detail ? detail : "Lua configuration failed");
}

char* gnoblin_config_take_runtime_callback_error(void) {
    return g_steal_pointer(&runtime_callback_error);
}

gboolean gnoblin_config_window_pattern_match(const char* pattern, const char* value,
                                             GError** error) {
    gboolean matched = FALSE;
    g_autoptr(GError) matcher_error = NULL;
    if (!gnoblin_lua_pattern_match(pattern, value, &matched, &matcher_error)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s",
                    matcher_error ? matcher_error->message : "Lua pattern matching failed");
        return FALSE;
    }
    return matched;
}

gboolean marked_array(lua_State* state, int index) {
    gboolean result = FALSE;
    if (!lua_getmetatable(state, index))
        return FALSE;
    lua_pushstring(state, "__gnoblin_array");
    lua_rawget(state, -2);
    result = lua_toboolean(state, -1);
    lua_pop(state, 2);
    return result;
}

void mark_array_table(lua_State* state, int index) {
    index = lua_absindex(state, index);
    lua_newtable(state);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "__gnoblin_array");
    lua_setmetatable(state, index);
}
