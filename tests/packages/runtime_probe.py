#!/usr/bin/env python3
"""Helpers of the runtime layer, run in the container of the target with the Python of the system.

usage: runtime_probe.py as-user USER COMMAND...       run a command as USER, with its home folder
       runtime_probe.py globals WAYLAND_INFO_OUTPUT   compare the advertised globals with the expected ones (R2)
       runtime_probe.py bridge                        greet, ping and read the API version of the compositor bridge (R3)
       runtime_probe.py image FILE WIDTH HEIGHT [RRGGBB]
                                                      check a PPM screenshot: its size, not blank, and with a colour,
                                                      that most of it is that colour (R6)
       runtime_probe.py logs FILE...                  report CRITICAL and ERROR lines that the allow-list does not
                                                      explain (R7)
"""

from __future__ import annotations

import json
import os
import pwd
import re
import socket
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PRIVATE = Path("/usr/lib/gnoblin")


def as_user(user: str, command: list[str]) -> int:
    entry = pwd.getpwnam(user)
    os.initgroups(entry.pw_name, entry.pw_gid)
    os.setgid(entry.pw_gid)
    os.setuid(entry.pw_uid)
    os.environ.update(HOME=entry.pw_dir, USER=entry.pw_name, LOGNAME=entry.pw_name)
    os.chdir(entry.pw_dir)
    os.execvp(command[0], command)
    return 1


def compiled_interfaces() -> set[str]:
    """The interfaces whose names the installed compositor holds: a generated protocol keeps the name as a string, and
    an interface of libwayland is an imported symbol <name>_interface."""
    blobs = []
    for path in [PRIVATE / "bin/gnoblin", *sorted((PRIVATE / "lib").glob("libmutter-*.so*"))]:
        if path.is_file() and not path.is_symlink():
            blobs.append(path.read_bytes())
    found = set()
    for item in json.loads((HERE / "globals.json").read_text())["globals"]:
        name = item["interface"].encode()
        if any(b"\0" + name + b"\0" in blob or b"\0" + name + b"_interface\0" in blob for blob in blobs):
            found.add(item["interface"])
    return found


def check_globals(output: Path) -> int:
    advertised = set(re.findall(r"interface: '([^']+)'", output.read_text(errors="replace")))
    inventory = json.loads((HERE / "globals.json").read_text())["globals"]
    compiled = compiled_interfaces()
    expected, excused, absent = set(), {}, []
    for item in inventory:
        name = item["interface"]
        if name not in compiled:
            absent.append(name)
        elif item.get("container"):
            excused[name] = item["container"]
        else:
            expected.add(name)
    missing = sorted(expected - advertised)
    print(
        f"inventory {len(inventory)}, in the binaries {len(compiled)}, expected {len(expected)}, advertised {len(advertised)}"
    )
    print(f"not in the binaries of this build: {', '.join(absent) or 'none'}")
    for name, reason in sorted(excused.items()):
        print(f"not expected in a container: {name}: {reason}")
    extra = sorted(advertised - {item["interface"] for item in inventory})
    if extra:
        print(f"advertised and not in the inventory: {', '.join(extra)}")
    for name in missing:
        print(f"MISSING: {name}")
    if len(expected) < 40:
        print("fewer than 40 expected interfaces: the binaries were not read")
        return 1
    return 1 if missing or extra else 0


def check_bridge() -> int:
    path = os.environ.get("GNOBLIN_COMPOSITOR_SOCKET") or os.path.join(
        os.environ["XDG_RUNTIME_DIR"], "gnoblin", "compositor-v1.sock"
    )
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(10)
        connection.connect(path)
        stream = connection.makefile("rwb")
        hello = json.loads(stream.readline())
        assert hello.get("event") == "hello", hello
        assert hello.get("version") == 1 and hello.get("api_major") == 1, hello
        assert isinstance(hello.get("api_minor"), int) and hello["api_minor"] > 0, hello
        assert hello.get("methods"), "the greeting lists no methods"
        stream.write(b'{"op": "ping", "id": "package-suite"}\n')
        stream.flush()
        reply = json.loads(stream.readline())
        assert reply.get("id") == "package-suite" and reply.get("result", {}).get("pong") == "pong", reply
        stream.write(
            json.dumps(
                {"op": "monitors", "id": "monitors", "api_version": {"major": 1, "minor": hello["api_minor"]}}
            ).encode()
            + b"\n"
        )
        stream.flush()
        reply = json.loads(stream.readline())
        print(
            f"bridge API 1.{hello['api_minor']}, {len(hello['methods'])} methods, monitors reply: {json.dumps(reply)[:300]}"
        )
    return 0


def check_image(path: Path, width: int, height: int, colour: str | None) -> int:
    data = path.read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+(\d+)\s", data)
    if not match:
        print("not a binary PPM file")
        return 1
    w, h, depth = (int(group) for group in match.groups())
    pixels = data[match.end() :]
    if depth != 255 or len(pixels) < w * h * 3:
        print(f"unexpected PPM: depth {depth}, {len(pixels)} bytes for {w}x{h}")
        return 1
    print(f"screenshot {w}x{h}")
    if (w, h) != (width, height):
        print(f"expected {width}x{height}")
        return 1
    if not pixels.strip(b"\0"):
        print("every pixel is black: the screenshot is blank")
        return 1
    if colour:
        want = bytes.fromhex(colour)
        count = sum(1 for index in range(0, w * h * 3, 3) if pixels[index : index + 3] == want)
        share = count / (w * h)
        print(f"{share:.1%} of the pixels are #{colour}")
        if share < 0.95:
            return 1
    else:
        print(f"{len(set(pixels[index : index + 3] for index in range(0, w * h * 3, 3 * 97)))} colours in a sample")
    return 0


def allow_list() -> list[tuple[re.Pattern, str]]:
    rules = []
    for line in (HERE / "log-allowlist.txt").read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        pattern, _, reason = line.partition("  # ")
        if not reason.strip():
            raise SystemExit(f"log-allowlist.txt: a rule without a reason: {line}")
        rules.append((re.compile(pattern.strip()), reason.strip()))
    return rules


SERIOUS = re.compile(
    r"CRITICAL|\bERROR\b|-ERROR \*\*|\[E\]|Segmentation fault|core dumped|AddressSanitizer|LeakSanitizer|"
    r"UndefinedBehaviorSanitizer|SIGSEGV|SIGABRT|Aborted|Trace/breakpoint trap|terminated by signal"
)


def check_logs(files: list[Path]) -> int:
    rules = allow_list()
    problems, allowed = [], {}
    for path in files:
        if not path.is_file():
            continue
        for line in path.read_text(errors="replace").splitlines():
            if not SERIOUS.search(line):
                continue
            rule = next((reason for pattern, reason in rules if pattern.search(line)), None)
            if rule:
                allowed[rule] = allowed.get(rule, 0) + 1
            else:
                problems.append(f"{path.name}: {line}")
    for reason, count in allowed.items():
        print(f"allowed {count}x: {reason}")
    for problem in problems:
        print(f"UNEXPECTED: {problem}")
    return 1 if problems else 0


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    command, args = sys.argv[1], sys.argv[2:]
    if command == "as-user":
        return as_user(args[0], args[1:])
    if command == "globals":
        return check_globals(Path(args[0]))
    if command == "bridge":
        return check_bridge()
    if command == "image":
        return check_image(Path(args[0]), int(args[1]), int(args[2]), args[3] if len(args) > 3 else None)
    if command == "logs":
        return check_logs([Path(arg) for arg in args])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
