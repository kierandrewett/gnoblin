#!/usr/bin/env python3
"""Check ext-session-lock-v1 wiring and the compositor's fail-closed paths."""

from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src/protocols/session-lock/meta-wayland-session-lock.c"
SURFACE_SOURCE = ROOT / "src/protocols/session-lock/meta-wayland-session-lock-surface.c"
XML = ROOT / "src/protocols/session-lock/ext-session-lock-v1.xml"
MANIFEST = ROOT / "src/protocols/session-lock/manifest"
INPUT_PATCH_DIR = ROOT / "patches/mutter/72-session-lock-input"


class SessionLockProtocolTests(unittest.TestCase):
    def test_standard_protocol_contains_required_lock_objects(self):
        interfaces = {
            interface.attrib["name"]: interface
            for interface in ET.parse(XML).getroot().findall("interface")
        }
        self.assertEqual(
            set(interfaces),
            {
                "ext_session_lock_manager_v1",
                "ext_session_lock_v1",
                "ext_session_lock_surface_v1",
            },
        )
        self.assertIsNotNone(
            interfaces["ext_session_lock_v1"].find("request[@name='unlock_and_destroy']")
        )
        self.assertIsNotNone(
            interfaces["ext_session_lock_v1"].find("event[@name='locked']")
        )

    def test_manager_is_advertised_only_for_gnoblin_sessions(self):
        source = SOURCE.read_text()
        self.assertIn('gnoblin_config_protocol_enabled ("ext-session-lock")', source)
        self.assertIn("wl_global_create", source)
        self.assertIn("controller->global ? 1 : 0", source)
        self.assertIn("session_lock_manager_interface", source)

    def test_failsafe_keeps_cover_and_input_embargo_in_compositor(self):
        source = SOURCE.read_text()
        self.assertIn("CLUTTER_BIND_ALL", source)
        self.assertIn("meta_wayland_input_attach_event_handler", source)
        self.assertIn("CLUTTER_EVENT_STOP", source)
        self.assertIn("meta_wayland_touch_cancel", source)
        self.assertIn("META_WAYLAND_SESSION_LOCK_FAILSAFE", source)
        self.assertIn('"presented"', source)
        self.assertIn("clutter_stage_peek_stage_views", source)
        self.assertIn("unpresented_stage_views", source)
        self.assertIn("controller->scene", source)
        self.assertIn("meta_wayland_session_lock_is_active", source)
        self.assertIn("meta_wayland_session_lock_is_presentation_confirmed", source)
        self.assertIn("META_WAYLAND_SESSION_LOCK_FAILSAFE", source)
        self.assertIn("reset_presentation_barrier (controller)", source)
        self.assertIn("global_frame_counter", source)
        self.assertIn("frame_info->global_frame_counter <= *minimum_frame", source)
        self.assertIn("Submit another covered", source)
        self.assertIn("clutter_actor_queue_redraw (CLUTTER_ACTOR (controller->stage))", source)
        self.assertIn("disconnect_scene_signals (controller)", source)
        self.assertIn("controller->presentation_confirmed = FALSE", source)
        self.assertIn("controller->presentation_confirmed = TRUE", source)
        self.assertIn("before-paint", source)
        self.assertIn("clutter_actor_get_last_child", source)

    def test_lock_surface_has_a_real_shell_role_and_strict_buffer_gate(self):
        source = SURFACE_SOURCE.read_text()
        self.assertIn("META_TYPE_WAYLAND_SHELL_SURFACE", source)
        self.assertIn("EXT_SESSION_LOCK_SURFACE_V1_ERROR_COMMIT_BEFORE_FIRST_ACK", source)
        self.assertIn("EXT_SESSION_LOCK_SURFACE_V1_ERROR_DIMENSIONS_MISMATCH", source)
        self.assertIn("meta_wayland_surface_assign_role", source)
        self.assertIn("ext_session_lock_surface_v1_send_configure", source)
        self.assertIn("output-destroyed", source)
        self.assertIn("meta_wayland_session_lock_get_scene", source)
        self.assertIn("clutter_actor_remove_child", source)
        self.assertIn("clutter_actor_set_position", source)
        self.assertNotIn("meta_window_move_resize_frame", source)
        self.assertIn("not ready during its first", source)
        self.assertIn("window->placed = TRUE", source)
        self.assertIn("normal first-show toplevel placement", source)
        self.assertIn("pending->newly_attached && pending->buffer", source)
        self.assertIn("wl_client_post_no_memory", source)
        self.assertIn("shell_class->managed = managed", source)
        self.assertIn("shell_class->ping = ping", source)

    def test_manager_rejects_duplicate_outputs_and_precommitted_roles(self):
        source = SOURCE.read_text()
        self.assertIn("EXT_SESSION_LOCK_V1_ERROR_DUPLICATE_OUTPUT", source)
        self.assertIn("meta_wayland_surface_has_initial_commit", source)
        self.assertIn("meta_wayland_session_lock_surface_new", source)
        self.assertIn("wl_client_post_no_memory", source)
        self.assertIn("finished_lock_interface", source)
        self.assertIn("ext_session_lock_v1_send_finished", source)
        self.assertIn("a finished session lock cannot unlock", source)
        self.assertIn("META_WAYLAND_SESSION_LOCK_FAILSAFE", source)
        self.assertIn("close_lock_surfaces", source)
        self.assertIn("g_hash_table_remove_all (controller->surfaces)", source)
        self.assertIn("owner was already notified", source)

    def test_surface_role_is_installed_and_built(self):
        manifest = MANIFEST.read_text()
        self.assertIn("meta-wayland-session-lock-surface.c", manifest)
        self.assertIn("meta-wayland-session-lock-surface.h", manifest)

    def test_input_embargo_precedes_every_normal_session_consumer(self):
        header = (ROOT / "src/protocols/session-lock/meta-wayland-session-lock.h").read_text()
        source = SOURCE.read_text()
        patch = "\n".join(path.read_text() for path in sorted(INPUT_PATCH_DIR.glob("*.patch")))

        self.assertIn("meta_wayland_session_lock_filter_event", header)
        self.assertIn("meta_wayland_session_lock_filter_event", source)
        self.assertIn("meta_display_process_captured_input", patch)
        self.assertLess(
            patch.index("meta_wayland_session_lock_filter_event"),
            patch.index("meta_display_process_captured_input"),
        )
        self.assertIn("meta_wayland_seat_handle_session_lock_event", patch)
        self.assertIn("Normal-session handlers never see input", source)
        self.assertIn("CLUTTER_PICK_ALL", source)
        self.assertIn("meta_wayland_session_lock_surface_is_mapped", source)
        self.assertIn("meta_wayland_session_lock_surface_get_wayland_surface", source)
        self.assertIn("meta_wayland_touch_update_session_lock", patch)
        self.assertIn("meta_wayland_pointer_focus_session_lock_surface", patch)
        self.assertIn("meta_wayland_pointer_set_focus_full", patch)
        self.assertIn("allow_hidden_cursor", patch)
        self.assertIn("Never call meta_wayland_pointer_update() here", patch)


if __name__ == "__main__":
    unittest.main()
