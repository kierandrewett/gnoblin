#!/usr/bin/env python3
"""Guard the fail-closed boundary around ext-session-lock-v1.

This is deliberately a source-level test.  A real compositor session is still
required before the protocol can be advertised; see the session-lock design
document for that acceptance suite.
"""

from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src/protocols/session-lock/meta-wayland-session-lock.c"
XML = ROOT / "src/protocols/session-lock/ext-session-lock-v1.xml"


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

    def test_boundary_cannot_advertise_an_incomplete_lock_protocol(self):
        source = SOURCE.read_text()
        self.assertIn('gnoblin_config_protocol_enabled ("ext-session-lock")', source)
        self.assertIn("required fail-closed scene", source)
        self.assertIn("output-hotplug, and client-death controller", source)
        self.assertNotIn("wl_global_create", source)
        self.assertNotIn("ext_session_lock_manager_v1_interface", source)


if __name__ == "__main__":
    unittest.main()
