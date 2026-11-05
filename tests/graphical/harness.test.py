#!/usr/bin/env python3
"""Check the parts of the graphical VM test that need no guest (tests/graphical/harness.py): checksum files, image
names, the screenshot properties, the allow-list, the QEMU command of each architecture's guest data and the run
deadline. No network, no QEMU: the aarch64 command is checked as text only."""

import argparse
import importlib.util
import json
import os
import shutil
import subprocess
from pathlib import Path
import signal
import struct
import time
import tempfile
import unittest
import zlib

HERE = Path(__file__).resolve().parent
specification = importlib.util.spec_from_file_location("harness", HERE / "harness.py")
harness = importlib.util.module_from_spec(specification)
specification.loader.exec_module(harness)

WIDTH, HEIGHT = 1280, 800
BACKGROUND = (40, 44, 52)


def frame(path, boxes=(), panel=True):
    """A PPM like a QMP screendump: a background, a panel strip, and boxes of (colour, left, top, width, height)."""
    pixels = bytearray(bytes(BACKGROUND) * WIDTH * HEIGHT)
    if panel:
        boxes = [((200, 200, 200), 0, 0, WIDTH, 32), *boxes]
    for colour, left, top, width, height in boxes:
        for y in range(top, top + height):
            start = (y * WIDTH + left) * 3
            pixels[start : start + width * 3] = bytes(colour) * width
    path.write_bytes(b"P6\n%d %d\n255\n" % (WIDTH, HEIGHT) + bytes(pixels))
    return path


class Checksums(unittest.TestCase):
    def test_gnu_and_bsd_lines(self):
        text = "\n".join(
            [
                "-----BEGIN PGP SIGNED MESSAGE-----",
                "# Fedora-Cloud-Base-Generic-42-1.1.x86_64.qcow2: 123 bytes",
                "SHA256 (Fedora-Cloud-Base-Generic-42-1.1.x86_64.qcow2) = " + "ab" * 32,
                "cd" * 32 + " *ubuntu-26.04-server-cloudimg-amd64.img",
                "ef" * 64 + "  debian-13-genericcloud-amd64.qcow2",
            ]
        )
        self.assertEqual(
            harness.checksum_of(text, "Fedora-Cloud-Base-Generic-42-1.1.x86_64.qcow2"), ("sha256", "ab" * 32)
        )
        self.assertEqual(harness.checksum_of(text, "ubuntu-26.04-server-cloudimg-amd64.img"), ("sha256", "cd" * 32))
        self.assertEqual(harness.checksum_of(text, "debian-13-genericcloud-amd64.qcow2"), ("sha512", "ef" * 64))
        with self.assertRaises(harness.CheckFailed):
            harness.checksum_of(text, "missing.qcow2")

    def test_a_bare_digest_counts_only_in_the_image_own_checksum_file(self):
        # Alpine's alpine-...-cloudinit-r0.qcow2.sha512 holds the digest and nothing else.
        self.assertEqual(harness.checksum_of("ab" * 64 + "\n", "a.qcow2", own_file=True), ("sha512", "ab" * 64))
        with self.assertRaises(harness.CheckFailed):
            harness.checksum_of("ab" * 64 + "\n", "a.qcow2")

    def test_newest_image_name_sorts_last(self):
        names = [
            "generic_alpine-3.24.1-x86_64-bios-cloudinit-r0.qcow2",
            "alpine-3.24.3-x86_64-cloudinit-r0.qcow2",
            "alpine-3.24.2-x86_64-cloudinit-r2.qcow2",
        ]
        self.assertEqual(sorted(names, key=harness.version_numbers)[-1], "alpine-3.24.3-x86_64-cloudinit-r0.qcow2")


class BuiltImages(unittest.TestCase):
    """A target without a published image (Arch Linux on arm64) is built by a script: the harness runs it, reads the
    .sha256 file that it writes and checks the image against it."""

    def script(self, body):
        folder = Path(tempfile.mkdtemp())
        script = folder / "build.sh"
        script.write_text('#!/bin/bash\nset -e\nwhile (($#)); do case "$1" in --out) o="$2";; --name) n="$2";; esac;'
                          f' shift 2; done\n{body}\n')  # fmt: skip
        script.chmod(0o755)
        return folder, {"script": str(script), "file": "image.qcow2", "timeout_minutes": 1}

    def test_a_good_build_returns_the_image_and_its_digest(self):
        folder, build = self.script('mkdir -p "$o"; echo image > "$o/$n"; (cd "$o"; sha256sum "$n" > "$n.sha256")')
        info = harness.build_image(build, folder / "cache", folder, folder / "build.log")
        self.assertEqual(Path(info["path"]).read_text(), "image\n")
        self.assertEqual(len(info["sha256"]), 64)

    def test_a_changed_image_fails_the_check(self):
        folder, build = self.script(
            'mkdir -p "$o"; echo image > "$o/$n"; echo "$(printf 0%.0s {1..64})  $n" > "$o/$n.sha256"'
        )
        with self.assertRaises(harness.CheckFailed):
            harness.build_image(build, folder / "cache", folder, folder / "build.log")

    def test_a_failing_build_fails_with_the_end_of_its_log(self):
        folder, build = self.script("echo the-last-line; exit 3")
        with self.assertRaises(harness.CheckFailed) as caught:
            harness.build_image(build, folder / "cache", folder, folder / "build.log")
        self.assertIn("the-last-line", caught.exception.excerpt)


class Screenshots(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp())

    def properties(self, name, colour=harness.FIRST_COLOUR, **options):
        return harness.analyse(frame(self.folder / f"{name}.ppm", **options), colour)

    def test_client_window_passes(self):
        result = self.properties("good", boxes=[(harness.FIRST_COLOUR, 400, 220, 480, 360)])
        harness.check_frame(result, harness.FIRST_COLOUR, "frame")
        self.assertEqual(result["client_box"], [400, 220, 879, 579])
        self.assertTrue((self.folder / "good.png").read_bytes().startswith(b"\x89PNG"))

    def test_blank_frame_fails(self):
        result = self.properties("blank", panel=False)
        with self.assertRaisesRegex(harness.CheckFailed, "blank"):
            harness.check_frame(result, harness.FIRST_COLOUR, "frame")

    def test_frame_without_client_fails(self):
        result = self.properties("empty")
        with self.assertRaisesRegex(harness.CheckFailed, "no pixel of the client colour"):
            harness.check_frame(result, harness.FIRST_COLOUR, "frame")

    def test_client_of_wrong_size_fails(self):
        result = self.properties("small", boxes=[(harness.FIRST_COLOUR, 400, 220, 200, 100)])
        with self.assertRaisesRegex(harness.CheckFailed, "covers 200x100"):
            harness.check_frame(result, harness.FIRST_COLOUR, "frame")

    def test_other_colour_does_not_count(self):
        result = self.properties(
            "typed", colour=harness.SECOND_COLOUR, boxes=[(harness.FIRST_COLOUR, 400, 220, 480, 360)]
        )
        with self.assertRaises(harness.CheckFailed):
            harness.check_frame(result, harness.SECOND_COLOUR, "frame")

    def test_png_decodes(self):
        harness.analyse(frame(self.folder / "png.ppm"), harness.FIRST_COLOUR)
        data = (self.folder / "png.png").read_bytes()
        width, height = struct.unpack(">II", data[16:24])
        self.assertEqual((width, height), (WIDTH, HEIGHT))
        start = data.index(b"IDAT") + 4
        length = struct.unpack(">I", data[start - 8 : start - 4])[0]
        rows = zlib.decompress(data[start : start + length])
        self.assertEqual(len(rows), HEIGHT * (WIDTH * 3 + 1))
        self.assertEqual(rows[1:4], bytes((200, 200, 200)))


class AllowList(unittest.TestCase):
    def test_rules_and_reasons(self):
        folder = Path(tempfile.mkdtemp())
        rules = folder / "allow.txt"
        rules.write_text("# a comment\n\nlibEGL warning: .*CRITICAL  # Mesa on llvmpipe\nGjs-CRITICAL \\*\\*: x\n")
        original = harness.ALLOW_LISTS
        harness.ALLOW_LISTS = (rules,)
        harness.ROOT = folder
        try:
            patterns, source = harness.load_allow_list()
        finally:
            harness.ALLOW_LISTS = original
            harness.ROOT = HERE.parent.parent
        self.assertEqual(source, "allow.txt")
        self.assertEqual(
            [pattern.pattern for pattern in patterns], ["libEGL warning: .*CRITICAL", "Gjs-CRITICAL \\*\\*: x"]
        )

    def test_log_level_matches_words_only(self):
        self.assertTrue(harness.LOG_LEVEL.search("(gnoblin:12): Gjs-CRITICAL **: oops"))
        self.assertTrue(harness.LOG_LEVEL.search("ERROR: failed"))
        self.assertFalse(harness.LOG_LEVEL.search("G_DEBUG=fatal-criticals ERRORS_SEEN=0"))


ROOT = HERE.parent.parent


def shipped_guest(arch):
    policy = json.loads((ROOT / "scripts/targets.json").read_text())
    return next(item["guest"] for item in policy["architectures"] if item["arch"] == arch)


class Guests(unittest.TestCase):
    """The QEMU command of each architecture, from the shipped guest data. The aarch64 command is checked as text
    only: no aarch64 guest has run here."""

    def command(self, arch, firmware=None):
        return harness.qemu_command(shipped_guest(arch), "tcg", 4096, 4, "d.qcow2", "s.iso", 2222, "q.sock", "ser.log",
                                    firmware)  # fmt: skip

    def test_amd64_is_q35_without_ps2_on_the_default_bios(self):
        command = self.command("amd64")
        self.assertEqual(command[0], "qemu-system-x86_64")
        self.assertIn("q35,i8042=off,vmport=off,accel=tcg", command)
        self.assertIn("virtio-vga,xres=1280,yres=800", command)
        self.assertEqual(command[-2:], ["-boot", "order=c"])
        self.assertFalse([part for part in command if "pflash" in part])

    def test_arm64_is_virt_on_uefi_with_a_pci_gpu_and_virtio_input(self):
        command = self.command("arm64", ("code.fd", "vars.fd"))
        self.assertEqual(command[0], "qemu-system-aarch64")
        self.assertIn("virt,gic-version=max,accel=tcg", command)
        self.assertIn("virtio-gpu-pci,xres=1280,yres=800", command)
        self.assertIn("if=pflash,format=raw,unit=0,readonly=on,file=code.fd", command)
        self.assertIn("if=pflash,format=raw,unit=1,file=vars.fd", command)
        self.assertIn("virtio-keyboard-pci,id=keyboard", command)
        self.assertIn("virtio-tablet-pci,id=tablet", command)
        self.assertNotIn("-boot", command)
        self.assertFalse([part for part in command if "i8042" in part or "virtio-vga" in part])
        self.assertEqual(command[command.index("-cpu") + 1], "max,pauth-impdef=on")

    def test_a_uefi_guest_without_firmware_fails_with_the_paths_it_tried(self):
        guest = {**shipped_guest("arm64"), "firmware": [{"code": "/nonexistent/code.fd", "vars": "/nonexistent/v"}]}
        with self.assertRaisesRegex(harness.CheckFailed, "no UEFI firmware for aarch64: none of /nonexistent/code.fd"):
            harness.find_firmware(guest)
        self.assertIsNone(harness.find_firmware(shipped_guest("amd64")))


@unittest.skipUnless(shutil.which("qemu-system-x86_64") and shutil.which("qemu-img") and shutil.which("ssh-keygen"),
                     "needs qemu-system-x86_64, qemu-img and ssh-keygen")  # fmt: skip
class GuestStart(unittest.TestCase):
    """Start the amd64 guest command for real on a blank disk (TCG): QEMU accepts it and QMP answers. A device that
    QEMU refuses at start (here an option ROM file that does not exist, as efi-virtio.rom on an Ubuntu arm64 runner)
    fails the check with QEMU's own message, not with a connection error."""

    def start(self, mutate=None):
        work = Path(tempfile.mkdtemp(dir=os.environ.get("TMPDIR", "/var/tmp")))
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", str(work / "base.qcow2"), "64M"], check=True)
        (work / "seed.iso").write_bytes(b"\0" * 4096)
        guest = harness.Guest(work, work, "tcg", 256, 1, shipped_guest("amd64"))
        original = harness.qemu_command
        if mutate:
            harness.qemu_command = lambda *a, **k: mutate(original(*a, **k))
        try:
            guest.start(work / "base.qcow2", work / "seed.iso")
            return guest, work
        finally:
            harness.qemu_command = original

    def test_the_amd64_guest_starts_and_answers_on_qmp(self):
        guest, work = self.start()
        try:
            self.assertIsNone(guest.process.poll())
            self.assertEqual(guest.qmp.command("query-status")["status"], "running")
        finally:
            guest.process.kill()
            guest.process.wait()
            shutil.rmtree(work, ignore_errors=True)

    def test_a_device_that_qemu_refuses_fails_with_its_message(self):
        def bad_rom(command):
            return [part.replace("romfile=", "romfile=/nonexistent/efi-virtio.rom") for part in command]

        with self.assertRaises(harness.CheckFailed) as caught:
            self.start(bad_rom)
        self.assertEqual(caught.exception.detail, "QEMU exited at start")
        self.assertIn("efi-virtio.rom", caught.exception.excerpt)


@unittest.skipUnless(shutil.which("qemu-system-x86_64"), "needs qemu-system-x86_64")
class TcgCpu(unittest.TestCase):
    """The CPU model without KVM comes from the guest data (aarch64: max,pauth-impdef=on). A QEMU that does not know a
    property of it falls back to max, with the reason printed, so a guest never fails to start over a speed option."""

    guest = {"qemu": "qemu-system-x86_64", "machine": "q35,i8042=off"}

    def test_a_model_that_qemu_knows_is_kept(self):
        self.assertEqual(harness.usable_tcg_cpu({**self.guest, "tcg_cpu": "max,pmu=off"}), "max,pmu=off")

    def test_an_unknown_property_falls_back_to_max(self):
        self.assertEqual(harness.usable_tcg_cpu({**self.guest, "tcg_cpu": "max,no-such-property=on"}), "max")

    def test_a_missing_emulator_falls_back_to_max(self):
        self.assertEqual(
            harness.usable_tcg_cpu({**self.guest, "qemu": "qemu-system-none", "tcg_cpu": "max,x=y"}), "max"
        )


class DeviceWait(unittest.TestCase):
    """An emulated guest gets systemd credentials (SMBIOS type 11) that give every device unit a 10-minute wait, in the
    initrd and, through a copy to /run/systemd/system, in the real root of Fedora 41 and 42 too."""

    def command(self, accel):
        return harness.qemu_command(shipped_guest("arm64"), accel, 4096, 4, "d", "s", 2222, "q", "ser", ("c", "v"))

    def credentials(self):
        import base64

        command = self.command("tcg")
        option = command[command.index("-smbios") + 1]
        self.assertTrue(option.startswith("type=11,value="))
        found = {}
        for value in option.split(",")[1:]:  # a comma inside a value would end the QEMU option
            prefix = "value=io.systemd.credential.binary:"
            self.assertTrue(value.startswith(prefix), value)
            name, _, encoded = value[len(prefix) :].partition("=")
            found[name] = base64.b64decode(encoded).decode()
        return found

    def test_tcg_passes_the_drop_in_and_kvm_does_not(self):
        credentials = self.credentials()
        self.assertIn("JobRunningTimeoutSec=10min", credentials["systemd.unit-dropin.dev-.device"])
        self.assertNotIn("-smbios", self.command("kvm"))

    def test_the_initrd_copies_the_drop_in_where_the_real_root_reads_it(self):
        credentials = self.credentials()
        copy = credentials["systemd.unit-dropin.initrd-switch-root.service"]
        self.assertIn("ImportCredential=systemd.unit-dropin.dev-.device", copy)
        steps = [line.split("=", 1)[1] for line in copy.splitlines() if line.startswith("ExecStartPre=")]
        self.assertTrue(steps and all(step.startswith("-") for step in steps), "a failed copy must not stop the switch")
        self.assertTrue(steps[-1].endswith("/run/systemd/system/dev-.device.d/50-gnoblin-device-wait.conf"))


# Run 41867851183, Fedora 42 aarch64: the serial console of a guest that went into emergency mode. systemd's own
# messages went to tty1, so "You are in emergency mode" is not there.
FEDORA_42_EMERGENCY = """\
[   77.789696] audit: type=1130 audit(1793810755.633:24): pid=1 uid=0 auid=4294967295 ses=4294967295 \
subj=system_u:system_r:init_t:s0 msg='unit=systemd-zram-setup@zram0 comm="systemd" exe="/usr/lib/systemd/systemd" \
hostname=? addr=? terminal=? res=success'
[  106.756924] audit: type=1130 audit(1793810784.600:25): pid=1 uid=0 auid=4294967295 ses=4294967295 \
subj=system_u:system_r:init_t:s0 msg='unit=emergency comm="systemd" exe="/usr/lib/systemd/systemd" hostname=? \
addr=? terminal=? res=success'
[  106.810872] Adding 3982332k swap on /dev/zram0.  Priority:100 extents:1 across:3982332k SSDsc
[  122.132037] audit: type=1334 audit(1793810799.962:30): prog-id=39 op=LOAD

Cannot open access to console, the root account is locked.
See sulogin(8) man page for more details.

Press Enter to continue.
[  148.364370] BTRFS info (device vda3): resize device /dev/vda3 (devid 1) from 4214206464 to 16025366528
"""

# The same guest in run 41835867674, where it booted: the lines of the same stretch of the boot.
FEDORA_42_BOOT = """\
[  103.862037] systemd[1]: Listening on systemd-initctl.socket - initctl Compatibility Named Pipe.
[  115.742344] audit: type=1130 audit(1793799028.388:24): pid=1 uid=0 auid=4294967295 ses=4294967295 \
subj=system_u:system_r:init_t:s0 msg='unit=systemd-zram-setup@zram0 comm="systemd" exe="/usr/lib/systemd/systemd" \
hostname=? addr=? terminal=? res=success'
[  119.700024] audit: type=1130 audit(1793799032.345:25): pid=1 uid=0 auid=4294967295 ses=4294967295 \
subj=system_u:system_r:init_t:s0 msg='unit=systemd-random-seed comm="systemd" exe="/usr/lib/systemd/systemd" \
hostname=? addr=? terminal=? res=success'
[  120.196339] audit: type=1130 audit(1793799032.839:26): pid=1 uid=0 auid=4294967295 ses=4294967295 \
subj=system_u:system_r:init_t:s0 msg='unit=systemd-binfmt comm="systemd" exe="/usr/lib/systemd/systemd" hostname=? \
addr=? terminal=? res=success'
[  123.141144] virtio_net virtio3 enp0s4: renamed from eth0
"""


class StuckForms(unittest.TestCase):
    """Each form of a boot that gave up matches, and a boot that goes on does not."""

    def test_the_fedora_42_emergency_matches_at_its_first_sign(self):
        found = harness.STUCK.search(FEDORA_42_EMERGENCY)
        self.assertIsNotNone(found)
        self.assertIn("unit=emergency", found.group(0))  # the audit record at 106 s, before sulogin at 122 s

    def test_sulogin_alone_matches(self):
        text = (
            FEDORA_42_EMERGENCY.split("[  106.756924]")[0]
            + "Cannot open access to console, the root account is locked."
        )
        self.assertIn("root account is locked", harness.STUCK.search(text).group(0))

    def test_other_forms_match(self):
        for line in (
            'You are in emergency mode. After logging in, type "journalctl -xb" to view system logs,',
            'You are in rescue mode. After logging in, type "journalctl -xb" to view system logs,',
            "Entering emergency mode. Exit the shell to continue.",
            "Give root password for maintenance",
            "[  100.1] systemd[1]: Failed to start up manager.",
            "[  100.2] systemd[1]: Freezing execution.",
            "[  100.3] Kernel panic - not syncing: Attempted to kill init! exitcode=0x00000100",
        ):
            self.assertIsNotNone(harness.STUCK.search(line), line)

    def test_a_boot_that_goes_on_does_not_match(self):
        self.assertIsNone(harness.STUCK.search(FEDORA_42_BOOT))
        record = next(line for line in FEDORA_42_EMERGENCY.splitlines() if "unit=emergency" in line)
        stopped = record.replace("type=1130", "type=1131")  # the service stopped, not started
        self.assertIsNone(harness.STUCK.search(stopped))


# Run 41947714180, Arch Linux aarch64 (failed): a module load oopses, and the next modprobe spins on the lock that the
# dead task held. The watchdog reports the same CPU every 22 s, until the boot timeout (1647 s).
ARCH_LOCKUP = """\
[   39.246038] Internal error: Oops: 0000000096000047 [#1]  SMP
[   96.225430] watchdog: BUG: soft lockup - CPU#0 stuck for 26s! [modprobe:398]
[   96.226003] CPU: 0 UID: 0 PID: 398 Comm: modprobe Tainted: G      D             7.3.1-2-aarch64-ARCH #1 PREEMPT(full)
[   96.226607]  queued_spin_lock_slowpath+0x38/0x5e0 (P)
[   96.226638]  idempotent_init_module+0xa4/0x2b0
[  120.225194] watchdog: BUG: soft lockup - CPU#0 stuck for 48s! [modprobe:398]
[  168.225223] watchdog: BUG: soft lockup - CPU#0 stuck for 93s! [modprobe:398]
[  192.225181] watchdog: BUG: soft lockup - CPU#0 stuck for 116s! [modprobe:398]
[  372.225235] watchdog: BUG: soft lockup - CPU#0 stuck for 283s! [modprobe:398]
[  396.225225] watchdog: BUG: soft lockup - CPU#0 stuck for 306s! [modprobe:398]
[  420.225250] watchdog: BUG: soft lockup - CPU#0 stuck for 328s! [modprobe:398]
"""

# Run 41918451629, Fedora 44 aarch64 (passed): the longest soft lockup that recovered in the arm64 legs of runs
# 41835867674 to 41947714180, then a short one on another CPU, then the boot went on.
FEDORA_44_LOCKUP = """\
[   96.998942] watchdog: BUG: soft lockup - CPU#3 stuck for 34s! [systemd:1]
[  123.786103] watchdog: BUG: soft lockup - CPU#3 stuck for 59s! [systemd:1]
[  148.986899] watchdog: BUG: soft lockup - CPU#3 stuck for 82s! [systemd:1]
[  174.877227] watchdog: BUG: soft lockup - CPU#3 stuck for 106s! [systemd:1]
[  241.306379] watchdog: BUG: soft lockup - CPU#1 stuck for 56s! [systemd:1]
[  OK  ] Reached target multi-user.target - Multi-User System.
"""


class SoftLockup(unittest.TestCase):
    """A soft lockup of at least SOFT_LOCKUP_LIMIT seconds means the guest gave up; a shorter one recovers."""

    def test_the_arch_lockup_matches_at_the_limit(self):
        self.assertEqual(harness.stuck_in(ARCH_LOCKUP), "soft lockup - CPU#0 stuck for 306s")
        before = ARCH_LOCKUP.split("[  396.225225]")[0]
        self.assertIsNone(harness.stuck_in(before))  # 283 s: not yet

    def test_the_longest_lockup_that_recovered_does_not_match(self):
        self.assertIsNone(harness.stuck_in(FEDORA_44_LOCKUP))

    def test_the_limit_is_well_above_every_lockup_that_recovered(self):
        self.assertGreaterEqual(harness.SOFT_LOCKUP_LIMIT, 2 * 106)

    def test_the_other_forms_still_match_through_stuck_in(self):
        self.assertIn("unit=emergency", harness.stuck_in(FEDORA_42_EMERGENCY))
        self.assertIsNone(harness.stuck_in(FEDORA_42_BOOT))

    def test_a_long_lockup_fails_the_boot_at_once(self):
        work = Path(tempfile.mkdtemp())
        (work / "serial.log").write_text(ARCH_LOCKUP)
        guest = harness.Guest.__new__(harness.Guest)
        guest.out, guest.process = work, type("Running", (), {"poll": lambda self: None})()
        guest.root = lambda *a, **k: (1, "", "")
        started = time.monotonic()
        with self.assertRaisesRegex(harness.GuestStuck, "stuck for 306s"):
            guest.wait_ssh(8)
        self.assertLess(time.monotonic() - started, 5)


class BootRetry(unittest.TestCase):
    """A guest whose serial console shows that it gave up booting fails at once. An emulated guest boots once more
    from a fresh disk (run 41770211755, Fedora 44 aarch64: systemd froze after a soft lockup); a KVM guest does not."""

    def guest_with_serial(self, text):
        work = Path(tempfile.mkdtemp())
        (work / "serial.log").write_text(text)
        guest = harness.Guest.__new__(harness.Guest)
        guest.out, guest.process = work, type("Running", (), {"poll": lambda self: None})()
        guest.root = lambda *a, **k: (1, "", "")
        return guest

    def test_emergency_mode_fails_at_once(self):
        guest = self.guest_with_serial("...\nYou are in emergency mode. After logging in, type ...\n")
        started = time.monotonic()
        with self.assertRaisesRegex(harness.GuestStuck, "emergency mode"):
            guest.wait_ssh(600)
        self.assertLess(time.monotonic() - started, 5)

    def test_the_fedora_42_serial_fails_at_once(self):
        guest = self.guest_with_serial(FEDORA_42_EMERGENCY)
        started = time.monotonic()
        with self.assertRaisesRegex(harness.GuestStuck, "unit=emergency"):  # not "no SSH login within 8 s"
            guest.wait_ssh(8)
        self.assertLess(time.monotonic() - started, 5)

    def test_the_screen_is_kept_as_png(self):
        guest = self.guest_with_serial("")

        class Qmp:
            def command(self, name, filename):
                Path(filename).write_bytes(b"P6\n2 1\n255\n" + bytes((255, 0, 255, 0, 0, 0)))

        guest.qmp = Qmp()
        guest.keep_screen()
        data = (guest.out / "boot-screen.png").read_bytes()
        self.assertEqual(struct.unpack(">II", data[16:24]), (2, 1))
        start = data.index(b"IDAT") + 4
        length = struct.unpack(">I", data[start - 8 : start - 4])[0]
        self.assertEqual(zlib.decompress(data[start : start + length])[1:4], bytes((255, 0, 255)))
        self.assertFalse((guest.out / "boot-screen.ppm").exists())

    def run_boot(self, accel, stuck_boots):
        calls = []

        class Guest:
            key = Path(tempfile.mkdtemp()) / "id"

            def __init__(self):
                self.accel = accel
                self.key.with_suffix(".pub").write_text("ssh-ed25519 AAAA test")

            def start(self, base, seed):
                calls.append("start")

            def restart(self, base, seed, reason):
                calls.append("restart")

            def wait_ssh(self, timeout):
                calls.append("wait")
                if calls.count("wait") <= stuck_boots:
                    raise harness.GuestStuck("the guest stopped booting: Freezing execution")

            def root(self, command, **options):
                return 0, "6.0\n", ""

        work = Path(tempfile.mkdtemp())
        run = harness.Run(argparse.Namespace(out=str(work), breaks=[], boot_timeout=1), {"name": "x", "image": {}})
        run.work, run.guest, run.image_info = work, Guest(), {"path": "/nonexistent.qcow2"}
        original = harness.make_seed
        harness.make_seed = lambda directory, key: directory / "seed.iso"
        try:
            return run.boot(), calls, run.facts
        finally:
            harness.make_seed = original

    def test_an_emulated_guest_boots_once_more(self):
        detail, calls, facts = self.run_boot("tcg", 1)
        self.assertEqual(calls, ["start", "wait", "restart", "wait"])
        self.assertEqual(facts["boot_retry"], "the guest stopped booting: Freezing execution")
        self.assertIn("kernel 6.0", detail)

    def test_only_once(self):
        with self.assertRaises(harness.GuestStuck):
            self.run_boot("tcg", 2)

    def test_a_kvm_guest_does_not_boot_again(self):
        with self.assertRaises(harness.GuestStuck):
            self.run_boot("kvm", 1)


# Run 41905248345, openSUSE Tumbleweed aarch64: Gnoblin's lines in the test user's journal. Mutter logs nothing after
# native control starts, the worker gives up waiting for its HELLO, and the supervisor cannot suspend Mutter.
TUMBLEWEED_LOST = """\
[  593.468374] gnoblin-graphical sh[2555]: libmutter-Message: 20:44:03.689: Using Wayland display name 'wayland-0'
[  593.570404] gnoblin-graphical sh[2555]: libmutter-Message: 20:44:03.794: gnoblin-compositor: scheduling policy \
0->0, nice 0->0, io 0->0
[  598.437903] gnoblin-graphical sh[2664]: gnoblin: runtime supervisor: compositor did not complete the private \
runtime handshake
[  618.533133] gnoblin-graphical sh[2555]: gnoblin: could not suspend Mutter for supervisor recovery; keeping the \
session alive: Mutter did not acknowledge worker suspension
"""

# Run 41896060198, the same guest where the session came up: its lines of the same stretch.
TUMBLEWEED_RUNNING = """\
[  589.835181] gnoblin-graphical sh[2553]: libmutter-Message: 19:45:46.090: Using Wayland display name 'wayland-0'
[  589.937197] gnoblin-graphical sh[2553]: libmutter-Message: 19:45:46.197: gnoblin-compositor: scheduling policy \
0->0, nice 0->0, io 0->0
[  590.337346] gnoblin-graphical sh[2553]: (gnoblin:2646): libmutter-WARNING **: 19:45:46.630: There is no colord \
server available
"""


class SessionCheck(unittest.TestCase):
    """A session that can no longer come up fails at once, with Gnoblin's own lines, the reason gnoblinctl gives, the
    screen and where each compositor thread is. A session that is still starting is waited for."""

    def test_a_lost_runtime_matches(self):
        found = harness.RUNTIME_LOST.search(TUMBLEWEED_LOST)
        self.assertTrue(found[0].startswith("gnoblin: could not suspend Mutter for supervisor recovery"))
        self.assertTrue(found[0].endswith("Mutter did not acknowledge worker suspension"))

    def test_a_runtime_that_can_still_recover_does_not_match(self):
        handshake = TUMBLEWEED_LOST.split("[  618.533133]")[0]
        for text in (handshake, TUMBLEWEED_RUNNING, "gnoblin: restarting Lua runtime worker (attempt 1 of 3)"):
            self.assertIsNone(harness.RUNTIME_LOST.search(text), text)

    def run_session(self, journal, timeout, gnoblinctl=(1, "", "gnoblinctl: cannot connect to the session\n")):
        commands = []

        class Guest:
            def root(self, command, **options):
                commands.append(command)
                if "pgrep" in command and "guest.sh" not in command:
                    # The first poll comes before the compositor exists (exit 3), as on a real guest.
                    return (3 if sum("pgrep" in c for c in commands) == 1 else 0), "", ""
                if "guest.sh status" in command:
                    return 0, "running\n", ""
                if "guest.sh threads" in command:
                    return 0, "compositor 2646: /usr/lib/gnoblin/bin/gnoblin --wayland\n", ""
                if "journalctl" in command:
                    return 0, journal, ""
                return 0, "", ""

            def user(self, command, **options):
                return gnoblinctl

            def keep_screen(self, settle=0, name="boot-screen"):
                commands.append(f"screen {name}")

        work = Path(tempfile.mkdtemp())
        run = harness.Run(argparse.Namespace(out=str(work), breaks=[], session_timeout=timeout), {"name": "x"})
        run.guest = Guest()
        started = time.monotonic()
        with self.assertRaises(harness.CheckFailed) as failed:
            run.session()
        return failed.exception, time.monotonic() - started, commands, work

    def test_a_lost_runtime_fails_at_once_with_its_line(self):
        failure, seconds, commands, work = self.run_session(TUMBLEWEED_LOST, 10)  # not "did not reach running in 10 s"
        self.assertLess(seconds, 5)
        self.assertIn("the Lua runtime is lost", failure.detail)
        self.assertIn("Mutter did not acknowledge worker suspension", failure.detail)
        self.assertIn("did not complete the private runtime handshake", failure.excerpt)
        self.assertIn("screen session-screen", commands)
        self.assertIn("gnoblin --wayland", (work / "compositor-threads.txt").read_text())

    def test_a_session_that_is_still_starting_waits_and_names_the_gnoblinctl_error(self):
        failure, seconds, _, _ = self.run_session(TUMBLEWEED_RUNNING, 3)
        self.assertGreaterEqual(seconds, 3)
        self.assertIn("did not reach running in 3 s", failure.detail)
        self.assertIn("gnoblinctl failed (1): gnoblinctl: cannot connect to the session", failure.detail)


class CompositorState(unittest.TestCase):
    """Run 41918451629, Alpine 3.24 amd64: after the window was mapped, the pointer, keyboard and shutdown checks failed
    with no message from the compositor. A check that needs the compositor now says whether it still answers, and the
    first such failure keeps the screen and where each compositor thread is."""

    def run_checks(self, status_code, names=("pointer", "keyboard")):
        commands = []

        class Guest:
            def user(self, command, **options):
                commands.append(command)
                return (
                    (status_code, "", "gnoblinctl: the session is gone\n")
                    if "gnoblinctl status" in command
                    else (0, "", "")
                )

            def root(self, command, **options):
                commands.append(command)
                return 0, "compositor 3873: /usr/lib/gnoblin/bin/gnoblin --wayland\n1 gnoblin S 5 3 do_epoll_wait\n", ""

            def keep_screen(self, settle=0, name="boot-screen"):
                commands.append(f"screen {name}")

        work = Path(tempfile.mkdtemp())
        run = harness.Run(argparse.Namespace(out=str(work), breaks=[]), {"name": "x"})
        run.guest = Guest()

        def failing():
            raise harness.CheckFailed("the pointer never entered the client window")

        for name in names:
            run.check(name, failing)
        return run, commands, work

    def test_a_stalled_compositor_is_named_and_its_threads_kept_once(self):
        run, commands, work = self.run_checks(124)
        details = [item["detail"] for item in run.checks]
        self.assertTrue(all(d.endswith("; the compositor does not answer gnoblinctl within 10 s") for d in details))
        self.assertEqual([c for c in commands if c.startswith("screen")], ["screen pointer-screen"])
        self.assertEqual(sum("guest.sh threads" in c for c in commands), 1)
        self.assertTrue((work / "compositor-threads.txt").read_text().startswith("after the pointer check failed\n"))

    def test_a_compositor_that_answers_says_so(self):
        run, _, _ = self.run_checks(0)
        self.assertTrue(run.checks[0]["detail"].endswith("; the compositor answers gnoblinctl"))

    def test_another_error_is_quoted(self):
        run, _, _ = self.run_checks(1)
        self.assertTrue(
            run.checks[0]["detail"].endswith("; gnoblinctl status exited 1: gnoblinctl: the session is gone")
        )

    def test_a_check_before_the_session_does_not_ask(self):
        run, commands, work = self.run_checks(124, names=("install",))
        self.assertEqual(run.checks[0]["detail"], "the pointer never entered the client window")
        self.assertEqual(commands, [])
        self.assertFalse((work / "compositor-threads.txt").exists())


class PollEvents(unittest.TestCase):
    """Run 41672380634: on TCG the client's events come later than the fixed waits of a KVM guest. The checks poll
    until the event they need arrives, up to the client timeout."""

    def run_for(self, timeout):
        run = harness.Run(argparse.Namespace(out=tempfile.mkdtemp(), breaks=[], client_timeout=timeout), {"name": "x"})
        started = time.monotonic()
        arrivals = [[], [], [{"event": "pointer-button", "pressed": False}]]
        run.client_events = lambda: arrivals.pop(0) if len(arrivals) > 1 else arrivals[0]
        return run, started

    def test_it_returns_when_the_event_arrives(self):
        run, started = self.run_for(30)
        events = run.poll_events(lambda found: any(event["event"] == "pointer-button" for event in found))
        self.assertEqual(events, [{"event": "pointer-button", "pressed": False}])
        self.assertLess(time.monotonic() - started, 10)

    def test_it_gives_up_at_the_client_timeout(self):
        run, started = self.run_for(2)
        events = run.poll_events(lambda found: False)
        self.assertGreaterEqual(time.monotonic() - started, 2)
        self.assertLess(time.monotonic() - started, 6)
        self.assertIsInstance(events, list)


class DownloadRetry(unittest.TestCase):
    """A download that does not match its checksum is fetched again, a bounded number of times (run 41672380634)."""

    GOOD = b"the image\n"

    def download(self, bodies, corrupt=False):
        import hashlib
        import io

        served = list(bodies)
        calls = []
        expected = hashlib.sha256(self.GOOD).hexdigest()

        class Response(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

        def urlopen(request, timeout):
            calls.append(request.full_url)
            return Response(served.pop(0))

        originals = (harness.resolve_image, harness.urllib.request.urlopen, harness.time.sleep)
        harness.resolve_image = lambda image: ("https://mirror.invalid/i.qcow2", "i.qcow2", "SUMS", "sha256", expected)
        harness.urllib.request.urlopen = urlopen
        harness.time.sleep = lambda seconds: None
        try:
            return harness.download_image({}, Path(tempfile.mkdtemp()), 0, corrupt), calls
        finally:
            harness.resolve_image, harness.urllib.request.urlopen, harness.time.sleep = originals

    def test_a_short_download_is_fetched_again(self):
        info, calls = self.download([b"the ima", self.GOOD])
        self.assertEqual(len(calls), 2)
        self.assertEqual(Path(info["path"]).read_bytes(), self.GOOD)

    def test_a_file_that_never_matches_fails_after_three_downloads(self):
        with self.assertRaisesRegex(harness.CheckFailed, "after 3 downloads"):
            self.download([b"x", b"y", b"z"])

    def test_a_deliberate_break_does_not_retry(self):
        with self.assertRaises(harness.CheckFailed):
            self.download([self.GOOD, self.GOOD], corrupt=True)


class Deadline(unittest.TestCase):
    def test_the_check_at_the_deadline_fails_and_the_rest_are_skipped(self):
        out = Path(tempfile.mkdtemp())
        arguments = argparse.Namespace(out=str(out), breaks=[], deadline=1)
        run = harness.Run(arguments, {"name": "x"})
        signal.signal(signal.SIGALRM, run.expire)
        signal.alarm(1)
        try:
            self.assertFalse(run.check("boot", lambda: time.sleep(5)))
        finally:
            signal.alarm(0)
        self.assertFalse(run.check("install", lambda: "never runs"))
        boot, install = run.checks
        self.assertEqual((boot["status"], boot["detail"]), ("fail", "the run reached its deadline of 1 s"))
        self.assertLess(boot["seconds"], 3)
        self.assertEqual(install["status"], "skip")


if __name__ == "__main__":
    unittest.main()
