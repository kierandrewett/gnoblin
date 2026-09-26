#!/usr/bin/env python3
"""Check the package-private runtime closure for legacy RPM targets."""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "compose_rpm_compat_runtime", ROOT / "packaging/rpm/compose-compat-runtime-manifest.py"
)
composer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(composer)


class RPMCompatibilityRuntimeManifestTests(unittest.TestCase):
    def test_composes_one_complete_pinned_private_runtime(self):
        runtime = composer.compose(composer.DEFAULT_MANIFESTS)
        names = [recipe["name"] for recipe in runtime]

        self.assertEqual(len(names), len(set(names)))
        self.assertIn("glib-final", names)
        self.assertNotIn("glib", names)
        self.assertLess(names.index("glib-bootstrap"), names.index("gobject-introspection-bootstrap"))
        self.assertLess(names.index("gobject-introspection-bootstrap"), names.index("glib-final"))
        self.assertLess(names.index("glib-final"), names.index("gtk4"))
        self.assertLess(names.index("mozjs"), names.index("gjs"))
        self.assertLess(names.index("hyprutils"), names.index("hyprcursor"))

    def test_rejects_a_missing_runtime_component(self):
        with self.assertRaisesRegex(RuntimeError, "incomplete private RPM runtime"):
            composer.compose([ROOT / "packaging/rpm/compat-bootstrap.json"])

    def test_runtime_builder_uses_the_composed_manifest_and_private_prefix(self):
        builder = (ROOT / "packaging/rpm/build-compat-runtime.sh").read_text()
        self.assertIn("compose-compat-runtime-manifest.py", builder)
        self.assertIn('--prefix "$PREFIX"', builder)
        self.assertIn('--manifest "$MANIFEST"', builder)
        self.assertIn("PREFIX=/usr/lib/gnoblin/deps", builder)


if __name__ == "__main__":
    unittest.main()
