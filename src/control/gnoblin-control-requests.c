#include "gnoblin-native-control-private.h"

char* handle_request(Client* client, const char* data, gsize length) {
    g_autoptr(JsonParser) parser = json_parser_new();
    g_autoptr(GError) error = NULL;
    const char* id = "";
    if (!json_parser_load_from_data(parser, data, length, &error) ||
        !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
        client->close_after_response = TRUE;
        return encode_response(id, NULL, "invalid JSON request");
    }

    JsonObject* request = json_node_get_object(json_parser_get_root(parser));
    const char* op = json_object_get_string_member_with_default(request, "op", "");
    if (g_str_equal(op, "ping")) {
        JsonNode* id_node = json_object_get_member(request, "id");
        if (!id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING ||
            !json_node_get_string(id_node)[0] || strlen(json_node_get_string(id_node)) > 64)
            return encode_response("", NULL, "invalid request ID");
        const char* ping_id = json_node_get_string(id_node);
        if (json_object_get_size(request) != 2)
            return encode_response(ping_id, NULL, "ping accepts only op and id");
        JsonObject* pong = json_object_new();
        json_object_set_string_member(pong, "pong", "pong");
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, pong);
        return encode_response(ping_id, result, NULL);
    }
    if (client->control && client->control->stopping) {
        JsonNode* id_node = json_object_get_member(request, "id");
        const char* request_id = id_node && JSON_NODE_HOLDS_VALUE(id_node) &&
                                         json_node_get_value_type(id_node) == G_TYPE_STRING
                                     ? json_node_get_string(id_node)
                                     : "";
        return encode_response(request_id, NULL, "Gnoblin compositor control is stopping");
    }
    if (g_str_equal(op, "ui-session"))
        return handle_ui_session_request(client, request);
    if (json_object_has_member(request, "api_version") ||
        json_object_has_member(request, "api_major") ||
        json_object_has_member(request, "api_minor")) {
        JsonNode* version_node = json_object_get_member(request, "api_version");
        JsonObject* version =
            JSON_NODE_HOLDS_OBJECT(version_node) ? json_node_get_object(version_node) : NULL;
        JsonNode* major_node = version ? json_object_get_member(version, "major")
                                       : json_object_get_member(request, "api_major");
        JsonNode* minor_node = version ? json_object_get_member(version, "minor")
                                       : json_object_get_member(request, "api_minor");
        if ((version && json_object_get_size(version) != 2) || !major_node || !minor_node ||
            !JSON_NODE_HOLDS_VALUE(major_node) || !JSON_NODE_HOLDS_VALUE(minor_node) ||
            (json_node_get_value_type(major_node) != G_TYPE_INT64 &&
             json_node_get_value_type(major_node) != G_TYPE_INT) ||
            (json_node_get_value_type(minor_node) != G_TYPE_INT64 &&
             json_node_get_value_type(minor_node) != G_TYPE_INT)) {
            if (g_str_equal(op, "api"))
                native_socket_consume_rejected_menu_context(client, request);
            client->close_after_response = TRUE;
            return encode_response("", NULL,
                                   "API version must contain integer major and minor fields");
        }
        gint64 major = json_node_get_int(major_node);
        gint64 minor = json_node_get_int(minor_node);
        if (major != GNOBLIN_NATIVE_CONTROL_API_MAJOR || minor < 0 ||
            minor > GNOBLIN_NATIVE_CONTROL_API_MINOR) {
            if (g_str_equal(op, "api"))
                native_socket_consume_rejected_menu_context(client, request);
            client->close_after_response = TRUE;
            return encode_response("", NULL, "requested compositor API version is unsupported");
        }
        client->api_minor = minor;
    }
    if (g_str_equal(op, "api") && client->api_minor < 30)
        native_socket_consume_rejected_menu_context(client, request);
    if (g_str_equal(op, "events")) {
        if (client->api_minor < 9)
            return encode_response("", NULL, "events subscriptions require API version 1.9");
        JsonNode* events_node = json_object_get_member(request, "events");
        if (!events_node || !JSON_NODE_HOLDS_ARRAY(events_node))
            return encode_response("", NULL, "events must be an array of event names");
        JsonArray* requested = json_node_get_array(events_node);
        if (json_array_get_length(requested) > 64)
            return encode_response("", NULL, "events may contain at most 64 names");
        GHashTable* subscriptions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
        for (guint i = 0; i < json_array_get_length(requested); i++) {
            JsonNode* name_node = json_array_get_element(requested, i);
            if (!JSON_NODE_HOLDS_VALUE(name_node) ||
                json_node_get_value_type(name_node) != G_TYPE_STRING) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events entries must be strings");
            }
            const char* name = json_node_get_string(name_node);
            if (!native_event_is_subscribable(name) &&
                !native_mutter_event_is_subscribable(client->control, name)) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events contains an unsupported event name");
            }
            if (g_str_equal(name, "gnoblin.shortcut.activated") && client->api_minor < 10) {
                g_hash_table_unref(subscriptions);
                return encode_response(
                    "", NULL, "shortcut activation subscriptions require API version 1.10");
            }
            if (g_str_equal(name, "gnoblin.shortcut.binding-activated") && client->api_minor < 11) {
                g_hash_table_unref(subscriptions);
                return encode_response(
                    "", NULL, "dynamic shortcut activation events require API version 1.11");
            }
            if (g_str_equal(name, "gnoblin.shortcut.binding-deactivated") &&
                client->api_minor < 36) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "shortcut deactivation events require API version 1.36");
            }
            if ((g_str_equal(name, "gnoblin.shortcut.session.activated") ||
                 g_str_equal(name, "gnoblin.shortcut.session.key") ||
                 g_str_equal(name, "gnoblin.shortcut.session.ended")) &&
                client->api_minor < 22) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "shortcut session events require API version 1.22");
            }
            if (g_str_equal(name, "gnoblin.focus.policy-changed") && client->api_minor < 13) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "focus policy events require API version 1.13");
            }
            if (g_str_equal(name, "gnoblin.permission.changed") && client->api_minor < 16) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "permission policy events require API version 1.16");
            }
            if (g_str_equal(name, "gnoblin.privacy.changed") && client->api_minor < 17) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "privacy state events require API version 1.17");
            }
            if (g_str_equal(name, "gnoblin.location.authorization-requested") &&
                client->api_minor < 65) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "location authorization events require API version 1.65");
            }
            if (g_str_has_prefix(name, "gnoblin.auth.") && client->api_minor < 78) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "auth events require API version 1.78");
            }
            if (g_str_has_prefix(name, "gnoblin.prompt.") && client->api_minor < 79) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "prompt events require API version 1.79");
            }
            if (g_str_equal(name, "gnoblin.input.orientation-lock-changed") &&
                client->api_minor < 66) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "orientation lock events require API version 1.66");
            }
            if (g_str_equal(name, "gnoblin.monitor.privacy-screen-changed") &&
                client->api_minor < 74) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "monitor privacy-screen events require API version 1.74");
            }
            if (g_str_equal(name, "gnoblin.capability.changed") && client->api_minor < 33) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "capability change events require API version 1.33");
            }
            if ((g_str_equal(name, "gnoblin.animation.started") ||
                 g_str_equal(name, "gnoblin.animation.finished")) &&
                client->api_minor < 18) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "animation events require API version 1.18");
            }
            if ((g_str_equal(name, "gnoblin.config.reloaded") ||
                 g_str_equal(name, "gnoblin.config.reload-failed")) &&
                client->api_minor < 20) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "config reload events require API version 1.20");
            }
            if ((g_str_equal(name, "gnoblin.portal.grant-added") ||
                 g_str_equal(name, "gnoblin.portal.grant-removed")) &&
                client->api_minor < 15) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "portal grant events require API version 1.15");
            }
            if (g_str_equal(name, "gnoblin.operation.completed") && client->api_minor < 11) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "gnoblin.operation.completed requires API version 1.11");
            }
            if ((g_str_equal(name, "gnoblin.session.lock-requested") ||
                 g_str_equal(name, "gnoblin.session.lock-state-changed")) &&
                client->api_minor < 21) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "session lock events require API version 1.21");
            }
            if (g_str_equal(name, "gnoblin.session.state-changed")) {
                if (client->api_minor < 76) {
                    g_hash_table_unref(subscriptions);
                    return encode_response("", NULL,
                                           "session lifecycle events require API version 1.76");
                }
                if (!client->control->session_lifecycle_supported) {
                    g_hash_table_unref(subscriptions);
                    return encode_response(
                        "", NULL, "session lifecycle events are unavailable in this session");
                }
            }
            if ((g_str_equal(name, "gnoblin.window.drag.started") ||
                 g_str_equal(name, "gnoblin.window.drag.updated") ||
                 g_str_equal(name, "gnoblin.window.drag.ended")) &&
                client->api_minor < 26) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "window drag events require API version 1.26");
            }
            if ((g_str_equal(name, "gnoblin.window.menu-requested") ||
                 g_str_equal(name, "gnoblin.osd.requested")) &&
                client->api_minor < 27) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "shell request events require API version 1.27");
            }
            if (g_str_equal(name, "gnoblin.pointer.locate-requested") && client->api_minor < 80) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "pointer locate events require API version 1.80");
            }
            if (g_str_equal(name, "gnoblin.input.pad-help-requested") && client->api_minor < 73) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "tablet-pad help events require API version 1.73");
            }
            if (g_str_equal(name, "gnoblin.window.activation-denied") && client->api_minor < 69) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "activation denial events require API version 1.69");
            }
            if (g_str_has_prefix(name, "gnoblin.layer.") && client->api_minor < 71) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "layer lifecycle events require API version 1.71");
            }
            if (g_str_equal(name, "gnoblin.runtime.status-changed") && client->api_minor < 72) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "runtime status events require API version 1.72");
            }
            if (g_str_equal(name, "gnoblin.session.activity-changed") && client->api_minor < 24) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "session activity events require API version 1.24");
            }
            if (g_str_equal(name, "gnoblin.appearance.color-scheme-changed") &&
                client->api_minor < 34) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL,
                                       "appearance change events require API version 1.34");
            }
            if (g_hash_table_contains(subscriptions, name)) {
                g_hash_table_unref(subscriptions);
                return encode_response("", NULL, "events must not contain duplicate names");
            }
            g_hash_table_add(subscriptions, g_strdup(name));
        }
        native_socket_revoke_client_tokens(client);
        if (client->focus_grants)
            g_hash_table_remove_all(client->focus_grants);
        gnoblin_control_window_drag_client_disconnected(client->control, client->client_id);
        gboolean refresh_mutter_watches = native_client_has_mutter_event_subscriptions(client);
        g_clear_pointer(&client->event_subscriptions, g_hash_table_unref);
        client->event_subscriptions = subscriptions;
        refresh_mutter_watches |= native_client_has_mutter_event_subscriptions(client);
        if (refresh_mutter_watches)
            native_refresh_all_mutter_signal_watches(client->control);
        client->event_api_minor = client->api_minor;
        return encode_event_subscription(requested, client->event_api_minor);
    }
    if (g_str_equal(op, "windows")) {
        client->windows_api_minor = client->api_minor;
        client->track_windows = TRUE;
        return window_snapshot(client->control);
    }
    if (g_str_equal(op, "monitors")) {
        client->monitors_api_minor = client->api_minor;
        client->track_monitors = TRUE;
        return monitor_snapshot(client->control);
    }
    if (json_object_has_member(request, "id") &&
        JSON_NODE_HOLDS_VALUE(json_object_get_member(request, "id")) &&
        json_node_get_value_type(json_object_get_member(request, "id")) == G_TYPE_STRING)
        id = json_object_get_string_member(request, "id");
    if (!*id || strlen(id) > 64)
        return encode_response("", NULL, "invalid request ID");
    const char* method = json_object_get_string_member_with_default(request, "method", "");
    if (!g_str_equal(op, "api"))
        return encode_response(id, NULL, "unsupported compositor operation");
    if (g_str_equal(method, "layer.list") && client->api_minor < 2)
        return encode_response(id, NULL, "layer.list requires API version 1.2");
    if (g_str_equal(method, "input.devices") && client->api_minor < 3)
        return encode_response(id, NULL, "input.devices requires API version 1.3");
    if ((g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source") ||
         g_str_equal(method, "input.select_source") || g_str_equal(method, "input.select")) &&
        client->api_minor < 6)
        return encode_response(id, NULL, "input source methods require API version 1.6");
    if ((g_str_equal(method, "input.orientation_lock") ||
         g_str_equal(method, "input.set_orientation_lock")) &&
        client->api_minor < 66)
        return encode_response(id, NULL, "orientation lock methods require API version 1.66");
    if ((g_str_equal(method, "monitors.privacy_screen") ||
         g_str_equal(method, "monitors.set_privacy_screen")) &&
        client->api_minor < 74)
        return encode_response(id, NULL, "monitor privacy-screen methods require API version 1.74");
    if (g_str_equal(method, "appearance.color_scheme") && client->api_minor < 70)
        return encode_response(id, NULL, "appearance.color_scheme requires API version 1.70");
    if (g_str_equal(method, "shortcut.actions") && client->api_minor < 5)
        return encode_response(id, NULL, "shortcut.actions requires API version 1.5");
    if (g_str_equal(method, "shortcut.capture") && client->api_minor < 8)
        return encode_response(id, NULL, "shortcut.capture requires API version 1.8");
    if (g_str_equal(method, "shortcut.list") && client->api_minor < 9)
        return encode_response(id, NULL, "shortcut.list requires API version 1.9");
    if (g_str_equal(method, "window.focus") && client->api_minor < 10)
        return encode_response(id, NULL, "window.focus requires API version 1.10");
    if ((g_str_equal(method, "window.begin_move") || g_str_equal(method, "window.begin_resize")) &&
        client->api_minor < 12)
        return encode_response(id, NULL, "interactive window operations require API version 1.12");
    if ((g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) &&
        client->api_minor < 11)
        return encode_response(id, NULL, "dynamic shortcut methods require API version 1.11");
    if (g_str_equal(method, "shortcut.session.end") && client->api_minor < 64)
        return encode_response(id, NULL, "shortcut.session.end requires API version 1.64");
    if (g_str_equal(method, "location.authorize_app") && client->api_minor < 65)
        return encode_response(id, NULL, "location.authorize_app requires API version 1.65");
    if (g_str_has_prefix(method, "auth.") && client->api_minor < 78)
        return encode_response(id, NULL, "auth methods require API version 1.78");
    if (g_str_has_prefix(method, "prompt.") && client->api_minor < 79)
        return encode_response(id, NULL, "prompt methods require API version 1.79");
    if ((g_str_equal(method, "grant.list") || g_str_equal(method, "grant.revoke")) &&
        client->api_minor < 14)
        return encode_response(id, NULL, "portal grant methods require API version 1.14");
    if (g_str_equal(method, "portals.grants") && client->api_minor < 15)
        return encode_response(id, NULL, "portals.grants requires API version 1.15");
    if (g_str_equal(method, "permissions.policy") && client->api_minor < 16)
        return encode_response(id, NULL, "permissions.policy requires API version 1.16");
    if (g_str_equal(method, "privacy.state") && client->api_minor < 17)
        return encode_response(id, NULL, "privacy.state requires API version 1.17");
    if (g_str_equal(method, "session.lock") && client->api_minor < 21)
        return encode_response(id, NULL, "session.lock requires API version 1.21");
    if (g_str_equal(method, "session.activity") && client->api_minor < 24)
        return encode_response(id, NULL, "session.activity requires API version 1.24");
    if (g_str_equal(method, "session.status") && client->api_minor < 29)
        return encode_response(id, NULL, "session.status requires API version 1.29");
    if (g_str_equal(method, "session.logout") && client->api_minor < 32)
        return encode_response(id, NULL, "session.logout requires API version 1.32");
    if (g_str_equal(method, "config.restore_default") && client->api_minor < 32)
        return encode_response(id, NULL, "config.restore_default requires API version 1.32");
    if ((g_str_equal(method, "session.suspend") || g_str_equal(method, "session.hibernate") ||
         g_str_equal(method, "session.reboot") || g_str_equal(method, "session.poweroff")) &&
        client->api_minor < 32)
        return encode_response(id, NULL, "session power methods require API version 1.32");
    if (g_str_equal(method, "window.restore_or_minimize") && client->api_minor < 38)
        return encode_response(id, NULL, "window.restore_or_minimize requires API version 1.38");
    if (g_str_equal(method, "launches.snapshot") && client->api_minor < 39)
        return encode_response(id, NULL, "launches.snapshot requires API version 1.39");
    if (g_str_equal(method, "shortcuts.list") && client->api_minor < 40)
        return encode_response(id, NULL, "shortcuts.list requires API version 1.40");
    if (g_str_equal(method, "shortcuts.actions") && client->api_minor < 41)
        return encode_response(id, NULL, "shortcuts.actions requires API version 1.41");
    if ((g_str_equal(method, "privacy.stop_sharing") ||
         g_str_equal(method, "privacy.stop_recording")) &&
        client->api_minor < 31)
        return encode_response(id, NULL, "privacy stop methods require API version 1.31");
    if (g_str_equal(method, "layer.animation_policy") && client->api_minor < 31)
        return encode_response(id, NULL, "layer.animation_policy requires API version 1.31");
    if ((g_str_equal(method, "outputs.list") || g_str_equal(method, "outputs.renderer")) &&
        client->api_minor < 82)
        return encode_response(id, NULL, "output reads require API version 1.82");
    if ((g_str_equal(method, "windows.list") || g_str_equal(method, "workspaces.list") ||
         g_str_equal(method, "monitors.list") || g_str_equal(method, "layers.list") ||
         g_str_equal(method, "launches.list")) &&
        client->api_minor < 37)
        return encode_response(id, NULL, "Lua snapshot collection reads require API version 1.37");
    if (g_str_equal(method, "window.thumbnail") && client->api_minor < 23)
        return encode_response(id, NULL, "window.thumbnail requires API version 1.23");
    if (g_str_equal(method, "window.snap.offer") && client->api_minor < 26)
        return encode_response(id, NULL, "window.snap.offer requires API version 1.26");
    if ((g_str_equal(method, "input.text_target") || g_str_equal(method, "input.insert_text") ||
         g_str_equal(method, "window.snap_context") || g_str_equal(method, "window.snap")) &&
        client->api_minor < 28)
        return encode_response(id, NULL,
                               "text insertion and keyboard snap methods require API version 1.28");
    if (g_str_has_prefix(method, "animation.") && client->api_minor < 18)
        return encode_response(id, NULL, "animation methods require API version 1.18");
    if (native_api_read_method(method) && client->api_minor < 19)
        return encode_response(id, NULL, "shared snapshot reads require API version 1.19");
    if (g_str_equal(method, "runtime.reload_config") && client->api_minor < 20)
        return encode_response(id, NULL, "runtime.reload_config requires API version 1.20");
    if (g_str_equal(method, "runtime.status") && client->api_minor < 67)
        return encode_response(id, NULL, "runtime.status requires API version 1.67");
    if (g_str_has_prefix(method, "launch.") && client->api_minor < 7)
        return encode_response(id, NULL, "launch methods require API version 1.7");
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (arguments_node && !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return encode_response(id, NULL, "arguments must be an object");
    if (native_api_animation_read_method(method)) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "animation API arguments are invalid");
        g_autoptr(GError) animation_error = NULL;
        g_autoptr(GVariant) result = meta_gnoblin_dispatch_native_api(
            client->control->display, method, arguments, &animation_error);
        if (!result)
            return encode_response(id, NULL,
                                   animation_error ? animation_error->message
                                                   : "native animation query failed");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "window.list") || g_str_equal(method, "workspace.list")) {
        if (g_str_equal(method, "workspace.list") && arguments_node &&
            json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "workspace.list does not accept arguments");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments) {
            const char* message = g_str_equal(method, "window.list")
                                      ? "window.list arguments are invalid"
                                      : "workspace.list arguments are invalid";
            return encode_response(id, NULL, message);
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "read");
    }
    if (g_str_equal(method, "outputs.list") || g_str_equal(method, "outputs.renderer")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "output reads do not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "session.status")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "session.status does not accept arguments");
        g_autoptr(GVariant) lock_snapshot = native_session_lock_snapshot(
            client->control->wayland_compositor, client->control->session_lock_revision);
        gboolean lock_available = FALSE;
        const char* lock_state = NULL;
        guint64 revision = 0;
        g_variant_lookup(lock_snapshot, "lock_available", "b", &lock_available);
        g_variant_lookup(lock_snapshot, "lock_state", "&s", &lock_state);
        g_variant_lookup(lock_snapshot, "revision", "t", &revision);
        JsonObject* result_object = json_object_new();
        json_object_set_string_member(result_object, "state", "running");
        json_object_set_boolean_member(result_object, "lock_available", lock_available);
        json_object_set_int_member(result_object, "revision", (gint64)MIN(revision, G_MAXINT64));
        if (lock_available && lock_state)
            json_object_set_string_member(result_object, "lock_state", lock_state);
        const char* session_state = NULL;
        guint64 session_revision = 0;
        if (client->api_minor >= 76 && client->control->session_lifecycle_supported &&
            client->control->session_lifecycle_snapshot &&
            g_variant_lookup(client->control->session_lifecycle_snapshot, "state", "&s",
                             &session_state) &&
            g_variant_lookup(client->control->session_lifecycle_snapshot, "revision", "t",
                             &session_revision)) {
            json_object_set_string_member(result_object, "session_state", session_state);
            json_object_set_int_member(result_object, "session_revision",
                                       (gint64)MIN(session_revision, G_MAXINT64));
        }
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, result_object);
        return encode_response(id, result, NULL);
    }
    if (g_str_equal(method, "window.restore_or_minimize")) {
        JsonObject* json_arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        const char* fields[] = {"id"};
        JsonNode* id_node = json_arguments ? json_object_get_member(json_arguments, "id") : NULL;
        if (!native_socket_has_exact_fields(json_arguments, fields, G_N_ELEMENTS(fields)) ||
            !id_node || !JSON_NODE_HOLDS_VALUE(id_node) ||
            json_node_get_value_type(id_node) != G_TYPE_STRING || !json_node_get_string(id_node) ||
            !*json_node_get_string(id_node))
            return encode_response(id, NULL,
                                   "window.restore_or_minimize requires only a stable window id");
        g_autoptr(JsonNode) arguments_object = json_node_new(JSON_NODE_OBJECT);
        json_node_set_object(arguments_object, json_object_ref(json_arguments));
        g_autoptr(GVariant) operation_arguments = variant_from_json(arguments_object);
        if (!operation_arguments)
            return encode_response(id, NULL, "window.restore_or_minimize arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, operation_arguments, "call");
    }
    if (g_str_equal(method, "input.text_target") || g_str_equal(method, "input.insert_text") ||
        g_str_equal(method, "window.snap_context") || g_str_equal(method, "window.snap")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        g_autoptr(GError) method_error = NULL;
        g_autoptr(GVariant) result = NULL;
        if (g_str_equal(method, "input.text_target"))
            result = native_socket_create_text_target(client, arguments, &method_error);
        else if (g_str_equal(method, "input.insert_text"))
            result = native_socket_insert_text(
                client, arguments, native_socket_has_nul_escape(data, length), &method_error);
        else if (g_str_equal(method, "window.snap_context"))
            result = native_socket_create_snap_context(client, arguments, &method_error);
        else
            result = native_socket_commit_snap_context(client, arguments, &method_error);
        if (!result)
            return encode_response(
                id, NULL, method_error ? method_error->message : "native API request failed");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "window.snap.offer"))
        return native_socket_window_snap_offer(
            client, id, arguments_node ? json_node_get_object(arguments_node) : NULL);
    if (g_str_equal(method, "runtime.reload_config")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "runtime.reload_config arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "reload");
    }
    if (g_str_equal(method, "runtime.status")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "runtime.status does not accept arguments");
        GVariantBuilder status;
        g_variant_builder_init(&status, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&status, "{sv}", "state",
                              g_variant_new_string(native_runtime_status_state(client->control)));
        g_variant_builder_add(&status, "{sv}", "generation",
                              g_variant_new_int64((gint64)client->control->runtime_generation));
        g_autoptr(GVariant) result = g_variant_ref_sink(g_variant_builder_end(&status));
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "session.lock")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "session.lock does not accept arguments");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "session.lock arguments are invalid");
        if (client->api_minor >= 49) {
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, arguments, "call");
        }
        g_autoptr(GVariant) result = gnoblin_native_control_request_session_lock(
            client->control->display, arguments, &error);
        if (!result)
            return encode_response(id, NULL,
                                   error ? error->message : "session-lock request failed");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (native_api_read_method(method)) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "layer.list")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "monitor.list")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "output.list")) {
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "privacy.state")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "privacy.state does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "portals.grants")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        const char* kind_filter = NULL;
        if (arguments &&
            (json_object_get_size(arguments) > 1 ||
             (json_object_get_size(arguments) == 1 && !json_object_has_member(arguments, "kind"))))
            return encode_response(id, NULL, "portals.grants accepts only an optional kind");
        if (arguments && json_object_has_member(arguments, "kind")) {
            JsonNode* kind_node = json_object_get_member(arguments, "kind");
            if (!JSON_NODE_HOLDS_VALUE(kind_node) ||
                json_node_get_value_type(kind_node) != G_TYPE_STRING)
                return encode_response(id, NULL, "portal grant kind must be a string");
            kind_filter = json_node_get_string(kind_node);
            if (!g_str_equal(kind_filter, "screen-cast") &&
                !g_str_equal(kind_filter, "remote-desktop"))
                return encode_response(id, NULL,
                                       "portal grant kind must be screen-cast or remote-desktop");
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!read_arguments)
            return encode_response(id, NULL, "portals.grants arguments are invalid");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.list does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.policy")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "permissions.policy does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "permissions.check")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        if (!arguments || json_object_get_size(arguments) != 2 ||
            !json_object_has_member(arguments, "capability") ||
            !json_object_has_member(arguments, "identity") ||
            !JSON_NODE_HOLDS_VALUE(json_object_get_member(arguments, "capability")) ||
            !JSON_NODE_HOLDS_VALUE(json_object_get_member(arguments, "identity")) ||
            json_node_get_value_type(json_object_get_member(arguments, "capability")) !=
                G_TYPE_STRING ||
            json_node_get_value_type(json_object_get_member(arguments, "identity")) !=
                G_TYPE_STRING)
            return encode_response(id, NULL,
                                   "permissions.check requires string capability and identity");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        g_autoptr(GVariant) read_arguments = variant_from_json(arguments_node);
        if (!read_arguments)
            return encode_response(id, NULL, "permissions.check arguments are invalid");
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "window.action")) {
        JsonObject* arguments = arguments_node && JSON_NODE_HOLDS_OBJECT(arguments_node)
                                    ? json_node_get_object(arguments_node)
                                    : NULL;
        JsonNode* action_node = arguments ? json_object_get_member(arguments, "action") : NULL;
        if (action_node && JSON_NODE_HOLDS_VALUE(action_node) &&
            json_node_get_value_type(action_node) == G_TYPE_STRING &&
            g_str_equal(json_node_get_string(action_node), "focus"))
            return encode_response(
                id, NULL,
                "native window.action focus is denied; use window.focus with a live focus_context");

        if (!action_node || !JSON_NODE_HOLDS_VALUE(action_node) ||
            json_node_get_value_type(action_node) != G_TYPE_STRING)
            return encode_response(id, NULL, "window.action requires a string action");

        const char* action = json_node_get_string(action_node);
        gboolean move_action = g_str_equal(action, "move");
        gboolean resize_action = g_str_equal(action, "resize");
        gboolean workspace_action = g_str_equal(action, "workspace");
        gboolean monitor_action = g_str_equal(action, "monitor");
        const char* action_fields[] = {"action", "window"};
        const char* move_fields[] = {"action", "window", "x", "y"};
        const char* move_fields_without_window[] = {"action", "x", "y"};
        const char* resize_fields[] = {"action", "window", "width", "height"};
        const char* resize_fields_without_window[] = {"action", "width", "height"};
        const char* workspace_fields[] = {"action", "window", "workspace"};
        const char* workspace_fields_without_window[] = {"action", "workspace"};
        const char* monitor_fields[] = {"action", "window", "monitor"};
        const char* monitor_fields_without_window[] = {"action", "monitor"};
        gboolean valid_fields =
            !arguments ? FALSE
            : move_action
                ? native_socket_has_exact_fields(arguments, move_fields,
                                                 G_N_ELEMENTS(move_fields)) ||
                      native_socket_has_exact_fields(arguments, move_fields_without_window,
                                                     G_N_ELEMENTS(move_fields_without_window))
            : resize_action
                ? native_socket_has_exact_fields(arguments, resize_fields,
                                                 G_N_ELEMENTS(resize_fields)) ||
                      native_socket_has_exact_fields(arguments, resize_fields_without_window,
                                                     G_N_ELEMENTS(resize_fields_without_window))
            : workspace_action
                ? native_socket_has_exact_fields(arguments, workspace_fields,
                                                 G_N_ELEMENTS(workspace_fields)) ||
                      native_socket_has_exact_fields(arguments, workspace_fields_without_window,
                                                     G_N_ELEMENTS(workspace_fields_without_window))
            : monitor_action
                ? native_socket_has_exact_fields(arguments, monitor_fields,
                                                 G_N_ELEMENTS(monitor_fields)) ||
                      native_socket_has_exact_fields(arguments, monitor_fields_without_window,
                                                     G_N_ELEMENTS(monitor_fields_without_window))
                : json_object_get_size(arguments) == 1 ||
                      native_socket_has_exact_fields(arguments, action_fields,
                                                     G_N_ELEMENTS(action_fields));
        if (!valid_fields)
            return encode_response(
                id, NULL,
                move_action
                    ? "window.action move requires integer x and y and accepts an optional string "
                      "window"
                : resize_action
                    ? "window.action resize requires integer width and height and accepts an "
                      "optional string window"
                : workspace_action
                    ? "window.action workspace requires one workspace selector and accepts an "
                      "optional string window"
                : monitor_action
                    ? "window.action monitor requires an integer monitor index and accepts an "
                      "optional string window"
                    : "window.action accepts only an optional string window");

        JsonNode* window_node = json_object_get_member(arguments, "window");
        if (window_node && (!JSON_NODE_HOLDS_VALUE(window_node) ||
                            json_node_get_value_type(window_node) != G_TYPE_STRING))
            return encode_response(id, NULL, "window.action window must be a stable ID string");

        JsonNode* x_node = move_action ? json_object_get_member(arguments, "x") : NULL;
        JsonNode* y_node = move_action ? json_object_get_member(arguments, "y") : NULL;
        if (move_action && (!x_node || !JSON_NODE_HOLDS_VALUE(x_node) ||
                            (json_node_get_value_type(x_node) != G_TYPE_INT &&
                             json_node_get_value_type(x_node) != G_TYPE_INT64) ||
                            !y_node || !JSON_NODE_HOLDS_VALUE(y_node) ||
                            (json_node_get_value_type(y_node) != G_TYPE_INT &&
                             json_node_get_value_type(y_node) != G_TYPE_INT64)))
            return encode_response(id, NULL, "window.action move requires integer x and y");

        JsonNode* width_node = resize_action ? json_object_get_member(arguments, "width") : NULL;
        JsonNode* height_node = resize_action ? json_object_get_member(arguments, "height") : NULL;
        if (resize_action && (!width_node || !JSON_NODE_HOLDS_VALUE(width_node) ||
                              (json_node_get_value_type(width_node) != G_TYPE_INT &&
                               json_node_get_value_type(width_node) != G_TYPE_INT64) ||
                              !height_node || !JSON_NODE_HOLDS_VALUE(height_node) ||
                              (json_node_get_value_type(height_node) != G_TYPE_INT &&
                               json_node_get_value_type(height_node) != G_TYPE_INT64)))
            return encode_response(id, NULL,
                                   "window.action resize requires integer width and height");

        JsonNode* workspace_node =
            workspace_action ? json_object_get_member(arguments, "workspace") : NULL;
        JsonObject* workspace_selector = workspace_node && JSON_NODE_HOLDS_OBJECT(workspace_node)
                                             ? json_node_get_object(workspace_node)
                                             : NULL;
        const char* workspace_selector_fields[] = {"id", "number"};
        JsonNode* workspace_id_node =
            workspace_selector ? json_object_get_member(workspace_selector, "id") : NULL;
        JsonNode* workspace_number_node =
            workspace_selector ? json_object_get_member(workspace_selector, "number") : NULL;
        gboolean workspace_id = workspace_id_node && JSON_NODE_HOLDS_VALUE(workspace_id_node) &&
                                json_node_get_value_type(workspace_id_node) == G_TYPE_STRING &&
                                json_node_get_string(workspace_id_node) &&
                                *json_node_get_string(workspace_id_node);
        gboolean workspace_number =
            workspace_number_node && JSON_NODE_HOLDS_VALUE(workspace_number_node) &&
            (json_node_get_value_type(workspace_number_node) == G_TYPE_INT ||
             json_node_get_value_type(workspace_number_node) == G_TYPE_INT64) &&
            json_node_get_int(workspace_number_node) > 0;
        if (workspace_action &&
            (!workspace_selector || json_object_get_size(workspace_selector) != 1 ||
             !native_socket_has_exact_fields(workspace_selector, workspace_selector_fields,
                                             G_N_ELEMENTS(workspace_selector_fields)) ||
             workspace_id == workspace_number))
            return encode_response(id, NULL, "window.action workspace requires {id} or {number}");

        JsonNode* monitor_node =
            monitor_action ? json_object_get_member(arguments, "monitor") : NULL;
        if (monitor_action &&
            (!monitor_node || !JSON_NODE_HOLDS_VALUE(monitor_node) ||
             (json_node_get_value_type(monitor_node) != G_TYPE_INT &&
              json_node_get_value_type(monitor_node) != G_TYPE_INT64) ||
             json_node_get_int(monitor_node) < 0 || json_node_get_int(monitor_node) > G_MAXINT))
            return encode_response(id, NULL,
                                   "window.action monitor requires a nonnegative integer index");

        if ((workspace_action || monitor_action) && client->api_minor < 63)
            return encode_response(id, NULL,
                                   "window.action workspace and monitor require API version 1.63");

        static const char* const native_actions[] = {
            "above",   "unabove",  "stick",      "unstick",    "close",        "minimize",
            "restore", "maximize", "unmaximize", "fullscreen", "unfullscreen", NULL};
        if (!g_strv_contains(native_actions, action) &&
            !(resize_action && client->api_minor >= 61) &&
            !(move_action && client->api_minor >= 62) &&
            !((workspace_action || monitor_action) && client->api_minor >= 63))
            return encode_response(
                id, NULL, "unsupported window.action; use a typed window operation when available");

        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");

        const char* target = window_node ? json_node_get_string(window_node) : "active";
        g_autofree char* active_window_id = NULL;
        if (g_str_equal(target, "active")) {
            MetaWindow* active_window = meta_display_get_focus_window(client->control->display);
            if (!active_window)
                return encode_response(id, NULL, "there is no active window");
            active_window_id = native_window_id(active_window);
            target = active_window_id;
        }

        const char* lua_method = method;
        gboolean enabled = TRUE;
        if (g_str_equal(action, "above") || g_str_equal(action, "unabove")) {
            lua_method = "window.set_above";
            enabled = g_str_equal(action, "above");
        } else if (g_str_equal(action, "stick") || g_str_equal(action, "unstick")) {
            lua_method = "window.set_sticky";
            enabled = g_str_equal(action, "stick");
        } else if (g_str_equal(action, "maximize") || g_str_equal(action, "unmaximize")) {
            lua_method = "window.set_maximized";
            enabled = g_str_equal(action, "maximize");
        } else if (g_str_equal(action, "fullscreen") || g_str_equal(action, "unfullscreen")) {
            lua_method = "window.set_fullscreen";
            enabled = g_str_equal(action, "fullscreen");
        } else if (resize_action) {
            lua_method = "window.resize";
        } else if (move_action) {
            lua_method = "window.move";
        } else {
            lua_method = g_str_equal(action, "close")      ? "window.close"
                         : g_str_equal(action, "minimize") ? "window.minimize"
                                                           : "window.unminimize";
        }

        GVariantBuilder typed_arguments;
        g_variant_builder_init(&typed_arguments, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&typed_arguments, "{sv}", "id", g_variant_new_string(target));
        if (g_str_has_prefix(lua_method, "window.set_"))
            g_variant_builder_add(&typed_arguments, "{sv}", "enabled",
                                  g_variant_new_boolean(enabled));
        if (resize_action) {
            g_variant_builder_add(&typed_arguments, "{sv}", "width",
                                  g_variant_new_int64(json_node_get_int(width_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "height",
                                  g_variant_new_int64(json_node_get_int(height_node)));
        }
        if (move_action) {
            g_variant_builder_add(&typed_arguments, "{sv}", "x",
                                  g_variant_new_int64(json_node_get_int(x_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "y",
                                  g_variant_new_int64(json_node_get_int(y_node)));
        }
        if (workspace_action) {
            GVariantBuilder selector;
            g_variant_builder_init(&selector, G_VARIANT_TYPE_VARDICT);
            if (workspace_id)
                g_variant_builder_add(
                    &selector, "{sv}", "id",
                    g_variant_new_string(json_node_get_string(workspace_id_node)));
            else
                g_variant_builder_add(
                    &selector, "{sv}", "number",
                    g_variant_new_int64(json_node_get_int(workspace_number_node)));
            g_variant_builder_add(&typed_arguments, "{sv}", "workspace",
                                  g_variant_builder_end(&selector));
            lua_method = "window.move_to_workspace";
        }
        if (monitor_action) {
            g_autoptr(GError) monitor_error = NULL;
            g_autoptr(JsonNode) snapshot =
                gnoblin_control_monitor_snapshot_json(client->control, FALSE, &monitor_error);
            g_autofree char* monitor_id =
                native_monitor_id_for_index(snapshot, (int)json_node_get_int(monitor_node));
            if (!monitor_id)
                return encode_response(
                    id, NULL, monitor_error ? monitor_error->message : "monitor not found");
            g_variant_builder_add(&typed_arguments, "{sv}", "monitor",
                                  g_variant_new_string(monitor_id));
            lua_method = "window.move_to_monitor";
        }
        g_autoptr(GVariant) operation_arguments =
            g_variant_ref_sink(g_variant_builder_end(&typed_arguments));
        return queue_runtime_api_request_internal(client, id, lua_method, operation_arguments,
                                                  "call", action, target);
    }
    if (g_str_equal(method, "window.match")) {
        JsonObject* arguments_object = arguments_node && JSON_NODE_HOLDS_OBJECT(arguments_node)
                                           ? json_node_get_object(arguments_node)
                                           : NULL;
        if ((arguments_node && !arguments_object) ||
            (arguments_object && json_object_get_size(arguments_object) > 1))
            return encode_response(id, NULL,
                                   "window.match accepts only an optional string window selector");
        if (arguments_object) {
            GList* keys = json_object_get_members(arguments_object);
            gboolean valid =
                !keys || (keys->next == NULL && g_str_equal((const char*)keys->data, "window"));
            g_list_free(keys);
            JsonNode* window_node = json_object_get_member(arguments_object, "window");
            if (!valid || (window_node && (!JSON_NODE_HOLDS_VALUE(window_node) ||
                                           json_node_get_value_type(window_node) != G_TYPE_STRING)))
                return encode_response(
                    id, NULL, "window.match accepts only an optional string window selector");
        }
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) arguments = arguments_node
                                            ? variant_from_json(arguments_node)
                                            : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!arguments)
            return encode_response(id, NULL, "window.match arguments are invalid");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        return queue_runtime_api_request(client, id, method, arguments, "read");
    }
    if (g_str_equal(method, "window.focus")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        g_autoptr(GVariant) result = native_socket_focus_window(client, arguments, &error);
        if (!result)
            return encode_response(id, NULL, error ? error->message : "window.focus was denied");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "window.begin_move") || g_str_equal(method, "window.begin_resize")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        gboolean menu_attempt = arguments && json_object_has_member(arguments, "menu_context");
        if (menu_attempt && client->api_minor < 30)
            return encode_response(id, NULL, "WM menu operations require API version 1.30");
        g_autoptr(GVariant) result =
            menu_attempt ? native_socket_begin_menu_window_grab(client, method, arguments, &error)
                         : native_socket_begin_window_grab(client, method, arguments, &error);
        if (!result)
            return encode_response(
                id, NULL, error ? error->message : "interactive window operation was denied");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "shortcut.list")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "shortcut.list does not accept arguments");
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, "shortcuts.list", read_arguments, "read");
    }
    if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        return g_str_equal(method, "shortcut.bind")
                   ? gnoblin_control_dynamic_shortcut_bind(client, id, arguments)
                   : gnoblin_control_dynamic_shortcut_unbind(client, id, arguments);
    }
    if (g_str_equal(method, "shortcut.session.end")) {
        JsonObject* arguments = arguments_node ? json_node_get_object(arguments_node) : NULL;
        return gnoblin_control_dynamic_shortcut_request_end_session(client, id, arguments);
    }
    if (g_str_has_prefix(method, "launch.")) {
        client->track_launches = TRUE;
        client->launch_api_minor = client->api_minor;
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) launch_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (g_str_equal(method, "launch.status")) {
            if (!launch_arguments)
                return encode_response(id, NULL, "launch arguments are invalid");
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, launch_arguments, "read");
        }
        if (client->api_minor >= 50 &&
            (g_str_equal(method, "launch.begin") || g_str_equal(method, "launch.end"))) {
            if (!launch_arguments)
                return encode_response(id, NULL, "launch arguments are invalid");
            if (!client->control->supervised_runtime)
                return encode_response(id, NULL, "Lua supervisor is not connected");
            return queue_runtime_api_request(client, id, method, launch_arguments, "call");
        }
        g_autoptr(GVariant) result = gnoblin_native_control_dispatch_launch(
            client->control->display, method, launch_arguments, &error);
        if (!result)
            return encode_response(id, NULL,
                                   error ? error->message : "launch feedback unavailable");
        g_autoptr(JsonNode) json = json_from_variant(result);
        return encode_response(id, json, NULL);
    }
    if (g_str_equal(method, "input.devices")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "input.devices does not accept arguments");
        if (client->api_minor >= 4) {
            client->track_input_devices = TRUE;
            client->input_devices_api_minor = client->api_minor;
        }
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source")) {
        if (arguments_node && json_object_get_size(json_node_get_object(arguments_node)) != 0)
            return encode_response(id, NULL, "input source reads do not accept arguments");
        client->track_input_sources = TRUE;
        client->input_sources_api_minor = client->api_minor;
        gnoblin_control_publish_input_source_changes(client->control,
                                                     client->control->state_revision);
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
        return queue_runtime_api_request(client, id, method, read_arguments, "read");
    }
    if (g_str_equal(method, "shortcut.actions")) {
        if (!client->control->supervised_runtime)
            return encode_response(id, NULL, "Lua supervisor is not connected");
        GVariantBuilder empty;
        g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) read_arguments =
            arguments_node ? variant_from_json(arguments_node)
                           : g_variant_ref_sink(g_variant_builder_end(&empty));
        if (!read_arguments)
            return encode_response(id, NULL, "shortcut.actions arguments are invalid");
        return queue_runtime_api_request(client, id, "shortcuts.actions", read_arguments, "read");
    }
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = arguments_node
                                        ? variant_from_json(arguments_node)
                                        : g_variant_ref_sink(g_variant_builder_end(&empty));
    if (!client->control->supervised_runtime)
        return encode_response(id, NULL, "Lua supervisor is not connected");
    return queue_runtime_api_request(client, id, method, arguments, "call");
}
