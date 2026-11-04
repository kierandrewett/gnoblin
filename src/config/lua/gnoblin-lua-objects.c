/* Live handles for the things a shell touches: windows, workspaces, layer surfaces, outputs,
 * shortcuts, launches and input sources.
 *
 * A record of one of these kinds is interned by kind and key, so every read of the same window
 * returns the same userdata and that userdata always shows the newest state the runtime has seen.
 * The key is the record's id, except that a launch has its token and an input source has its type
 * and id. Each kind declares its signals here; its properties are the fields of its annotated
 * class (the one declaration, see api/windows.h). A handle reports its end with the "closed"
 * signal and `alive == false`, and `watch` calls back when one declared property changes.
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
    const char* key;               /* record field that names one object; "id" when NULL */
    gboolean diffs; /* its events carry no `changed` list: compare the record with the handle */
} LuaObjectKind;

static const char* const window_signals[] = {"changed", "closed", "focused", "unfocused", NULL};
static const char* const workspace_signals[] = {"changed", "closed", "activated", NULL};
static const char* const changed_closed[] = {"changed", "closed", NULL};
static const char* const shortcut_signals[] = {"activated",   "deactivated",   "session_activated",
                                               "session_key", "session_ended", "closed",
                                               NULL};
static const char* const launch_signals[] = {"changed", "started", "timed_out", "closed", NULL};
static const char* const window_animatable[] = {"opacity", "scale", "rotation", "offset", NULL};
static const char* const none[] = {NULL};

static const LuaObjectKind object_kinds[] = {
    {"Window", window_signals, window_animatable, NULL},
    {"Workspace", workspace_signals, none, NULL},
    {"LayerSurface", changed_closed, none, NULL},
    {"Output", changed_closed, none, NULL},
    {"Shortcut", shortcut_signals, none, NULL},
    {"Launch", launch_signals, none, "token", TRUE},
};

/* Which runtime events move a live object, where the object is in the payload, which signal they
 * raise and which property (besides any `changed` list) they change. An entry with a state applies
 * only when the record's "state" is `when_state`, or is not `unless_state`; one event may have
 * several entries. */
static const struct {
    const char* event;
    const char* kind;
    const char* field;
    const char* signal;
    const char* property;
    const char* when_state;
    const char* unless_state;
} object_events[] = {
    {"gnoblin.window.changed", "Window", "window", "changed", NULL, NULL, NULL},
    {"gnoblin.window.focused", "Window", "window", "focused", "focused", NULL, NULL},
    {"gnoblin.window.unfocused", "Window", "window", "unfocused", "focused", NULL, NULL},
    {"gnoblin.window.closed", "Window", "last", "closed", NULL, NULL, NULL},
    {"gnoblin.workspace.changed", "Workspace", "workspace", "changed", NULL, NULL, NULL},
    {"gnoblin.workspace.renamed", "Workspace", "workspace", "changed", "name", NULL, NULL},
    {"gnoblin.workspace.activated", "Workspace", "workspace", "activated", "active", NULL, NULL},
    {"gnoblin.workspace.removed", "Workspace", "last", "closed", NULL, NULL, NULL},
    {"gnoblin.layer.changed", "LayerSurface", "layer", "changed", NULL, NULL, NULL},
    {"gnoblin.layer.removed", "LayerSurface", "last", "closed", NULL, NULL, NULL},
    {"gnoblin.output.changed", "Output", "output", "changed", NULL, NULL, NULL},
    {"gnoblin.output.removed", "Output", "last", "closed", NULL, NULL, NULL},
    {"gnoblin.launch.changed", "Launch", "launch", "changed", NULL, NULL, NULL},
    {"gnoblin.launch.changed", "Launch", "launch", "started", NULL, "started", NULL},
    {"gnoblin.launch.changed", "Launch", "launch", "timed_out", NULL, "timed_out", NULL},
    {"gnoblin.launch.changed", "Launch", "launch", "closed", NULL, NULL, "pending"},
};

/* Events that raise a signal on an object without changing its record: the payload names the
 * object by "id" and goes to the listener as its event. */
static const struct {
    const char* event;
    const char* kind;
    const char* signal;
} signal_events[] = {
    {"gnoblin.shortcut.binding-activated", "Shortcut", "activated"},
    {"gnoblin.shortcut.binding-deactivated", "Shortcut", "deactivated"},
    {"gnoblin.shortcut.session.activated", "Shortcut", "session_activated"},
    {"gnoblin.shortcut.session.key", "Shortcut", "session_key"},
    {"gnoblin.shortcut.session.ended", "Shortcut", "session_ended"},
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

/* Push "Kind:key" for the handle at `index`, or nothing and FALSE when it is not a live handle. */
static gboolean push_handle_key(lua_State* state, int index, const char* kind) {
    index = lua_absindex(state, index);
    if (!luaL_testudata(state, index, LUA_READONLY_SNAPSHOT_METATABLE))
        return FALSE;
    const LuaObjectKind* type = object_kind(kind);
    int base = lua_gettop(state);
    lua_getiuservalue(state, index, 1);
    int backing = lua_absindex(state, -1);
    const char* id =
        lua_getfield(state, backing, type && type->key ? type->key : "id") == LUA_TSTRING
            ? lua_tostring(state, -1)
            : NULL;
    if (!id) {
        lua_settop(state, base);
        return FALSE;
    }
    lua_pushfstring(state, "%s:%s", kind, id);
    lua_replace(state, base + 1);
    lua_settop(state, base + 1);
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

/* Whether the "state" field of the record at `index` equals `value`. */
static gboolean record_state_is(lua_State* state, int index, const char* value) {
    index = lua_absindex(state, index);
    lua_getiuservalue(state, index, 1);
    gboolean same = lua_getfield(state, -1, "state") == LUA_TSTRING &&
                    g_str_equal(lua_tostring(state, -1), value);
    lua_pop(state, 2);
    return same;
}

/* A launch ends when it stops waiting for its window. */
static gboolean record_has_ended(lua_State* state, const char* kind, int index) {
    return g_str_equal(kind, "Launch") && !record_state_is(state, index, "pending");
}

/* Replace the new record at `index` with the interned handle for its kind and key, moving the new
 * state into the handle. The handle keeps its identity and its ended state. A new launch with the
 * token of an ended one is another object. */
void lua_object_intern(lua_State* state, const char* kind, int index) {
    index = lua_absindex(state, index);
    if (!push_handle_key(state, index, kind))
        return;
    int key = lua_absindex(state, -1);
    int handles = push_registry_table(state, LUA_OBJECT_HANDLES_KEY, TRUE);
    if (g_str_equal(kind, "Launch") && record_state_is(state, index, "pending")) {
        lua_pushvalue(state, key);
        if (lua_rawget(state, handles) == LUA_TUSERDATA && handle_closed(state, -1)) {
            lua_pushvalue(state, key);
            lua_pushnil(state);
            lua_rawset(state, handles);
        }
        lua_pop(state, 1);
    }
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
    if (record_has_ended(state, kind, index))
        *(gboolean*)lua_touserdata(state, index) = TRUE;
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

/* Keep the handle at `index` alive, and listed, until it ends. */
void lua_object_hold(lua_State* state, const char* kind, int index) {
    if (push_listener_record(state, index, kind, TRUE))
        lua_pop(state, 1);
}

static int compare_keys(gconstpointer a, gconstpointer b) {
    return strcmp(*(const char* const*)a, *(const char* const*)b);
}

/* Push an array of the live handles of `kind` that something holds, ordered by key. */
void lua_object_push_held(lua_State* state, const char* kind) {
    int listeners = push_registry_table(state, LUA_OBJECT_LISTENERS_KEY, FALSE);
    g_autofree char* prefix = g_strdup_printf("%s:", kind);
    g_autoptr(GPtrArray) keys = g_ptr_array_new_with_free_func(g_free);
    lua_pushnil(state);
    while (lua_next(state, listeners)) {
        if (lua_type(state, -2) == LUA_TSTRING && g_str_has_prefix(lua_tostring(state, -2), prefix))
            g_ptr_array_add(keys, g_strdup(lua_tostring(state, -2)));
        lua_pop(state, 1);
    }
    g_ptr_array_sort(keys, compare_keys);
    lua_newtable(state);
    mark_array_table(state, -1);
    int result = lua_absindex(state, -1);
    for (guint i = 0; i < keys->len; i++) {
        lua_pushstring(state, g_ptr_array_index(keys, i));
        lua_rawget(state, listeners);
        lua_getfield(state, -1, "handle");
        lua_remove(state, -2);
        lua_rawseti(state, result, i + 1);
    }
    lua_replace(state, listeners);
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
 * from the stack top, or, when `event` is given, as fn(handle, event) with a new event table for
 * each entry. Returns how many ran; a failing callback is logged. */
static guint run_entries(lua_State* state, int list, int handle, int extra, const char* label,
                         const char* event, GVariant* event_payload) {
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
        int arguments = 1;
        if (event) {
            push_event_table(state, event, event_payload);
            arguments++;
        }
        for (int arg = 0; !event && arg < extra; arg++, arguments++)
            lua_pushvalue(state, first_extra + arg);
        if (lua_pcall(state, arguments, 0, 0) != LUA_OK) {
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

/* Run the watches of each field in `names`, then the listeners of "changed" (given the names) and
 * of `signal`, for the handle at `handle`. Returns how many callbacks ran. */
static guint raise_signals(lua_State* state, int handle, const char* kind, GPtrArray* names,
                           const char* signal) {
    handle = lua_absindex(state, handle);
    int base = lua_gettop(state);
    guint ran = 0;
    if (push_listener_record(state, handle, kind, FALSE)) {
        int listeners = lua_absindex(state, -1);
        lua_getfield(state, listeners, "watches");
        int watches = lua_absindex(state, -1);
        for (guint name = 0; name < names->len; name++)
            ran += run_watches(state, watches, handle, g_ptr_array_index(names, name));
        lua_getfield(state, listeners, "signals");
        if (lua_getfield(state, -1, signal) == LUA_TTABLE) {
            if (g_str_equal(signal, "changed")) {
                lua_createtable(state, (int)names->len, 0);
                for (guint name = 0; name < names->len; name++) {
                    lua_pushstring(state, g_ptr_array_index(names, name));
                    lua_rawseti(state, -2, name + 1);
                }
                ran += run_entries(state, -2, handle, 1, signal, NULL, NULL);
            } else {
                ran += run_entries(state, -1, handle, 0, signal, NULL, NULL);
            }
        }
    }
    lua_settop(state, base);
    return ran;
}

static void mark_closed(lua_State* state, int handle) {
    handle = lua_absindex(state, handle);
    gboolean* closed = lua_touserdata(state, handle);
    *closed = TRUE;
    lua_getiuservalue(state, handle, 2);
    lua_pushboolean(state, FALSE);
    lua_setfield(state, -2, "alive");
    lua_pop(state, 1);
}

/* An ended object raises nothing more; let go of its listeners and handle. */
static void release_listeners(lua_State* state, int handle, const char* kind) {
    int base = lua_gettop(state);
    if (push_handle_key(state, handle, kind)) {
        push_registry_table(state, LUA_OBJECT_LISTENERS_KEY, FALSE);
        lua_insert(state, -2);
        lua_pushnil(state);
        lua_rawset(state, -3);
    }
    lua_settop(state, base);
}

/* End the live object at `handle`: it stops being alive and its "closed" listeners run, given
 * `reason` when there is one. Does nothing when it has ended. */
void lua_object_end(lua_State* state, const char* kind, int handle, const char* reason) {
    handle = lua_absindex(state, handle);
    if (!luaL_testudata(state, handle, LUA_READONLY_SNAPSHOT_METATABLE) ||
        handle_closed(state, handle))
        return;
    mark_closed(state, handle);
    int base = lua_gettop(state);
    if (push_listener_record(state, handle, kind, FALSE)) {
        lua_getfield(state, -1, "signals");
        if (lua_getfield(state, -1, "closed") == LUA_TTABLE) {
            if (reason)
                lua_pushstring(state, reason);
            run_entries(state, -1 - (reason != NULL), handle, reason != NULL, "closed", NULL, NULL);
        }
    }
    lua_settop(state, base);
    release_listeners(state, handle, kind);
}

static gboolean record_state_allows(GVariant* record, const char* when_state,
                                    const char* unless_state) {
    const char* state = NULL;
    g_variant_lookup(record, "state", "&s", &state);
    return (!when_state || g_strcmp0(state, when_state) == 0) &&
           (!unless_state || g_strcmp0(state, unless_state) != 0);
}

/* Push the handle the registry holds for `key`, or nothing and FALSE. */
static gboolean push_registered_handle(lua_State* state, const char* key) {
    int handles = push_registry_table(state, LUA_OBJECT_HANDLES_KEY, TRUE);
    lua_pushstring(state, key);
    if (lua_rawget(state, handles) != LUA_TUSERDATA) {
        lua_pop(state, 2);
        return FALSE;
    }
    lua_remove(state, -2);
    return TRUE;
}

/* A signal that names an object but does not change it: call its listeners with the event. */
static gboolean dispatch_signal_event(lua_State* state, const char* event, GVariant* payload) {
    const char* id = NULL;
    for (gsize i = 0; i < G_N_ELEMENTS(signal_events); i++) {
        if (!g_str_equal(signal_events[i].event, event))
            continue;
        if (!g_variant_lookup(payload, "id", "&s", &id))
            return FALSE;
        int base = lua_gettop(state);
        g_autofree char* key = g_strdup_printf("%s:%s", signal_events[i].kind, id);
        guint ran = 0;
        if (push_registered_handle(state, key) &&
            push_listener_record(state, -1, signal_events[i].kind, FALSE)) {
            lua_getfield(state, -1, "signals");
            if (lua_getfield(state, -1, signal_events[i].signal) == LUA_TTABLE)
                ran = run_entries(state, -1, base + 1, 0, signal_events[i].signal, event, payload);
        }
        lua_settop(state, base);
        return ran > 0;
    }
    return FALSE;
}

/* The registry key of the object a record describes: "Kind:key", or NULL without a key. */
static char* record_handle_key(const LuaObjectKind* type, GVariant* record) {
    const char* value = NULL;
    if (!type || !g_variant_lookup(record, type->key ? type->key : "id", "&s", &value))
        return NULL;
    return g_strdup_printf("%s:%s", type->kind, value);
}

/* Push a shallow copy of the fields of the record behind the handle at `handle`. */
static void push_field_copy(lua_State* state, int handle) {
    handle = lua_absindex(state, handle);
    lua_newtable(state);
    int copy = lua_absindex(state, -1);
    lua_getiuservalue(state, handle, 1);
    lua_pushnil(state);
    while (lua_next(state, -2)) {
        lua_pushvalue(state, -2);
        lua_insert(state, -2);
        lua_rawset(state, copy);
    }
    lua_pop(state, 1);
}

/* Add to `names` each field, except the revision, that differs between the copy at `before`
 * and the handle at `handle`. */
static void collect_field_changes(lua_State* state, int before, int handle, GPtrArray* names) {
    before = lua_absindex(state, before);
    handle = lua_absindex(state, handle);
    lua_getiuservalue(state, handle, 1);
    int backing = lua_absindex(state, -1);
    for (int pass = 0; pass < 2; pass++) {
        lua_pushnil(state);
        while (lua_next(state, pass == 0 ? backing : before)) {
            const char* name = lua_type(state, -2) == LUA_TSTRING ? lua_tostring(state, -2) : NULL;
            lua_pop(state, 1); /* the value; compare through both tables */
            if (!name || g_str_equal(name, "revision"))
                continue;
            lua_getfield(state, backing, name);
            lua_getfield(state, before, name);
            gboolean same = lua_rawequal(state, -1, -2);
            lua_pop(state, 2);
            gboolean listed = FALSE;
            for (guint i = 0; i < names->len; i++)
                listed |= g_str_equal(g_ptr_array_index(names, i), name);
            if (!same && !listed)
                g_ptr_array_add(names, g_strdup(name));
        }
    }
    lua_pop(state, 1);
}

/* Update the live object an event names and run its signals and watches. Returns TRUE when a
 * callback ran. */
gboolean lua_object_dispatch_event(lua_State* state, const char* event, GVariant* payload) {
    gboolean ran_any = dispatch_signal_event(state, event, payload);
    for (gsize i = 0; i < G_N_ELEMENTS(object_events); i++) {
        if (!g_str_equal(object_events[i].event, event))
            continue;
        g_autoptr(GVariant) record =
            g_variant_lookup_value(payload, object_events[i].field, G_VARIANT_TYPE_VARDICT);
        if (!record || !record_state_allows(record, object_events[i].when_state,
                                            object_events[i].unless_state))
            continue;
        const LuaObjectKind* type = object_kind(object_events[i].kind);
        int base = lua_gettop(state);
        /* A record without a `changed` list is compared with the state the handle had. */
        int before = 0;
        if (type && type->diffs) {
            g_autofree char* key = record_handle_key(type, record);
            if (key && push_registered_handle(state, key)) {
                push_field_copy(state, -1);
                lua_remove(state, -2);
                before = lua_gettop(state);
            }
        }
        push_variant(state, record);
        push_readonly_copy(state, -1); /* interns: updates the handle */
        int handle = lua_absindex(state, -1);
        const char* kind = object_events[i].kind;
        gboolean closing = g_str_equal(object_events[i].signal, "closed");
        if (closing && luaL_testudata(state, handle, LUA_READONLY_SNAPSHOT_METATABLE))
            mark_closed(state, handle);
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
        if (before)
            collect_field_changes(state, before, handle, names);
        gboolean listed = FALSE;
        for (guint name = 0; object_events[i].property && name < names->len; name++)
            listed |= g_str_equal(g_ptr_array_index(names, name), object_events[i].property);
        if (object_events[i].property && !listed)
            g_ptr_array_add(names, g_strdup(object_events[i].property));
        guint ran = raise_signals(state, handle, kind, names, object_events[i].signal);
        if (closing)
            release_listeners(state, handle, kind);
        lua_settop(state, base);
        ran_any |= ran > 0;
    }
    return ran_any;
}

/* Callback on an Operation that decides the fate of a handle. Upvalue 1 is the kind, 2 the
 * handle and 3 whether success (not failure) ends it. A failure ends it with the error message,
 * so a listener of "closed" learns why. */
static int lua_object_operation_done(lua_State* state) {
    const char* kind = lua_tostring(state, lua_upvalueindex(1));
    gboolean end_on_success = lua_toboolean(state, lua_upvalueindex(3));
    lua_settop(state, 2);
    gboolean failed = !lua_isnil(state, 2);
    if (failed == end_on_success)
        return 0;
    const char* reason = NULL;
    if (failed && lua_istable(state, 2) && lua_getfield(state, 2, "message") == LUA_TSTRING)
        reason = lua_tostring(state, -1);
    if (failed)
        g_warning("Lua %s ended: %s", kind, reason ? reason : "the operation failed");
    lua_pushvalue(state, lua_upvalueindex(2));
    int handle = lua_gettop(state);
    if (failed) {
        /* A launch that never began is no longer pending. */
        lua_getiuservalue(state, handle, 1);
        if (lua_getfield(state, -1, "state") == LUA_TSTRING &&
            g_str_equal(lua_tostring(state, -1), "pending")) {
            lua_pushliteral(state, "ended");
            lua_setfield(state, -3, "state");
        }
        lua_pop(state, 2);
    }
    lua_object_end(state, kind, handle, reason);
    return 0;
}

/* End the handle at `handle` when the pending Operation at `operation` fails, or, with
 * `end_on_success`, when it succeeds. */
void lua_object_end_after(lua_State* state, LuaConfig* config, int operation, const char* kind,
                          int handle, gboolean end_on_success) {
    operation = lua_absindex(state, operation);
    handle = lua_absindex(state, handle);
    lua_pushstring(state, kind);
    lua_pushvalue(state, handle);
    lua_pushboolean(state, end_on_success);
    lua_pushcclosure(state, lua_object_operation_done, 3);
    subscribe_operation(state, config, operation);
    lua_pop(state, 1);
}

/* Push the registered handle of `kind` with this key, or nothing and FALSE. */
gboolean lua_object_push_registered(lua_State* state, const char* kind, const char* key) {
    g_autofree char* full = g_strdup_printf("%s:%s", kind, key);
    return push_registered_handle(state, full);
}

/* Forget the registered handle of `kind` with this key, so the next record with it is a new
 * object. Handles already given out stay as they are. */
void lua_object_forget(lua_State* state, const char* kind, const char* key) {
    int handles = push_registry_table(state, LUA_OBJECT_HANDLES_KEY, TRUE);
    lua_pushfstring(state, "%s:%s", kind, key);
    lua_pushnil(state);
    lua_rawset(state, handles);
    lua_pop(state, 1);
}
