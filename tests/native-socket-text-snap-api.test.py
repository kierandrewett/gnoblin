#!/usr/bin/env python3
"""Guard connection ownership for native text and keyboard-snap methods."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]
CONTROL = ROOT / "src/native-control/gnoblin-native-control.c"
HEADER = ROOT / "src/native-control/gnoblin-native-control.h"
LUA = ROOT / "src/config/gnoblin-lua.c"


def function_body(source: str, signature: str, end_marker: str) -> str:
    start = source.index(signature)
    end = source.index(end_marker, start)
    return source[start:end]


def api_minor(header: str) -> int:
    match = re.search(r"GNOBLIN_NATIVE_CONTROL_API_MINOR (\d+)", header)
    if not match:
        raise AssertionError("native-control API minor is missing")
    return int(match.group(1))


class NativeSocketTextSnapTests(unittest.TestCase):
    def test_handshake_advertises_current_runtime_methods(self):
        source = CONTROL.read_text()
        connected = function_body(
            source,
            "static gboolean client_connected(",
            "GVariant* gnoblin_native_control_receive_runtime_config(",
        )
        methods = connected.split("static const char* methods[] = {", 1)[1].split("NULL,", 1)[0]

        for method in (
            "privacy.stop_sharing",
            "privacy.stop_recording",
            "session.activity",
            "layer.animation_policy",
            "windows.list",
            "workspaces.list",
            "monitors.list",
            "layers.list",
            "launches.list",
            "window.restore_or_minimize",
        ):
            with self.subTest(method=method):
                self.assertIn(f'"{method}"', methods)

    def test_window_match_uses_native_query_and_is_advertised(self):
        source = CONTROL.read_text()
        connected = function_body(
            source,
            "static gboolean client_connected(",
            "GVariant* gnoblin_native_control_receive_runtime_config(",
        )
        methods = connected.split("static const char* methods[] = {", 1)[1].split("NULL,", 1)[0]
        dispatcher = function_body(
            source,
            'if (g_str_equal(method, "window.match")) {',
            'if (g_str_equal(method, "window.focus")) {',
        )

        self.assertIn('"window.match"', methods)
        self.assertIn("meta_gnoblin_dispatch_native_api", dispatcher)
        self.assertIn("client->control->display, method, arguments, &error", dispatcher)
        self.assertNotIn("queue_runtime_api_request", dispatcher)

    def test_window_socket_events_keep_legacy_field_aliases(self):
        source = CONTROL.read_text()
        aliases = function_body(
            source,
            "static void native_window_event_add_compat_aliases(",
            "static void publish_native_socket_event(",
        )
        publish = function_body(
            source,
            "static void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {",
            "static void native_publish_request_event(",
        )

        self.assertIn('{"window", "last"}', aliases)
        self.assertIn("native_window_record_add_public_aliases", aliases)
        self.assertIn("window_property_names[property].lua_name", aliases)
        self.assertIn("window_property_names[property].native_name", aliases)
        self.assertIn('g_str_has_prefix(name, "gnoblin.window.")', publish)
        self.assertIn("native_window_event_add_compat_aliases(socket_object)", publish)

    def test_appearance_changes_reach_lua_and_socket_at_api_134(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        startup = function_body(
            source,
            "GnoblinNativeControl* gnoblin_native_control_start(",
            "void gnoblin_native_control_stop(",
        )
        callback = function_body(
            source,
            "static void appearance_color_scheme_changed(",
            "static GVariant* privacy_snapshot_new(",
        )
        subscription = function_body(
            source,
            'if (g_str_equal(op, "events"))',
            'if (g_str_equal(op, "windows"))',
        )
        events = function_body(
            source,
            "static const char* native_socket_events[] = {",
            "static const char native_policy_introspection[]",
        )

        self.assertGreaterEqual(api_minor(header), 38)
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)
        self.assertIn('"gnoblin.appearance.color-scheme-changed"', events)
        self.assertIn("client->api_minor < 34", subscription)
        self.assertIn('"org.gnome.desktop.interface"', startup)
        self.assertIn('"changed::color-scheme"', startup)
        self.assertIn('g_settings_get_string(settings, "color-scheme")', callback)
        self.assertIn('"prefer-dark"', callback)
        self.assertIn('"prefer-light"', callback)
        self.assertIn(
            'native_publish_request_event(control, "gnoblin.appearance.color-scheme-changed"',
            callback,
        )
        self.assertIn(
            "gnoblin.appearance.color-scheme-changed",
            (ROOT / "docs/config/lua-events.md").read_text(),
        )

    def test_microphone_capability_changes_reach_lua_and_socket_at_api_133(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        callback = function_body(
            source,
            "static void privacy_microphone_state_changed(",
            "static void privacy_refresh_state(",
        )
        capability = function_body(
            source,
            "static GVariant* capability_snapshot_record(",
            "static GVariant* capability_snapshot(",
        )
        subscription = function_body(
            source,
            'if (g_str_equal(op, "events"))',
            'if (g_str_equal(op, "windows"))',
        )

        self.assertGreaterEqual(api_minor(header), 38)
        self.assertIn('"gnoblin.capability.changed"', source)
        self.assertIn("client->api_minor < 33", subscription)
        self.assertIn('g_str_equal(native_capability->id, "microphone-monitor")', capability)
        self.assertIn('"pipewire_unavailable"', capability)
        self.assertIn('"remote_desktop_disabled"', capability)
        self.assertIn('native_capability_by_id("microphone-monitor")', callback)
        self.assertNotIn("G_N_ELEMENTS(native_capabilities) - 1", callback)
        self.assertLess(
            callback.index('native_publish_runtime_snapshot(control, "capabilities"'),
            callback.index('native_publish_request_event(control, "gnoblin.capability.changed"'),
        )
        self.assertIn('native_publish_request_event(control, "gnoblin.capability.changed"', callback)

    def test_ping_is_a_transport_operation_not_an_api_method(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        ping = function_body(
            dispatcher,
            'if (g_str_equal(op, "ping")) {',
            'if (json_object_has_member(request, "api_version")',
        )

        self.assertIn('"ping accepts only op and id"', ping)
        self.assertIn('json_object_set_string_member(pong, "pong", "pong")', ping)
        self.assertNotIn("shell.ping", dispatcher)

    def test_stopping_control_rejects_new_requests_but_keeps_ping_available(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        ping = dispatcher.index('if (g_str_equal(op, "ping"))')
        stopping = dispatcher.index("if (client->control && client->control->stopping)")
        version_handling = dispatcher.index('if (json_object_has_member(request, "api_version")')

        self.assertLess(ping, stopping)
        self.assertLess(stopping, version_handling)
        self.assertIn('"Gnoblin compositor control is stopping"', dispatcher[stopping:version_handling])

    def test_runtime_abort_and_stop_drain_deferred_requests(self):
        source = CONTROL.read_text()
        abort = function_body(source, "static void native_runtime_abort(", "static gboolean native_runtime_send(")
        drain_start = source.rindex("static void native_runtime_fail_pending_requests(")
        drain_end = source.index("static gboolean native_runtime_send_worker_suspended(", drain_start)
        drain = source[drain_start:drain_end]
        operation = function_body(
            source,
            "static gboolean native_runtime_handle_operation(GnoblinNativeControl* control,",
            "static gboolean native_runtime_fd_ready(",
        )
        fd_ready = function_body(
            source,
            "static gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data) {",
            "gboolean gnoblin_native_control_dispatch_runtime_event(",
        )
        stop = function_body(
            source,
            "void gnoblin_native_control_stop(",
            "control->teardown_complete = TRUE;",
        )

        self.assertIn(
            'native_runtime_fail_pending_requests(control, "Lua runtime stopped before replying")',
            abort,
        )
        self.assertIn('clear_runtime_dynamic_shortcuts(control, "runtime_stopped")', abort)
        self.assertIn("stop_native_shortcut_capture(control, FALSE", abort)
        self.assertIn(
            'native_runtime_fail_pending_requests(control, "Gnoblin compositor stopped before replying")',
            stop,
        )
        self.assertIn('"Lua worker restarted before replying"', source)
        self.assertLess(
            drain.index("process_buffer(client)"),
            drain.index("client->pending_deferred_requests--"),
        )
        self.assertIn("client_maybe_free(client)", drain)
        self.assertLess(
            operation.index("if (!control || control->stopping)"),
            operation.index("meta_gnoblin_dispatch_native_api"),
        )
        self.assertLess(
            fd_ready.index("if (control->stopping)"),
            fd_ready.index("gnoblin_runtime_reader_receive"),
        )
        self.assertIn("g_clear_pointer(&control->pending_runtime_requests, g_hash_table_unref)", stop)

    def test_api_128_methods_are_advertised_and_routed_directly(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        direct = dispatcher.split('if (g_str_equal(method, "input.text_target") ||', 1)[1]
        direct = direct.split('if (g_str_equal(method, "window.snap.offer"))', 1)[0]

        self.assertGreaterEqual(api_minor(header), 38)
        for method in (
            "input.text_target",
            "input.insert_text",
            "window.snap_context",
            "window.snap",
        ):
            self.assertIn(f'"{method}",', source)
            self.assertIn(f'g_str_equal(method, "{method}")', dispatcher)
        self.assertIn("client->api_minor < 28", dispatcher)
        self.assertNotIn("queue_runtime_api_request", direct)

    def test_status_method_is_gated_at_api_129(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        self.assertIn('g_str_equal(method, "session.status") && client->api_minor < 29', dispatcher)

    def test_wm_menu_authority_is_api_130_typed_and_target_bound(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        menu_emit = function_body(
            source,
            "void gnoblin_native_control_window_menu_requested(",
            "#define NATIVE_DRAG_MOD_CONTROL",
        )
        socket_issue = function_body(
            menu_emit,
            "GList* clients = g_hash_table_get_keys(control->clients);",
            "if (runtime_handle)\n        native_runtime_dispatch_event",
        )
        socket_action = function_body(
            source,
            "static GVariant* native_socket_begin_menu_window_grab(",
            "static void native_launch_free(",
        )
        native_action = function_body(
            source,
            "GVariant* gnoblin_native_control_begin_menu_window_grab(",
            "void gnoblin_native_control_revoke_focus_contexts(",
        )

        self.assertGreaterEqual(api_minor(header), 38)
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)
        self.assertIn("menu == META_WINDOW_MENU_WM", menu_emit)
        self.assertIn("client->event_api_minor >= 30", socket_issue)
        self.assertNotIn("client->api_minor >= 30", socket_issue)
        self.assertIn('"menu_context", token', socket_issue)
        self.assertIn("native_menu_context_create(control, window_id, client->client_id", socket_issue)
        self.assertLess(
            socket_action.index("g_hash_table_remove(client->menu_grants, token)"),
            socket_action.index("gboolean exact ="),
        )
        self.assertLess(
            native_action.index("g_hash_table_remove(control->menu_contexts"), native_action.index("gboolean exact =")
        )
        self.assertIn("context.socket_owner_client_id != socket_owner_client_id", native_action)
        self.assertIn("context.window_id", native_action)
        self.assertNotIn('g_variant_lookup(arguments, "id"', native_action)
        self.assertIn("meta_wayland_session_lock_is_active", native_action)
        self.assertIn("meta_window_allows_move", native_action)
        self.assertIn("meta_window_allows_resize", native_action)
        self.assertIn("meta_display_get_current_time_roundtrip(display)", native_action)
        self.assertNotIn('json_object_set_int_member(socket_object, "expires_at_us"', socket_issue)

    def test_xdg_activation_focus_is_pid_bound_and_api_132(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        focus = function_body(
            source,
            "static GVariant* native_socket_focus_window(",
            "static GVariant* native_socket_begin_window_grab(",
        )
        connected = function_body(
            source, "static gboolean client_connected(", "GVariant* gnoblin_native_control_receive_runtime_config("
        )
        patch = (ROOT / "patches/mutter/99-typed-window-api/0050-focus-with-xdg-activation-token.patch").read_text()

        self.assertGreaterEqual(api_minor(header), 38)
        self.assertIn('g_str_equal(method, "window.focus") && client->api_minor < 10', dispatcher)
        self.assertIn("XDG Activation window focus requires API version 1.32", focus)
        self.assertIn("client->peer_pid <= 0", focus)
        self.assertIn("native_window_by_stable_id(client->control, wanted_id)", focus)
        self.assertIn("meta_wayland_activation_focus_window_with_token", focus)
        self.assertIn("g_socket_get_credentials", connected)
        self.assertIn("g_credentials_get_unix_pid", connected)
        self.assertIn("token->requesting_pid != peer_pid", patch)
        self.assertLess(patch.index("token->consumed = TRUE"), patch.index("meta_window_activate_full"))

    def test_session_logout_waits_for_matching_compositor_success(self):
        lua = (ROOT / "src/config/gnoblin-lua.c").read_text()
        runtime = (ROOT / "src/session/gnoblin-runtime.c").read_text()
        source = CONTROL.read_text()
        operation = function_body(
            source,
            "static gboolean native_runtime_handle_operation(",
            "static gboolean native_runtime_fd_ready(",
        )
        completion = function_body(
            runtime,
            "static gboolean handle_completion(",
            "static gboolean dispatch_parent_event(",
        )

        self.assertIn('"session.logout",', lua)
        self.assertIn("session.logout takes no arguments", lua)
        self.assertIn('g_str_equal(method, "session.logout") && client->api_minor < 32', source)
        self.assertIn('g_str_equal(method, "session.logout")', operation)
        self.assertIn('"accepted",', operation)
        self.assertIn("gboolean matching_success", completion)
        self.assertIn('g_variant_lookup(operation_result, "accepted", "b", &logout_accepted)', completion)
        self.assertIn('g_str_equal(method, "session.logout")', completion)
        self.assertIn('g_variant_lookup_value(payload, "value", G_VARIANT_TYPE_VARDICT)', completion)
        self.assertLess(
            completion.index("send_pending_operations(runtime, error)"), completion.index("if (matching_success)")
        )
        self.assertIn("runtime->exit_status = EXIT_SUCCESS", completion)
        self.assertIn("g_main_loop_quit(runtime->loop)", completion)

    def test_menu_authority_uses_subscription_version_not_latest_request_version(self):
        source = CONTROL.read_text()
        menu_emit = function_body(
            source,
            "void gnoblin_native_control_window_menu_requested(",
            "#define NATIVE_DRAG_MOD_CONTROL",
        )
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")

        self.assertIn("client->event_api_minor = client->api_minor", dispatcher)
        self.assertIn("client->event_api_minor >= 30", menu_emit)
        self.assertNotIn("client->api_minor >= 30", menu_emit)

    def test_rejected_api_versions_consume_matching_menu_context(self):
        source = CONTROL.read_text()
        consumer = function_body(
            source,
            "static void native_socket_consume_rejected_menu_context(",
            "static char* handle_request(",
        )
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        version_block = function_body(
            dispatcher,
            'if (json_object_has_member(request, "api_version")',
            'if (g_str_equal(op, "events"))',
        )
        malformed_rejection = function_body(
            version_block,
            "if ((version && json_object_get_size(version) != 2)",
            "gint64 major = json_node_get_int(major_node);",
        )
        unsupported_rejection = function_body(
            version_block,
            "if (major != GNOBLIN_NATIVE_CONTROL_API_MAJOR",
            "client->api_minor = minor;",
        )

        self.assertIn('"window.begin_move"', consumer)
        self.assertIn('"window.begin_resize"', consumer)
        self.assertIn('"menu_context"', consumer)
        self.assertIn("g_hash_table_remove(client->menu_grants, token)", consumer)
        self.assertIn("g_hash_table_remove(client->control->menu_contexts, &handle)", consumer)
        self.assertLess(
            malformed_rejection.index("native_socket_consume_rejected_menu_context"),
            malformed_rejection.index("client->close_after_response = TRUE"),
        )
        self.assertLess(
            unsupported_rejection.index("native_socket_consume_rejected_menu_context"),
            unsupported_rejection.index("client->close_after_response = TRUE"),
        )
        self.assertIn('if (g_str_equal(op, "api") && client->api_minor < 30)', version_block)
        self.assertIn("native_socket_consume_rejected_menu_context(client, request)", version_block)

    def test_menu_contexts_revoke_with_socket_and_runtime_authorities(self):
        source = CONTROL.read_text()
        revoke = function_body(
            source,
            "static void revoke_menu_contexts(GnoblinNativeControl* control) {",
            "static guint64 native_menu_context_create(",
        )
        socket_revoke = function_body(
            source,
            "static void native_socket_revoke_client_tokens(Client* client) {",
            "static void prune_focus_contexts(",
        )
        self.assertIn("g_hash_table_remove_all(control->menu_contexts)", revoke)
        self.assertIn("g_hash_table_remove_all(client->menu_grants)", revoke)
        self.assertIn("native_menu_context_revoke_client(control, client->client_id)", socket_revoke)
        self.assertIn("revoke_menu_contexts(control)", source)

    def test_status_socket_read_uses_lua_for_current_clients_and_keeps_legacy_path(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        status = function_body(
            dispatcher,
            'if (g_str_equal(method, "session.status")) {',
            'if (g_str_equal(method, "window.restore_or_minimize")) {',
        )
        reads = function_body(
            source,
            "static gboolean native_api_read_method(",
            "static gboolean runtime_reload_document_supported(",
        )

        self.assertIn("client->api_minor >= 51", status)
        self.assertIn("supervised_runtime", status)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', status)
        self.assertIn("native_session_status_json", status)
        self.assertIn('"session.status"', status)
        self.assertIn('g_str_equal(method, "session.status")', reads)

    def test_workspace_list_uses_lua_snapshot_for_api_152_and_keeps_legacy_path(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        workspace_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "window.list") || g_str_equal(method, "workspace.list")) {',
            'if (g_str_equal(method, "session.status")) {',
        )
        lua = LUA.read_text()
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error) {",
            "void gnoblin_config_finish_load(gboolean commit)",
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=57", cmake)
        self.assertIn("client->api_minor >= 52", workspace_list)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', workspace_list)
        self.assertIn("meta_gnoblin_dispatch_native_api", workspace_list)
        self.assertIn('"workspace.list"', read_api)
        self.assertIn("legacy_workspace_list_from_lua(value, error)", read_api)

    def test_window_list_uses_lua_snapshot_for_api_153_and_keeps_legacy_path(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        window_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "window.list") || g_str_equal(method, "workspace.list")) {',
            'if (g_str_equal(method, "session.status")) {',
        )
        lua = LUA.read_text()
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error) {",
            "void gnoblin_config_finish_load(gboolean commit)",
        )

        self.assertIn("client->api_minor >= 53", window_list)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', window_list)
        self.assertIn("meta_gnoblin_dispatch_native_api", window_list)
        self.assertIn('"window.list"', read_api)
        self.assertIn("legacy_window_list_from_lua(value, error)", read_api)

    def test_layer_animation_policy_socket_read_uses_supervised_lua_read(self):
        source = CONTROL.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        generic_read = function_body(
            dispatcher,
            "if (native_api_read_method(method)) {",
            'if (g_str_equal(method, "privacy.state")) {',
        )
        reads = function_body(
            source,
            "static gboolean native_api_read_method(",
            "static gboolean runtime_reload_document_supported(",
        )

        self.assertIn('g_str_equal(method, "layer.animation_policy")', reads)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', generic_read)
        self.assertIn("Lua supervisor is not connected", generic_read)

    def test_aborted_runtime_waits_for_native_teardown_before_free(self):
        source = CONTROL.read_text()
        maybe_free = function_body(
            source,
            "static void native_control_maybe_free_stopped(GnoblinNativeControl* control) {",
            "static void clear_pending_grant_delivery(",
        )
        stop = source.split("void gnoblin_native_control_stop(", 1)[1]

        self.assertIn("!control->teardown_complete", maybe_free)
        self.assertLess(
            stop.index("control->teardown_complete = TRUE;"),
            stop.index("native_control_maybe_free_stopped(control);"),
        )

    def test_expired_snap_contexts_and_targets_are_pruned_and_bounded(self):
        source = CONTROL.read_text()
        pruner = function_body(
            source,
            "static void prune_focus_contexts(GnoblinNativeControl* control, gint64 now) {",
            "static gboolean focus_context_expiry_tick(",
        )
        text_target = function_body(
            source,
            "GVariant* gnoblin_native_control_create_text_target(",
            "static GVariant* native_insert_text_owned(",
        )
        snap_context = function_body(
            source,
            "GVariant* gnoblin_native_control_create_snap_context(",
            "static GVariant* native_socket_create_snap_context(",
        )

        self.assertIn("control->snap_contexts", pruner)
        self.assertIn("context->expires_at_us <= now", pruner)
        self.assertIn("MAX_TEXT_TARGETS", text_target)
        self.assertIn("MAX_SNAP_CONTEXTS", snap_context)

    def test_text_target_requires_and_consumes_the_connection_focus_grant(self):
        source = CONTROL.read_text()
        take_grant = function_body(
            source,
            "static gboolean native_socket_take_focus_grant(",
            "static GVariant* native_socket_focus_window(",
        )
        create = function_body(
            source,
            "static GVariant* native_socket_create_text_target(",
            "static GVariant* native_socket_insert_text(",
        )

        self.assertLess(
            take_grant.index("g_hash_table_remove(client->focus_grants"),
            take_grant.index("revoke_focus_grants_for_handle"),
        )
        self.assertLess(
            create.index("native_socket_take_focus_grant"), create.index("gnoblin_native_control_create_text_target")
        )
        self.assertIn("target->socket_owner_client_id = client->client_id", create)

    def test_text_target_owner_check_precedes_consumption_and_invalid_fields_consume(self):
        source = CONTROL.read_text()
        insert = function_body(
            source,
            "static GVariant* native_socket_insert_text(",
            "static gboolean focus_identity_matches(",
        )

        owner_check = insert.index("stored->socket_owner_client_id != client->client_id")
        first_remove = insert.index("g_hash_table_remove(client->control->text_targets, token)")
        validation = insert.index("native_socket_has_exact_fields")
        self.assertLess(owner_check, first_remove)
        self.assertLess(first_remove, validation)
        self.assertIn("native_socket_has_nul_escape(data, length)", source)

        insert_native = function_body(
            source,
            "static GVariant* native_insert_text_owned(",
            "GVariant* gnoblin_native_control_insert_text(",
        )
        self.assertIn("stored->socket_owner_client_id != socket_owner_client_id", insert_native)
        self.assertLess(
            insert_native.index("stored->socket_owner_client_id != socket_owner_client_id"),
            insert_native.index("g_hash_table_remove(control->text_targets, token)"),
        )

    def test_snap_context_is_owner_bound_and_consumed_before_argument_validation(self):
        source = CONTROL.read_text()
        commit = function_body(
            source,
            "static GVariant* native_commit_snap_context_owned(",
            "GVariant* gnoblin_native_control_commit_snap_context(",
        )
        socket_context = function_body(
            source,
            "static GVariant* native_socket_create_snap_context(",
            "static GVariant* native_commit_snap_context_owned(",
        )

        owner_check = commit.index("stored->socket_owner_client_id != socket_owner_client_id")
        consume = commit.index("g_hash_table_remove(control->snap_contexts, token)")
        argument_check = commit.index("g_variant_n_children(arguments) != 3")
        self.assertLess(owner_check, consume)
        self.assertLess(consume, argument_check)
        self.assertIn("context->socket_owner_client_id = client->client_id", socket_context)
        self.assertIn('!g_str_equal(key, "expires_at_us")', socket_context)

        socket_snap = function_body(
            source,
            "static GVariant* native_socket_commit_snap_context(",
            "GVariant* gnoblin_native_control_create_text_target(",
        )
        self.assertIn("native_socket_snap_rect(frame_node)", socket_snap)
        self.assertRegex(socket_snap, r"client->client_id,\s*error")
        self.assertIn("revoke_focus_contexts(control)", source)

    def test_owner_tokens_are_revoked_on_disconnect_subscription_reset_and_global_revocation(self):
        source = CONTROL.read_text()
        revoke = function_body(
            source,
            "static void native_socket_revoke_client_tokens(Client* client) {",
            "static void prune_focus_contexts(",
        )
        client_close = function_body(source, "static void client_close(", "static JsonNode* json_from_variant(")
        subscription = function_body(source, 'if (g_str_equal(op, "events"))', 'if (g_str_equal(op, "windows"))')
        focus_revoke = function_body(
            source,
            "static void revoke_focus_contexts(GnoblinNativeControl* control) {",
            "static void revoke_text_targets(",
        )

        self.assertIn("control->text_targets", revoke)
        self.assertIn("control->snap_contexts", revoke)
        self.assertIn("native_socket_revoke_client_tokens(client)", client_close)
        self.assertIn("native_socket_revoke_client_tokens(client)", subscription)
        self.assertIn("g_hash_table_remove_all(control->snap_contexts)", focus_revoke)
        self.assertIn("revoke_text_targets(control)", focus_revoke)

    def test_lua_snapshot_collection_reads_require_api_137(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        connected = function_body(
            source,
            "static gboolean client_connected(",
            "GVariant* gnoblin_native_control_receive_runtime_config(",
        )
        read_methods = function_body(
            source,
            "static gboolean native_api_read_method(const char* method)",
            "static gboolean runtime_reload_document_supported(",
        )
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")

        for method in (
            "windows.list",
            "workspaces.list",
            "monitors.list",
            "layers.list",
            "launches.list",
            "launches.snapshot",
            "shortcuts.list",
            "shortcuts.actions",
        ):
            with self.subTest(method=method):
                self.assertIn(f'g_str_equal(method, "{method}")', read_methods)
                self.assertIn(f'g_str_equal(method, "{method}")', dispatcher)
        self.assertIn('"launches.snapshot"', connected)
        self.assertIn("client->api_minor < 37", dispatcher)
        self.assertIn('g_str_equal(method, "launches.snapshot") && client->api_minor < 39', dispatcher)
        self.assertIn('g_str_equal(method, "shortcuts.list") && client->api_minor < 40', dispatcher)
        self.assertIn('g_str_equal(method, "shortcuts.actions") && client->api_minor < 41', dispatcher)
        self.assertGreaterEqual(api_minor(header), 41)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', dispatcher)

        header = HEADER.read_text()
        self.assertGreaterEqual(api_minor(header), 39)

    def test_permissions_list_uses_lua_at_api_142_and_keeps_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        permission_list = function_body(
            source,
            'if (g_str_equal(method, "permissions.list")) {',
            'if (g_str_equal(method, "permissions.policy")) {',
        )

        self.assertIn("client->api_minor >= 42", permission_list)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', permission_list)
        self.assertIn("native_config_document(client->control)", permission_list)
        self.assertIn("gnoblin_permission_policy_list(document, config_path)", permission_list)
        self.assertLess(
            permission_list.index("client->api_minor >= 42"),
            permission_list.index("native_config_document(client->control)"),
        )
        self.assertGreaterEqual(api_minor(header), 42)

    def test_permissions_check_uses_lua_at_api_143_and_keeps_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        permission_check = function_body(
            source,
            'if (g_str_equal(method, "permissions.check")) {',
            'if (g_str_equal(method, "window.action")) {',
        )

        self.assertIn("client->api_minor >= 43", permission_check)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', permission_check)
        self.assertIn("permission_decision_json(document, capability, identity", permission_check)
        self.assertLess(
            permission_check.index("client->api_minor >= 43"),
            permission_check.index("permission_decision_json(document, capability, identity"),
        )
        self.assertGreaterEqual(api_minor(header), 43)

    def test_permissions_policy_uses_lua_at_api_144_and_keeps_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        permission_policy = function_body(
            source,
            'if (g_str_equal(method, "permissions.policy")) {',
            'if (g_str_equal(method, "permissions.check")) {',
        )

        self.assertIn("client->api_minor >= 44", permission_policy)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', permission_policy)
        self.assertIn("gnoblin_permission_policy_snapshot(document", permission_policy)
        self.assertLess(
            permission_policy.index("client->api_minor >= 44"),
            permission_policy.index("gnoblin_permission_policy_snapshot(document"),
        )
        self.assertGreaterEqual(api_minor(header), 44)

    def test_legacy_window_actions_dispatch_natively(self):
        source = CONTROL.read_text()
        handler = function_body(
            source,
            'if (g_str_equal(method, "window.action")) {',
            'if (g_str_equal(method, "window.match")) {',
        )

        self.assertIn('g_str_equal(json_node_get_string(action_node), "focus")', handler)
        self.assertIn("arguments_node ? json_node_get_object(arguments_node) : NULL", handler)
        self.assertIn("native_socket_has_exact_fields(arguments, fields, G_N_ELEMENTS(fields))", handler)
        self.assertIn('"unsupported native window.action; use a typed window operation when available"', handler)
        self.assertIn("meta_gnoblin_dispatch_native_api(", handler)
        self.assertIn("client->control->display, method, native_arguments", handler)

    def test_portal_grants_uses_lua_at_api_145_and_keeps_native_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        lua_source = LUA.read_text()
        grant_handler = function_body(
            source,
            'if (g_str_equal(method, "portals.grants")) {',
            'if (g_str_equal(method, "permissions.list")) {',
        )
        lua_read = function_body(
            lua_source,
            "GVariant* gnoblin_config_read_api(",
            "void gnoblin_config_finish_load(",
        )

        self.assertIn("client->api_minor >= 45", grant_handler)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', grant_handler)
        self.assertIn("client->control->portal_grant_snapshot", grant_handler)
        self.assertIn('"portals.grants"', lua_read)
        self.assertIn('g_str_equal(method, "portals.grants")', lua_read)
        self.assertIn('lua_getfield(state, -1, "grants")', lua_read)
        self.assertGreaterEqual(api_minor(header), 45)

    def test_input_snapshots_use_lua_at_api_146_and_keep_legacy_routes(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua_source = LUA.read_text()
        devices = function_body(
            source,
            'if (g_str_equal(method, "input.devices")) {',
            'if (g_str_equal(method, "input.sources") ||',
        )
        sources = function_body(
            source,
            'if (g_str_equal(method, "input.sources") || g_str_equal(method, "input.current_source")) {',
            'if (g_str_equal(method, "shortcut.actions")) {',
        )
        lua_read = function_body(
            lua_source,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        for method in ("input.devices", "input.sources", "input.current_source"):
            with self.subTest(method=method):
                self.assertIn(f'"{method}"', lua_read)
                self.assertIn(f'g_str_equal(method, "{method}")', lua_source)

        self.assertIn("client->api_minor >= 46", devices)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', devices)
        self.assertIn("client->api_minor >= 46", sources)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', sources)
        self.assertLess(sources.index("publish_input_source_changes"), sources.index("client->api_minor >= 46"))
        self.assertIn("input_device_snapshot(client->control)", devices)
        self.assertIn("input_source_snapshot(client->control)", sources)
        self.assertIn('g_str_equal(method, "input.devices") ? "devices" : "sources"', lua_source)
        self.assertIn('g_str_equal(method, "input.current_source") && lua_isnil(state, -1)', lua_source)
        self.assertIn("gnoblin_config_update_input_device_snapshot", lua_source)
        self.assertIn("gnoblin_config_update_input_source_snapshot", lua_source)
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)
        self.assertGreaterEqual(api_minor(header), 46)

    def test_privacy_state_uses_lua_at_api_147_and_keeps_native_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua_source = LUA.read_text()
        handler = function_body(
            source,
            'if (g_str_equal(method, "privacy.state")) {',
            'if (g_str_equal(method, "portals.grants")) {',
        )
        reads = function_body(
            source,
            "static gboolean native_api_read_method(",
            "static gboolean runtime_reload_document_supported(",
        )
        lua_read = function_body(
            lua_source,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertIn("client->api_minor >= 47", handler)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', handler)
        self.assertIn("client->control->privacy_snapshot", handler)
        self.assertNotIn('g_str_equal(method, "privacy.state")', reads)
        self.assertIn('"privacy.state"', lua_read)
        self.assertIn('g_str_equal(method, "privacy.state")', lua_read)
        self.assertIn('lua_getfield(state, -1, "state")', lua_read)
        self.assertGreaterEqual(api_minor(header), 47)
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)

    def test_restore_or_minimize_uses_lua_for_api_148_and_keeps_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        helper = function_body(
            source,
            "GVariant* gnoblin_native_control_restore_or_minimize_window(",
            "static void pending_thumbnail_free(",
        )
        unmanaged = function_body(
            source,
            "static void window_unmanaged(MetaWindow* window, gpointer user_data) {",
            "static void track_window(",
        )
        dispatcher = function_body(
            source,
            "static char* handle_request(",
            "static void process_buffer(",
        )
        snap = function_body(
            source,
            "static GVariant* native_commit_snap_context_owned(",
            "GVariant* gnoblin_native_control_commit_snap_context(",
        )
        drag_begin = function_body(
            source,
            "guint64 gnoblin_native_control_window_drag_begin(",
            "void gnoblin_native_control_window_drag_update(",
        )
        drag_snap = function_body(
            source,
            "gboolean gnoblin_native_control_take_window_drag_snap(",
            "GVariant* gnoblin_native_control_focus_window(",
        )

        self.assertGreaterEqual(api_minor(header), 48)
        self.assertIn("client->api_minor >= 48", dispatcher)
        self.assertIn('queue_runtime_api_request(client, id, method, operation_arguments, "call")', dispatcher)
        self.assertIn("client->control->supervised_runtime", dispatcher)
        self.assertIn("window.restore_or_minimize", source)
        self.assertIn("client->api_minor < 38", dispatcher)
        self.assertIn("gnoblin_native_control_restore_or_minimize_window", dispatcher)
        self.assertIn("meta_wayland_session_lock_is_active", helper)
        self.assertLess(
            helper.index("meta_window_get_maximize_flags"),
            helper.index("meta_window_can_minimize"),
        )
        self.assertIn("meta_window_move_resize_frame", helper)
        self.assertIn("g_hash_table_remove(control->snap_restore_frames, window)", unmanaged)
        self.assertIn("context.original_frame", snap)
        self.assertIn("meta_window_set_unmaximize_flags", snap)
        self.assertIn("g_hash_table_contains(control->snap_restore_frames, window)", snap)
        self.assertIn("drag->original_frame = drag->frame", drag_begin)
        self.assertIn("*original_frame = drag->original_frame", drag_snap)
        cmake = (ROOT / "CMakeLists.txt").read_text()
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)

    def test_session_lock_uses_lua_for_api_149_and_keeps_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        dispatcher = function_body(
            source,
            "static char* handle_request(",
            "static void process_buffer(",
        )
        operation_dispatch = function_body(
            source,
            "static gboolean native_runtime_handle_operation(",
            "static gboolean native_runtime_fd_ready(",
        )

        self.assertGreaterEqual(api_minor(header), 49)
        self.assertIn("client->api_minor >= 49", dispatcher)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "call")', dispatcher)
        self.assertIn("client->control->supervised_runtime", dispatcher)
        self.assertIn("client->api_minor < 21", dispatcher)
        self.assertIn("gnoblin_native_control_request_session_lock", dispatcher)
        self.assertIn('g_str_equal(method, "session.lock")', operation_dispatch)
        self.assertIn("gnoblin_native_control_request_session_lock", operation_dispatch)
        cmake = (ROOT / "CMakeLists.txt").read_text()
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)

    def test_launch_mutations_use_lua_for_api_150_and_keep_legacy_route(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        launch = function_body(
            source,
            'if (g_str_has_prefix(method, "launch.")) {',
            'if (g_str_equal(method, "input.devices")) {',
        )
        operation = function_body(
            source,
            "static gboolean native_runtime_handle_operation(",
            "static gboolean native_runtime_fd_ready(",
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("client->track_launches = TRUE", launch)
        self.assertIn("client->launch_api_minor = client->api_minor", launch)
        self.assertLess(launch.index("client->track_launches = TRUE"), launch.index("client->api_minor >= 50"))
        self.assertIn('g_str_equal(method, "launch.begin")', launch)
        self.assertIn('g_str_equal(method, "launch.end")', launch)
        self.assertIn('queue_runtime_api_request(client, id, method, launch_arguments, "call")', launch)
        self.assertIn("client->control->supervised_runtime", launch)
        self.assertIn("gnoblin_native_control_dispatch_launch", launch)
        self.assertIn('g_str_has_prefix(method, "launch.")', operation)
        self.assertIn("gnoblin_native_control_dispatch_launch", operation)
        cmake = (ROOT / "CMakeLists.txt").read_text()
        self.assertIn(f"GNOBLIN_NATIVE_CONTROL_API_MINOR={api_minor(header)}", cmake)

    def test_launch_status_uses_lua_for_api_154_and_keeps_event_tracking(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        launch = function_body(
            dispatcher,
            'if (g_str_has_prefix(method, "launch.")) {',
            'if (g_str_equal(method, "input.devices")) {',
        )
        lua = LUA.read_text()
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(const char* method, GVariant* arguments, GError** error) {",
            "void gnoblin_config_finish_load(gboolean commit)",
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=57", cmake)
        self.assertIn('g_str_equal(method, "launch.status") && client->api_minor >= 54', launch)
        self.assertLess(
            launch.index("client->track_launches = TRUE"),
            launch.index('g_str_equal(method, "launch.status")'),
        )
        self.assertIn('queue_runtime_api_request(client, id, method, launch_arguments, "read")', launch)
        self.assertIn("gnoblin_native_control_dispatch_launch", launch)
        self.assertIn('"launch.status"', read_api)
        self.assertIn('g_str_equal(method, "launches.snapshot") || g_str_equal(method, "launch.status")', read_api)

    def test_legacy_shortcut_list_uses_lua_for_api_155(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        shortcut_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "shortcut.list")) {',
            'if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {',
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=57", cmake)
        self.assertIn("client->api_minor >= 55", shortcut_list)
        self.assertIn("client->control->supervised_runtime", shortcut_list)
        self.assertIn('queue_runtime_api_request(client, id, "shortcuts.list", read_arguments, "read")', shortcut_list)
        self.assertIn("native_shortcut_snapshot", shortcut_list)

    def test_legacy_shortcut_actions_use_lua_for_api_156(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        shortcut_actions = function_body(
            dispatcher,
            'if (g_str_equal(method, "shortcut.actions")) {',
            "    GVariantBuilder empty;\n    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);\n    g_autoptr(GVariant) arguments",
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=57", cmake)
        self.assertIn("client->api_minor >= 56", shortcut_actions)
        self.assertIn("client->control->supervised_runtime", shortcut_actions)
        self.assertIn('queue_runtime_api_request(client, id, "shortcuts.actions", read_arguments,', shortcut_actions)
        self.assertIn("shortcut_actions_snapshot", shortcut_actions)

    def test_legacy_layer_list_uses_lua_for_api_157(self):
        source = CONTROL.read_text()
        header = HEADER.read_text()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua = LUA.read_text()
        dispatcher = function_body(source, "static char* handle_request(", "static void process_buffer(")
        layer_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "layer.list") && client->api_minor >= 57) {',
            'if (g_str_equal(method, "privacy.state")) {',
        )
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertEqual(api_minor(header), 57)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=57", cmake)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', layer_list)
        self.assertIn('"layer.list"', read_api)
        self.assertIn("legacy_layer_list_from_lua", read_api)
        self.assertIn('"surfaces"', lua)


if __name__ == "__main__":
    unittest.main()
