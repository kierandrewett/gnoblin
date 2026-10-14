#include "gnoblin-lua-internal.h"

static GVariant* focus_policy_snapshot(GVariant* document, guint64 revision);

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
        {"focus-new-windows", "prevent"},
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

gboolean gnoblin_config_has_pending_document(void) {
    return active_runtime && active_runtime->pending_document;
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
    /* Use the staged config being sent to Mutter before falling back to the
     * currently active runtime. */
    LuaRuntime* runtime = pending_runtime ? pending_runtime : deferred_runtime;
    if (!runtime)
        runtime = active_runtime;
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
