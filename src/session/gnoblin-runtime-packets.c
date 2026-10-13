#include "gnoblin-runtime-internal.h"

void runtime_fail(Runtime* runtime, const char* message) {
    if (!runtime || runtime->failed)
        return;
    runtime->failed = TRUE;
    g_printerr("gnoblin: runtime supervisor: %s\n", message);
    if (runtime->loop)
        g_main_loop_quit(runtime->loop);
}

static GVariant* config_packet_payload(GVariant* document, guint64 revision, guint64 generation) {
    g_auto(GStrv) runtime_events = gnoblin_config_runtime_events();
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document_version", g_variant_new_uint32(1));
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation", g_variant_new_uint64(generation));
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_variant_builder_add(&builder, "{sv}", "runtime_events",
                          g_variant_new_strv((const char* const*)runtime_events, -1));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

GVariant* resume_packet_payload(GVariant* document, guint64 revision, guint64 generation,
                                guint64 operation_id_watermark, GVariant* saved_events) {
    g_auto(GStrv) runtime_events = saved_events ? NULL : gnoblin_config_runtime_events();
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "document", document);
    g_variant_builder_add(&builder, "{sv}", "settings_revision", g_variant_new_uint64(revision));
    g_variant_builder_add(&builder, "{sv}", "runtime_generation", g_variant_new_uint64(generation));
    g_variant_builder_add(&builder, "{sv}", "operation_id_watermark",
                          g_variant_new_uint64(operation_id_watermark));
    g_variant_builder_add(
        &builder, "{sv}", "runtime_events",
        saved_events ? saved_events : g_variant_new_strv((const char* const*)runtime_events, -1));
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

gboolean is_pending_logout(Runtime* runtime, guint64 request_id) {
    return runtime->pending_logout_ids &&
           g_hash_table_contains(runtime->pending_logout_ids, &request_id);
}

gboolean send_autostart_snapshot(int fd, GVariant* document, GVariant* environment,
                                 GError** error) {
    g_autoptr(GVariant) entries =
        document ? g_variant_lookup_value(document, "autostart", G_VARIANT_TYPE("av")) : NULL;
    if (!entries)
        entries = g_variant_ref_sink(g_variant_new_array(G_VARIANT_TYPE_VARIANT, NULL, 0));
    if (!environment) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Mutter did not provide the session environment");
        return FALSE;
    }
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "entries", entries);
    g_variant_builder_add(&payload, "{sv}", "environment", environment);
    g_autoptr(GVariant) packet_payload = g_variant_ref_sink(g_variant_builder_end(&payload));
    g_autoptr(GnoblinRuntimeWriter) writer = gnoblin_runtime_writer_new();
    if (!gnoblin_runtime_writer_queue(writer, GNOBLIN_RUNTIME_PACKET_HOST_AUTOSTART, 0,
                                      packet_payload, error))
        return FALSE;
    return gnoblin_runtime_writer_flush(writer, fd, error);
}

static void remember_pending_logout(Runtime* runtime, guint64 request_id) {
    guint64* key = g_new(guint64, 1);
    *key = request_id;
    g_hash_table_add(runtime->pending_logout_ids, key);
}

gboolean send_config(Runtime* runtime, GVariant* document, guint64 revision, guint64 generation,
                     guint64 transaction_id, GError** error) {
    g_autoptr(GVariant) normalized = gnoblin_native_input_config_normalize(document);
    if (!normalized) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "could not normalize input configuration for Mutter");
        return FALSE;
    }
    g_autoptr(GVariant) payload = config_packet_payload(normalized, revision, generation);
    return queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_CONFIG, transaction_id, payload, error);
}

void runtime_reload_free(RuntimeReload* reload) {
    if (!reload)
        return;
    if (reload->timeout_id)
        g_source_remove(reload->timeout_id);
    g_free(reload->path);
    g_clear_pointer(&reload->document, g_variant_unref);
    g_clear_pointer(&reload->ignored, g_ptr_array_unref);
    g_free(reload);
}

void runtime_deferred_packet_free(gpointer data) {
    RuntimeDeferredPacket* packet = data;
    if (!packet)
        return;
    g_clear_pointer(&packet->payload, g_variant_unref);
    g_free(packet);
}

gboolean send_api_response(Runtime* runtime, guint64 request_id, GVariant* result,
                           const char* message, GError** error) {
    GVariantBuilder response;
    g_variant_builder_init(&response, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&response, "{sv}", "ok", g_variant_new_boolean(result != NULL));
    if (result)
        g_variant_builder_add(&response, "{sv}", "result", result);
    else
        g_variant_builder_add(
            &response, "{sv}", "error",
            g_variant_new_string(message ? message : "runtime API request failed"));
    g_autoptr(GVariant) packet = g_variant_ref_sink(g_variant_builder_end(&response));
    return queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_API_RESPONSE, request_id, packet, error);
}

void runtime_signal_ready(Runtime* runtime) {
    if (!runtime || runtime->ready_fd < 0)
        return;
    const guint8 ready = 1;
    (void)write(runtime->ready_fd, &ready, sizeof ready);
    close(runtime->ready_fd);
    runtime->ready_fd = -1;
}

void runtime_reload_finish(Runtime* runtime, gboolean commit, const char* message) {
    RuntimeReload* reload = runtime ? runtime->pending_reload : NULL;
    if (!reload)
        return;
    runtime->pending_reload = NULL;
    gnoblin_config_finish_deferred_load(commit);
    if (commit) {
        gboolean recovered_snapshot = reload->startup && runtime->recovery_snapshot;
        runtime->config_fallback = recovered_snapshot || reload->saved_fallback;
        runtime->config_fallback_last_good = reload->saved_fallback;
        if (recovered_snapshot)
            runtime->recovery_snapshot = FALSE;
        set_config_fallback_marker(runtime->config_fallback, reload->saved_fallback,
                                   runtime->startup_failure);
        g_autoptr(GVariant) warnings = gnoblin_config_document_warnings(reload->document);
        if (g_variant_n_children(warnings) > 0) {
            g_autoptr(GString) notice = g_string_new("Configuration loaded with warnings:");
            GVariantIter iter;
            const char* warning;
            g_variant_iter_init(&iter, warnings);
            while (g_variant_iter_next(&iter, "&s", &warning)) {
                /* Leave room for recovery/salvage diagnostics in the notice file. */
                if (notice->len + strlen(warning) + 1 > 4096) {
                    g_string_append(notice,
                                    "\nAdditional warnings are available in the reload result.");
                    break;
                }
                g_string_append_printf(notice, "\n%s", warning);
            }
            append_config_warning_notice(notice->str);
        }
        if (reload->ignored && reload->ignored->len > 0) {
            /* The rest of the configuration is active. Say plainly what was left out, in the
             * session log and in the notice the panel reads. */
            g_autoptr(GString) notice =
                g_string_new("Some settings were ignored because they are invalid:");
            for (guint i = 0; i < reload->ignored->len; i++) {
                const char* line = g_ptr_array_index(reload->ignored, i);
                g_printerr("gnoblin: %s\n", line);
                g_string_append_printf(notice, "\n%s", line);
            }
            append_config_notice(notice->str);
        }
        g_autoptr(GError) save_error = NULL;
        if (!recovered_snapshot &&
            !gnoblin_config_save_last_good_document(reload->path, reload->document, &save_error))
            g_warning("gnoblin: could not save last accepted configuration: %s",
                      save_error ? save_error->message : "unknown error");
        g_autoptr(GError) portal_error = NULL;
        if (!gnoblin_config_sync_portal_selection(reload->document, NULL, &portal_error))
            g_warning("gnoblin: could not update portal selection from Lua config: %s",
                      portal_error ? portal_error->message : "unknown error");
        else if (reload->portal_routes_changed) {
            g_autoptr(GError) restart_error = NULL;
            if (!restart_active_portal_service(&restart_error))
                g_warning("gnoblin: portal routes were saved, but the active portal service "
                          "could not be restarted: %s",
                          restart_error ? restart_error->message : "unknown error");
        }
        g_autoptr(GError) autostart_error = NULL;
        if (runtime->host_control_fd >= 0 && runtime->session_environment &&
            !send_autostart_snapshot(runtime->host_control_fd, reload->document,
                                     runtime->session_environment, &autostart_error))
            g_warning("gnoblin: could not update session autostart: %s",
                      autostart_error ? autostart_error->message : "unknown error");
        if (recovered_snapshot)
            runtime_signal_ready(runtime);
        GVariantBuilder builder;
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&builder, "{sv}", "ok", g_variant_new_boolean(TRUE));
        g_variant_builder_add(&builder, "{sv}", "action", g_variant_new_string("config reload"));
        g_variant_builder_add(&builder, "{sv}", "settings_revision",
                              g_variant_new_uint64(reload->revision));
        g_variant_builder_add(&builder, "{sv}", "runtime_generation",
                              g_variant_new_uint64(reload->generation));
        g_variant_builder_add(&builder, "{sv}", "warnings", warnings);
        g_autoptr(GVariant) result = g_variant_ref_sink(g_variant_builder_end(&builder));
        dispatch_reload_event(runtime, "gnoblin.config.reloaded", reload->path, reload->revision,
                              NULL);
        g_autoptr(GError) error = NULL;
        if (!reload->startup &&
            !send_api_response(runtime, reload->request_id, result, NULL, &error))
            runtime_fail(runtime, error ? error->message : "could not reply to config reload");
    } else {
        if (reload->startup && runtime->recovery_snapshot) {
            runtime_fail(runtime,
                         message ? message : "built-in recovery configuration was rejected");
        } else if (reload->startup && !reload->saved_fallback) {
            g_free(runtime->startup_failure);
            runtime->startup_failure =
                g_strdup(message ? message : "Startup configuration rejected");
            runtime->startup_saved_pending = TRUE;
        } else if (reload->saved_fallback) {
            g_autofree char* combined =
                g_strdup_printf("%s\nSaved configuration also failed: %s",
                                runtime->startup_failure ? runtime->startup_failure
                                                         : "Startup configuration failed",
                                message ? message : "native settings were rejected");
            g_free(runtime->startup_failure);
            runtime->startup_failure = g_steal_pointer(&combined);
        }
        set_reload_failure_notice(runtime, reload->startup ? runtime->startup_failure : message);
        dispatch_reload_event(runtime, "gnoblin.config.reload-failed", reload->path, 0, message);
        g_autoptr(GError) error = NULL;
        if (!reload->startup &&
            !send_api_response(runtime, reload->request_id, NULL, message, &error))
            runtime_fail(runtime, error ? error->message : "could not reply to config reload");
    }
    runtime_reload_free(reload);
    if (runtime->startup_saved_pending && !runtime->startup_config_idle_id)
        runtime->startup_config_idle_id = g_idle_add(runtime_load_startup_config, runtime);
    runtime_schedule_deferred_packets(runtime);
}

gboolean runtime_reload_timeout(gpointer user_data) {
    Runtime* runtime = user_data;
    if (runtime->pending_reload && runtime->pending_reload->sent) {
        runtime->pending_reload->timeout_id = 0;
        runtime_fail(runtime, "Mutter did not acknowledge the configuration transaction");
    } else {
        if (runtime->pending_reload)
            runtime->pending_reload->timeout_id = 0;
        runtime_reload_finish(runtime, FALSE,
                              "timed out waiting for earlier runtime operations to finish");
    }
    return G_SOURCE_REMOVE;
}

static gboolean runtime_reload_progress(gpointer user_data) {
    Runtime* runtime = user_data;
    runtime->reload_progress_id = 0;
    RuntimeReload* reload = runtime->pending_reload;
    if (!reload || runtime->failed)
        return G_SOURCE_REMOVE;
    if (g_hash_table_size(runtime->pending_operation_ids) != 0 ||
        !g_queue_is_empty(&runtime->policy_events) ||
        gnoblin_config_runtime_pending_operations() != 0)
        return G_SOURCE_REMOVE;
    g_autoptr(GVariant) active_document = gnoblin_config_current_document();
    reload->revision = gnoblin_config_settings_revision();
    if (!active_document || !g_variant_equal(active_document, reload->document))
        reload->revision++;
    g_autoptr(GError) error = NULL;
    if (!send_config(runtime, reload->document, reload->revision, reload->generation,
                     reload->request_id, &error)) {
        runtime_reload_finish(runtime, FALSE,
                              error ? error->message : "could not publish candidate configuration");
        return G_SOURCE_REMOVE;
    }
    reload->sent = TRUE;
    return G_SOURCE_REMOVE;
}

void runtime_schedule_reload_progress(Runtime* runtime) {
    if (runtime && runtime->pending_reload && !runtime->reload_progress_id)
        runtime->reload_progress_id =
            g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, runtime_reload_progress, runtime, NULL);
}

static gboolean runtime_write_ready(gint fd, GIOCondition condition, gpointer user_data) {
    Runtime* runtime = user_data;
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        runtime->write_watch_id = 0;
        runtime_fail(runtime, "compositor closed the private runtime channel");
        return G_SOURCE_REMOVE;
    }
    g_autoptr(GError) error = NULL;
    if (!gnoblin_runtime_writer_flush(runtime->writer, fd, &error)) {
        if (error && g_error_matches(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK))
            return G_SOURCE_CONTINUE;
        runtime->write_watch_id = 0;
        runtime_fail(runtime, error ? error->message : "could not write compositor packet");
        return G_SOURCE_REMOVE;
    }
    runtime->write_watch_id = 0;
    return G_SOURCE_REMOVE;
}

gboolean queue_packet(Runtime* runtime, GnoblinRuntimePacketType type, guint64 request_id,
                      GVariant* payload, GError** error) {
    if (!gnoblin_runtime_writer_queue(runtime->writer, type, request_id, payload, error))
        return FALSE;
    g_autoptr(GError) flush_error = NULL;
    if (!gnoblin_runtime_writer_flush(runtime->writer, runtime->channel_fd, &flush_error)) {
        if (g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            if (!runtime->write_watch_id)
                runtime->write_watch_id =
                    g_unix_fd_add(runtime->channel_fd, G_IO_OUT | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                                  runtime_write_ready, runtime);
            return TRUE;
        }
        g_propagate_error(error, g_steal_pointer(&flush_error));
        return FALSE;
    }
    return TRUE;
}

/* The guardian treats STARTED as proof that Mutter has received the initial
 * CONFIG/RESUME packet. queue_packet() may return with bytes still queued when
 * the socket is full, so finish that first packet before publishing the
 * status. Otherwise a supervisor killed in that gap could make the guardian
 * restart without suspending Mutter. */
gboolean flush_startup_packet(Runtime* runtime, GError** error) {
    gint64 deadline = g_get_monotonic_time() + STARTUP_TIMEOUT_MS * 1000;
    for (;;) {
        g_autoptr(GError) flush_error = NULL;
        if (gnoblin_runtime_writer_flush(runtime->writer, runtime->channel_fd, &flush_error))
            break;
        if (!g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            g_propagate_error(error, g_steal_pointer(&flush_error));
            return FALSE;
        }

        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "Mutter did not accept the initial runtime packet");
            return FALSE;
        }
        struct pollfd pfd = {.fd = runtime->channel_fd, .events = POLLOUT};
        int timeout = (int)MIN((remaining + 999) / 1000, 100);
        int result;
        do {
            result = poll(&pfd, 1, timeout);
        } while (result < 0 && errno == EINTR);
        if (result < 0 || (result > 0 && (pfd.revents & (POLLERR | POLLHUP | POLLNVAL)))) {
            g_set_error(error, G_IO_ERROR,
                        result < 0 ? g_io_error_from_errno(errno) : G_IO_ERROR_CLOSED,
                        "Mutter closed the runtime channel during startup: %s",
                        result < 0 ? g_strerror(errno) : "peer closed the channel");
            return FALSE;
        }
    }
    if (runtime->write_watch_id) {
        g_source_remove(runtime->write_watch_id);
        runtime->write_watch_id = 0;
    }
    return TRUE;
}

gboolean send_pending_operations(Runtime* runtime, GError** error) {
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    if (!operations || !g_variant_is_of_type(operations, G_VARIANT_TYPE("aa{sv}"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Lua runtime returned an invalid operation batch");
        return FALSE;
    }
    GVariantIter iter;
    GVariant* operation;
    g_variant_iter_init(&iter, operations);
    while ((operation = g_variant_iter_next_value(&iter))) {
        g_autoptr(GVariant) owned_operation = operation;
        g_autoptr(GVariant) input_scoped_operation = NULL;
        if (runtime->active_input_request_id) {
            GVariantBuilder scoped;
            GVariantIter operation_fields;
            const char* operation_field_name;
            GVariant* operation_field_value;
            g_variant_builder_init(&scoped, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&operation_fields, operation);
            while (g_variant_iter_next(&operation_fields, "{&sv}", &operation_field_name,
                                       &operation_field_value)) {
                g_autoptr(GVariant) value = operation_field_value;
                g_variant_builder_add(&scoped, "{sv}", operation_field_name, value);
            }
            g_variant_builder_add(&scoped, "{sv}", "input_request_id",
                                  g_variant_new_uint64(runtime->active_input_request_id));
            input_scoped_operation = g_variant_ref_sink(g_variant_builder_end(&scoped));
            operation = input_scoped_operation;
        }
        gint64 request_id = 0;
        const char* method = NULL;
        g_autoptr(GVariant) arguments =
            g_variant_lookup_value(operation, "arguments", G_VARIANT_TYPE_VARDICT);
        if (!g_variant_lookup(operation, "request_id", "x", &request_id) || request_id <= 0 ||
            !g_variant_lookup(operation, "method", "&s", &method) || !method || !*method ||
            !arguments) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime emitted a malformed operation");
            return FALSE;
        }
        gboolean needs_focus_context =
            g_str_equal(method, "window.focus") || g_str_equal(method, "window.begin_move") ||
            g_str_equal(method, "window.begin_resize") ||
            g_str_equal(method, "input.text_target") || g_str_equal(method, "window.snap_context");
        g_autoptr(GVariant) packet = NULL;
        if (needs_focus_context) {
            guint64 handle = 0;
            guint64 runtime_generation = 0;
            guint64 native_generation = 0;
            gint64 expires_at_us = 0;
            if (!gnoblin_config_take_focus_context(request_id, &handle, &runtime_generation,
                                                   &native_generation, &expires_at_us) ||
                !handle || !native_generation || expires_at_us <= g_get_monotonic_time()) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "Lua focus operation lost its one-use native context");
                return FALSE;
            }
            if (runtime_generation != gnoblin_config_runtime_generation()) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                    "Lua focus operation belongs to an old runtime generation");
                return FALSE;
            }
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_handle",
                                  g_variant_new_uint64(handle));
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_generation",
                                  g_variant_new_uint64(native_generation));
            g_variant_builder_add(&operation_builder, "{sv}", "focus_context_expires_at_us",
                                  g_variant_new_int64(expires_at_us));
            if (g_str_equal(method, "window.snap_context"))
                g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                      g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.thumbnail") && runtime->current_client_id) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "client_id",
                                  g_variant_new_uint64(runtime->current_client_id));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.snap.offer") || g_str_equal(method, "command.run") ||
                   g_str_equal(method, "command.capture")) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                  g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else if (g_str_equal(method, "window.snap")) {
            GVariantBuilder operation_builder;
            GVariantIter fields;
            const char* field_name;
            GVariant* field_value;
            g_variant_builder_init(&operation_builder, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&fields, operation);
            while (g_variant_iter_next(&fields, "{&sv}", &field_name, &field_value)) {
                g_autoptr(GVariant) value = field_value;
                g_variant_builder_add(&operation_builder, "{sv}", field_name, value);
            }
            g_variant_builder_add(&operation_builder, "{sv}", "runtime_generation",
                                  g_variant_new_uint64(gnoblin_config_runtime_generation()));
            packet = g_variant_ref_sink(g_variant_builder_end(&operation_builder));
        } else {
            packet = g_variant_ref(operation);
        }
        gboolean logout_operation = g_str_equal(method, "session.logout");
        if (logout_operation && is_pending_logout(runtime, (guint64)request_id)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime reused a pending session logout request ID");
            return FALSE;
        }
        if (g_hash_table_contains(runtime->pending_operation_ids,
                                  &(guint64){(guint64)request_id})) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                "Lua runtime reused a pending operation request ID");
            return FALSE;
        }
        if (logout_operation)
            remember_pending_logout(runtime, (guint64)request_id);
        if (!queue_packet(runtime, GNOBLIN_RUNTIME_PACKET_OPERATION, (guint64)request_id, packet,
                          error)) {
            if (logout_operation)
                g_hash_table_remove(runtime->pending_logout_ids, &(guint64){(guint64)request_id});
            return FALSE;
        }
        guint64* operation_key = g_new(guint64, 1);
        *operation_key = (guint64)request_id;
        g_hash_table_add(runtime->pending_operation_ids, operation_key);
    }
    return TRUE;
}
