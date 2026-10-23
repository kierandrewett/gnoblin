#include "gnoblin-native-control-private.h"

static void read_request(Client* client);

void workspace_manager_index_changed(MetaWorkspaceManager* manager, int index, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    gnoblin_control_refresh_all_workspace_signal_watches(control);
    workspace_manager_changed(manager, control);
}

void process_buffer(Client* client) {
    if (client->closing || client->writing)
        return;
    const char* newline = memchr(client->request->str, '\n', client->request->len);
    if (newline) {
        gsize line_length = newline - client->request->str;
        if (line_length > MAX_REQUEST_BYTES) {
            client->close_after_response = TRUE;
            send_response(client, encode_response("", NULL, "request is too large"));
            return;
        }
        char* response = handle_request(client, client->request->str, line_length);
        g_string_erase(client->request, 0, line_length + 1);
        if (!response)
            return;
        send_response(client, response);
        return;
    }
    if (client->request->len > MAX_REQUEST_BYTES) {
        client->close_after_response = TRUE;
        send_response(client, encode_response("", NULL, "request is too large"));
        return;
    }
    if (!client->reading)
        read_request(client);
}

static void read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    Client* client = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes =
        g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, &error);
    client->reading = FALSE;
    if (client->closing || !client->control || !bytes || g_bytes_get_size(bytes) == 0) {
        client_close(client);
        return;
    }
    gsize length;
    const char* data = g_bytes_get_data(bytes, &length);
    g_string_append_len(client->request, data, length);
    process_buffer(client);
}

static void read_request(Client* client) {
    client->reading = TRUE;
    g_input_stream_read_bytes_async(g_io_stream_get_input_stream(G_IO_STREAM(client->connection)),
                                    4096, G_PRIORITY_DEFAULT, NULL, read_done, client);
}

gboolean client_connected(GSocketService* service, GSocketConnection* connection,
                          GObject* source_object, gpointer user_data) {
    static const char* methods[] = {
        "capabilities.list",
        "focus.history",
        "focus.policy",
        "config.path",
        "config.default",
        "input.devices",
        "input.sources",
        "input.current_source",
        "input.orientation_lock",
        "input.select",
        "input.set_orientation_lock",
        "input.text_target",
        "input.insert_text",
        "launch.status",
        "launch.begin",
        "launch.end",
        "launches.list",
        "launches.snapshot",
        "shortcuts.list",
        "shortcuts.actions",
        "layer.list",
        "layers.list",
        "monitor.list",
        "monitors.list",
        "output.list",
        "outputs.list",
        "outputs.renderer",
        "monitors.privacy_screen",
        "monitors.set_privacy_screen",
        "settings",
        "shortcut.actions",
        "shortcut.bind",
        "shortcut.capture",
        "shortcut.list",
        "shortcut.session.end",
        "shortcut.unbind",
        "privacy.state",
        "privacy.stop_sharing",
        "privacy.stop_recording",
        "location.authorize_app",
        "auth.begin",
        "auth.respond",
        "auth.cancel",
        "prompt.respond",
        "prompt.cancel",
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
        "window.list",
        "windows.list",
        "window.match",
        "window.thumbnail",
        "window.snap.offer",
        "window.action",
        "window.close",
        "window.kill",
        "window.raise",
        "window.lower",
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
        "window.snap_context",
        "window.snap",
        "window.begin_move",
        "window.begin_resize",
        "workspace.list",
        "workspaces.list",
        "workspace.create",
        "workspace.rename",
        "workspace.remove",
        "workspace.switch",
        "workspace.next",
        "workspace.previous",
        "workspace.move_active",
        "workspace.move_window",
        "runtime.reload_config",
        "config.restore_default",
        "runtime.status",
        "session.lock",
        "session.activity",
        "session.logout",
        "session.suspend",
        "session.hibernate",
        "session.reboot",
        "session.poweroff",
        "session.status",
        "appearance.color_scheme",
        "layer.animation_policy",
        "version",
        NULL,
    };
    GnoblinNativeControl* control = user_data;
    Client* client = g_new0(Client, 1);
    client->control = control;
    client->client_id = ++control->next_client_id;
    if (client->client_id == 0)
        client->client_id = ++control->next_client_id;
    client->connection = g_object_ref(connection);
    g_autoptr(GError) credentials_error = NULL;
    g_autoptr(GCredentials) credentials =
        g_socket_get_credentials(g_socket_connection_get_socket(connection), &credentials_error);
    if (credentials)
        client->peer_pid = g_credentials_get_unix_pid(credentials, &credentials_error);
    client->request = g_string_new(NULL);
    client->outgoing = g_queue_new();
    client->focus_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    client->menu_grants = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
    g_hash_table_add(control->clients, client);
    g_autoptr(JsonNode) greeting = json_node_new(JSON_NODE_OBJECT);
    JsonObject* hello = json_object_new();
    json_node_take_object(greeting, hello);
    json_object_set_string_member(hello, "event", "hello");
    json_object_set_int_member(hello, "version", GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_object_set_int_member(hello, "api_major", GNOBLIN_NATIVE_CONTROL_API_MAJOR);
    json_object_set_int_member(hello, "api_minor", GNOBLIN_NATIVE_CONTROL_API_MINOR);
    json_object_set_int_member(hello, "state_revision", control->state_revision);
    json_object_set_string_member(
        hello, "revision_scope",
        "window-workspace-monitor-layer-input-device-source-capability-state");
    json_object_set_string_member(hello, "revision_semantics", "monotonic-per-session");
    JsonArray* method_array = json_array_new();
    for (guint i = 0; methods[i]; i++)
        json_array_add_string_element(method_array, methods[i]);
    json_object_set_array_member(hello, "methods", method_array);
    JsonArray* event_array = json_array_new();
    for (guint i = 0; native_socket_events[i]; i++) {
        if (g_str_equal(native_socket_events[i], "gnoblin.session.state-changed") &&
            !control->session_lifecycle_supported)
            continue;
        json_array_add_string_element(event_array, native_socket_events[i]);
    }
    json_object_set_array_member(hello, "events", event_array);
    JsonArray* capability_array = json_array_new();
    for (guint i = 0; i < native_capabilities_count; i++)
        json_array_add_string_element(capability_array, native_capabilities[i].id);
    json_object_set_array_member(hello, "capabilities", capability_array);
    json_object_set_array_member(hello, "features", json_array_new());
    g_autofree char* greeting_text = json_to_string(greeting, FALSE);
    send_response(client, g_strconcat(greeting_text, "\n", NULL));
    return TRUE;
}

GVariant* gnoblin_native_control_receive_runtime_config(int runtime_fd, guint64* settings_revision,
                                                        GError** error) {
    g_return_val_if_fail(runtime_fd >= 0, NULL);
    g_return_val_if_fail(error == NULL || *error == NULL, NULL);
    g_autoptr(GnoblinRuntimeReader) reader = gnoblin_runtime_reader_new();
    gint64 deadline = g_get_monotonic_time() + 10 * G_USEC_PER_SEC;
    for (;;) {
        GnoblinRuntimePacket packet = {0};
        gboolean available = FALSE;
        if (!gnoblin_runtime_reader_receive(reader, runtime_fd, &packet, &available, error))
            return NULL;
        if (available) {
            if (packet.type != GNOBLIN_RUNTIME_PACKET_CONFIG || packet.request_id != 0) {
                gnoblin_runtime_packet_clear(&packet);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "expected initial Gnoblin runtime configuration");
                return NULL;
            }
            guint32 version = 0;
            GVariant* document =
                g_variant_lookup_value(packet.payload, "document", G_VARIANT_TYPE_VARDICT);
            guint64 revision = 0;
            guint64 runtime_generation = 0;
            gboolean valid =
                document && g_variant_lookup(packet.payload, "document_version", "u", &version) &&
                version == 1 &&
                g_variant_lookup(packet.payload, "settings_revision", "t", &revision) &&
                revision > 0 &&
                g_variant_lookup(packet.payload, "runtime_generation", "t", &runtime_generation) &&
                runtime_generation > 0 && g_variant_is_normal_form(document);
            gnoblin_runtime_packet_clear(&packet);
            if (!valid) {
                g_clear_pointer(&document, g_variant_unref);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "initial Gnoblin runtime configuration is invalid");
                return NULL;
            }
            if (settings_revision)
                *settings_revision = revision;
            bootstrap_runtime_generation = runtime_generation;
            if (!bootstrap_runtime_cache)
                bootstrap_runtime_cache = gnoblin_runtime_cache_new();
            if (!gnoblin_runtime_cache_replace(bootstrap_runtime_cache, document, revision,
                                               error)) {
                g_variant_unref(document);
                return NULL;
            }
            return document;
        }
        gint64 remaining = deadline - g_get_monotonic_time();
        if (remaining <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                                "timed out waiting for Gnoblin runtime configuration");
            return NULL;
        }
        struct pollfd pfd = {.fd = runtime_fd, .events = POLLIN};
        int result;
        do {
            result = poll(&pfd, 1, (int)MIN(remaining / 1000, G_MAXINT));
        } while (result < 0 && errno == EINTR);
        if (result == 0)
            continue;
        if (result < 0 || (pfd.revents & (POLLERR | POLLHUP | POLLNVAL))) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                        "Gnoblin runtime channel failed during startup: %s",
                        result < 0 ? g_strerror(errno) : "peer closed the channel");
            return NULL;
        }
    }
}

static gboolean native_runtime_write_ready(gint fd, GIOCondition condition, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    if (condition & (G_IO_HUP | G_IO_ERR | G_IO_NVAL)) {
        control->runtime_write_source_id = 0;
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    g_autoptr(GError) error = NULL;
    if (!gnoblin_runtime_writer_flush(control->runtime_writer, fd, &error)) {
        if (g_error_matches(error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK))
            return G_SOURCE_CONTINUE;
        control->runtime_write_source_id = 0;
        g_warning("gnoblin-native-control: private runtime write failed: %s", error->message);
        native_runtime_abort(control);
        return G_SOURCE_REMOVE;
    }
    control->runtime_write_source_id = 0;
    return G_SOURCE_REMOVE;
}

gboolean native_runtime_send(GnoblinNativeControl* control, GnoblinRuntimePacketType type,
                             guint64 request_id, GVariant* payload, GError** error) {
    if (control->runtime_worker_suspended && type != GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED &&
        type != GNOBLIN_RUNTIME_PACKET_ERROR &&
        type != GNOBLIN_RUNTIME_PACKET_SESSION_STATE_PUBLISHED) {
        /* Runtime output is intentionally discarded while no Lua worker owns
         * the session. Resume publishes fresh snapshots after HELLO. The
         * lifecycle acknowledgement goes to the guardian, not the worker. */
        return TRUE;
    }
    if (!control->supervised_runtime || control->runtime_fd < 0 || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin runtime channel is unavailable");
        return FALSE;
    }
    if (!gnoblin_runtime_writer_queue(control->runtime_writer, type, request_id, payload, error))
        return FALSE;
    g_autoptr(GError) flush_error = NULL;
    if (!gnoblin_runtime_writer_flush(control->runtime_writer, control->runtime_fd, &flush_error)) {
        if (g_error_matches(flush_error, G_IO_ERROR, G_IO_ERROR_WOULD_BLOCK)) {
            if (!control->runtime_write_source_id)
                control->runtime_write_source_id =
                    g_unix_fd_add(control->runtime_fd, G_IO_OUT | G_IO_HUP | G_IO_ERR | G_IO_NVAL,
                                  native_runtime_write_ready, control);
            return TRUE;
        }
        g_propagate_error(error, g_steal_pointer(&flush_error));
        return FALSE;
    }
    return TRUE;
}

gboolean gnoblin_control_runtime_send(GnoblinNativeControl* control, GnoblinRuntimePacketType type,
                                      guint64 request_id, GVariant* payload, GError** error) {
    return native_runtime_send(control, type, request_id, payload, error);
}

gboolean native_runtime_send_event_packet(GnoblinNativeControl* control, const char* event,
                                          GVariant* packet) {
    if (control->runtime_worker_suspended)
        return TRUE;
    if (control->supervised_runtime && !control->runtime_hello_sent &&
        control->pending_runtime_events) {
        g_queue_push_tail(control->pending_runtime_events, g_variant_ref(packet));
        return TRUE;
    }
    g_autoptr(GError) error = NULL;
    if (native_runtime_send(control, GNOBLIN_RUNTIME_PACKET_EVENT, 0, packet, &error))
        return TRUE;
    native_runtime_abort(control);
    g_warning("gnoblin-native-control: could not queue runtime event %s: %s", event,
              error ? error->message : "unknown error");
    return FALSE;
}
