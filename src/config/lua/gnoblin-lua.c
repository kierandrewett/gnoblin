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

GnoblinConfigSettingsChangedFunc settings_changed_callback;

gpointer settings_changed_data;

/* Every function the Lua API installs. `path` is where Lua finds it: a dotted path under the
 * gnoblin global, or Class:method for a method on a snapshot record. install_api, the record
 * methods, gnoblin_config_call_api and scripts/audit-lua-annotations.py all read this table.
 *
 * An entry without a `function` is an operation: Lua sends `operation` to the compositor through
 * lua_generic_api_action (lua_workspace_action for workspace.*), or through lua_record_method on
 * a record. An entry with a `function` runs in the runtime. Its `operation`, when set, is the
 * method name socket clients use for it; a record method's `operation` is its wire name.
 *
 * Socket clients reach an entry with an `operation` by that name or by its gnoblin path. */
const LuaApiEntry lua_api[] = {
    {"version", NULL, lua_gnoblin_version},
    {"config.path", NULL, lua_config_path},
    {"config.default", NULL, lua_config_default},
    {"config.restore_default", "config.restore_default", NULL},
    {"load", NULL, lua_config_load},
    {"array", NULL, lua_array},
    {"snapshot", NULL, lua_snapshot},
    {"timers.after", NULL, lua_timers_after},
    {"timers.now", NULL, lua_timers_now},
    {"ease", NULL, lua_ease},
    {"lerp", NULL, lua_lerp},
    {"tween", NULL, lua_tween_new},

    {"process.spawn", NULL, lua_process_spawn},
    {"on", NULL, lua_on},
    {"once", NULL, lua_once},

    {"windows.list", NULL, lua_windows_list},
    {"windows.focused", NULL, lua_windows_focused},
    {"windows.by_id", NULL, lua_windows_by_id},
    {"windows.snap_context", NULL, lua_windows_snap_context},
    {"Window:close", "window.close", NULL},
    {"Window:kill", "window.kill", NULL},
    {"Window:raise", "window.raise", NULL},
    {"Window:lower", "window.lower", NULL},
    {"Window:set_minimized", "window.set_minimized", NULL},
    {"Window:restore_or_minimize", "window.restore_or_minimize", NULL},
    {"Window:set_maximized", "window.set_maximized", NULL},
    {"Window:set_fullscreen", "window.set_fullscreen", NULL},
    {"Window:set_above", "window.set_above", NULL},
    {"Window:set_sticky", "window.set_sticky", NULL},
    {"Window:move", "window.move", NULL},
    {"Window:resize", "window.resize", NULL},
    {"Window:move_to_workspace", "window.move_to_workspace", NULL},
    {"Window:move_to_monitor", "window.move_to_monitor", NULL},
    {"Window:focus", "window.focus", NULL},
    {"Window:begin_move", "window.begin_move", NULL},
    {"Window:begin_resize", "window.begin_resize", NULL},
    {"Window:thumbnail", "window.thumbnail", NULL},
    {"Window:present", "window.present", NULL},
    {"Window:set_opacity", "window.set_visual", NULL},
    {"Window:set_scale", "window.set_visual", NULL},
    {"Window:set_rotation", "window.set_visual", NULL},
    {"Window:set_offset", "window.set_visual", NULL},
    {"Window:reset_visuals", "window.set_visual", NULL},
    {"Window:set_effect", "window.set_effect", NULL},
    {"Window:on", NULL, lua_object_on},
    {"Window:watch", NULL, lua_object_watch},

    {"workspaces.list", NULL, lua_workspaces_list},
    {"workspaces.active", NULL, lua_workspaces_active},
    {"workspaces.by_id", NULL, lua_workspaces_by_id},
    {"workspaces.create", "workspace.create", NULL, TRUE},
    {"workspaces.next", "workspace.next", NULL},
    {"workspaces.previous", "workspace.previous", NULL},
    {"Workspace:activate", "workspace.switch", NULL},
    {"Workspace:rename", "workspace.rename", NULL},
    {"Workspace:remove", "workspace.remove", NULL},
    {"Workspace:on", NULL, lua_object_on},
    {"Workspace:watch", NULL, lua_object_watch},

    {"outputs.list", NULL, lua_outputs_list},
    {"outputs.primary", NULL, lua_outputs_primary},
    {"outputs.by_id", NULL, lua_outputs_by_id},
    {"outputs.renderer", NULL, lua_outputs_renderer},
    {"Output:on", NULL, lua_object_on},
    {"Output:watch", NULL, lua_object_watch},
    {"layers.list", NULL, lua_layers_list},
    {"layers.animation_policy", NULL, lua_layers_animation_policy},
    {"LayerSurface:on", NULL, lua_object_on},
    {"LayerSurface:watch", NULL, lua_object_watch},
    {"capabilities.list", NULL, lua_capabilities_list},
    {"appearance.color_scheme", NULL, lua_appearance_color_scheme},

    {"animations.list", NULL, lua_animations_list},
    {"animations.get", NULL, lua_animation_get},
    {"animations.surfaces", "animation.surfaces", NULL},
    {"animations.inspect", "animation.inspect", NULL, TRUE},
    {"animations.preview", "animation.preview", NULL, TRUE},
    {"AnimationPreview:seek", "animation.seek", lua_animation_preview_method},
    {"AnimationPreview:step", "animation.step", lua_animation_preview_method},
    {"AnimationPreview:play", "animation.play", lua_animation_preview_method},
    {"AnimationPreview:pause", "animation.pause", lua_animation_preview_method},
    {"AnimationPreview:stop", "animation.stop", lua_animation_preview_method},

    {"input.devices", NULL, lua_input_devices},
    {"input.sources", "input.sources", lua_input_sources},
    {"input.current_source", "input.current_source", lua_input_current_source},
    {"InputSource:select", "input.select", NULL},
    {"InputSource:on", NULL, lua_object_on},
    {"InputSource:watch", NULL, lua_object_watch},
    {"input.text_target", "input.text_target", lua_input_text_target},
    {"input.orientation_lock", NULL, lua_input_orientation_lock},
    {"input.on", NULL, lua_input_on},
    {"input.grab", NULL, lua_input_grab, TRUE},
    {"Grab:renew", "input.grab.renew", NULL},
    {"Grab:release", "input.grab.release", NULL},
    {"Grab:on", NULL, lua_object_on},

    {"shortcuts.actions", NULL, lua_shortcut_actions},
    {"shortcuts.configured", NULL, lua_shortcuts_configured},
    {"shortcuts.list", NULL, lua_shortcuts_list},
    {"shortcuts.capture", "shortcut.capture", NULL},
    {"shortcuts.bind", NULL, lua_shortcuts_bind, TRUE},
    {"Shortcut:unbind", "shortcut.unbind", NULL},
    {"Shortcut:end_session", "shortcut.session.end", NULL},
    {"Shortcut:on", NULL, lua_object_on},

    {"launches.list", NULL, lua_launches_list},
    {"launches.begin", NULL, lua_launches_begin, TRUE},
    {"Launch:finish", "launch.end", NULL},
    {"Launch:on", NULL, lua_object_on},
    {"Launch:watch", NULL, lua_object_watch},

    {"idle.watch", NULL, lua_idle_watch, TRUE},
    {"IdleWatch:cancel", "idle.cancel", NULL},
    {"IdleWatch:on", NULL, lua_object_on},
    {"IdleWatch:watch", NULL, lua_object_watch},

    {"Override:revoke", "override.revoke", NULL},
    {"Override:on", NULL, lua_object_on},

    {"privacy.state", NULL, lua_privacy_state},
    {"privacy.stop_sharing", "privacy.stop_sharing", NULL},
    {"privacy.stop_recording", "privacy.stop_recording", NULL},

    {"session.status", "session.status", lua_session_status},
    {"session.lock", "session.lock", NULL},
    {"session.logout", "session.logout", NULL},
    {"session.suspend", "session.suspend", NULL},
    {"session.hibernate", "session.hibernate", NULL},
    {"session.reboot", "session.reboot", NULL},
    {"session.poweroff", "session.poweroff", NULL},
    {"runtime.status", "runtime.status", lua_runtime_status},
    {"runtime.reload_config", "runtime.reload_config", NULL},
    {"console.toggle", "console.toggle", NULL},
    {NULL, NULL, NULL},
};

/* Wire operations that socket clients call directly but Lua does not install under that name. Lua
 * reaches Request answers through the Request object (gnoblin-lua-request.c), reaches the preview
 * controls through AnimationPreview, window minimize state through Window:set_minimized, and moves
 * a window to a workspace through Window:move_to_workspace. Lua changes the orientation lock and
 * the privacy screen with gnoblin.settings.override; the two wire names stay for socket clients.
 * The launch names are those shells use on the wire. gnoblin_config_call_api accepts them;
 * install_api does not install them. */
static const LuaApiEntry bridge_operations[] = {
    {"prompt.respond", "prompt.respond", NULL},
    {"prompt.cancel", "prompt.cancel", NULL},
    {"auth.begin", "auth.begin", NULL},
    {"auth.respond", "auth.respond", NULL},
    {"auth.cancel", "auth.cancel", NULL},
    {"location.authorize_app", "location.authorize_app", NULL},
    {"launch.begin", "launch.begin", NULL},
    {"shortcut.bind", "shortcut.bind", NULL},
    {"window.minimize", "window.minimize", NULL},
    {"window.unminimize", "window.unminimize", NULL},
    {"window.toggle_minimize", "window.toggle_minimize", NULL},
    {"window.restore", "window.restore", NULL},
    {"workspace.move_active", "workspace.move_active", NULL},
    {"workspace.move_window", "workspace.move_window", NULL},
    {"animation.seek", "animation.seek", NULL},
    {"animation.step", "animation.step", NULL},
    {"animation.play", "animation.play", NULL},
    {"animation.pause", "animation.pause", NULL},
    {"animation.stop", "animation.stop", NULL},
    {"input.set_orientation_lock", "input.set_orientation_lock", NULL},
    {"monitors.set_privacy_screen", "monitors.set_privacy_screen", lua_monitors_set_privacy_screen},
    {NULL, NULL, NULL},
};

/* The entry a socket client names by operation or by gnoblin path. Paths under gnoblin come
 * first, then the wire-only operations, then record methods, so workspace.switch resolves to
 * Workspace:activate and animation.seek to the generic operation, not AnimationPreview:seek. */
const LuaApiEntry* lua_api_find_call(const char* method) {
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (entry->operation && !strchr(entry->path, ':') &&
            (g_str_equal(entry->operation, method) || g_str_equal(entry->path, method)))
            return entry;
    for (const LuaApiEntry* entry = bridge_operations; entry->path; entry++)
        if (g_str_equal(entry->operation, method))
            return entry;
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (entry->operation && strchr(entry->path, ':') && g_str_equal(entry->operation, method))
            return entry;
    return NULL;
}

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
        g_strdup_printf("Lua %s failed and stays active: %s", scope ? scope : "callback",
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
