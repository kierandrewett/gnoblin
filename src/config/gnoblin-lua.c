/* Restricted, single-state Lua configuration evaluator. */
#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"

#include <gio/gio.h>
#include <lauxlib.h>
#include <lua.h>
#include <lualib.h>
#include <math.h>
#include <string.h>

#if __has_include("../core/gnoblin-native-control.h")
#include "../core/gnoblin-native-control.h"
#endif

#define MAX_CONFIG_BYTES (8 * 1024 * 1024)
#define MAX_CONFIG_STEPS 1000000
#define MAX_CONFIG_DEPTH 64
#define MAX_CONFIG_FILES 32
#define LUA_FOCUS_CONTEXT_METATABLE "gnoblin.FocusContext"

typedef struct {
    guint64 handle;
    guint64 generation;
    gint64 expires_at_us;
    gboolean consumed;
} LuaFocusContext;

typedef struct {
    guint64 handle;
    guint64 generation;
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
    GVariant* shortcut_snapshot;
    guint64 shortcut_revision;
    GVariant* launch_snapshot;
    guint64 launch_revision;
    GVariant* portal_grant_snapshot;
    guint64 portal_grant_revision;
    guint64 input_gesture_sequence;
    guint64 dispatch_focus_handle;
    guint64 dispatch_focus_generation;
    gint64 dispatch_focus_expires_at_us;
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
static guint64 next_runtime_operation_id;
static guint64 next_settings_revision;
static guint64 next_runtime_generation;
static GnoblinConfigRuntimeWakeupFunc runtime_wakeup_callback;
static gpointer runtime_wakeup_data;
static GnoblinConfigFocusPolicyChangedFunc focus_policy_changed_callback;
static gpointer focus_policy_changed_data;
static GnoblinConfigPermissionPolicyChangedFunc permission_policy_changed_callback;
static gpointer permission_policy_changed_data;

static int lua_workspace_action(lua_State* state);
static int lua_generic_api_action(lua_State* state);
static int lua_focus_history(lua_State* state);
static int lua_permissions_list(lua_State* state);
static int lua_permissions_policy(lua_State* state);
static int lua_permissions_check(lua_State* state);
static int lua_launches_begin(lua_State* state);
static int lua_launches_end(lua_State* state);
static int lua_portal_grants(lua_State* state);
static guint64 allocate_operation_id(void);
static GVariant* focus_policy_snapshot(GVariant* document, guint64 revision);
static void push_operation_handle(lua_State* state, LuaConfig* config, guint64 id,
                                  const char* method);

static int lua_focus_context_tostring(lua_State* state) {
    lua_pushliteral(state, "FocusContext");
    return 1;
}

static void push_focus_context(lua_State* state, guint64 handle, guint64 generation,
                               gint64 expires_at_us) {
    LuaFocusContext* context = lua_newuserdatauv(state, sizeof(*context), 0);
    *context = (LuaFocusContext){.handle = handle,
                                 .generation = generation,
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
    "workspace.list",
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
    "window.restore",
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
    "window.list",
    "window.match",
    "window.action",
    "layer.list",
    "monitor.list",
    "animation.list",
    "animation.surfaces",
    "animation.inspect",
    "animation.preview",
    "animation.seek",
    "animation.step",
    "animation.play",
    "animation.pause",
    "animation.stop",
    "feature.list",
    "feature.show",
    "feature.enable",
    "feature.disable",
    "script.list",
    "input.list",
    "input.current",
    "input.select",
    "input.sources",
    "input.current_source",
    "privacy.get",
    "permissions.list",
    "permissions.policy",
    "permissions.check",
    "grant.list",
    "grant.revoke",
    "launch.status",
    "launch.begin",
    "launch.end",
    "shell.ping",
    "shell.version",
    "shell.status",
    "shell.reload",
    "runtime.reload_config",
    "shortcut.capture",
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
};

static gboolean append_array_key(const char* key) {
    return key && (!strcmp(key, "autostart") || !strcmp(key, "window-rules") ||
                   !strcmp(key, "shortcuts") || !strcmp(key, "animations") ||
                   !strcmp(key, "rules") || !strcmp(key, "workspaces") ||
                   !strcmp(key, "workspace-names") || !strcmp(key, "workspace-ids") ||
                   !strcmp(key, "xkb-options") || !strcmp(key, "sources"));
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

static GVariant* variant_from_lua(lua_State* state, int index, int depth, gboolean force_array,
                                  int keybinding_depth, GError** error) {
    int type = lua_type(state, index);
    if (depth > MAX_CONFIG_DEPTH) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Lua config nesting exceeds 64 levels");
        return NULL;
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
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        lua_pushnumber(state, g_variant_get_double(value));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        lua_pushstring(state, g_variant_get_string(value, NULL));
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING_ARRAY)) {
        lua_newtable(state);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            lua_pushstring(state, g_variant_get_string(child, NULL));
            lua_rawseti(state, -2, i + 1);
        }
    } else if (g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        lua_newtable(state);
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
        g_hash_table_replace(config->focus_operations, g_strdup(key), operation);
        context->consumed = TRUE;
        config->actions_in_dispatch++;
        push_operation_handle(state, config, request_id, method);
        return 1;
    }
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.minimize") ||
        g_str_equal(method, "window.toggle_minimize") || g_str_equal(method, "window.restore") ||
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

static void add_snapshot_methods(lua_State* state, int backing, int method_table) {
    lua_getfield(state, backing, "id");
    gboolean has_id = lua_type(state, -1) == LUA_TSTRING;
    lua_pop(state, 1);
    if (!has_id)
        return;
    lua_getfield(state, backing, "frame");
    gboolean is_window = lua_istable(state, -1);
    lua_pop(state, 1);
    lua_getfield(state, backing, "number");
    gboolean is_workspace = lua_isinteger(state, -1);
    lua_pop(state, 1);
    if (is_window) {
        const char* methods[][2] = {{"close", "window.close"},
                                    {"minimize", "window.minimize"},
                                    {"toggle_minimize", "window.toggle_minimize"},
                                    {"restore", "window.restore"},
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
                                    {NULL, NULL}};
        for (guint i = 0; methods[i][0]; i++)
            add_record_method(state, method_table, methods[i][0], methods[i][1]);
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
    lua_newtable(state); /* Methods stay out of pairs()/snapshot fields. */
    int method_table = lua_absindex(state, -1);
    add_snapshot_methods(state, backing, method_table);
    lua_pushvalue(state, method_table);
    lua_setiuservalue(state, readonly, 2);
    lua_pushvalue(state, backing);
    lua_setiuservalue(state, readonly, 1);
    lua_pop(state, 2); /* method table, backing */

    lua_newtable(state); /* metatable */
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
    lua_setmetatable(state, readonly);
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
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart"));
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
            {"focus_new_windows", "smart", FALSE, 0, FOCUS_STRING},
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
static int lua_legacy_set(lua_State* state) {
    g_warning("gnoblin.set is deprecated and may be removed at any time; migrate to "
              "gnoblin.configure with public snake_case setting names");
    return lua_set(state);
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
    /* Keep the former built-in event as an input alias while listeners are
     * stored under the canonical event name. */
    if (event_length == strlen("pointer_window_changed") &&
        !memcmp(event, "pointer_window_changed", event_length))
        event = "mutter.wayland.pointer-window-changed";
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
    if (g_str_equal(method, "workspace.list") || g_str_equal(method, "workspace.next") ||
        g_str_equal(method, "workspace.previous")) {
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

    GVariant* arguments = NULL;
    if (lua_gettop(state) == 0) {
        if (g_str_has_prefix(method, "window.") && !g_str_equal(method, "window.list") &&
            !g_str_equal(method, "window.match") && !g_str_equal(method, "window.action"))
            return luaL_error(state, "%s requires an argument table", method);
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        arguments = g_variant_builder_end(&empty);
    } else if (lua_gettop(state) == 1 && lua_istable(state, 1)) {
        if (g_str_has_prefix(method, "window.") && !g_str_equal(method, "window.list") &&
            !g_str_equal(method, "window.match") && !g_str_equal(method, "window.action"))
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
 * snake_case until the shell maps them to their native GSettings names. */
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
                   !strcmp(parent, "shortcuts") || !strcmp(parent, "autostart"));
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
    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && (!strcmp(api, "gnoblin.shortcut") || !strcmp(api, "gnoblin.autostart"))) {
        const char* migration = !strcmp(api, "gnoblin.shortcut")
                                    ? "migrate to gnoblin.configure {shortcuts = {[\"<name>\"] = "
                                      "{binding = ..., command = ...}}}"
                                    : "migrate to gnoblin.configure {autostart = {[\"<name>\"] = "
                                      "{command = ...}}}";
        g_warning("%s is deprecated and may be removed at any time; %s", api, migration);
    }
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

static int lua_remove_declaration(lua_State* state) {
    const char* name = luaL_checkstring(state, 1);
    const char* api = lua_tostring(state, lua_upvalueindex(4));
    if (api && *api) {
        if (!strcmp(api, "gnoblin.remove_shortcut"))
            g_warning("%s is deprecated and may be removed at any time; remove key \"%s\" from "
                      "gnoblin.configure.shortcuts or set "
                      "gnoblin.configure.shortcuts[\"%s\"].enable = false",
                      api, name, name);
        else if (!strcmp(api, "gnoblin.remove_autostart"))
            g_warning("%s is deprecated and may be removed at any time; remove key \"%s\" from "
                      "gnoblin.configure.autostart or set "
                      "gnoblin.configure.autostart[\"%s\"].enable = false",
                      api, name, name);
    }
    push_config_list(state);
    remove_named_entry(state, -1, name);
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
    g_variant_builder_add(&result, "{sv}", "devices", g_variant_new_uint32(decision.devices));
    g_variant_builder_add(&result, "{sv}", "clipboard", g_variant_new_boolean(decision.clipboard));
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

static int lua_shortcuts_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.shortcuts.list takes no arguments");
    if (!config || !config->shortcut_snapshot)
        return luaL_error(state, "native shortcut snapshot is unavailable");

    g_autoptr(GVariant) shortcuts =
        g_variant_lookup_value(config->shortcut_snapshot, "shortcuts", G_VARIANT_TYPE("av"));
    lua_newtable(state);
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

static int lua_launches_list(lua_State* state) {
    LuaConfig* config = lua_touserdata(state, lua_upvalueindex(1));
    if (lua_gettop(state) != 0)
        return luaL_error(state, "gnoblin.launches.list takes no arguments");
    if (!config || !config->launch_snapshot)
        return luaL_error(state, "native launch snapshot is unavailable");

    g_autoptr(GVariant) launches =
        g_variant_lookup_value(config->launch_snapshot, "launches", G_VARIANT_TYPE("av"));
    lua_newtable(state);
    for (gsize i = 0; launches && i < g_variant_n_children(launches); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(launches, i);
        g_autoptr(GVariant) launch = g_variant_get_variant(wrapped);
        push_variant(state, launch);
        push_readonly_copy(state, -1);
        lua_remove(state, -2);
        lua_rawseti(state, -2, i + 1);
    }
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

static int lua_launches_end(lua_State* state) {
    if (lua_gettop(state) != 1 || lua_type(state, 1) != LUA_TSTRING || !*lua_tostring(state, 1) ||
        g_utf8_strlen(lua_tostring(state, 1), -1) > 128)
        return luaL_error(
            state, "gnoblin.launches.end requires a non-empty token of at most 128 characters");
    lua_newtable(state);
    lua_pushvalue(state, 1);
    lua_setfield(state, -2, "token");
    int arguments = lua_absindex(state, -1);
    return call_launch_operation(state, "launch.end", arguments);
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
        g_strv_sort(keys);
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

static int lua_version(lua_State* state) {
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
    lua_pushcfunction(state, lua_version);
    lua_setfield(state, -2, "version");
    lua_newtable(state);
    lua_setfield(state, -2, "config");
    for (guint i = 0; api_methods[i]; i++) {
        if (g_str_equal(api_methods[i], "shortcut.capture"))
            continue; /* Public collection API is gnoblin.shortcuts.capture. */
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
        if (g_str_equal(api_methods[i], "permissions.list") ||
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
    lua_setfield(state, -2, "shortcuts");
    lua_newtable(state);
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_list, 1);
    lua_setfield(state, -2, "list");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_begin, 1);
    lua_setfield(state, -2, "begin");
    lua_pushlightuserdata(state, config);
    lua_pushcclosure(state, lua_launches_end, 1);
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
    lua_pushcfunction(state, lua_legacy_set);
    lua_setfield(state, -2, "set");
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
        gboolean named, remove;
    } declarations[] = {
        {"window_rule", "", "window-rules", FALSE, FALSE},
        {"permission_rule", "permissions", "rules", FALSE, FALSE},
        {"shortcut", "", "shortcuts", TRUE, FALSE},
        {"animation", "", "animations", TRUE, FALSE},
        {"autostart", "", "autostart", TRUE, FALSE},
        {"remove_shortcut", "", "shortcuts", TRUE, TRUE},
        {"remove_autostart", "", "autostart", TRUE, TRUE},
    };
    for (guint i = 0; i < G_N_ELEMENTS(declarations); i++) {
        lua_pushstring(state, declarations[i].section);
        lua_pushstring(state, declarations[i].key);
        lua_pushboolean(state, declarations[i].named);
        lua_pushstring(
            state, !strcmp(declarations[i].name, "shortcut")           ? "gnoblin.shortcut"
                   : !strcmp(declarations[i].name, "autostart")        ? "gnoblin.autostart"
                   : !strcmp(declarations[i].name, "animation")        ? "gnoblin.animation"
                   : !strcmp(declarations[i].name, "remove_shortcut")  ? "gnoblin.remove_shortcut"
                   : !strcmp(declarations[i].name, "remove_autostart") ? "gnoblin.remove_autostart"
                                                                       : "");
        lua_pushcclosure(state, declarations[i].remove ? lua_remove_declaration : lua_declare, 4);
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

static gboolean merge_variant(lua_State* state, GVariant* document, GError** error) {
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "loaded config must be a table");
        return FALSE;
    }
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    push_variant(state, document);
    merge_table(state, -2, -1, NULL, TRUE);
    lua_pop(state, 3);
    return TRUE;
}

static gboolean evaluate_toml(lua_State* state, LuaConfig* config, const char* path,
                              GError** error) {
    g_autofree char* contents = NULL;
    if (!g_file_get_contents(path, &contents, NULL, error))
        return FALSE;
    g_autoptr(GVariant) document = gnoblin_config_parse_toml(contents, error);
    if (!document)
        return FALSE;
    return merge_variant(state, document, error);
}

static gboolean evaluate_path(lua_State* state, LuaConfig* config, const char* given, gboolean root,
                              GError** error) {
    g_autofree char* path = g_canonicalize_filename(given, NULL);
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
    gboolean ok;
    if (!g_str_has_suffix(path, ".lua")) {
        ok = evaluate_toml(state, config, path, error);
    } else {
        int status = luaL_loadfilex(state, path, "t");
        if (status == LUA_OK)
            status = lua_pcall(state, 0, 1, 0);
        if (status != LUA_OK) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s: %s", path,
                        lua_error_text(state));
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
    g_clear_pointer(&runtime->config.shortcut_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.launch_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.portal_grant_snapshot, g_variant_unref);
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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

void gnoblin_config_update_shortcut_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime};
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
    // reference to the Shell; otherwise a later event can unref a stale value.
    runtime->document = run.result ? g_variant_ref_sink(run.result) : NULL;
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

    GVariantBuilder legacy;
    g_variant_builder_init(&legacy, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&legacy, "{sv}", "request_id", g_variant_new_int64(request_id));
    g_variant_builder_add(&legacy, "{sv}", "method", g_variant_new_string(method));
    g_variant_builder_add(&legacy, "{sv}", "ok", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&legacy, "{sv}", "error", g_variant_new_string(reason));
    g_autoptr(GVariant) legacy_payload = g_variant_ref_sink(g_variant_builder_end(&legacy));
    dispatch_rejected_operation_event(state, legacy_payload, "gnoblin.api.operation-completed");
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

GVariant* gnoblin_config_dispatch_event(const char* event, GVariant* payload, GError** error) {
    // Shell is applying a newly parsed configuration before committing it.
    // Compositor preference changes in that window can emit transition events;
    // do not send those to the old Lua runtime or let it mutate the pending one.
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
    gboolean canonical_operation_event = g_str_equal(event, "gnoblin.operation.completed");
    if (g_str_equal(event, "gnoblin.api.operation-completed") || canonical_operation_event) {
        gint64 request_id = 0;
        gboolean ok = FALSE;
        const char* id_field = canonical_operation_event ? "operation_id" : "request_id";
        const char* value_field = canonical_operation_event ? "value" : "result";
        if (g_variant_lookup(payload, id_field, "x", &request_id) && request_id > 0) {
            g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
            gpointer reference = g_hash_table_lookup(active_runtime->config.operations, key);
            if (reference) {
                operation_completed = TRUE;
                lua_rawgeti(state, LUA_REGISTRYINDEX, GPOINTER_TO_INT(reference));
                int handle = lua_absindex(state, -1);
                g_variant_lookup(payload, "ok", "b", &ok);
                lua_pushstring(state, ok ? "succeeded" : "failed");
                lua_setfield(state, handle, "status");
                GVariant* result = g_variant_lookup_value(payload, value_field, NULL);
                GVariant* failure = g_variant_lookup_value(payload, "error", NULL);
                if (result) {
                    push_variant(state, result);
                    lua_setfield(state, handle, "value");
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
                lua_pushstring(state, event_names[event_index]);
                lua_setfield(state, -2, "name");
                if (g_str_equal(event_names[event_index], "gnoblin.shortcut.activated") &&
                    active_runtime->config.dispatch_focus_handle) {
                    push_focus_context(state, active_runtime->config.dispatch_focus_handle,
                                       active_runtime->config.dispatch_focus_generation,
                                       active_runtime->config.dispatch_focus_expires_at_us);
                    lua_setfield(state, -2, "focus_context");
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
    // No Shell document update is needed when Lua did not handle the event.
    // Touchpad gestures still reach the Shell router through this signal.
    if (!dispatched) {
        if (!g_str_equal(event, "mutter.touchpad.gesture"))
            return NULL;
        return g_variant_ref(active_runtime->document);
    }
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

GVariant* gnoblin_config_dispatch_shortcut_event(const char* event, GVariant* payload,
                                                 guint64 context_handle, gint64 expires_at_us,
                                                 GError** error) {
    if (!active_runtime || !context_handle || expires_at_us <= g_get_monotonic_time() || !event ||
        !g_str_equal(event, "gnoblin.shortcut.activated")) {
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
    config->dispatch_focus_expires_at_us = expires_at_us;
    GVariant* document = gnoblin_config_dispatch_event(event, payload, error);
    config->dispatch_focus_handle = 0;
    config->dispatch_focus_generation = 0;
    config->dispatch_focus_expires_at_us = 0;
    return document;
}

guint64 gnoblin_config_runtime_generation(void) {
    return active_runtime ? active_runtime->generation : 0;
}

gboolean gnoblin_config_take_focus_context(gint64 request_id, guint64* context_handle,
                                           guint64* generation) {
    if (!active_runtime || request_id <= 0 || !context_handle || !generation)
        return FALSE;
    g_autofree char* key = g_strdup_printf("%" G_GINT64_FORMAT, request_id);
    LuaFocusOperation* operation =
        g_hash_table_lookup(active_runtime->config.focus_operations, key);
    if (!operation)
        return FALSE;
    gboolean valid = operation->handle != 0 && operation->generation == active_runtime->generation;
    if (valid) {
        *context_handle = operation->handle;
        *generation = operation->generation;
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
    } else {
        domain = g_strndup(method, separator - method);
        operation = separator + 1;
    }
    lua_State* state = active_runtime->state;
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->api_calling = TRUE;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, domain);
    lua_getfield(state, -1, operation);
    if (!lua_isfunction(state, -1)) {
        lua_pop(state, 3);
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
        lua_pop(state, 1);
        lua_pop(state, 2);
        config->api_calling = FALSE;
        g_ptr_array_set_size(config->runtime_actions, action_start);
        config->actions_in_dispatch = 0;
        return NULL;
    }
    gboolean valid_ticket = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0 &&
                            config->runtime_actions->len == action_start + 1;
    lua_pop(state, 1);
    lua_pop(state, 2);
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
    g_ptr_array_remove_index(config->runtime_actions, action_start);
    return operation_call;
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
        if (next_runtime_generation < G_MAXUINT64)
            active_runtime->generation = ++next_runtime_generation;
        else
            active_runtime->generation = next_runtime_generation;
        g_clear_pointer(&active_runtime->config.settings_document, g_variant_unref);
        active_runtime->config.settings_document = g_variant_ref(active_runtime->document);
        active_runtime->config.settings_revision =
            settings_changed ? ++next_settings_revision : settings_revision;
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
    } else {
        lua_runtime_free(g_steal_pointer(&pending_runtime));
        if (active_runtime && active_runtime->config.deferred_callbacks &&
            active_runtime->config.deferred_callbacks->len > 0) {
            active_runtime->config.deferred_callbacks_scheduled = FALSE;
            schedule_deferred_callbacks(&active_runtime->config);
        }
    }
}

static GVariant* focus_policy_snapshot(GVariant* document, guint64 revision) {
    static const struct {
        const char* key;
        const char* default_value;
    } string_fields[] = {
        {"focus-mode", "click"},
        {"focus-new-windows", "smart"},
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

#include "gnoblin-console-lua.inc"
