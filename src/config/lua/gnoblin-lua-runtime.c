#include "gnoblin-lua-internal.h"

GVariant* gnoblin_config_default_document(void) {
    LuaConfig config = {
        .paths = g_ptr_array_new_with_free_func(g_free),
        .directories = g_ptr_array_new_with_free_func(g_free),
        .active = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
        .modules = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL),
        .current_path = g_strdup("/gnoblin-builtin/init.lua"),
        .embedded_defaults = TRUE,
    };
    lua_State* state = new_config_state(&config);
    EvalRun run = {.config = &config, .path = config.current_path};
    if (!state) {
        g_warning("gnoblin: could not allocate Lua for built-in defaults");
    } else {
        int steps = MAX_CONFIG_STEPS / 1000;
        memcpy(lua_getextraspace(state), &steps, sizeof steps);
        lua_sethook(state, limit_hook, LUA_MASKCOUNT, 1000);
        lua_pushlightuserdata(state, &run);
        lua_pushcclosure(state, protected_eval, 1);
        if (lua_pcall(state, 0, 0, 0) != LUA_OK) {
            g_set_error(&run.error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(state));
            lua_pop(state, 1);
        }
        lua_close(state);
    }
    if (run.error)
        g_warning("gnoblin: could not evaluate embedded defaults: %s", run.error->message);
    GVariant* document = run.result ? g_variant_ref_sink(run.result) : NULL;
    if (!document || !gnoblin_config_validate_document(document, &run.error)) {
        if (run.error)
            g_warning("gnoblin: embedded defaults failed validation: %s", run.error->message);
        g_clear_pointer(&document, g_variant_unref);
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        document = g_variant_ref_sink(g_variant_builder_end(&empty));
    }
    g_clear_error(&run.error);
    g_ptr_array_unref(config.paths);
    g_ptr_array_unref(config.directories);
    g_hash_table_unref(config.active);
    g_hash_table_unref(config.modules);
    g_free(config.current_path);
    return document;
}

void lua_runtime_free(LuaRuntime* runtime) {
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
    g_clear_pointer(&runtime->config.skipped_files, g_ptr_array_unref);
    g_clear_pointer(&runtime->config.modules, g_hash_table_unref);
    g_clear_pointer(&runtime->config.window_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.workspace_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.monitor_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.output_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.monitor_privacy_screen_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.layer_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.capability_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.input_device_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.input_source_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.orientation_lock_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.appearance_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.shortcut_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.launch_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.animation_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.privacy_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.session_activity_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.session_lock_snapshot, g_variant_unref);
    g_clear_pointer(&runtime->config.session_lifecycle_snapshot, g_variant_unref);
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

void gnoblin_config_update_output_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.output_snapshot, g_variant_unref);
        runtime->config.output_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.output_snapshot = g_variant_ref(snapshot);
    }
}

static gboolean monitor_privacy_screen_snapshot_valid(GVariant* snapshot) {
    static const char* const snapshot_fields[] = {"requested_enabled", "source", "revision",
                                                  "monitors", NULL};
    static const char* const monitor_fields[] = {"id", "available", "enabled", "locked", NULL};
    if (!snapshot || !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    GVariantIter fields;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&fields, snapshot);
    while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
        g_autoptr(GVariant) field = value;
        gboolean known = FALSE;
        for (guint i = 0; snapshot_fields[i]; i++)
            known |= g_str_equal(key, snapshot_fields[i]);
        if (!known)
            return FALSE;
    }
    g_autoptr(GVariant) requested_any = g_variant_lookup_value(snapshot, "requested_enabled", NULL);
    g_autoptr(GVariant) requested =
        g_variant_lookup_value(snapshot, "requested_enabled", G_VARIANT_TYPE_BOOLEAN);
    g_autoptr(GVariant) source = g_variant_lookup_value(snapshot, "source", G_VARIANT_TYPE_STRING);
    g_autoptr(GVariant) monitors =
        g_variant_lookup_value(snapshot, "monitors", G_VARIANT_TYPE("av"));
    g_autoptr(GVariant) monitors_any = g_variant_lookup_value(snapshot, "monitors", NULL);
    g_autoptr(GVariant) revision = g_variant_lookup_value(snapshot, "revision", NULL);
    const char* source_name = source ? g_variant_get_string(source, NULL) : NULL;
    if ((requested_any && !requested) || !source ||
        !(g_str_equal(source_name, "runtime") || g_str_equal(source_name, "config") ||
          g_str_equal(source_name, "system")) ||
        !monitors || (monitors_any && !g_variant_is_of_type(monitors_any, G_VARIANT_TYPE("av"))) ||
        (revision && !g_variant_is_of_type(revision, G_VARIANT_TYPE_UINT64) &&
         !g_variant_is_of_type(revision, G_VARIANT_TYPE_INT64)))
        return FALSE;
    if (revision && g_variant_is_of_type(revision, G_VARIANT_TYPE_INT64) &&
        g_variant_get_int64(revision) < 0)
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(monitors); i++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(monitors, i);
        g_autoptr(GVariant) monitor = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                          ? g_variant_get_variant(wrapped)
                                          : g_variant_ref(wrapped);
        if (!g_variant_is_of_type(monitor, G_VARIANT_TYPE_VARDICT))
            return FALSE;
        GVariantIter monitor_iter;
        const char* monitor_key;
        GVariant* monitor_value;
        g_variant_iter_init(&monitor_iter, monitor);
        while (g_variant_iter_next(&monitor_iter, "{&sv}", &monitor_key, &monitor_value)) {
            g_autoptr(GVariant) field = monitor_value;
            gboolean known = FALSE;
            for (guint j = 0; monitor_fields[j]; j++)
                known |= g_str_equal(monitor_key, monitor_fields[j]);
            if (!known)
                return FALSE;
        }
        g_autoptr(GVariant) id = g_variant_lookup_value(monitor, "id", G_VARIANT_TYPE_STRING);
        g_autoptr(GVariant) available =
            g_variant_lookup_value(monitor, "available", G_VARIANT_TYPE_BOOLEAN);
        g_autoptr(GVariant) id_any = g_variant_lookup_value(monitor, "id", NULL);
        g_autoptr(GVariant) available_any = g_variant_lookup_value(monitor, "available", NULL);
        if (!id || !g_variant_get_string(id, NULL)[0] || (id_any && !id) || !available ||
            (available_any && !available))
            return FALSE;
        const char* const optional_boolean_fields[] = {"enabled", "locked", NULL};
        for (guint j = 0; optional_boolean_fields[j]; j++) {
            g_autoptr(GVariant) optional =
                g_variant_lookup_value(monitor, optional_boolean_fields[j], NULL);
            if (optional && !g_variant_is_of_type(optional, G_VARIANT_TYPE_BOOLEAN))
                return FALSE;
        }
    }
    return TRUE;
}

void gnoblin_config_update_monitor_privacy_screen_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !monitor_privacy_screen_snapshot_valid(snapshot))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.monitor_privacy_screen_snapshot, g_variant_unref);
        if (!snapshot)
            continue;
        GVariantBuilder with_revision;
        g_variant_builder_init(&with_revision, G_VARIANT_TYPE_VARDICT);
        GVariantIter iter;
        const char* name;
        GVariant* field;
        g_variant_iter_init(&iter, snapshot);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &field)) {
            g_autoptr(GVariant) item = field;
            if (!g_str_equal(name, "revision"))
                g_variant_builder_add(&with_revision, "{sv}", name, item);
        }
        g_variant_builder_add(&with_revision, "{sv}", "revision", g_variant_new_uint64(revision));
        runtime->config.monitor_privacy_screen_snapshot =
            g_variant_ref_sink(g_variant_builder_end(&with_revision));
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

void gnoblin_config_update_appearance_snapshot(GVariant* snapshot, guint64 revision) {
    (void)revision;
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    if (snapshot) {
        const char* color_scheme = NULL;
        if (!g_variant_lookup(snapshot, "color_scheme", "&s", &color_scheme) ||
            (!g_str_equal(color_scheme, "default") && !g_str_equal(color_scheme, "prefer-dark") &&
             !g_str_equal(color_scheme, "prefer-light")))
            return;
    }
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.appearance_snapshot, g_variant_unref);
        runtime->config.appearance_snapshot_received = TRUE;
        if (snapshot)
            runtime->config.appearance_snapshot = g_variant_ref(snapshot);
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
        runtime->config.session_lock_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.session_lock_snapshot = g_variant_ref(snapshot);
    }
}

void gnoblin_config_update_session_lifecycle_snapshot(GVariant* snapshot, guint64 revision) {
    if (snapshot && !g_variant_is_of_type(snapshot, G_VARIANT_TYPE_VARDICT))
        return;
    const char* state = NULL;
    guint64 snapshot_revision = 0;
    if (snapshot && (g_variant_n_children(snapshot) != 2 ||
                     !g_variant_lookup(snapshot, "state", "&s", &state) ||
                     !(g_str_equal(state, "starting") || g_str_equal(state, "running") ||
                       g_str_equal(state, "stopping")) ||
                     !g_variant_lookup(snapshot, "revision", "t", &snapshot_revision) ||
                     snapshot_revision != revision))
        return;
    LuaRuntime* runtimes[] = {active_runtime, pending_runtime, deferred_runtime};
    for (guint i = 0; i < G_N_ELEMENTS(runtimes); i++) {
        LuaRuntime* runtime = runtimes[i];
        if (!runtime || (i == 1 && runtime == runtimes[0]))
            continue;
        g_clear_pointer(&runtime->config.session_lifecycle_snapshot, g_variant_unref);
        runtime->config.session_lifecycle_revision = snapshot ? revision : 0;
        if (snapshot)
            runtime->config.session_lifecycle_snapshot = g_variant_ref(snapshot);
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

static LuaRuntime* lua_runtime_new(const char* path, GError** error) {
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
    if (snapshot_source)
        runtime->config.input_gesture_sequence = snapshot_source->config.input_gesture_sequence;
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
    if (snapshot_source && snapshot_source->config.output_snapshot) {
        runtime->config.output_snapshot = g_variant_ref(snapshot_source->config.output_snapshot);
        runtime->config.output_revision = snapshot_source->config.output_revision;
    }
    if (snapshot_source && snapshot_source->config.monitor_privacy_screen_snapshot)
        runtime->config.monitor_privacy_screen_snapshot =
            g_variant_ref(snapshot_source->config.monitor_privacy_screen_snapshot);
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
    if (snapshot_source && snapshot_source->config.appearance_snapshot_received) {
        if (snapshot_source->config.appearance_snapshot)
            runtime->config.appearance_snapshot =
                g_variant_ref(snapshot_source->config.appearance_snapshot);
        runtime->config.appearance_snapshot_received = TRUE;
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
    if (snapshot_source && snapshot_source->config.session_lock_snapshot) {
        runtime->config.session_lock_snapshot =
            g_variant_ref(snapshot_source->config.session_lock_snapshot);
        runtime->config.session_lock_revision = snapshot_source->config.session_lock_revision;
    }
    if (snapshot_source && snapshot_source->config.session_lifecycle_snapshot) {
        runtime->config.session_lifecycle_snapshot =
            g_variant_ref(snapshot_source->config.session_lifecycle_snapshot);
        runtime->config.session_lifecycle_revision =
            snapshot_source->config.session_lifecycle_revision;
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
    return runtime;
}

static GVariant* load_defaults_impl(GPtrArray** paths, GPtrArray** directories,
                                    gboolean system_dropins, GPtrArray** ignored, GError** error);

/* Drop-ins that failed are not fatal: report them with the other ignored items. They come first
 * because they were dropped before any setting was checked. */
static void report_skipped_files(LuaRuntime* runtime, GPtrArray** ignored) {
    if (!ignored || !runtime->config.skipped_files)
        return;
    if (!*ignored)
        *ignored = g_ptr_array_new_with_free_func(g_free);
    for (guint i = 0; i < runtime->config.skipped_files->len; i++)
        g_ptr_array_insert(*ignored, (gint)i,
                           g_strdup(g_ptr_array_index(runtime->config.skipped_files, i)));
}

static GVariant* load_runtime_impl(const char* path, GPtrArray** paths, GPtrArray** directories,
                                   gboolean salvage, GPtrArray** ignored, GError** error) {
    if (ignored)
        *ignored = NULL;
    GStatBuf config_stat;
    /* No user config: the built-in init.lua runs first, then the system drop-ins. */
    if (g_lstat(path, &config_stat) != 0 && errno == ENOENT)
        return load_defaults_impl(paths, directories, TRUE, ignored, error);
    /* A dangling symlink or inaccessible path is a load error, not an absent
     * configuration. Let evaluation produce the diagnostic for recovery. */
    LuaRuntime* runtime = lua_runtime_new(path, error);
    if (!runtime)
        return NULL;
    runtime->config.skip_failed_files = salvage;
    EvalRun run = {.config = &runtime->config, .path = path, .system_dropins = TRUE};
    lua_pushlightuserdata(runtime->state, &run);
    lua_pushcclosure(runtime->state, protected_eval, 1);
    if (lua_pcall(runtime->state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(runtime->state));
        lua_pop(runtime->state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
    runtime->config.skip_failed_files = FALSE;
    // The builder returns a floating variant. Own it before exposing another
    // reference to the supervisor; otherwise a later event can unref a stale value.
    runtime->document = run.result ? g_variant_ref_sink(g_steal_pointer(&run.result)) : NULL;
    if (runtime->document && salvage) {
        /* Keep every valid setting; only the broken keys and entries are ignored. */
        g_autoptr(GError) validation_error = NULL;
        if (!gnoblin_config_validate_document(runtime->document, &validation_error)) {
            g_autoptr(GError) salvage_error = NULL;
            g_autoptr(GVariant) repaired =
                gnoblin_config_salvage_document(runtime->document, ignored, &salvage_error);
            if (!repaired) {
                g_propagate_error(error, g_steal_pointer(&salvage_error));
                lua_runtime_free(runtime);
                return NULL;
            }
            g_clear_pointer(&runtime->config.settings_document, g_variant_unref);
            runtime->config.settings_document = g_variant_ref(repaired);
            g_clear_pointer(&runtime->document, g_variant_unref);
            runtime->document = g_variant_ref(repaired);
            lua_getglobal(runtime->state, "gnoblin");
            push_variant(runtime->state, repaired);
            lua_setfield(runtime->state, -2, "config");
            lua_pop(runtime->state, 1);
        }
    }
    if (!runtime->document || !gnoblin_config_validate_document(runtime->document, error)) {
        lua_runtime_free(runtime);
        return NULL;
    }
    report_skipped_files(runtime, ignored);
    lua_runtime_free(pending_runtime);
    pending_runtime = runtime;
    if (paths)
        *paths = g_ptr_array_ref(runtime->config.paths);
    if (directories)
        *directories = g_ptr_array_ref(runtime->config.directories);
    return g_variant_ref(runtime->document);
}

GVariant* gnoblin_config_load_runtime(const char* path, GPtrArray** paths, GPtrArray** directories,
                                      GError** error) {
    return load_runtime_impl(path, paths, directories, FALSE, NULL, error);
}

GVariant* gnoblin_config_load_runtime_reporting(const char* path, GPtrArray** paths,
                                                GPtrArray** directories, GPtrArray** ignored,
                                                GError** error) {
    return load_runtime_impl(path, paths, directories, FALSE, ignored, error);
}

GVariant* gnoblin_config_load_runtime_salvaged(const char* path, GPtrArray** paths,
                                               GPtrArray** directories, GPtrArray** ignored,
                                               GError** error) {
    return load_runtime_impl(path, paths, directories, TRUE, ignored, error);
}

static GVariant* load_defaults_impl(GPtrArray** paths, GPtrArray** directories,
                                    gboolean system_dropins, GPtrArray** ignored, GError** error) {
    LuaRuntime* runtime = lua_runtime_new("/gnoblin-builtin/init.lua", error);
    if (!runtime)
        return NULL;
    runtime->config.embedded_defaults = TRUE;
    g_free(runtime->config.current_path);
    runtime->config.current_path = g_strdup("/gnoblin-builtin/init.lua");
    EvalRun run = {.config = &runtime->config,
                   .path = runtime->config.current_path,
                   .system_dropins = system_dropins};
    lua_pushlightuserdata(runtime->state, &run);
    lua_pushcclosure(runtime->state, protected_eval, 1);
    if (lua_pcall(runtime->state, 0, 0, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", lua_error_text(runtime->state));
        lua_pop(runtime->state, 1);
    } else if (run.error) {
        g_propagate_error(error, g_steal_pointer(&run.error));
    }
    runtime->document = run.result ? g_variant_ref_sink(g_steal_pointer(&run.result)) : NULL;
    if (!runtime->document || !gnoblin_config_validate_document(runtime->document, error)) {
        lua_runtime_free(runtime);
        return NULL;
    }
    report_skipped_files(runtime, ignored);
    lua_runtime_free(pending_runtime);
    pending_runtime = runtime;
    if (paths)
        *paths = g_ptr_array_ref(runtime->config.paths);
    if (directories)
        *directories = g_ptr_array_ref(runtime->config.directories);
    return g_variant_ref(runtime->document);
}

/* The built-in configuration alone. Bootstrap and recovery use it, so no file on disk can
 * keep it from loading. */
GVariant* gnoblin_config_load_runtime_defaults(GPtrArray** paths, GPtrArray** directories,
                                               GError** error) {
    return load_defaults_impl(paths, directories, FALSE, NULL, error);
}

GVariant* gnoblin_config_load_runtime_fallback(const char* path, GVariant* document,
                                               GPtrArray** paths, GPtrArray** directories,
                                               GError** error) {
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error))
        return NULL;

    /* Saved settings cannot restore Lua closures. Keep working embedded input
     * callbacks instead of publishing handler IDs with no callable registry. */
    g_autoptr(GVariant) defaults = gnoblin_config_load_runtime_defaults(NULL, NULL, error);
    if (!defaults)
        return NULL;
    LuaRuntime* runtime = pending_runtime;
    g_autoptr(GVariant) input_handlers = g_variant_lookup_value(defaults, "input-handlers", NULL);
    GVariantDict recovered;
    g_variant_dict_init(&recovered, document);
    if (input_handlers)
        g_variant_dict_insert_value(&recovered, "input-handlers", input_handlers);
    else
        g_variant_dict_remove(&recovered, "input-handlers");
    g_autoptr(GVariant) recovered_document = g_variant_ref_sink(g_variant_dict_end(&recovered));
    if (!gnoblin_config_validate_document(recovered_document, error)) {
        lua_runtime_free(pending_runtime);
        pending_runtime = NULL;
        return NULL;
    }
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    g_autofree char* directory = g_path_get_dirname(canonical);
    add_path(runtime->config.paths, canonical);
    g_ptr_array_add(runtime->config.directories, g_steal_pointer(&directory));
    g_clear_pointer(&runtime->config.settings_document, g_variant_unref);
    runtime->config.settings_document = g_variant_ref(recovered_document);
    g_clear_pointer(&runtime->document, g_variant_unref);
    runtime->document = g_variant_ref(recovered_document);
    lua_getglobal(runtime->state, "gnoblin");
    push_variant(runtime->state, recovered_document);
    lua_setfield(runtime->state, -2, "config");
    lua_pop(runtime->state, 1);

    if (paths)
        *paths = g_ptr_array_ref(runtime->config.paths);
    if (directories)
        *directories = g_ptr_array_ref(runtime->config.directories);
    return g_variant_ref(runtime->document);
}

GVariant* gnoblin_config_load_runtime_recovery_snapshot(const char* path, GVariant* document,
                                                        GPtrArray** paths, GPtrArray** directories,
                                                        GError** error) {
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, error))
        return NULL;
    /* The accepted native snapshot must be reproduced byte-for-value for RESUME.
     * The caller gates events until it transactionally installs built-in defaults. */
    g_autoptr(GVariant) defaults = gnoblin_config_load_runtime_defaults(NULL, NULL, error);
    if (!defaults)
        return NULL;
    LuaRuntime* runtime = pending_runtime;
    g_autofree char* canonical = g_canonicalize_filename(path, NULL);
    g_autofree char* directory = g_path_get_dirname(canonical);
    add_path(runtime->config.paths, canonical);
    g_ptr_array_add(runtime->config.directories, g_steal_pointer(&directory));
    g_clear_pointer(&runtime->config.settings_document, g_variant_unref);
    runtime->config.settings_document = g_variant_ref(document);
    g_clear_pointer(&runtime->document, g_variant_unref);
    runtime->document = g_variant_ref(document);
    lua_getglobal(runtime->state, "gnoblin");
    push_variant(runtime->state, document);
    lua_setfield(runtime->state, -2, "config");
    lua_pop(runtime->state, 1);
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

void fail_dropped_operation(LuaRuntime* runtime, GVariant* operation, const char* reason) {
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

void schedule_deferred_callbacks(LuaConfig* config) {
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
