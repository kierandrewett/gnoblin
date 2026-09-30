#!/usr/bin/env python3
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTROL = (ROOT / "src/native-control/gnoblin-native-control.c").read_text()


class PermissionContractTest(unittest.TestCase):
    def test_socket_decision_uses_public_string_arrays_and_revision(self):
        start = CONTROL.index("static JsonNode* permission_decision_json(")
        end = CONTROL.index("static gboolean native_api_read_method(", start)
        implementation = CONTROL[start:end]

        self.assertIn('json_array_add_string_element(devices, "keyboard")', implementation)
        self.assertIn('json_array_add_string_element(devices, "pointer")', implementation)
        self.assertIn('json_array_add_string_element(devices, "touchscreen")', implementation)
        self.assertIn('json_object_set_array_member(object, "devices", devices)', implementation)
        self.assertIn('json_object_set_int_member(object, "revision", (gint64)revision)', implementation)
        self.assertNotIn('json_object_set_int_member(object, "devices"', implementation)

    def test_native_privacy_reports_only_observable_microphone_state(self):
        start = CONTROL.index("static GVariant* privacy_snapshot_new(")
        end = CONTROL.index("static void publish_privacy_snapshot(", start)
        implementation = CONTROL[start:end]

        self.assertIn('"screen_sharing"', implementation)
        self.assertIn('"microphone_in_use"', implementation)
        self.assertIn("if (control->privacy_microphone_available)", implementation)
        for unsupported in ("camera_in_use", "location_in_use"):
            self.assertIn(
                f'g_variant_builder_add(&available, "{{sv}}", "{unsupported}"',
                implementation,
            )
            self.assertNotIn(
                f'g_variant_builder_add(&snapshot, "{{sv}}", "{unsupported}"',
                implementation,
            )

    def test_window_rules_accept_public_lua_workspace_field_names(self):
        start = CONTROL.index("static gboolean native_window_rule_matches(")
        end = CONTROL.index("static void native_apply_window_rules(", start)
        implementation = CONTROL[start:end]

        for field in ("workspace_id", "workspace-id", "workspace_number", "workspace-number"):
            self.assertIn(f'g_str_equal(key, "{field}")', implementation)

    def test_corner_rule_booleans_accept_public_lua_field_names(self):
        matcher = CONTROL.index("static gboolean native_window_rule_matches(")
        start = CONTROL.index("static void native_apply_window_rules(", matcher)
        end = CONTROL.index("static void native_apply_all_window_rules(", start)
        implementation = CONTROL[start:end]

        for field in (
            "keep_maximized",
            "keep_fullscreen",
            "keep_tiled",
            "skip_libadwaita",
            "skip_libhandy",
            "remove_csd",
        ):
            self.assertIn(f'native_rule_get_boolean(corners, "{field}"', implementation)


if __name__ == "__main__":
    unittest.main()
