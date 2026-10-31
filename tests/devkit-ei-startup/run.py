#!/usr/bin/env python3
"""Run complete Mutter mdk-ei.c with real libei/libeis, without GTK or a desktop."""

import argparse
import json
import os
from pathlib import Path
import resource
import shlex
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PATCH = ROOT / "patches/mutter/10-tooling/0009-devkit-fail-on-eis-startup-disconnect.patch"
CASES = ("closed-peer", "null-error", "repeat", "rejected", "seat")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--sanitize", action="store_true")
    args = parser.parse_args()
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    revision = json.loads((ROOT / "src/gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-2.0", "libei-1.0", "libeis-1.0"], text=True)
    )
    with tempfile.TemporaryDirectory(prefix=".run-", dir=HERE) as scratch:
        directory = Path(scratch)
        subprocess.run(["git", "init", "--quiet", scratch], check=True)
        mdk = directory / "mdk"
        mdk.mkdir()
        for name in ("mdk-ei.c", "mdk-ei.h", "mdk-seat.h", "mdk-types.h"):
            (mdk / name).write_bytes(
                subprocess.check_output(
                    ["git", "-C", str(ROOT / "subprojects/mutter"), "show", f"{revision}:mdk/{name}"]
                )
            )
        if not args.baseline:
            subprocess.run(["git", "apply", "--check", "--whitespace=error", str(PATCH)], cwd=directory, check=True)
            subprocess.run(["git", "apply", str(PATCH)], cwd=directory, check=True)
        (mdk / "config.h").write_text("/* Non-GUI fixture: no build options needed. */\n")
        binary = directory / "fixture"
        subprocess.run(
            [
                os.environ.get("CC", "cc"),
                "-std=gnu11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Wno-unused-parameter",
                "-g",
                "-I",
                str(mdk),
                str(HERE / "fixture.c"),
                *(["-fsanitize=address,undefined", "-fno-omit-frame-pointer"] if args.sanitize else []),
                "-o",
                str(binary),
                *flags,
            ],
            check=True,
            env={**os.environ, "CCACHE_DISABLE": "1"},
        )
        failures = []
        test_env = {**os.environ, "G_DEBUG": "fatal-criticals"}
        if args.sanitize:
            test_env.update(ASAN_OPTIONS="detect_leaks=1:abort_on_error=1", UBSAN_OPTIONS="halt_on_error=1")
        for case in (args.case,) if args.case else CASES:
            try:
                result = subprocess.run([str(binary), case], capture_output=True, text=True, timeout=3, env=test_env)
            except subprocess.TimeoutExpired as error:
                observed = error.stderr or b""
                print(
                    f"FAIL {case}: constructor did not return within 3 s; "
                    f"real disconnect observed={b'Observed real EI_EVENT_DISCONNECT' in observed}",
                    flush=True,
                )
                failures.append(case)
                continue
            if result.returncode:
                print(result.stdout + result.stderr, end="")
                print(f"FAIL {case}: exit status {result.returncode}", flush=True)
                failures.append(case)
            else:
                print(result.stdout, end="", flush=True)
        print(f"{'BASELINE' if args.baseline else 'PATCHED'}: {len(failures)} failed", flush=True)
        return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
