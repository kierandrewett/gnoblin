#!/usr/bin/env python3
"""Boot one system release in a QEMU guest, start a real Gnoblin session in it and check that it renders.

usage: harness.py (--release NAME | --entry FILE) --packages SOURCE --out DIR [options]

NAME is a graphical target of scripts/supported-targets.py: its name ("Fedora 42", "Arch Linux") or its slug (fc42).
--entry reads the target from a JSON file instead, as the workflow does with its matrix entry. SOURCE is a folder of
package files (the build artifacts), or "nightly" or "stable" for that channel of repo.gnoblin.org.

The checks, in order. design/graphical-testing.md lists how each one fails.

  packaged    the release has Gnoblin packages. Without, the result is "not packaged" with the blocker
  image       the cloud image of the release downloads and matches the distribution's checksum file
  boot        the guest boots, cloud-init finishes, and SSH reaches it
  install     the graphics stack and the Gnoblin packages install
  session     the session starts on tty1: the compositor runs, holds a DRM device (the native KMS backend) and
              gnoblinctl status reports the session state "running"
  globals     the Wayland globals in tests/graphical/expected-globals.txt are announced
  window      the test client (wlclient.py) maps its window, and gnoblinctl lists it
  screenshot  a QMP screendump, taken outside the guest, is not blank and shows the client's colour at its size
  pointer     QMP pointer events move the pointer into the window, by the right distance, and click it
  keyboard    QMP key events reach the client in order, and the client's repaint shows in a second screendump
  shutdown    gnoblinctl logout ends the session with status 0, the compositor exits and nothing dumped core
  logs        no CRITICAL or ERROR line in the session log or the user's journal, beyond the allow-list

Every run writes its evidence to --out: result.json (each check, its status, detail and time), command.txt, the QEMU
version, the guest versions, serial.log, journal.txt, session.log, the screenshots as PNG and the client's files.

--break CHECK makes the guest fail that check on purpose, to prove that the check can fail. See the design note.
Exit status: 0 pass, 1 fail, 3 not packaged, 2 usage or harness error.
"""

import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.request
import zlib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CHECKS = (
    "packaged",
    "image",
    "boot",
    "install",
    "session",
    "globals",
    "window",
    "screenshot",
    "pointer",
    "keyboard",
    "shutdown",
    "logs",
)
# What the client draws (wlclient.py). The screenshot check looks for these colours.
FIRST_COLOUR, SECOND_COLOUR = (255, 0, 255), (0, 200, 0)
CLIENT_SIZE = (480, 360)
APP_ID = "org.gnoblin.GraphicalTest"
# pgrep -f patterns of the compositor (the session spawns "<prefix>/bin/gnoblin --wayland ...") and of the test client.
# They are anchored at the start of the command line, so they never match the shell that runs pgrep.
COMPOSITOR = "^/[^ ]*/gnoblin --wayland"
CLIENT = "^python3 [^ ]*wlclient\\.py"
USER = "gtest"
GUEST_DIR = "/var/tmp/gnoblin-graphical"
SCREEN = (1280, 800)
# The keys typed into the client: QMP qcode and the evdev code that the client must see. G makes it repaint.
TYPED = (("g", 34), ("n", 49), ("o", 24), ("b", 48))
BTN_LEFT = 272
POINTER_STEP = (40, 30)
ALLOW_LISTS = (ROOT / "tests/packages/log-allowlist.txt", HERE / "log-allowlist.txt")
LOG_LEVEL = re.compile(r"(?<![A-Za-z])(CRITICAL|ERROR)(?![A-Za-z])")
# The systemd credential of an emulated guest: a drop-in for every device unit (prefix "dev-") that waits 10 minutes.
DEVICE_WAIT_DROPIN = "[Unit]\nJobRunningTimeoutSec=10min\nJobTimeoutSec=10min\n"
DEVICE_WAIT_CREDENTIAL = "systemd.unit-dropin.dev-.device=" + base64.b64encode(DEVICE_WAIT_DROPIN.encode()).decode()

# The guest of an entry without "guest" data: an amd64 guest on the default BIOS. scripts/targets.json gives every
# architecture its own data (architectures[].guest).
DEFAULT_GUEST = {
    "cpu": "x86_64",
    "boot": "bios",
    "qemu": "qemu-system-x86_64",
    "machine": "q35,i8042=off,vmport=off",
    "gpu": "virtio-vga",
    "firmware": [],
}


class CheckFailed(Exception):
    def __init__(self, detail, excerpt=""):
        super().__init__(detail)
        self.detail, self.excerpt = detail, excerpt


class Deadline(CheckFailed):
    """The run reached --deadline. The check that runs fails with it, and the checks after it are skipped."""


class GuestStuck(CheckFailed):
    """The serial console shows that the guest gave up booting, so SSH will never come up."""


# What a guest prints on its serial console when it has given up booting.
STUCK = re.compile(r"You are in emergency mode|Freezing execution|Kernel panic - not syncing")


def guest_of(entry):
    return entry.get("guest") or DEFAULT_GUEST


def find_firmware(guest):
    """The first UEFI code and vars pair of the guest data whose files exist, or None for a guest on the default BIOS."""
    pairs = guest.get("firmware") or []
    if not pairs:
        return None
    for pair in pairs:
        if Path(pair["code"]).is_file() and Path(pair["vars"]).is_file():
            return Path(pair["code"]), Path(pair["vars"])
    tried = ", ".join(pair["code"] for pair in pairs)
    raise CheckFailed(f"no UEFI firmware for {guest['cpu']}: none of {tried} exists")


def usable_tcg_cpu(guest):
    """The guest's tcg_cpu when this QEMU accepts it, else "max". A QEMU that does not know a property of the CPU
    model stops at once; one that knows it keeps running, stopped (-S), until it is killed. The fallback is printed."""
    wanted = guest.get("tcg_cpu", "max")
    if wanted == "max":
        return wanted
    machine = guest["machine"].split(",")[0]
    probe = [guest["qemu"], "-machine", f"{machine},accel=tcg", "-cpu", wanted, "-nodefaults", "-display", "none", "-S"]
    try:
        result = subprocess.run(probe, capture_output=True, text=True, timeout=3)
    except subprocess.TimeoutExpired:
        return wanted
    except OSError as error:
        print(f"[graphical] cannot check the CPU model {wanted}: {error}; using max", flush=True)
        return "max"
    print(f"[graphical] QEMU refuses the CPU model {wanted} ({tail(result.stderr, 2)}); using max", flush=True)
    return "max"


def qemu_command(guest, accel, memory, cpus, disk, seed, port, qmp_path, serial, firmware=None):
    """The QEMU command line of one guest. guest is the guest data of the architecture (scripts/targets.json):
    the emulator, the machine, the GPU device and, for a UEFI guest, firmware as (code, writable vars copy)."""
    command = [
        guest["qemu"],
        "-name", "gnoblin-graphical",
        "-machine", f"{guest['machine']},accel={accel}",
        # Without KVM the CPU model comes from the guest data. On aarch64 it is max with pauth-impdef=on: TCG emulates
        # the architected pointer authentication (QARMA) very slowly, and every function return of the guest pays for
        # it. The implementation-defined algorithm is what QEMU documents for a fast TCG guest. (Run 41707260240:
        # Fedora 41, 42, 43 and 45 aarch64 missed systemd's 90 s wait for their disks, and Fedora 44 passed at 627 s.)
        "-cpu", "host" if accel == "kvm" else guest.get("tcg_cpu", "max"),
        "-smp", str(cpus),
        "-m", str(memory),
    ]  # fmt: skip
    if accel != "kvm":
        # systemd waits 90 s for a device of /etc/fstab, and an emulated guest can need longer to find its disks
        # (run 41770211755: Fedora 43 aarch64, "Timed out waiting for device", then emergency mode). The cloud image
        # is not changed: a systemd credential in SMBIOS type 11 gives every device unit a drop-in (the dev- prefix,
        # through systemd-debug-generator, systemd 256 and newer) with a 10-minute wait. An older systemd ignores it.
        command += ["-smbios", f"type=11,value=io.systemd.credential.binary:{DEVICE_WAIT_CREDENTIAL}"]
    if firmware:
        code, variables = firmware
        command += [
            "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
            "-drive", f"if=pflash,format=raw,unit=1,file={variables}",
        ]  # fmt: skip
    command += [
        "-drive", f"file={disk},if=virtio,format=qcow2,discard=unmap",
        "-drive", f"file={seed},if=virtio,format=raw,readonly=on",
        # virtio-gpu without virgl: the guest renders with Mesa's llvmpipe, and QMP screendump reads the scanout.
        "-device", f"{guest['gpu']},xres={SCREEN[0]},yres={SCREEN[1]}",
        "-display", "none",
        # No PS/2 controller (i8042=off on q35, none on virt): the virtio keyboard and tablet are the only input
        # devices, so QMP input reaches them.
        "-device", "virtio-keyboard-pci,id=keyboard",
        "-device", "virtio-tablet-pci,id=tablet",
        "-netdev", f"user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22",
        # No option ROM: the guest boots from its disk, never from the network. QEMU on an Ubuntu arm64 runner has no
        # efi-virtio.rom (it is in ipxe-qemu, a recommended package), and stops at start when the device asks for it.
        "-device", "virtio-net-pci,netdev=net0,romfile=",
        "-qmp", f"unix:{qmp_path},server=on,wait=off",
        "-serial", f"file:{serial}",
    ]  # fmt: skip
    if not firmware:
        command += ["-boot", "order=c"]
    return command


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")


def tail(text, lines=40):
    return "\n".join(text.strip().splitlines()[-lines:])


# ----------------------------------------------------------------------------------------------------------------------
# Images


NETWORK_ERRORS = (urllib.error.URLError, TimeoutError, ConnectionError)


def fetch(url, timeout=120):
    """The body of url and the URL it came from after redirects (a mirror)."""
    request = urllib.request.Request(url, headers={"User-Agent": "gnoblin-graphical-test"})
    for attempt in (1, 2, 3):
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                return response.read(), response.geturl()
        except urllib.error.HTTPError:
            raise
        except NETWORK_ERRORS:
            if attempt == 3:
                raise
            time.sleep(5 * attempt)


def version_numbers(name):
    return tuple(int(number) for number in re.findall(r"\d+", name))


def checksum_of(text, file_name, own_file=False):
    """The checksum of file_name in a checksum file: GNU ("HEX  name", "HEX *name") or BSD ("SHA256 (name) = HEX")
    style. Fedora signs its file, so lines around the checksums are skipped. own_file: the checksum file belongs to
    this one image ({file} in the image data), and may hold the bare digest only, as Alpine's .sha512 files do."""
    bare = re.fullmatch(r"\s*([0-9a-fA-F]{64}|[0-9a-fA-F]{128})\s*", text)
    if own_file and bare:
        return ("sha256" if len(bare.group(1)) == 64 else "sha512"), bare.group(1).lower()
    for line in text.splitlines():
        line = line.strip()
        bsd = re.fullmatch(r"(SHA\d+) \((.+)\) = ([0-9a-fA-F]+)", line)
        if bsd and bsd.group(2) == file_name:
            return bsd.group(1).lower(), bsd.group(3).lower()
        gnu = re.fullmatch(r"([0-9a-fA-F]{64}|[0-9a-fA-F]{128})\s+\*?(.+)", line)
        if gnu and gnu.group(2) == file_name:
            return ("sha256" if len(gnu.group(1)) == 64 else "sha512"), gnu.group(1).lower()
    raise CheckFailed(f"the checksum file has no line for {file_name}")


def resolve_image(image):
    """The newest image that the target's image data names, from the first index that lists it with its checksum
    file: (URL, file name, checksum file URL, algorithm, checksum). Each index is read through one mirror: a redirect
    is followed once, and the image and the checksum file come from the same place."""
    errors = []
    for index in image["index"]:
        try:
            listing, base = fetch(index)
            names = set(re.findall(r'href="(?:\./)?([^"/?#]+)"', listing.decode(errors="replace")))
            matches = sorted((name for name in names if re.fullmatch(image["file"], name)), key=version_numbers)
            if not matches:
                errors.append(f"{index}: no file matches {image['file']}")
                continue
            file_name = matches[-1]
            checksum_pattern = image["checksum"].replace("{file}", re.escape(file_name))
            sums = sorted((name for name in names if re.fullmatch(checksum_pattern, name)), key=version_numbers)
            if not sums:
                errors.append(f"{index}: no checksum file matches {checksum_pattern}")
                continue
            text, _ = fetch(base + sums[-1])
            own_file = "{file}" in image["checksum"]
            algorithm, expected = checksum_of(text.decode(errors="replace"), file_name, own_file)
            return base + file_name, file_name, base + sums[-1], algorithm, expected
        except NETWORK_ERRORS as exception:
            errors.append(f"{index}: {exception}")
        except CheckFailed as failure:
            errors.append(f"{index}: {failure.detail}")
    raise CheckFailed("no image with a checksum found", "\n".join(errors))


def download_image(image, cache, min_free_gb, corrupt=False, attempts=3):
    """Download the newest image and check it against the checksum file. A download that does not match is removed
    and fetched again, up to attempts times in all, each time with the reason printed: a mirror can end a transfer
    early without an error, or serve a file that its checksum file does not list yet (run 41672380634, Fedora 41
    aarch64). A file that never matches fails the check. --break image (corrupt) does not retry."""
    for attempt in range(1, attempts + 1):
        url, file_name, sums_url, algorithm, expected = resolve_image(image)
        if corrupt:
            expected = "0" * len(expected)
        cache.mkdir(parents=True, exist_ok=True)
        path = cache / file_name
        if not path.exists():
            free = shutil.disk_usage(cache).free / 2**30
            if free < min_free_gb:
                raise CheckFailed(f"only {free:.1f} GiB free in {cache}, below the {min_free_gb} GiB floor")
            partial = path.with_suffix(path.suffix + ".part")
            request = urllib.request.Request(url, headers={"User-Agent": "gnoblin-graphical-test"})
            try:
                with urllib.request.urlopen(request, timeout=120) as response, partial.open("wb") as handle:
                    shutil.copyfileobj(response, handle, 1 << 20)
            except NETWORK_ERRORS as exception:
                partial.unlink(missing_ok=True)
                raise CheckFailed(f"the download of {url} failed: {exception}") from None
            partial.rename(path)
        digest = hashlib.new(algorithm)
        with path.open("rb") as handle:
            for block in iter(lambda: handle.read(1 << 20), b""):
                digest.update(block)
        if digest.hexdigest() == expected:
            return {"url": url, "file": file_name, "checksum_url": sums_url, algorithm: expected, "path": str(path)}
        problem = f"{file_name}: {algorithm} {digest.hexdigest()} does not match {expected} of {sums_url}"
        if corrupt or attempt == attempts:
            raise CheckFailed(problem + (f" (after {attempt} downloads)" if attempt > 1 else ""))
        print(f"[graphical] image download {attempt} of {attempts}: {problem}; downloading again", flush=True)
        path.unlink()
        time.sleep(10 * attempt)


def build_image(build, cache, work, log, corrupt=False, min_free_gb=0.0):
    """The image of a target that publishes none (Arch Linux on arm64): run the build script that the image data
    names, as root when it says so. The script writes cache/<file>, cache/<file>.sha256 and cache/<file>.manifest. The
    digest is read from the .sha256 file and checked again here, so a file that changed after the build fails the
    check. The signature of the root file system archive is checked by the script itself, against a pinned key."""
    file_name = build["file"]
    cache.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(cache).free / 2**30
    if free < min_free_gb:
        raise CheckFailed(f"only {free:.1f} GiB free in {cache}, below the {min_free_gb} GiB floor of the image build")
    command = [str(ROOT / build["script"]), "--out", str(cache), "--name", file_name,
               "--cache", str(cache / "sources"), "--work", str(work / "image-build")]  # fmt: skip
    if build.get("sudo"):
        command = ["sudo", "-n", "-E", *command]
    try:
        with log.open("w") as handle:
            subprocess.run(command, stdout=handle, stderr=subprocess.STDOUT, check=True,
                           timeout=build.get("timeout_minutes", 40) * 60)  # fmt: skip
    except subprocess.CalledProcessError:
        raise CheckFailed(f"the image build {build['script']} failed", tail(log.read_text(errors="replace"))) from None
    except subprocess.TimeoutExpired:
        raise CheckFailed(f"the image build {build['script']} took longer than {build.get('timeout_minutes', 40)} min",
                          tail(log.read_text(errors="replace"))) from None  # fmt: skip
    path = cache / file_name
    sums = cache / (file_name + ".sha256")
    if not path.exists() or not sums.exists():
        raise CheckFailed(f"the image build wrote no {file_name} with a .sha256 file")
    expected = sums.read_text().split()[0].lower()
    if corrupt:
        expected = "0" * len(expected)
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    if digest.hexdigest() != expected:
        raise CheckFailed(f"{file_name}: sha256 {digest.hexdigest()} does not match {expected} of {sums.name}")
    manifest = cache / (file_name + ".manifest")
    return {"url": f"built by {build['script']}", "file": file_name, "checksum_url": sums.name, "sha256": expected,
            "manifest": manifest.read_text() if manifest.exists() else "", "path": str(path)}  # fmt: skip


# ----------------------------------------------------------------------------------------------------------------------
# Screenshots: PPM from QMP screendump in, PNG out, and the pixel properties that the checks use.


def read_ppm(path):
    data = path.read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+(\d+)\s", data)
    if not match:
        raise CheckFailed(f"{path.name} is not a binary PPM")
    width, height = int(match.group(1)), int(match.group(2))
    return width, height, data[match.end() : match.end() + width * height * 3]


def write_png(path, width, height, pixels):
    rows = b"".join(b"\0" + pixels[row * width * 3 : (row + 1) * width * 3] for row in range(height))

    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b"")
    )


def histogram(pixels):
    """Pixel counts of each colour, quantised to 32 levels a channel."""
    counts = {}
    for offset in range(0, len(pixels), 3):
        key = (pixels[offset] >> 3, pixels[offset + 1] >> 3, pixels[offset + 2] >> 3)
        counts[key] = counts.get(key, 0) + 1
    return counts


def colour_region(width, height, pixels, colour, tolerance=12):
    """The pixels within tolerance of colour: how many, and their bounding box (left, top, right, bottom)."""
    red, green, blue = colour
    count = 0
    left, top, right, bottom = width, height, -1, -1
    for y in range(height):
        row = pixels[y * width * 3 : (y + 1) * width * 3]
        for x in range(width):
            offset = x * 3
            if (
                abs(row[offset] - red) <= tolerance
                and abs(row[offset + 1] - green) <= tolerance
                and abs(row[offset + 2] - blue) <= tolerance
            ):
                count += 1
                left, right = min(left, x), max(right, x)
                top, bottom = min(top, y), max(bottom, y)
    return count, (left, top, right, bottom) if count else None


def analyse(path, colour):
    width, height, pixels = read_ppm(path)
    write_png(path.with_suffix(".png"), width, height, pixels)
    counts = histogram(pixels)
    total = width * height
    dominant = max(counts.values()) / total
    significant = sum(1 for count in counts.values() if count >= total * 0.01)
    count, box = colour_region(width, height, pixels, colour)
    return {
        "png": path.with_suffix(".png").name,
        "size": [width, height],
        "colours": len(counts),
        "dominant_share": round(dominant, 4),
        "significant_colours": significant,
        "client_pixels": count,
        "client_box": list(box) if box else None,
    }


def check_frame(properties, colour, name):
    """A frame that is not blank and shows the client: more than one significant colour, and a box of colour of
    the client's size that the colour fills."""
    if properties["significant_colours"] < 2 or properties["dominant_share"] > 0.995:
        raise CheckFailed(
            f"{name} is blank: {properties['colours']} colours, one covers {properties['dominant_share']:.1%}"
        )
    box = properties["client_box"]
    if not box:
        raise CheckFailed(f"{name} has no pixel of the client colour {colour}")
    width, height = box[2] - box[0] + 1, box[3] - box[1] + 1
    expected_w, expected_h = CLIENT_SIZE
    if not (0.9 * expected_w <= width <= 1.1 * expected_w and 0.9 * expected_h <= height <= 1.1 * expected_h):
        raise CheckFailed(f"{name}: the client colour covers {width}x{height}, not about {expected_w}x{expected_h}")
    if properties["client_pixels"] < 0.9 * width * height:
        raise CheckFailed(f"{name}: the client colour fills {properties['client_pixels']} of {width * height} pixels")


# ----------------------------------------------------------------------------------------------------------------------
# QEMU


class Qmp:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(60)
        self.socket.connect(str(path))
        self.file = self.socket.makefile("rwb")
        self.file.readline()
        self.command("qmp_capabilities")

    def command(self, name, **arguments):
        message = {"execute": name}
        if arguments:
            message["arguments"] = arguments
        self.file.write(json.dumps(message).encode() + b"\n")
        self.file.flush()
        while True:
            reply = json.loads(self.file.readline())
            if "event" in reply:
                continue
            if "error" in reply:
                raise CheckFailed(f"QMP {name}: {reply['error'].get('desc')}")
            return reply.get("return")

    def close(self):
        self.socket.close()


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def accelerator(choice):
    usable = os.access("/dev/kvm", os.R_OK | os.W_OK)
    if choice == "kvm" and not usable:
        raise SystemExit("harness: --accel kvm, and /dev/kvm is not usable")
    if choice == "auto":
        return "kvm" if usable else "tcg"
    return choice


def make_seed(directory, public_key):
    """A NoCloud seed: the test user and root accept the run's SSH key. Root logs in with the key only."""
    user_data = f"""#cloud-config
users:
  - name: {USER}
    gecos: Gnoblin graphical test
    # No password, but not locked: "*" rather than "!", which an OpenSSH without PAM (Alpine) refuses for a key login.
    lock_passwd: false
    hashed_passwd: "*"
    ssh_authorized_keys:
      - {public_key}
disable_root: false
ssh_pwauth: false
write_files:
  - path: /etc/ssh/sshd_config.d/90-gnoblin-graphical.conf
    content: |
      PermitRootLogin prohibit-password
runcmd:
  - mkdir -p /root/.ssh
  - chmod 700 /root/.ssh
  - echo '{public_key}' > /root/.ssh/authorized_keys
  - chmod 600 /root/.ssh/authorized_keys
  # An OpenSSH without PAM (Alpine) refuses a key login to a locked account ("!" in /etc/shadow). "*" has no
  # password either, but is not locked.
  - sed -i 's/^root:![^:]*:/root:*:/' /etc/shadow
  - systemctl reload sshd 2>/dev/null || systemctl reload ssh 2>/dev/null || rc-service sshd reload 2>/dev/null || true
"""
    (directory / "user-data").write_text(user_data)
    (directory / "meta-data").write_text("instance-id: gnoblin-graphical\nlocal-hostname: gnoblin-graphical\n")
    seed = directory / "seed.iso"
    tool = next((name for name in ("genisoimage", "mkisofs", "xorriso") if shutil.which(name)), None)
    if tool is None:
        raise CheckFailed("genisoimage, mkisofs or xorriso is needed for the cloud-init seed")
    prefix = [tool, "-as", "mkisofs"] if tool == "xorriso" else [tool]
    subprocess.run(
        [*prefix, "-quiet", "-output", str(seed), "-volid", "cidata", "-joliet", "-rock",
         str(directory / "user-data"), str(directory / "meta-data")],
        check=True,
    )  # fmt: skip
    return seed


class Guest:
    def __init__(self, work, out, accel, memory, cpus, guest):
        self.work, self.out, self.accel = work, out, accel
        self.memory, self.cpus, self.guest = memory, cpus, guest
        self.process = None
        self.port = free_port()
        self.key = work / "id_ed25519"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.key)], check=True)

    def start(self, base, seed):
        disk = self.work / "disk.qcow2"
        info = json.loads(subprocess.run(["qemu-img", "info", "--output=json", str(base)], capture_output=True,
                                         text=True, check=True).stdout)  # fmt: skip
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-F", info["format"], "-b", str(base), str(disk),
                        "16G"], check=True)  # fmt: skip
        self.qmp_path = self.work / "qmp.sock"
        firmware = find_firmware(self.guest)
        if firmware:
            # The UEFI variables are written at boot, so the guest gets its own copy of the template.
            variables = self.work / "efi-vars.fd"
            shutil.copyfile(firmware[1], variables)
            firmware = (firmware[0], variables)
        guest = self.guest
        if self.accel != "kvm":
            guest = {**guest, "tcg_cpu": usable_tcg_cpu(guest)}
        command = qemu_command(guest, self.accel, self.memory, self.cpus, disk, seed, self.port, self.qmp_path,
                               self.out / "serial.log", firmware)  # fmt: skip
        (self.out / "command.txt").write_text(" ".join(shlex.quote(part) for part in command) + "\n")
        self.process = subprocess.Popen(command, stdout=(self.out / "qemu.log").open("w"), stderr=subprocess.STDOUT)
        for _ in range(100):
            if self.qmp_path.exists():
                break
            if self.process.poll() is not None:
                raise CheckFailed("QEMU exited at start", (self.out / "qemu.log").read_text())
            time.sleep(0.1)
        try:
            self.qmp = Qmp(self.qmp_path)
        except OSError as error:
            # QEMU makes the socket and then stops on a bad device: say so, with its own message.
            time.sleep(0.5)
            if self.process.poll() is not None:
                raise CheckFailed("QEMU exited at start", (self.out / "qemu.log").read_text()) from None
            raise CheckFailed(f"QMP socket {self.qmp_path} refused the connection: {error}") from None

    def ssh(self, user, command, timeout=600, check=True, stdin=None):
        result = subprocess.run(
            ["ssh", "-q", "-i", str(self.key), "-p", str(self.port), "-o", "BatchMode=yes",
             "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "ConnectTimeout=10",
             # One connection a user for the whole run: the many short checks open no new login session each.
             "-o", "ControlMaster=auto", "-o", f"ControlPath={self.work}/ssh-%r", "-o", "ControlPersist=300",
             f"{user}@127.0.0.1", command],
            input=stdin, capture_output=True, timeout=timeout,
        )  # fmt: skip
        stdout, stderr = result.stdout.decode(errors="replace"), result.stderr.decode(errors="replace")
        if check and result.returncode != 0:
            raise CheckFailed(f"guest command failed ({result.returncode}): {command}", tail(stdout + stderr))
        return result.returncode, stdout, stderr

    def root(self, command, **options):
        return self.ssh("root", command, **options)

    def user(self, command, **options):
        environment = f"export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0; {command}"
        return self.ssh(USER, environment, **options)

    def wait_ssh(self, timeout):
        deadline = time.monotonic() + timeout
        serial = self.out / "serial.log"
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise CheckFailed("QEMU exited during boot", tail(serial.read_text(errors="replace")))
            # A guest that has given up (emergency mode, PID 1 frozen, a kernel panic) never lets SSH in: say so now,
            # rather than after the whole boot timeout.
            text = serial.read_text(errors="replace") if serial.exists() else ""
            stuck = STUCK.search(text)
            if stuck:
                raise GuestStuck(f"the guest stopped booting: {stuck.group(0)}", tail(text))
            try:
                code, _, _ = self.root("test -e /var/lib/cloud/instance/boot-finished", timeout=30, check=False)
                if code == 0:
                    return
            except subprocess.TimeoutExpired:
                pass
            time.sleep(5)
        raise CheckFailed(f"no SSH login within {timeout} s", tail(serial.read_text(errors="replace")))

    def restart(self, base, seed, reason):
        """Stop the guest at once and boot it again from a fresh copy of the image. The first serial log is kept."""
        print(f"[graphical] boot again from a fresh disk: {reason}", flush=True)
        self.process.kill()
        self.process.wait()
        (self.out / "serial.log").rename(self.out / "serial-first-boot.log")
        self.qmp_path.unlink(missing_ok=True)
        self.start(base, seed)

    def copy_in(self, sources, destination):
        """Copy files and folders into a guest folder, through tar over SSH."""
        archive = subprocess.run(
            ["tar", "-cf", "-", "--transform", "s,^.*/,,", *[str(path) for path in sources if path.is_file()]],
            capture_output=True, check=True,
        ).stdout  # fmt: skip
        self.root(f"mkdir -p {destination} && tar -xf - -C {destination}", stdin=archive)

    def screendump(self, path):
        self.qmp.command("screendump", filename=str(path))
        for _ in range(50):
            if path.exists() and path.stat().st_size > 0:
                return
            time.sleep(0.1)
        raise CheckFailed(f"QMP screendump wrote no {path.name}")

    # QMP input goes to the guest's only keyboard and only absolute pointer. The "device" argument of input-send-event
    # is not used: QEMU 10.1 aborts on it ("qemu-fixed-text-console.device not found").
    def pointer_to(self, x, y):
        width, height = SCREEN
        self.qmp.command(
            "input-send-event",
            events=[{"type": "abs", "data": {"axis": "x", "value": round(x * 32767 / (width - 1))}},
                    {"type": "abs", "data": {"axis": "y", "value": round(y * 32767 / (height - 1))}}],
        )  # fmt: skip

    def click(self):
        for down in (True, False):
            self.qmp.command("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": "left"}}])
            time.sleep(0.05)

    def press(self, qcode):
        for down in (True, False):
            event = {"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}
            self.qmp.command("input-send-event", events=[event])
            time.sleep(0.05)

    def stop(self):
        if self.process is None or self.process.poll() is not None:
            return
        try:
            self.qmp.command("system_powerdown")
            self.process.wait(60)
        except (CheckFailed, OSError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait()


# ----------------------------------------------------------------------------------------------------------------------
# The run


class Run:
    def __init__(self, arguments, entry):
        self.arguments, self.entry = arguments, entry
        self.out = Path(arguments.out).resolve()
        self.out.mkdir(parents=True, exist_ok=True)
        self.breaks = set(arguments.breaks)
        self.checks = []
        self.facts = {}
        self.started = time.monotonic()
        self.guest = None
        self.expired = False
        self.in_check = False

    # -- bookkeeping

    def expire(self, *_):
        """SIGALRM at --deadline: the check that runs fails, and every later check is skipped, so the run still ends
        with a result.json before the job's own timeout kills it."""
        self.expired = True
        if self.in_check:
            raise Deadline(f"the run reached its deadline of {self.arguments.deadline} s")

    def record(self, name, status, detail="", excerpt="", seconds=0.0):
        self.checks.append({"name": name, "status": status, "detail": detail, "excerpt": excerpt,
                            "seconds": round(seconds, 1)})  # fmt: skip
        mark = {"pass": "PASS", "fail": "FAIL", "skip": "SKIP"}[status]
        elapsed = time.monotonic() - self.started
        print(f"[graphical] {mark} {name} ({seconds:.1f} s, {elapsed:.0f} s since the start)"
              f"{': ' + detail if detail else ''}", flush=True)  # fmt: skip
        if status == "fail" and excerpt:
            print(excerpt, flush=True)

    def check(self, name, function):
        """Run one check. It returns True when the check passed."""
        if any(item["status"] != "pass" for item in self.checks if item["name"] in self.needs(name)):
            failed = next(
                item["name"] for item in self.checks if item["name"] in self.needs(name) and item["status"] != "pass"
            )
            self.record(name, "skip", f"needs {failed}")
            return False
        if self.expired:
            self.record(name, "skip", f"the run reached its deadline of {self.arguments.deadline} s")
            return False
        started = time.monotonic()
        self.in_check = True
        try:
            detail = function() or ""
        except CheckFailed as failure:
            self.record(name, "fail", failure.detail, failure.excerpt, time.monotonic() - started)
            return False
        except subprocess.TimeoutExpired as exception:
            self.record(name, "fail", f"timed out: {exception.cmd[-1] if exception.cmd else ''}", "",
                        time.monotonic() - started)  # fmt: skip
            return False
        except Exception as exception:  # noqa: BLE001 (every run must end with a result.json)
            # A harness or QEMU fault (QMP connection lost, a malformed client log) fails the check, with the cause.
            self.record(name, "fail", f"harness error: {type(exception).__name__}: {exception}",
                        traceback.format_exc(), time.monotonic() - started)  # fmt: skip
            return False
        finally:
            self.in_check = False
        self.record(name, "pass", detail, "", time.monotonic() - started)
        return True

    @staticmethod
    def needs(name):
        """The checks that must pass before a check can run. The logs check runs whenever the guest is up."""
        chain = {
            "image": ("packaged",),
            "boot": ("image",),
            "install": ("boot",),
            "session": ("install",),
            "globals": ("session",),
            "window": ("session",),
            "screenshot": ("window",),
            "pointer": ("screenshot",),
            "keyboard": ("screenshot",),
            "shutdown": ("session",),
            "logs": ("install",),
        }
        return chain.get(name, ())

    def result(self):
        entry = self.entry
        failed = next((item for item in self.checks if item["status"] == "fail"), None)
        if not entry["packaged"]:
            outcome = "not packaged"
        else:
            outcome = "fail" if failed or len(self.checks) < len(CHECKS) else "pass"
        return {
            "name": entry["name"],
            "family": entry["family"],
            "release": entry["release"],
            "slug": entry["slug"],
            "arch": entry.get("arch", "amd64"),
            "eol": entry["eol"],
            "eol_date": entry.get("eol_date"),
            "state": entry.get("state"),
            "packaged": entry["packaged"],
            "reason": entry.get("reason", ""),
            "result": outcome,
            "first_failure": {k: failed[k] for k in ("name", "detail", "excerpt")} if failed else None,
            "checks": self.checks,
            "breaks": sorted(self.breaks),
            "boot_retry": self.facts.get("boot_retry"),
            "packages_from": self.arguments.packages,
            "accel": self.facts.get("accel"),
            "qemu": self.facts.get("qemu"),
            "image": self.facts.get("image"),
            "guest": self.facts.get("guest"),
            "renderer": self.facts.get("renderer"),
            "started": self.facts.get("started"),
            "seconds": round(time.monotonic() - self.started, 1),
            "artifact": self.arguments.artifact,
            "run_url": self.arguments.run_url,
        }

    # -- the checks

    def packaged(self):
        if not self.entry["packaged"]:
            raise CheckFailed(f"not packaged: {self.entry.get('reason') or 'no package job builds this release'}")
        source = self.arguments.packages
        if source not in ("nightly", "stable") and not Path(source).is_dir():
            raise CheckFailed(f"the package source {source} is not a folder, nightly or stable")
        return f"packages from {source}"

    def image(self):
        cache = Path(self.arguments.cache).resolve()
        if self.entry.get("image") is None:
            arch = self.entry.get("arch", "amd64")
            raise CheckFailed(f"no cloud image for {arch}: {self.entry.get('image_missing') or 'no image data'}")
        build = self.entry["image"].get("build")
        if build:
            self.image_info = build_image(build, cache, self.work, self.out / "image-build.log", "image" in self.breaks,
                                          self.arguments.min_free_gb)  # fmt: skip
            (self.out / "image-manifest.txt").write_text(self.image_info.pop("manifest"))
        else:
            self.image_info = download_image(self.entry["image"], cache, self.arguments.min_free_gb,
                                             "image" in self.breaks)  # fmt: skip
        self.facts["image"] = {key: value for key, value in self.image_info.items() if key != "path"}
        return self.image_info["file"]

    def boot(self):
        work = self.work
        public_key = (self.guest.key.with_suffix(".pub")).read_text().strip()
        if "boot" in self.breaks:
            # A key that is not the run's: the guest boots, and SSH never lets the harness in.
            other = work / "other_ed25519"
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(other)], check=True)
            public_key = other.with_suffix(".pub").read_text().strip()
        seed = make_seed(work, public_key)
        self.guest.start(Path(self.image_info["path"]), seed)
        try:
            self.guest.wait_ssh(self.arguments.boot_timeout)
        except GuestStuck as stuck:
            # Only an emulated guest boots again, and once: there the first boot races the host's speed (run
            # 41770211755, Fedora 44 aarch64: a 46 s soft lockup, then systemd "Failed to fork off sandboxing
            # environment ... Freezing execution"; the same image passed in the runs before). A second boot is a new
            # draw of that race. With KVM a stuck boot is a real failure, and it fails at once.
            if self.guest.accel == "kvm":
                raise
            self.facts["boot_retry"] = stuck.detail
            self.guest.restart(Path(self.image_info["path"]), seed, stuck.detail)
            self.guest.wait_ssh(self.arguments.boot_timeout)
        upgrade = self.entry["image"].get("upgrade")
        if upgrade:
            self.guest.root(upgrade, timeout=self.arguments.install_timeout)
            self.guest.root("(sleep 1; reboot) >/dev/null 2>&1 &", check=False)
            time.sleep(15)
            self.guest.wait_ssh(self.arguments.boot_timeout)
        _, kernel, _ = self.guest.root("uname -r")
        return f"kernel {kernel.strip()}" + (" after the upgrade to the rolling release" if upgrade else "")

    def install(self):
        guest = self.guest
        guest.copy_in([HERE / "guest.sh", HERE / "wlclient.py", HERE / "ready.lua"], GUEST_DIR)
        source = self.arguments.packages
        guest_source = source
        if source not in ("nightly", "stable"):
            folder = Path(source)
            files = [path for path in sorted(folder.rglob("*")) if path.is_file()]
            if "install" in self.breaks:
                files = []  # an empty package folder
            guest.root(f"mkdir -p {GUEST_DIR}/packages")
            if files:
                guest.copy_in(files, f"{GUEST_DIR}/packages")
            guest_source = f"{GUEST_DIR}/packages"
        guest.root("command -v bash >/dev/null || apk add bash >/dev/null")
        _, output, _ = guest.root(f"bash {GUEST_DIR}/guest.sh install {USER} {shlex.quote(guest_source)} 2>&1",
                                  timeout=self.arguments.install_timeout)  # fmt: skip
        (self.out / "install.log").write_text(output)
        _, info, _ = guest.root(f"bash {GUEST_DIR}/guest.sh info")
        self.facts["guest"] = json.loads(info)
        return f"gnoblin {self.facts['guest'].get('gnoblin')} on {self.facts['guest'].get('os')}"

    def renderer_name(self):
        """The GL renderer of the first output, from gnoblinctl output list."""
        try:
            outputs = json.loads(self.facts.get("renderer") or "{}").get("outputs", [])
            return outputs[0]["renderer"]["name"]
        except (json.JSONDecodeError, LookupError, TypeError, AttributeError):
            return "renderer unknown"

    def session(self):
        guest = self.guest
        if "session" in self.breaks:
            guest.root(f"bash {GUEST_DIR}/guest.sh unbind-gpu")
        guest.root(f"bash {GUEST_DIR}/guest.sh start {USER}")
        deadline = time.monotonic() + self.arguments.session_timeout
        last = ""
        while time.monotonic() < deadline:
            # The compositor is not dumpable, so only root can read its file descriptors.
            code, _, _ = guest.root(
                f"pid=$(pgrep -u {USER} -f -- '{COMPOSITOR}' | head -n 1); [ -n \"$pid\" ] || exit 3; "
                "ls -l /proc/$pid/fd 2>/dev/null | grep -q /dev/dri/card || exit 4",
                check=False, timeout=60,
            )  # fmt: skip
            if code == 0:
                code, output, _ = guest.user(
                    f"[ -S \"$XDG_RUNTIME_DIR/wayland-0\" ] || exit 5; gnoblinctl lua {GUEST_DIR}/ready.lua",
                    check=False, timeout=60,
                )  # fmt: skip
            last = {3: "no compositor process", 4: "the compositor holds no DRM device", 5: "no wayland-0 socket"}.get(
                code, output.strip() if code == 0 else f"gnoblinctl failed ({code})"
            )
            if code == 0:
                ready = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
                if ready.get("runtime") == "running" and ready.get("session") == "running":
                    _, outputs, _ = guest.user("gnoblinctl output list --json", check=False)
                    self.facts["renderer"] = outputs.strip()
                    (self.out / "outputs.json").write_text(outputs)
                    return f"runtime and session running, {self.renderer_name()}"
                last = ", ".join(f"{key} {value}" for key, value in ready.items())
            _, state, _ = guest.root(f"bash {GUEST_DIR}/guest.sh status", check=False)
            if state.startswith("ended"):
                raise CheckFailed(f"the session {state.strip()} before it reached running", tail(self.session_log()))
            time.sleep(2)
        raise CheckFailed(f"the session did not reach running in {self.arguments.session_timeout} s: {last}",
                          tail(self.session_log()))  # fmt: skip

    def globals(self):
        guest = self.guest
        script = f"{GUEST_DIR}/wlclient.py"
        width, height = CLIENT_SIZE
        if "window" in self.breaks:
            # A client that reads the globals and then cannot make a buffer: it never maps a window.
            width = height = 0
        guest.user(
            f"rm -f {GUEST_DIR}/session/globals.json {GUEST_DIR}/session/events.jsonl; "
            f"cd {GUEST_DIR}/session && (setsid python3 {script} --globals globals.json --events events.jsonl "
            f"--width {width} --height {height} > client.log 2>&1 < /dev/null &)",
        )
        text = self.wait_guest_file(f"{GUEST_DIR}/session/globals.json", self.arguments.client_timeout)
        if text is None:
            raise CheckFailed("the client wrote no globals", self.guest_file(f"{GUEST_DIR}/session/client.log"))
        (self.out / "globals.json").write_text(text)
        announced = {item["interface"] for item in json.loads(text)}
        expected = [line.split("#")[0].strip() for line in (HERE / "expected-globals.txt").read_text().splitlines()]
        expected = [name for name in expected if name]
        if "globals" in self.breaks:
            expected.append("gnoblin_graphical_test_missing_v1")
        missing = [name for name in expected if name not in announced]
        if missing:
            raise CheckFailed(f"missing globals: {', '.join(missing)}", ", ".join(sorted(announced)))
        return f"{len(announced)} globals, all {len(expected)} expected present"

    def window(self):
        events = self.wait_event("mapped", self.arguments.client_timeout)
        if events is None:
            # What the screen and the compositor show at that moment, for a window that never got its first frame.
            _, windows, _ = self.guest.user("gnoblinctl window list --json", check=False)
            (self.out / "windows.json").write_text(windows)
            try:
                self.guest.screendump(self.out / "window-not-mapped.ppm")
                analyse(self.out / "window-not-mapped.ppm", FIRST_COLOUR)
                (self.out / "window-not-mapped.ppm").unlink()
            except CheckFailed:
                pass
            events = self.client_events()
            raise CheckFailed(f"the client window did not map within {self.arguments.client_timeout} s",
                              self.events_excerpt(events) + "\n" + self.guest_file(f"{GUEST_DIR}/session/client.log")
                              + "\nwindows: " + tail(windows))  # fmt: skip
        _, windows, _ = self.guest.user("gnoblinctl window list --json", check=False)
        (self.out / "windows.json").write_text(windows)
        if APP_ID not in windows:
            raise CheckFailed(f"gnoblinctl window list does not list {APP_ID}", tail(windows))
        if "screenshot" in self.breaks:
            # The client goes away after it mapped: the screen no longer shows it.
            self.guest.user(f"pkill -f -- '{CLIENT}'", check=False)
            time.sleep(2)
        return "mapped and listed by gnoblinctl"

    def screenshot(self):
        time.sleep(1)
        path = self.out / "screenshot.ppm"
        self.guest.screendump(path)
        properties = analyse(path, FIRST_COLOUR)
        path.unlink()
        self.facts["screenshot"] = properties
        check_frame(properties, FIRST_COLOUR, "the screenshot")
        return f"{properties['colours']} colours, client {properties['client_box']}"

    def pointer(self):
        guest = self.guest
        if "pointer" in self.breaks:
            guest.root(f"bash {GUEST_DIR}/guest.sh unbind Tablet")
        box = self.facts["screenshot"]["client_box"]
        x, y = (box[0] + box[2]) // 2, (box[1] + box[3]) // 2
        guest.pointer_to(x, y)
        time.sleep(0.5)
        guest.pointer_to(x + POINTER_STEP[0], y + POINTER_STEP[1])
        time.sleep(0.5)
        guest.click()
        # Wait for the release of the click, up to the client timeout: under TCG the events come seconds later.
        events = self.poll_events(
            lambda found: any(e["event"] == "pointer-button" and not e.get("pressed") for e in found)
        )
        enter = [event for event in events if event["event"] == "pointer-enter" and event.get("surface")]
        motion = [event for event in events if event["event"] in ("pointer-enter", "pointer-motion")]
        buttons = [event for event in events if event["event"] == "pointer-button" and event["button"] == BTN_LEFT]
        if not enter:
            raise CheckFailed("the pointer never entered the client window", self.events_excerpt(events))
        last = motion[-1]
        local = (x + POINTER_STEP[0] - box[0], y + POINTER_STEP[1] - box[1])
        if abs(last["x"] - local[0]) > 3 or abs(last["y"] - local[1]) > 3:
            raise CheckFailed(f"the pointer is at {last['x']:.0f},{last['y']:.0f} in the window, not {local[0]},{local[1]}",
                              self.events_excerpt(events))  # fmt: skip
        if [event["pressed"] for event in buttons] != [True, False]:
            raise CheckFailed("the client did not see one left click", self.events_excerpt(events))
        return f"entered, moved to {last['x']:.0f},{last['y']:.0f}, clicked"

    def keyboard(self):
        guest = self.guest
        if "keyboard" in self.breaks:
            guest.root(f"bash {GUEST_DIR}/guest.sh unbind Keyboard")
        for qcode, _ in TYPED:
            guest.press(qcode)
        expected_keys = [code for _, code in TYPED]
        events = self.poll_events(
            lambda found: (
                [e["key"] for e in found if e["event"] == "key" and e["pressed"]][-len(expected_keys) :]
                == expected_keys
            )
        )
        pressed = [event["key"] for event in events if event["event"] == "key" and event["pressed"]]
        expected = [code for _, code in TYPED]
        if pressed[-len(expected) :] != expected:
            raise CheckFailed(f"the client saw the keys {pressed}, not {expected}", self.events_excerpt(events))
        path = self.out / "screenshot-typed.ppm"
        # The repaint after G reaches the screen within the client timeout; on KVM the first screendump has it.
        deadline = time.monotonic() + self.arguments.client_timeout
        while True:
            guest.screendump(path)
            properties = analyse(path, SECOND_COLOUR)
            path.unlink()
            if properties["client_pixels"] or time.monotonic() >= deadline:
                break
            time.sleep(5)
        self.facts["screenshot_typed"] = properties
        check_frame(properties, SECOND_COLOUR, "the screenshot after typing")
        return f"keys {pressed[-len(expected) :]} seen, the repaint is on screen"

    def shutdown(self):
        guest = self.guest
        if "shutdown" in self.breaks:
            guest.user(f"pkill -SEGV -u $(id -u) -f -- '{COMPOSITOR}'", check=False)
            time.sleep(3)
        guest.user(f"pkill -f -- '{CLIENT}'", check=False)
        guest.user("gnoblinctl logout", check=False, timeout=60)
        deadline = time.monotonic() + 60
        state = ""
        while time.monotonic() < deadline:
            _, state, _ = guest.root(f"bash {GUEST_DIR}/guest.sh status", check=False)
            if state.startswith("ended"):
                break
            time.sleep(1)
        if not state.startswith("ended"):
            raise CheckFailed("the session did not end within 60 s of gnoblinctl logout", tail(self.session_log()))
        _, left, _ = guest.root(f"pgrep -u {USER} -af gnoblin", check=False)
        _, cores, _ = guest.root(
            "(coredumpctl --no-legend --no-pager list 2>/dev/null; journalctl -b --no-pager -q 2>/dev/null | "
            "grep -E 'dumped core|Process [0-9]+ \\((gnoblin|gnoblinctl|mutter)' ) | grep -i -E 'gnoblin|mutter' || true",
            check=False,
        )  # fmt: skip
        problems = []
        if state.strip() != "ended 0":
            problems.append(f"the session {state.strip()}, not with status 0")
        if left.strip():
            problems.append("Gnoblin processes are still running")
        if cores.strip():
            problems.append("a Gnoblin process dumped core")
        if problems:
            raise CheckFailed("; ".join(problems), tail(cores + left + self.session_log()))
        return "status 0, no process left, no core dump"

    def logs(self):
        guest = self.guest
        if "logs" in self.breaks:
            guest.user(
                "logger -t gnoblin 'gnoblin-CRITICAL **: injected by the graphical test --break logs'", check=False
            )
            time.sleep(1)
        lines = self.collect_logs()
        patterns, source = load_allow_list()
        found = [line for line in lines if LOG_LEVEL.search(line)]
        unexpected = [line for line in found if not any(pattern.search(line) for pattern in patterns)]
        (self.out / "log-errors.txt").write_text("\n".join(found) + ("\n" if found else ""))
        if unexpected:
            raise CheckFailed(f"{len(unexpected)} CRITICAL or ERROR lines outside {source}", "\n".join(unexpected[:30]))
        return f"{len(found)} CRITICAL or ERROR lines, all in {source}" if found else "no CRITICAL or ERROR line"

    # -- guest helpers

    def guest_file(self, path):
        _, output, _ = self.guest.root(f"cat {path} 2>/dev/null", check=False)
        return output

    def wait_guest_file(self, path, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            code, output, _ = self.guest.root(f"cat {path}", check=False)
            if code == 0 and output:
                return output
            time.sleep(1)
        return None

    def client_events(self):
        text = self.guest_file(f"{GUEST_DIR}/session/events.jsonl")
        (self.out / "client-events.jsonl").write_text(text)
        return [json.loads(line) for line in text.splitlines() if line.strip()]

    def wait_event(self, name, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            events = self.client_events()
            if any(event["event"] == name for event in events):
                return events
            time.sleep(1)
        return None

    def poll_events(self, done):
        """The client's events once done(events) is true, or when the client timeout runs out. A short first wait
        keeps the KVM path as fast as before."""
        deadline = time.monotonic() + self.arguments.client_timeout
        time.sleep(1)
        while True:
            events = self.client_events()
            if done(events) or time.monotonic() >= deadline:
                return events
            time.sleep(1)

    @staticmethod
    def events_excerpt(events):
        return "\n".join(json.dumps(event) for event in events[-20:])

    def session_log(self):
        _, log, _ = self.guest.root(f"bash {GUEST_DIR}/guest.sh log", check=False)
        return log

    def collect_logs(self):
        """The session log and the test user's journal of this boot. Both are saved as evidence. A line in both
        counts once."""
        guest = self.guest
        session = self.session_log()
        (self.out / "session.log").write_text(session)
        _, journal, _ = guest.root(f"journalctl -b --no-pager -o short-monotonic _UID=$(id -u {USER}) 2>/dev/null || "
                                   "cat /var/log/messages 2>/dev/null", check=False)  # fmt: skip
        (self.out / "journal.txt").write_text(journal)
        return list(dict.fromkeys(session.splitlines() + journal.splitlines()))

    def collect(self):
        """Evidence that every run keeps, whatever failed."""
        guest = self.guest
        try:
            _, journal, _ = guest.root("journalctl -b --no-pager -o short-monotonic 2>/dev/null || dmesg", check=False,
                                       timeout=120)  # fmt: skip
            (self.out / "journal-system.txt").write_text(journal)
            if not (self.out / "session.log").exists():
                (self.out / "session.log").write_text(self.session_log())
            for name in ("client.log", "globals.json"):
                text = self.guest_file(f"{GUEST_DIR}/session/{name}")
                if text:
                    (self.out / name).write_text(text)
            self.client_events()
        except (subprocess.TimeoutExpired, OSError):
            pass

    # -- the whole run

    def run(self):
        self.facts["started"] = now()
        self.facts["accel"] = accelerator(self.arguments.accel)
        guest = guest_of(self.entry)
        try:
            self.facts["qemu"] = subprocess.run([guest["qemu"], "--version"], capture_output=True,
                                                text=True).stdout.splitlines()[0]  # fmt: skip
        except (OSError, IndexError):
            self.facts["qemu"] = f"{guest['qemu']} is not installed"
        (self.out / "qemu-version.txt").write_text(self.facts["qemu"] + "\n")
        (self.out / "harness.txt").write_text(" ".join(shlex.quote(part) for part in sys.argv) + "\n")
        (self.out / "target.json").write_text(json.dumps(self.entry, indent=2) + "\n")
        work = Path(tempfile.mkdtemp(prefix="gnoblin-graphical-", dir=self.arguments.work))
        self.work = work
        if self.arguments.deadline:
            signal.signal(signal.SIGALRM, self.expire)
            signal.alarm(self.arguments.deadline)
        try:
            if self.check("packaged", self.packaged):
                self.guest = Guest(work, self.out, self.facts["accel"], self.arguments.memory, self.arguments.cpus,
                                   guest)  # fmt: skip
                for name in CHECKS[1:]:
                    self.check(name, getattr(self, name))
            else:
                for name in CHECKS[1:]:
                    self.record(name, "skip", "not packaged")
        finally:
            if self.guest and self.guest.process:
                if self.guest.process.poll() is None and any(
                    c["name"] == "boot" and c["status"] == "pass" for c in self.checks
                ):
                    self.collect()
                    if self.arguments.hold:
                        print(f"[graphical] holding the guest for {self.arguments.hold} s: ssh -i {self.guest.key} "
                              f"-p {self.guest.port} root@127.0.0.1", flush=True)  # fmt: skip
                        time.sleep(self.arguments.hold)
                self.guest.stop()
            shutil.rmtree(work, ignore_errors=True)
            if not self.arguments.keep_image and getattr(self, "image_info", None):
                Path(self.image_info["path"]).unlink(missing_ok=True)
        result = self.result()
        (self.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(f"[graphical] {result['name']}: {result['result']} in {result['seconds']} s, evidence in {self.out}")
        return {"pass": 0, "fail": 1, "not packaged": 3}[result["result"]]


def load_allow_list():
    """The tolerated log lines: the shared list of the package tests, or the graphical test's own one until it exists.
    One extended regular expression a line, with an optional "  # reason"."""
    for path in ALLOW_LISTS:
        if path.is_file():
            patterns = []
            for line in path.read_text().splitlines():
                rule = re.split(r"\s+#\s", line, maxsplit=1)[0].strip()
                if rule and not rule.startswith("#"):
                    patterns.append(re.compile(rule))
            return patterns, str(path.relative_to(ROOT))
    return [], "no allow-list"


def find_target(name, arch):
    output = subprocess.run([sys.executable, str(ROOT / "scripts/supported-targets.py")], capture_output=True,
                            text=True, check=True).stdout  # fmt: skip
    targets = json.loads(output)["graphical"]
    for target in targets:
        if name in (target["name"], target["slug"]) and target["arch"] == arch:
            return target
    names = ", ".join(f"{target['name']} ({target['slug']})" for target in targets)
    raise SystemExit(f"harness: no graphical target {name!r}. The targets: {names}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    which = parser.add_mutually_exclusive_group(required=True)
    which.add_argument("--release", help='a graphical target: "Fedora 42", "Arch Linux" or a slug such as fc42')
    which.add_argument("--entry", type=Path, help="a JSON file with one graphical target")
    parser.add_argument("--packages", required=True, help="a folder of package files, or nightly or stable")
    parser.add_argument("--out", required=True, help="the evidence folder")
    parser.add_argument("--work", default=os.environ.get("TMPDIR", "/var/tmp"), help="where the guest disk goes")
    parser.add_argument(
        "--cache", default=os.path.join(os.environ.get("TMPDIR", "/var/tmp"), "gnoblin-graphical-images")
    )
    parser.add_argument("--keep-image", action="store_true", help="keep the downloaded cloud image in --cache")
    parser.add_argument("--accel", choices=("auto", "kvm", "tcg"), default="auto")
    parser.add_argument("--memory", type=int, default=4096, help="guest memory in MiB")
    parser.add_argument("--cpus", type=int, default=4)
    parser.add_argument("--min-free-gb", type=float, default=4.0, help="refuse to download below this free space")
    parser.add_argument("--boot-timeout", type=int, help="seconds; default 600 with KVM and 1800 with TCG")
    parser.add_argument("--install-timeout", type=int, help="seconds; default 1800 with KVM and 7200 with TCG")
    parser.add_argument("--session-timeout", type=int, help="seconds; default 120 with KVM and 600 with TCG")
    parser.add_argument("--client-timeout", type=int,
                        help="seconds for the test client's globals and first frame; default 30 with KVM, 300 with TCG")  # fmt: skip
    parser.add_argument("--break", dest="breaks", action="append", default=[], choices=CHECKS[1:],
                        help="make the guest fail this check on purpose (repeatable)")  # fmt: skip
    parser.add_argument("--artifact", default="", help="the name of the evidence artifact, for the report")
    parser.add_argument("--run-url", default="", help="the URL of the CI run, for the report")
    parser.add_argument("--hold", type=int, default=0, help="keep the guest up this many seconds before it stops")
    parser.add_argument("--arch", default="amd64", help="the architecture of --release (the guest runs that one)")
    parser.add_argument("--deadline", type=int, default=0,
                        help="seconds for the whole run; the check that runs then fails and the rest are skipped, "
                        "so result.json is written before a CI job timeout kills the harness. 0: none")  # fmt: skip
    arguments = parser.parse_args()
    tcg = accelerator(arguments.accel) == "tcg"
    arguments.boot_timeout = arguments.boot_timeout or (1800 if tcg else 600)
    # An install on TCG may hold a full upgrade of a rolling release (Debian testing): two hours, still inside the job.
    arguments.install_timeout = arguments.install_timeout or (7200 if tcg else 1800)
    arguments.session_timeout = arguments.session_timeout or (600 if tcg else 120)
    # The client's first frame needs the compositor and llvmpipe to draw: under TCG that takes far longer than 30 s.
    arguments.client_timeout = arguments.client_timeout or (300 if tcg else 30)
    entry = (
        json.loads(arguments.entry.read_text()) if arguments.entry else find_target(arguments.release, arguments.arch)
    )
    if arguments.packages not in ("nightly", "stable"):
        arguments.packages = str(Path(arguments.packages).resolve())
    return Run(arguments, entry).run()


if __name__ == "__main__":
    sys.exit(main())
