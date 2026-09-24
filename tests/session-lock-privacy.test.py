#!/usr/bin/env python3
"""Source-level guardrails for session-lock clipboard privacy.

The native Mutter patch is checked separately because it protects offers made
before the lock transition, while the Gnoblin ext-data-control implementation
also has to revoke its non-focus clients immediately.
"""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
DATA_CONTROL = ROOT / "src/protocols/data-control/meta-wayland-data-control.c"
MANAGER = ROOT / "src/protocols/session-lock/meta-wayland-session-lock.c"
PRIVACY_PATCH = ROOT / "patches/mutter/73-session-lock-privacy"


class SessionLockPrivacyTests(unittest.TestCase):
    def test_data_control_revokes_offers_but_preserves_selection_owner(self):
        source = DATA_CONTROL.read_text()

        self.assertIn("meta_wayland_session_lock_add_state_changed_callback", source)
        self.assertIn("ext_data_control_device_v1_send_selection(device->resource, NULL)", source)
        self.assertIn("ext_data_control_device_v1_send_primary_selection(device->resource, NULL)", source)
        self.assertIn("device_advertise_selection(device, META_SELECTION_CLIPBOARD)", source)
        self.assertIn("data_control_is_embargoed(offer->data_control)", source)
        self.assertIn("data-control source is unavailable while the session is locked", source)
        self.assertNotIn("meta_selection_unset_owner(selection, META_SELECTION_CLIPBOARD", source)

    def test_native_clipboard_primary_and_drag_paths_are_embargoed(self):
        patch = "\n".join(path.read_text() for path in PRIVACY_PATCH.glob("*.patch"))

        self.assertIn("meta-wayland-data-offer.c", patch)
        self.assertIn("meta-wayland-data-offer-primary.c", patch)
        self.assertIn("meta-wayland-data-device.c", patch)
        self.assertGreaterEqual(patch.count("meta_wayland_session_lock_is_active"), 3)
        self.assertIn("close (fd)", patch)
        self.assertIn("meta_wayland_data_source_cancel (source)", patch)

    def test_lock_transition_ends_an_in_progress_drag(self):
        source = MANAGER.read_text()

        self.assertIn("meta_wayland_data_device_end_drag (&seat->data_device)", source)


if __name__ == "__main__":
    unittest.main()
