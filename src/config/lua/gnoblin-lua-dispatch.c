#include "gnoblin-lua-internal.h"

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
    gint64 source_sequence = 0;
    guint64 sequence = config->input_gesture_sequence;
    if (g_variant_lookup(payload, "sequence", "x", &source_sequence) && source_sequence > 0 &&
        (guint64)source_sequence > sequence)
        sequence = (guint64)source_sequence;
    else if (sequence < G_MAXINT64)
        sequence++;
    config->input_gesture_sequence = sequence;
    g_variant_builder_add(&event, "{sv}", "sequence", g_variant_new_int64((gint64)sequence));
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
                gboolean focus_context_event =
                    g_str_equal(event_names[event_index], "gnoblin.shortcut.activated") ||
                    g_str_equal(event_names[event_index], "gnoblin.shortcut.session.key") ||
                    g_str_equal(event_names[event_index], "gnoblin.pointer.binding-activated");
                if (g_str_equal(event_names[event_index], "gnoblin.shortcut.binding-activated")) {
                    lua_getfield(state, -1, "first");
                    focus_context_event = lua_toboolean(state, -1);
                    lua_pop(state, 1);
                }
                if (focus_context_event && active_runtime->config.dispatch_focus_handle) {
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

/* Input policy is a decision channel, not a configuration event.  It never
 * serializes gnoblin.config or commits a settings revision: compositor input
 * delivery must remain independent of config transaction latency. */
GVariant* gnoblin_config_dispatch_input(GVariant* payload, GError** error) {
    if (!active_runtime || !payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid Lua input policy request");
        return NULL;
    }
    g_autoptr(GVariant) event = g_variant_lookup_value(payload, "event", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) handler_ids =
        g_variant_lookup_value(payload, "handler_ids", G_VARIANT_TYPE_STRING_ARRAY);
    if (!event || !handler_ids) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input policy request requires event and handler_ids");
        return NULL;
    }
    lua_State* state = active_runtime->state;
    LuaConfig* config = &active_runtime->config;
    guint64 focus_handle = 0, native_generation = 0;
    gint64 focus_expiry = 0;
    gboolean has_focus =
        g_variant_lookup(payload, "focus_context_handle", "t", &focus_handle) && focus_handle &&
        g_variant_lookup(payload, "focus_context_generation", "t", &native_generation) &&
        native_generation &&
        g_variant_lookup(payload, "focus_context_expires_at_us", "x", &focus_expiry) &&
        focus_expiry > g_get_monotonic_time();
    if (has_focus) {
        config->dispatch_focus_handle = focus_handle;
        config->dispatch_focus_generation = active_runtime->generation;
        config->dispatch_native_focus_generation = native_generation;
        config->dispatch_focus_expires_at_us = focus_expiry;
    }
    g_autofree char* selected = NULL;
    g_autofree char* action = g_strdup("forward");
    gboolean callback_failed = FALSE;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "_input_handlers");
    if (!lua_istable(state, -1)) {
        lua_pop(state, 2);
        goto result;
    }
    config->dispatching = TRUE;
    config->input_dispatching = TRUE;
    guint action_start = config->runtime_actions ? config->runtime_actions->len : 0;
    for (gsize i = 0; i < g_variant_n_children(handler_ids); i++) {
        g_autoptr(GVariant) item = g_variant_get_child_value(handler_ids, i);
        const char* id = g_variant_get_string(item, NULL);
        if (!id || !*id)
            continue;
        gboolean handler_failed = FALSE;
        lua_getfield(state, -1, id);
        if (!lua_isfunction(state, -1)) {
            lua_pop(state, 1);
            continue;
        }
        push_variant(state, event);
        push_readonly_event_fields(state, event, -1);
        lua_pushliteral(state, "gnoblin.input");
        lua_setfield(state, -2, "name");
        if (has_focus) {
            push_focus_context(state, focus_handle, active_runtime->generation, native_generation,
                               focus_expiry);
            lua_setfield(state, -2, "focus_context");
        }
        if (lua_pcall(state, 1, 1, 0) != LUA_OK) {
            g_warning("Lua input handler '%s' failed: %s", id, lua_error_text(state));
            record_runtime_callback_error("input handler", lua_error_text(state));
            lua_pop(state, 1);
            lua_pushnil(state);
            lua_setfield(state, -2, id);
            callback_failed = TRUE;
            handler_failed = TRUE;
            break;
        }
        const char* returned = lua_tostring(state, -1);
        if (returned && (g_str_equal(returned, "forward") || g_str_equal(returned, "consume"))) {
            g_free(action);
            action = g_strdup(returned);
            g_free(selected);
            selected = g_strdup(id);
            lua_pop(state, 1);
            if (g_str_equal(action, "consume"))
                break;
            continue;
        }
        if (lua_istable(state, -1)) {
            if (!table_fields_allowed(state, -1, (const char* const[]){"action", NULL}))
                handler_failed = TRUE;
            lua_getfield(state, -1, "action");
            returned = lua_tostring(state, -1);
            if (returned &&
                (g_str_equal(returned, "forward") || g_str_equal(returned, "consume"))) {
                g_free(action);
                action = g_strdup(returned);
                g_free(selected);
                selected = g_strdup(id);
            } else if (returned)
                handler_failed = TRUE;
            else
                handler_failed = TRUE;
            lua_pop(state, 1);
        } else if (!lua_isnil(state, -1)) {
            handler_failed = TRUE;
        }
        lua_pop(state, 1);
        if (handler_failed) {
            record_runtime_callback_error("input handler", "invalid return value");
            lua_pushnil(state);
            lua_setfield(state, -2, id);
            callback_failed = TRUE;
        }
        if (g_str_equal(action, "consume"))
            break;
    }
    config->dispatching = FALSE;
    config->input_dispatching = FALSE;
    config->actions_in_dispatch = 0;
    lua_pop(state, 2); /* handler registry, gnoblin */
    if (callback_failed && config->runtime_actions) {
        for (guint i = action_start; i < config->runtime_actions->len; i++)
            fail_dropped_operation(active_runtime, g_ptr_array_index(config->runtime_actions, i),
                                   "input policy handler failed");
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_free(action);
        action = g_strdup("forward");
        g_clear_pointer(&selected, g_free);
    }
    config->dispatch_focus_handle = 0;
    config->dispatch_focus_generation = 0;
    config->dispatch_native_focus_generation = 0;
    config->dispatch_focus_expires_at_us = 0;
result:
    config->dispatch_focus_handle = 0;
    config->dispatch_focus_generation = 0;
    config->dispatch_native_focus_generation = 0;
    config->dispatch_focus_expires_at_us = 0;
    GVariantBuilder decision;
    g_variant_builder_init(&decision, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&decision, "{sv}", "action", g_variant_new_string(action));
    if (selected)
        g_variant_builder_add(&decision, "{sv}", "handler_id", g_variant_new_string(selected));
    if (callback_failed)
        g_variant_builder_add(&decision, "{sv}", "error",
                              g_variant_new_string("Lua input handler failed"));
    return g_variant_ref_sink(g_variant_builder_end(&decision));
}

GVariant* gnoblin_config_dispatch_shortcut_event(const char* event, GVariant* payload,
                                                 guint64 context_handle, guint64 native_generation,
                                                 gint64 expires_at_us, GError** error) {
    if (!active_runtime || !context_handle || !native_generation ||
        expires_at_us <= g_get_monotonic_time() || !event ||
        (!g_str_equal(event, "gnoblin.shortcut.activated") &&
         !g_str_equal(event, "gnoblin.shortcut.binding-activated") &&
         !g_str_equal(event, "gnoblin.shortcut.session.key") &&
         !g_str_equal(event, "gnoblin.pointer.binding-activated"))) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "invalid trusted shortcut event context");
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
