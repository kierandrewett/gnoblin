/* Live handles for the things a shell touches: windows, workspaces and layer surfaces.
 *
 * A record of one of these kinds is interned by kind and id, so every read of the same window
 * returns the same userdata and that userdata always shows the newest state the runtime has seen.
 * Each kind declares its signals here; its properties are the fields of its annotated class (the
 * one declaration, see api/windows.h). A handle reports its end with the "closed" signal and
 * `alive == false`, and `watch` calls back when one declared property changes.
 *
 *   handle:on(signal, fn)       fn(handle, ...) for each signal; returns {unsubscribe = fn}
 *   handle:watch(property, fn)  fn(handle, value, old) when the property changes
 *   handle.alive                false once the object has ended */
#include "gnoblin-lua-internal.h"

#define LUA_OBJECT_HANDLES_KEY "gnoblin.object_handles"
#define LUA_OBJECT_LISTENERS_KEY "gnoblin.object_listeners"

typedef struct {
    const char* kind;
    const char* const* signals;
    const char* const* animatable; /* properties a tween can drive (gnoblin-lua-tween.c) */
} LuaObjectKind;

static const char* const window_signals[] = {"changed", "closed", "focused", "unfocused", NULL};
static const char* const workspace_signals[] = {"changed", "closed", "activated", NULL};
static const char* const layer_signals[] = {"changed", "closed", NULL};
static const char* const window_animatable[] = {"opacity", "scale", "rotation", "offset", NULL};
static const char* const none[] = {NULL};

static const LuaObjectKind object_kinds[] = {
    {"Window", window_signals, window_animatable},
    {"Workspace", workspace_signals, none},
    {"LayerSurface", layer_signals, none},
};

/* Which runtime events move a live object, where the object is in the payload, which signal they
 * raise and which property (besides any `changed` list) they change. */
static const struct {
    const char* event;
    const char* kind;
    const char* field;
    const char* signal;
    const char* property;
} object_events[] = {
    {"gnoblin.window.changed", "Window", "window", "changed", NULL},
    {"gnoblin.window.focused", "Window", "window", "focused", "focused"},
    {"gnoblin.window.unfocused", "Window", "window", "unfocused", "focused"},
    {"gnoblin.window.closed", "Window", "last", "closed", NULL},
    {"gnoblin.workspace.changed", "Workspace", "workspace", "changed", NULL},
    {"gnoblin.workspace.renamed", "Workspace", "workspace", "changed", "name"},
    {"gnoblin.workspace.activated", "Workspace", "workspace", "activated", "active"},
    {"gnoblin.workspace.removed", "Workspace", "last", "closed", NULL},
    {"gnoblin.layer.changed", "LayerSurface", "layer", "changed", NULL},
    {"gnoblin.layer.removed", "LayerSurface", "last", "closed", NULL},
};

static const LuaObjectKind* object_kind(const char* kind) {
    for (gsize i = 0; kind && i < G_N_ELEMENTS(object_kinds); i++)
        if (g_str_equal(object_kinds[i].kind, kind))
            return &object_kinds[i];
    return NULL;
}

gboolean lua_object_kind_is_live(const char* kind) {
    return object_kind(kind) != NULL;
}

gboolean lua_object_property_animatable(const char* kind, const char* property) {
    const LuaObjectKind* type = object_kind(kind);
    for (guint i = 0; type && property && type->animatable[i]; i++)
        if (g_str_equal(type->animatable[i], property))
            return TRUE;
    return FALSE;
}

/* Push the registry table `key`, created on first use; weak values when `weak`. */
static int push_registry_table(lua_State* state, const char* key, gboolean weak) {
    if (lua_getfield(state, LUA_REGISTRYINDEX, key) != LUA_TTABLE) {
        lua_pop(state, 1);
        lua_newtable(state);
        if (weak) {
            lua_createtable(state, 0, 1);
            lua_pushliteral(state, "v");
            lua_setfield(state, -2, "__mode");
            lua_setmetatable(state, -2);
        }
        lua_pushvalue(state, -1);
        lua_setfield(state, LUA_REGISTRYINDEX, key);
    }
    return lua_absindex(state, -1);
}

/* Push "Kind:id" for the handle at `index`, or nothing and FALSE when it is not a live handle. */
static gboolean push_handle_key(lua_State* state, int index, const char* kind) {
    index = lua_absindex(state, index);
    if (!luaL_testudata(state, index, LUA_READONLY_SNAPSHOT_METATABLE))
        return FALSE;
    lua_getiuservalue(state, index, 1);
    lua_getfield(state, -1, "id");
    if (lua_type(state, -1) != LUA_TSTRING) {
        lua_pop(state, 2);
        return FALSE;
    }
    lua_pushfstring(state, "%s:%s", kind, lua_tostring(state, -1));
    lua_replace(state, -3);
    lua_pop(state, 1);
    return TRUE;
}

static gboolean handle_closed(lua_State* state, int index);

/* Whether the value at `index` is the live handle of a `kind` object. */
gboolean lua_object_is(lua_State* state, int index, const char* kind) {
    index = lua_absindex(state, index);
    if (!push_handle_key(state, index, kind))
        return FALSE;
    int handles = push_registry_table(state, LUA_OBJECT_HANDLES_KEY, TRUE);
    lua_pushvalue(state, -2);
    lua_rawget(state, handles);
    gboolean same = lua_rawequal(state, -1, index);
    lua_pop(state, 3);
    return same;
}

static gboolean handle_closed(lua_State* state, int index) {
    gboolean* closed = lua_touserdata(state, index);
    return closed && *closed;
}

/* Replace the new record at `index` with the interned handle for its kind and id, moving the new
 * state into the handle. The handle keeps its identity and its ended state. */
void lua_object_intern(lua_State* state, const char* kind, int index) {
    index = lua_absindex(state, index);
    if (!push_handle_key(state, index, kind))
        return;
    int key = lua_absindex(state, -1);
    int handles = push_registry_table(state, LUA_OBJECT_HANDLES_KEY, TRUE);
    lua_pushvalue(state, key);
    if (lua_rawget(state, handles) == LUA_TUSERDATA && !lua_rawequal(state, -1, index)) {
        int handle = lua_absindex(state, -1);
        for (int slot = 1; slot <= 2; slot++) {
            lua_getiuservalue(state, index, slot);
            lua_setiuservalue(state, handle, slot);
        }
        lua_pushvalue(state, handle);
        lua_replace(state, index);
    } else {
        lua_pushvalue(state, key);
        lua_pushvalue(state, index);
        lua_rawset(state, handles);
    }
    lua_settop(state, key - 1);
    /* `alive` lives in the method table, which every new state replaces. */
    lua_getiuservalue(state, index, 2);
    lua_pushboolean(state, !handle_closed(state, index));
    lua_setfield(state, -2, "alive");
    lua_pop(state, 1);
}

/* The listener record of the handle at `index`, created when `create`: {handle, signals,
 * watches}. It holds the handle strongly while anything listens. */
static gboolean push_listener_record(lua_State* state, int index, const char* kind,
                                     gboolean create) {
    index = lua_absindex(state, index);
    if (!push_handle_key(state, index, kind))
        return FALSE;
    int key = lua_absindex(state, -1);
    int listeners = push_registry_table(state, LUA_OBJECT_LISTENERS_KEY, FALSE);
    lua_pushvalue(state, key);
    if (lua_rawget(state, listeners) != LUA_TTABLE) {
        lua_pop(state, 1);
        if (!create) {
            lua_settop(state, key - 1);
            return FALSE;
        }
        lua_createtable(state, 0, 3);
        lua_pushvalue(state, index);
        lua_setfield(state, -2, "handle");
        lua_newtable(state);
        lua_setfield(state, -2, "signals");
        lua_newtable(state);
        lua_setfield(state, -2, "watches");
        lua_pushvalue(state, key);
        lua_pushvalue(state, -2);
        lua_rawset(state, listeners);
    }
    lua_replace(state, key);
    lua_settop(state, key);
    return TRUE;
}

/* Append `entry` (top of stack, popped) to list `name` in the table at `parent`. */
static void append_entry(lua_State* state, int parent, const char* name) {
    parent = lua_absindex(state, parent);
    if (lua_getfield(state, parent, name) != LUA_TTABLE) {
        lua_pop(state, 1);
        lua_newtable(state);
        lua_pushvalue(state, -1);
        lua_setfield(state, parent, name);
    }
    lua_insert(state, -2);
    lua_rawseti(state, -2, (lua_Integer)lua_rawlen(state, -2) + 1);
    lua_pop(state, 1);
}

static int lua_object_unsubscribe(lua_State* state) {
    lua_pushvalue(state, lua_upvalueindex(1));
    lua_pushboolean(state, FALSE);
    lua_setfield(state, -2, "active");
    return 0;
}

static void push_subscription(lua_State* state, int entry) {
    lua_createtable(state, 0, 1);
    lua_pushvalue(state, entry);
    lua_pushcclosure(state, lua_object_unsubscribe, 1);
    lua_setfield(state, -2, "unsubscribe");
}

int lua_object_on(lua_State* state) {
    const char* kind = lua_tostring(state, lua_upvalueindex(1));
    const LuaObjectKind* type = object_kind(kind);
    const char* signal = luaL_checkstring(state, 2);
    luaL_checktype(state, 3, LUA_TFUNCTION);
    gboolean known = FALSE;
    for (guint i = 0; type && type->signals[i]; i++)
        known |= g_str_equal(type->signals[i], signal);
    if (!known)
        return luaL_error(state, "a %s has no %s signal", kind, signal);
    if (handle_closed(state, 1))
        return luaL_error(state, "this %s has ended", kind);
    if (!push_listener_record(state, 1, kind, TRUE))
        return luaL_error(state, "%s:on needs a %s", kind, kind);
    int record = lua_absindex(state, -1);
    lua_createtable(state, 0, 2);
    int entry = lua_absindex(state, -1);
    lua_pushvalue(state, 3);
    lua_setfield(state, entry, "fn");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, entry, "active");
    lua_getfield(state, record, "signals");
    lua_pushvalue(state, entry);
    append_entry(state, -2, signal);
    lua_pop(state, 1);
    push_subscription(state, entry);
    return 1;
}

int lua_object_watch(lua_State* state) {
    const char* kind = lua_tostring(state, lua_upvalueindex(1));
    const char* property = luaL_checkstring(state, 2);
    luaL_checktype(state, 3, LUA_TFUNCTION);
    if (!lua_api_class_has_field(kind, property))
        return luaL_error(state, "a %s has no %s property", kind, property);
    if (handle_closed(state, 1))
        return luaL_error(state, "this %s has ended", kind);
    if (!push_listener_record(state, 1, kind, TRUE))
        return luaL_error(state, "%s:watch needs a %s", kind, kind);
    int record = lua_absindex(state, -1);
    lua_createtable(state, 0, 3);
    int entry = lua_absindex(state, -1);
    lua_pushvalue(state, 3);
    lua_setfield(state, entry, "fn");
    lua_pushboolean(state, TRUE);
    lua_setfield(state, entry, "active");
    lua_getfield(state, 1, property);
    lua_setfield(state, entry, "last");
    lua_getfield(state, record, "watches");
    lua_pushvalue(state, entry);
    append_entry(state, -2, property);
    lua_pop(state, 1);
    push_subscription(state, entry);
    return 1;
}

/* Mark a new live record alive; its on and watch methods come from lua_api (Window:on). */
void lua_object_add_methods(lua_State* state, int method_table, const char* kind) {
    (void)kind;
    method_table = lua_absindex(state, method_table);
    lua_pushboolean(state, TRUE);
    lua_setfield(state, method_table, "alive");
}

/* Call each active entry of the list at `list` as fn(handle, ...) with `extra` arguments taken
 * from the stack top. Returns how many ran; a failing callback is logged. */
static guint run_entries(lua_State* state, int list, int handle, int extra, const char* label) {
    list = lua_absindex(state, list);
    handle = lua_absindex(state, handle);
    int first_extra = lua_gettop(state) - extra + 1;
    guint ran = 0;
    lua_Integer count = (lua_Integer)lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        lua_getfield(state, -1, "active");
        gboolean active = lua_toboolean(state, -1);
        lua_pop(state, 1);
        if (!active) {
            lua_pop(state, 1);
            continue;
        }
        lua_getfield(state, -1, "fn");
        lua_remove(state, -2);
        lua_pushvalue(state, handle);
        for (int arg = 0; arg < extra; arg++)
            lua_pushvalue(state, first_extra + arg);
        if (lua_pcall(state, 1 + extra, 0, 0) != LUA_OK) {
            g_warning("Lua %s callback failed: %s", label, lua_error_text(state));
            lua_pop(state, 1);
        }
        ran++;
    }
    return ran;
}

/* Run the watches of `property` whose last value differs from the handle's current one. */
static guint run_watches(lua_State* state, int watches, int handle, const char* property) {
    watches = lua_absindex(state, watches);
    handle = lua_absindex(state, handle);
    if (lua_getfield(state, watches, property) != LUA_TTABLE) {
        lua_pop(state, 1);
        return 0;
    }
    int list = lua_absindex(state, -1);
    guint ran = 0;
    lua_Integer count = (lua_Integer)lua_rawlen(state, list);
    for (lua_Integer i = 1; i <= count; i++) {
        lua_rawgeti(state, list, i);
        int entry = lua_absindex(state, -1);
        lua_getfield(state, entry, "active");
        gboolean active = lua_toboolean(state, -1);
        lua_pop(state, 1);
        lua_getfield(state, handle, property);
        int value = lua_absindex(state, -1);
        lua_getfield(state, entry, "last");
        int last = lua_absindex(state, -1);
        if (active && !lua_rawequal(state, value, last)) {
            lua_getfield(state, entry, "fn");
            lua_pushvalue(state, handle);
            lua_pushvalue(state, value);
            lua_pushvalue(state, last);
            lua_pushvalue(state, value);
            lua_setfield(state, entry, "last");
            if (lua_pcall(state, 3, 0, 0) != LUA_OK) {
                g_warning("Lua watch callback for %s failed: %s", property, lua_error_text(state));
                lua_pop(state, 1);
            }
            ran++;
        }
        lua_settop(state, list);
    }
    lua_pop(state, 1);
    return ran;
}

/* Update the live object an event names and run its signals and watches. Returns TRUE when a
 * callback ran. */
gboolean lua_object_dispatch_event(lua_State* state, const char* event, GVariant* payload) {
    for (gsize i = 0; i < G_N_ELEMENTS(object_events); i++) {
        if (!g_str_equal(object_events[i].event, event))
            continue;
        g_autoptr(GVariant) record =
            g_variant_lookup_value(payload, object_events[i].field, G_VARIANT_TYPE_VARDICT);
        if (!record)
            return FALSE;
        int base = lua_gettop(state);
        push_variant(state, record);
        push_readonly_copy(state, -1); /* interns: updates the handle */
        int handle = lua_absindex(state, -1);
        const char* kind = object_events[i].kind;
        gboolean closing = g_str_equal(object_events[i].signal, "closed");
        if (closing && luaL_testudata(state, handle, LUA_READONLY_SNAPSHOT_METATABLE)) {
            gboolean* closed = lua_touserdata(state, handle);
            *closed = TRUE;
            lua_getiuservalue(state, handle, 2);
            lua_pushboolean(state, FALSE);
            lua_setfield(state, -2, "alive");
            lua_pop(state, 1);
        }
        guint ran = 0;
        if (push_listener_record(state, handle, kind, FALSE)) {
            int listeners = lua_absindex(state, -1);
            lua_getfield(state, listeners, "watches");
            int watches = lua_absindex(state, -1);
            /* `changed` names the fields that moved; it arrives as strings or as variants. */
            g_autoptr(GVariant) changed = g_variant_lookup_value(payload, "changed", NULL);
            g_autoptr(GPtrArray) names = g_ptr_array_new_with_free_func(g_free);
            if (changed && g_variant_is_of_type(changed, G_VARIANT_TYPE_ARRAY)) {
                for (gsize item = 0; item < g_variant_n_children(changed); item++) {
                    g_autoptr(GVariant) child = g_variant_get_child_value(changed, item);
                    if (g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)) {
                        g_autoptr(GVariant) inner = g_variant_get_variant(child);
                        g_clear_pointer(&child, g_variant_unref);
                        child = g_steal_pointer(&inner);
                    }
                    if (g_variant_is_of_type(child, G_VARIANT_TYPE_STRING))
                        g_ptr_array_add(names, g_variant_dup_string(child, NULL));
                }
            }
            if (object_events[i].property)
                g_ptr_array_add(names, g_strdup(object_events[i].property));
            for (guint name = 0; name < names->len; name++)
                ran += run_watches(state, watches, handle, g_ptr_array_index(names, name));
            lua_getfield(state, listeners, "signals");
            if (lua_getfield(state, -1, object_events[i].signal) == LUA_TTABLE) {
                if (g_str_equal(object_events[i].signal, "changed")) {
                    lua_createtable(state, (int)names->len, 0);
                    for (guint name = 0; name < names->len; name++) {
                        lua_pushstring(state, g_ptr_array_index(names, name));
                        lua_rawseti(state, -2, name + 1);
                    }
                    ran += run_entries(state, -2, handle, 1, object_events[i].signal);
                    lua_pop(state, 1);
                } else {
                    ran += run_entries(state, -1, handle, 0, object_events[i].signal);
                }
            }
            if (closing) {
                /* An ended object raises nothing more; let go of its listeners and handle. */
                push_handle_key(state, handle, kind);
                push_registry_table(state, LUA_OBJECT_LISTENERS_KEY, FALSE);
                lua_insert(state, -2);
                lua_pushnil(state);
                lua_rawset(state, -3);
            }
        }
        lua_settop(state, base);
        return ran > 0;
    }
    return FALSE;
}
