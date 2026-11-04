#!/usr/bin/env python3
"""Guard native compositor socket API contracts."""

from pathlib import Path
import re
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _sources import (  # noqa: E402
    control_header,
    control_source,
    native_dispatch_source,
    overlay_manifests,
    LUA_SOURCES,
    RUNTIME_SOURCES,
)

ROOT = Path(__file__).resolve().parents[1]
LUA = LUA_SOURCES
SESSION = RUNTIME_SOURCES
CLI_FILES = sorted((ROOT / "src/cli").glob("gnoblinctl*.c"))
EVENT_DOCS = "\n".join(path.read_text() for path in sorted((ROOT / "docs/events").glob("*.md")))
BRIDGE_DOCS = "\n".join(path.read_text() for path in sorted((ROOT / "docs/bridge").glob("*.md")))


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
    def test_configured_shortcut_input_capture_uses_dynamic_bare_super_path(self):
        source = control_source()
        capture_start = source.index("gboolean apply_configured_capture_shortcut(")
        apply_capture = source[capture_start : source.index("void dispatch_dynamic_shortcut_activated(", capture_start)]
        start_shortcuts = function_body(
            source,
            "gboolean start_native_shortcuts(",
            "typedef enum {",
        )
        settings_changed = function_body(
            source,
            "void native_settings_changed(",
            "static gboolean native_touchpad_action_supported(",
        )
        lua_owner = function_body(
            source,
            "static gboolean dynamic_shortcut_routes_to_lua_runtime(",
            "void dynamic_shortcut_publish_event(",
        )
        publish_event = function_body(
            source,
            "void dynamic_shortcut_publish_event(",
            "gboolean issue_focus_context_for_session_key(",
        )
        focus_context = function_body(
            source,
            "gboolean issue_focus_context_for_session_key(",
            "void dynamic_shortcut_end_session(",
        )

        self.assertIn('g_str_equal(binding, "Super")', apply_capture)
        self.assertIn('g_str_equal(trigger, "release")', apply_capture)
        self.assertIn("command || action", apply_capture)
        self.assertIn("capture_count > 1", apply_capture)
        self.assertIn("if (has_overlay || has_capture)", start_shortcuts)
        self.assertIn("arm_bare_super_shortcut", apply_capture)
        self.assertIn('g_str_has_prefix(shortcut->owner_id, "config:")', source)
        self.assertIn('g_str_has_prefix(shortcut->owner_id, "lua:")', lua_owner)
        self.assertIn('g_str_has_prefix(shortcut->owner_id, "config:")', lua_owner)
        self.assertIn("dynamic_shortcut_routes_to_lua_runtime(shortcut)", publish_event)
        self.assertIn("dynamic_shortcut_routes_to_lua_runtime(shortcut)", focus_context)
        self.assertIn("apply_configured_capture_shortcut(control, document, error)", start_shortcuts)
        self.assertIn('clear_configured_capture_shortcut(control, "config_changed")', settings_changed)
        self.assertIn("apply_configured_capture_shortcut(control, config, &capture_error)", settings_changed)

    def test_cmake_api_minor_is_derived_from_control_header(self):
        cmake = (ROOT / "CMakeLists.txt").read_text()
        self.assertIn('REGEX "^#define GNOBLIN_NATIVE_CONTROL_API_MINOR [0-9]+$"', cmake)
        self.assertIn('GNOBLIN_NATIVE_CONTROL_API_MINOR "${GNOBLIN_NATIVE_CONTROL_API_MINOR_LINE}"', cmake)

    def test_orientation_lock_snapshot_name_matches_runtime_worker(self):
        source = control_source()
        session = SESSION.read_text()
        self.assertIn('g_str_equal(name, "input-orientation-lock")', session)
        self.assertEqual(source.count('native_publish_runtime_snapshot(control, "input-orientation-lock"'), 2)
        self.assertNotIn('native_publish_runtime_snapshot(control, "orientation-lock"', source)

    def test_window_socket_events_keep_legacy_field_aliases(self):
        source = control_source()
        aliases = function_body(
            source,
            "static void native_window_event_add_compat_aliases(",
            "void publish_native_socket_event(",
        )
        publish = function_body(
            source,
            "void publish_native_socket_event(GnoblinNativeControl* control, JsonNode* payload) {",
            "void native_publish_request_event(",
        )

        self.assertIn('{"window", "last"}', aliases)
        self.assertIn("native_window_record_add_public_aliases", aliases)
        self.assertIn("window_property_names[property].lua_name", aliases)
        self.assertIn("window_property_names[property].native_name", aliases)
        self.assertIn('g_str_has_prefix(name, "gnoblin.window.")', publish)
        self.assertIn("native_window_event_add_compat_aliases(socket_object)", publish)

    def test_layer_event_baseline_is_seeded_before_startup_returns(self):
        source = control_source()
        startup = function_body(
            source,
            "GnoblinNativeControl* gnoblin_native_control_start(",
            "void gnoblin_native_control_stop(",
        )
        publisher = function_body(
            source,
            "void publish_layer_changes(",
            "void publish_window_changes(",
        )

        snapshot = startup.index("layer_snapshot_json(control, TRUE, &layer_error)")
        layer_state = startup.index("layer_state_from_snapshot(initial_layer_snapshot)")
        initialized = startup.index("control->layer_state_initialized = TRUE")
        returned = startup.index("return control;")
        self.assertLess(snapshot, layer_state)
        self.assertLess(layer_state, initialized)
        self.assertLess(initialized, returned)
        self.assertIn("if (control->layer_state_initialized)", publisher)

    def test_capture_capabilities_and_camera_privacy_reach_lua(self):
        source = control_source()
        header = control_header()
        callback = function_body(
            source,
            "void privacy_pipewire_state_changed(",
            "static void privacy_refresh_state(",
        )
        monitor = (ROOT / "src/agents/gnoblin-pipewire-monitor.c").read_text()
        capability = function_body(
            source,
            "GVariant* capability_snapshot_record(",
            "GVariant* capability_snapshot(",
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
        self.assertIn("native_capability_by_id(changed_capabilities[i])", callback)
        self.assertIn('"camera-monitor"', source)
        self.assertIn('g_str_equal(native_capability->id, "camera-monitor")', capability)
        self.assertIn("PW_KEY_MEDIA_ROLE", monitor)
        self.assertIn('g_str_equal(media_role, "Camera")', monitor)
        self.assertIn("g_timeout_add(500, privacy_camera_disable, control)", source)
        self.assertNotIn("G_N_ELEMENTS(native_capabilities) - 1", callback)
        self.assertLess(
            callback.index('native_publish_runtime_snapshot(control, "capabilities"'),
            callback.index('native_publish_request_event(control, "gnoblin.capability.changed"'),
        )
        self.assertIn('native_publish_request_event(control, "gnoblin.capability.changed"', callback)

    def test_ping_is_a_transport_operation_not_an_api_method(self):
        source = control_source()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        ping = function_body(
            dispatcher,
            'if (g_str_equal(op, "ping")) {',
            'if (json_object_has_member(request, "api_version")',
        )

        self.assertIn('"ping accepts only op and id"', ping)
        self.assertIn('json_object_set_string_member(pong, "pong", "pong")', ping)
        self.assertNotIn("shell.ping", dispatcher)

    def test_stopping_control_rejects_new_requests_but_keeps_ping_available(self):
        source = control_source()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        ping = dispatcher.index('if (g_str_equal(op, "ping"))')
        stopping = dispatcher.index("if (client->control && client->control->stopping)")
        version_handling = dispatcher.index('if (json_object_has_member(request, "api_version")')

        self.assertLess(ping, stopping)
        self.assertLess(stopping, version_handling)
        self.assertIn('"Gnoblin compositor control is stopping"', dispatcher[stopping:version_handling])

    def test_runtime_abort_and_stop_drain_deferred_requests(self):
        source = control_source()
        abort = function_body(source, "void native_runtime_abort(", "gboolean native_runtime_send(")
        drain_start = source.rindex("void native_runtime_fail_pending_requests(")
        drain_end = source.index("static gboolean native_runtime_send_worker_suspended(", drain_start)
        drain = source[drain_start:drain_end]
        operation = function_body(
            source,
            "static gboolean native_runtime_handle_operation(GnoblinNativeControl* control,",
            "gboolean native_runtime_fd_ready(",
        )
        fd_ready = function_body(
            source,
            "gboolean native_runtime_fd_ready(gint fd, GIOCondition condition, gpointer user_data) {",
            "gboolean gnoblin_native_control_dispatch_runtime_event(",
        )
        stop = function_body(
            source,
            "void gnoblin_native_control_stop(",
            "native_control_finish_teardown(control);",
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

    def test_wm_menu_authority_is_api_130_typed_and_target_bound(self):
        source = control_source()
        header = control_header()
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
            "GVariant* native_socket_begin_menu_window_grab(",
            "void native_launch_free(",
        )
        native_action = function_body(
            source,
            "GVariant* gnoblin_native_control_begin_menu_window_grab(",
            "void gnoblin_native_control_revoke_focus_contexts(",
        )

        self.assertGreaterEqual(api_minor(header), 38)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
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

    def test_menu_authority_uses_subscription_version_not_latest_request_version(self):
        source = control_source()
        menu_emit = function_body(
            source,
            "void gnoblin_native_control_window_menu_requested(",
            "#define NATIVE_DRAG_MOD_CONTROL",
        )
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")

        self.assertIn("client->event_api_minor = client->api_minor", dispatcher)
        self.assertIn("client->event_api_minor >= 30", menu_emit)
        self.assertNotIn("client->api_minor >= 30", menu_emit)

    def test_rejected_api_versions_consume_matching_menu_context(self):
        source = control_source()
        consumer = function_body(
            source,
            "void native_socket_consume_rejected_menu_context(",
            "char* handle_request(",
        )
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
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
        source = control_source()
        revoke = function_body(
            source,
            "void revoke_menu_contexts(GnoblinNativeControl* control) {",
            "guint64 native_menu_context_create(",
        )
        socket_revoke = function_body(
            source,
            "void native_socket_revoke_client_tokens(Client* client) {",
            "void prune_focus_contexts(",
        )
        self.assertIn("g_hash_table_remove_all(control->menu_contexts)", revoke)
        self.assertIn("g_hash_table_remove_all(client->menu_grants)", revoke)
        self.assertIn("native_menu_context_revoke_client(control, client->client_id)", socket_revoke)
        self.assertIn("revoke_menu_contexts(control)", source)

    def test_workspace_list_always_uses_lua_snapshot_and_preserves_legacy_shape(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
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

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', workspace_list)
        self.assertIn("if (!client->control->supervised_runtime)", workspace_list)
        self.assertNotIn("meta_gnoblin_dispatch_native_api", workspace_list)
        self.assertIn('"workspace.list"', read_api)
        self.assertIn("legacy_workspace_list_from_lua(value, error)", read_api)

    def test_window_list_always_uses_lua_snapshot_and_preserves_legacy_shape(self):
        source = control_source()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
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

        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', window_list)
        self.assertIn("if (!client->control->supervised_runtime)", window_list)
        self.assertNotIn("meta_gnoblin_dispatch_native_api", window_list)
        self.assertIn('"window.list"', read_api)
        self.assertIn("legacy_window_list_from_lua(value, error)", read_api)

    def test_expired_snap_contexts_and_targets_are_pruned_and_bounded(self):
        source = control_source()
        pruner = function_body(
            source,
            "void prune_focus_contexts(GnoblinNativeControl* control, gint64 now) {",
            "gboolean focus_context_expiry_tick(",
        )
        text_target = function_body(
            source,
            "GVariant* gnoblin_native_control_create_text_target(",
            "GVariant* native_insert_text_owned(",
        )
        snap_context = function_body(
            source,
            "GVariant* gnoblin_native_control_create_snap_context(",
            "GVariant* native_socket_create_snap_context(",
        )

        self.assertIn("control->snap_contexts", pruner)
        self.assertIn("context->expires_at_us <= now", pruner)
        self.assertIn("MAX_TEXT_TARGETS", text_target)
        self.assertIn("MAX_SNAP_CONTEXTS", snap_context)

    def test_text_target_requires_the_connection_focus_grant_and_takes_only_its_scope(self):
        source = control_source()
        take_grant = function_body(
            source,
            "gboolean native_socket_take_focus_grant(",
            "GVariant* native_socket_focus_window(",
        )
        create = function_body(
            source,
            "GVariant* native_socket_create_text_target(",
            "GVariant* native_socket_insert_text(",
        )
        text_target = function_body(
            source,
            "GVariant* gnoblin_native_control_create_text_target(",
            "GVariant* native_insert_text_owned(",
        )

        # A token survives a use, so one gesture can take each operation once; the connection that
        # used it keeps the gesture and every other connection loses its token for it.
        self.assertNotIn("g_hash_table_remove(client->focus_grants", take_grant)
        self.assertIn("revoke_other_focus_grants_for_handle", take_grant)
        self.assertLess(
            create.index("native_socket_take_focus_grant"), create.index("gnoblin_native_control_create_text_target")
        )
        self.assertIn("target->socket_owner_client_id = client->client_id", create)
        self.assertIn("FOCUS_SCOPE_TEXT_TARGET", text_target)

    def test_every_gesture_operation_takes_its_own_scope_once(self):
        source = control_source()
        claim = function_body(
            source,
            "NativeFocusClaim native_focus_context_claim(",
            "gboolean native_grant_window_allows(",
        )
        self.assertIn("stored->used_scopes & scope", claim)
        self.assertIn("FOCUS_SCOPE_ALL", claim)
        for scope, signature, end in (
            (
                "FOCUS_SCOPE_FOCUS",
                "GVariant* gnoblin_native_control_focus_window(",
                "gboolean native_text_modifiers_allowed(",
            ),
            (
                "FOCUS_SCOPE_SNAP_CONTEXT",
                "GVariant* gnoblin_native_control_create_snap_context(",
                "GVariant* native_socket_create_snap_context(",
            ),
            (
                "FOCUS_SCOPE_BEGIN_MOVE",
                "GVariant* gnoblin_native_control_begin_window_grab(",
                "GVariant* gnoblin_native_control_begin_menu_window_grab(",
            ),
        ):
            body = function_body(source, signature, end)
            self.assertIn(scope, body)
            self.assertIn("native_focus_context_claim", body)
            self.assertIn("native_grant_window_allows", body)
            self.assertNotIn("g_hash_table_remove(control->focus_contexts", body)
        grab = function_body(
            source,
            "GVariant* gnoblin_native_control_begin_window_grab(",
            "GVariant* gnoblin_native_control_begin_menu_window_grab(",
        )
        self.assertIn("FOCUS_SCOPE_BEGIN_RESIZE", grab)
        self.assertIn("meta_wayland_session_lock_is_active", grab)

    def test_text_target_owner_check_precedes_consumption_and_invalid_fields_consume(self):
        source = control_source()
        insert = function_body(
            source,
            "GVariant* native_socket_insert_text(",
            "gboolean focus_identity_matches(",
        )

        owner_check = insert.index("stored->socket_owner_client_id != client->client_id")
        first_remove = insert.index("g_hash_table_remove(client->control->text_targets, token)")
        validation = insert.index("native_socket_has_exact_fields")
        self.assertLess(owner_check, first_remove)
        self.assertLess(first_remove, validation)
        self.assertIn("native_socket_has_nul_escape(data, length)", source)

        insert_native = function_body(
            source,
            "GVariant* native_insert_text_owned(",
            "GVariant* gnoblin_native_control_insert_text(",
        )
        self.assertIn("stored->socket_owner_client_id != socket_owner_client_id", insert_native)
        self.assertLess(
            insert_native.index("stored->socket_owner_client_id != socket_owner_client_id"),
            insert_native.index("g_hash_table_remove(control->text_targets, token)"),
        )

    def test_snap_context_is_owner_bound_and_consumed_before_argument_validation(self):
        source = control_source()
        commit = function_body(
            source,
            "static GVariant* native_commit_snap_context_owned(",
            "GVariant* gnoblin_native_control_commit_snap_context(",
        )
        socket_context = function_body(
            source,
            "GVariant* native_socket_create_snap_context(",
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
            "GVariant* native_socket_commit_snap_context(",
            "GVariant* gnoblin_native_control_create_text_target(",
        )
        self.assertIn("native_socket_snap_rect(frame_node)", socket_snap)
        self.assertRegex(socket_snap, r"client->client_id,\s*error")
        self.assertIn("revoke_focus_contexts(control)", source)

    def test_owner_tokens_are_revoked_on_disconnect_subscription_reset_and_global_revocation(self):
        source = control_source()
        revoke = function_body(
            source,
            "void native_socket_revoke_client_tokens(Client* client) {",
            "void prune_focus_contexts(",
        )
        client_close = function_body(source, "void client_close(", "JsonNode* json_from_variant(")
        subscription = function_body(source, 'if (g_str_equal(op, "events"))', 'if (g_str_equal(op, "windows"))')
        focus_revoke = function_body(
            source,
            "void revoke_focus_contexts(GnoblinNativeControl* control) {",
            "void revoke_text_targets(",
        )

        self.assertIn("control->text_targets", revoke)
        self.assertIn("control->snap_contexts", revoke)
        self.assertIn("native_socket_revoke_client_tokens(client)", client_close)
        self.assertIn("native_socket_revoke_client_tokens(client)", subscription)
        self.assertIn("g_hash_table_remove_all(control->snap_contexts)", focus_revoke)
        self.assertIn("revoke_text_targets(control)", focus_revoke)

    def test_legacy_window_actions_use_typed_lua_operations_for_all_api_versions(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        overlay_manifest = overlay_manifests()
        lua = LUA.read_text()
        mutter_patch = native_dispatch_source()
        handler = function_body(
            source,
            'if (g_str_equal(method, "window.action")) {',
            'if (g_str_equal(method, "window.match")) {',
        )

        self.assertGreaterEqual(api_minor(header), 60)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn(
            "mutter gnoblin-native-control.c src/core/gnoblin-native-control.c",
            overlay_manifest,
        )
        self.assertIn('g_str_equal(json_node_get_string(action_node), "focus")', handler)
        self.assertIn("arguments_node && JSON_NODE_HOLDS_OBJECT(arguments_node)", handler)
        self.assertIn('const char* action_fields[] = {"action", "window"}', handler)
        self.assertIn('"unsupported window.action; use a typed window operation when available"', handler)
        self.assertNotIn("client->api_minor >= 60", handler)
        self.assertNotIn("meta_gnoblin_dispatch_native_api(", handler)
        self.assertIn('const char* target = window_node ? json_node_get_string(window_node) : "active"', handler)
        self.assertIn("native_window_id(active_window)", handler)
        for lua_method in (
            '"window.set_above"',
            '"window.set_sticky"',
            '"window.set_maximized"',
            '"window.set_fullscreen"',
            '"window.close"',
            '"window.minimize"',
            '"window.unminimize"',
        ):
            with self.subTest(lua_method=lua_method):
                self.assertIn(lua_method, handler)
        self.assertIn(
            "queue_runtime_api_request_internal(client, id, lua_method, operation_arguments,",
            handler,
        )
        self.assertIn('"call", action, target)', handler)
        self.assertIn('"window.unminimize"', lua)
        self.assertIn('"window.unminimize"', mutter_patch)
        self.assertIn("return queue_runtime_api_request_internal(client, id, lua_method, operation_arguments,", handler)

        request_queue = function_body(
            source,
            "char* queue_runtime_api_request_internal(",
            "char* queue_runtime_api_request(",
        )
        self.assertIn("pending->legacy_window_action = g_strdup(legacy_window_action)", request_queue)
        self.assertIn("pending->legacy_window_id = g_strdup(legacy_window_id)", request_queue)

        response = function_body(
            source,
            "else if (packet.type == GNOBLIN_RUNTIME_PACKET_API_RESPONSE && packet.request_id > 0)",
            "else if (packet.type == GNOBLIN_RUNTIME_PACKET_CONFIG)",
        )
        for field in ('"ok"', '"pending"', '"window"', '"action"'):
            with self.subTest(response_field=field):
                self.assertIn(field, response)
        self.assertIn('"Lua returned an invalid window action operation"', response)

    def test_legacy_window_resize_uses_typed_lua_operation_for_api_161(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        resize_patch = native_dispatch_source()
        handler = function_body(
            source,
            'if (g_str_equal(method, "window.action")) {',
            'if (g_str_equal(method, "window.match")) {',
        )

        self.assertGreaterEqual(api_minor(header), 61)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('gboolean resize_action = g_str_equal(action, "resize")', handler)
        self.assertIn("client->api_minor >= 61", handler)
        self.assertIn('lua_method = "window.resize"', handler)
        self.assertIn("g_variant_new_int64(json_node_get_int(width_node))", handler)
        self.assertIn("g_variant_new_int64(json_node_get_int(height_node))", handler)
        self.assertIn('"window.action resize requires integer width and height"', handler)
        self.assertIn("window.action resize requires integer width and height", handler)
        self.assertIn('"call", action, target)', handler)
        self.assertIn(
            "meta_window_move_resize_frame(window, TRUE, frame.x, frame.y,",
            resize_patch,
        )

    def test_legacy_window_move_uses_typed_lua_operation_for_api_162(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        move_patch = native_dispatch_source()
        handler = function_body(
            source,
            'if (g_str_equal(method, "window.action")) {',
            'if (g_str_equal(method, "window.match")) {',
        )

        self.assertGreaterEqual(api_minor(header), 62)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('gboolean move_action = g_str_equal(action, "move")', handler)
        self.assertIn("client->api_minor >= 62", handler)
        self.assertIn('lua_method = "window.move"', handler)
        self.assertIn("g_variant_new_int64(json_node_get_int(x_node))", handler)
        self.assertIn("g_variant_new_int64(json_node_get_int(y_node))", handler)
        self.assertIn('"window.action move requires integer x and y"', handler)
        self.assertIn("meta_window_is_fullscreen(window)", move_patch)
        self.assertIn("meta_window_get_maximize_flags(window) != META_MAXIMIZE_NONE", move_patch)
        self.assertIn("meta_window_move_frame(window, TRUE, (int)x, (int)y)", move_patch)

    def test_legacy_workspace_and_monitor_actions_use_typed_lua_operations(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        handler = function_body(
            source,
            'if (g_str_equal(method, "window.action")) {',
            'if (g_str_equal(method, "window.match")) {',
        )

        self.assertGreaterEqual(api_minor(header), 63)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('gboolean workspace_action = g_str_equal(action, "workspace")', handler)
        self.assertIn('gboolean monitor_action = g_str_equal(action, "monitor")', handler)
        self.assertIn("client->api_minor < 63", handler)
        self.assertIn('lua_method = "window.move_to_workspace"', handler)
        self.assertIn('lua_method = "window.move_to_monitor"', handler)
        self.assertIn("native_monitor_id_for_index(", handler)
        self.assertIn('"workspace"', handler)
        self.assertIn('"monitor"', handler)
        self.assertIn('"call", action, target)', handler)

    def test_modal_key_events_refresh_focus_authority_from_real_input(self):
        source = control_source()
        header = control_header()
        capture_start = source.rindex("void native_shortcut_capture_key(")
        capture_end = source.index("gboolean gnoblin_native_control_overlay_modifier_pressed(", capture_start)
        capture = source[capture_start:capture_end]
        issue_context = function_body(
            source,
            "gboolean issue_focus_context_for_session_key(",
            "void dynamic_shortcut_end_session(",
        )
        runtime_event = function_body(
            source,
            "gboolean native_runtime_dispatch_event(",
            "static GVariant* native_runtime_shortcut_operation(",
        )
        runtime_handler = function_body(
            SESSION.read_text(),
            "gboolean handle_event(",
            "static gboolean dispatch_parent_event(",
        )
        lua_dispatch = function_body(
            LUA.read_text(),
            "void push_event_table(",
            "static GVariant* gnoblin_config_dispatch_event_internal(",
        )

        self.assertIn("issue_focus_context_for_session_key", capture)
        self.assertIn("CLUTTER_EVENT_FLAG_REPEATED", capture)
        self.assertIn("CLUTTER_EVENT_FLAG_SYNTHETIC", issue_context)
        self.assertIn("CLUTTER_EVENT_FLAG_INPUT_METHOD", issue_context)
        self.assertIn("CLUTTER_EVENT_FLAG_REPEATED", issue_context)
        self.assertGreaterEqual(api_minor(header), 68)
        self.assertIn("client->api_minor < 68", issue_context)
        self.assertIn("g_hash_table_insert(client->focus_grants, g_strdup(token), grant)", issue_context)
        self.assertIn('"focus_context"', capture)
        self.assertIn("g_variant_new_string(focus_token)", capture)
        self.assertIn('g_str_equal(event, "gnoblin.shortcut.session.key")', runtime_event)
        self.assertIn('g_str_equal(field_name, "focus_context_handle")', runtime_event)
        self.assertIn('g_str_equal(event, "gnoblin.shortcut.session.key")', runtime_handler)
        self.assertIn('g_str_equal(event, "gnoblin.shortcut.session.key")', lua_dispatch)
        self.assertIn("grant_event && active_runtime->config.dispatch_focus_handle", lua_dispatch)

    def test_input_snapshots_always_use_lua(self):
        source = control_source()
        header = control_header()
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

        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', devices)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', sources)
        self.assertLess(sources.index("publish_input_source_changes"), sources.index("queue_runtime_api_request"))
        self.assertNotIn("input_device_snapshot(client->control)", devices)
        self.assertNotIn("input_source_snapshot(client->control)", sources)
        self.assertNotIn("client->api_minor >= 46", devices + sources)
        self.assertIn('g_str_equal(method, "input.devices") ? "devices" : "sources"', lua_source)
        self.assertIn('g_str_equal(method, "input.current_source") && lua_isnil(state, -1)', lua_source)
        self.assertIn("gnoblin_config_update_input_device_snapshot", lua_source)
        self.assertIn("gnoblin_config_update_input_source_snapshot", lua_source)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertGreaterEqual(api_minor(header), 46)

    def test_privacy_state_always_uses_lua(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua_source = LUA.read_text()
        handler = function_body(
            source,
            'if (g_str_equal(method, "privacy.state")) {',
            'if (g_str_equal(method, "window.action")) {',
        )
        reads = function_body(
            source,
            "gboolean native_api_read_method(",
            "gboolean runtime_config_values_equal(",
        )
        lua_read = function_body(
            lua_source,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertNotIn("client->api_minor >= 47", handler)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', handler)
        self.assertNotIn("client->control->privacy_snapshot", handler)
        self.assertNotIn('g_str_equal(method, "privacy.state")', reads)
        self.assertIn('"privacy.state"', lua_read)
        self.assertIn('g_str_equal(method, "privacy.state")', lua_read)
        self.assertIn('lua_getfield(state, -1, "state")', lua_read)
        self.assertGreaterEqual(api_minor(header), 47)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)

    def test_launch_status_always_uses_lua_and_keeps_event_tracking(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
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
        status_read = function_body(
            launch,
            'if (g_str_equal(method, "launch.status")) {',
            "if (client->api_minor >= 50 &&",
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('g_str_equal(method, "launch.status")', status_read)
        self.assertNotIn("client->api_minor", status_read)
        self.assertLess(
            launch.index("client->track_launches = TRUE"),
            launch.index('g_str_equal(method, "launch.status")'),
        )
        self.assertIn('queue_runtime_api_request(client, id, method, launch_arguments, "read")', launch)
        self.assertNotIn("gnoblin_native_control_dispatch_launch", status_read)
        self.assertIn("gnoblin_native_control_dispatch_launch", launch)
        self.assertIn('"launch.status"', read_api)
        self.assertIn('g_str_equal(method, "launches.snapshot") || g_str_equal(method, "launch.status")', read_api)

    def test_legacy_shortcut_list_always_uses_lua(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        shortcut_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "shortcut.list")) {',
            'if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {',
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertNotIn("client->api_minor >= 55", shortcut_list)
        self.assertIn("client->control->supervised_runtime", shortcut_list)
        self.assertIn('queue_runtime_api_request(client, id, "shortcuts.list", read_arguments, "read")', shortcut_list)
        self.assertNotIn("native_shortcut_snapshot", shortcut_list)

    def test_legacy_shortcut_actions_always_use_lua(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        shortcut_actions = function_body(
            dispatcher,
            'if (g_str_equal(method, "shortcut.actions")) {',
            "    GVariantBuilder empty;\n    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);\n    g_autoptr(GVariant) arguments",
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertNotIn("client->api_minor >= 56", shortcut_actions)
        self.assertIn("client->control->supervised_runtime", shortcut_actions)
        self.assertIn('queue_runtime_api_request(client, id, "shortcuts.actions", read_arguments,', shortcut_actions)
        self.assertNotIn("shortcut_actions_snapshot", shortcut_actions)

    def test_legacy_layer_list_always_uses_lua(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua = LUA.read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        layer_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "layer.list")) {',
            'if (g_str_equal(method, "privacy.state")) {',
        )
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertNotIn("client->api_minor >= 57", layer_list)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', layer_list)
        self.assertIn('"layer.list"', read_api)
        self.assertIn("legacy_layer_list_from_lua", read_api)
        self.assertIn('"surfaces"', lua)

    def test_legacy_monitor_list_always_uses_lua(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua = LUA.read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        monitor_list = function_body(
            dispatcher,
            'if (g_str_equal(method, "monitor.list")) {',
            'if (g_str_equal(method, "privacy.state")) {',
        )
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertNotIn("client->api_minor >= 58", monitor_list)
        self.assertIn('queue_runtime_api_request(client, id, method, read_arguments, "read")', monitor_list)
        self.assertIn('g_str_equal(method, "monitor.list")', read_api)
        self.assertIn("Lua monitor record has no integer index", read_api)
        self.assertIn('"monitors"', lua)

    def test_legacy_window_match_always_uses_lua_and_preserves_compatibility(self):
        source = control_source()
        header = control_header()
        cmake = (ROOT / "CMakeLists.txt").read_text()
        lua = LUA.read_text()
        dispatcher = function_body(source, "char* handle_request(", "void process_buffer(")
        window_match = function_body(
            dispatcher,
            'if (g_str_equal(method, "window.match")) {',
            'if (g_str_equal(method, "window.focus")) {',
        )
        read_api = function_body(
            lua,
            "GVariant* gnoblin_config_read_api(",
            "GVariant* gnoblin_config_current_document(",
        )

        self.assertGreaterEqual(api_minor(header), 59)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", cmake)
        self.assertIn('queue_runtime_api_request(client, id, method, arguments, "read")', window_match)
        self.assertNotIn("meta_gnoblin_dispatch_native_api", window_match)
        self.assertIn('g_str_equal(method, "window.match")', read_api)
        self.assertIn("legacy_window_match_arguments_valid", read_api)
        self.assertIn("legacy_window_match_from_lua", read_api)


if __name__ == "__main__":
    unittest.main()
