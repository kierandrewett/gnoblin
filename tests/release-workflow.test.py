#!/usr/bin/env python3
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_release_build_is_tag_versioned_and_complete(self):
        script = (ROOT / "scripts/build-release-assets.sh").read_text()
        self.assertIn('EXPECTED_TAG="v$VERSION"', script)
        self.assertIn('make-tarball.sh" mutter', script)
        self.assertIn('make-tarball.sh" gnome-shell', script)
        self.assertIn('make-tarball.sh" gsettings-desktop-schemas', script)
        for project in ("gsettings-desktop-schemas", "mutter", "gnome-shell", "gnoblin"):
            self.assertIn(f'build-srpm.sh" {project}', script)
        self.assertIn("gnoblin-$VERSION.PKGBUILD", script)
        self.assertIn("gnoblin-$VERSION-debian.tar.xz", script)
        self.assertIn("SHA256SUMS", script)

    def test_release_workflow_publishes_only_after_artifacts_build(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('tags:\n      - "v*"', workflow)
        self.assertIn("contents: write", workflow)
        self.assertIn("needs: source-packages", workflow)
        self.assertIn("--verify-tag", workflow)
        self.assertIn("--clobber", workflow)

    def test_nix_source_of_truth_is_a_ci_gate(self):
        workflow = (ROOT / ".github/workflows/nix.yml").read_text()
        self.assertIn("./scripts/sync-package-manifest.py check", workflow)
        self.assertIn("nix flake check -L", workflow)


if __name__ == "__main__":
    unittest.main()
