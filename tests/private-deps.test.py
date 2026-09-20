#!/usr/bin/env python3
"""Check private dependency downloads, installation boundaries and runtime links."""

import hashlib
import importlib.util
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("private_deps", ROOT / "scripts/build-private-deps.py")
deps = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deps)


class PrivateDependencies(unittest.TestCase):
    def test_cached_download_is_verified_before_use(self):
        with tempfile.TemporaryDirectory() as directory:
            downloads = Path(directory)
            archive = downloads / "example.tar.xz"
            archive.write_bytes(b"verified archive")
            recipe = {
                "url": "https://invalid.example/example.tar.xz",
                "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            }
            self.assertEqual(deps.checked_archive(recipe, downloads), archive)
            archive.write_bytes(b"corrupt archive")
            with self.assertRaisesRegex(RuntimeError, "Checksum mismatch"):
                deps.checked_archive(recipe, downloads)

    def test_private_library_loads_without_exporting_loader_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "deps"
            (prefix / "lib64").mkdir(parents=True)
            library = prefix / "value.c"
            library.write_text("int private_value(void) { return 42; }\n")
            main = prefix / "main.c"
            main.write_text("int private_value(void); int main(void) { return private_value() != 42; }\n")
            env = deps.build_environment(prefix)
            subprocess.run(
                ["cc", "-shared", "-fPIC", str(library), "-o", str(prefix / "lib64/libprivate.so")], check=True
            )
            subprocess.run(
                [
                    "cc",
                    str(main),
                    f"-L{prefix}/lib64",
                    "-lprivate",
                    *shlex.split(env["LDFLAGS"]),
                    "-o",
                    str(prefix / "program"),
                ],
                env=env,
                check=True,
            )
            clean = {key: value for key, value in os.environ.items() if key != "LD_LIBRARY_PATH"}
            subprocess.run([str(prefix / "program")], env=clean, check=True)
            patcher = os.environ.get("GNOBLIN_TEST_PATCHELF")
            if patcher:
                (prefix / "bin").mkdir()
                (prefix / "bin/patchelf").symlink_to(patcher)
                deps.fix_linkage(prefix, prefix)
                other = Path(directory) / "host"
                other.mkdir()
                library.write_text("int private_value(void) { return 0; }\n")
                subprocess.run(["cc", "-shared", "-fPIC", str(library), "-o", str(other / "libprivate.so")], check=True)
                subprocess.run([str(prefix / "program")], env={**clean, "LD_LIBRARY_PATH": str(other)}, check=True)

    def test_shared_prefixes_are_rejected(self):
        for prefix in ("/usr", "/usr/local", "/etc", "/"):
            result = subprocess.run(
                ["python3", str(ROOT / "scripts/build-private-deps.py"), "--prefix", prefix, "--dry-run"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2, result.stderr)

    def test_runtime_keeps_dependency_binaries_out_of_application_path(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)
            (prefix / "deps/lib64/girepository-1.0").mkdir(parents=True)
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    'source "$1"; gnoblin_env_apply "$2" lib64; printf "%s\\n%s\\n" "$PATH" "$GI_TYPELIB_PATH"',
                    "test",
                    str(ROOT / "src/tools/gnoblin-env.sh"),
                    str(prefix),
                ],
                capture_output=True,
                text=True,
                check=True,
            )
            path, typelibs = result.stdout.splitlines()
            self.assertNotIn(str(prefix / "deps/bin"), path.split(":"))
            self.assertNotIn(str(prefix / "deps/lib64/girepository-1.0"), typelibs.split(":"))

    def test_reinstall_replaces_symlinks_and_files_without_following_them(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            prefix = Path(directory) / "prefix"
            (source / "lib64").mkdir(parents=True)
            (source / "lib64/library.so.1").write_text("new")
            (source / "lib64/library.so").symlink_to("library.so.1")
            deps.install_tree(source, prefix)
            deps.install_tree(source, prefix)
            self.assertTrue((prefix / "lib64/library.so").is_symlink())
            self.assertEqual((prefix / "lib64/library.so").read_text(), "new")


if __name__ == "__main__":
    unittest.main()
