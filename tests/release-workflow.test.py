#!/usr/bin/env python3
import unittest
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_release_tags_match_the_pinned_base_and_positive_revision(self):
        script = ROOT / "scripts/check-release-tag.sh"
        version = subprocess.check_output(
            [str(ROOT / "scripts/gnome-versions.py"), "get", "mutter", "version"], text=True
        ).strip()
        for suffix, revision in (("", "1"), ("-1", "1"), ("-12", "12")):
            self.assertEqual(subprocess.check_output([str(script), f"v{version}{suffix}"], text=True).strip(), revision)
        for tag in ("main", "v0.0", f"v{version}-0", f"v{version}-01", f"v{version}-preview"):
            self.assertNotEqual(subprocess.run([str(script), tag], capture_output=True).returncode, 0)

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

    def test_release_tarballs_preserve_relative_link_targets(self):
        script = (ROOT / "scripts/make-tarball.sh").read_text()
        self.assertIn('--transform="s,^,${PROJ}-${VER}/,SH"', script)

    def test_release_workflow_publishes_only_after_artifacts_build(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('tags:\n      - "v*"', workflow)
        self.assertIn("contents: write", workflow)
        self.assertIn("needs: [source-packages, debian-packages]", workflow)
        self.assertIn("git submodule foreach --recursive 'git fetch --force --tags origin'", workflow)
        self.assertIn("GIT_COMMITTER_NAME: Gnoblin release automation", workflow)
        self.assertIn("GIT_COMMITTER_EMAIL: release@gnoblin.local", workflow)
        self.assertIn("--verify-tag", workflow)
        self.assertIn("--clobber", workflow)

    def test_nix_source_of_truth_is_a_ci_gate(self):
        workflow = (ROOT / ".github/workflows/nix.yml").read_text()
        self.assertIn("./scripts/sync-package-manifest.py check", workflow)
        self.assertIn("nix flake check -L", workflow)

    def test_source_build_bootstraps_private_gnome_schemas(self):
        justfile = (ROOT / "Justfile").read_text()
        self.assertIn("dev-schemas: check-install-prefix", justfile)
        self.assertIn("dev-mutter: dev-schemas", justfile)
        self.assertIn("make-tarball.sh gsettings-desktop-schemas", justfile)
        self.assertIn(
            'private_pkg_config_path := prefix + "/" + libdir + "/pkgconfig:" + prefix + "/share/pkgconfig:"', justfile
        )
        self.assertIn("GI_GIR_PATH={{private_gir_path}}", justfile)

    def test_default_source_build_contains_only_required_runtime(self):
        script = (ROOT / "build.sh").read_text()
        self.assertIn("just build-local", script)
        self.assertNotIn("just build-local dev-settings dev-portal", script)
        self.assertIn("Optional Settings and portal forks", script)


if __name__ == "__main__":
    unittest.main()
