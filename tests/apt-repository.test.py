#!/usr/bin/env python3
"""Keep packages for different Ubuntu releases in their own APT indexes."""

from pathlib import Path
import os
import importlib.util
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("build_apt_repository", ROOT / "scripts/build-apt-repository.py")
assert spec and spec.loader
apt_repository = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = apt_repository
spec.loader.exec_module(apt_repository)


class AptRepositoryTests(unittest.TestCase):
    def test_each_suite_index_contains_only_its_distro_builds(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "tools"
            tools.mkdir()
            dpkg_deb = tools / "dpkg-deb"
            dpkg_deb.write_text('#!/bin/sh\ncat "$2"\n', encoding="utf-8")
            dpkg_deb.chmod(0o755)
            previous_path = os.environ["PATH"]
            os.environ["PATH"] = f"{tools}:{previous_path}"
            try:
                pool = root / "apt/ubuntu/pool/main/g/gnoblin"
                pool.mkdir(parents=True)
                versions = (
                    "51.0+gnoblin0.1.7-2~ubuntu24.04",
                    "51.0+gnoblin0.1.7-2~ubuntu26.04",
                    "51.0+gnoblin0.1.7-3~ubuntu24.04",
                    "51.0+gnoblin0.1.7-3~ubuntu26.04",
                )
                for version in versions:
                    (pool / f"gnoblin_{version}_amd64.deb").write_text(
                        f"Package: gnoblin\nVersion: {version}\nArchitecture: amd64\n",
                        encoding="utf-8",
                    )

                for release, suffix in (("24.04", "ubuntu24.04"), ("26.04", "ubuntu26.04")):
                    suite = apt_repository.Suite("ubuntu", release, "unused.deb", suffix, f"Ubuntu {release}")
                    latest_version = f"51.0+gnoblin0.1.7-3~{suffix}"
                    apt_repository.write_packages(root / "apt", suite, latest_version)
                    index = (root / "apt/ubuntu/dists" / release / "main/binary-amd64/Packages").read_text(
                        encoding="utf-8"
                    )
                    self.assertIn(latest_version, index)
                    self.assertNotIn(f"0.1.7-2~{suffix}", index)
                    other_suffix = "ubuntu26.04" if suffix == "ubuntu24.04" else "ubuntu24.04"
                    self.assertNotIn(f"~{other_suffix}", index)
            finally:
                os.environ["PATH"] = previous_path


if __name__ == "__main__":
    unittest.main()
