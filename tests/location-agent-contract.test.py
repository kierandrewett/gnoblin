"""Contract checks for Gnoblin's native GeoClue2 authorization agent."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
CONTROL = ROOT / "src/native-control/gnoblin-native-control.c"
API_HEADER = ROOT / "src/native-control/gnoblin-native-control.h"
BUILD = ROOT / "CMakeLists.txt"
LUA = ROOT / "src/config/gnoblin-lua.c"


class LocationAgentContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = (ROOT / "src/native-control/gnoblin-location-agent.c").read_text()
        cls.header = (ROOT / "src/native-control/gnoblin-location-agent.h").read_text()
        cls.control = CONTROL.read_text()
        cls.lua = LUA.read_text()

    def test_agent_exports_expected_geo_clue_contract(self):
        self.assertIn('"/org/freedesktop/GeoClue2/Agent"', self.source)
        self.assertIn("<method name='AuthorizeApp'>", self.source)
        self.assertIn("<property name='MaxAccuracyLevel' type='u' access='read'/>", self.source)
        self.assertIn('g_variant_new("(s)", "gnoblin")', self.source)
        self.assertIn('"AddAgent"', self.source)

    def test_accuracy_setting_is_mapped_to_geo_clue_levels(self):
        self.assertRegex(self.source, r"g_settings_schema_source_lookup\(source, \"org\.gnome\.system\.location\"")
        accuracy_helper = self.source.split("static guint accuracy_from_setting", 1)[1].split(
            "static guint clamp_accuracy", 1
        )[0]
        self.assertIn('!g_settings_get_boolean(settings, "enabled")', accuracy_helper)
        self.assertIn('g_signal_connect(agent->settings, "changed"', self.source)
        for nick, level in (("country", 1), ("city", 4), ("neighborhood", 5), ("street", 6), ("exact", 8)):
            self.assertRegex(self.source, rf'(?s)g_strcmp0\(nick, "{nick}"\) == 0\).*?return {level};')
        self.assertIn("MIN(accuracy_level, 8)", self.source)
        self.assertIn("clamp_accuracy(accuracy_level", self.source)
        self.assertIn("guint requested_accuracy;", self.source)
        self.assertIn("MIN(request->requested_accuracy,", self.source)

    def test_async_authorization_fails_closed_and_is_bounded(self):
        self.assertIn("#define AUTHORIZATION_TIMEOUT_SECONDS 30", self.source)
        self.assertIn(".method_call = method_call", self.source)
        self.assertIn("!agent->available || !agent->authorize || !agent->settings", self.source)
        self.assertIn('g_settings_get_boolean(agent->settings, "enabled")', self.source)
        self.assertIn("if (!allowed) {\n        accuracy_level = 0;", self.source)
        self.assertIn("gnoblin_location_request_ref", self.header)
        self.assertIn("g_main_context_invoke_full(request->agent->context", self.source)
        self.assertIn("g_hash_table_remove(agent->pending, request)", self.source)

    def test_manager_lifecycle_and_in_use_are_observed(self):
        self.assertIn("g_bus_watch_name_on_connection", self.source)
        self.assertIn("manager_name_vanished", self.source)
        self.assertIn('"InUse"', self.source)
        self.assertIn("fail_all_pending(agent)", self.source)
        self.assertIn("notify_state(agent, FALSE, FALSE)", self.source)

    def test_overlay_and_meson_patch_register_the_module(self):
        manifest = (ROOT / "src/native-control/manifest").read_text()
        self.assertIn("mutter gnoblin-location-agent.c src/core/gnoblin-location-agent.c", manifest)
        self.assertIn("mutter gnoblin-location-agent.h src/core/gnoblin-location-agent.h", manifest)
        patch_path = ROOT / "patches/mutter/99-typed-window-api/0067-build-location-agent.patch"
        patch = patch_path.read_text()
        self.assertIn("From: kierandrewett <kieran@drewett.dev>", patch)
        self.assertIn("files('core/gnoblin-location-agent.c')", patch)

    def test_authorization_is_brokered_to_lua_with_bounded_one_use_requests(self):
        api_minor = next(
            int(line.rsplit(" ", 1)[1])
            for line in API_HEADER.read_text().splitlines()
            if line.startswith("#define GNOBLIN_NATIVE_CONTROL_API_MINOR ")
        )
        self.assertGreaterEqual(api_minor, 65)
        build = BUILD.read_text()
        self.assertIn("src/native-control/gnoblin-native-control.h", build)
        self.assertIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}", build)
        self.assertNotIn("GNOBLIN_NATIVE_CONTROL_API_MINOR=64", build)
        self.assertIn('"location.authorize_app"', self.lua)
        self.assertIn('"gnoblin.location.authorization-requested"', self.control)
        self.assertIn("MAX_PENDING_LOCATION_AUTHORIZATIONS 32", self.control)
        self.assertIn("LOCATION_AUTHORIZATION_TIMEOUT_SECONDS 25", self.control)
        self.assertIn("recipient_client_ids", self.control)
        self.assertIn("only a client that received the location request can answer it", self.control)
        self.assertIn("clear_pending_location_authorizations(control)", self.control)
        self.assertIn("native_location_authorize_operation(control, arguments, client_id", self.control)

    def test_shutdown_does_not_dispatch_callbacks_through_destroyed_control(self):
        stop = self.source.split("static gboolean begin_stop(", 1)[1].split("static void settings_changed(", 1)[0]
        self.assertIn("agent->stopped = TRUE;", stop)
        self.assertNotIn("notify_state(agent", stop)


if __name__ == "__main__":
    unittest.main()
