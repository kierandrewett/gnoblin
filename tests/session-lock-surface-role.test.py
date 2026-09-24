#!/usr/bin/env python3
"""Focused structural guards for Gnoblin's ext-session-lock surface role."""

from pathlib import Path
import unittest


SOURCE = (
    Path(__file__).resolve().parents[1]
    / "src/protocols/session-lock/meta-wayland-session-lock-surface.c"
)


class SessionLockSurfaceRoleTests(unittest.TestCase):
    def test_reparents_existing_actor_before_adding_to_private_scene(self):
        source = SOURCE.read_text()

        self.assertIn("g_object_ref (actor)", source)
        self.assertIn("clutter_actor_remove_child (parent, CLUTTER_ACTOR (actor))", source)
        self.assertIn("clutter_actor_add_child (scene, CLUTTER_ACTOR (actor))", source)
        self.assertLess(
            source.index("clutter_actor_remove_child (parent, CLUTTER_ACTOR (actor))"),
            source.index("clutter_actor_add_child (scene, CLUTTER_ACTOR (actor))"),
        )

    def test_uses_stage_relative_output_geometry_and_removes_stale_actor(self):
        source = SOURCE.read_text()

        self.assertIn("clutter_actor_set_position (CLUTTER_ACTOR (actor), layout.x, layout.y)", source)
        self.assertIn("meta_window_move_resize_frame", source)
        self.assertIn("output-destroyed", source)
        self.assertIn("clutter_actor_remove_child (scene, CLUTTER_ACTOR (actor))", source)


if __name__ == "__main__":
    unittest.main()
