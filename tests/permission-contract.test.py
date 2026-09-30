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


if __name__ == "__main__":
    unittest.main()
