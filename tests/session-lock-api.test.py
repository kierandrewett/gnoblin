#!/usr/bin/env python3
"""Guard the public session-lock request and compositor-state contract."""

from pathlib import Path
import re
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from _sources import control_header, control_source  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
LUA = ROOT / "src/config/gnoblin-lua.c"
DISPATCH_PATCH = ROOT / "patches/mutter/99-typed-window-api/0032-dispatch-shell-session-lock-request.patch"


class SessionLockApiTests(unittest.TestCase):
    def test_lua_operation_and_socket_version_are_registered(self):
        lua = LUA.read_text()
        control = control_source()
        header = control_header()

        self.assertIn('"session.lock",', lua)
        api_minor = re.search(r"GNOBLIN_NATIVE_CONTROL_API_MINOR (\d+)", header)
        self.assertIsNotNone(api_minor)
        self.assertGreaterEqual(int(api_minor.group(1)), 21)
        self.assertIn('"session.lock",', control)
        self.assertIn('g_str_equal(method, "session.lock") && client->api_minor < 21', control)

    def test_request_requires_available_compositor_and_subscribed_client(self):
        source = control_source()
        body = source.split("GVariant* gnoblin_native_control_request_session_lock(", 1)[1]
        body = body.split("static void input_source_keymap_set_done", 1)[0]

        self.assertIn("meta_wayland_session_lock_get_capability", body)
        self.assertIn("META_WAYLAND_SESSION_LOCK_UNLOCKED", body)
        self.assertIn('"gnoblin.session.lock-requested"', body)
        self.assertIn("client->api_minor >= 21 && client->event_api_minor >= 21", body)
        self.assertIn('"no session-lock client is subscribed to lock requests"', body)
        self.assertLess(body.index("if (listeners == 0)"), body.index("publish_native_socket_event"))
        self.assertIn('"dispatched", g_variant_new_boolean(TRUE)', body)
        self.assertIn('"subscribers", g_variant_new_uint32(listeners)', body)

    def test_lock_state_event_comes_from_mutter_state_callback(self):
        source = control_source()
        state_names = source.split("static const char* native_session_lock_state_name(", 1)[1]
        state_names = state_names.split("static GVariant* native_session_lock_snapshot", 1)[0]
        body = source.split("static void native_session_lock_changed(", 1)[1]
        body = body.split("GVariant* gnoblin_native_control_request_session_lock(", 1)[0]

        for enum, state in (
            ("UNLOCKED", "unlocked"),
            ("COVERING", "covering"),
            ("LOCKED", "locked"),
            ("FAILSAFE", "failsafe"),
        ):
            self.assertIn(f"META_WAYLAND_SESSION_LOCK_{enum}", state_names)
            self.assertIn(f'return "{state}";', state_names)
        self.assertIn("native_session_lock_state_name(state)", body)
        self.assertIn('"gnoblin.session.lock-state-changed"', body)
        self.assertRegex(
            body,
            r'g_variant_builder_add\(&payload_builder, "\{sv\}", "revision",\s*'
            r"g_variant_new_uint64\(sequence\)\);",
        )
        self.assertRegex(body, r'json_object_set_int_member\(object, "revision", sequence\);')
        self.assertRegex(
            body,
            r'native_publish_runtime_snapshot\(control, "session-lock", status_snapshot,\s*'
            r"sequence\);",
        )
        self.assertIn("publish_native_socket_event", body)
        self.assertIn("native_runtime_dispatch_event", body)
        self.assertNotIn("session.unlock", LUA.read_text())
        self.assertLess(body.index("publish_native_socket_event"), body.index("native_runtime_dispatch_event"))

    def test_mutter_dispatch_patch_uses_managed_export_identity(self):
        patch = DISPATCH_PATCH.read_text()
        self.assertIn("From: kierandrewett <kieran@drewett.dev>", patch)
        self.assertIn('method, "session.lock"', patch)
        self.assertIn("gnoblin_native_control_request_session_lock", patch)

    def test_docs_separate_delivery_from_compositor_confirmation(self):
        bridge = (ROOT / "docs/compositor-bridge.md").read_text()
        runtime = (ROOT / "docs/shell-api/session-runtime.md").read_text()
        events = (ROOT / "docs/config/lua-events.md").read_text()
        design = (ROOT / "design/lua-api.md").read_text()

        for document in (bridge, runtime, design):
            self.assertIn("dispatched", document)
            self.assertIn("subscribers", document)
            self.assertIn("does not", document)
        self.assertIn("gnoblin.session.lock-state-changed", bridge)
        self.assertIn("gnoblin.session.lock-state-changed", events)
        self.assertIn('"failsafe"', design)
        self.assertNotIn("session.unlock", design)


if __name__ == "__main__":
    unittest.main()
