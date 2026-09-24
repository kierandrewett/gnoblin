#!/usr/bin/env python3
"""Source-level contract for the compositor lock capture embargo."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCREENCOPY = ROOT / "src/protocols/screencopy/meta-wayland-screencopy.c"
SESSION_LOCK_HEADER = ROOT / "src/protocols/session-lock/meta-wayland-session-lock.h"
SESSION_LOCK_SOURCE = ROOT / "src/protocols/session-lock/meta-wayland-session-lock.c"
PATCH_DIR = ROOT / "patches/mutter/71-session-lock-capture"


class SessionLockCaptureTests(unittest.TestCase):
    def setUp(self):
        self.screencopy = SCREENCOPY.read_text()
        self.session_lock_header = SESSION_LOCK_HEADER.read_text()
        self.session_lock_source = SESSION_LOCK_SOURCE.read_text()
        self.patch = "\n".join(path.read_text() for path in sorted(PATCH_DIR.glob("*.patch")))

    def test_screencopy_rejects_frames_created_or_copied_after_covering(self):
        self.assertIn('#include "wayland/meta-wayland-session-lock.h"', self.screencopy)
        self.assertGreaterEqual(self.screencopy.count("meta_wayland_session_lock_is_active"), 2)
        self.assertIn("pre-lock frame can never expose the previous desktop", self.screencopy)
        self.assertIn("Do not even advertise a usable buffer", self.screencopy)
        self.assertIn("COGL_PIXEL_FORMAT_ARGB32_NATIVE, NULL, paint_flags", self.screencopy)

    def test_pipewire_blacks_existing_streams_for_both_buffer_types(self):
        self.assertIn("src/backends/meta-stream-source.c", self.patch)
        self.assertIn("record_black_frame", self.patch)
        self.assertIn("SPA_DATA_MemFd", self.patch)
        self.assertIn("SPA_DATA_DmaBuf", self.patch)
        self.assertIn("cogl_framebuffer_clear", self.patch)
        self.assertIn("meta_wayland_session_lock_is_active", self.patch)
        self.assertIn("meta_wayland_session_lock_add_state_changed_callback", self.patch)
        self.assertIn("on_session_lock_state_changed", self.patch)

    def test_pipewire_closes_when_a_black_frame_cannot_be_produced(self):
        self.assertIn("spa_data->type == SPA_DATA_MemFd && spa_data->data", self.patch)
        self.assertIn("cannot produce lock cover frame", self.patch)
        self.assertIn("meta_stream_source_close (source)", self.patch)

    def test_capture_callbacks_follow_the_compositor_state_transition(self):
        self.assertIn("MetaWaylandSessionLockStateChangedFunc", self.session_lock_header)
        self.assertIn("meta_wayland_session_lock_add_state_changed_callback", self.session_lock_header)
        self.assertIn("meta_wayland_session_lock_remove_state_changed_callback", self.session_lock_header)
        self.assertIn("set_state (controller, META_WAYLAND_SESSION_LOCK_COVERING)", self.session_lock_source)
        self.assertIn("set_state (controller, META_WAYLAND_SESSION_LOCK_FAILSAFE)", self.session_lock_source)

    def test_only_presented_monitor_lock_scene_and_its_input_are_allowed(self):
        self.assertIn("src/backends/meta-screen-cast-session.c", self.patch)
        self.assertIn("check_capture_allowed", self.patch)
        self.assertIn("is_session_lock_ready_for_lock_scene", self.patch)
        self.assertIn("META_WAYLAND_SESSION_LOCK_LOCKED", self.patch)
        self.assertIn("meta_wayland_session_lock_is_presentation_confirmed", self.patch)
        self.assertIn("check_capture_allowed (session, invocation, TRUE)", self.patch)
        self.assertGreaterEqual(
            self.patch.count("check_capture_allowed (session, invocation, FALSE)"), 3)
        self.assertIn("Screen capture is unavailable while the session is locked", self.patch)
        self.assertIn("src/backends/meta-remote-desktop-session.c", self.patch)
        self.assertIn("meta_remote_desktop_session_check_can_notify", self.patch)
        self.assertIn("is_session_lock_ready_for_remote_input", self.patch)
        self.assertIn("Remote input is unavailable while the session is locked", self.patch)

    def test_only_monitor_streams_switch_from_black_to_the_lock_scene(self):
        self.assertIn('#include "backends/meta-stream-monitor.h"', self.patch)
        self.assertIn("is_session_lock_scene_stream", self.patch)
        self.assertIn("META_IS_STREAM_MONITOR", self.patch)
        self.assertIn("holds\n+   * direct scan-out off", self.patch)
        self.assertIn("!is_session_lock_scene_stream (source)", self.patch)

    def test_screencast_helper_follows_its_private_session_struct(self):
        screen_cast_patch = self.patch.split("diff --git a/src/backends/meta-stream-source.c", 1)[0]
        self.assertLess(
            screen_cast_patch.index("struct _MetaScreenCastSession"),
            screen_cast_patch.index("is_session_lock_active (MetaScreenCastSession"),
        )


if __name__ == "__main__":
    unittest.main()
