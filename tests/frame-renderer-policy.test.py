#!/usr/bin/env python3
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class FrameRendererPolicyTests(unittest.TestCase):
    def test_timeout_preserves_a_presented_external_frame(self):
        source = (ROOT / "src/protocols/window-frame/meta-gnoblin-frame-renderer.c").read_text()
        timeout = source.split("static gboolean renderer_timeout", 1)[1].split("#define RESIZE_OUTSET", 1)[0]
        self.assertIn("if (!frame->external)\n        fallback(frame);", timeout)


if __name__ == "__main__":
    unittest.main()
