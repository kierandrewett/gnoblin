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

/* The row of an operation in gnoblin-api-operations.h, for the `operation` of an entry. */
#define OPERATION(id) (&gnoblin_api_operations[GNOBLIN_OP_##id])
/* The positional arguments of a record method (LuaApiEntry.arguments). */
#define ARGS(...) ((const LuaApiArg[]){__VA_ARGS__, {NULL, LUA_ARG_END}})

const LuaApiArg lua_no_arguments[] = {{NULL, LUA_ARG_END}};

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
    {"config.restore_default", OPERATION(CONFIG_RESTORE_DEFAULT), NULL},
    {"load", NULL, lua_config_load},
    {"array", NULL, lua_array},
    {"snapshot", NULL, lua_snapshot},
    {"timers.after", NULL, lua_timers_after},
    {"timers.now", NULL, lua_timers_now},
    {"ease", NULL, lua_ease},
    {"lerp", NULL, lua_lerp},
    {"tween", NULL, lua_tween_new},
    {"Tween:property", NULL, lua_tween_property},
    {"Tween:interval", NULL, lua_tween_interval},
    {"Tween:callback", NULL, lua_tween_callback},
    {"Tween:parallel", NULL, lua_tween_parallel},
    {"Tween:play", NULL, lua_tween_play},
    {"Tween:pause", NULL, lua_tween_pause},
    {"Tween:seek", NULL, lua_tween_seek},
    {"Tween:kill", NULL, lua_tween_kill},
    {"Tween:on", NULL, lua_object_on},
    {"Tween:watch", NULL, lua_object_watch},

    {"process.spawn", NULL, lua_process_spawn},
    {"process.list", NULL, lua_process_list},
    {"process.by_id", NULL, lua_process_by_id},
    {"Process:wait", NULL, lua_process_wait},
    {"Process:kill", NULL, lua_process_kill},
    {"Process:on", NULL, lua_object_on},
    {"Process:watch", NULL, lua_object_watch},
    {"on", NULL, lua_on},
    {"once", NULL, lua_once},
    {"find", NULL, lua_find},

    {"windows.list", NULL, lua_windows_list},
    {"windows.focused", NULL, lua_windows_focused},
    {"windows.by_id", NULL, lua_windows_by_id},
    {"windows.snap_context", NULL, lua_windows_snap_context},
    {"Window:close", OPERATION(WINDOW_CLOSE), NULL, FALSE, lua_no_arguments},
    {"Window:kill", OPERATION(WINDOW_KILL), NULL, FALSE, lua_no_arguments},
    {"Window:raise", OPERATION(WINDOW_RAISE), NULL, FALSE, lua_no_arguments},
    {"Window:lower", OPERATION(WINDOW_LOWER), NULL, FALSE, lua_no_arguments},
    {"Window:set_minimized", OPERATION(WINDOW_SET_MINIMIZED), NULL, FALSE,
     ARGS({NULL, LUA_ARG_BOOLEAN})},
    {"Window:restore_or_minimize", OPERATION(WINDOW_RESTORE_OR_MINIMIZE), NULL, FALSE,
     lua_no_arguments},
    {"Window:set_maximized", OPERATION(WINDOW_SET_MAXIMIZED), NULL, FALSE,
     ARGS({"enabled", LUA_ARG_BOOLEAN})},
    {"Window:set_fullscreen", OPERATION(WINDOW_SET_FULLSCREEN), NULL, FALSE,
     ARGS({"enabled", LUA_ARG_BOOLEAN})},
    {"Window:set_above", OPERATION(WINDOW_SET_ABOVE), NULL, FALSE,
     ARGS({"enabled", LUA_ARG_BOOLEAN})},
    {"Window:set_sticky", OPERATION(WINDOW_SET_STICKY), NULL, FALSE,
     ARGS({"enabled", LUA_ARG_BOOLEAN})},
    {"Window:move", OPERATION(WINDOW_MOVE), NULL, FALSE,
     ARGS({"x", LUA_ARG_INTEGER}, {"y", LUA_ARG_INTEGER})},
    {"Window:resize", OPERATION(WINDOW_RESIZE), NULL, FALSE,
     ARGS({"width", LUA_ARG_INTEGER}, {"height", LUA_ARG_INTEGER})},
    {"Window:move_to_workspace", OPERATION(WINDOW_MOVE_TO_WORKSPACE), NULL, FALSE,
     ARGS({"workspace", LUA_ARG_WORKSPACE}, {"follow", LUA_ARG_OPTION_BOOLEAN})},
    {"Window:move_to_monitor", OPERATION(WINDOW_MOVE_TO_MONITOR), NULL, FALSE,
     ARGS({"monitor", LUA_ARG_MONITOR})},
    {"Window:focus", OPERATION(WINDOW_FOCUS), NULL},
    {"Window:begin_move", OPERATION(WINDOW_BEGIN_MOVE), NULL},
    {"Window:begin_resize", OPERATION(WINDOW_BEGIN_RESIZE), NULL},
    {"Window:thumbnail", OPERATION(WINDOW_THUMBNAIL), NULL, FALSE,
     ARGS({"width", LUA_ARG_SIZE, 480}, {"height", LUA_ARG_SIZE, 320})},
    {"Window:present", OPERATION(WINDOW_PRESENT), NULL, FALSE, lua_no_arguments},
    {"Window:set_opacity", OPERATION(WINDOW_SET_VISUAL), NULL, FALSE,
     ARGS({"opacity", LUA_ARG_NUMBER})},
    {"Window:set_scale", OPERATION(WINDOW_SET_VISUAL), NULL, FALSE,
     ARGS({"scale", LUA_ARG_NUMBER})},
    {"Window:set_rotation", OPERATION(WINDOW_SET_VISUAL), NULL, FALSE,
     ARGS({"rotation", LUA_ARG_NUMBER})},
    {"Window:set_offset", OPERATION(WINDOW_SET_VISUAL), NULL, FALSE,
     ARGS({"offset_x", LUA_ARG_NUMBER}, {"offset_y", LUA_ARG_NUMBER})},
    {"Window:reset_visuals", OPERATION(WINDOW_SET_VISUAL), NULL, FALSE,
     ARGS({"reset", LUA_ARG_CONSTANT_TRUE})},
    {"Window:inspect_animation", OPERATION(ANIMATION_INSPECT), NULL},
    {"Window:preview_animation", OPERATION(ANIMATION_PREVIEW), NULL},
    {"Window:set_effect", OPERATION(WINDOW_SET_EFFECT), NULL, FALSE,
     ARGS({"name", LUA_ARG_STRING}, {"value", LUA_ARG_ANY})},
    {"Window:on", NULL, lua_object_on},
    {"Window:watch", NULL, lua_object_watch},

    {"workspaces.list", NULL, lua_workspaces_list},
    {"workspaces.active", NULL, lua_workspaces_active},
    {"workspaces.by_id", NULL, lua_workspaces_by_id},
    {"workspaces.create", OPERATION(WORKSPACE_CREATE), NULL, TRUE},
    {"workspaces.next", OPERATION(WORKSPACE_NEXT), NULL},
    {"workspaces.previous", OPERATION(WORKSPACE_PREVIOUS), NULL},
    {"Workspace:activate", OPERATION(WORKSPACE_SWITCH), NULL, FALSE, lua_no_arguments},
    {"Workspace:rename", OPERATION(WORKSPACE_RENAME), NULL, FALSE, ARGS({"name", LUA_ARG_STRING})},
    {"Workspace:remove", OPERATION(WORKSPACE_REMOVE), NULL, FALSE, lua_no_arguments},
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
    {"LayerSurface:inspect_animation", OPERATION(ANIMATION_INSPECT), NULL},
    {"LayerSurface:preview_animation", OPERATION(ANIMATION_PREVIEW), NULL},
    {"LayerSurface:watch", NULL, lua_object_watch},
    {"capabilities.list", NULL, lua_capabilities_list},
    {"appearance.color_scheme", NULL, lua_appearance_color_scheme},

    {"animations.list", NULL, lua_animations_list},
    {"animations.get", NULL, lua_animation_get},
    /* Lua inspects and previews an animation through Window and LayerSurface. */
    {"AnimationPreview:seek", OPERATION(ANIMATION_SEEK), lua_animation_preview_method},
    {"AnimationPreview:step", OPERATION(ANIMATION_STEP), lua_animation_preview_method},
    {"AnimationPreview:play", OPERATION(ANIMATION_PLAY), lua_animation_preview_method},
    {"AnimationPreview:pause", OPERATION(ANIMATION_PAUSE), lua_animation_preview_method},
    {"AnimationPreview:stop", OPERATION(ANIMATION_STOP), lua_animation_preview_method},
    {"AnimationPreview:on", NULL, lua_object_on},
    {"AnimationPreview:watch", NULL, lua_object_watch},

    {"input.devices", NULL, lua_input_devices},
    {"input.sources", OPERATION(INPUT_SOURCES), lua_input_sources},
    {"input.current_source", OPERATION(INPUT_CURRENT_SOURCE), lua_input_current_source},
    {"InputSource:select", OPERATION(INPUT_SELECT), NULL, FALSE, lua_no_arguments},
    {"InputSource:on", NULL, lua_object_on},
    {"InputSource:watch", NULL, lua_object_watch},
    {"input.text_target", OPERATION(INPUT_TEXT_TARGET), lua_input_text_target},
    {"input.orientation_lock", NULL, lua_input_orientation_lock},
    {"input.on", NULL, lua_input_on},
    {"input.grab", NULL, lua_input_grab, TRUE},
    {"Grab:renew", OPERATION(INPUT_GRAB_RENEW), NULL, FALSE, lua_no_arguments},
    {"Grab:release", OPERATION(INPUT_GRAB_RELEASE), NULL, FALSE, lua_no_arguments},
    {"Grab:on", NULL, lua_object_on},

    {"shortcuts.actions", NULL, lua_shortcut_actions},
    {"shortcuts.configured", NULL, lua_shortcuts_configured},
    {"shortcuts.list", NULL, lua_shortcuts_list},
    {"shortcuts.capture", OPERATION(SHORTCUT_CAPTURE), NULL},
    {"shortcuts.bind", NULL, lua_shortcuts_bind, TRUE},
    {"Shortcut:unbind", OPERATION(SHORTCUT_UNBIND), NULL, FALSE, lua_no_arguments},
    {"Shortcut:end_session", OPERATION(SHORTCUT_SESSION_END), NULL, FALSE,
     ARGS({"session_id", LUA_ARG_POSITIVE_INTEGER})},
    {"Shortcut:on", NULL, lua_object_on},

    {"launches.list", NULL, lua_launches_list},
    {"launches.begin", NULL, lua_launches_begin, TRUE},
    {"Launch:finish", OPERATION(LAUNCH_END), NULL, FALSE, lua_no_arguments},
    {"Launch:on", NULL, lua_object_on},
    {"Launch:watch", NULL, lua_object_watch},

    {"idle.watch", NULL, lua_idle_watch, TRUE},
    {"IdleWatch:cancel", OPERATION(IDLE_CANCEL), NULL, FALSE, lua_no_arguments},
    {"IdleWatch:on", NULL, lua_object_on},
    {"IdleWatch:watch", NULL, lua_object_watch},

    {"Override:revoke", OPERATION(OVERRIDE_REVOKE), NULL, FALSE, lua_no_arguments},
    {"Override:on", NULL, lua_object_on},

    {"privacy.state", NULL, lua_privacy_state},
    {"privacy.stop_sharing", OPERATION(PRIVACY_STOP_SHARING), NULL},
    {"privacy.stop_recording", OPERATION(PRIVACY_STOP_RECORDING), NULL},

    {"session.status", OPERATION(SESSION_STATUS), lua_session_status},
    {"session.lock", OPERATION(SESSION_LOCK), NULL},
    {"session.logout", OPERATION(SESSION_LOGOUT), NULL},
    {"session.suspend", OPERATION(SESSION_SUSPEND), NULL},
    {"session.hibernate", OPERATION(SESSION_HIBERNATE), NULL},
    {"session.reboot", OPERATION(SESSION_REBOOT), NULL},
    {"session.poweroff", OPERATION(SESSION_POWEROFF), NULL},
    {"runtime.status", OPERATION(RUNTIME_STATUS), lua_runtime_status},
    {"runtime.reload_config", OPERATION(RUNTIME_RELOAD_CONFIG), NULL},
    {"console.toggle", OPERATION(CONSOLE_TOGGLE), NULL},
    {NULL, NULL, NULL},
};

/* Wire operations that socket clients call directly but Lua does not install under that name. Lua
 * reaches Request answers through the Request object (gnoblin-lua-request.c), reaches the preview
 * controls through AnimationPreview, starts one through Window:preview_animation and
 * LayerSurface:preview_animation, reads layer surfaces through gnoblin.layers, window minimize
 * state through Window:set_minimized, and moves a window to a workspace through
 * Window:move_to_workspace. Lua changes the orientation lock and
 * the privacy screen with gnoblin.settings.override; the two wire names stay for socket clients.
 * The launch names are those shells use on the wire. gnoblin_config_call_api accepts them;
 * install_api does not install them. These entries have no path: the name is the operation's. */
static const LuaApiEntry bridge_operations[] = {
    {NULL, OPERATION(PROMPT_RESPOND), NULL},
    {NULL, OPERATION(PROMPT_CANCEL), NULL},
    {NULL, OPERATION(AUTH_BEGIN), NULL},
    {NULL, OPERATION(AUTH_RESPOND), NULL},
    {NULL, OPERATION(AUTH_CANCEL), NULL},
    {NULL, OPERATION(LOCATION_AUTHORIZE_APP), NULL},
    {NULL, OPERATION(LAUNCH_BEGIN), NULL},
    {NULL, OPERATION(SHORTCUT_BIND), NULL},
    {NULL, OPERATION(WINDOW_MINIMIZE), NULL},
    {NULL, OPERATION(WINDOW_UNMINIMIZE), NULL},
    {NULL, OPERATION(WINDOW_TOGGLE_MINIMIZE), NULL},
    {NULL, OPERATION(WINDOW_RESTORE), NULL},
    {NULL, OPERATION(WORKSPACE_MOVE_ACTIVE), NULL},
    {NULL, OPERATION(WORKSPACE_MOVE_WINDOW), NULL},
    {NULL, OPERATION(ANIMATION_SEEK), NULL},
    {NULL, OPERATION(ANIMATION_SURFACES), NULL},
    {NULL, OPERATION(ANIMATION_INSPECT), NULL},
    {NULL, OPERATION(ANIMATION_PREVIEW), NULL},
    {NULL, OPERATION(ANIMATION_STEP), NULL},
    {NULL, OPERATION(ANIMATION_PLAY), NULL},
    {NULL, OPERATION(ANIMATION_PAUSE), NULL},
    {NULL, OPERATION(ANIMATION_STOP), NULL},
    {NULL, OPERATION(INPUT_SET_ORIENTATION_LOCK), NULL},
    {NULL, OPERATION(MONITORS_SET_PRIVACY_SCREEN), lua_monitors_set_privacy_screen},
    {NULL, NULL, NULL},
};

/* The entry a socket client names by operation or by gnoblin path. Paths under gnoblin come
 * first, then the wire-only operations, then record methods, so workspace.switch resolves to
 * Workspace:activate and animation.seek to the generic operation, not AnimationPreview:seek. */
const LuaApiEntry* lua_api_find_call(const char* method) {
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (entry->operation && !strchr(entry->path, ':') &&
            (g_str_equal(entry->operation->name, method) || g_str_equal(entry->path, method)))
            return entry;
    for (const LuaApiEntry* entry = bridge_operations; entry->operation; entry++)
        if (g_str_equal(entry->operation->name, method))
            return entry;
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++)
        if (entry->operation && strchr(entry->path, ':') &&
            g_str_equal(entry->operation->name, method))
            return entry;
    return NULL;
}

const LuaApiArg* lua_api_operation_argument(const GnoblinApiOperation* operation,
                                            const char* field) {
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (!entry->operation || entry->operation->id != operation->id || !entry->arguments)
            continue;
        for (const LuaApiArg* argument = entry->arguments; argument->kind != LUA_ARG_END;
             argument++)
            if (argument->field && g_str_equal(argument->field, field))
                return argument;
    }
    return NULL;
}

/* The wire fields of the record methods that run `operation`: "id", then the field of each
 * positional argument of each method. The Lua runtime checks a table call of the operation against
 * these (gnoblin-lua-operations.c). */
gsize lua_api_operation_fields(const GnoblinApiOperation* operation, const char** names,
                               gsize capacity) {
    gsize count = 0;
    names[count++] = "id";
    for (const LuaApiEntry* entry = lua_api; entry->path; entry++) {
        if (!entry->operation || entry->operation->id != operation->id || !entry->arguments)
            continue;
        for (const LuaApiArg* argument = entry->arguments; argument->kind != LUA_ARG_END;
             argument++) {
            gboolean known = !argument->field;
            for (gsize i = 0; argument->field && i < count; i++)
                known |= g_str_equal(names[i], argument->field);
            if (!known && count + 1 < capacity)
                names[count++] = argument->field;
        }
    }
    names[count] = NULL;
    return count;
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
