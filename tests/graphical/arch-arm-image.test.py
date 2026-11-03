#!/usr/bin/env python3
"""Check the trust step of scripts/build-arch-arm-image.sh: the Arch Linux ARM archive is unpacked only when its GPG
signature is good and made by the pinned key. Needs gpg, curl and md5sum. No network, no root, no aarch64.

The test runs the script's own fetch and extract stages against a local mirror (ALARM_BASE=file://...). Throw-away
keys sign a small archive, so the script is run from a copy whose pinned fingerprint is the test key's: that is the
positive control. The negative cases (a changed archive, a changed signature, a signature by another key, another key
in the key file) must each stop the build. The last test checks that the key file shipped in the repository is the
key that the script pins.
"""

from __future__ import annotations

import hashlib
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SCRIPT = ROOT / "scripts/build-arch-arm-image.sh"
TARBALL = "ArchLinuxARM-aarch64-latest.tar.gz"
PIN = re.compile(r'^readonly ALARM_KEY_FPR="([0-9A-F]{40})"$', re.M)


def gpg(home, *arguments, **options):
    return subprocess.run(["gpg", "--homedir", str(home), "--batch", *arguments], check=True, capture_output=True,
                          **options)  # fmt: skip


def new_key(home, name):
    gpg(home, "--pinentry-mode", "loopback", "--passphrase", "", "--quick-gen-key", f"{name} <{name}@example.invalid>",
        "ed25519", "sign", "never")  # fmt: skip
    listing = gpg(home, "--with-colons", "--list-keys", f"{name}@example.invalid", text=True).stdout
    return next(line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:"))


@unittest.skipUnless(all(shutil.which(tool) for tool in ("gpg", "curl", "md5sum")), "needs gpg, curl and md5sum")
class Signature(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.work = Path(tempfile.mkdtemp(prefix="gnoblin-arch-arm-test-"))
        cls.home = cls.work / "gnupg"
        cls.home.mkdir(mode=0o700)
        cls.key_a = new_key(cls.home, "pinned")
        cls.key_b = new_key(cls.home, "other")
        for name, keys in (("a.asc", [cls.key_a]), ("ab.asc", [cls.key_a, cls.key_b])):
            (cls.work / name).write_bytes(gpg(cls.home, "--armor", "--export", *keys).stdout)
        # The script, with the test key pinned in place of the Arch Linux ARM key. Nothing else changes.
        text = SCRIPT.read_text()
        cls.pinned = PIN.search(text).group(1)
        copy = cls.work / "build-arch-arm-image.sh"
        copy.write_text(PIN.sub(f'readonly ALARM_KEY_FPR="{cls.key_a}"', text))
        copy.chmod(0o755)
        cls.script = copy

    @classmethod
    def tearDownClass(cls):
        subprocess.run(["gpgconf", "--homedir", str(cls.home), "--kill", "all"], check=False, capture_output=True)
        shutil.rmtree(cls.work, ignore_errors=True)

    def mirror(self, signer, tamper_archive=False, tamper_signature=False):
        mirror = Path(tempfile.mkdtemp(dir=self.work))
        archive = mirror / TARBALL
        archive.write_bytes(b"an Arch Linux ARM root file system, for the test\n" * 100)
        gpg(self.home, "--local-user", signer, "--detach-sign", "--output", f"{archive}.sig", str(archive))
        if tamper_signature:
            data = bytearray(Path(f"{archive}.sig").read_bytes())
            data[-5] ^= 0x01
            Path(f"{archive}.sig").write_bytes(bytes(data))
        if tamper_archive:
            archive.write_bytes(archive.read_bytes() + b"one more line\n")
        Path(f"{archive}.md5").write_text(f"{hashlib.md5(archive.read_bytes()).hexdigest()}  {TARBALL}\n")
        return mirror

    def run_stage(self, mirror, key_file="a.asc", stage="fetch"):
        cache = mirror / "cache"
        environment = {**os.environ, "ALARM_BASE": f"file://{mirror}", "ALARM_KEY_FILE": str(self.work / key_file)}
        return subprocess.run([str(self.script), "--stage", stage, "--cache", str(cache), "--work", str(mirror / "w")],
                              env=environment, capture_output=True, text=True)  # fmt: skip

    def test_a_good_signature_by_the_pinned_key_passes(self):
        result = self.run_stage(self.mirror(self.key_a))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"signature of {TARBALL} is good (key {self.key_a})", result.stderr)

    def test_a_changed_archive_fails(self):
        result = self.run_stage(self.mirror(self.key_a, tamper_archive=True))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error:", result.stderr)

    def test_a_changed_signature_fails(self):
        result = self.run_stage(self.mirror(self.key_a, tamper_signature=True))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error:", result.stderr)

    def test_a_signature_by_another_key_fails(self):
        # The other key is not in the key file: gpg cannot check the signature at all (ERRSIG, no public key).
        result = self.run_stage(self.mirror(self.key_b))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("error: bad, expired or revoked signature", result.stderr)

    def test_another_key_in_the_key_file_does_not_count(self):
        # Both keys are in the key file and the signature by the other one is good: only the pinned one counts.
        result = self.run_stage(self.mirror(self.key_b), key_file="ab.asc")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"is not signed by {self.key_a}", result.stderr)

    def test_a_missing_key_file_stops_the_build_and_fetches_no_key(self):
        result = self.run_stage(self.mirror(self.key_a), key_file="missing.asc")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("the pinned Arch Linux ARM key is part of the repository", result.stderr)

    def test_extract_checks_the_cached_archive_again(self):
        mirror = self.mirror(self.key_a)
        self.assertEqual(self.run_stage(mirror).returncode, 0)
        cached = mirror / "cache" / TARBALL
        cached.write_bytes(cached.read_bytes() + b"changed in the cache\n")
        result = self.run_stage(mirror, stage="extract")
        self.assertNotEqual(result.returncode, 0)
        # It stops at the signature (or for a missing bsdtar before it), never at the root check after it.
        self.assertNotIn("needs root", result.stderr)
        self.assertFalse((mirror / "w" / "rootfs").exists())

    def test_the_shipped_key_file_is_the_pinned_key(self):
        listing = subprocess.run(["gpg", "--batch", "--with-colons", "--show-keys", str(SCRIPT.parent / "alarm-signing-key.asc")],
                                 check=True, capture_output=True, text=True).stdout  # fmt: skip
        primary = [line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:")][0]
        self.assertEqual(primary, self.pinned)


class CheckSpace(unittest.TestCase):
    """pacman's CheckSpace fails in the build tree, which is a folder and not a mount (run 41672380634): the prepare
    stage turns it off and the configure stage turns it on again, so the guest's own pacman checks the space."""

    def test_prepare_turns_it_off_and_configure_turns_it_on_again(self):
        work = Path(tempfile.mkdtemp(prefix="gnoblin-arch-arm-checkspace-"))
        etc = work / "rootfs/etc"
        etc.mkdir(parents=True)
        (work / "rootfs/usr").mkdir()
        (etc / "mkinitcpio.conf").write_text("HOOKS=(base udev autodetect modconf)\n")
        (etc / "pacman.conf").write_text("[options]\nHoldPkg = pacman\nCheckSpace\n")
        run = [str(SCRIPT), "--work", str(work), "--stage"]
        subprocess.run([*run, "prepare"], check=True, capture_output=True)
        self.assertNotIn("\nCheckSpace\n", (etc / "pacman.conf").read_text())
        # configure stops later on the missing boot loader of this fake tree, after CheckSpace is back.
        subprocess.run([*run, "configure"], capture_output=True)
        self.assertIn("\nCheckSpace\n", (etc / "pacman.conf").read_text())
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
