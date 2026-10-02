/* Restricted, single-state Lua configuration evaluator. */
#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"

#include <gio/gio.h>
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

#include "gnoblin-lua-pattern.h"

#if __has_include("../core/gnoblin-native-control.h")
#include "../core/gnoblin-native-control.h"
#endif

#define MAX_CONFIG_BYTES (8 * 1024 * 1024)
#define MAX_CONFIG_STEPS 1000000
#define MAX_CONFIG_DEPTH 64
#define MAX_CONFIG_FILES 32
#define LUA_FOCUS_CONTEXT_METATABLE "gnoblin.FocusContext"
#define LUA_MENU_CONTEXT_METATABLE "gnoblin.MenuContext"
#define LUA_TEXT_TARGET_METATABLE "gnoblin.TextTarget"
#define LUA_WINDOW_DRAG_METATABLE "gnoblin.WindowDrag"
#define LUA_SNAP_CONTEXT_METATABLE "gnoblin.SnapContext"
#define LUA_READONLY_SNAPSHOT_METATABLE "gnoblin.ReadonlySnapshot"

typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
    gboolean consumed;
} LuaFocusContext;

typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
    char window_id[24];
    gboolean consumed;
} LuaMenuContext;

typedef struct {
    char token[65];
    char window_id[24];
    guint64 generation;
    gboolean consumed;
    gboolean has_caret;
    double x, y, width, height;
} LuaTextTarget;

typedef struct {
    guint64 id;
    guint64 settings_revision;
    guint64 generation;
    gboolean ended;
} LuaWindowDrag;

typedef struct {
    char token[65];
    guint64 generation;
    gboolean consumed;
} LuaSnapContext;

typedef struct {
    guint64 handle;
    guint64 generation;
    guint64 native_generation;
    gint64 expires_at_us;
} LuaFocusOperation;

typedef struct {
    gsize allocated;
    GPtrArray *paths, *directories;
    GPtrArray* runtime_actions;
    GPtrArray* deferred_callbacks;
    GHashTable* operations;
    GHashTable* focus_operations;
    GVariant* window_snapshot;
    guint64 window_revision;
    GVariant* workspace_snapshot;
    guint64 workspace_revision;
    GVariant* monitor_snapshot;
    guint64 monitor_revision;
    GVariant* layer_snapshot;
    guint64 layer_revision;
    GVariant* capability_snapshot;
    guint64 capability_revision;
    GVariant* input_device_snapshot;
    guint64 input_device_revision;
    GVariant* input_source_snapshot;
    guint64 input_source_revision;
    GVariant* orientation_lock_snapshot;
    guint64 orientation_lock_revision;
    GVariant* shortcut_snapshot;
    guint64 shortcut_revision;
    GVariant* launch_snapshot;
    guint64 launch_revision;
    GVariant* portal_grant_snapshot;
    guint64 portal_grant_revision;
    GVariant* animation_snapshot;
    guint64 animation_revision;
    GVariant* privacy_snapshot;
    guint64 privacy_revision;
    GVariant* session_activity_snapshot;
    guint64 session_activity_revision;
    gint64 session_activity_sampled_at_us;
    GVariant* session_lock_snapshot;
    guint64 input_gesture_sequence;
    guint64 dispatch_focus_handle;
    guint64 dispatch_focus_generation;
    guint64 dispatch_native_focus_generation;
    gint64 dispatch_focus_expires_at_us;
    guint64 dispatch_menu_handle;
    guint64 dispatch_menu_generation;
    guint64 dispatch_native_menu_generation;
    gint64 dispatch_menu_expires_at_us;
    char dispatch_menu_window_id[24];
    GVariant* settings_document;
    guint64 settings_revision;
    GPtrArray* focus_history_ids;
    GHashTable *active, *modules;
    char* current_path;
    guint actions_in_dispatch;
    gboolean dispatching;
    gboolean api_calling;
    gboolean rejecting_operations;
    gboolean deferred_callbacks_scheduled;
} LuaConfig;

typedef struct {
    LuaConfig config;
    lua_State* state;
    GVariant* document;
    GVariant* pending_document;
    guint64 generation;
} LuaRuntime;

static LuaRuntime* active_runtime;
static LuaRuntime* pending_runtime;
static LuaRuntime* deferred_runtime;
static guint64 next_runtime_operation_id;
static guint64 next_settings_revision;
static guint64 next_runtime_generation;
static gboolean revision_counters_seeded;
static GnoblinConfigRuntimeWakeupFunc runtime_wakeup_callback;
static gpointer runtime_wakeup_data;
static GnoblinConfigFocusPolicyChangedFunc focus_policy_changed_callback;
static gpointer focus_policy_changed_data;
static GnoblinConfigPermissionPolicyChangedFunc permission_policy_changed_callback;
static gpointer permission_policy_changed_data;
static GnoblinConfigSettingsChangedFunc settings_changed_callback;
static gpointer settings_changed_data;

static int lua_workspace_action(lua_State* state);
static int lua_generic_api_action(lua_State* state);
static int lua_focus_history(lua_State* state);
static int lua_animation_preview_method(lua_State* state);
static int lua_permissions_list(lua_State* state);
static int lua_permissions_policy(lua_State* state);
static int lua_privacy_state(lua_State* state);
static int lua_session_activity(lua_State* state);
static int lua_session_status(lua_State* state);
static int lua_runtime_status(lua_State* state);
static int lua_input_text_target(lua_State* state);
static int lua_text_target_insert_text(lua_State* state);
static int lua_text_target_index(lua_State* state);
static void push_window_drag(lua_State* state, GVariant* value);
static void push_snap_context(lua_State* state, GVariant* value);
static int lua_window_drag_offer_targets(lua_State* state);
static int lua_snap_context_commit(lua_State* state);
static int lua_windows_snap_context(lua_State* state);
static int lua_permissions_check(lua_State* state);
static int lua_launches_begin(lua_State* state);
static int lua_launches_finish(lua_State* state);
static int lua_portal_grants(lua_State* state);
static guint64 allocate_operation_id(void);
static GVariant* focus_policy_snapshot(GVariant* document, guint64 revision);
static void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id,
                                  const char* method);

static int lua_focus_context_tostring(lua_State* state) {
    lua_pushliteral(state, "FocusContext");
    return 1;
}

static int lua_menu_context_tostring(lua_State* state) {
    lua_pushliteral(state, "MenuContext");
    return 1;
}

static int lua_menu_context_begin_move(lua_State* state);
static int lua_menu_context_begin_resize(lua_State* state);

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

static void push_menu_context(lua_State* state, guint64 handle, guint64 generation,
                              guint64 native_generation, gint64 expires_at_us,
                              const char* window_id) {
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

static void push_focus_context(lua_State* state, guint64 handle, guint64 generation,
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

static const char* api_methods[] = {
    "workspace.create",
    "workspace.rename",
    "workspace.remove",
    "workspace.switch",
    "workspace.next",
    "workspace.previous",
    "workspace.move_active",
    "workspace.move_window",
    "window.close",
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
    "runtime.reload_config",
    "runtime.status",
    "shortcut.capture",
    "shortcut.bind",
    "shortcut.unbind",
    "shortcut.session.end",
    NULL,
};

/* Public plural collection names are aliases over the operation names that
 * the compositor currently registers. Keep the mapping here so Lua-facing
 * names can evolve without making the wire API depend on presentation. */
static const struct {
    const char* public_name;
    const char* operation;
} api_aliases[] = {
    {"workspaces.create", "workspace.create"},
    {"workspaces.rename", "workspace.rename"},
    {"workspaces.remove", "workspace.remove"},
    {"workspaces.activate", "workspace.switch"},
    {"workspaces.next", "workspace.next"},
    {"workspaces.previous", "workspace.previous"},
    {"workspaces.move_active", "workspace.move_active"},
    {"workspaces.move_window", "workspace.move_window"},
    {"input.select_source", "input.select"},
    {"animations.surfaces", "animation.surfaces"},
    {"animations.inspect", "animation.inspect"},
    {"animations.preview", "animation.preview"},
    {"animations.seek", "animation.seek"},
    {"animations.step", "animation.step"},
    {"animations.play", "animation.play"},
    {"animations.pause", "animation.pause"},
    {"animations.stop", "animation.stop"},
};

static gboolean append_array_key(const char* key) {
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

static lua_State* new_config_state(LuaConfig* config) {
#if LUA_VERSION_NUM >= 505
    return lua_newstate(limited_alloc, config, g_random_int());
#else
    return lua_newstate(limited_alloc, config);
#endif
}

static void limit_hook(lua_State* state, lua_Debug* debug) {
    int* remaining = (int*)lua_getextraspace(state);
    (void)debug;
    if (--*remaining <= 0)
        luaL_error(state, "configuration instruction limit exceeded");
}

static void add_path(GPtrArray* paths, const char* path) {
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    for (guint i = 0; i < paths->len; i++)
        if (!strcmp(g_ptr_array_index(paths, i), canonical))
            return;
    g_ptr_array_add(paths, g_steal_pointer(&canonical));
}

static char* resolve_path(const char* current, const char* given) {
    if (g_str_has_prefix(given, "~/"))
        return g_canonicalize_filename(given + 2, g_get_home_dir());
    if (g_path_is_absolute(given))
        return g_canonicalize_filename(given, NULL);
    g_autofree char* directory = g_path_get_dirname(current);
    return g_canonicalize_filename(given, directory);
}

static LuaConfig* config_from_upvalue(lua_State* state) {
    return lua_touserdata(state, lua_upvalueindex(1));
}
static const char* lua_error_text(lua_State* state) {
    return lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1)
                                              : "Lua configuration failed";
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

static gboolean marked_array(lua_State* state, int index) {
    gboolean result = FALSE;
    if (!lua_getmetatable(state, index))
        return FALSE;
    lua_pushstring(state, "__gnoblin_array");
    lua_rawget(state, -2);
    result = lua_toboolean(state, -1);
    lua_pop(state, 2);
    return result;
}

static void mark_array_table(lua_State* state, int index) {
    index = lua_absindex(state, index);
    lua_newtable(state);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "__gnoblin_array");
    lua_setmetatable(state, index);
}

static GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
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
    GVariantBuilder out;
    g_variant_builder_init(&out, G_VARIANT_TYPE_VARDICT);
    lua_pushnil(state);
    while (lua_next(state, index)) {
        const char* key = lua_tostring(state, -2);
        gboolean child_array = append_array_key(key) || keybinding_depth == 2;
        int child_keybinding_depth = !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1     ? 2
                                                                 : 0;
        g_autoptr(GVariant) child =
            variant_from_lua(state, -1, depth + 1, child_array, child_keybinding_depth, error);
        if (!child) {
            lua_pop(state, 1);
            g_variant_builder_clear(&out);
            return NULL;
        }
        g_variant_builder_add(&out, "{sv}", key, g_variant_ref(child));
        lua_pop(state, 1);
    }
    return g_variant_builder_end(&out);
}

static void push_variant(lua_State* state, GVariant* value) {
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

static gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed);

/* Methods on snapshot records are thin adapters to the existing typed
 * operation dispatcher. Keeping the ID on the detached record means stale
 * records still fail in the compositor rather than targeting a reused row. */
static int lua_record_method(lua_State* state) {
    LuaConfig* config = active_runtime ? &active_runtime->config : NULL;
    const char* method = lua_tostring(state, lua_upvalueindex(1));
    gboolean workspace = g_str_has_prefix(method, "workspace.");
    gboolean workspace_move = g_str_equal(method, "workspace.move_window");
    gboolean portal_grant = g_str_equal(method, "grant.revoke");
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
    if (portal_grant) {
        lua_getfield(state, backing, "kind");
        lua_setfield(state, arguments, "kind");
        lua_getfield(state, backing, "created_at");
        lua_setfield(state, arguments, "created_at");
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
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.minimize") ||
        g_str_equal(method, "window.toggle_minimize") || g_str_equal(method, "window.restore") ||
        g_str_equal(method, "window.unminimize") ||
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
    } else {
        lua_getfield(state, backing, "kind");
        gboolean is_portal_grant = lua_type(state, -1) == LUA_TSTRING;
        lua_pop(state, 1);
        lua_getfield(state, backing, "created_at");
        is_portal_grant &= lua_isinteger(state, -1);
        lua_pop(state, 1);
        if (is_portal_grant)
            add_record_method(state, method_table, "revoke", "grant.revoke");
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
static void push_readonly_copy(lua_State* state, int index) {
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
static void push_readonly_event_fields(lua_State* state, GVariant* payload, int event_index) {
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

static void push_window_drag(lua_State* state, GVariant* value) {
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

static void push_operation_result(lua_State* state, const char* method, GVariant* value) {
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

static gboolean is_array(lua_State* state, int index) {
    return marked_array(state, index) || lua_rawlen(state, index) > 0;
}

static void merge_table(lua_State* state, int destination, int source, const char* key,
                        gboolean append_lists);
static gboolean workspace_id_valid(const char* id);

static void remove_named_entry(lua_State* state, int list, const char* name) {
    list = lua_absindex(state, list);
    lua_Integer count = lua_rawlen(state, list), next = 1;
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches)
            lua_pop(state, 1);
        else
            lua_rawseti(state, list, next++);
    }
    for (lua_Integer i = next; i <= count; i++) {
        lua_pushnil(state);
        lua_rawseti(state, list, i);
    }
}

static gboolean named_entries_key(const char* key) {
    return key && (!strcmp(key, "shortcuts") || !strcmp(key, "autostart"));
}

static gboolean named_entries_map(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (!lua_istable(state, index) || is_array(state, index))
        return FALSE;
    lua_pushnil(state);
    if (!lua_next(state, index))
        return FALSE;
    gboolean map = lua_type(state, -2) == LUA_TSTRING;
    lua_pop(state, 2);
    return map;
}

static void merge_named_entries(lua_State* state, int list, int entries) {
    list = lua_absindex(state, list);
    entries = lua_absindex(state, entries);
    lua_pushnil(state);
    while (lua_next(state, entries)) {
        if (lua_type(state, -2) != LUA_TSTRING || !lua_istable(state, -1))
            luaL_error(state, "named shortcuts and autostart entries must be tables");
        const char* name = lua_tostring(state, -2);
        int source = lua_gettop(state);
        lua_getfield(state, source, "name");
        if (!lua_isnil(state, -1) &&
            (lua_type(state, -1) != LUA_TSTRING || strcmp(name, lua_tostring(state, -1))))
            luaL_error(state, "named entry key and name must match");
        lua_pop(state, 1);
        lua_getfield(state, source, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (disabled) {
            remove_named_entry(state, list, name);
            lua_pop(state, 1);
            continue;
        }
        lua_newtable(state);
        int entry = lua_gettop(state);
        merge_table(state, entry, source, NULL, FALSE);
        lua_pushnil(state);
        lua_setfield(state, entry, "enable");
        lua_pushstring(state, name);
        lua_setfield(state, entry, "name");
        lua_Integer count = lua_rawlen(state, list);
        gboolean merged = FALSE;
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, list, i);
            lua_getfield(state, -1, "name");
            gboolean matches =
                lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
            lua_pop(state, 1);
            if (matches) {
                lua_pushnil(state);
                lua_setfield(state, -2, "enable");
                merge_table(state, -1, entry, NULL, FALSE);
                merged = TRUE;
            }
            lua_pop(state, 1);
            if (merged)
                break;
        }
        if (!merged) {
            lua_pushvalue(state, entry);
            lua_rawseti(state, list, count + 1);
        }
        lua_pop(state, 2);
    }
}

static void merge_table(lua_State* state, int destination, int source, const char* key,
                        gboolean append_lists) {
    destination = lua_absindex(state, destination);
    source = lua_absindex(state, source);
    if (key && !strcmp(key, "workspaces") && lua_istable(state, destination) &&
        lua_istable(state, source)) {
        lua_Integer count = lua_rawlen(state, source);
        lua_newtable(state);
        int seen_ids = lua_absindex(state, -1);
        lua_pushnil(state);
        while (lua_next(state, source)) {
            gboolean dense_key = lua_isinteger(state, -2) && lua_tointeger(state, -2) >= 1 &&
                                 lua_tointeger(state, -2) <= count;
            lua_pop(state, 1);
            if (!dense_key)
                luaL_error(state, "workspaces must be a dense array starting at 1");
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, source, i);
            if (!lua_istable(state, -1))
                luaL_error(state, "workspaces must be an array of tables");
            int entry = lua_absindex(state, -1);
            lua_getfield(state, entry, "id");
            const char* id = luaL_checkstring(state, -1);
            if (!workspace_id_valid(id))
                luaL_error(state, "workspace id must start with a letter or number and contain "
                                  "only letters, numbers, '.', '_' or '-' (up to 64 characters)");
            lua_pushvalue(state, -1);
            lua_rawget(state, seen_ids);
            gboolean duplicate = lua_toboolean(state, -1);
            lua_pop(state, 1);
            if (duplicate)
                luaL_error(state, "duplicate workspace id in workspaces array: %s", id);
            lua_pushvalue(state, -1);
            lua_pushboolean(state, TRUE);
            lua_rawset(state, seen_ids);
            lua_pop(state, 1);
            lua_Integer existing_count = lua_rawlen(state, destination);
            gboolean merged = FALSE;
            for (lua_Integer j = 1; j <= existing_count; j++) {
                lua_rawgeti(state, destination, j);
                lua_getfield(state, -1, "id");
                gboolean matches =
                    lua_type(state, -1) == LUA_TSTRING && g_str_equal(lua_tostring(state, -1), id);
                lua_pop(state, 1);
                if (matches) {
                    merge_table(state, -1, entry, NULL, FALSE);
                    merged = TRUE;
                }
                lua_pop(state, 1);
                if (merged)
                    break;
            }
            if (!merged) {
                lua_pushvalue(state, entry);
                lua_rawseti(state, destination, existing_count + 1);
            }
            lua_pop(state, 1);
        }
        lua_pop(state, 1);
        return;
    }
    if (append_lists && append_array_key(key) && lua_istable(state, destination) &&
        lua_istable(state, source)) {
        lua_Integer offset = lua_rawlen(state, destination), count = lua_rawlen(state, source);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, source, i);
            lua_rawseti(state, destination, offset + i);
        }
        return;
    }
    lua_pushnil(state);
    while (lua_next(state, source)) {
        const char* name = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
        if (named_entries_key(name) && named_entries_map(state, -1)) {
            lua_pushvalue(state, -2);
            lua_rawget(state, destination);
            if (lua_isnil(state, -1)) {
                lua_pop(state, 1);
                lua_newtable(state);
                lua_pushvalue(state, -3);
                lua_pushvalue(state, -2);
                lua_rawset(state, destination);
            }
            if (!lua_istable(state, -1) || named_entries_map(state, -1))
                luaL_error(state, "%s must be a list before named overrides", name);
            merge_named_entries(state, -1, -2);
            lua_pop(state, 1);
        } else if (name && lua_istable(state, -1)) {
            lua_pushvalue(state, -2);
            lua_rawget(state, destination);
            if (lua_istable(state, -1) && !is_array(state, -1) && !is_array(state, -2)) {
                merge_table(state, -1, -2, name, append_lists);
                lua_pop(state, 1);
            } else {
                lua_pop(state, 1);
                lua_pushvalue(state, -2);
                lua_pushvalue(state, -2);
                lua_rawset(state, destination);
            }
        } else {
            lua_pushvalue(state, -2);
            lua_pushvalue(state, -2);
            lua_rawset(state, destination);
        }
        lua_pop(state, 1);
    }
}

static int lua_array(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_newtable(state);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "__gnoblin_array");
    lua_setmetatable(state, 1);
    lua_settop(state, 1);
    return 1;
}

static void copy_config_value(lua_State* state, int source, int depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Lua config nesting exceeds 64 levels");
    source = lua_absindex(state, source);
    if (!lua_istable(state, source)) {
        lua_pushvalue(state, source);
        return;
    }
    lua_newtable(state);
    int destination = lua_gettop(state);
    lua_pushnil(state);
    while (lua_next(state, source)) {
        lua_pushvalue(state, -2);
        copy_config_value(state, -2, depth + 1);
        lua_rawset(state, destination);
        lua_pop(state, 1);
    }
    if (marked_array(state, source) && lua_getmetatable(state, source))
        lua_setmetatable(state, destination);
}

static void push_public_settings(lua_State* state, GVariant* value, const char* parent, int depth,
                                 int keybinding_depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Gnoblin settings nesting exceeds 64 levels");
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        push_public_settings(state, child, parent, depth, keybinding_depth);
        return;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        lua_newtable(state);
        mark_array_table(state, -1);
        GVariantIter iter;
        GVariant* child;
        guint i = 1;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "v", &child)) {
            push_public_settings(state, child, parent, depth + 1, keybinding_depth);
            lua_rawseti(state, -2, i++);
            g_variant_unref(child);
        }
        return;
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        push_variant(state, value);
        return;
    }

    lua_newtable(state);
    gboolean literal =
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers") ||
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart") ||
                   !strcmp(parent, "interfaces"));
    GVariantIter iter;
    const char* name;
    GVariant* child;
    g_variant_iter_init(&iter, value);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &child)) {
        gboolean action_name = keybinding_depth == 2;
        g_autofree char* public_name = g_strdup(name);
        if (!literal && !action_name)
            g_strdelimit(public_name, "-", '_');
        int child_keybinding_depth = !strcmp(name, "keybindings") ? 1
                                     : keybinding_depth == 1      ? 2
                                                                  : 0;
        push_public_settings(state, child, name, depth + 1, child_keybinding_depth);
        lua_setfield(state, -2, public_name);
        g_variant_unref(child);
    }
}

static int lua_gnoblin_index(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    if (lua_type(state, 2) != LUA_TSTRING) {
        lua_pushnil(state);
        return 1;
    }
    const char* property = lua_tostring(state, 2);
    static const struct {
        const char* name;
        const char* replacement;
    } removed_apis[] = {
        {"shortcut", "use gnoblin.configure.shortcuts.name = {binding = ..., command = ...}"},
        {"autostart", "use gnoblin.configure.autostart.name = {command = ...}"},
        {"remove_shortcut", "set gnoblin.configure.shortcuts.name.enable = false"},
        {"remove_autostart", "set gnoblin.configure.autostart.name.enable = false"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(removed_apis); i++)
        if (g_str_equal(property, removed_apis[i].name))
            return luaL_error(state, "gnoblin.%s was removed; %s", removed_apis[i].name,
                              removed_apis[i].replacement);
    if (g_str_equal(property, "focus")) {
        if (!config || !config->settings_document) {
            return luaL_error(state,
                              "gnoblin.focus.policy is unavailable before the config is active");
        }

        push_public_settings(state, config->settings_document, NULL, 0, 0);
        lua_getfield(state, -1, "window_management");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
        }
        int window_management = lua_absindex(state, -1);
        lua_createtable(state, 0, 7);
        int policy = lua_absindex(state, -1);
        static const struct {
            const char* name;
            const char* string_default;
            gboolean boolean_default;
            lua_Integer integer_default;
            enum { FOCUS_STRING, FOCUS_BOOLEAN, FOCUS_INTEGER } kind;
        } fields[] = {
            {"focus_mode", "click", FALSE, 0, FOCUS_STRING},
            {"focus_new_windows", "strict", FALSE, 0, FOCUS_STRING},
            {"raise_on_click", NULL, TRUE, 0, FOCUS_BOOLEAN},
            {"auto_raise", NULL, FALSE, 0, FOCUS_BOOLEAN},
            {"focus_change_on_pointer_rest", NULL, FALSE, 0, FOCUS_BOOLEAN},
            {"auto_raise_delay", NULL, FALSE, 500, FOCUS_INTEGER},
        };
        for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
            lua_getfield(state, window_management, fields[i].name);
            if (lua_isnil(state, -1)) {
                lua_pop(state, 1);
                if (fields[i].kind == FOCUS_STRING)
                    lua_pushstring(state, fields[i].string_default);
                else if (fields[i].kind == FOCUS_BOOLEAN)
                    lua_pushboolean(state, fields[i].boolean_default);
                else
                    lua_pushinteger(state, fields[i].integer_default);
            }
            lua_setfield(state, policy, fields[i].name);
        }
        lua_pushinteger(state, (lua_Integer)config->settings_revision);
        lua_setfield(state, policy, "revision");
        push_readonly_copy(state, policy);
        int readonly_policy = lua_absindex(state, -1);
        lua_createtable(state, 0, 2);
        lua_pushvalue(state, readonly_policy);
        lua_setfield(state, -2, "policy");
        lua_pushlightuserdata(state, config);
        lua_pushcclosure(state, lua_focus_history, 1);
        lua_setfield(state, -2, "history");
        push_readonly_copy(state, -1);
        return 1;
    }
    if (!g_str_equal(property, "settings")) {
        lua_pushnil(state);
        return 1;
    }
    if (!config || !config->settings_document)
        return luaL_error(state, "gnoblin.settings is unavailable before the config is active");

    GVariant* settings = config->settings_document;
    push_public_settings(state, settings, NULL, 0, 0);
    lua_pushinteger(state, (lua_Integer)config->settings_revision);
    lua_setfield(state, -2, "revision");
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_snapshot(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "snapshot takes no arguments");
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    copy_config_value(state, -1, 0);
    return 1;
}

static int lua_set(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        return luaL_error(state, "gnoblin.config must be a table");
    merge_table(state, -1, 1, NULL, FALSE);
    lua_pop(state, 2);
    return 0;
}
static int lua_subscription_unsubscribe(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getfield(state, 1, "_active");
    gboolean active = lua_isnil(state, -1) || lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_pushboolean(state, FALSE);
    lua_setfield(state, 1, "_active");
    lua_getfield(state, 1, "_event");
    lua_getfield(state, 1, "_callback");
    const char* event = lua_tostring(state, -2);
    if (!event)
        return 0;
    int callback = lua_absindex(state, -1);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    lua_getfield(state, -1, event);
    if (lua_istable(state, -1)) {
        int list = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, list);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, list, i);
            gboolean match = lua_rawequal(state, callback, -1);
            lua_pop(state, 1);
            if (!match)
                continue;
            lua_getglobal(state, "table");
            lua_getfield(state, -1, "remove");
            lua_pushvalue(state, list);
            lua_pushinteger(state, i);
            lua_call(state, 2, 0);
            break;
        }
    }
    return 0;
}

static int lua_listener_wrapper(lua_State* state) {
    lua_getfield(state, lua_upvalueindex(1), "_active");
    gboolean active = lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_getfield(state, lua_upvalueindex(1), "_user_callback");
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 0, 0) != LUA_OK)
        return lua_error(state);
    return 0;
}

static int lua_on(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    if (event_length == 0 || event_length > 128 || memchr(event, '\0', event_length) ||
        !g_utf8_validate(event, event_length, NULL))
        return luaL_error(state, "event name must contain 1 to 128 bytes");
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    int listeners = lua_absindex(state, -1);
    lua_getfield(state, -1, event);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, listeners, event);
    }
    lua_newtable(state);
    int subscription = lua_absindex(state, -1);
    lua_pushstring(state, event);
    lua_setfield(state, subscription, "_event");
    lua_pushvalue(state, 2);
    lua_setfield(state, subscription, "_user_callback");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, subscription, "_active");
    lua_pushcfunction(state, lua_subscription_unsubscribe);
    lua_setfield(state, subscription, "unsubscribe");
    lua_pushvalue(state, subscription);
    lua_pushcclosure(state, lua_listener_wrapper, 1);
    lua_pushvalue(state, -1);
    lua_setfield(state, subscription, "_callback");
    lua_rawseti(state, -3, lua_rawlen(state, -3) + 1);
    lua_pushvalue(state, subscription);
    return 1;
}

static int lua_mutter_on(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    if (event_length <= strlen("mutter.") || !g_str_has_prefix(event, "mutter."))
        return luaL_error(state, "Mutter event names must start with 'mutter.'");
    return lua_on(state);
}

static int lua_once_callback(lua_State* state) {
    lua_getfield(state, lua_upvalueindex(1), "fired");
    if (lua_toboolean(state, -1))
        return 0;
    lua_pop(state, 1);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, lua_upvalueindex(1), "fired");
    lua_getfield(state, lua_upvalueindex(1), "subscription");
    if (lua_istable(state, -1)) {
        lua_getfield(state, -1, "unsubscribe");
        lua_pushvalue(state, -2);
        lua_call(state, 1, 0);
    }
    lua_pop(state, 1);
    lua_getfield(state, lua_upvalueindex(1), "callback");
    lua_pushvalue(state, 1);
    if (lua_pcall(state, 1, 0, 0) != LUA_OK)
        return lua_error(state);
    return 0;
}

static int lua_once(lua_State* state) {
    luaL_checktype(state, 2, LUA_TFUNCTION);
    lua_newtable(state);
    lua_pushvalue(state, 2);
    lua_setfield(state, -2, "callback");
    lua_pushboolean(state, FALSE);
    lua_setfield(state, -2, "fired");
    lua_pushvalue(state, -1);
    lua_pushcclosure(state, lua_once_callback, 1);
    lua_remove(state, 2);
    lua_remove(state, 2);
    lua_pushcfunction(state, lua_on);
    lua_insert(state, 1);
    lua_call(state, 2, 1);
    lua_getfield(state, 1, "_user_callback");
    lua_getupvalue(state, -1, 1);
    if (lua_istable(state, -1)) {
        lua_pushvalue(state, 1);
        lua_setfield(state, -2, "subscription");
    }
    lua_settop(state, 1);
    return 1;
}

static int lua_mutter_once(lua_State* state) {
    size_t event_length;
    const char* event = luaL_checklstring(state, 1, &event_length);
    if (event_length <= strlen("mutter.") || !g_str_has_prefix(event, "mutter."))
        return luaL_error(state, "Mutter event names must start with 'mutter.'");
    return lua_once(state);
}

static gboolean workspace_id_valid(const char* id) {
    return id && g_regex_match_simple("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$", id, G_REGEX_OPTIMIZE,
                                      G_REGEX_MATCH_NOTEMPTY);
}

static gboolean workspace_selector_id_valid(const char* id) {
    return workspace_id_valid(id) ||
           (id && g_regex_match_simple("^@session-[1-9][0-9]*$", id, G_REGEX_OPTIMIZE,
                                       G_REGEX_MATCH_NOTEMPTY));
}

static guint64 allocate_operation_id(void) {
    if (next_runtime_operation_id >= G_MAXINT64)
        return 0;
    return ++next_runtime_operation_id;
}

static int lua_operation_unsubscribe(lua_State* state);
static void schedule_deferred_callbacks(LuaConfig* config);

static void push_operation_subscription(lua_State* state, int operation, int callback) {
    operation = lua_absindex(state, operation);
    callback = lua_absindex(state, callback);
    lua_newtable(state);
    lua_pushvalue(state, operation);
    lua_setfield(state, -2, "_operation");
    lua_pushvalue(state, callback);
    lua_setfield(state, -2, "_callback");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, -2, "_active");
    lua_pushcfunction(state, lua_operation_unsubscribe);
    lua_setfield(state, -2, "unsubscribe");
}

static int lua_operation_on_complete(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    luaL_checktype(state, 2, LUA_TFUNCTION);
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    lua_getfield(state, 1, "status");
    gboolean pending = lua_isstring(state, -1) && g_str_equal(lua_tostring(state, -1), "pending");
    lua_pop(state, 1);
    if (pending) {
        lua_getfield(state, 1, "_callbacks");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, 1, "_callbacks");
        }
        int callbacks = lua_absindex(state, -1);
        push_operation_subscription(state, 1, 2);
        lua_pushvalue(state, -1);
        lua_rawseti(state, callbacks, lua_rawlen(state, callbacks) + 1);
        return 1;
    } else {
        if (!config || !config->deferred_callbacks) {
            return luaL_error(state, "deferred operation callbacks are unavailable");
        }
        push_operation_subscription(state, 1, 2);
        lua_pushvalue(state, -1);
        int reference = luaL_ref(state, LUA_REGISTRYINDEX);
        g_ptr_array_add(config->deferred_callbacks, GINT_TO_POINTER(reference));
        schedule_deferred_callbacks(config);
        return 1;
    }
}

static int lua_operation_unsubscribe(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    lua_getfield(state, 1, "_active");
    gboolean active = lua_toboolean(state, -1);
    lua_pop(state, 1);
    if (!active)
        return 0;
    lua_pushboolean(state, FALSE);
    lua_setfield(state, 1, "_active");
    lua_getfield(state, 1, "_operation");
    lua_getfield(state, -1, "_callbacks");
    if (lua_istable(state, -1)) {
        int callbacks = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, callbacks);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, callbacks, i);
            gboolean match = lua_rawequal(state, 1, -1);
            lua_pop(state, 1);
            if (!match)
                continue;
            lua_getglobal(state, "table");
            lua_getfield(state, -1, "remove");
            lua_pushvalue(state, callbacks);
            lua_pushinteger(state, i);
            lua_call(state, 2, 0);
            break;
        }
    }
    return 0;
}

static void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id,
                                  const char* method) {
    lua_newtable(state);
    lua_pushinteger(state, id);
    lua_setfield(state, -2, "id");
    lua_pushstring(state, method);
    lua_setfield(state, -2, "method");
    lua_pushliteral(state, "pending");
    lua_setfield(state, -2, "status");
    lua_pushnil(state);
    lua_setfield(state, -2, "value");
    lua_pushnil(state);
    lua_setfield(state, -2, "error");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_operation_on_complete, 1);
    lua_setfield(state, -2, "on_complete");
    if (config->operations) {
        g_autofree char* key = g_strdup_printf("%" G_GUINT64_FORMAT, id);
        lua_pushvalue(state, -1);
        int reference = luaL_ref(state, LUA_REGISTRYINDEX);
        g_hash_table_replace(config->operations, g_strdup(key), GINT_TO_POINTER(reference));
    }
}

static gboolean table_fields_allowed(lua_State* state, int index, const char* const* allowed) {
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

static GVariant* workspace_selector(lua_State* state, int index, gboolean allow_name);

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

static int lua_workspace_action(lua_State* state) {
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

static int lua_generic_api_action(lua_State* state) {
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

    if (g_str_equal(method, "grant.list") && lua_gettop(state) != 0)
        return luaL_error(state, "grant.list takes no arguments");
    if ((g_str_equal(method, "privacy.stop_sharing") ||
         g_str_equal(method, "privacy.stop_recording")) &&
        lua_gettop(state) != 0)
        return luaL_error(state, "%s takes no arguments", method);
    if (g_str_equal(method, "session.lock") && lua_gettop(state) != 0)
        return luaL_error(state, "session.lock takes no arguments");
    if (g_str_equal(method, "session.logout") && lua_gettop(state) != 0)
        return luaL_error(state, "session.logout takes no arguments");
    if (g_str_equal(method, "grant.revoke")) {
        static const char* const fields[] = {"kind", "id", "created_at", NULL};
        if (lua_gettop(state) != 1 || !lua_istable(state, 1) ||
            !table_fields_allowed(state, 1, fields))
            return luaL_error(state, "grant.revoke requires kind and id, with optional created_at");
        lua_getfield(state, 1, "kind");
        gboolean valid_kind = lua_type(state, -1) == LUA_TSTRING &&
                              (g_str_equal(lua_tostring(state, -1), "screen-cast") ||
                               g_str_equal(lua_tostring(state, -1), "remote-desktop"));
        lua_pop(state, 1);
        lua_getfield(state, 1, "id");
        gboolean valid_id =
            lua_type(state, -1) == LUA_TSTRING && lua_tostring(state, -1)[0] != '\0';
        lua_pop(state, 1);
        if (!valid_kind || !valid_id)
            return luaL_error(state, "grant.revoke requires a supported kind and opaque id");
        lua_getfield(state, 1, "created_at");
        gboolean valid_created_at =
            lua_isnil(state, -1) || (lua_isinteger(state, -1) && lua_tointeger(state, -1) >= 0);
        lua_pop(state, 1);
        if (!valid_created_at)
            return luaL_error(state, "grant.revoke created_at must be a nonnegative integer");
    }
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

static int lua_input_text_target(lua_State* state) {
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

static int lua_menu_context_begin_move(lua_State* state) {
    return lua_menu_context_begin(state, FALSE);
}

static int lua_menu_context_begin_resize(lua_State* state) {
    return lua_menu_context_begin(state, TRUE);
}

static int lua_windows_snap_context(lua_State* state) {
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

static int lua_snap_context_commit(lua_State* state) {
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

static gboolean lua_dense_array(lua_State* state, int index, lua_Integer maximum) {
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

static int lua_window_drag_offer_targets(lua_State* state) {
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

static int lua_text_target_index(lua_State* state) {
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

static int lua_animation_preview_method(lua_State* state) {
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

static gboolean animation_name_valid(const char* name) {
    gsize length = name ? strlen(name) : 0;
    if (length == 0 || length > 80)
        return FALSE;
    for (gsize i = 0; i < length; i++) {
        unsigned char character = (unsigned char)name[i];
        if (!g_ascii_isalnum(character) && character != '_' && character != '-')
            return FALSE;
    }
    return TRUE;
}

static GVariant* animation_records(LuaConfig* config) {
    return config && config->animation_snapshot
               ? g_variant_lookup_value(config->animation_snapshot, "animations",
                                        G_VARIANT_TYPE("aa{sv}"))
               : NULL;
}

static GVariant* policy_array_record(GVariant* array, gsize index) {
    if (!array || index >= g_variant_n_children(array))
        return NULL;
    GVariant* child = g_variant_get_child_value(array, index);
    if (g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)) {
        GVariant* value = g_variant_get_variant(child);
        g_variant_unref(child);
        return value;
    }
    return child;
}

static gboolean policy_rule_matches(GVariant* match, const char* type, const char* app_id,
                                    const char* title, const char* layer, gboolean focused,
                                    GError** error) {
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, match);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean matched = FALSE;
        if (g_str_equal(key, "type")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
                      g_str_equal(g_variant_get_string(value, NULL), type);
        } else if (g_str_equal(key, "focused")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                      g_variant_get_boolean(value) == focused;
        } else if (g_str_equal(key, "app-id") || g_str_equal(key, "app_id") ||
                   g_str_equal(key, "title") || g_str_equal(key, "layer")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            const char* candidate = g_str_equal(key, "app-id") || g_str_equal(key, "app_id")
                                        ? app_id
                                    : g_str_equal(key, "title") ? title
                                                                : layer;
            matched = pattern && candidate &&
                      gnoblin_config_window_pattern_match(pattern, candidate, error);
            if (error && *error) {
                g_variant_unref(value);
                return FALSE;
            }
        }
        /* Workspace and other match fields have no value in a namespace-only
         * query, matching the old shell bridge's synthetic layer record. */
        g_variant_unref(value);
        if (!matched)
            return FALSE;
    }
    return TRUE;
}

static gboolean builtin_animation_supports(const char* name, const char* event) {
    if (g_str_equal(name, "none"))
        return TRUE;
    if (g_str_equal(name, "fade") || g_str_equal(name, "gnome"))
        return g_str_equal(event, "minimize") || g_str_equal(event, "restore") ||
               g_str_equal(event, "open") || g_str_equal(event, "close") ||
               g_str_equal(event, "dialog-open") || g_str_equal(event, "dialog-close") ||
               g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close");
    if (g_str_equal(name, "slide"))
        return g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close");
    if (g_str_equal(name, "gnoblin-layer-open"))
        return g_str_equal(event, "layer-open");
    if (g_str_equal(name, "gnoblin-layer-close"))
        return g_str_equal(event, "layer-close");
    if (g_str_equal(name, "gnome-minimize"))
        return g_str_equal(event, "minimize");
    if (g_str_equal(name, "gnome-restore"))
        return g_str_equal(event, "restore");
    if (g_str_equal(name, "gnome-open"))
        return g_str_equal(event, "open");
    if (g_str_equal(name, "gnome-close"))
        return g_str_equal(event, "close");
    if (g_str_equal(name, "gnome-dialog-open"))
        return g_str_equal(event, "dialog-open");
    if (g_str_equal(name, "gnome-dialog-close"))
        return g_str_equal(event, "dialog-close");
    if (g_str_equal(name, "gnome-workspace-switch"))
        return g_str_equal(event, "workspace-switch");
    if (g_str_equal(name, "gnome-resize"))
        return g_str_equal(event, "resize");
    if (g_str_equal(name, "gnoblin-shadow-change"))
        return g_str_equal(event, "shadow-change");
    return FALSE;
}

static gboolean policy_animation_supports(GVariant* document, const char* name, const char* event) {
    g_autoptr(GVariant) animations = g_variant_lookup_value(document, "animations", NULL);
    for (gsize i = 0; animations && i < g_variant_n_children(animations); i++) {
        g_autoptr(GVariant) record = policy_array_record(animations, i);
        const char* record_name = NULL;
        const char* record_event = NULL;
        gboolean enabled = TRUE;
        if (!record || !g_variant_is_of_type(record, G_VARIANT_TYPE_VARDICT) ||
            !g_variant_lookup(record, "name", "&s", &record_name) ||
            !g_str_equal(record_name, name))
            continue;
        g_variant_lookup(record, "event", "&s", &record_event);
        g_variant_lookup(record, "enable", "b", &enabled);
        return enabled && g_strcmp0(record_event, event) == 0;
    }
    return builtin_animation_supports(name, event);
}

static GVariant* policy_layer_phase(GVariant* document, const char* namespace, gboolean opening,
                                    GError** error) {
    const char* event = opening ? "layer-open" : "layer-close";
    const char* alias = opening ? "in" : "out";
    const char* selected = "slide";
    g_autoptr(GVariant) selected_duration = NULL;
    g_autoptr(GVariant) selected_easing = NULL;
    g_autoptr(GVariant) rules = g_variant_lookup_value(document, "window-rules", NULL);
    for (gsize i = 0; rules && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) rule = policy_array_record(rules, i);
        if (!rule || !g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT) ||
            !policy_rule_matches(match, "layer", "", "", namespace, FALSE, error)) {
            if (error && *error)
                return NULL;
            continue;
        }
        g_autoptr(GVariant) animation = g_variant_lookup_value(rule, "animation", NULL);
        if (!animation)
            continue;
        const char* name = NULL;
        gboolean has_override = FALSE;
        if (g_variant_is_of_type(animation, G_VARIANT_TYPE_STRING)) {
            name = g_variant_get_string(animation, NULL);
        } else if (g_variant_is_of_type(animation, G_VARIANT_TYPE_VARDICT)) {
            g_autoptr(GVariant) phase_name = g_variant_lookup_value(animation, event, NULL);
            if (!phase_name) {
                phase_name =
                    g_variant_lookup_value(animation, opening ? "layer_open" : "layer_close", NULL);
            }
            if (!phase_name)
                phase_name = g_variant_lookup_value(animation, alias, NULL);
            if (!phase_name || !g_variant_is_of_type(phase_name, G_VARIANT_TYPE_STRING))
                continue;
            name = g_variant_get_string(phase_name, NULL);
            has_override = TRUE;
        } else {
            continue;
        }
        if (!policy_animation_supports(document, name, event))
            continue;
        selected = name;
        g_clear_pointer(&selected_duration, g_variant_unref);
        g_clear_pointer(&selected_easing, g_variant_unref);
        if (has_override) {
            selected_duration = g_variant_lookup_value(animation, "duration", NULL);
            selected_easing = g_variant_lookup_value(animation, "ease", NULL);
            if (!selected_easing)
                selected_easing = g_variant_lookup_value(animation, "easing", NULL);
        }
    }

    GVariantBuilder phase;
    g_variant_builder_init(&phase, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&phase, "{sv}", "animation", g_variant_new_string(selected));
    if (selected_duration)
        g_variant_builder_add(&phase, "{sv}", "duration", selected_duration);
    if (selected_easing)
        g_variant_builder_add(&phase, "{sv}", "easing", selected_easing);
    return g_variant_ref_sink(g_variant_builder_end(&phase));
}

static GVariant* layer_animation_policy(LuaConfig* config, const char* namespace, GError** error) {
    if (!config || !config->settings_document) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua configuration is unavailable");
        return NULL;
    }
    g_autoptr(GVariant) enter =
        policy_layer_phase(config->settings_document, namespace, TRUE, error);
    if (!enter)
        return NULL;
    g_autoptr(GVariant) exit =
        policy_layer_phase(config->settings_document, namespace, FALSE, error);
    if (!exit)
        return NULL;

    /* The former policy adapter resolved window shadow against the default
     * window identity, distinct from the queried layer namespace. */
    g_autoptr(GVariant) window_shadow = g_variant_ref_sink(g_variant_new_boolean(FALSE));
    g_autoptr(GVariant) window_rules =
        g_variant_lookup_value(config->settings_document, "window-rules", NULL);
    for (gsize i = 0; window_rules && i < g_variant_n_children(window_rules); i++) {
        g_autoptr(GVariant) rule = policy_array_record(window_rules, i);
        if (!rule || !g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT) ||
            !policy_rule_matches(match, "window", "", "", NULL, TRUE, error)) {
            if (error && *error)
                return NULL;
            continue;
        }
        g_autoptr(GVariant) corners = g_variant_lookup_value(rule, "corners", NULL);
        g_autoptr(GVariant) shadow =
            corners ? g_variant_lookup_value(corners, "shadow", NULL) : NULL;
        if (shadow) {
            g_clear_pointer(&window_shadow, g_variant_unref);
            window_shadow = g_variant_ref(shadow);
        }
    }

    GVariantBuilder policy;
    g_variant_builder_init(&policy, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&policy, "{sv}", "namespace", g_variant_new_string(namespace));
    g_variant_builder_add(&policy, "{sv}", "enter", enter);
    g_variant_builder_add(&policy, "{sv}", "exit", exit);
    g_variant_builder_add(&policy, "{sv}", "window_shadow", window_shadow);
    g_variant_builder_add(&policy, "{sv}", "revision",
                          g_variant_new_uint64(config->settings_revision));
    return g_variant_ref_sink(g_variant_builder_end(&policy));
}

static int lua_layers_animation_policy(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.layers.animation_policy requires one namespace string");
    const char* namespace = lua_tostring(state, 1);
    gsize length = strlen(namespace);
    if (length == 0 || length > 128 || !g_utf8_validate(namespace, length, NULL))
        return luaL_error(state, "layer namespace must be 1 to 128 bytes of UTF-8");
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) result = layer_animation_policy(config, namespace, &error);
    if (!result)
        return luaL_error(state, "%s", error ? error->message : "could not resolve layer policy");
    push_variant(state, result);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_animations_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.animations.list takes no arguments");
    g_autoptr(GVariant) records = animation_records(config);
    if (!records)
        return luaL_error(state, "native animation snapshot is unavailable");
    push_variant(state, records);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

/* Read the current native animation registry without queuing a runtime action. */
static int lua_animation_get(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.animations.get requires one animation name");
    const char* name = lua_tostring(state, 1);
    if (!animation_name_valid(name))
        return luaL_error(
            state, "animation name must be 1 to 80 ASCII letters, digits, underscores, or hyphens");
    g_autoptr(GVariant) records = animation_records(config);
    if (!records)
        return luaL_error(state, "native animation snapshot is unavailable");
    for (gsize i = 0; i < g_variant_n_children(records); i++) {
        g_autoptr(GVariant) record = g_variant_get_child_value(records, i);
        const char* record_name = NULL;
        if (!g_variant_lookup(record, "name", "&s", &record_name) ||
            !g_str_equal(record_name, name))
            continue;
        push_variant(state, record);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        return 1;
    }
    lua_pushnil(state);
    return 1;
}

static gboolean is_keybinding_action_name(const char* key) {
    if (!key || !*key)
        return FALSE;
    gboolean previous_underscore = TRUE;
    for (const unsigned char* p = (const unsigned char*)key; *p; p++) {
        if (*p == '_') {
            if (previous_underscore)
                return FALSE;
            previous_underscore = TRUE;
        } else if (g_ascii_islower(*p) || g_ascii_isdigit(*p))
            previous_underscore = FALSE;
        else
            return FALSE;
    }
    return !previous_underscore;
}

/* Public declarations use Lua identifiers. Keep keybinding action names in
 * snake_case until native control maps them to their GSettings keys. */
static void push_settings(lua_State* state, int source, const char* parent, int depth,
                          int keybinding_depth) {
    if (depth > MAX_CONFIG_DEPTH)
        luaL_error(state, "Lua config nesting exceeds 64 levels");
    luaL_checkstack(state, 6, "settings nesting");
    source = lua_absindex(state, source);
    if (!lua_istable(state, source)) {
        lua_pushvalue(state, source);
        return;
    }
    lua_newtable(state);
    int destination = lua_gettop(state);
    gboolean literal =
        parent && (!strcmp(parent, "shader-uniforms") || !strcmp(parent, "frame-renderers") ||
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart") ||
                   !strcmp(parent, "interfaces"));
    lua_pushnil(state);
    while (lua_next(state, source)) {
        gboolean keybinding_action = keybinding_depth == 2 && lua_type(state, -2) == LUA_TSTRING;
        if (keybinding_action && !is_keybinding_action_name(lua_tostring(state, -2)))
            luaL_error(state, "keybinding action names must use snake_case");
        if (lua_type(state, -2) == LUA_TSTRING && !literal && !keybinding_action) {
            char* key = g_strdup(lua_tostring(state, -2));
            g_strdelimit(key, "_", '-');
            lua_pushstring(state, key);
            g_free(key);
        } else {
            lua_pushvalue(state, -2);
        }
        lua_pushvalue(state, -1);
        lua_rawget(state, destination);
        if (!lua_isnil(state, -1))
            luaL_error(state, "duplicate setting after snake_case conversion");
        lua_pop(state, 1);
        const char* key = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        int child_keybinding_depth = key && !strcmp(key, "keybindings") ? 1
                                     : keybinding_depth == 1            ? 2
                                                                        : 0;
        push_settings(state, -2, key, depth + 1, child_keybinding_depth);
        lua_rawset(state, destination);
        lua_pop(state, 1);
    }
    if (marked_array(state, source) && lua_getmetatable(state, source))
        lua_setmetatable(state, destination);
}

static int lua_configure(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    push_settings(state, 1, NULL, 0, 0);
    lua_replace(state, 1);
    return lua_set(state);
}

static int lua_configure_call(lua_State* state) {
    lua_remove(state, 1);
    return lua_configure(state);
}

static void push_config_list(lua_State* state) {
    const char* section = lua_tostring(state, lua_upvalueindex(1));
    const char* key = lua_tostring(state, lua_upvalueindex(2));
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    if (!lua_istable(state, -1))
        luaL_error(state, "gnoblin.config must be a table");
    if (section[0]) {
        lua_getfield(state, -1, section);
        if (lua_isnil(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, section);
        }
        if (!lua_istable(state, -1))
            luaL_error(state, "%s must be a table", section);
    }
    lua_getfield(state, -1, key);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, key);
    }
    if (!lua_istable(state, -1))
        luaL_error(state, "%s must be a list", key);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), entries = 0;
    lua_pushnil(state);
    while (lua_next(state, list)) {
        if (!lua_isinteger(state, -2) || lua_tointeger(state, -2) < 1 ||
            lua_tointeger(state, -2) > count || !lua_istable(state, -1))
            luaL_error(state, "%s must be a dense list of tables", key);
        entries++;
        lua_pop(state, 1);
    }
    if (entries != count)
        luaL_error(state, "%s must be a dense list of tables", key);
}

/* Named views expose the actual loaded entries, while the stored document
 * keeps the existing ordered-list representation. */
static int lua_named_entry_index(lua_State* state) {
    const char* key = luaL_checkstring(state, 2);
    g_autofree char* internal = g_strdup(key);
    g_strdelimit(internal, "_", '-');
    lua_pushstring(state, internal);
    lua_rawget(state, lua_upvalueindex(1));
    if (!strcmp(key, "enable") && lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_pushboolean(state, TRUE);
    }
    return 1;
}

static int lua_named_entry_assign(lua_State* state) {
    const char* key = luaL_checkstring(state, 2);
    if (!strcmp(key, "name"))
        return luaL_error(state, "named entry name cannot be changed");
    g_autofree char* internal = g_strdup(key);
    g_strdelimit(internal, "_", '-');
    lua_pushstring(state, internal);
    lua_pushvalue(state, 3);
    lua_rawset(state, lua_upvalueindex(1));
    return 0;
}

static int lua_named_entry_pairs(lua_State* state) {
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_pushnil(state);
    while (lua_next(state, lua_upvalueindex(1))) {
        if (lua_type(state, -2) == LUA_TSTRING) {
            g_autofree char* public_key = g_strdup(lua_tostring(state, -2));
            g_strdelimit(public_key, "-", '_');
            lua_pushvalue(state, -1);
            lua_setfield(state, entries, public_key);
        }
        lua_pop(state, 1);
    }
    lua_getfield(state, entries, "enable");
    gboolean has_enable = !lua_isnil(state, -1);
    lua_pop(state, 1);
    if (!has_enable) {
        lua_pushboolean(state, TRUE);
        lua_setfield(state, entries, "enable");
    }
    lua_getglobal(state, "next");
    lua_pushvalue(state, entries);
    lua_pushnil(state);
    return 3;
}

static void push_named_entry_proxy(lua_State* state, int entry) {
    entry = lua_absindex(state, entry);
    lua_newtable(state);
    lua_newtable(state);
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_index, 1);
    lua_setfield(state, -2, "__index");
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_assign, 1);
    lua_setfield(state, -2, "__newindex");
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_named_entry_pairs, 1);
    lua_setfield(state, -2, "__pairs");
    lua_setmetatable(state, -2);
}

static int lua_named_view_index(lua_State* state) {
    const char* name = luaL_checkstring(state, 2);
    push_config_list(state);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches) {
            push_named_entry_proxy(state, -1);
            return 1;
        }
        lua_pop(state, 1);
    }
    lua_pushnil(state);
    return 1;
}

static int lua_named_view_pairs(lua_State* state) {
    push_config_list(state);
    int list = lua_gettop(state);
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        if (lua_type(state, -1) == LUA_TSTRING) {
            const char* name = lua_tostring(state, -1);
            push_named_entry_proxy(state, -2);
            lua_setfield(state, entries, name);
        }
        lua_pop(state, 2);
    }
    lua_getglobal(state, "next");
    lua_pushvalue(state, entries);
    lua_pushnil(state);
    return 3;
}

static int lua_named_view_assign(lua_State* state) {
    const char* name = luaL_checkstring(state, 2);
    luaL_checktype(state, 3, LUA_TTABLE);
    push_settings(state, 3, NULL, 0, 0);
    lua_newtable(state);
    int entries = lua_gettop(state);
    lua_pushvalue(state, -2);
    lua_setfield(state, entries, name);
    push_config_list(state);
    merge_named_entries(state, -1, entries);
    return 0;
}

static void install_named_view(lua_State* state, const char* key) {
    lua_newtable(state);
    lua_newtable(state);
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_index, 2);
    lua_setfield(state, -2, "__index");
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_pairs, 2);
    lua_setfield(state, -2, "__pairs");
    lua_pushstring(state, "");
    lua_pushstring(state, key);
    lua_pushcclosure(state, lua_named_view_assign, 2);
    lua_setfield(state, -2, "__newindex");
    lua_setmetatable(state, -2);
    lua_setfield(state, -2, key);
}

static void finish_named_entries(lua_State* state, int config, const char* key) {
    config = lua_absindex(state, config);
    lua_getfield(state, config, key);
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        return;
    }
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list), next = 1;
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_pushnil(state);
        lua_setfield(state, -2, "enable");
        if (disabled)
            lua_pop(state, 1);
        else
            lua_rawseti(state, list, next++);
    }
    for (lua_Integer i = next; i <= count; i++) {
        lua_pushnil(state);
        lua_rawseti(state, list, i);
    }
    lua_pop(state, 1);
}

/* Named commands merge in place; ordered rules always append. */
static int lua_declare(lua_State* state) {
    luaL_checktype(state, 1, LUA_TTABLE);
    push_settings(state, 1, NULL, 0, 0);
    int entry = lua_gettop(state);
    const char* name = NULL;
    if (lua_toboolean(state, lua_upvalueindex(3))) {
        lua_getfield(state, entry, "name");
        if (lua_type(state, -1) != LUA_TSTRING || !lua_rawlen(state, -1))
            return luaL_error(state, "declaration needs a nonempty string name");
        name = lua_tostring(state, -1);
        lua_getfield(state, entry, "enable");
        if (!lua_isnil(state, -1) && !lua_isboolean(state, -1))
            return luaL_error(state, "named entry enable must be a boolean");
        gboolean disabled = lua_isboolean(state, -1) && !lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_pushnil(state);
        lua_setfield(state, entry, "enable");
        if (disabled) {
            push_config_list(state);
            remove_named_entry(state, -1, name);
            return 0;
        }
    }
    push_config_list(state);
    int list = lua_gettop(state);
    lua_Integer count = lua_rawlen(state, list);
    for (lua_Integer i = 1; name && i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "name");
        gboolean matches =
            lua_type(state, -1) == LUA_TSTRING && !strcmp(name, lua_tostring(state, -1));
        lua_pop(state, 1);
        if (matches) {
            lua_pushnil(state);
            lua_setfield(state, -2, "enable");
            merge_table(state, -1, entry, NULL, FALSE);
            return 0;
        }
        lua_pop(state, 1);
    }
    lua_pushvalue(state, entry);
    lua_rawseti(state, list, count + 1);
    return 0;
}

static gboolean evaluate_path(lua_State*, LuaConfig*, const char*, gboolean, GError**);

static int lua_config_load(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    const char* pattern = luaL_checkstring(state, 1);
    g_autoptr(GError) error = NULL;
    g_autoptr(GPtrArray) paths =
        gnoblin_config_expand_paths(config->current_path, pattern, config->directories, &error);
    if (!paths)
        return luaL_error(state, "%s", error->message);
    for (guint i = 0; i < paths->len; i++)
        if (!evaluate_path(state, config, g_ptr_array_index(paths, i), FALSE, &error))
            return luaL_error(state, "%s", error->message);
    return 0;
}

static gboolean safe_module_name(const char* name) {
    if (!name[0] || strstr(name, ".."))
        return FALSE;
    for (const char* p = name; *p; p++)
        if (!g_ascii_isalnum(*p) && *p != '_' && *p != '-' && *p != '.')
            return FALSE;
    return TRUE;
}

static int lua_require(lua_State* state) {
    LuaConfig* config = config_from_upvalue(state);
    const char* name = luaL_checkstring(state, 1);
    if (!strcmp(name, "gnoblin")) {
        lua_getglobal(state, "gnoblin");
        return 1;
    }
    if (!safe_module_name(name))
        return luaL_error(state, "invalid Lua module name: %s", name);
    gpointer saved = g_hash_table_lookup(config->modules, name);
    if (saved) {
        lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(saved));
        return 1;
    }
    g_autofree char* relative = g_strconcat(name, ".lua", NULL);
    g_autofree char* path = resolve_path(config->current_path, relative);
    if (!g_file_test(path, G_FILE_TEST_IS_REGULAR)) {
        g_autofree char* under_lua = g_build_filename("lua", relative, NULL);
        g_free(path);
        path = resolve_path(config->current_path, under_lua);
    }
    add_path(config->paths, path);
    if (!g_file_test(path, G_FILE_TEST_IS_REGULAR))
        return luaL_error(state, "Lua module not found: %s", name);
    if (g_hash_table_contains(config->active, path))
        return luaL_error(state, "%s: config cycle", path);
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    int status = luaL_loadfilex(state, path, "t");
    if (status == LUA_OK)
        status = lua_pcall(state, 0, 1, 0);
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    if (status != LUA_OK)
        return lua_error(state);
    if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        lua_pushboolean(state, TRUE);
    }
    int reference = luaL_ref(state, LUA_REGISTRYINDEX);
    g_hash_table_insert(config->modules, g_strdup(name), GINT_TO_POINTER(reference));
    lua_rawgeti(state, LUA_REGISTRYINDEX, reference);
    return 1;
}

static GVariant* lua_window_array(LuaConfig* config) {
    if (!config || !config->window_snapshot)
        return NULL;
    return g_variant_lookup_value(config->window_snapshot, "windows", G_VARIANT_TYPE("av"));
}

static gboolean focus_history_contains(GPtrArray* history, const char* id) {
    for (guint i = 0; history && i < history->len; i++)
        if (g_str_equal(g_ptr_array_index(history, i), id))
            return TRUE;
    return FALSE;
}

static void focus_history_touch(LuaConfig* config, const char* id) {
    if (!config->focus_history_ids)
        config->focus_history_ids = g_ptr_array_new_with_free_func(g_free);
    for (guint i = 0; i < config->focus_history_ids->len; i++) {
        if (g_str_equal(g_ptr_array_index(config->focus_history_ids, i), id)) {
            g_ptr_array_remove_index(config->focus_history_ids, i);
            break;
        }
    }
    g_ptr_array_insert(config->focus_history_ids, 0, g_strdup(id));
}

static void focus_history_remove(LuaConfig* config, const char* id) {
    if (!config->focus_history_ids)
        return;
    for (guint i = config->focus_history_ids->len; i > 0; i--)
        if (g_str_equal(g_ptr_array_index(config->focus_history_ids, i - 1), id))
            g_ptr_array_remove_index(config->focus_history_ids, i - 1);
}

static void focus_history_reconcile_snapshot(LuaConfig* config, GVariant* snapshot) {
    if (!config->focus_history_ids)
        config->focus_history_ids = g_ptr_array_new_with_free_func(g_free);
    if (!snapshot) {
        g_ptr_array_set_size(config->focus_history_ids, 0);
        return;
    }

    g_autoptr(GVariant) windows = g_variant_lookup_value(snapshot, "windows", G_VARIANT_TYPE("av"));
    if (!windows) {
        g_ptr_array_set_size(config->focus_history_ids, 0);
        return;
    }

    g_autoptr(GHashTable) live_ids = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autofree char* focused_id = NULL;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        const char* id = NULL;
        gboolean focused = FALSE;
        if (!g_variant_lookup(window, "id", "&s", &id) || !id || !*id)
            continue;
        if (!g_hash_table_contains(live_ids, id)) {
            g_hash_table_add(live_ids, g_strdup(id));
            if (!focus_history_contains(config->focus_history_ids, id))
                g_ptr_array_add(config->focus_history_ids, g_strdup(id));
        }
        if (!focused_id && g_variant_lookup(window, "focused", "b", &focused) && focused)
            focused_id = g_strdup(id);
    }

    for (guint i = config->focus_history_ids->len; i > 0; i--) {
        const char* id = g_ptr_array_index(config->focus_history_ids, i - 1);
        if (!g_hash_table_contains(live_ids, id))
            g_ptr_array_remove_index(config->focus_history_ids, i - 1);
    }
    if (focused_id)
        focus_history_touch(config, focused_id);
}

static gboolean focus_history_snapshot_has_id(LuaConfig* config, const char* id) {
    g_autoptr(GVariant) windows = lua_window_array(config);
    for (gsize i = 0; windows && i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        const char* window_id = NULL;
        if (g_variant_lookup(window, "id", "&s", &window_id) && g_str_equal(window_id, id))
            return TRUE;
    }
    return FALSE;
}

static void focus_history_update_event(LuaConfig* config, const char* event, GVariant* payload) {
    const char* id = NULL;
    if (g_str_equal(event, "gnoblin.window.closed")) {
        if (g_variant_lookup(payload, "window_id", "&s", &id) && id && *id)
            focus_history_remove(config, id);
        return;
    }
    if (!g_str_equal(event, "gnoblin.window.focused") ||
        !g_variant_lookup(payload, "window_id", "&s", &id) || !id || !*id)
        return;

    g_autoptr(GVariant) window = g_variant_lookup_value(payload, "window", G_VARIANT_TYPE_VARDICT);
    const char* record_id = NULL;
    gboolean focused = FALSE;
    if (window && g_variant_lookup(window, "id", "&s", &record_id) && g_str_equal(record_id, id) &&
        g_variant_lookup(window, "focused", "b", &focused) && focused &&
        focus_history_snapshot_has_id(config, id))
        focus_history_touch(config, id);
}

static gboolean focus_history_matches_filter(GVariant* window, lua_State* state, int filter) {
    static const char* fields[] = {"workspace_id", "monitor_id"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            const char* expected = lua_tostring(state, -1);
            const char* actual = NULL;
            gboolean matches = expected && g_variant_lookup(window, fields[i], "&s", &actual) &&
                               g_str_equal(actual, expected);
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

static int lua_focus_history(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.focus.history takes no arguments or one filter table");
    lua_Integer limit = 50;
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"workspace_id", "monitor_id", "limit"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || (g_str_equal(key, "limit")
                               ? (!lua_isinteger(state, -1) || lua_tointeger(state, -1) < 1 ||
                                  lua_tointeger(state, -1) > 256)
                               : lua_type(state, -1) != LUA_TSTRING))
                return luaL_error(state, "invalid gnoblin.focus.history filter field or value");
            lua_pop(state, 1);
        }
        lua_getfield(state, 1, "limit");
        if (!lua_isnil(state, -1))
            limit = lua_tointeger(state, -1);
        lua_pop(state, 1);
    }
    if (!config || !config->window_snapshot || !config->focus_history_ids)
        return luaL_error(state, "native focus history is unavailable");

    g_autoptr(GVariant) windows = lua_window_array(config);
    guint result_limit = (guint)limit;
    lua_newtable(state);
    mark_array_table(state, -1);
    guint result_index = 1;
    for (guint history_index = 0;
         windows && history_index < config->focus_history_ids->len && result_index <= result_limit;
         history_index++) {
        const char* wanted_id = g_ptr_array_index(config->focus_history_ids, history_index);
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* id = NULL;
            if (!g_variant_lookup(window, "id", "&s", &id) || !g_str_equal(id, wanted_id))
                continue;
            if (!lua_istable(state, 1) || focus_history_matches_filter(window, state, 1)) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                lua_rawseti(state, -2, result_index++);
            }
            break;
        }
    }
    return 1;
}

static gboolean window_matches_filter(GVariant* window, lua_State* state, int filter) {
    static const char* fields[] = {"app_id", "title", "focused", "workspace_id", "monitor_id"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            g_autoptr(GVariant) value = g_variant_lookup_value(window, fields[i], NULL);
            gboolean matches = FALSE;
            if (i == 2) {
                matches = value && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                          lua_isboolean(state, -1) &&
                          g_variant_get_boolean(value) == lua_toboolean(state, -1);
            } else if (lua_type(state, -1) == LUA_TSTRING && value &&
                       g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* expected = lua_tostring(state, -1);
                const char* actual = g_variant_get_string(value, NULL);
                if (i == 1) {
                    g_autofree char* folded_expected = g_utf8_casefold(expected, -1);
                    g_autofree char* folded_actual = g_utf8_casefold(actual, -1);
                    matches = folded_expected && folded_actual &&
                              strstr(folded_actual, folded_expected) != NULL;
                } else {
                    matches = g_str_equal(actual, expected);
                }
            }
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

static int lua_windows_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.windows.list takes no arguments or one filter table");
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"app_id", "title", "focused", "workspace_id", "monitor_id"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_tostring(state, -2);
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || (g_str_equal(key, "focused") ? !lua_isboolean(state, -1)
                                                       : lua_type(state, -1) != LUA_TSTRING))
                return luaL_error(state, "invalid gnoblin.windows.list filter field or value");
            lua_pop(state, 1);
        }
    }
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");

    g_autoptr(GVariant) windows = lua_window_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    if (!windows)
        return 1;
    guint result_index = 1;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                         ? g_variant_get_variant(wrapped)
                                         : g_variant_ref(wrapped);
        if (lua_istable(state, 1) && !window_matches_filter(window, state, 1))
            continue;
        push_variant(state, window);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, result_index++);
    }
    return 1;
}

static int lua_windows_focused(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.windows.focused takes no arguments");
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");
    g_autoptr(GVariant) windows = lua_window_array(config);
    if (windows) {
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            gboolean focused = FALSE;
            if (g_variant_lookup(window, "focused", "b", &focused) && focused) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_windows_by_id(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.windows.by_id requires one string ID");
    const char* id = lua_tostring(state, 1);
    if (!config || !config->window_snapshot)
        return luaL_error(state, "native window snapshot is unavailable");
    g_autoptr(GVariant) windows = lua_window_array(config);
    if (windows) {
        for (gsize i = 0; i < g_variant_n_children(windows); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(windows, i);
            g_autoptr(GVariant) window = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* window_id = NULL;
            if (g_variant_lookup(window, "id", "&s", &window_id) && g_str_equal(id, window_id)) {
                push_variant(state, window);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static GVariant* lua_workspace_array(LuaConfig* config) {
    if (!config || !config->workspace_snapshot)
        return NULL;
    return g_variant_lookup_value(config->workspace_snapshot, "workspaces", G_VARIANT_TYPE("av"));
}

static int lua_workspaces_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.list takes no arguments");
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    if (!workspaces)
        return 1;
    for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
        g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                            ? g_variant_get_variant(wrapped)
                                            : g_variant_ref(wrapped);
        push_variant(state, workspace);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_workspaces_active(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.workspaces.active takes no arguments");
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    if (workspaces) {
        for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                                ? g_variant_get_variant(wrapped)
                                                : g_variant_ref(wrapped);
            gboolean active = FALSE;
            if (g_variant_lookup(workspace, "active", "b", &active) && active) {
                push_variant(state, workspace);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static int lua_workspaces_by_id(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING)
        return luaL_error(state, "gnoblin.workspaces.by_id requires one string ID");
    const char* id = lua_tostring(state, 1);
    if (!config || !config->workspace_snapshot)
        return luaL_error(state, "native workspace snapshot is unavailable");

    g_autoptr(GVariant) workspaces = lua_workspace_array(config);
    if (workspaces) {
        for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(workspaces, i);
            g_autoptr(GVariant) workspace = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                                ? g_variant_get_variant(wrapped)
                                                : g_variant_ref(wrapped);
            const char* workspace_id = NULL;
            if (g_variant_lookup(workspace, "id", "&s", &workspace_id) &&
                g_str_equal(id, workspace_id)) {
                push_variant(state, workspace);
                push_readonly_copy(state, -1);
                lua_remove(state, -2);
                return 1;
            }
        }
    }
    lua_pushnil(state);
    return 1;
}

static GVariant* lua_monitor_array(LuaConfig* config) {
    if (!config || !config->monitor_snapshot)
        return NULL;
    return g_variant_lookup_value(config->monitor_snapshot, "monitors", G_VARIANT_TYPE("av"));
}

static int lua_monitors_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.list takes no arguments");
    if (!config || !config->monitor_snapshot)
        return luaL_error(state, "native monitor snapshot is unavailable");

    g_autoptr(GVariant) monitors = lua_monitor_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; monitors && i < g_variant_n_children(monitors); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(monitors, i);
        g_autoptr(GVariant) monitor = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(wrapped)
                                          : g_variant_ref(wrapped);
        push_variant(state, monitor);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
    return 1;
}

static int lua_monitors_primary(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.monitors.primary takes no arguments");
    if (!config || !config->monitor_snapshot)
        return luaL_error(state, "native monitor snapshot is unavailable");

    g_autoptr(GVariant) monitors = lua_monitor_array(config);
    for (gsize i = 0; monitors && i < g_variant_n_children(monitors); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(monitors, i);
        g_autoptr(GVariant) monitor = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(wrapped)
                                          : g_variant_ref(wrapped);
        gboolean primary = FALSE;
        if (g_variant_lookup(monitor, "primary", "b", &primary) && primary) {
            push_variant(state, monitor);
            push_readonly_copy(state, -1);
            lua_remove(state, -2);
            return 1;
        }
    }
    lua_pushnil(state);
    return 1;
}

static GVariant* lua_layer_array(LuaConfig* config) {
    if (!config || !config->layer_snapshot)
        return NULL;
    return g_variant_lookup_value(config->layer_snapshot, "layers", G_VARIANT_TYPE("av"));
}

static gboolean layer_matches_filter(GVariant* layer, lua_State* state, int filter) {
    static const char* fields[] = {"monitor_id", "namespace", "layer"};
    filter = lua_absindex(state, filter);
    for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
        lua_getfield(state, filter, fields[i]);
        if (!lua_isnil(state, -1)) {
            const char* expected = lua_tostring(state, -1);
            const char* actual = NULL;
            gboolean matches = expected && g_variant_lookup(layer, fields[i], "&s", &actual) &&
                               g_str_equal(actual, expected);
            lua_pop(state, 1);
            if (!matches)
                return FALSE;
        } else {
            lua_pop(state, 1);
        }
    }
    return TRUE;
}

static int lua_layers_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.layers.list takes no arguments or one filter table");
    if (lua_istable(state, 1)) {
        static const char* fields[] = {"monitor_id", "namespace", "layer"};
        lua_pushnil(state);
        while (lua_next(state, 1)) {
            const char* key = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            gboolean known = FALSE;
            for (guint i = 0; key && i < G_N_ELEMENTS(fields); i++)
                known |= g_str_equal(key, fields[i]);
            if (!known || lua_type(state, -1) != LUA_TSTRING)
                return luaL_error(state, "invalid gnoblin.layers.list filter field or value");
            lua_pop(state, 1);
        }
    }
    if (!config || !config->layer_snapshot)
        return luaL_error(state, "native layer snapshot is unavailable");

    g_autoptr(GVariant) layers = lua_layer_array(config);
    lua_newtable(state);
    mark_array_table(state, -1);
    guint result_index = 1;
    for (gsize i = 0; layers && i < g_variant_n_children(layers); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(layers, i);
        g_autoptr(GVariant) layer = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                        ? g_variant_get_variant(wrapped)
                                        : g_variant_ref(wrapped);
        if (lua_istable(state, 1) && !layer_matches_filter(layer, state, 1))
            continue;
        push_variant(state, layer);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, result_index++);
    }
    return 1;
}

static int lua_capabilities_list(lua_State* state) {
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

static int lua_permissions_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.list takes no arguments");
    if (!config || !config->settings_document)
        return luaL_error(state, "committed permission policy is unavailable");
    g_autofree char* path = gnoblin_config_path();
    g_autoptr(GVariant) snapshot = gnoblin_permission_policy_list(config->settings_document, path);
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_permissions_policy(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.permissions.policy takes no arguments");
    if (!config || !config->settings_document)
        return luaL_error(state, "committed permission policy is unavailable");
    g_autoptr(GVariant) snapshot =
        gnoblin_permission_policy_snapshot(config->settings_document, config->settings_revision);
    if (!snapshot)
        return luaL_error(state, "committed permission policy is unavailable");
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_privacy_state(lua_State* state) {
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

static int lua_session_activity(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.activity takes no arguments");
    if (!config || !config->session_activity_snapshot)
        return luaL_error(state, "native session activity snapshot is unavailable");
    gboolean available = FALSE;
    gboolean idle = FALSE;
    guint64 threshold_ms = 0;
    guint64 idle_for_ms = 0;
    g_variant_lookup(config->session_activity_snapshot, "available", "b", &available);
    g_variant_lookup(config->session_activity_snapshot, "idle", "b", &idle);
    g_variant_lookup(config->session_activity_snapshot, "threshold_ms", "t", &threshold_ms);
    g_variant_lookup(config->session_activity_snapshot, "idle_for_ms", "t", &idle_for_ms);
    if (available && idle && config->session_activity_sampled_at_us > 0) {
        guint64 elapsed_ms =
            (guint64)MAX(0, g_get_monotonic_time() - config->session_activity_sampled_at_us) / 1000;
        idle_for_ms =
            G_MAXUINT64 - idle_for_ms < elapsed_ms ? G_MAXUINT64 : idle_for_ms + elapsed_ms;
    }
    GVariantBuilder snapshot_builder;
    g_variant_builder_init(&snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot_builder, "{sv}", "available", g_variant_new_boolean(available));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle", g_variant_new_boolean(idle));
    g_variant_builder_add(&snapshot_builder, "{sv}", "threshold_ms",
                          g_variant_new_uint64(threshold_ms));
    g_variant_builder_add(&snapshot_builder, "{sv}", "idle_for_ms",
                          g_variant_new_uint64(idle_for_ms));
    g_variant_builder_add(&snapshot_builder, "{sv}", "revision",
                          g_variant_new_uint64(config->session_activity_revision));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&snapshot_builder));
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_session_status(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.session.status takes no arguments");
    gboolean lock_available = FALSE;
    const char* lock_state = NULL;
    if (config && config->session_lock_snapshot) {
        g_variant_lookup(config->session_lock_snapshot, "lock_available", "b", &lock_available);
        g_variant_lookup(config->session_lock_snapshot, "lock_state", "&s", &lock_state);
    }
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "state", g_variant_new_string("running"));
    g_variant_builder_add(&builder, "{sv}", "lock_available",
                          g_variant_new_boolean(lock_available));
    if (lock_available && lock_state)
        g_variant_builder_add(&builder, "{sv}", "lock_state", g_variant_new_string(lock_state));
    g_autoptr(GVariant) status = g_variant_ref_sink(g_variant_builder_end(&builder));
    push_variant(state, status);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_runtime_status(lua_State* state) {
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

static int lua_permissions_check(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* capability = NULL;
    const char* identity = NULL;
    if (lua_gettop(state) == 2 && lua_type(state, 1) == LUA_TSTRING &&
        lua_type(state, 2) == LUA_TSTRING) {
        capability = lua_tostring(state, 1);
        identity = lua_tostring(state, 2);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        static const char* const fields[] = {"capability", "identity", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "gnoblin.permissions.check accepts capability and identity");
        lua_getfield(state, 1, "capability");
        capability = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
        lua_getfield(state, 1, "identity");
        identity = lua_type(state, -1) == LUA_TSTRING ? lua_tostring(state, -1) : NULL;
    } else {
        return luaL_error(state, "gnoblin.permissions.check requires capability and identity");
    }
    if (!capability || !identity) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state,
                          "gnoblin.permissions.check requires string capability and identity");
    }
    if (!gnoblin_permission_capability_supported(capability)) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state, "unsupported permission capability");
    }
    if (!config || !config->settings_document) {
        if (lua_gettop(state) == 3)
            lua_pop(state, 2);
        return luaL_error(state, "committed permission policy is unavailable");
    }
    g_auto(GnoblinPermission) decision =
        gnoblin_permission_policy_evaluate(config->settings_document, capability, identity);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(
        &result, "{sv}", "level",
        g_variant_new_string(decision.level == GNOBLIN_PERMISSION_ASK     ? "ask"
                             : decision.level == GNOBLIN_PERMISSION_ALLOW ? "allow"
                             : decision.level == GNOBLIN_PERMISSION_DENY  ? "deny"
                                                                          : "default"));
    g_variant_builder_add(&result, "{sv}", "rule",
                          g_variant_new_string(decision.rule ? decision.rule : ""));
    static const char* const empty_monitors[] = {NULL};
    const char* const* monitor_values =
        decision.monitors ? (const char* const*)decision.monitors : empty_monitors;
    g_variant_builder_add(&result, "{sv}", "monitors", g_variant_new_strv(monitor_values, -1));
    GVariantBuilder devices;
    g_variant_builder_init(&devices, G_VARIANT_TYPE_STRING_ARRAY);
    if (decision.devices & 1)
        g_variant_builder_add(&devices, "s", "keyboard");
    if (decision.devices & 2)
        g_variant_builder_add(&devices, "s", "pointer");
    if (decision.devices & 4)
        g_variant_builder_add(&devices, "s", "touchscreen");
    g_variant_builder_add(&result, "{sv}", "devices", g_variant_builder_end(&devices));
    g_variant_builder_add(&result, "{sv}", "clipboard", g_variant_new_boolean(decision.clipboard));
    g_variant_builder_add(&result, "{sv}", "revision",
                          g_variant_new_uint64(config->settings_revision));
    g_autoptr(GVariant) snapshot = g_variant_ref_sink(g_variant_builder_end(&result));
    if (lua_gettop(state) == 3)
        lua_pop(state, 2);
    push_variant(state, snapshot);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    return 1;
}

static int lua_input_devices(lua_State* state) {
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

static int lua_input_sources(lua_State* state) {
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

static int lua_input_current_source(lua_State* state) {
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

static int lua_input_orientation_lock(lua_State* state) {
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

static int lua_shortcuts_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
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

static int lua_launches_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.launches.list takes no arguments");
    if (!config || !config->launch_snapshot)
        return luaL_error(state, "native launch snapshot is unavailable");
    push_launches_array(state, config);
    return 1;
}

static int lua_launches_snapshot(lua_State* state) {
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

static int lua_portal_grants(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    const char* kind_filter = NULL;
    if (lua_gettop(state) > 1 || (lua_gettop(state) == 1 && !lua_istable(state, 1)))
        return luaL_error(state, "gnoblin.portals.grants takes no arguments or a filter table");
    if (lua_istable(state, 1)) {
        static const char* const fields[] = {"kind", NULL};
        if (!table_fields_allowed(state, 1, fields))
            return luaL_error(state, "gnoblin.portals.grants filter accepts only kind");
        lua_getfield(state, 1, "kind");
        if (!lua_isnil(state, -1) && lua_type(state, -1) != LUA_TSTRING)
            return luaL_error(state, "portal grant kind must be a string");
        kind_filter = lua_tostring(state, -1);
        lua_pop(state, 1);
    }
    if (kind_filter && !g_str_equal(kind_filter, "screen-cast") &&
        !g_str_equal(kind_filter, "remote-desktop"))
        return luaL_error(state, "grant kind must be screen-cast or remote-desktop");
    if (!config || !config->portal_grant_snapshot)
        return luaL_error(state, "native portal grant snapshot is unavailable");

    g_autoptr(GVariant) grants =
        g_variant_lookup_value(config->portal_grant_snapshot, "grants", G_VARIANT_TYPE("aa{sv}"));
    lua_newtable(state);
    mark_array_table(state, -1);
    for (gsize i = 0; grants && i < g_variant_n_children(grants); i++) {
        g_autoptr(GVariant) grant = g_variant_get_child_value(grants, i);
        const char* kind = NULL;
        g_variant_lookup(grant, "kind", "&s", &kind);
        if (kind_filter && (!kind || !g_str_equal(kind_filter, kind)))
            continue;
        push_variant(state, grant);
        lua_pushinteger(state, (lua_Integer)config->portal_grant_revision);
        lua_setfield(state, -2, "revision");
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, lua_rawlen(state, -2) + 1);
    }
    return 1;
}

static int call_launch_operation(lua_State* state, const char* method, int arguments) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    lua_pushlightuserdata(state, config);
    lua_pushstring(state, method);
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_insert(state, arguments);
    lua_call(state, 1, 1);
    return 1;
}

static int lua_launches_begin(lua_State* state) {
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
    return call_launch_operation(state, "launch.begin", arguments);
}

static int lua_launches_finish(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1) ||
        g_utf8_strlen(lua_tostring(state, 1), -1) > 128)
        return luaL_error(
            state, "gnoblin.launches.finish requires a non-empty token of at most 128 characters");
    lua_newtable(state);
    lua_pushvalue(state, 1);
    lua_setfield(state, -2, "token");
    int arguments = lua_absindex(state, -1);
    return call_launch_operation(state, "launch.end", arguments);
}

static gint compare_string_pointers(gconstpointer left, gconstpointer right) {
    return g_strcmp0(*(const gchar* const*)left, *(const gchar* const*)right);
}

static int lua_shortcut_actions(lua_State* state) {
    static const struct {
        const char* group;
        const char* schema_id;
    } schemas[] = {
        {"wm", "org.gnome.desktop.wm.keybindings"},
        {"mutter", "org.gnome.mutter.keybindings"},
        {"wayland", "org.gnome.mutter.wayland.keybindings"},
    };
    if (lua_gettop(state) > 1 ||
        (lua_gettop(state) == 1 && !lua_isnil(state, 1) && lua_type(state, 1) != LUA_TSTRING))
        return luaL_error(state, "gnoblin.shortcuts.actions takes an optional group string");

    const char* requested_group = lua_isnoneornil(state, 1) ? NULL : lua_tostring(state, 1);
    if (requested_group) {
        gboolean known = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(schemas); i++)
            known |= g_str_equal(requested_group, schemas[i].group);
        if (!known)
            return luaL_error(
                state, "unknown shortcut action group; expected 'wm', 'mutter', or 'wayland'");
    }

    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    if (!source)
        return luaL_error(state, "GSettings schema source is unavailable");

    lua_newtable(state);
    int result = lua_absindex(state, -1);
    guint result_index = 1;
    for (guint group_index = 0; group_index < G_N_ELEMENTS(schemas); group_index++) {
        const char* group = schemas[group_index].group;
        if (requested_group && !g_str_equal(requested_group, group))
            continue;

        GSettingsSchema* schema =
            g_settings_schema_source_lookup(source, schemas[group_index].schema_id, TRUE);
        if (!schema) {
            if (requested_group)
                return luaL_error(state, "shortcut schema is not installed for group '%s'", group);
            continue;
        }

        gchar** keys = g_settings_schema_list_keys(schema);
        if (!keys) {
            g_settings_schema_unref(schema);
            continue;
        }
        qsort(keys, g_strv_length(keys), sizeof(*keys), compare_string_pointers);
        for (guint key_index = 0; keys[key_index]; key_index++) {
            GSettingsSchemaKey* schema_key = g_settings_schema_get_key(schema, keys[key_index]);
            if (!schema_key)
                continue;
            if (!g_variant_type_equal(g_settings_schema_key_get_value_type(schema_key),
                                      G_VARIANT_TYPE_STRING_ARRAY)) {
                g_settings_schema_key_unref(schema_key);
                continue;
            }

            GVariant* defaults = g_settings_schema_key_get_default_value(schema_key);
            if (!defaults || !g_variant_is_of_type(defaults, G_VARIANT_TYPE_STRING_ARRAY)) {
                g_clear_pointer(&defaults, g_variant_unref);
                g_settings_schema_key_unref(schema_key);
                continue;
            }

            g_autofree char* public_key = g_strdup(keys[key_index]);
            g_strdelimit(public_key, "-", '_');
            g_autofree char* id = g_strdup_printf("%s.%s", group, public_key);
            const char* description = g_settings_schema_key_get_description(schema_key);

            lua_newtable(state);
            lua_pushstring(state, id);
            lua_setfield(state, -2, "id");
            lua_pushstring(state, group);
            lua_setfield(state, -2, "group");
            lua_pushstring(state, public_key);
            lua_setfield(state, -2, "key");
            if (description && *description) {
                lua_pushstring(state, description);
                lua_setfield(state, -2, "description");
            }
            lua_newtable(state);
            for (gsize binding_index = 0; binding_index < g_variant_n_children(defaults);
                 binding_index++) {
                g_autoptr(GVariant) binding = g_variant_get_child_value(defaults, binding_index);
                lua_pushstring(state, g_variant_get_string(binding, NULL));
                lua_rawseti(state, -2, binding_index + 1);
            }
            lua_setfield(state, -2, "default_bindings");
            push_readonly_copy(state, -1);
            lua_remove(state, -2);
            lua_rawseti(state, result, result_index++);

            g_variant_unref(defaults);
            g_settings_schema_key_unref(schema_key);
        }
        g_strfreev(keys);
        g_settings_schema_unref(schema);
    }

    push_readonly_copy(state, result);
    lua_remove(state, result);
    return 1;
}

typedef struct {
    char* gnoblin;
    char* gnome;
    char* mutter;
    char* git_remote;
    char* git_sha;
    char* build_id;
} VersionIdentity;

static void version_identity_clear(VersionIdentity* identity) {
    g_clear_pointer(&identity->gnoblin, g_free);
    g_clear_pointer(&identity->gnome, g_free);
    g_clear_pointer(&identity->mutter, g_free);
    g_clear_pointer(&identity->git_remote, g_free);
    g_clear_pointer(&identity->git_sha, g_free);
    g_clear_pointer(&identity->build_id, g_free);
}

static char* sanitize_git_remote(const char* remote) {
    if (!remote || !*remote)
        return g_strdup("unknown");

    g_autoptr(GError) error = NULL;
    g_autoptr(GUri) uri = g_uri_parse(remote, G_URI_FLAGS_NONE, &error);
    if (uri && g_uri_get_scheme(uri))
        return g_uri_join(G_URI_FLAGS_NONE, g_uri_get_scheme(uri), NULL, g_uri_get_host(uri),
                          g_uri_get_port(uri), g_uri_get_path(uri), NULL, NULL);

    /* SSH's scp-like spelling has no URI scheme. Drop any user prefix rather
     * than exposing a possibly credential-bearing value. */
    const char* at = strchr(remote, '@');
    return g_strdup(at ? at + 1 : remote);
}

static char* executable_prefix(void) {
    g_autofree char* path = g_file_read_link("/proc/self/exe", NULL);
    if (!path)
        return NULL;
    g_autofree char* directory = g_path_get_dirname(path);
    return g_path_get_dirname(directory);
}

static void version_identity_load_key_file(GKeyFile* key_file, VersionIdentity* identity) {
    identity->gnoblin = g_key_file_get_string(key_file, "version", "gnoblin", NULL);
    identity->gnome = g_key_file_get_string(key_file, "version", "gnome", NULL);
    identity->mutter = g_key_file_get_string(key_file, "version", "mutter", NULL);
    identity->git_remote = g_key_file_get_string(key_file, "version", "git_remote", NULL);
    identity->git_sha = g_key_file_get_string(key_file, "version", "git_sha", NULL);
    identity->build_id = g_key_file_get_string(key_file, "version", "build_id", NULL);
}

static void load_installed_identity(VersionIdentity* identity) {
    g_autoptr(GPtrArray) candidates = g_ptr_array_new_with_free_func(g_free);
    const char* override = g_getenv("GNOBLIN_VERSION_METADATA_FILE");
    if (override && *override)
        g_ptr_array_add(candidates, g_strdup(override));
    g_autofree char* prefix = executable_prefix();
    if (prefix)
        g_ptr_array_add(candidates,
                        g_build_filename(prefix, "share", "gnoblin", "version.ini", NULL));
    const char* const* data_dirs = g_get_system_data_dirs();
    for (guint i = 0; data_dirs[i]; i++)
        g_ptr_array_add(candidates, g_build_filename(data_dirs[i], "gnoblin", "version.ini", NULL));

    for (guint i = 0; i < candidates->len; i++) {
        const char* path = g_ptr_array_index(candidates, i);
        g_autoptr(GKeyFile) key_file = g_key_file_new();
        if (!g_key_file_load_from_file(key_file, path, G_KEY_FILE_NONE, NULL))
            continue;
        version_identity_load_key_file(key_file, identity);
        return;
    }
}

static void lua_set_version_string(lua_State* state, const char* field, const char* value) {
    lua_pushstring(state, value && *value ? value : "unknown");
    lua_setfield(state, -2, field);
}

static int lua_gnoblin_version(lua_State* state) {
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.version takes no arguments");

    VersionIdentity identity = {0};
    load_installed_identity(&identity);
    g_autofree char* safe_remote = sanitize_git_remote(identity.git_remote);
    g_autofree char* api_version = NULL;
#ifdef GNOBLIN_NATIVE_CONTROL_API_MAJOR
    api_version = g_strdup_printf("%d.%d", GNOBLIN_NATIVE_CONTROL_API_MAJOR,
                                  GNOBLIN_NATIVE_CONTROL_API_MINOR);
#else
    api_version = g_strdup("unknown");
#endif

    lua_createtable(state, 0, 8);
    lua_set_version_string(state, "gnoblin", identity.gnoblin);
    lua_set_version_string(state, "gnome", identity.gnome);
    lua_set_version_string(state, "mutter", identity.mutter);
    lua_set_version_string(state, "lua", LUA_VERSION);
    lua_set_version_string(state, "api", api_version);
    lua_set_version_string(state, "git_remote", safe_remote);
    lua_set_version_string(state, "git_sha", identity.git_sha);
    lua_set_version_string(state, "build_id", identity.build_id);
    push_readonly_copy(state, -1);
    lua_remove(state, -2);
    version_identity_clear(&identity);
    return 1;
}

static void install_api(lua_State* state, LuaConfig* config) {
    luaL_openlibs(state);
    const char* blocked[] = {"os", "io", "debug", "package", "dofile", "loadfile", NULL};
    for (guint i = 0; blocked[i]; i++) {
        lua_pushnil(state);
        lua_setglobal(state, blocked[i]);
    }
    lua_newtable(state);
    lua_pushcfunction(state, lua_gnoblin_version);
    lua_setfield(state, -2, "version");
    lua_newtable(state);
    lua_setfield(state, -2, "config");
    for (guint i = 0; api_methods[i]; i++) {
        if (g_str_has_prefix(api_methods[i], "workspace."))
            continue; /* Lua exposes workspace operations through workspaces.*. */
        if (g_str_has_prefix(api_methods[i], "window."))
            continue; /* Lua exposes window operations on Window snapshots. */
        if (g_str_equal(api_methods[i], "input.select"))
            continue; /* Lua exposes source selection as input.select_source(). */
        if (g_str_equal(api_methods[i], "input.list") ||
            g_str_equal(api_methods[i], "input.current"))
            continue; /* These legacy names are snapshot reads, not operations. */
        if (g_str_equal(api_methods[i], "shortcut.capture") ||
            g_str_equal(api_methods[i], "shortcut.bind") ||
            g_str_equal(api_methods[i], "shortcut.unbind") ||
            g_str_equal(api_methods[i], "shortcut.session.end"))
            continue; /* Public collection API is gnoblin.shortcuts.capture. */
        if (g_str_equal(api_methods[i], "privacy.get"))
            continue; /* Exposed as the native snapshot read gnoblin.privacy.state(). */
        if (g_str_has_prefix(api_methods[i], "animation."))
            continue; /* Controls are gnoblin.animations.*; gnoblin.animation declares config. */
        const char* separator = strchr(api_methods[i], '.');
        g_autofree char* domain = g_strndup(api_methods[i], separator - api_methods[i]);
        const char* operation = separator + 1;
        lua_getfield(state, -1, domain);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, domain);
        }
        if (g_str_equal(api_methods[i], "session.status")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_session_status, 1);
        } else if (g_str_equal(api_methods[i], "runtime.status")) {
            lua_pushcfunction(state, lua_runtime_status);
        } else if (g_str_equal(api_methods[i], "session.activity")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_session_activity, 1);
        } else if (g_str_equal(api_methods[i], "input.text_target")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state, lua_input_text_target, 1);
        } else if (g_str_equal(api_methods[i], "permissions.list") ||
                   g_str_equal(api_methods[i], "permissions.policy") ||
                   g_str_equal(api_methods[i], "permissions.check")) {
            lua_pushlightuserdata(state, config);
            lua_pushcclosure(state,
                             g_str_equal(api_methods[i], "permissions.list") ? lua_permissions_list
                             : g_str_equal(api_methods[i], "permissions.policy")
                                 ? lua_permissions_policy
                                 : lua_permissions_check,
                             1);
        } else {
            lua_pushlightuserdata(state, config);
            lua_pushstring(state, api_methods[i]);
            lua_pushcclosure(state,
                             g_str_has_prefix(api_methods[i], "workspace.")
                                 ? lua_workspace_action
                                 : lua_generic_api_action,
                             2);
        }
        lua_setfield(state, -2, operation);
        lua_pop(state, 1);
    }
    for (guint i = 0; i < G_N_ELEMENTS(api_aliases); i++) {
        const char* separator = strchr(api_aliases[i].public_name, '.');
        g_autofree char* domain =
            g_strndup(api_aliases[i].public_name, separator - api_aliases[i].public_name);
        const char* operation = separator + 1;
        lua_getfield(state, -1, domain);
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            lua_newtable(state);
            lua_pushvalue(state, -1);
            lua_setfield(state, -3, domain);
        }
        lua_pushlightuserdata(state, config);
        lua_pushstring(state, api_aliases[i].operation);
        lua_pushcclosure(state,
                         g_str_has_prefix(api_aliases[i].operation, "workspace.")
                             ? lua_workspace_action
                             : lua_generic_api_action,
                         2);
        lua_setfield(state, -2, operation);
        lua_pop(state, 1);
    }
    lua_getfield(state, -1, "animations");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_animations_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_animation_get, 1);
    lua_setfield(state, -2, "get");
    lua_pop(state, 1);
    lua_getfield(state, -1, "privacy");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "privacy");
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_privacy_state, 1);
    lua_setfield(state, -2, "state");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_focused, 1);
    lua_setfield(state, -2, "focused");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_by_id, 1);
    lua_setfield(state, -2, "by_id");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_windows_snap_context, 1);
    lua_setfield(state, -2, "snap_context");
    lua_setfield(state, -2, "windows");
    lua_getfield(state, -1, "workspaces");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_active, 1);
    lua_setfield(state, -2, "active");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_workspaces_by_id, 1);
    lua_setfield(state, -2, "by_id");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_monitors_primary, 1);
    lua_setfield(state, -2, "primary");
    lua_setfield(state, -2, "monitors");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_layers_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_layers_animation_policy, 1);
    lua_setfield(state, -2, "animation_policy");
    lua_setfield(state, -2, "layers");
    lua_getfield(state, -1, "capabilities");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, -3, "capabilities");
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_capabilities_list, 1);
    lua_setfield(state, -2, "list");
    lua_pop(state, 1);
    lua_getfield(state, -1, "input");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_devices, 1);
    lua_setfield(state, -2, "devices");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_sources, 1);
    lua_setfield(state, -2, "sources");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_current_source, 1);
    lua_setfield(state, -2, "current_source");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_input_orientation_lock, 1);
    lua_setfield(state, -2, "orientation_lock");
    lua_getfield(state, -1, "sources");
    lua_setfield(state, -2, "list");
    lua_getfield(state, -1, "current_source");
    lua_setfield(state, -2, "current");
    lua_pop(state, 1);
    lua_newtable(state);
    lua_pushcfunction(state, lua_shortcut_actions);
    lua_setfield(state, -2, "actions");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_shortcuts_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.capture");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "capture");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.bind");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "bind");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.unbind");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "unbind");
    lua_pushlightuserdata(state, config);
    lua_pushliteral(state, "shortcut.session.end");
    lua_pushcclosure(state, lua_generic_api_action, 2);
    lua_setfield(state, -2, "end_session");
    lua_setfield(state, -2, "shortcuts");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_snapshot, 1);
    lua_setfield(state, -2, "snapshot");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_begin, 1);
    lua_setfield(state, -2, "begin");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_finish, 1);
    lua_setfield(state, -2, "finish");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_finish, 1);
    lua_setfield(state, -2, "end");
    lua_setfield(state, -2, "launches");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_portal_grants, 1);
    lua_setfield(state, -2, "grants");
    lua_setfield(state, -2, "portals");
    lua_pushcfunction(state, lua_on);
    lua_setfield(state, -2, "on");
    lua_newtable(state);
    int events = lua_absindex(state, -1);
    lua_pushcfunction(state, lua_on);
    lua_setfield(state, events, "on");
    lua_pushcfunction(state, lua_once);
    lua_setfield(state, events, "once");
    lua_newtable(state);
    lua_pushcfunction(state, lua_mutter_on);
    lua_setfield(state, -2, "on");
    lua_pushcfunction(state, lua_mutter_once);
    lua_setfield(state, -2, "once");
    lua_setfield(state, events, "mutter");
    lua_setfield(state, -2, "events");
    lua_newtable(state);
    lua_setfield(state, -2, "listeners");
    lua_newtable(state);
    lua_newtable(state);
    lua_pushcfunction(state, lua_configure_call);
    lua_setfield(state, -2, "__call");
    lua_setmetatable(state, -2);
    install_named_view(state, "shortcuts");
    install_named_view(state, "autostart");
    lua_setfield(state, -2, "configure");
    lua_pushcfunction(state, lua_snapshot);
    lua_setfield(state, -2, "snapshot");
    const struct {
        const char *name, *section, *key;
        gboolean named;
    } declarations[] = {
        {"window_rule", "", "window-rules", FALSE},
        {"permission_rule", "permissions", "rules", FALSE},
        {"animation", "", "animations", TRUE},
    };
    for (guint i = 0; i < G_N_ELEMENTS(declarations); i++) {
        lua_pushstring(state, declarations[i].section);
        lua_pushstring(state, declarations[i].key);
        lua_pushboolean(state, declarations[i].named);
        lua_pushcclosure(state, lua_declare, 3);
        lua_setfield(state, -2, declarations[i].name);
    }
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_config_load, 1);
    lua_setfield(state, -2, "load");
    lua_pushcfunction(state, lua_array);
    lua_setfield(state, -2, "array");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_gnoblin_index, 1);
    lua_setfield(state, -2, "__index");
    lua_setmetatable(state, -2);
    lua_setglobal(state, "gnoblin");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_require, 1);
    lua_setglobal(state, "require");
}

static gboolean evaluate_path(lua_State* state, LuaConfig* config, const char* given, gboolean root,
                              GError** error) {
    g_autofree char* path = g_canonicalize_filename(given, NULL);
    if (!g_str_has_suffix(path, ".lua")) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "%s: Gnoblin configuration uses Lua only. Convert this file to Lua and "
                    "give it a .lua filename.",
                    path);
        return FALSE;
    }
    add_path(config->paths, path);
    if (!g_file_test(path, G_FILE_TEST_EXISTS)) {
        if (root)
            return TRUE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_NOENT, "%s: config file does not exist",
                    path);
        return FALSE;
    }
    if (g_hash_table_contains(config->active, path)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: config cycle", path);
        return FALSE;
    }
    if (g_hash_table_size(config->active) >= MAX_CONFIG_FILES) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "config nesting exceeds 32 files");
        return FALSE;
    }
    g_hash_table_add(config->active, g_strdup(path));
    g_autofree char* old = g_steal_pointer(&config->current_path);
    config->current_path = g_strdup(path);
    int status = luaL_loadfilex(state, path, "t");
    if (status == LUA_OK)
        status = lua_pcall(state, 0, 1, 0);
    gboolean ok;
    if (status != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: %s", path, lua_error_text(state));
        lua_pop(state, 1);
        ok = FALSE;
    } else if (lua_isnil(state, -1)) {
        lua_pop(state, 1);
        ok = TRUE;
    } else if (!lua_istable(state, -1)) {
        lua_pop(state, 1);
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config must return a table or nil");
        ok = FALSE;
    } else {
        g_autoptr(GError) conversion = NULL;
        g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, FALSE, FALSE, &conversion);
        ok = value != NULL && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT);
        if (ok) {
            lua_getglobal(state, "gnoblin");
            lua_getfield(state, -1, "config");
            merge_table(state, -1, -3, NULL, TRUE);
            lua_pop(state, 3);
        } else {
            lua_pop(state, 1);
            if (conversion)
                g_propagate_error(error, g_steal_pointer(&conversion));
            else
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua config must return a table of settings");
        }
    }
    g_free(config->current_path);
    config->current_path = g_steal_pointer(&old);
    g_hash_table_remove(config->active, path);
    return ok;
}

typedef struct {
    LuaConfig* config;
    const char* path;
    GVariant* result;
    GError* error;
} EvalRun;

static int protected_eval(lua_State* state) {
    EvalRun* run = lua_touserdata(state, lua_upvalueindex(1));

    install_api(state, run->config);
    if (!evaluate_path(state, run->config, run->path, TRUE, &run->error))
        return 0;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    finish_named_entries(state, -1, "shortcuts");
    finish_named_entries(state, -1, "autostart");
    run->result = variant_from_lua(state, -1, 0, FALSE, 0, &run->error);
    lua_pop(state, 2);
    return 0;
}

GVariant* gnoblin_config_evaluate_file(const char* path, GPtrArray* paths, GPtrArray* directories,
                                       GError** error) {
    LuaConfig config = {.paths = paths ? paths : g_ptr_array_new_with_free_func(g_free),
                        .directories = directories,
                        .active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
                        .current_path = g_canonicalize_filename(path, NULL)};
    lua_State* state = new_config_state(&config);
    EvalRun run = {.config = &config, .path = path};
    int steps = MAX_CONFIG_STEPS / 1000;
    if (!state) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOMEM,
                            "Lua config exceeded memory limit");
        goto out;
    }
    memcpy(lua_getextraspace(state), &steps, sizeof steps);
    lua_sethook(state, limit_hook, LUA_MASKCOUNT, 1000);
    lua_pushlightuserdata(state, &run);
    lua_pushcclosure(state, protected_eval, 1);
    if (lua_pcall(state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(state));
        lua_pop(state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
out:
    if (state)
        lua_close(state);
    g_hash_table_unref(config.active);
    g_hash_table_unref(config.modules);
    g_free(config.current_path);
    if (!paths)
        g_ptr_array_free(config.paths, TRUE);
    return run.result;
}

static void lua_runtime_free(LuaRuntime* runtime) {
    if (!runtime)
        return;
    if (runtime->config.operations) {
        if (runtime->state) {
            GHashTableIter iter;
            gpointer value;
            g_hash_table_iter_init(&iter, runtime->config.operations);
            while (g_hash_table_iter_next(&iter, NULL, &value))
                luaL_unref(runtime->state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(value));
        }
        g_clear_pointer(&runtime->config.operations, g_hash_table_unref);
    }
    g_clear_pointer(&runtime->config.focus_operations, g_hash_table_unref);
    if (runtime->config.deferred_callbacks && runtime->state) {
        for (guint i = 0; i < runtime->config.deferred_callbacks->len; i++)
            luaL_unref(runtime->state, LUA_REGISTRYINDEX,
                       GPOINTER_TO_INT(g_ptr_array_index(runtime->config.deferred_callbacks, i)));
    }
    g_clear_pointer(&runtime->config.deferred_callbacks, g_ptr_array_unref);
    if (runtime->state)
        lua_close(runtime->state);
    g_clear_pointer(&runtime->config.active, g_hash_table_unref);
    g_clear_pointer(&runtime->config.modules, g_hash_table_unref);
    g_clear_pointer(&runtime->config.window_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.workspace_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.monitor_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.layer_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.capability_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.input_device_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.input_source_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.orientation_lock_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.shortcut_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.launch_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.portal_grant_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.animation_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.privacy_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.session_activity_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.session_lock_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.settings_document, g_variant_unref);
    g_clear_pointer(&runtime->config.focus_history_ids, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.paths, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.directories, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.runtime_actions, g_ptr_array_unref);
    g_clear_pointer(&runtime->document, g_variant_unref);
    g_clear_pointer(&runtime->pending_document, g_variant_unref);
    g_free(runtime->config.current_path);
    g_free(runtime);
}

void gnoblin_config_update_window_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.window_snapshot, g_variant_unref);
        runtime->config.window_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.window_snapshot = g_variant_ref(snapshot);
        focus_history_reconcile_snapshot(&runtime->config, runtime->config.window_snapshot);
    }
}

void gnoblin_config_update_workspace_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.workspace_snapshot, g_variant_unref);
        runtime->config.workspace_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.workspace_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_monitor_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.monitor_snapshot, g_variant_unref);
        runtime->config.monitor_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.monitor_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_layer_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.layer_snapshot, g_variant_unref);
        runtime->config.layer_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.layer_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_capability_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.capability_snapshot, g_variant_unref);
        runtime->config.capability_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.capability_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_input_device_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.input_device_snapshot, g_variant_unref);
        runtime->config.input_device_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.input_device_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_input_source_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.input_source_snapshot, g_variant_unref);
        runtime->config.input_source_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.input_source_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_orientation_lock_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.orientation_lock_snapshot, g_variant_unref);
        runtime->config.orientation_lock_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.orientation_lock_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_shortcut_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.shortcut_snapshot, g_variant_unref);
        runtime->config.shortcut_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.shortcut_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_launch_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.launch_snapshot, g_variant_unref);
        runtime->config.launch_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.launch_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_portal_grant_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.portal_grant_snapshot, g_variant_unref);
        runtime->config.portal_grant_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.portal_grant_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_session_activity_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    if (snapshot) {
        const struct {
            const char* key;
            const GVariantType* type;
        } fields[] = {{"available", G_VARIANT_TYPE_BOOLEAN},
                      {"idle", G_VARIANT_TYPE_BOOLEAN},
                      {"threshold_ms", G_VARIANT_TYPE_UINT64},
                      {"idle_for_ms", G_VARIANT_TYPE_UINT64}};
        for (guint i = 0; i < G_N_ELEMENTS(fields); i++) {
            g_autoptr(GVariant) value =
                g_variant_lookup_value(snapshot, fields[i].key, fields[i].type);
            if (!value)
                return;
        }
    }
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.session_activity_snapshot, g_variant_unref);
        runtime->config.session_activity_revision = snapshot ? revision : 0;
        runtime->config.session_activity_sampled_at_us = snapshot ? g_get_monotonic_time() : 0;
        if (snapshot)
            runtime->config.session_activity_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_session_lock_snapshot(GVariant* snapshot, guint64 revision) {
    (void)revision;
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    gboolean available = FALSE;
    if (snapshot && !g_variant_lookup(snapshot, "lock_available", "b", &available))
        return;
    if (available) {
        const char* state = NULL;
        if (!g_variant_lookup(snapshot, "lock_state", "&s", &state) ||
            !(g_str_equal(state, "unlocked") || g_str_equal(state, "covering") ||
              g_str_equal(state, "locked") || g_str_equal(state, "failsafe")))
            return;
    }
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.session_lock_snapshot, g_variant_unref);
        if (snapshot)
            runtime->config.session_lock_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_privacy_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    static const char* const fields[] = {"screen_sharing", "recording",       "microphone_in_use",
                                         "camera_in_use",  "location_in_use", NULL};
    static const char* const allowed[] = {
        "available",     "screen_sharing",  "recording", "microphone_in_use",
        "camera_in_use", "location_in_use", NULL};
    g_autoptr(GVariant) available =
        snapshot ? g_variant_lookup_value(snapshot, "available", G_VARIANT_TYPE_VARDICT) : NULL;
    if (snapshot && !available)
        return;
    for (guint i = 0; available && fields[i]; i++) {
        g_autoptr(GVariant) value =
            g_variant_lookup_value(available, fields[i], G_VARIANT_TYPE_BOOLEAN);
        if (!value)
            return;
        gboolean is_available = g_variant_get_boolean(value);
        g_autoptr(GVariant) activity = g_variant_lookup_value(snapshot, fields[i], NULL);
        if (is_available != (activity != NULL) ||
            (activity && !g_variant_is_of_type(activity, G_VARIANT_TYPE_BOOLEAN)))
            return;
    }
    if (available) {
        GVariantIter iter;
        const char* key;
        GVariant* value;
        guint count = 0;
        g_variant_iter_init(&iter, available);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_variant_unref(value);
            gboolean known = FALSE;
            for (guint i = 0; fields[i]; i++)
                known |= g_str_equal(key, fields[i]);
            if (!known)
                return;
            count++;
        }
        if (count != G_N_ELEMENTS(fields) - 1)
            return;
    }
    if (snapshot) {
        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, snapshot);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_variant_unref(value);
            gboolean known = FALSE;
            for (guint i = 0; allowed[i]; i++)
                known |= g_str_equal(key, allowed[i]);
            if (!known)
                return;
        }
    }

    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.privacy_snapshot, g_variant_unref);
        runtime->config.privacy_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.privacy_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_animation_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    g_autoptr(GVariant) records =
        snapshot ? g_variant_lookup_value(snapshot, "animations", G_VARIANT_TYPE("aa{sv}")) : NULL;
    if (snapshot && !records)
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.animation_snapshot, g_variant_unref);
        runtime->config.animation_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.animation_snapshot = g_variant_ref(snapshot);
    }
}

GVariant* gnoblin_config_load_runtime(const char* path, GPtrArray** paths, GPtrArray** directories,
                                      GError** error) {
    LuaRuntime* runtime = g_new0(LuaRuntime, 1);
    runtime->config.paths = g_ptr_array_new_with_free_func(g_free);
    runtime->config.directories = g_ptr_array_new_with_free_func(g_free);
    runtime->config.runtime_actions =
        g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
    runtime->config.deferred_callbacks = g_ptr_array_new();
    runtime->config.operations = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    runtime->config.focus_operations =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    runtime->config.focus_history_ids = g_ptr_array_new_with_free_func(g_free);
    runtime->config.active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    runtime->config.modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    runtime->config.current_path = g_canonicalize_filename(path, NULL);
    LuaRuntime* snapshot_source = active_runtime ? active_runtime : pending_runtime;
    if (snapshot_source && snapshot_source->config.window_snapshot) {
        runtime->config.window_snapshot = g_variant_ref(snapshot_source->config.window_snapshot);
        runtime->config.window_revision = snapshot_source->config.window_revision;
    }
    if (snapshot_source && snapshot_source->config.focus_history_ids) {
        for (guint i = 0; i < snapshot_source->config.focus_history_ids->len; i++)
            g_ptr_array_add(
                runtime->config.focus_history_ids,
                g_strdup(g_ptr_array_index(snapshot_source->config.focus_history_ids, i)));
    }
    focus_history_reconcile_snapshot(&runtime->config, runtime->config.window_snapshot);
    if (snapshot_source && snapshot_source->config.workspace_snapshot) {
        runtime->config.workspace_snapshot =
            g_variant_ref(snapshot_source->config.workspace_snapshot);
        runtime->config.workspace_revision = snapshot_source->config.workspace_revision;
    }
    if (snapshot_source && snapshot_source->config.monitor_snapshot) {
        runtime->config.monitor_snapshot = g_variant_ref(snapshot_source->config.monitor_snapshot);
        runtime->config.monitor_revision = snapshot_source->config.monitor_revision;
    }
    if (snapshot_source && snapshot_source->config.layer_snapshot) {
        runtime->config.layer_snapshot = g_variant_ref(snapshot_source->config.layer_snapshot);
        runtime->config.layer_revision = snapshot_source->config.layer_revision;
    }
    if (snapshot_source && snapshot_source->config.capability_snapshot) {
        runtime->config.capability_snapshot =
            g_variant_ref(snapshot_source->config.capability_snapshot);
        runtime->config.capability_revision = snapshot_source->config.capability_revision;
    }
    if (snapshot_source && snapshot_source->config.input_device_snapshot) {
        runtime->config.input_device_snapshot =
            g_variant_ref(snapshot_source->config.input_device_snapshot);
        runtime->config.input_device_revision = snapshot_source->config.input_device_revision;
    }
    if (snapshot_source && snapshot_source->config.input_source_snapshot) {
        runtime->config.input_source_snapshot =
            g_variant_ref(snapshot_source->config.input_source_snapshot);
        runtime->config.input_source_revision = snapshot_source->config.input_source_revision;
    }
    if (snapshot_source && snapshot_source->config.orientation_lock_snapshot) {
        runtime->config.orientation_lock_snapshot =
            g_variant_ref(snapshot_source->config.orientation_lock_snapshot);
        runtime->config.orientation_lock_revision =
            snapshot_source->config.orientation_lock_revision;
    }
    if (snapshot_source && snapshot_source->config.shortcut_snapshot) {
        runtime->config.shortcut_snapshot =
            g_variant_ref(snapshot_source->config.shortcut_snapshot);
        runtime->config.shortcut_revision = snapshot_source->config.shortcut_revision;
    }
    if (snapshot_source && snapshot_source->config.launch_snapshot) {
        runtime->config.launch_snapshot = g_variant_ref(snapshot_source->config.launch_snapshot);
        runtime->config.launch_revision = snapshot_source->config.launch_revision;
    }
    if (snapshot_source && snapshot_source->config.portal_grant_snapshot) {
        runtime->config.portal_grant_snapshot =
            g_variant_ref(snapshot_source->config.portal_grant_snapshot);
        runtime->config.portal_grant_revision = snapshot_source->config.portal_grant_revision;
    }
    if (snapshot_source && snapshot_source->config.animation_snapshot) {
        runtime->config.animation_snapshot =
            g_variant_ref(snapshot_source->config.animation_snapshot);
        runtime->config.animation_revision = snapshot_source->config.animation_revision;
    }
    if (snapshot_source && snapshot_source->config.privacy_snapshot) {
        runtime->config.privacy_snapshot = g_variant_ref(snapshot_source->config.privacy_snapshot);
        runtime->config.privacy_revision = snapshot_source->config.privacy_revision;
    }
    if (snapshot_source && snapshot_source->config.session_activity_snapshot) {
        runtime->config.session_activity_snapshot =
            g_variant_ref(snapshot_source->config.session_activity_snapshot);
        runtime->config.session_activity_revision =
            snapshot_source->config.session_activity_revision;
        runtime->config.session_activity_sampled_at_us =
            snapshot_source->config.session_activity_sampled_at_us;
    }
    if (snapshot_source && snapshot_source->config.session_lock_snapshot)
        runtime->config.session_lock_snapshot =
            g_variant_ref(snapshot_source->config.session_lock_snapshot);
    if (snapshot_source && snapshot_source->config.settings_document) {
        runtime->config.settings_document =
            g_variant_ref(snapshot_source->config.settings_document);
        runtime->config.settings_revision = snapshot_source->config.settings_revision;
    }
    runtime->state = new_config_state(&runtime->config);
    if (!runtime->state) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOMEM,
                            "Lua config exceeded memory limit");
        lua_runtime_free(runtime);
        return NULL;
    }
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(runtime->state), &steps, sizeof steps);
    lua_sethook(runtime->state, limit_hook, LUA_MASKCOUNT, 1000);
    EvalRun run = {.config = &runtime->config, .path = path};
    lua_pushlightuserdata(runtime->state, &run);
    lua_pushcclosure(runtime->state, protected_eval, 1);
    if (lua_pcall(runtime->state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(runtime->state));
        lua_pop(runtime->state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
    // The builder returns a floating variant. Own it before exposing another
    // reference to the supervisor; otherwise a later event can unref a stale value.
    runtime->document = run.result ? g_variant_ref_sink(g_steal_pointer(&run.result)) : NULL;
    if (!runtime->document || !gnoblin_config_validate_document(runtime->document, error)) {
        lua_runtime_free(runtime);
        return NULL;
    }
    lua_runtime_free(pending_runtime);
    pending_runtime = runtime;
    if (paths)
        *paths = g_ptr_array_ref(runtime->config.paths);
    if (directories)
        *directories = g_ptr_array_ref(runtime->config.directories);
    return g_variant_ref(runtime->document);
}

static void dispatch_rejected_operation_event(lua_State* state, GVariant* payload,
                                              const char* event_name) {
    int base = lua_gettop(state);
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    for (int pass = 0; pass < 2; pass++) {
        lua_getfield(state, -1, pass == 0 ? event_name : "*");
        if (!lua_istable(state, -1)) {
            lua_pop(state, 1);
            continue;
        }
        int registered = lua_absindex(state, -1);
        lua_newtable(state);
        int listeners = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, registered);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, registered, i);
            lua_rawseti(state, listeners, i);
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, listeners, i);
            push_variant(state, payload);
            lua_pushstring(state, event_name);
            lua_setfield(state, -2, "name");
            if (lua_pcall(state, 1, 0, 0) != LUA_OK) {
                g_warning("Lua event '%s' failed: %s", event_name, lua_error_text(state));
                lua_pop(state, 1);
            }
        }
        lua_pop(state, 2);
    }
    lua_settop(state, base);
}

static void fail_dropped_operation(LuaRuntime* runtime, GVariant* operation, const char* reason) {
    LuaConfig* config = &runtime->config;
    lua_State* state = runtime->state;
    gint64 request_id = 0;
    const char* method = NULL;
    if (!g_variant_lookup(operation, "request_id", "x", &request_id) || request_id <= 0 ||
        !g_variant_lookup(operation, "method", "&s", &method))
        return;

    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
    g_hash_table_remove(config->focus_operations, key);
    gpointer reference = g_hash_table_lookup(config->operations, key);
    if (!reference)
        return;

    int base = lua_gettop(state);
    lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    int handle = lua_absindex(state, -1);
    lua_pushliteral(state, "failed");
    lua_setfield(state, handle, "status");
    lua_newtable(state);
    lua_pushliteral(state, "invalid_argument");
    lua_setfield(state, -2, "code");
    lua_pushstring(state, reason);
    lua_setfield(state, -2, "message");
    lua_setfield(state, handle, "error");

    lua_getfield(state, handle, "_callbacks");
    if (lua_istable(state, -1)) {
        int registered = lua_absindex(state, -1);
        lua_newtable(state);
        int callbacks = lua_absindex(state, -1);
        lua_Integer count = lua_rawlen(state, registered);
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, registered, i);
            lua_rawseti(state, callbacks, i);
        }
        for (lua_Integer i = 1; i <= count; i++) {
            lua_rawgeti(state, callbacks, i);
            int subscription = lua_absindex(state, -1);
            lua_getfield(state, subscription, "_active");
            gboolean active = lua_toboolean(state, -1);
            lua_pop(state, 1);
            if (!active) {
                lua_pop(state, 1);
                continue;
            }
            lua_pushboolean(state, FALSE);
            lua_setfield(state, subscription, "_active");
            lua_getfield(state, subscription, "_callback");
            lua_getfield(state, handle, "value");
            lua_getfield(state, handle, "error");
            if (lua_pcall(state, 2, 0, 0) != LUA_OK) {
                g_warning("Lua operation completion callback failed: %s", lua_error_text(state));
                lua_pop(state, 1);
            }
            lua_pop(state, 1);
        }
        lua_newtable(state);
        lua_setfield(state, handle, "_callbacks");
    }
    lua_settop(state, base);
    luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
    g_hash_table_remove(config->operations, key);

    GVariantBuilder error_record;
    GVariantBuilder completion;
    g_variant_builder_init(&error_record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&error_record, "{sv}", "code", g_variant_new_string("invalid_argument"));
    g_variant_builder_add(&error_record, "{sv}", "message", g_variant_new_string(reason));
    g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&completion, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&completion, "{sv}", "error", g_variant_builder_end(&error_record));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&completion));
    dispatch_rejected_operation_event(state, payload, "gnoblin.operation.completed");
}

static void schedule_deferred_callbacks(LuaConfig* config) {
    if (!config || !config->deferred_callbacks || config->deferred_callbacks->len == 0 ||
        config->deferred_callbacks_scheduled || !runtime_wakeup_callback)
        return;
    config->deferred_callbacks_scheduled = TRUE;
    runtime_wakeup_callback(runtime_wakeup_data);
}

void gnoblin_config_set_runtime_wakeup_callback(GnoblinConfigRuntimeWakeupFunc callback,
                                                gpointer user_data) {
    runtime_wakeup_callback = callback;
    runtime_wakeup_data = user_data;
    if (active_runtime)
        active_runtime->config.deferred_callbacks_scheduled = FALSE;
    schedule_deferred_callbacks(active_runtime ? &active_runtime->config : NULL);
}

GVariant* gnoblin_config_dispatch_deferred_callbacks(GError** error) {
    if (pending_runtime) {
        if (active_runtime)
            active_runtime->config.deferred_callbacks_scheduled = FALSE;
        return NULL;
    }
    if (!active_runtime || !active_runtime->config.deferred_callbacks ||
        active_runtime->config.deferred_callbacks->len == 0)
        return NULL;
    LuaRuntime* runtime = active_runtime;
    LuaConfig* config = &runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua callbacks cannot be dispatched reentrantly");
        return NULL;
    }

    GPtrArray* callbacks = config->deferred_callbacks;
    config->deferred_callbacks = g_ptr_array_new();
    config->deferred_callbacks_scheduled = FALSE;
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->dispatching = TRUE;
    lua_State* state = runtime->state;
    int base = lua_gettop(state);
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(state), &steps, sizeof steps);

    guint dispatched = 0;
    for (guint i = 0; i < callbacks->len; i++) {
        int reference = GPOINTER_TO_INT(g_ptr_array_index(callbacks, i));
        lua_rawgeti(state, LUA_REGISTRYINDEX, reference);
        int subscription = lua_absindex(state, -1);
        lua_getfield(state, subscription, "_active");
        gboolean active = lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (active) {
            dispatched++;
            lua_pushboolean(state, FALSE);
            lua_setfield(state, subscription, "_active");
            lua_getfield(state, subscription, "_callback");
            lua_getfield(state, subscription, "_operation");
            int operation = lua_absindex(state, -1);
            lua_getfield(state, operation, "value");
            lua_getfield(state, operation, "error");
            if (lua_pcall(state, 2, 0, 0) != LUA_OK) {
                g_warning("Lua operation completion callback failed: %s", lua_error_text(state));
                lua_pop(state, 1);
            }
            lua_pop(state, 1);
        }
        lua_pop(state, 1);
        luaL_unref(state, LUA_REGISTRYINDEX, reference);
    }
    g_ptr_array_unref(callbacks);
    config->dispatching = FALSE;
    config->actions_in_dispatch = 0;
    lua_settop(state, base);
    if (dispatched == 0)
        return NULL;

    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    GVariant* document = variant_from_lua(state, -1, 0, FALSE, 0, error);
    lua_pop(state, 2);
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error)) {
        g_clear_pointer(&document, g_variant_unref);
        g_autoptr(GPtrArray) dropped =
            g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
        for (guint i = action_start; i < config->runtime_actions->len; i++)
            g_ptr_array_add(dropped, g_variant_ref(g_ptr_array_index(config->runtime_actions, i)));
        g_ptr_array_set_size(config->runtime_actions, action_start);
        config->dispatching = TRUE;
        config->rejecting_operations = TRUE;
        const char* reason =
            error && *error ? (*error)->message : "deferred callback configuration was rejected";
        for (guint i = 0; i < dropped->len; i++)
            fail_dropped_operation(runtime, g_ptr_array_index(dropped, i), reason);
        config->rejecting_operations = FALSE;
        config->dispatching = FALSE;
        config->actions_in_dispatch = 0;
        lua_getglobal(state, "gnoblin");
        push_variant(state, runtime->document);
        lua_setfield(state, -2, "config");
        lua_pop(state, 1);
        g_clear_pointer(&runtime->pending_document, g_variant_unref);
        return NULL;
    }

    if (g_variant_equal(document, runtime->document)) {
        g_variant_unref(document);
        return NULL;
    }
    g_clear_pointer(&runtime->pending_document, g_variant_unref);
    runtime->pending_document = g_variant_ref_sink(document);
    return g_variant_ref(runtime->pending_document);
}

static GVariant* input_gesture_event(LuaConfig* config, GVariant* payload) {
    static const struct {
        const char* source;
        const char* target;
        const GVariantType* type;
    } fields[] = {
        {"gesture", "gesture", G_VARIANT_TYPE_STRING},
        {"phase", "phase", G_VARIANT_TYPE_STRING},
        {"fingers", "fingers", G_VARIANT_TYPE_INT64},
        {"time", "input_time", G_VARIANT_TYPE_INT64},
        {"dx", "dx", G_VARIANT_TYPE_DOUBLE},
        {"dy", "dy", G_VARIANT_TYPE_DOUBLE},
        {"scale", "scale", G_VARIANT_TYPE_DOUBLE},
        {"angle_delta", "angle_delta", G_VARIANT_TYPE_DOUBLE},
    };
    GVariantBuilder event;
    g_variant_builder_init(&event, G_VARIANT_TYPE_VARDICT);
    for (gsize i = 0; i < G_N_ELEMENTS(fields); i++) {
        g_autoptr(GVariant) value =
            g_variant_lookup_value(payload, fields[i].source, fields[i].type);
        if (value)
            g_variant_builder_add(&event, "{sv}", fields[i].target, value);
    }
    g_variant_builder_add(&event, "{sv}", "sequence",
                          g_variant_new_int64(++config->input_gesture_sequence));
    g_variant_builder_add(&event, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    return g_variant_ref_sink(g_variant_builder_end(&event));
}

static GVariant* gnoblin_config_dispatch_event_internal(const char* event, GVariant* payload,
                                                        GError** error) {
    // The supervisor is applying a candidate configuration before committing it.
    // Compositor preference changes in that window can emit transition events;
    // do not send those to the old Lua runtime or let them mutate the candidate.
    if (pending_runtime)
        return NULL;
    if (!active_runtime)
        return NULL;
    if (!event || !*event || strlen(event) > 128 || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid Gnoblin Lua event or payload");
        return NULL;
    }
    if (active_runtime->config.dispatching || active_runtime->config.api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua runtime operations cannot be dispatched reentrantly");
        return NULL;
    }
    focus_history_update_event(&active_runtime->config, event, payload);
    guint action_start = active_runtime->config.runtime_actions->len;
    active_runtime->config.actions_in_dispatch = 0;
    active_runtime->config.dispatching = TRUE;
    lua_State* state = active_runtime->state;
    int steps = MAX_CONFIG_STEPS / 1000;
    memcpy(lua_getextraspace(state), &steps, sizeof steps);
    gboolean operation_completed = FALSE;
    gboolean callback_failed = FALSE;
    if (g_str_equal(event, "gnoblin.operation.completed")) {
        gint64 request_id = 0;
        gboolean ok = FALSE;
        if (g_variant_lookup(payload, "operation_id", "x", &request_id) && request_id > 0) {
            g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
            gpointer reference = g_hash_table_lookup(active_runtime->config.operations, key);
            if (reference) {
                operation_completed = TRUE;
                lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
                int handle = lua_absindex(state, -1);
                g_variant_lookup(payload, "ok", "b", &ok);
                lua_pushstring(state, ok ? "succeeded" : "failed");
                lua_setfield(state, handle, "status");
                GVariant* result = g_variant_lookup_value(payload, "value", NULL);
                GVariant* failure = g_variant_lookup_value(payload, "error", NULL);
                if (result) {
                    lua_getfield(state, handle, "method");
                    const char* method = lua_tostring(state, -1);
                    push_operation_result(state, method ? method : "", result);
                    lua_setfield(state, handle, "value");
                    lua_pop(state, 1);
                    g_variant_unref(result);
                }
                if (failure) {
                    push_variant(state, failure);
                    lua_setfield(state, handle, "error");
                    g_variant_unref(failure);
                }
                lua_getfield(state, handle, "_callbacks");
                if (lua_istable(state, -1)) {
                    int registered = lua_absindex(state, -1);
                    lua_newtable(state);
                    int callbacks = lua_absindex(state, -1);
                    lua_Integer count = lua_rawlen(state, registered);
                    for (lua_Integer i = 1; i <= count; i++) {
                        lua_rawgeti(state, registered, i);
                        lua_rawseti(state, callbacks, i);
                    }
                    for (lua_Integer i = 1; i <= count; i++) {
                        lua_rawgeti(state, callbacks, i);
                        int subscription = lua_absindex(state, -1);
                        lua_getfield(state, subscription, "_active");
                        gboolean active = lua_toboolean(state, -1);
                        lua_pop(state, 1);
                        if (!active) {
                            lua_pop(state, 1);
                            continue;
                        }
                        lua_pushboolean(state, FALSE);
                        lua_setfield(state, subscription, "_active");
                        lua_getfield(state, subscription, "_callback");
                        lua_getfield(state, handle, "value");
                        lua_getfield(state, handle, "error");
                        if (lua_pcall(state, 2, 0, 0) != LUA_OK) {
                            g_warning("Lua operation completion callback failed: %s",
                                      lua_error_text(state));
                            lua_pop(state, 1);
                        }
                        lua_pop(state, 1);
                    }
                    lua_newtable(state);
                    lua_setfield(state, handle, "_callbacks");
                    lua_pop(state, 1);
                }
                lua_pop(state, 2);
                luaL_unref(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
                g_hash_table_remove(active_runtime->config.operations, key);
            }
        }
    }
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    gboolean dispatched = operation_completed;
    g_autoptr(GVariant) stable_gesture = NULL;
    const char* event_names[2] = {event, NULL};
    GVariant* event_payloads[2] = {payload, NULL};
    guint event_count = 1;
    if (g_str_equal(event, "mutter.touchpad.gesture")) {
        stable_gesture = input_gesture_event(&active_runtime->config, payload);
        event_names[event_count] = "gnoblin.input.gesture";
        event_payloads[event_count] = stable_gesture;
        event_count++;
    }
    for (guint event_index = 0; event_index < event_count; event_index++) {
        for (int pass = 0; pass < 2; pass++) {
            lua_getfield(state, -1, pass == 0 ? event_names[event_index] : "*");
            if (!lua_istable(state, -1)) {
                lua_pop(state, 1);
                continue;
            }
            int registered = lua_absindex(state, -1);
            lua_newtable(state);
            int listeners = lua_absindex(state, -1);
            lua_Integer count = lua_rawlen(state, registered);
            for (lua_Integer i = 1; i <= count; i++) {
                lua_rawgeti(state, registered, i);
                lua_rawseti(state, listeners, i);
            }
            dispatched |= count > 0;
            for (lua_Integer i = 1; i <= count; i++) {
                lua_rawgeti(state, listeners, i);
                push_variant(state, event_payloads[event_index]);
                push_readonly_event_fields(state, event_payloads[event_index], -1);
                lua_pushstring(state, event_names[event_index]);
                lua_setfield(state, -2, "name");
                if (g_str_equal(event_names[event_index], "gnoblin.window.drag.started") ||
                    g_str_equal(event_names[event_index], "gnoblin.window.drag.updated")) {
                    push_window_drag(state, event_payloads[event_index]);
                    lua_setfield(state, -2, "drag");
                }
                gboolean trusted_activation =
                    g_str_equal(event_names[event_index], "gnoblin.shortcut.activated");
                if (g_str_equal(event_names[event_index], "gnoblin.shortcut.binding-activated")) {
                    lua_getfield(state, -1, "first");
                    trusted_activation = lua_toboolean(state, -1);
                    lua_pop(state, 1);
                }
                if (trusted_activation && active_runtime->config.dispatch_focus_handle) {
                    push_focus_context(state, active_runtime->config.dispatch_focus_handle,
                                       active_runtime->config.dispatch_focus_generation,
                                       active_runtime->config.dispatch_native_focus_generation,
                                       active_runtime->config.dispatch_focus_expires_at_us);
                    lua_setfield(state, -2, "focus_context");
                }
                if (g_str_equal(event_names[event_index], "gnoblin.window.menu-requested") &&
                    active_runtime->config.dispatch_menu_handle) {
                    push_menu_context(state, active_runtime->config.dispatch_menu_handle,
                                      active_runtime->config.dispatch_menu_generation,
                                      active_runtime->config.dispatch_native_menu_generation,
                                      active_runtime->config.dispatch_menu_expires_at_us,
                                      active_runtime->config.dispatch_menu_window_id);
                    lua_setfield(state, -2, "menu_context");
                }
                if (lua_pcall(state, 1, 0, 0) != LUA_OK) {
                    g_warning("Lua event '%s' failed: %s", event_names[event_index],
                              lua_error_text(state));
                    callback_failed = TRUE;
                    lua_pop(state, 1);
                }
            }
            lua_pop(state, 2);
        }
    }
    active_runtime->config.dispatching = FALSE;
    active_runtime->config.actions_in_dispatch = 0;
    lua_pop(state, 2);
    // Native touchpad actions are routed before this event reaches Lua.
    if (!dispatched)
        return NULL;
    if (callback_failed) {
        lua_getglobal(state, "gnoblin");
        push_variant(state, active_runtime->document);
        lua_setfield(state, -2, "config");
        lua_pop(state, 1);
        g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
        active_runtime->pending_document = g_variant_ref(active_runtime->document);
        return g_variant_ref(active_runtime->pending_document);
    }
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    GVariant* document = variant_from_lua(state, -1, 0, FALSE, 0, error);
    lua_pop(state, 2);
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error)) {
        g_clear_pointer(&document, g_variant_unref);
        g_autoptr(GPtrArray) dropped =
            g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
        for (guint i = action_start; i < active_runtime->config.runtime_actions->len; i++)
            g_ptr_array_add(dropped, g_variant_ref(g_ptr_array_index(
                                         active_runtime->config.runtime_actions, i)));
        g_ptr_array_set_size(active_runtime->config.runtime_actions, action_start);
        active_runtime->config.dispatching = TRUE;
        active_runtime->config.rejecting_operations = TRUE;
        const char* reason =
            error && *error ? (*error)->message : "event configuration was rejected";
        for (guint i = 0; i < dropped->len; i++)
            fail_dropped_operation(active_runtime, g_ptr_array_index(dropped, i), reason);
        active_runtime->config.rejecting_operations = FALSE;
        active_runtime->config.dispatching = FALSE;
        active_runtime->config.actions_in_dispatch = 0;
        lua_getglobal(state, "gnoblin");
        push_variant(state, active_runtime->document);
        lua_setfield(state, -2, "config");
        lua_pop(state, 1);
        g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
        return NULL;
    }
    g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
    active_runtime->pending_document = g_variant_ref_sink(document);
    return g_variant_ref(active_runtime->pending_document);
}

GVariant* gnoblin_config_dispatch_event(const char* event, GVariant* payload, GError** error) {
    if (!event || !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return gnoblin_config_dispatch_event_internal(event, payload, error);
    guint64 handle = 0, native_generation = 0;
    gint64 expires_at_us = 0;
    const char* window_id = NULL;
    const char* menu_type = NULL;
    gboolean has_handle = g_variant_lookup(payload, "_menu_context_handle", "t", &handle);
    gboolean has_generation =
        g_variant_lookup(payload, "_menu_context_generation", "t", &native_generation);
    gboolean has_expiry =
        g_variant_lookup(payload, "_menu_context_expires_at_us", "x", &expires_at_us);
    gboolean has_any = has_handle || has_generation || has_expiry;
    if (!has_any)
        return gnoblin_config_dispatch_event_internal(event, payload, error);
    gboolean valid = g_str_equal(event, "gnoblin.window.menu-requested") && has_handle && handle &&
                     has_generation && native_generation && has_expiry &&
                     expires_at_us > g_get_monotonic_time() &&
                     g_variant_lookup(payload, "window_id", "&s", &window_id) && window_id &&
                     *window_id && strlen(window_id) < 24 &&
                     g_variant_lookup(payload, "menu_type", "&s", &menu_type) &&
                     g_str_equal(menu_type, "wm");
    if (!valid) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid private WM menu capability event");
        return NULL;
    }
    if (!active_runtime || active_runtime->config.dispatch_menu_handle) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "WM menu capability cannot be nested or dispatched without a runtime");
        return NULL;
    }
    GVariantBuilder clean;
    GVariantIter fields;
    const char* name;
    GVariant* value;
    g_variant_builder_init(&clean, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&fields, payload);
    while (g_variant_iter_next(&fields, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) field = value;
        if (g_str_equal(name, "_menu_context_handle") ||
            g_str_equal(name, "_menu_context_generation") ||
            g_str_equal(name, "_menu_context_expires_at_us"))
            continue;
        g_variant_builder_add(&clean, "{sv}", name, field);
    }
    g_autoptr(GVariant) public_payload = g_variant_ref_sink(g_variant_builder_end(&clean));
    LuaConfig* config = &active_runtime->config;
    config->dispatch_menu_handle = handle;
    config->dispatch_menu_generation = active_runtime->generation;
    config->dispatch_native_menu_generation = native_generation;
    config->dispatch_menu_expires_at_us = expires_at_us;
    g_strlcpy(config->dispatch_menu_window_id, window_id, sizeof(config->dispatch_menu_window_id));
    GVariant* document = gnoblin_config_dispatch_event_internal(event, public_payload, error);
    config->dispatch_menu_handle = 0;
    config->dispatch_menu_generation = 0;
    config->dispatch_native_menu_generation = 0;
    config->dispatch_menu_expires_at_us = 0;
    config->dispatch_menu_window_id[0] = '\0';
    return document;
}

GVariant* gnoblin_config_dispatch_shortcut_event(const char* event, GVariant* payload,
                                                 guint64 context_handle, guint64 native_generation,
                                                 gint64 expires_at_us, GError** error) {
    if (!active_runtime || !context_handle || !native_generation ||
        expires_at_us <= g_get_monotonic_time() || !event ||
        (!g_str_equal(event, "gnoblin.shortcut.activated") &&
         !g_str_equal(event, "gnoblin.shortcut.binding-activated"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid trusted shortcut activation context");
        return NULL;
    }
    LuaConfig* config = &active_runtime->config;
    if (config->dispatch_focus_handle || config->dispatch_focus_generation) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "a shortcut activation context is already being dispatched");
        return NULL;
    }
    config->dispatch_focus_handle = context_handle;
    config->dispatch_focus_generation = active_runtime->generation;
    config->dispatch_native_focus_generation = native_generation;
    config->dispatch_focus_expires_at_us = expires_at_us;
    GVariant* document = gnoblin_config_dispatch_event(event, payload, error);
    config->dispatch_focus_handle = 0;
    config->dispatch_focus_generation = 0;
    config->dispatch_native_focus_generation = 0;
    config->dispatch_focus_expires_at_us = 0;
    return document;
}

guint64 gnoblin_config_runtime_generation(void) {
    return active_runtime ? active_runtime->generation : 0;
}

gboolean gnoblin_config_seed_runtime_counters(guint64 settings_revision, guint64 runtime_generation,
                                              guint64 operation_id_watermark, GError** error) {
    if (active_runtime || pending_runtime || deferred_runtime || revision_counters_seeded ||
        next_settings_revision || next_runtime_generation || next_runtime_operation_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "runtime counters can only be seeded before the first load");
        return FALSE;
    }
    if (!settings_revision || !runtime_generation || operation_id_watermark > G_MAXINT64) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "replacement runtime counters are invalid");
        return FALSE;
    }
    next_settings_revision = settings_revision;
    next_runtime_generation = runtime_generation;
    next_runtime_operation_id = operation_id_watermark;
    revision_counters_seeded = TRUE;
    return TRUE;
}

gboolean gnoblin_config_take_focus_context(gint64 request_id, guint64* context_handle,
                                           guint64* generation, guint64* native_generation,
                                           gint64* expires_at_us) {
    if (!active_runtime || request_id <= 0 || !context_handle || !generation ||
        !native_generation || !expires_at_us)
        return FALSE;
    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
    LuaFocusOperation* operation =
        g_hash_table_lookup(active_runtime->config.focus_operations, key);
    if (!operation)
        return FALSE;
    gboolean valid =
        operation->handle != 0 && operation->generation == active_runtime->generation &&
        operation->native_generation != 0 && operation->expires_at_us > g_get_monotonic_time();
    if (valid) {
        *context_handle = operation->handle;
        *generation = operation->generation;
        *native_generation = operation->native_generation;
        *expires_at_us = operation->expires_at_us;
    }
    g_hash_table_remove(active_runtime->config.focus_operations, key);
    return valid;
}

GVariant* gnoblin_config_drain_runtime_operations(void) {
    GVariantBuilder operations;
    g_variant_builder_init(&operations, G_VARIANT_TYPE("aa{sv}"));
    if (pending_runtime || !active_runtime || active_runtime->config.dispatching ||
        active_runtime->config.api_calling)
        return g_variant_ref_sink(g_variant_builder_end(&operations));
    GPtrArray* pending = active_runtime->config.runtime_actions;
    for (guint i = 0; i < pending->len; i++)
        g_variant_builder_add_value(&operations, g_ptr_array_index(pending, i));
    g_ptr_array_set_size(pending, 0);
    return g_variant_ref_sink(g_variant_builder_end(&operations));
}

guint gnoblin_config_runtime_pending_operations(void) {
    if (!active_runtime)
        return 0;

    return g_hash_table_size(active_runtime->config.operations) +
           (active_runtime->config.deferred_callbacks
                ? active_runtime->config.deferred_callbacks->len
                : 0);
}

GVariant* gnoblin_config_call_api(const char* method, GVariant* arguments, GError** error) {
    if (pending_runtime) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API is temporarily unavailable while config reloads");
        return NULL;
    }
    if (!active_runtime || !method || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(
            error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
            "Gnoblin API call requires an active Lua runtime, method, and argument table");
        return NULL;
    }
    gboolean known = FALSE;
    for (guint i = 0; api_methods[i]; i++)
        known |= g_str_equal(api_methods[i], method);
    for (guint i = 0; i < G_N_ELEMENTS(api_aliases); i++)
        known |= g_str_equal(api_aliases[i].public_name, method);
    if (!known) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "unknown Gnoblin API method '%s'",
                    method);
        return NULL;
    }
    LuaConfig* config = &active_runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API calls cannot be dispatched reentrantly");
        return NULL;
    }
    if (config->runtime_actions->len >= 256) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOSPC,
                            "too many pending Gnoblin session actions");
        return NULL;
    }
    const char* separator = strchr(method, '.');
    g_autofree char* domain = NULL;
    const char* operation = NULL;
    if (g_str_equal(method, "shortcut.capture")) {
        domain = g_strdup("shortcuts");
        operation = "capture";
    } else if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {
        domain = g_strdup("shortcuts");
        operation = g_str_equal(method, "shortcut.bind") ? "bind" : "unbind";
    } else if (g_str_equal(method, "shortcut.session.end")) {
        domain = g_strdup("shortcuts");
        operation = "end_session";
    } else {
        domain = g_strndup(method, separator - method);
        operation = separator + 1;
    }
    lua_State* state = active_runtime->state;
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->api_calling = TRUE;
    gboolean native_operation = g_str_has_prefix(method, "workspace.") ||
                                g_str_has_prefix(method, "window.") ||
                                g_str_equal(method, "input.select");
    int stack_base = lua_gettop(state);
    if (native_operation) {
        lua_pushlightuserdata(state, config);
        lua_pushstring(state, method);
        lua_pushcclosure(state,
                         g_str_has_prefix(method, "workspace.") ? lua_workspace_action
                                                                : lua_generic_api_action,
                         2);
    } else {
        lua_getglobal(state, "gnoblin");
        lua_getfield(state, -1, domain);
        lua_getfield(state, -1, operation);
    }
    if (!lua_isfunction(state, -1)) {
        lua_settop(state, stack_base);
        config->api_calling = FALSE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Lua API method '%s' is not registered", method);
        return NULL;
    }
    int argument_count = g_variant_n_children(arguments) ? 1 : 0;
    if (argument_count)
        push_variant(state, arguments);
    if (lua_pcall(state, argument_count, 1, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API method '%s' failed: %s",
                    method, lua_error_text(state));
        lua_settop(state, stack_base);
        config->api_calling = FALSE;
        g_ptr_array_set_size(config->runtime_actions, action_start);
        config->actions_in_dispatch = 0;
        return NULL;
    }
    gboolean valid_ticket = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0 &&
                            config->runtime_actions->len == action_start + 1;
    lua_settop(state, stack_base);
    config->api_calling = FALSE;
    config->actions_in_dispatch = 0;
    if (!valid_ticket) {
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                    "Lua API method '%s' did not produce exactly one operation", method);
        return NULL;
    }
    GVariant* operation_call =
        g_variant_ref(g_ptr_array_index(config->runtime_actions, action_start));
    return operation_call;
}

static GVariant* legacy_workspace_list_from_lua(GVariant* workspaces, GError** error) {
    if (!workspaces || !g_variant_is_of_type(workspaces, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua workspace snapshot is invalid");
        return NULL;
    }

    GVariantBuilder workspace_records;
    g_variant_builder_init(&workspace_records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(workspaces); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(workspaces, i);
        g_autoptr(GVariant) workspace = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(workspace, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua workspace record is invalid");
            return NULL;
        }

        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, workspace);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) field = value;
            if (g_str_equal(key, "revision"))
                continue;
            g_variant_builder_add(&record, "{sv}",
                                  g_str_equal(key, "window_count") ? "windows" : key, field);
        }
        g_variant_builder_add_value(&workspace_records, g_variant_builder_end(&record));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "workspaces", g_variant_builder_end(&workspace_records));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static gboolean legacy_window_list_arguments_valid(GVariant* arguments, GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) > 3) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window.list accepts optional app_id, title, and focused filters");
        return FALSE;
    }

    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (g_str_equal(key, "focused")) {
            if (!g_variant_is_of_type(field, G_VARIANT_TYPE_BOOLEAN))
                goto invalid;
        } else if ((g_str_equal(key, "app_id") || g_str_equal(key, "title")) &&
                   g_variant_is_of_type(field, G_VARIANT_TYPE_STRING)) {
            continue;
        } else {
            goto invalid;
        }
    }
    return TRUE;

invalid:
    g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "window.list accepts optional app_id, title, and focused filters");
    return FALSE;
}

static gboolean legacy_window_match_arguments_valid(GVariant* arguments, GError** error) {
    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(arguments) > 1) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "window.match accepts only an optional string window selector");
        return FALSE;
    }
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (!g_str_equal(key, "window") || !g_variant_is_of_type(field, G_VARIANT_TYPE_STRING)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "window.match accepts only an optional string window selector");
            return FALSE;
        }
    }
    return TRUE;
}

static GVariant* legacy_window_list_arguments_for_lua(GVariant* arguments) {
    gboolean focused = TRUE;
    if (!g_variant_lookup(arguments, "focused", "b", &focused) || focused)
        return g_variant_ref(arguments);

    GVariantBuilder filtered;
    g_variant_builder_init(&filtered, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        if (!g_str_equal(key, "focused"))
            g_variant_builder_add(&filtered, "{sv}", key, field);
    }
    return g_variant_ref_sink(g_variant_builder_end(&filtered));
}

static GVariant* legacy_window_list_from_lua(GVariant* windows, GError** error) {
    static const struct {
        const char* lua_name;
        const char* legacy_name;
    } fields[] = {{"id", "id"},
                  {"title", "title"},
                  {"app_id", "appId"},
                  {"gtk_app_id", "gtkAppId"},
                  {"wm_class", "wmClass"},
                  {"rule_app_id", "ruleAppId"},
                  {"focused", "focused"},
                  {"minimized", "minimized"},
                  {"last_user_time", "lastUserTime"},
                  {"parent", "parent"},
                  {"workspace_id", "workspaceId"},
                  {"workspace_number", "workspaceNumber"},
                  {"monitor_index", "monitorIndex"},
                  {"monitor_id", "monitorId"},
                  {"monitor", "monitor"},
                  {"above", "above"},
                  {"sticky", "sticky"},
                  {"demands_attention", "demandsAttention"},
                  {"closable", "closable"},
                  {"minimizable", "minimizable"},
                  {"maximizable", "maximizable"},
                  {"movable", "movable"},
                  {"resizable", "resizable"},
                  {"role", "role"},
                  {"type", "type"},
                  {"maximized", "maximized"},
                  {"fullscreen", "fullscreen"},
                  {"frame", "geometry"}};
    if (!windows || !g_variant_is_of_type(windows, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window snapshot is invalid");
        return NULL;
    }

    GVariantBuilder window_records;
    g_variant_builder_init(&window_records, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua window record is invalid");
            return NULL;
        }

        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        for (guint field_index = 0; field_index < G_N_ELEMENTS(fields); field_index++) {
            g_autoptr(GVariant) value =
                g_variant_lookup_value(window, fields[field_index].lua_name, NULL);
            if (value)
                g_variant_builder_add(&record, "{sv}", fields[field_index].legacy_name, value);
        }
        g_variant_builder_add_value(&window_records, g_variant_builder_end(&record));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "windows", g_variant_builder_end(&window_records));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* legacy_window_match_from_lua(GVariant* windows, GVariant* arguments,
                                              GError** error) {
    if (!windows || !g_variant_is_of_type(windows, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window snapshot is invalid");
        return NULL;
    }
    const char* selector = "active";
    g_variant_lookup(arguments, "window", "&s", &selector);
    GVariant* selected = NULL;
    for (gsize i = 0; i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(windows, i);
        g_autoptr(GVariant) window = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(window, G_VARIANT_TYPE_VARDICT)) {
            g_clear_pointer(&selected, g_variant_unref);
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua window record is invalid");
            return NULL;
        }
        const char* id = NULL;
        gboolean focused = FALSE;
        g_variant_lookup(window, "id", "&s", &id);
        g_variant_lookup(window, "focused", "b", &focused);
        if ((g_str_equal(selector, "active") && focused) ||
            (!g_str_equal(selector, "active") && id && g_str_equal(selector, id))) {
            selected = g_variant_ref(window);
            break;
        }
    }
    if (!selected) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOENT, "window not found");
        return NULL;
    }

    const char* id = NULL;
    const char* title = "";
    const char* desktop_app_id = "";
    const char* gtk_app_id = "";
    const char* wm_class = "";
    const char* rule_app_id = "";
    gboolean focused = FALSE;
    g_variant_lookup(selected, "id", "&s", &id);
    g_variant_lookup(selected, "title", "&s", &title);
    g_variant_lookup(selected, "app_id", "&s", &desktop_app_id);
    g_variant_lookup(selected, "gtk_app_id", "&s", &gtk_app_id);
    g_variant_lookup(selected, "wm_class", "&s", &wm_class);
    g_variant_lookup(selected, "rule_app_id", "&s", &rule_app_id);
    g_variant_lookup(selected, "focused", "b", &focused);
    if (!id || !*id) {
        g_variant_unref(selected);
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua window record has no stable ID");
        return NULL;
    }

    GVariantBuilder identity;
    g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&identity, "{sv}", "desktop_app_id",
                          g_variant_new_string(desktop_app_id));
    g_variant_builder_add(&identity, "{sv}", "gtk_app_id", g_variant_new_string(gtk_app_id));
    g_variant_builder_add(&identity, "{sv}", "wm_class", g_variant_new_string(wm_class));
    g_variant_builder_add(&identity, "{sv}", "rule_app_id", g_variant_new_string(rule_app_id));

    GVariantBuilder match;
    g_variant_builder_init(&match, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&match, "{sv}", "type", g_variant_new_string("window"));
    g_variant_builder_add(&match, "{sv}", "title", g_variant_new_string(title));
    g_variant_builder_add(&match, "{sv}", "focused", g_variant_new_boolean(focused));
    if (*rule_app_id)
        g_variant_builder_add(&match, "{sv}", "app_id", g_variant_new_string(rule_app_id));

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "identity", g_variant_builder_end(&identity));
    g_variant_builder_add(&result, "{sv}", "match", g_variant_builder_end(&match));
    g_variant_unref(selected);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* legacy_layer_list_from_lua(GVariant* layers, GError** error) {
    if (!layers || !g_variant_is_of_type(layers, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua layer snapshot is invalid");
        return NULL;
    }

    GVariantBuilder surfaces;
    g_variant_builder_init(&surfaces, G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; i < g_variant_n_children(layers); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(layers, i);
        g_autoptr(GVariant) layer = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(layer, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua layer record is invalid");
            return NULL;
        }

        g_autoptr(GVariant) id = g_variant_lookup_value(layer, "id", G_VARIANT_TYPE_STRING);
        g_autoptr(GVariant) namespace =
            g_variant_lookup_value(layer, "namespace", G_VARIANT_TYPE_STRING);
        if (!namespace)
            continue;
        if (!id) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "Lua layer record has no string ID");
            return NULL;
        }

        GVariantBuilder surface;
        g_variant_builder_init(&surface, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&surface, "{sv}", "id", id);
        g_variant_builder_add(&surface, "{sv}", "namespace", namespace);
        g_autoptr(GVariant) title = g_variant_lookup_value(layer, "title", G_VARIANT_TYPE_STRING);
        g_variant_builder_add(&surface, "{sv}", "title", title ? title : g_variant_new_string(""));
        g_variant_builder_add_value(&surfaces, g_variant_builder_end(&surface));
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "surfaces", g_variant_builder_end(&surfaces));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error) {
    static const char* read_methods[] = {
        "version",
        "windows.list",
        "window.list",
        "capabilities.list",
        "focus.history",
        "settings",
        "focus.policy",
        "session.activity",
        "session.status",
        "runtime.status",
        "launch.status",
        "window.match",
        "workspace.list",
        "layer.list",
        "monitor.list",
        "layer.animation_policy",
        "workspaces.list",
        "monitors.list",
        "layers.list",
        "launches.list",
        "launches.snapshot",
        "shortcuts.list",
        "shortcuts.actions",
        "permissions.list",
        "permissions.policy",
        "permissions.check",
        "portals.grants",
        "privacy.state",
        "input.devices",
        "input.sources",
        "input.current_source",
        "input.orientation_lock",
        NULL,
    };
    gboolean known = FALSE;
    for (guint i = 0; method && read_methods[i]; i++)
        known |= g_str_equal(read_methods[i], method);
    if (!known || !arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Gnoblin API read requires a supported method and argument table");
        return NULL;
    }
    if (pending_runtime || !active_runtime || !active_runtime->config.settings_document) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API is unavailable before configuration is active");
        return NULL;
    }

    gboolean accepts_arguments =
        g_str_equal(method, "focus.history") || g_str_equal(method, "windows.list") ||
        g_str_equal(method, "window.list") || g_str_equal(method, "layer.animation_policy") ||
        g_str_equal(method, "window.match") || g_str_equal(method, "layers.list") ||
        g_str_equal(method, "shortcuts.actions") || g_str_equal(method, "permissions.check") ||
        g_str_equal(method, "portals.grants");
    if (!accepts_arguments && g_variant_n_children(arguments) != 0) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Lua API read '%s' does not accept arguments", method);
        return NULL;
    }
    if (g_str_equal(method, "layer.animation_policy")) {
        const char* namespace = NULL;
        if (g_variant_n_children(arguments) != 1 ||
            !g_variant_lookup(arguments, "namespace", "&s", &namespace) || !namespace) {
            g_set_error_literal(
                error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                "Lua API read 'layer.animation_policy' requires a namespace string");
            return NULL;
        }
        gsize namespace_length = strlen(namespace);
        if (namespace_length == 0 || namespace_length > 128 ||
            !g_utf8_validate(namespace, namespace_length, NULL)) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "layer namespace must be 1 to 128 bytes of UTF-8");
            return NULL;
        }
    }
    const char* shortcut_action_group = NULL;
    if (g_str_equal(method, "shortcuts.actions") &&
        (g_variant_n_children(arguments) > 1 ||
         (g_variant_n_children(arguments) == 1 &&
          !g_variant_lookup(arguments, "group", "&s", &shortcut_action_group)))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "shortcuts.actions accepts an optional group string");
        return NULL;
    }
    if (g_str_equal(method, "permissions.check")) {
        const char* capability = NULL;
        const char* identity = NULL;
        if (g_variant_n_children(arguments) != 2 ||
            !g_variant_lookup(arguments, "capability", "&s", &capability) ||
            !g_variant_lookup(arguments, "identity", "&s", &identity) || !capability || !identity) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "permissions.check requires string capability and identity");
            return NULL;
        }
    }
    if (g_str_equal(method, "portals.grants")) {
        const char* kind = NULL;
        if (g_variant_n_children(arguments) > 1 ||
            (g_variant_n_children(arguments) == 1 &&
             !g_variant_lookup(arguments, "kind", "&s", &kind))) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "portals.grants accepts only an optional kind string");
            return NULL;
        }
    }
    if (g_str_equal(method, "window.list") && !legacy_window_list_arguments_valid(arguments, error))
        return NULL;
    if (g_str_equal(method, "window.match") &&
        !legacy_window_match_arguments_valid(arguments, error))
        return NULL;

    LuaConfig* config = &active_runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API reads cannot be dispatched reentrantly");
        return NULL;
    }

    lua_State* state = active_runtime->state;
    int base = lua_gettop(state);
    guint action_start = config->runtime_actions->len;
    config->api_calling = TRUE;

    lua_getglobal(state, "gnoblin");
    if (g_str_equal(method, "version")) {
        lua_getfield(state, -1, "version");
        lua_remove(state, -2);
    } else if (g_str_equal(method, "capabilities.list")) {
        lua_getfield(state, -1, "capabilities");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "focus.history")) {
        lua_getfield(state, -1, "focus");
        lua_getfield(state, -1, "history");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "windows.list") || g_str_equal(method, "window.list") ||
               g_str_equal(method, "window.match")) {
        lua_getfield(state, -1, "windows");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "settings")) {
        lua_getfield(state, -1, "settings");
        lua_remove(state, -2);
    } else if (g_str_equal(method, "session.activity") || g_str_equal(method, "session.status")) {
        lua_getfield(state, -1, "session");
        lua_getfield(state, -1, g_str_equal(method, "session.status") ? "status" : "activity");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "runtime.status")) {
        lua_getfield(state, -1, "runtime");
        lua_getfield(state, -1, "status");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "layer.animation_policy")) {
        lua_getfield(state, -1, "layers");
        lua_getfield(state, -1, "animation_policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "workspace.list") || g_str_equal(method, "workspaces.list")) {
        lua_getfield(state, -1, "workspaces");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "monitor.list") || g_str_equal(method, "monitors.list")) {
        lua_getfield(state, -1, "monitors");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "layers.list") || g_str_equal(method, "layer.list")) {
        lua_getfield(state, -1, "layers");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "launches.list")) {
        lua_getfield(state, -1, "launches");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "launches.snapshot") || g_str_equal(method, "launch.status")) {
        lua_getfield(state, -1, "launches");
        lua_getfield(state, -1, "snapshot");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "shortcuts.list")) {
        lua_getfield(state, -1, "shortcuts");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "shortcuts.actions")) {
        lua_getfield(state, -1, "shortcuts");
        lua_getfield(state, -1, "actions");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "permissions.list")) {
        lua_getfield(state, -1, "permissions");
        lua_getfield(state, -1, "list");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "permissions.policy")) {
        lua_getfield(state, -1, "permissions");
        lua_getfield(state, -1, "policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "permissions.check")) {
        lua_getfield(state, -1, "permissions");
        lua_getfield(state, -1, "check");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "portals.grants")) {
        lua_getfield(state, -1, "portals");
        lua_getfield(state, -1, "grants");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "privacy.state")) {
        lua_getfield(state, -1, "privacy");
        lua_getfield(state, -1, "state");
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else if (g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources") ||
               g_str_equal(method, "input.current_source") ||
               g_str_equal(method, "input.orientation_lock")) {
        lua_getfield(state, -1, "input");
        const char* field = g_str_equal(method, "input.devices")          ? "devices"
                            : g_str_equal(method, "input.sources")        ? "sources"
                            : g_str_equal(method, "input.current_source") ? "current_source"
                                                                          : "orientation_lock";
        lua_getfield(state, -1, field);
        lua_remove(state, -2);
        lua_remove(state, -2);
    } else { /* focus.policy */
        lua_getfield(state, -1, "focus");
        lua_getfield(state, -1, "policy");
        lua_remove(state, -2);
        lua_remove(state, -2);
    }

    gboolean is_function = !g_str_equal(method, "settings") && !g_str_equal(method, "focus.policy");
    int argument_count = 0;
    if (!lua_isfunction(state, -1) && is_function) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API read '%s' is not registered",
                    method);
        lua_settop(state, base);
        config->api_calling = FALSE;
        return NULL;
    }
    if (is_function) {
        if (g_str_equal(method, "layer.animation_policy")) {
            const char* namespace = NULL;
            g_variant_lookup(arguments, "namespace", "&s", &namespace);
            lua_pushstring(state, namespace);
            argument_count = 1;
        } else if (g_str_equal(method, "shortcuts.actions") && shortcut_action_group) {
            lua_pushstring(state, shortcut_action_group);
            argument_count = 1;
        } else if (g_variant_n_children(arguments) > 0 && !g_str_equal(method, "window.match")) {
            g_autoptr(GVariant) legacy_window_arguments =
                g_str_equal(method, "window.list") ? legacy_window_list_arguments_for_lua(arguments)
                                                   : NULL;
            push_variant(state, legacy_window_arguments ? legacy_window_arguments : arguments);
            argument_count = 1;
        }
        if (lua_pcall(state, argument_count, 1, 0) != LUA_OK) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API read '%s' failed: %s",
                        method, lua_error_text(state));
            lua_settop(state, base);
            config->api_calling = FALSE;
            g_ptr_array_set_size(config->runtime_actions, action_start);
            return NULL;
        }
    }

    GVariant* result = NULL;
    if (g_str_equal(method, "input.current_source") && lua_isnil(state, -1)) {
        GVariantBuilder response;
        g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&response, "{sv}", "revision",
                              g_variant_new_int64((gint64)config->input_source_revision));
        result = g_variant_ref_sink(g_variant_builder_end(&response));
    } else {
        gboolean force_array =
            g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources");
        g_autoptr(GVariant) value = variant_from_lua(state, -1, 0, force_array, 0, error);
        if (value && g_str_equal(method, "workspace.list")) {
            result = legacy_workspace_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "window.list")) {
            result = legacy_window_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "window.match")) {
            result = legacy_window_match_from_lua(value, arguments, error);
        } else if (value && g_str_equal(method, "layer.list")) {
            result = legacy_layer_list_from_lua(value, error);
        } else if (value && g_str_equal(method, "monitor.list")) {
            if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
                g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                    "Lua monitor snapshot is invalid");
            } else {
                GVariantBuilder monitors;
                gboolean valid_records = TRUE;
                g_variant_builder_init(&monitors, G_VARIANT_TYPE("aa{sv}"));
                for (gsize i = 0; i < g_variant_n_children(value); i++) {
                    g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
                    g_autoptr(GVariant) monitor = g_variant_get_variant(boxed);
                    if (!g_variant_is_of_type(monitor, G_VARIANT_TYPE_VARDICT)) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record is invalid");
                        valid_records = FALSE;
                        break;
                    }
                    GVariantBuilder record;
                    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
                    static const struct {
                        const char* source;
                        const GVariantType* type;
                    } fields[] = {
                        {"x", G_VARIANT_TYPE_INT32},         {"y", G_VARIANT_TYPE_INT32},
                        {"width", G_VARIANT_TYPE_INT32},     {"height", G_VARIANT_TYPE_INT32},
                        {"primary", G_VARIANT_TYPE_BOOLEAN}, {"scale", G_VARIANT_TYPE_DOUBLE},
                    };
                    g_autoptr(GVariant) index =
                        g_variant_lookup_value(monitor, "index", G_VARIANT_TYPE_INT32);
                    if (!index) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record has no integer index");
                        g_variant_builder_clear(&record);
                        valid_records = FALSE;
                        break;
                    }
                    g_variant_builder_add(&record, "{sv}", "id", index);
                    gboolean valid = TRUE;
                    for (guint field = 0; field < G_N_ELEMENTS(fields); field++) {
                        g_autoptr(GVariant) field_value = g_variant_lookup_value(
                            monitor, fields[field].source, fields[field].type);
                        if (!field_value) {
                            valid = FALSE;
                            break;
                        }
                        g_variant_builder_add(&record, "{sv}", fields[field].source, field_value);
                    }
                    if (!valid) {
                        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                            "Lua monitor record is missing a legacy field");
                        g_variant_builder_clear(&record);
                        valid_records = FALSE;
                        break;
                    }
                    g_variant_builder_add_value(&monitors, g_variant_builder_end(&record));
                }
                if (valid_records) {
                    GVariantBuilder response;
                    g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
                    g_variant_builder_add(&response, "{sv}", "monitors",
                                          g_variant_builder_end(&monitors));
                    result = g_variant_ref_sink(g_variant_builder_end(&response));
                } else {
                    g_variant_builder_clear(&monitors);
                }
            }
        } else if (value && (force_array || g_str_equal(method, "input.current_source"))) {
            GVariantBuilder response;
            g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
            if (g_str_equal(method, "input.devices") || g_str_equal(method, "input.sources")) {
                g_variant_builder_add(&response, "{sv}",
                                      g_str_equal(method, "input.devices") ? "devices" : "sources",
                                      g_variant_ref(value));
                guint64 revision = g_str_equal(method, "input.devices")
                                       ? config->input_device_revision
                                       : config->input_source_revision;
                g_variant_builder_add(&response, "{sv}", "revision",
                                      g_variant_new_int64((gint64)revision));
            } else {
                g_variant_builder_add(&response, "{sv}", "available", g_variant_new_boolean(TRUE));
                g_variant_builder_add(&response, "{sv}", "source", g_variant_ref(value));
                g_variant_builder_add(&response, "{sv}", "revision",
                                      g_variant_new_int64((gint64)config->input_source_revision));
            }
            result = g_variant_ref_sink(g_variant_builder_end(&response));
        } else if (value) {
            result = g_variant_ref(value);
        }
    }
    lua_settop(state, base);
    config->api_calling = FALSE;
    if (config->runtime_actions->len != action_start) {
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_clear_pointer(&result, g_variant_unref);
        if (error && !*error)
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                                "Gnoblin API read unexpectedly queued a mutation");
        return NULL;
    }
    return result ? g_variant_ref_sink(result) : NULL;
}

void gnoblin_config_finish_load(gboolean commit) {
    if (!pending_runtime)
        return;
    if (commit) {
        g_autoptr(GVariant) previous_focus_policy =
            active_runtime ? focus_policy_snapshot(active_runtime->document, 0) : NULL;
        g_autoptr(GVariant) previous_permission_policy =
            active_runtime ? gnoblin_permission_policy_snapshot(active_runtime->document, 0) : NULL;
        gboolean settings_changed =
            !active_runtime || !active_runtime->document ||
            !g_variant_equal(active_runtime->document, pending_runtime->document);
        guint64 settings_revision = active_runtime ? active_runtime->config.settings_revision : 0;
        lua_runtime_free(active_runtime);
        active_runtime = g_steal_pointer(&pending_runtime);
        if (revision_counters_seeded) {
            /* RESUME reconstructs the exact configuration Mutter suspended.
             * Keep its accepted identity for this first committed load. */
            active_runtime->generation = next_runtime_generation;
        } else if (next_runtime_generation < G_MAXUINT64) {
            active_runtime->generation = ++next_runtime_generation;
        } else {
            active_runtime->generation = next_runtime_generation;
        }
        g_clear_pointer(&active_runtime->config.settings_document, g_variant_unref);
        active_runtime->config.settings_document = g_variant_ref(active_runtime->document);
        if (revision_counters_seeded) {
            active_runtime->config.settings_revision = next_settings_revision;
            revision_counters_seeded = FALSE;
        } else {
            active_runtime->config.settings_revision =
                settings_changed ? ++next_settings_revision : settings_revision;
        }
        g_autoptr(GVariant) committed_focus_policy =
            focus_policy_snapshot(active_runtime->document, 0);
        if (previous_focus_policy &&
            !g_variant_equal(previous_focus_policy, committed_focus_policy) &&
            focus_policy_changed_callback) {
            g_autoptr(GVariant) policy = focus_policy_snapshot(
                active_runtime->document, active_runtime->config.settings_revision);
            focus_policy_changed_callback(policy, active_runtime->config.settings_revision,
                                          focus_policy_changed_data);
        }
        g_autoptr(GVariant) committed_permission_policy =
            gnoblin_permission_policy_snapshot(active_runtime->document, 0);
        if (previous_permission_policy && committed_permission_policy &&
            !g_variant_equal(previous_permission_policy, committed_permission_policy) &&
            permission_policy_changed_callback) {
            g_autoptr(GVariant) policy = gnoblin_permission_policy_snapshot(
                active_runtime->document, active_runtime->config.settings_revision);
            permission_policy_changed_callback(policy, active_runtime->config.settings_revision,
                                               permission_policy_changed_data);
        }
        if (settings_changed && settings_changed_callback)
            settings_changed_callback(active_runtime->config.settings_revision,
                                      settings_changed_data);
    } else {
        lua_runtime_free(g_steal_pointer(&pending_runtime));
        if (active_runtime && active_runtime->config.deferred_callbacks &&
            active_runtime->config.deferred_callbacks->len > 0) {
            active_runtime->config.deferred_callbacks_scheduled = FALSE;
            schedule_deferred_callbacks(&active_runtime->config);
        }
    }
}

gboolean gnoblin_config_defer_load_commit(void) {
    if (!pending_runtime || deferred_runtime)
        return FALSE;
    deferred_runtime = g_steal_pointer(&pending_runtime);
    return TRUE;
}

void gnoblin_config_finish_deferred_load(gboolean commit) {
    if (!deferred_runtime)
        return;
    g_assert(!pending_runtime);
    pending_runtime = g_steal_pointer(&deferred_runtime);
    gnoblin_config_finish_load(commit);
}

guint64 gnoblin_config_deferred_runtime_generation(void) {
    if (!deferred_runtime)
        return gnoblin_config_runtime_generation();
    return next_runtime_generation < G_MAXUINT64 ? next_runtime_generation + 1
                                                 : next_runtime_generation;
}

static GVariant* focus_policy_snapshot(GVariant* document, guint64 revision) {
    static const struct {
        const char* key;
        const char* default_value;
    } string_fields[] = {
        {"focus-mode", "click"},
        {"focus-new-windows", "strict"},
    };
    static const struct {
        const char* key;
        gboolean default_value;
    } boolean_fields[] = {
        {"raise-on-click", TRUE},
        {"auto-raise", FALSE},
        {"focus-change-on-pointer-rest", FALSE},
    };
    GVariantBuilder policy;
    g_autoptr(GVariant) window_management =
        document ? g_variant_lookup_value(document, "window-management", G_VARIANT_TYPE_VARDICT)
                 : NULL;
    g_variant_builder_init(&policy, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < G_N_ELEMENTS(string_fields); i++) {
        const char* value = string_fields[i].default_value;
        if (window_management)
            g_variant_lookup(window_management, string_fields[i].key, "&s", &value);
        g_variant_builder_add(&policy, "{sv}", i == 0 ? "focus_mode" : "focus_new_windows",
                              g_variant_new_string(value));
    }
    for (guint i = 0; i < G_N_ELEMENTS(boolean_fields); i++) {
        gboolean value = boolean_fields[i].default_value;
        if (window_management)
            g_variant_lookup(window_management, boolean_fields[i].key, "b", &value);
        g_variant_builder_add(&policy, "{sv}",
                              i == 0   ? "raise_on_click"
                              : i == 1 ? "auto_raise"
                                       : "focus_change_on_pointer_rest",
                              g_variant_new_boolean(value));
    }
    gint64 auto_raise_delay = 500;
    if (window_management) {
        g_autoptr(GVariant) value =
            g_variant_lookup_value(window_management, "auto-raise-delay", NULL);
        if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            auto_raise_delay = g_variant_get_int64(value);
        else if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
            auto_raise_delay = g_variant_get_int32(value);
    }
    g_variant_builder_add(&policy, "{sv}", "auto_raise_delay",
                          g_variant_new_int64(auto_raise_delay));
    g_variant_builder_add(&policy, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    return g_variant_ref_sink(g_variant_builder_end(&policy));
}

void gnoblin_config_set_focus_policy_changed_callback(GnoblinConfigFocusPolicyChangedFunc callback,
                                                      gpointer user_data) {
    focus_policy_changed_callback = callback;
    focus_policy_changed_data = user_data;
}

void gnoblin_config_set_permission_policy_changed_callback(
    GnoblinConfigPermissionPolicyChangedFunc callback, gpointer user_data) {
    permission_policy_changed_callback = callback;
    permission_policy_changed_data = user_data;
}

void gnoblin_config_set_settings_changed_callback(GnoblinConfigSettingsChangedFunc callback,
                                                  gpointer user_data) {
    settings_changed_callback = callback;
    settings_changed_data = user_data;
}

GVariant* gnoblin_config_current_document(void) {
    return active_runtime && active_runtime->document ? g_variant_ref(active_runtime->document)
                                                      : NULL;
}

guint64 gnoblin_config_settings_revision(void) {
    return active_runtime ? active_runtime->config.settings_revision : 0;
}

void gnoblin_config_finish_event(gboolean commit) {
    if (!active_runtime || !active_runtime->pending_document)
        return;
    if (commit) {
        g_autoptr(GVariant) previous_focus_policy =
            focus_policy_snapshot(active_runtime->document, 0);
        g_autoptr(GVariant) previous_permission_policy =
            gnoblin_permission_policy_snapshot(active_runtime->document, 0);
        gboolean settings_changed =
            !active_runtime->document ||
            !g_variant_equal(active_runtime->document, active_runtime->pending_document);
        g_clear_pointer(&active_runtime->document, g_variant_unref);
        active_runtime->document = g_steal_pointer(&active_runtime->pending_document);
        if (settings_changed) {
            g_clear_pointer(&active_runtime->config.settings_document, g_variant_unref);
            active_runtime->config.settings_document = g_variant_ref(active_runtime->document);
            active_runtime->config.settings_revision = ++next_settings_revision;
        }
        g_autoptr(GVariant) committed_focus_policy =
            focus_policy_snapshot(active_runtime->document, 0);
        if (!g_variant_equal(previous_focus_policy, committed_focus_policy) &&
            focus_policy_changed_callback) {
            g_autoptr(GVariant) policy = focus_policy_snapshot(
                active_runtime->document, active_runtime->config.settings_revision);
            focus_policy_changed_callback(policy, active_runtime->config.settings_revision,
                                          focus_policy_changed_data);
        }
        g_autoptr(GVariant) committed_permission_policy =
            gnoblin_permission_policy_snapshot(active_runtime->document, 0);
        if (previous_permission_policy && committed_permission_policy &&
            !g_variant_equal(previous_permission_policy, committed_permission_policy) &&
            permission_policy_changed_callback) {
            g_autoptr(GVariant) policy = gnoblin_permission_policy_snapshot(
                active_runtime->document, active_runtime->config.settings_revision);
            permission_policy_changed_callback(policy, active_runtime->config.settings_revision,
                                               permission_policy_changed_data);
        }
        if (settings_changed && settings_changed_callback)
            settings_changed_callback(active_runtime->config.settings_revision,
                                      settings_changed_data);
        return;
    }
    lua_State* state = active_runtime->state;
    lua_getglobal(state, "gnoblin");
    push_variant(state, active_runtime->document);
    lua_setfield(state, -2, "config");
    lua_pop(state, 1);
    g_clear_pointer(&active_runtime->pending_document, g_variant_unref);
}

char** gnoblin_config_runtime_events(void) {
    LuaRuntime* runtime = pending_runtime ? pending_runtime : active_runtime;
    if (!runtime)
        return g_new0(char*, 1);
    GPtrArray* events = g_ptr_array_new_with_free_func(g_free);
    lua_State* state = runtime->state;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "listeners");
    lua_pushnil(state);
    while (lua_next(state, -2)) {
        if (lua_type(state, -2) == LUA_TSTRING && lua_istable(state, -1))
            g_ptr_array_add(events, g_strdup(lua_tostring(state, -2)));
        lua_pop(state, 1);
    }
    lua_pop(state, 2);
    g_ptr_array_add(events, NULL);
    return (char**)g_ptr_array_free(events, FALSE);
}
