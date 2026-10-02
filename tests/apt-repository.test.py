#!/usr/bin/env python3
"""Keep packages for different Debian and Ubuntu releases in their own APT indexes."""

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
                for suite in apt_repository.SUITES:
                    pool = root / "apt" / suite.archive / "pool/main/g/gnoblin"
                    pool.mkdir(parents=True, exist_ok=True)
                    for revision in (2, 3):
                        version = f"51.0+gnoblin0.1.7-{revision}~{suite.version_suffix}"
                        (pool / f"gnoblin_{version}_amd64.deb").write_text(
                            f"Package: gnoblin\nVersion: {version}\nArchitecture: amd64\n",
                            encoding="utf-8",
                        )

                for suite in apt_repository.SUITES:
                    suffix = suite.version_suffix
                    latest_version = f"51.0+gnoblin0.1.7-3~{suffix}"
                    apt_repository.write_packages(root / "apt", suite, latest_version)
                    index = (
                        root / "apt" / suite.archive / "dists" / suite.suite / "main/binary-amd64/Packages"
                    ).read_text(encoding="utf-8")
                    self.assertIn(latest_version, index)
                    self.assertNotIn(f"0.1.7-2~{suffix}", index)
                    for other_suite in apt_repository.SUITES:
                        if other_suite is not suite:
                            self.assertNotIn(f"~{other_suite.version_suffix}", index)
            finally:
                os.environ["PATH"] = previous_path


if __name__ == "__main__":
    unittest.main()
