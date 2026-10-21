#include "gnoblin-runtime-internal.h"

static gboolean dispatch_parent_event(Runtime* runtime, const char* event, GVariant* payload,
                                      GError** error);

static gboolean commit_event_result(Runtime* runtime, GVariant* document, GError** error) {
    if (!document)
        return TRUE;
    gnoblin_config_finish_event(TRUE);
    g_autoptr(GVariant) committed = gnoblin_config_current_document();
    if (!committed) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "Lua event did not produce a committed configuration");
        return FALSE;
    }
    if (!send_config(runtime, committed, gnoblin_config_settings_revision(),
                     gnoblin_config_runtime_generation(), 0, error))
        return FALSE;
    return TRUE;
}

gboolean handle_event(Runtime* runtime, GVariant* payload, GError** error) {
    /* RESUME must reproduce Mutter's accepted snapshot before replacing it with
     * built-ins. Do not invoke default closures for old handler identities. */
    if (runtime->recovery_snapshot)
        return TRUE;
    const char* event = NULL;
    g_autoptr(GVariant) event_payload =
        g_variant_lookup_value(payload, "payload", G_VARIANT_TYPE_VARDICT);
    if (!g_variant_lookup(payload, "event", "&s", &event) || !event || !*event || !event_payload) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid runtime event");
        return FALSE;
    }

    g_autoptr(GVariant) document = NULL;
    gboolean trusted_binding = g_str_equal(event, "gnoblin.shortcut.binding-activated");
    gboolean trusted_shortcut = g_str_equal(event, "gnoblin.shortcut.activated") ||
                                g_str_equal(event, "gnoblin.pointer.binding-activated");
    gboolean session_key = g_str_equal(event, "gnoblin.shortcut.session.key");
    if (trusted_binding || trusted_shortcut || session_key) {
        gboolean first = FALSE;
        if (trusted_binding && (!g_variant_lookup(event_payload, "first", "b", &first) || !first)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "only a first binding activation may carry focus context");
            return FALSE;
        }
        guint64 handle = 0;
        guint64 native_generation = 0;
        gint64 expires_at_us = 0;
        gboolean has_context =
            g_variant_lookup(payload, "focus_context_handle", "t", &handle) && handle &&
            g_variant_lookup(payload, "focus_context_generation", "t", &native_generation) &&
            native_generation &&
            g_variant_lookup(payload, "focus_context_expires_at_us", "x", &expires_at_us) &&
            expires_at_us > g_get_monotonic_time();
        g_autoptr(GVariant) context_handle =
            g_variant_lookup_value(payload, "focus_context_handle", NULL);
        g_autoptr(GVariant) context_generation =
            g_variant_lookup_value(payload, "focus_context_generation", NULL);
        g_autoptr(GVariant) context_expiry =
            g_variant_lookup_value(payload, "focus_context_expires_at_us", NULL);
        gboolean has_any_context = context_handle || context_generation || context_expiry;
        if (has_context) {
            document = gnoblin_config_dispatch_shortcut_event(
                event, event_payload, handle, native_generation, expires_at_us, error);
        } else if ((trusted_binding || session_key) && !has_any_context) {
            /* These events still reach Lua if optional focus authority is unavailable. */
            document = gnoblin_config_dispatch_event(event, event_payload, error);
        } else {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "shortcut event has no live native focus context");
        }
    } else {
        document = gnoblin_config_dispatch_event(event, event_payload, error);
    }
    g_autofree char* callback_error = gnoblin_config_take_runtime_callback_error();
    if (callback_error) {
        g_warning("gnoblin: %s", callback_error);
        set_reload_failure_notice(runtime, callback_error);
    }
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    return send_pending_operations(runtime, error);
}

static gboolean send_input_default_decision(Runtime* runtime, guint64 request_id,
                                            guint64 config_generation, const char* reason,
                                            GError** error) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "action", g_variant_new_string("forward"));
    g_variant_builder_add(&builder, "{sv}", "config_generation",
                          g_variant_new_uint64(config_generation));
    if (reason)
        g_variant_builder_add(&builder, "{sv}", "error", g_variant_new_string(reason));
    g_autoptr(GVariant) decision = g_variant_ref_sink(g_variant_builder_end(&builder));
    return queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_INPUT_DECISION, request_id, decision,
                        error);
}

gboolean handle_input(Runtime* runtime, guint64 request_id, GVariant* payload, GError** error) {
    guint64 config_generation = 0;
    g_autoptr(GVariant) handler_ids =
        g_variant_lookup_value(payload, "handler_ids", G_VARIANT_TYPE("as"));
    g_autoptr(GVariant) event = g_variant_lookup_value(payload, "event", G_VARIANT_TYPE_VARDICT);
    if (!request_id || !handler_ids || !event ||
        !g_variant_lookup(payload, "config_generation", "t", &config_generation) ||
        !config_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid input request");
        return FALSE;
    }

    /* Input authority is ephemeral. Do not defer it across a startup or
     * reload boundary: Mutter can immediately use this default decision and
     * discard any later reply for the retired request ID. */
    /* The compositor stamps each request with the settings revision it holds. The runtime
     * generation advances on every reload, including one that changes nothing, so comparing
     * against it rejected every later request and silently disabled Lua key handlers. */
    if (!runtime->ready || runtime->recovery_snapshot || runtime->pending_reload ||
        config_generation != gnoblin_config_settings_revision())
        return send_input_default_decision(runtime, request_id, config_generation,
                                           "input handlers are unavailable", error);

    g_autoptr(GError) dispatch_error = NULL;
    g_autoptr(GVariant) decision = gnoblin_config_dispatch_input(payload, &dispatch_error);
    g_autofree char* callback_error = gnoblin_config_take_runtime_callback_error();
    if (callback_error) {
        g_warning("gnoblin: %s", callback_error);
        set_reload_failure_notice(runtime, callback_error);
    }
    if (!decision) {
        g_warning("gnoblin: Lua input handler failed: %s",
                  dispatch_error ? dispatch_error->message : "invalid decision");
        return send_input_default_decision(
            runtime, request_id, config_generation,
            dispatch_error ? dispatch_error->message : "invalid input decision", error);
    }
    if (!g_variant_is_of_type(decision, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_is_normal_form(decision)) {
        g_warning("gnoblin: Lua input handler returned an invalid decision");
        return send_input_default_decision(runtime, request_id, config_generation,
                                           "invalid input decision", error);
    }

    GVariantBuilder response_builder;
    GVariantIter fields;
    const char* field_name;
    GVariant* field_value;
    g_variant_builder_init(&response_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&fields, decision);
    while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
        g_autoptr(GVariant) value = field_value;
        g_variant_builder_add(&response_builder, "{sv}", field_name, value);
    }
    g_variant_builder_add(&response_builder, "{sv}", "config_generation",
                          g_variant_new_uint64(config_generation));
    g_autoptr(GVariant) response = g_variant_ref_sink(g_variant_builder_end(&response_builder));
    if (!queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_INPUT_DECISION, request_id, response, error))
        return FALSE;
    runtime->active_input_request_id = request_id;
    gboolean sent = send_pending_operations(runtime, error);
    runtime->active_input_request_id = 0;
    return sent;
}

gboolean handle_completion(Runtime* runtime, guint64 packet_id, GVariant* payload, GError** error) {
    if (runtime->recovery_snapshot)
        return TRUE;
    gint64 operation_id = 0;
    if (!g_variant_lookup(payload, "operation_id", "x", &operation_id) || operation_id <= 0 ||
        (guint64)operation_id != packet_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor completion request ID does not match its frame");
        return FALSE;
    }

    gboolean logout_operation = is_pending_logout(runtime, packet_id);
    const char* method = NULL;
    gboolean succeeded = FALSE;
    gboolean logout_accepted = FALSE;
    g_autoptr(GVariant) operation_result =
        g_variant_lookup_value(payload, "value", G_VARIANT_TYPE_VARDICT);
    if (operation_result)
        g_variant_lookup(operation_result, "accepted", "b", &logout_accepted);
    gboolean matching_success = logout_operation &&
                                g_variant_lookup(payload, "method", "&s", &method) &&
                                g_str_equal(method, "session.logout") &&
                                g_variant_lookup(payload, "ok", "b", &succeeded) && succeeded;
    matching_success = matching_success && operation_result && logout_accepted;
    int power_exit_status = 0;
    gboolean restore_requested = FALSE;
    if (g_variant_lookup(payload, "method", "&s", &method) &&
        g_variant_lookup(payload, "ok", "b", &succeeded) && succeeded && operation_result &&
        g_variant_lookup(operation_result, "accepted", "b", &logout_accepted) && logout_accepted) {
        if (g_str_equal(method, "session.suspend"))
            power_exit_status = EXIT_POWER_SUSPEND;
        else if (g_str_equal(method, "session.hibernate"))
            power_exit_status = EXIT_POWER_HIBERNATE;
        else if (g_str_equal(method, "session.reboot"))
            power_exit_status = EXIT_POWER_REBOOT;
        else if (g_str_equal(method, "session.poweroff"))
            power_exit_status = EXIT_POWER_POWEROFF;
        else if (g_str_equal(method, "config.restore_default"))
            restore_requested = TRUE;
    }

    g_autoptr(GVariant) document =
        gnoblin_config_dispatch_event("gnoblin.operation.completed", payload, error);
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    if (!send_pending_operations(runtime, error))
        return FALSE;

    g_hash_table_remove(runtime->pending_operation_ids, &packet_id);
    runtime_schedule_reload_progress(runtime);

    if (logout_operation) {
        g_hash_table_remove(runtime->pending_logout_ids, &packet_id);
        if (matching_success) {
            runtime->logout_requested = TRUE;
            runtime->exit_status = EXIT_SUCCESS;
            g_main_loop_quit(runtime->loop);
        }
    }
    if (power_exit_status) {
        runtime->exit_status = power_exit_status;
        g_main_loop_quit(runtime->loop);
    }
    if (restore_requested) {
        runtime->exit_status = EXIT_CONFIG_RESTORE;
        g_main_loop_quit(runtime->loop);
    }
    return TRUE;
}

static gboolean dispatch_parent_event(Runtime* runtime, const char* event, GVariant* payload,
                                      GError** error) {
    if (runtime->recovery_snapshot)
        return TRUE;
    g_autoptr(GVariant) document = gnoblin_config_dispatch_event(event, payload, error);
    if (error && *error) {
        gnoblin_config_finish_event(FALSE);
        return FALSE;
    }
    if (!commit_event_result(runtime, document, error))
        return FALSE;
    return send_pending_operations(runtime, error);
}

static gboolean dispatch_deferred_callbacks(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->deferred_callback_idle_id = 0;
    if (runtime->failed)
        return G_SOURCE_REMOVE;

    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) document = gnoblin_config_dispatch_deferred_callbacks(&error);
    if (error) {
        gnoblin_config_finish_event(FALSE);
        g_warning("gnoblin: deferred Lua operation callback failed: %s", error->message);
        g_clear_error(&error);
    } else if (!commit_event_result(runtime, document, &error)) {
        runtime_fail(runtime,
                     error ? error->message : "could not commit deferred Lua operation callback");
        return G_SOURCE_REMOVE;
    }
    if (!send_pending_operations(runtime, &error)) {
        runtime_fail(runtime,
                     error ? error->message : "could not send deferred Lua callback operations");
        return G_SOURCE_REMOVE;
    }
    runtime_schedule_reload_progress(runtime);
    return G_SOURCE_REMOVE;
}

void runtime_schedule_deferred_callbacks(gpointer user_data) {
    Runtime* runtime = user_data;
    if (!runtime || runtime->failed || runtime->deferred_callback_idle_id)
        return;
    runtime->deferred_callback_idle_id =
        g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_deferred_callbacks, runtime, NULL);
}

static GVariant* policy_event_payload(Runtime* runtime, GVariant* policy, guint64 revision) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "policy", policy);
    g_variant_builder_add(&builder, "{sv}", "revision", g_variant_new_int64((gint64)revision));
    g_variant_builder_add(&builder, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++runtime->event_sequence));
    g_variant_builder_add(&builder, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

void runtime_policy_event_free(gpointer data) {
    RuntimePolicyEvent* event = data;
    if (!event)
        return;
    g_clear_pointer(&event->payload, g_variant_unref);
    g_free(event);
}

static gboolean dispatch_commit_event(Runtime* runtime, const char* event, GVariant* payload) {
    g_autoptr(GError) error = NULL;
    if (!dispatch_parent_event(runtime, event, payload, &error)) {
        g_warning("gnoblin: could not dispatch %s: %s", event,
                  error ? error->message : "unknown runtime error");
        return FALSE;
    }
    return TRUE;
}

static gboolean dispatch_policy_events(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->policy_event_idle_id = 0;
    while (!g_queue_is_empty(&runtime->policy_events)) {
        RuntimePolicyEvent* event = g_queue_pop_head(&runtime->policy_events);
        dispatch_commit_event(runtime, event->event, event->payload);
        runtime_policy_event_free(event);
    }
    runtime_schedule_reload_progress(runtime);
    return G_SOURCE_REMOVE;
}

static void queue_policy_event(Runtime* runtime, const char* event, GVariant* policy,
                               guint64 revision) {
    if (!runtime || runtime->failed || !policy)
        return;
    RuntimePolicyEvent* queued = g_new0(RuntimePolicyEvent, 1);
    queued->event = event;
    queued->payload = policy_event_payload(runtime, policy, revision);
    g_queue_push_tail(&runtime->policy_events, queued);
    if (!runtime->policy_event_idle_id)
        runtime->policy_event_idle_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, dispatch_policy_events, runtime, NULL);
}

void focus_policy_committed(GVariant* policy, guint64 revision, gpointer user_data) {
    queue_policy_event(user_data, "gnoblin.focus.policy-changed", policy, revision);
}

void dispatch_reload_event(Runtime* runtime, const char* event, const char* path, guint64 revision,
                           const char* failure_message) {
    GVariantBuilder payload_builder;
    g_variant_builder_init(&payload_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload_builder, "{sv}", "path", g_variant_new_string(path));
    g_variant_builder_add(&payload_builder, "{sv}", "sequence",
                          g_variant_new_int64((gint64)++runtime->event_sequence));
    g_variant_builder_add(&payload_builder, "{sv}", "time",
                          g_variant_new_int64(g_get_monotonic_time()));
    if (g_str_equal(event, "gnoblin.config.reloaded")) {
        g_variant_builder_add(&payload_builder, "{sv}", "revision", g_variant_new_uint64(revision));
        g_autoptr(GVariant) document = gnoblin_config_current_document();
        g_autoptr(GVariant) warnings = gnoblin_config_document_warnings(document);
        g_variant_builder_add(&payload_builder, "{sv}", "warnings", warnings);
    } else
        g_variant_builder_add(&payload_builder, "{sv}", "error",
                              g_variant_new_string(failure_message
                                                       ? failure_message
                                                       : "configuration reload failed"));
    g_autoptr(GVariant) payload = g_variant_ref_sink(g_variant_builder_end(&payload_builder));
    dispatch_commit_event(runtime, event, payload);
}

gboolean handle_state(Runtime* runtime, GVariant* payload, GError** error) {
    guint32 version = 0;
    guint64 revision = 0;
    gboolean available = FALSE;
    const char* name = NULL;
    g_autoptr(GVariant) snapshot = NULL;
    if (!g_variant_lookup(payload, "state_version", "u", &version) || version != 1 ||
        !g_variant_lookup(payload, "revision", "t", &revision) ||
        !g_variant_lookup(payload, "available", "b", &available) ||
        !g_variant_lookup(payload, "name", "&s", &name) || !name || !*name) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid state snapshot");
        return FALSE;
    }
    if (available) {
        snapshot = g_variant_lookup_value(payload, "snapshot", G_VARIANT_TYPE_VARDICT);
        if (!snapshot) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "compositor state snapshot is missing a valid a{sv} value");
            return FALSE;
        }
    } else {
        g_autoptr(GVariant) unexpected_snapshot = g_variant_lookup_value(payload, "snapshot", NULL);
        if (unexpected_snapshot) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "unavailable compositor state snapshot must not include a value");
            return FALSE;
        }
    }
    guint64* previous = g_hash_table_lookup(runtime->state_revisions, name);
    if (previous && revision < *previous) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                    "compositor state snapshot '%s' moved backwards in revision", name);
        return FALSE;
    }
    void (*update)(GVariant*, guint64) = NULL;
    if (g_str_equal(name, "windows"))
        update = gnoblin_config_update_window_snapshot;
    else if (g_str_equal(name, "workspaces"))
        update = gnoblin_config_update_workspace_snapshot;
    else if (g_str_equal(name, "monitors"))
        update = gnoblin_config_update_monitor_snapshot;
    else if (g_str_equal(name, "outputs"))
        update = gnoblin_config_update_output_snapshot;
    else if (g_str_equal(name, "monitor-privacy-screen"))
        update = gnoblin_config_update_monitor_privacy_screen_snapshot;
    else if (g_str_equal(name, "layers"))
        update = gnoblin_config_update_layer_snapshot;
    else if (g_str_equal(name, "capabilities"))
        update = gnoblin_config_update_capability_snapshot;
    else if (g_str_equal(name, "input-devices"))
        update = gnoblin_config_update_input_device_snapshot;
    else if (g_str_equal(name, "input-sources"))
        update = gnoblin_config_update_input_source_snapshot;
    else if (g_str_equal(name, "input-orientation-lock"))
        update = gnoblin_config_update_orientation_lock_snapshot;
    else if (g_str_equal(name, "appearance"))
        update = gnoblin_config_update_appearance_snapshot;
    else if (g_str_equal(name, "shortcuts"))
        update = gnoblin_config_update_shortcut_snapshot;
    else if (g_str_equal(name, "launches"))
        update = gnoblin_config_update_launch_snapshot;
    else if (g_str_equal(name, "privacy"))
        update = gnoblin_config_update_privacy_snapshot;
    else if (g_str_equal(name, "session-activity"))
        update = gnoblin_config_update_session_activity_snapshot;
    else if (g_str_equal(name, "session-lock"))
        update = gnoblin_config_update_session_lock_snapshot;
    else if (g_str_equal(name, "session-lifecycle"))
        update = gnoblin_config_update_session_lifecycle_snapshot;
    else if (g_str_equal(name, "animations"))
        update = gnoblin_config_update_animation_snapshot;
    if (!update) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "compositor state snapshot '%s' is unsupported", name);
        return FALSE;
    }
    update(snapshot, revision);
    guint64* saved_revision = g_new(guint64, 1);
    *saved_revision = revision;
    g_hash_table_replace(runtime->state_revisions, g_strdup(name), saved_revision);
    return TRUE;
}

gboolean handle_api_request(Runtime* runtime, guint64 request_id, GVariant* payload,
                            GError** error) {
    gboolean startup_request = request_id == G_MAXUINT64 &&
                               (runtime->startup_config_pending || runtime->startup_saved_pending);
    const char* kind = NULL;
    const char* method = NULL;
    g_autoptr(GVariant) arguments =
        g_variant_lookup_value(payload, "arguments", G_VARIANT_TYPE_VARDICT);
    guint64 client_id = 0;
    if (request_id == 0 || !g_variant_lookup(payload, "kind", "&s", &kind) ||
        !g_variant_lookup(payload, "method", "&s", &method) || !*method || !arguments ||
        (!g_str_equal(kind, "read") && !g_str_equal(kind, "call") && !g_str_equal(kind, "reload") &&
         !g_str_equal(kind, "console"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "compositor sent an invalid runtime API request");
        return FALSE;
    }
    if (runtime->recovery_snapshot && !startup_request)
        return send_api_response(runtime, request_id, NULL,
                                 "configuration recovery is applying built-in defaults", error);
    if (g_str_equal(kind, "call"))
        g_variant_lookup(payload, "client_id", "t", &client_id);
    runtime->current_client_id = client_id;

    g_autoptr(GError) api_error = NULL;
    g_autoptr(GVariant) result = NULL;
    if (g_str_equal(kind, "read")) {
        result = gnoblin_config_read_api(method, arguments, &api_error);
    } else if (g_str_equal(kind, "call")) {
        result = gnoblin_config_call_api(method, arguments, &api_error);
    } else if (g_str_equal(kind, "console")) {
        const char* source = NULL;
        if ((!g_str_equal(method, "console.eval") && !g_str_equal(method, "console.complete")) ||
            g_variant_n_children(arguments) != 1 ||
            !g_variant_lookup(arguments, "code", "&s", &source)) {
            g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "invalid Lua console request");
        } else if (g_str_equal(method, "console.complete")) {
            result = gnoblin_config_complete_console(source, &api_error);
        } else {
            result = gnoblin_config_eval_console(source, &api_error);
            if (result && gnoblin_config_has_pending_document() &&
                !commit_event_result(runtime, result, &api_error))
                g_clear_pointer(&result, g_variant_unref);
        }
    } else if (!g_str_equal(method, "runtime.reload_config") ||
               g_variant_n_children(arguments) != 0) {
        g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid runtime config reload request");
    } else {
        if (runtime->pending_reload) {
            g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                "a configuration reload is already in progress");
            set_reload_failure_notice(runtime, api_error->message);
        } else {
            g_autofree char* path = gnoblin_config_path();
            g_autoptr(GVariant) candidate = NULL;
            g_autoptr(GPtrArray) ignored_settings = NULL;
            if (startup_request && runtime->recovery_snapshot) {
                candidate = gnoblin_config_load_runtime_defaults(NULL, NULL, &api_error);
            } else if (startup_request && runtime->startup_saved_pending) {
                g_autoptr(GVariant) saved =
                    gnoblin_config_load_last_good_document(path, &api_error);
                if (saved)
                    candidate =
                        gnoblin_config_load_runtime_fallback(path, saved, NULL, NULL, &api_error);
            } else if (startup_request) {
                /* At login keep every valid setting and ignore only the broken ones. A reload stays
                 * strict for now: a failed reload keeps the active config and reports the error. */
                candidate = gnoblin_config_load_runtime_salvaged(path, NULL, NULL,
                                                                 &ignored_settings, &api_error);
            } else {
                candidate = gnoblin_config_load_runtime(path, NULL, NULL, &api_error);
            }
            if (candidate) {
                if (!gnoblin_config_defer_load_commit()) {
                    gnoblin_config_finish_load(FALSE);
                    g_set_error_literal(&api_error, G_IO_ERROR, G_IO_ERROR_BUSY,
                                        "could not stage configuration reload");
                    dispatch_reload_event(runtime, "gnoblin.config.reload-failed", path, 0,
                                          api_error->message);
                    set_reload_failure_notice(runtime, api_error->message);
                } else {
                    RuntimeReload* reload = g_new0(RuntimeReload, 1);
                    reload->request_id = request_id;
                    reload->startup = startup_request;
                    reload->saved_fallback = startup_request && runtime->startup_saved_pending;
                    reload->ignored = g_steal_pointer(&ignored_settings);
                    reload->generation = gnoblin_config_deferred_runtime_generation();
                    reload->path = g_strdup(path);
                    reload->document = g_variant_ref(candidate);
                    g_autoptr(GVariant) current_document = gnoblin_config_current_document();
                    g_autoptr(GVariant) previous_portals =
                        current_document ? g_variant_lookup_value(current_document, "portals", NULL)
                                         : NULL;
                    g_autoptr(GVariant) candidate_portals =
                        g_variant_lookup_value(candidate, "portals", NULL);
                    reload->portal_routes_changed =
                        (!previous_portals != !candidate_portals) ||
                        (previous_portals && candidate_portals &&
                         !g_variant_equal(previous_portals, candidate_portals));
                    reload->timeout_id = g_timeout_add_seconds(15, runtime_reload_timeout, runtime);
                    runtime->pending_reload = reload;
                    runtime_schedule_reload_progress(runtime);
                    runtime->current_client_id = 0;
                    return TRUE;
                }
            } else {
                gnoblin_config_finish_load(FALSE);
                dispatch_reload_event(runtime, "gnoblin.config.reload-failed", path, 0,
                                      api_error ? api_error->message : NULL);
                if (startup_request && runtime->startup_config_pending) {
                    g_free(runtime->startup_failure);
                    runtime->startup_failure =
                        g_strdup(api_error ? api_error->message : "Startup config failed");
                }
                set_reload_failure_notice(runtime, startup_request && runtime->startup_saved_pending
                                                       ? runtime->startup_failure
                                                   : api_error ? api_error->message
                                                               : NULL);
            }
        }
    }
    if (startup_request && api_error) {
        if (runtime->recovery_snapshot) {
            runtime_fail(runtime, api_error->message);
        } else if (runtime->startup_saved_pending) {
            if (!g_error_matches(api_error, G_FILE_ERROR, G_FILE_ERROR_NOENT)) {
                g_autofree char* combined =
                    g_strdup_printf("%s\nSaved configuration also failed: %s",
                                    runtime->startup_failure ? runtime->startup_failure
                                                             : "Startup configuration failed",
                                    api_error->message);
                g_free(runtime->startup_failure);
                runtime->startup_failure = g_steal_pointer(&combined);
            }
        } else {
            g_free(runtime->startup_failure);
            runtime->startup_failure = g_strdup(api_error->message);
        }
        set_reload_failure_notice(runtime, runtime->startup_failure);
    }
    if (!startup_request && !send_api_response(runtime, request_id, result,
                                               api_error ? api_error->message : NULL, error)) {
        runtime->current_client_id = 0;
        return FALSE;
    }
    gboolean sent = (!g_str_equal(kind, "call") && !g_str_equal(kind, "console")) ||
                    send_pending_operations(runtime, error);
    runtime->current_client_id = 0;
    return sent;
}
