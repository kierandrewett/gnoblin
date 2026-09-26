#!/usr/bin/env python3
import unittest
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ReleaseWorkflowTests(unittest.TestCase):
    def test_release_tags_match_the_pinned_gnoblin_semver(self):
        script = ROOT / "scripts/check-release-tag.sh"
        version = subprocess.check_output(
            [str(ROOT / "scripts/gnoblin-version.py"), "get", "version"], text=True
        ).strip()
        self.assertEqual(subprocess.check_output([str(script), f"gnoblin-v{version}"], text=True).strip(), "1")
        for tag in ("main", "v51.0", "gnoblin-v0.1", "gnoblin-v0.1.0-1", "gnoblin-v0.1.0.1"):
            self.assertNotEqual(subprocess.run([str(script), tag], capture_output=True).returncode, 0)

    def test_release_build_is_tag_versioned_and_complete(self):
        script = (ROOT / "scripts/build-release-assets.sh").read_text()
        self.assertIn('EXPECTED_TAG="gnoblin-v$GNOBLIN_VERSION"', script)
        self.assertIn('make-tarball.sh" mutter', script)
        self.assertIn('make-tarball.sh" gnome-shell', script)
        self.assertIn('make-tarball.sh" gsettings-desktop-schemas', script)
        for project in ("gsettings-desktop-schemas", "mutter", "gnome-shell", "gnoblin"):
            self.assertIn(f'build-srpm.sh" {project}', script)
        self.assertIn("gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION.PKGBUILD", script)
        self.assertIn("gnoblin-$GNOBLIN_VERSION-gnome-$GNOME_VERSION-debian.tar.xz", script)
        self.assertIn("SHA256SUMS", script)

    def test_release_tarballs_preserve_relative_link_targets(self):
        script = (ROOT / "scripts/make-tarball.sh").read_text()
        self.assertIn('--transform="s,^,${PROJ}-${VER}/,SH"', script)

    def test_release_workflow_publishes_only_after_artifacts_build(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('tags:\n      - "gnoblin-v*"', workflow)
        self.assertIn("contents: write", workflow)
        self.assertIn("pages: write", workflow)
        self.assertIn("needs: [source-packages, debian-packages]", workflow)
        self.assertIn("git submodule foreach --recursive 'git fetch --force --tags origin'", workflow)
        self.assertIn("GIT_COMMITTER_NAME: Gnoblin release automation", workflow)
        self.assertIn("GIT_COMMITTER_EMAIL: release@gnoblin.local", workflow)
        self.assertIn("--verify-tag", workflow)
        self.assertIn("--clobber", workflow)
        self.assertIn("Gnoblin $(./scripts/gnoblin-version.py get version)", workflow)
        self.assertIn("copr-repository:", workflow)
        self.assertIn("uses: ./.github/workflows/copr.yml", workflow)
        self.assertRegex(workflow, r"dnf -y install[^\n]*\binkscape\b")
        self.assertRegex(workflow, r"dnf -y install[^\n]*\bhyprcursor\b")
        self.assertRegex(workflow, r"dnf -y install[^\n]*\badwaita-cursor-theme\b")

    def test_release_waits_for_and_publishes_opensuse_rpms(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        opensuse = (ROOT / ".github/workflows/opensuse-rpm.yml").read_text()
        self.assertIn("opensuse-package:", workflow)
        self.assertIn("uses: ./.github/workflows/opensuse-rpm.yml", workflow)
        self.assertIn("with:\n      ref:", workflow)
        self.assertIn("opensuse-package", workflow.split("github-release:", 1)[1].split("apt-repository:", 1)[0])
        self.assertIn("name: opensuse-tumbleweed-rpms", opensuse)
        self.assertIn("workflow_call:", opensuse)
        self.assertIn("ref: ${{ inputs.ref || github.sha }}", opensuse)
        self.assertIn("Flatten openSUSE RPM assets", workflow)
        self.assertIn("find opensuse-rpms -type f -name '*.rpm'", workflow)
        self.assertIn('"release-assets/opensuse-$(basename "$rpm")"', workflow)

    def test_release_builds_the_pinned_nixos_package_before_publication(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        nix = (ROOT / ".github/workflows/nix.yml").read_text()
        self.assertIn("nixos-release:", workflow)
        self.assertIn("uses: ./.github/workflows/nix.yml", workflow)
        self.assertIn("nixos-release", workflow.split("github-release:", 1)[1].split("apt-repository:", 1)[0])
        self.assertIn("workflow_call:", nix)
        self.assertIn("ref: ${{ inputs.ref || github.sha }}", nix)
        self.assertIn("nix build -L .#packages.x86_64-linux.gnoblin-nixos-26_05", nix)
        self.assertIn("test -x result/bin/gnoblin-session", nix)
        self.assertNotIn("release-nixos-26-05:", nix)

    def test_copr_release_job_publishes_and_installs_the_tagged_source_rpms(self):
        workflow = (ROOT / ".github/workflows/copr.yml").read_text()
        self.assertIn("COPR_CONFIG:", workflow)
        self.assertIn("required: true", workflow)
        self.assertIn('gh release download "$RELEASE_TAG"', workflow)
        self.assertIn("scripts/publish-copr.sh kierandrewett/gnoblin", workflow)
        self.assertIn("dnf -y install --refresh gnoblin", workflow)
        self.assertIn("rpm -q gnoblin gnoblin-mutter gnoblin-shell gnoblin-session", workflow)

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
        self.assertIn("just build-source", script)
        self.assertNotIn("dev-settings", script)
        self.assertIn("Optional portal backend build", script)


if __name__ == "__main__":
    unittest.main()
