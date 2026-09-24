"""Contract tests for gnoblinctl animation preview commands."""

import importlib.machinery
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader(
    "gnoblinctl_animation", str(Path(__file__).resolve().parents[1] / "src/tools/gnoblinctl")
)
spec = importlib.util.spec_from_loader(loader.name, loader)
ctl = importlib.util.module_from_spec(spec)
loader.exec_module(ctl)


class AnimationCliTests(unittest.TestCase):
    def request(self, words):
        parser = ctl.parser()
        args = parser.parse_args(words)
        args.timeout = 5
        args.socket = "test-socket"
        with patch.object(ctl, "compositor", return_value={"ok": True}) as rpc:
            result = ctl.dispatch(args, parser)
        self.assertEqual(result, {"ok": True})
        return rpc.call_args.args[0]

    def test_preview_selects_layer_namespace_and_starts_paused_by_default(self):
        self.assertEqual(
            self.request(
                [
                    "animation",
                    "preview",
                    "gnoblin-layer-open",
                    "--namespace",
                    "panel:test",
                ]
            ),
            {
                "command": "animation",
                "action": "preview",
                "name": "gnoblin-layer-open",
                "targetType": "namespace",
                "target": "panel:test",
                "autoplay": False,
            },
        )

    def test_preview_supports_a_stable_layer_id_and_autoplay(self):
        self.assertEqual(
            self.request(
                [
                    "animation",
                    "preview",
                    "slide",
                    "--event",
                    "close",
                    "--layer",
                    "layer-17",
                    "--autoplay",
                ]
            ),
            {
                "command": "animation",
                "action": "preview",
                "name": "slide",
                "event": "close",
                "targetType": "layer",
                "target": "layer-17",
                "autoplay": True,
            },
        )

    def test_seek_converts_percentage_to_sampler_progress_and_step_keeps_milliseconds(self):
        self.assertEqual(
            self.request(["animation", "seek", "preview-3", "37"]),
            {
                "command": "animation",
                "action": "seek",
                "session": "preview-3",
                "progress": 0.37,
            },
        )
        self.assertEqual(
            self.request(["animation", "step", "preview-3", "16"]),
            {
                "command": "animation",
                "action": "step",
                "session": "preview-3",
                "milliseconds": 16,
            },
        )

    def test_stop_targets_the_preview_session(self):
        self.assertEqual(
            self.request(["animation", "stop", "preview-3"]),
            {
                "command": "animation",
                "action": "stop",
                "session": "preview-3",
            },
        )

    def test_layer_surfaces_are_discoverable(self):
        self.assertEqual(
            self.request(["animation", "surfaces"]),
            {
                "command": "animation",
                "action": "surfaces",
            },
        )


if __name__ == "__main__":
    unittest.main()
