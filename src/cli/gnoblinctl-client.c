#include "gnoblinctl-internal.h"

JsonNode* call_compositor_with_api_minor(Cli* cli, const char* op, const char* method,
                                         JsonObject* arguments, guint api_minor, GError** error) {
    const char* method_name = method ? method : "";
    const char* operation_method = method_name;
    g_autofree char* id = g_uuid_string_random();
    g_autoptr(JsonBuilder) builder = json_builder_new();
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "op");
    json_builder_add_string_value(builder, op);
    json_builder_set_member_name(builder, "id");
    json_builder_add_string_value(builder, id);
    if (method) {
        json_builder_set_member_name(builder, "method");
        json_builder_add_string_value(builder, method_name);
    }
    if (g_str_equal(method_name, "window.thumbnail") || g_str_equal(method_name, "layer.list") ||
        g_str_equal(method_name, "window.restore_or_minimize") ||
        g_str_equal(method_name, "input.devices") || g_str_equal(method_name, "input.sources") ||
        g_str_equal(method_name, "input.current_source") ||
        g_str_equal(method_name, "input.select") || g_str_equal(method_name, "shortcut.actions") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcut.capture") || g_str_equal(method_name, "shortcut.list") ||
        g_str_equal(method_name, "privacy.state") || g_str_equal(method_name, "version") ||
        g_str_equal(method_name, "session.status") || g_str_equal(method_name, "session.logout") ||
        g_str_equal(method_name, "session.suspend") ||
        g_str_equal(method_name, "session.hibernate") ||
        g_str_equal(method_name, "session.reboot") ||
        g_str_equal(method_name, "session.poweroff") ||
        g_str_equal(method_name, "config.restore_default") ||
        g_str_equal(method_name, "session.activity") || g_str_equal(method_name, "session.lock") ||
        g_str_equal(method_name, "privacy.stop_sharing") ||
        g_str_equal(method_name, "privacy.stop_recording") ||
        g_str_equal(method_name, "launch.status") ||
        g_str_equal(method_name, "launches.snapshot") ||
        g_str_equal(method_name, "shortcuts.actions") ||
        g_str_equal(method_name, "shortcuts.list") ||
        g_str_equal(method_name, "capabilities.list") || g_str_equal(method_name, "windows.list") ||
        g_str_equal(method_name, "workspaces.list") || g_str_equal(method_name, "monitors.list") ||
        g_str_equal(method_name, "outputs.list") || g_str_equal(method_name, "outputs.renderer") ||
        g_str_equal(method_name, "monitors.privacy_screen") ||
        g_str_equal(method_name, "monitors.set_privacy_screen") ||
        g_str_equal(method_name, "layers.list") || g_str_equal(method_name, "focus.history") ||
        g_str_equal(method_name, "focus.policy") || g_str_equal(method_name, "settings") ||
        g_str_equal(method_name, "appearance.color_scheme") ||
        g_str_equal(method_name, "layer.animation_policy") ||
        g_str_equal(method_name, "runtime.reload_config") ||
        g_str_equal(method_name, "runtime.status") || g_str_equal(method_name, "launch.begin") ||
        g_str_equal(method_name, "launch.end") ||
        g_str_equal(method_name, "input.orientation_lock") ||
        g_str_equal(method_name, "input.set_orientation_lock") ||
        g_str_has_prefix(method_name, "window.") || g_str_has_prefix(method_name, "workspace.") ||
        g_str_has_prefix(method_name, "animation.")) {
        json_builder_set_member_name(builder, "api_version");
        json_builder_begin_object(builder);
        json_builder_set_member_name(builder, "major");
        json_builder_add_int_value(builder, 1);
        json_builder_set_member_name(builder, "minor");
        json_builder_add_int_value(builder, api_minor);
        json_builder_end_object(builder);
    }
    if (method) {
        json_builder_set_member_name(builder, "arguments");
        json_builder_add_value(builder, json_node_init_object(json_node_alloc(), arguments));
    }
    json_builder_end_object(builder);
    g_autoptr(JsonNode) request = json_builder_get_root(builder);
    g_autofree char* encoded = json_to_string(request, FALSE);
    g_autofree char* payload = NULL;
    if (method) {
        g_autofree char* subscription_id = g_uuid_string_random();
        g_autoptr(JsonBuilder) subscription_builder = json_builder_new();
        json_builder_begin_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "op");
        json_builder_add_string_value(subscription_builder, "events");
        json_builder_set_member_name(subscription_builder, "id");
        json_builder_add_string_value(subscription_builder, subscription_id);
        json_builder_set_member_name(subscription_builder, "api_version");
        json_builder_begin_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "major");
        json_builder_add_int_value(subscription_builder, 1);
        json_builder_set_member_name(subscription_builder, "minor");
        json_builder_add_int_value(subscription_builder, 11);
        json_builder_end_object(subscription_builder);
        json_builder_set_member_name(subscription_builder, "events");
        json_builder_begin_array(subscription_builder);
        json_builder_add_string_value(subscription_builder, "gnoblin.operation.completed");
        json_builder_add_string_value(subscription_builder, "gnoblin.api.operation-completed");
        json_builder_end_array(subscription_builder);
        json_builder_end_object(subscription_builder);
        g_autoptr(JsonNode) subscription_request = json_builder_get_root(subscription_builder);
        g_autofree char* subscription_encoded = json_to_string(subscription_request, FALSE);
        payload = g_strdup_printf("%s\n%s\n", subscription_encoded, encoded);
    } else {
        payload = g_strconcat(encoded, "\n", NULL);
    }

    gboolean waits_for_operation = g_str_equal(method_name, "window.thumbnail") ||
                                   g_str_equal(method_name, "input.select") ||
                                   g_str_equal(method_name, "input.set_orientation_lock") ||
                                   g_str_equal(method_name, "monitors.set_privacy_screen") ||
                                   g_str_equal(method_name, "shortcut.capture") ||
                                   g_str_equal(method_name, "animation.preview");
    guint wait_timeout = cli->timeout + (g_str_equal(method_name, "shortcut.capture") ? 2 : 0);
    g_autoptr(GSocketClient) client = g_socket_client_new();
    g_socket_client_set_timeout(client, wait_timeout);
    g_autoptr(GSocketAddress) address = g_unix_socket_address_new(cli->socket_path);
    g_autoptr(GError) connect_error = NULL;
    g_autoptr(GSocketConnection) connection =
        g_socket_client_connect(client, G_SOCKET_CONNECTABLE(address), NULL, &connect_error);
    if (!connection) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED,
                    "Cannot connect to compositor socket %s: %s. Check that a Gnoblin session is "
                    "running and that the socket path is correct.",
                    cli->socket_path, connect_error->message);
        return NULL;
    }
    g_socket_set_timeout(g_socket_connection_get_socket(connection), wait_timeout);
    if (!g_output_stream_write_all(g_io_stream_get_output_stream(G_IO_STREAM(connection)), payload,
                                   strlen(payload), NULL, NULL, error))
        return NULL;

    gint64 deadline = g_get_monotonic_time() + (gint64)wait_timeout * G_USEC_PER_SEC;
    gint64 operation_request_id = 0;
    g_autoptr(GString) pending = g_string_new(NULL);
    GInputStream* input = g_io_stream_get_input_stream(G_IO_STREAM(connection));
    while (g_get_monotonic_time() < deadline) {
        char chunk[4096];
        gssize count = g_input_stream_read(input, chunk, sizeof chunk, NULL, error);
        if (count < 0)
            return NULL;
        if (count == 0) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                "Compositor disconnected before replying; the request was not retried");
            return NULL;
        }
        g_string_append_len(pending, chunk, count);
        if (pending->len > 4 * 1024 * 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "Compositor response is too large");
            return NULL;
        }
        char* line_end;
        while ((line_end = memchr(pending->str, '\n', pending->len))) {
            g_autofree char* line = g_strndup(pending->str, line_end - pending->str);
            g_string_erase(pending, 0, line_end - pending->str + 1);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, line, -1, NULL) ||
                !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "Invalid compositor response");
                return NULL;
            }
            JsonObject* response = json_node_get_object(json_parser_get_root(parser));
            const char* event = member_string(response, "event", "");
            gboolean legacy_completion = g_str_equal(event, "gnoblin.api.operation-completed");
            gboolean canonical_completion = g_str_equal(event, "gnoblin.operation.completed");
            gint64 completion_id = json_object_get_int_member_with_default(
                response, legacy_completion ? "request_id" : "operation_id", 0);
            if (operation_request_id > 0 && (legacy_completion || canonical_completion) &&
                g_str_equal(member_string(response, "method", ""), operation_method) &&
                completion_id == operation_request_id) {
                if (!json_object_get_boolean_member_with_default(response, "ok", FALSE)) {
                    const char* message = member_string(response, "error", NULL);
                    JsonObject* details = member_object(response, "error");
                    if (!message && details)
                        message = member_string(details, "message", NULL);
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "%s",
                                message ? message : "Gnoblin operation failed");
                    return NULL;
                }
                const char* result_key = legacy_completion ? "result" : "value";
                JsonObject* result = member_object(response, result_key);
                if (!result) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Invalid operation completion");
                    return NULL;
                }
                JsonNode* result_node = json_object_get_member(response, result_key);
                return json_node_copy(result_node);
            }
            if (!g_str_equal(member_string(response, "id", ""), id))
                continue;
            if (g_str_equal(event, "error")) {
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    member_string(response, "message", "Compositor rejected request"));
                return NULL;
            }
            if (g_str_equal(event, "reply")) {
                JsonNode* result_node = json_object_get_member(response, "result");
                JsonObject* result = member_object(response, "result");
                gboolean read_method = word_in("version capabilities.list focus.history settings "
                                               "focus.policy launches.snapshot",
                                               method_name);
                gboolean operation_descriptor = result &&
                                                json_object_has_member(result, "request_id") &&
                                                json_object_has_member(result, "method");
                gboolean array_result = result_node && JSON_NODE_HOLDS_ARRAY(result_node) &&
                                        word_in("capabilities.list focus.history windows.list "
                                                "workspaces.list monitors.list layers.list "
                                                "shortcuts.actions shortcut.actions shortcut.list "
                                                "outputs.list "
                                                "shortcuts.list",
                                                method_name);
                gboolean null_result = result_node && JSON_NODE_HOLDS_NULL(result_node) &&
                                       (read_method || g_str_equal(method_name, "animation.get"));
                if (!result && !array_result && !null_result) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Invalid compositor response");
                    return NULL;
                }
                if (waits_for_operation || operation_descriptor) {
                    JsonNode* request_id = json_object_get_member(result, "request_id");
                    if (!request_id || !JSON_NODE_HOLDS_VALUE(request_id) ||
                        (json_node_get_value_type(request_id) != G_TYPE_INT &&
                         json_node_get_value_type(request_id) != G_TYPE_INT64) ||
                        json_node_get_int(request_id) <= 0 ||
                        !g_str_equal(member_string(result, "method", ""), operation_method)) {
                        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                            "Invalid operation descriptor");
                        return NULL;
                    }
                    operation_request_id = json_node_get_int(request_id);
                    continue;
                }
                return json_node_copy(result_node);
            }
        }
    }
    g_set_error_literal(
        error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
        "Request timed out; it was not retried. Check current state before repeating an action.");
    return NULL;
}

JsonNode* call_compositor(Cli* cli, const char* op, const char* method, JsonObject* arguments,
                          GError** error) {
    return call_compositor_with_api_minor(cli, op, method, arguments,
                                          api_minor_for_method(method ? method : ""), error);
}

static JsonNode*
cli_lua_call_compositor_on_subscription_internal(CliLuaEventSubscription* subscription,
                                                 const char* method, JsonObject* arguments,
                                                 gboolean use_shortcut_owner, GError** error) {
    if (!subscription || !subscription->active || !subscription->connection || !subscription->cli ||
        !method || !arguments) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Lua event subscription is no longer active");
        return NULL;
    }

    Cli* cli = subscription->cli;
    CliLuaEventSubscription* transport =
        use_shortcut_owner && subscription->shortcut_owner ? cli->lua_shortcut_owner : subscription;
    if (!transport || !transport->active || !transport->connection) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "shortcut owner connection is no longer active");
        return NULL;
    }
    g_autofree char* id = g_uuid_string_random();
    g_autoptr(JsonBuilder) builder = json_builder_new();
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "op");
    json_builder_add_string_value(builder, "api");
    json_builder_set_member_name(builder, "id");
    json_builder_add_string_value(builder, id);
    json_builder_set_member_name(builder, "method");
    json_builder_add_string_value(builder, method);
    json_builder_set_member_name(builder, "api_version");
    json_builder_begin_object(builder);
    json_builder_set_member_name(builder, "major");
    json_builder_add_int_value(builder, 1);
    json_builder_set_member_name(builder, "minor");
    json_builder_add_int_value(builder, api_minor_for_method(method));
    json_builder_end_object(builder);
    json_builder_set_member_name(builder, "arguments");
    json_builder_add_value(builder, json_node_init_object(json_node_alloc(), arguments));
    json_builder_end_object(builder);
    g_autoptr(JsonNode) request = json_builder_get_root(builder);
    g_autofree char* encoded = json_to_string(request, FALSE);
    g_autofree char* payload = g_strconcat(encoded, "\n", NULL);

    GSocket* socket = g_socket_connection_get_socket(transport->connection);
    g_socket_set_timeout(socket, cli->timeout);
    g_socket_set_blocking(socket, TRUE);

    JsonNode* result = NULL;
    g_autoptr(GString) incoming =
        g_string_new_len(transport->pending->str, transport->pending->len);
    g_string_truncate(transport->pending, 0);
    if (!g_output_stream_write_all(
            g_io_stream_get_output_stream(G_IO_STREAM(transport->connection)), payload,
            strlen(payload), NULL, NULL, error))
        goto out;

    gint64 deadline = g_get_monotonic_time() + (gint64)cli->timeout * G_USEC_PER_SEC;
    GInputStream* input = g_io_stream_get_input_stream(G_IO_STREAM(transport->connection));
    while (g_get_monotonic_time() < deadline) {
        char* line_end;
        while ((line_end = memchr(incoming->str, '\n', incoming->len))) {
            g_autofree char* line = g_strndup(incoming->str, line_end - incoming->str);
            g_string_erase(incoming, 0, line_end - incoming->str + 1);
            g_autoptr(JsonParser) parser = json_parser_new();
            if (!json_parser_load_from_data(parser, line, -1, NULL) ||
                !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
                g_string_append_printf(transport->pending, "%s\n", line);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "Invalid compositor response on event connection");
                goto out;
            }

            JsonObject* response = json_node_get_object(json_parser_get_root(parser));
            if (g_str_equal(member_string(response, "id", ""), id)) {
                const char* event = member_string(response, "event", "");
                if (g_str_equal(event, "error")) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "%s",
                                member_string(response, "message", "Compositor rejected request"));
                    goto out;
                }
                JsonNode* response_result = json_object_get_member(response, "result");
                if (!g_str_equal(event, "reply") || !response_result) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                        "Invalid compositor reply on event connection");
                    goto out;
                }
                result = json_node_copy(response_result);
                goto out;
            }

            g_string_append_printf(transport->pending, "%s\n", line);
        }

        if (incoming->len > 4 * 1024 * 1024 || transport->pending->len > 4 * 1024 * 1024) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                                "Compositor data exceeded 4 MiB during event request");
            goto out;
        }

        char chunk[4096];
        g_autoptr(GError) read_error = NULL;
        gssize count = g_input_stream_read(input, chunk, sizeof chunk, NULL, &read_error);
        if (count < 0) {
            g_propagate_error(error, g_steal_pointer(&read_error));
            goto out;
        }
        if (count == 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                                "Compositor disconnected before replying to the event request");
            goto out;
        }
        g_string_append_len(incoming, chunk, count);
    }

    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_TIMED_OUT,
                        "Compositor event request timed out; a one-use context may have expired");

out:
    if (incoming->len > 0)
        g_string_append_len(transport->pending, incoming->str, incoming->len);
    g_socket_set_timeout(socket, 0);
    g_socket_set_blocking(socket, FALSE);
    return result;
}

JsonNode* cli_lua_call_compositor_on_subscription(CliLuaEventSubscription* subscription,
                                                  const char* method, JsonObject* arguments,
                                                  GError** error) {
    return cli_lua_call_compositor_on_subscription_internal(subscription, method, arguments, TRUE,
                                                            error);
}

JsonNode* cli_lua_call_compositor_direct_on_subscription(CliLuaEventSubscription* subscription,
                                                         const char* method, JsonObject* arguments,
                                                         GError** error) {
    return cli_lua_call_compositor_on_subscription_internal(subscription, method, arguments, FALSE,
                                                            error);
}
